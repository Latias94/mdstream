use std::collections::VecDeque;
use std::error::Error;
use std::fmt;

use mdstream::{EngineError, EngineOutput, StreamEngine};
use mdstream_protocol::{ChangeSet, DocumentLifecycle};
use tokio::sync::mpsc;
use tokio::task::{JoinError, JoinHandle};
use tokio::time::Instant;

use crate::coalesce::{PendingChunks, PendingInput, ScannedChunk};
use crate::stats::CoalesceWork;
use crate::{ActorStats, CoalesceOptions};

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

/// Results committed by one coalescer flush or lifecycle command.
///
/// Constituent append boundaries remain visible even though the whole batch is
/// published through one channel operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActorBatch {
    transitions: Vec<EngineOutput>,
}

impl ActorBatch {
    fn new(transitions: Vec<EngineOutput>) -> Self {
        Self { transitions }
    }

    pub fn transitions(&self) -> &[EngineOutput] {
        &self.transitions
    }

    pub fn into_transitions(self) -> Vec<EngineOutput> {
        self.transitions
    }

    pub fn changes(&self) -> impl Iterator<Item = &ChangeSet> {
        self.transitions.iter().flat_map(EngineOutput::changes)
    }

    pub fn change_count(&self) -> usize {
        self.transitions
            .iter()
            .map(|output| output.changes().len())
            .sum()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActorDrainState {
    MoreAvailable,
    PendingPermits,
    Complete,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ActorDrainBatch {
    pub commands: Vec<ActorCommand>,
    pub state: ActorDrainState,
}

/// Closed actor input retained after failure or cancellation.
///
/// Tokio permits reserved before closure may still send. `PendingPermits`
/// therefore means only that no command is ready yet; it is not completion.
#[derive(Debug)]
pub struct ActorCommandDrain {
    prefix: Option<ActorCommand>,
    receiver: mpsc::Receiver<ActorCommand>,
}

impl ActorCommandDrain {
    fn new(prefix: Option<ActorCommand>, mut receiver: mpsc::Receiver<ActorCommand>) -> Self {
        receiver.close();
        Self { prefix, receiver }
    }

    pub fn drain_ready(&mut self, max_commands: usize) -> ActorDrainBatch {
        let mut commands = Vec::with_capacity(max_commands.min(usize::from(self.prefix.is_some())));
        while commands.len() < max_commands {
            if let Some(command) = self.prefix.take() {
                commands.push(command);
                continue;
            }
            match self.receiver.try_recv() {
                Ok(command) => commands.push(command),
                Err(mpsc::error::TryRecvError::Empty) => {
                    return ActorDrainBatch {
                        commands,
                        state: ActorDrainState::PendingPermits,
                    };
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    return ActorDrainBatch {
                        commands,
                        state: ActorDrainState::Complete,
                    };
                }
            }
        }
        ActorDrainBatch {
            commands,
            state: self.ready_state(),
        }
    }

    pub async fn recv(&mut self) -> Option<ActorCommand> {
        if let Some(command) = self.prefix.take() {
            return Some(command);
        }
        self.receiver.recv().await
    }

    fn ready_state(&mut self) -> ActorDrainState {
        if self.prefix.is_some() {
            return ActorDrainState::MoreAvailable;
        }
        match self.receiver.try_recv() {
            Ok(command) => {
                self.prefix = Some(command);
                ActorDrainState::MoreAvailable
            }
            Err(mpsc::error::TryRecvError::Empty) => ActorDrainState::PendingPermits,
            Err(mpsc::error::TryRecvError::Disconnected) => ActorDrainState::Complete,
        }
    }
}

#[derive(Debug)]
pub struct ActorCompletion {
    pub engine: StreamEngine,
    pub stats: ActorStats,
}

#[derive(Debug)]
pub struct ActorFailure {
    pub engine: StreamEngine,
    pub error: EngineError,
    /// Constituent transitions committed before the failing constituent.
    pub completed: Vec<EngineOutput>,
    pub pending: PendingInput,
    pub commands: ActorCommandDrain,
    pub stats: ActorStats,
}

#[derive(Debug)]
pub struct ActorCancellation {
    pub engine: StreamEngine,
    /// Results committed after the output receiver stopped accepting batches.
    pub unpublished: Option<ActorBatch>,
    pub pending: PendingInput,
    pub commands: ActorCommandDrain,
    pub stats: ActorStats,
}

#[derive(Debug)]
pub enum ActorExit {
    Completed(ActorCompletion),
    Failed(ActorFailure),
    Cancelled(ActorCancellation),
}

#[derive(Debug)]
pub struct ActorJoinOutcome {
    pub unread: Vec<ActorBatch>,
    pub exit: ActorExit,
}

#[derive(Debug)]
pub enum ActorJoinError {
    Task {
        unread: Vec<ActorBatch>,
        source: JoinError,
    },
    OutcomeTaken,
}

impl ActorJoinError {
    pub fn join_error(&self) -> Option<&JoinError> {
        match self {
            Self::Task { source, .. } => Some(source),
            Self::OutcomeTaken => None,
        }
    }

    pub fn unread(&self) -> &[ActorBatch] {
        match self {
            Self::Task { unread, .. } => unread,
            Self::OutcomeTaken => &[],
        }
    }
}

impl fmt::Display for ActorJoinError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Task { source, .. } => {
                write!(formatter, "stream-engine actor task failed: {source}")
            }
            Self::OutcomeTaken => {
                formatter.write_str("stream-engine actor outcome was already taken")
            }
        }
    }
}

impl Error for ActorJoinError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.join_error()
            .map(|source| source as &(dyn Error + 'static))
    }
}

/// Output and completion handle for a spawned stream-engine actor.
pub struct StreamEngineActor {
    output: mpsc::Receiver<ActorBatch>,
    task: Option<JoinHandle<ActorExit>>,
    unread: VecDeque<ActorBatch>,
}

impl StreamEngineActor {
    pub async fn recv(&mut self) -> Option<ActorBatch> {
        if let Some(batch) = self.unread.pop_front() {
            return Some(batch);
        }
        self.output.recv().await
    }

    pub fn is_finished(&self) -> bool {
        self.task.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Starts intentional cancellation without giving ownership to a future.
    pub fn begin_cancel(&mut self) {
        self.output.close();
    }

    /// Closes output intentionally and returns every remaining ownership plane.
    /// The borrowed operation can be cancelled and retried without losing the
    /// actor handle, already-drained output, or terminal ownership.
    pub async fn cancel(&mut self) -> Result<ActorJoinOutcome, ActorJoinError> {
        self.begin_cancel();
        self.join().await
    }

    /// Drains unread batches and returns the actor's engine and terminal state.
    /// The borrowed operation can be cancelled and retried.
    pub async fn join(&mut self) -> Result<ActorJoinOutcome, ActorJoinError> {
        if self.task.is_none() {
            return Err(ActorJoinError::OutcomeTaken);
        }
        while let Some(batch) = self.output.recv().await {
            self.unread.push_back(batch);
        }
        let result = self
            .task
            .as_mut()
            .expect("task presence checked before draining output")
            .await;
        self.task.take();
        let unread = std::mem::take(&mut self.unread).into_iter().collect();
        match result {
            Ok(exit) => Ok(ActorJoinOutcome { unread, exit }),
            Err(source) => Err(ActorJoinError::Task { unread, source }),
        }
    }
}

/// Spawns one ordered owner for a [`StreamEngine`].
///
/// Coalescing changes scheduling and publication granularity only. Canonical
/// appends execute over original non-empty constituent boundaries, so no joined
/// transition can erase recovery ownership.
pub fn spawn_stream_engine_actor(
    engine: StreamEngine,
    input: mpsc::Receiver<ActorCommand>,
    options: CoalesceOptions,
) -> StreamEngineActor {
    let (output, output_rx) = mpsc::channel(OUTPUT_CAPACITY);
    let task = tokio::spawn(run_actor(engine, ActorInbox::new(input, options), output));
    StreamEngineActor {
        output: output_rx,
        task: Some(task),
        unread: VecDeque::new(),
    }
}

async fn run_actor(
    mut engine: StreamEngine,
    mut inbox: ActorInbox,
    output: mpsc::Sender<ActorBatch>,
) -> ActorExit {
    let mut runtime = ActorRuntimeStats::default();
    loop {
        let action = tokio::select! {
            biased;
            _ = output.closed() => {
                return cancelled(engine, inbox, runtime, None);
            }
            action = inbox.next_action() => action,
        };

        match action {
            InboxAction::Flush => {
                match apply_pending(&mut engine, &mut inbox.pending, &mut runtime) {
                    Ok(batch) => {
                        if let Err(error) = publish(&output, batch, &mut runtime).await {
                            return cancelled(engine, inbox, runtime, Some(error.0));
                        }
                    }
                    Err(failure) => {
                        return failed(engine, inbox, runtime, failure.error, failure.completed);
                    }
                }
            }
            InboxAction::Barrier(command) => {
                let result = match command {
                    ActorCommand::Reset => engine.reset(),
                    ActorCommand::Finish => engine.finish(),
                    ActorCommand::Append(_) => {
                        unreachable!("append commands remain in pending input")
                    }
                };
                match result {
                    Ok(result) if result.is_empty() => {}
                    Ok(result) => {
                        runtime.record_committed(&result);
                        let batch = ActorBatch::new(vec![result]);
                        if let Err(error) = publish(&output, batch, &mut runtime).await {
                            return cancelled(engine, inbox, runtime, Some(error.0));
                        }
                    }
                    Err(error) => {
                        inbox.prepend_barrier(command);
                        return failed(engine, inbox, runtime, error, Vec::new());
                    }
                }
            }
            InboxAction::Closed => {
                if engine.lifecycle() == DocumentLifecycle::Open {
                    match engine.finish() {
                        Ok(result) if result.is_empty() => {}
                        Ok(result) => {
                            runtime.record_committed(&result);
                            let batch = ActorBatch::new(vec![result]);
                            if let Err(error) = publish(&output, batch, &mut runtime).await {
                                return cancelled(engine, inbox, runtime, Some(error.0));
                            }
                        }
                        Err(error) => {
                            return failed(engine, inbox, runtime, error, Vec::new());
                        }
                    }
                }
                let stats = runtime.snapshot(&inbox);
                return ActorExit::Completed(ActorCompletion { engine, stats });
            }
        }
    }
}

async fn publish(
    output: &mpsc::Sender<ActorBatch>,
    batch: ActorBatch,
    runtime: &mut ActorRuntimeStats,
) -> Result<(), mpsc::error::SendError<ActorBatch>> {
    let result_count = batch.transitions.len();
    output.send(batch).await?;
    runtime.published_results = runtime
        .published_results
        .saturating_add(u64::try_from(result_count).unwrap_or(u64::MAX));
    Ok(())
}

fn apply_pending(
    engine: &mut StreamEngine,
    pending: &mut PendingChunks,
    runtime: &mut ActorRuntimeStats,
) -> Result<ActorBatch, PendingFailure> {
    let mut completed = Vec::with_capacity(pending.constituents());
    while let Some(chunk) = pending.front() {
        runtime.append_attempts = runtime.append_attempts.saturating_add(1);
        match engine.append(chunk) {
            Ok(result) => {
                runtime.successful_appends = runtime.successful_appends.saturating_add(1);
                runtime.record_committed(&result);
                pending.commit_front();
                completed.push(result);
            }
            Err(error) => return Err(PendingFailure { error, completed }),
        }
    }
    Ok(ActorBatch::new(completed))
}

#[derive(Debug)]
struct PendingFailure {
    error: EngineError,
    completed: Vec<EngineOutput>,
}

fn failed(
    engine: StreamEngine,
    inbox: ActorInbox,
    runtime: ActorRuntimeStats,
    error: EngineError,
    completed: Vec<EngineOutput>,
) -> ActorExit {
    let stats = runtime.snapshot(&inbox);
    let (pending, commands) = inbox.into_recovery();
    ActorExit::Failed(ActorFailure {
        engine,
        error,
        completed,
        pending,
        commands,
        stats,
    })
}

fn cancelled(
    engine: StreamEngine,
    inbox: ActorInbox,
    runtime: ActorRuntimeStats,
    unpublished: Option<ActorBatch>,
) -> ActorExit {
    let stats = runtime.snapshot(&inbox);
    let (pending, commands) = inbox.into_recovery();
    ActorExit::Cancelled(ActorCancellation {
        engine,
        unpublished,
        pending,
        commands,
        stats,
    })
}

#[derive(Default)]
struct ActorRuntimeStats {
    append_attempts: u64,
    successful_appends: u64,
    committed_bytes: u64,
    published_results: u64,
}

impl ActorRuntimeStats {
    fn record_committed(&mut self, result: &EngineOutput) {
        let bytes = result
            .changes()
            .iter()
            .map(|change| change.source().suffix.len())
            .sum::<usize>();
        self.committed_bytes = self
            .committed_bytes
            .saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
    }

    fn snapshot(&self, inbox: &ActorInbox) -> ActorStats {
        let (pending_bytes, pending_constituents, boundary_metadata_bytes) = inbox.pending_facts();
        ActorStats {
            input_attempts: inbox.work.input_attempts,
            append_attempts: self.append_attempts,
            successful_appends: self.successful_appends,
            committed_bytes: self.committed_bytes,
            published_results: self.published_results,
            pending_bytes,
            pending_constituents,
            boundary_metadata_bytes,
            scan_bytes: inbox.work.scan_bytes,
            join_copy_bytes: inbox.work.join_copy_bytes,
            replay_count: 0,
        }
    }
}

enum InboxAction {
    Flush,
    Barrier(ActorCommand),
    Closed,
}

enum DeferredCommand {
    Append(ScannedChunk, Instant),
    Barrier(ActorCommand),
}

struct ActorInbox {
    receiver: mpsc::Receiver<ActorCommand>,
    options: CoalesceOptions,
    pending: PendingChunks,
    deferred: Option<DeferredCommand>,
    closed: bool,
    work: CoalesceWork,
}

impl ActorInbox {
    fn new(receiver: mpsc::Receiver<ActorCommand>, options: CoalesceOptions) -> Self {
        Self {
            receiver,
            options,
            pending: PendingChunks::default(),
            deferred: None,
            closed: false,
            work: CoalesceWork::default(),
        }
    }

    async fn next_action(&mut self) -> InboxAction {
        loop {
            if self.pending.flush_reason(self.options).is_some() {
                return InboxAction::Flush;
            }
            if self.pending.is_empty() {
                if let Some(command) = self.deferred.take() {
                    match command {
                        DeferredCommand::Append(chunk, arrived_at) => {
                            self.pending.accept(chunk, arrived_at);
                        }
                        DeferredCommand::Barrier(command) => {
                            return InboxAction::Barrier(command);
                        }
                    }
                    continue;
                }
                if self.closed {
                    self.pending.clear_empty_messages();
                    return InboxAction::Closed;
                }
                match self.receiver.recv().await {
                    Some(command) => self.accept(command),
                    None => self.closed = true,
                }
                continue;
            }

            let received = match self.pending.deadline(self.options) {
                Some(deadline) => tokio::time::timeout_at(deadline, self.receiver.recv()).await,
                None => Ok(self.receiver.recv().await),
            };
            match received {
                Ok(Some(ActorCommand::Append(text))) => {
                    let arrived_at = Instant::now();
                    let chunk = ScannedChunk::scan(text, &mut self.work);
                    if self.pending.overflow_reason(&chunk, self.options).is_some() {
                        debug_assert!(self.deferred.is_none());
                        self.deferred = Some(DeferredCommand::Append(chunk, arrived_at));
                        return InboxAction::Flush;
                    }
                    self.pending.accept(chunk, arrived_at);
                }
                Ok(Some(command)) => {
                    debug_assert!(self.deferred.is_none());
                    self.deferred = Some(DeferredCommand::Barrier(command));
                    return InboxAction::Flush;
                }
                Ok(None) => {
                    self.closed = true;
                    return InboxAction::Flush;
                }
                Err(_) => return InboxAction::Flush,
            }
        }
    }

    fn accept(&mut self, command: ActorCommand) {
        match command {
            ActorCommand::Append(text) => {
                let arrived_at = Instant::now();
                let chunk = ScannedChunk::scan(text, &mut self.work);
                self.pending.accept(chunk, arrived_at);
            }
            command => {
                debug_assert!(self.deferred.is_none());
                self.deferred = Some(DeferredCommand::Barrier(command));
            }
        }
    }

    fn prepend_barrier(&mut self, command: ActorCommand) {
        debug_assert!(self.deferred.is_none());
        self.deferred = Some(DeferredCommand::Barrier(command));
    }

    fn pending_facts(&self) -> (usize, usize, usize) {
        let facts = (
            self.pending.bytes(),
            self.pending.constituents(),
            self.pending.boundary_metadata_bytes(),
        );
        match self.deferred.as_ref() {
            Some(DeferredCommand::Append(chunk, _)) if !chunk.is_empty() => (
                facts.0.saturating_add(chunk.len()),
                facts.1.saturating_add(1),
                facts
                    .2
                    .saturating_add(std::mem::size_of::<DeferredCommand>()),
            ),
            _ => facts,
        }
    }

    fn into_recovery(self) -> (PendingInput, ActorCommandDrain) {
        let Self {
            receiver,
            pending,
            deferred,
            ..
        } = self;
        let prefix = deferred.map(|command| match command {
            DeferredCommand::Append(chunk, _) => ActorCommand::Append(chunk.into_text()),
            DeferredCommand::Barrier(command) => command,
        });
        (
            PendingInput::from_pending(pending),
            ActorCommandDrain::new(prefix, receiver),
        )
    }
}

#[cfg(test)]
mod value_gate {
    use mdstream_protocol::{ProtocolLimits, Snapshot, encode_change_json};
    use serde_json::Value;

    use super::*;

    const GOLDEN_AI_STREAM: &str = include_str!("../../examples/fixtures/golden-ai-stream.json");

    #[derive(Clone, Copy, Debug, Default)]
    struct CandidateMetrics {
        append_attempts: u64,
        encoded_result_bytes: u64,
        scan_bytes: u64,
        join_copy_bytes: u64,
        replay_count: u64,
    }

    #[test]
    fn constituent_first_wins_the_semantic_join_value_gate() {
        let workloads = [
            (
                "one-byte",
                "# Linear input\n\nOne byte at a time."
                    .chars()
                    .map(|character| character.to_string())
                    .collect::<Vec<_>>(),
            ),
            (
                "bursty",
                [
                    "# Bursty\n\n",
                    "A ",
                    "short burst",
                    " followed by ",
                    "another.\n",
                ]
                .into_iter()
                .map(str::to_string)
                .collect(),
            ),
            (
                "unicode",
                ["多", "语言", " 🙂", " cafe\u{301}", "\n"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            ),
            (
                "crlf",
                ["alpha\r", "\nbe", "ta\r", "gamma\r", "\n"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            ),
            ("golden-ai", golden_chunks()),
        ];

        for (name, chunks) in workloads {
            let (joined_snapshot, joined) = run_joined(&chunks);
            let (constituent_snapshot, constituent) = run_constituents(&chunks);
            assert_final_ir_eq(name, &joined_snapshot, &constituent_snapshot);
            assert_eq!(joined.scan_bytes, constituent.scan_bytes, "{name} scan");
            assert_eq!(joined.replay_count, 0, "{name} joined replay");
            assert_eq!(constituent.replay_count, 0, "{name} constituent replay");
            assert!(
                improves_by_quarter(joined.append_attempts, constituent.append_attempts)
                    || improves_by_quarter(
                        joined.encoded_result_bytes,
                        constituent.encoded_result_bytes,
                    ),
                "{name} must demonstrate the intended batching benefit"
            );
            assert!(
                within_twenty_percent(joined.append_attempts, constituent.append_attempts,)
                    && within_twenty_percent(
                        joined.encoded_result_bytes,
                        constituent.encoded_result_bytes,
                    )
                    && within_twenty_percent(joined.scan_bytes, constituent.scan_bytes),
                "{name} non-copy work must remain inside the regression budget"
            );
            assert!(
                !within_twenty_percent(joined.join_copy_bytes, constituent.join_copy_bytes,),
                "{name} joined copy work must fail the no-regression gate"
            );
            println!(
                "KTD3 {name}: joined attempts={} encoded={} scan={} copy={}; constituent attempts={} encoded={} scan={} copy={}; decision=constituent-first",
                joined.append_attempts,
                joined.encoded_result_bytes,
                joined.scan_bytes,
                joined.join_copy_bytes,
                constituent.append_attempts,
                constituent.encoded_result_bytes,
                constituent.scan_bytes,
                constituent.join_copy_bytes,
            );
        }
    }

    fn run_joined(chunks: &[String]) -> (Snapshot, CandidateMetrics) {
        let (mut pending, mut work) = pending(chunks);
        let (text, _) = pending.take_text(&mut work);
        let mut engine = StreamEngine::new();
        let mut metrics = CandidateMetrics {
            append_attempts: 1,
            scan_bytes: work.scan_bytes,
            join_copy_bytes: work.join_copy_bytes,
            ..CandidateMetrics::default()
        };
        record_output(&mut metrics, engine.append(&text).unwrap());
        record_output(&mut metrics, engine.finish().unwrap());
        (engine.snapshot().unwrap(), metrics)
    }

    fn run_constituents(chunks: &[String]) -> (Snapshot, CandidateMetrics) {
        let (mut pending, work) = pending(chunks);
        let mut engine = StreamEngine::new();
        let mut runtime = ActorRuntimeStats::default();
        let mut metrics = CandidateMetrics {
            scan_bytes: work.scan_bytes,
            join_copy_bytes: work.join_copy_bytes,
            ..CandidateMetrics::default()
        };
        let batch = apply_pending(&mut engine, &mut pending, &mut runtime)
            .expect("constituent candidate must succeed");
        metrics.append_attempts = runtime.append_attempts;
        for output in batch.into_transitions() {
            record_output(&mut metrics, output);
        }
        record_output(&mut metrics, engine.finish().unwrap());
        (engine.snapshot().unwrap(), metrics)
    }

    fn pending(chunks: &[String]) -> (PendingChunks, CoalesceWork) {
        let mut pending = PendingChunks::default();
        let mut work = CoalesceWork::default();
        let now = Instant::now();
        for chunk in chunks {
            pending.accept(ScannedChunk::scan(chunk.clone(), &mut work), now);
        }
        (pending, work)
    }

    fn record_output(metrics: &mut CandidateMetrics, output: EngineOutput) {
        for change in output.changes() {
            let encoded = encode_change_json(change, usize::MAX, ProtocolLimits::default())
                .expect("candidate output must encode");
            metrics.encoded_result_bytes = metrics
                .encoded_result_bytes
                .saturating_add(u64::try_from(encoded.len()).unwrap_or(u64::MAX));
        }
    }

    fn improves_by_quarter(candidate: u64, baseline: u64) -> bool {
        candidate.saturating_mul(4) <= baseline.saturating_mul(3)
    }

    fn assert_final_ir_eq(name: &str, left: &Snapshot, right: &Snapshot) {
        assert_eq!(left.source(), right.source(), "{name} source");
        assert_eq!(left.lifecycle(), right.lifecycle(), "{name} lifecycle");
        assert_eq!(
            left.projection_cursor(),
            right.projection_cursor(),
            "{name} projection cursor"
        );
        assert_eq!(left.roots(), right.roots(), "{name} roots");
        assert_eq!(left.nodes(), right.nodes(), "{name} nodes");
        assert_eq!(left.resources(), right.resources(), "{name} resources");
    }

    fn within_twenty_percent(candidate: u64, baseline: u64) -> bool {
        candidate.saturating_mul(5) <= baseline.saturating_mul(6)
    }

    fn golden_chunks() -> Vec<String> {
        let scenario: Value = serde_json::from_str(GOLDEN_AI_STREAM).unwrap();
        scenario["episodes"]["mainline"]["actions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|action| action["kind"] == "append")
            .map(|action| action["chunk"].as_str().unwrap().to_string())
            .collect()
    }
}
