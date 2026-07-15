use mdstream::{EngineError, EngineOutput, StreamEngine};
use mdstream_protocol::{ChangeSet, DocumentLifecycle};
use tokio::sync::mpsc;
use tokio::task::{JoinError, JoinHandle};
use tokio::time::Instant;

use crate::CoalesceOptions;

const OUTPUT_CAPACITY: usize = 64;

/// An ordered command accepted by the stream-engine actor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActorCommand {
    Append(String),
    Reset,
    Finish,
}

impl From<String> for ActorCommand {
    fn from(chunk: String) -> Self {
        Self::Append(chunk)
    }
}

/// One atomic actor response.
///
/// Every successful item contains all change sets produced by one engine
/// transition. An output receiver can therefore close without observing only
/// part of a transition.
pub type ActorResult = Result<Vec<ChangeSet>, EngineError>;

/// Output and completion handle for a spawned stream-engine actor.
pub struct StreamEngineActor {
    output: mpsc::Receiver<ActorResult>,
    task: JoinHandle<()>,
}

impl StreamEngineActor {
    pub async fn recv(&mut self) -> Option<ActorResult> {
        self.output.recv().await
    }

    /// Stop accepting actor output. The actor observes this as cancellation.
    pub fn close_output(&mut self) {
        self.output.close();
    }

    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// Wait for the actor task after its input closed or output was cancelled.
    pub async fn join(self) -> Result<(), JoinError> {
        self.task.await
    }
}

/// Spawn a task that owns a [`StreamEngine`] and emits atomic change-set batches.
///
/// Adjacent [`ActorCommand::Append`] commands are losslessly coalesced. Reset and
/// finish commands are ordering barriers: buffered content is applied before the
/// command. Closing the input finishes an open document exactly once. Closing
/// the output cancels the actor and drops its input receiver.
pub fn spawn_stream_engine_actor(
    mut engine: StreamEngine,
    input: mpsc::Receiver<ActorCommand>,
    options: CoalesceOptions,
) -> StreamEngineActor {
    let (output, output_rx) = mpsc::channel(OUTPUT_CAPACITY);

    let task = tokio::spawn(async move {
        let mut input = CoalescingCommandReceiver::new(input, options);
        loop {
            let command = tokio::select! {
                biased;
                _ = output.closed() => return,
                command = input.recv() => command,
            };

            let Some(command) = command else {
                if engine.lifecycle() == DocumentLifecycle::Open {
                    let _ = publish(&output, engine.finish()).await;
                }
                return;
            };

            let result = match command {
                CoalescedCommand::Append(chunk) => engine.append(&chunk),
                CoalescedCommand::Reset => engine.reset(),
                CoalescedCommand::Finish => engine.finish(),
            };
            if !publish(&output, result).await {
                return;
            }
        }
    });

    StreamEngineActor {
        output: output_rx,
        task,
    }
}

async fn publish(
    output: &mpsc::Sender<ActorResult>,
    result: Result<EngineOutput, EngineError>,
) -> bool {
    let result = result.map(EngineOutput::into_changes);
    if matches!(&result, Ok(changes) if changes.is_empty()) {
        return true;
    }
    output.send(result).await.is_ok()
}

enum CoalescedCommand {
    Append(String),
    Reset,
    Finish,
}

struct CoalescingCommandReceiver {
    input: mpsc::Receiver<ActorCommand>,
    options: CoalesceOptions,
    buffer: String,
    deadline: Option<Instant>,
    pending_barrier: Option<ActorCommand>,
    closed: bool,
}

impl CoalescingCommandReceiver {
    fn new(input: mpsc::Receiver<ActorCommand>, options: CoalesceOptions) -> Self {
        Self {
            input,
            options,
            buffer: String::new(),
            deadline: None,
            pending_barrier: None,
            closed: false,
        }
    }

    async fn recv(&mut self) -> Option<CoalescedCommand> {
        loop {
            if self.should_flush() {
                return Some(self.take_append());
            }
            if self.buffer.is_empty() {
                if let Some(barrier) = self.pending_barrier.take() {
                    return Some(Self::barrier(barrier));
                }
                if self.closed {
                    return None;
                }

                match self.input.recv().await {
                    Some(ActorCommand::Append(chunk)) if chunk.is_empty() => {
                        return Some(CoalescedCommand::Append(chunk));
                    }
                    Some(ActorCommand::Append(chunk)) => self.push(chunk),
                    Some(barrier) => return Some(Self::barrier(barrier)),
                    None => self.closed = true,
                }
                continue;
            }

            let deadline = self
                .deadline
                .get_or_insert_with(|| Instant::now() + self.options.max_delay)
                .to_owned();
            match tokio::time::timeout_at(deadline, self.input.recv()).await {
                Ok(Some(ActorCommand::Append(chunk))) => self.push(chunk),
                Ok(Some(barrier)) => {
                    self.pending_barrier = Some(barrier);
                    return Some(self.take_append());
                }
                Ok(None) => {
                    self.closed = true;
                    return Some(self.take_append());
                }
                Err(_) => return Some(self.take_append()),
            }
        }
    }

    fn push(&mut self, chunk: String) {
        if self.buffer.is_empty() {
            self.deadline = Some(Instant::now() + self.options.max_delay);
        }
        self.buffer.push_str(&chunk);
    }

    fn should_flush(&self) -> bool {
        !self.buffer.is_empty()
            && (self.buffer.len() >= self.options.max_bytes
                || (self.options.flush_on_newline && self.buffer.contains('\n')))
    }

    fn take_append(&mut self) -> CoalescedCommand {
        self.deadline = None;
        CoalescedCommand::Append(std::mem::take(&mut self.buffer))
    }

    fn barrier(command: ActorCommand) -> CoalescedCommand {
        match command {
            ActorCommand::Reset => CoalescedCommand::Reset,
            ActorCommand::Finish => CoalescedCommand::Finish,
            ActorCommand::Append(_) => unreachable!("append commands are coalesced separately"),
        }
    }
}
