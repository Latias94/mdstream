use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::coalesce::{PendingChunks, ScannedChunk};
use crate::stats::CoalesceWork;
use crate::{CoalesceOptions, CoalesceStats, FlushReason};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoalescedChunk {
    pub text: String,
    pub reason: FlushReason,
    /// Number of channel messages represented by this output. Empty messages
    /// count here but never consume a constituent boundary.
    pub merged_messages: usize,
}

/// A cancellation-safe receiver that merges owned deltas with bounded metadata.
pub struct CoalescingReceiver {
    rx: mpsc::Receiver<String>,
    options: CoalesceOptions,
    pending: PendingChunks,
    lookahead: Option<LookaheadChunk>,
    work: CoalesceWork,
}

struct LookaheadChunk {
    chunk: ScannedChunk,
    arrived_at: Instant,
}

impl CoalescingReceiver {
    pub fn new(rx: mpsc::Receiver<String>, options: CoalesceOptions) -> Self {
        Self {
            rx,
            options,
            pending: PendingChunks::default(),
            lookahead: None,
            work: CoalesceWork::default(),
        }
    }

    /// Applies a new policy without changing the first pending input's origin.
    /// Cached byte, constituent, and newline facts are reused without rescans.
    pub fn set_options(&mut self, options: CoalesceOptions) {
        self.options = options;
    }

    pub fn options(&self) -> CoalesceOptions {
        self.options
    }

    pub fn stats(&self) -> CoalesceStats {
        let lookahead_bytes = self
            .lookahead
            .as_ref()
            .map_or(0, |lookahead| lookahead.chunk.len());
        let lookahead_constituents = usize::from(
            self.lookahead
                .as_ref()
                .is_some_and(|lookahead| !lookahead.chunk.is_empty()),
        );
        let lookahead_metadata_bytes =
            lookahead_constituents.saturating_mul(std::mem::size_of::<LookaheadChunk>());
        self.work.snapshot(
            self.pending.bytes().saturating_add(lookahead_bytes),
            self.pending
                .constituents()
                .saturating_add(lookahead_constituents),
            self.pending
                .boundary_metadata_bytes()
                .saturating_add(lookahead_metadata_bytes),
        )
    }

    pub async fn recv(&mut self) -> Option<String> {
        self.recv_with_meta().await.map(|chunk| chunk.text)
    }

    pub async fn recv_with_meta(&mut self) -> Option<CoalescedChunk> {
        loop {
            if let Some(reason) = self.pending.flush_reason(self.options) {
                return Some(self.flush(reason));
            }

            if self.pending.is_empty() {
                if let Some(lookahead) = self.lookahead.take() {
                    self.pending.accept(lookahead.chunk, lookahead.arrived_at);
                    continue;
                }
                match self.rx.recv().await {
                    Some(text) => {
                        let arrived_at = Instant::now();
                        let chunk = ScannedChunk::scan(text, &mut self.work);
                        self.pending.accept(chunk, arrived_at);
                    }
                    None => {
                        self.pending.clear_empty_messages();
                        return None;
                    }
                }
                continue;
            }

            let received = match self.pending.deadline(self.options) {
                Some(deadline) => tokio::time::timeout_at(deadline, self.rx.recv()).await,
                None => Ok(self.rx.recv().await),
            };
            match received {
                Ok(Some(text)) => {
                    let arrived_at = Instant::now();
                    let chunk = ScannedChunk::scan(text, &mut self.work);
                    if let Some(reason) = self.pending.overflow_reason(&chunk, self.options) {
                        self.lookahead = Some(LookaheadChunk { chunk, arrived_at });
                        return Some(self.flush(reason));
                    }
                    self.pending.accept(chunk, arrived_at);
                }
                Ok(None) => return Some(self.flush(FlushReason::ChannelClosed)),
                Err(_) => return Some(self.flush(FlushReason::MaxDelay)),
            }
        }
    }

    fn flush(&mut self, reason: FlushReason) -> CoalescedChunk {
        let (text, merged_messages) = self.pending.take_text(&mut self.work);
        self.work
            .record_output(text.len(), merged_messages, Some(reason));
        CoalescedChunk {
            text,
            reason,
            merged_messages,
        }
    }
}
