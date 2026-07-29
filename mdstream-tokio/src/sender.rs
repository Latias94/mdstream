use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::CoalesceStats;
use crate::coalesce::{PendingChunks, PendingInput, ScannedChunk, scan_newline};
use crate::stats::CoalesceWork;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackpressurePolicy {
    Block,
    CoalesceLocal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendOutcome {
    Sent,
    /// Accepted into local pending state but not yet admitted to the channel.
    Buffered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    Closed,
}

/// Producer-side bounded-channel helper with explicit lossless pending state.
pub struct DeltaSender {
    tx: mpsc::Sender<String>,
    policy: BackpressurePolicy,
    pending: PendingChunks,
    work: CoalesceWork,
    local_max_bytes: usize,
    local_max_chunks: usize,
}

impl DeltaSender {
    pub fn new(
        tx: mpsc::Sender<String>,
        policy: BackpressurePolicy,
        local_max_bytes: usize,
        local_max_chunks: usize,
    ) -> Self {
        Self {
            tx,
            policy,
            pending: PendingChunks::default(),
            work: CoalesceWork::default(),
            local_max_bytes: local_max_bytes.max(1),
            local_max_chunks: local_max_chunks.max(1),
        }
    }

    pub fn policy(&self) -> BackpressurePolicy {
        self.policy
    }

    pub fn stats(&self) -> CoalesceStats {
        self.work.snapshot(
            self.pending.bytes(),
            self.pending.constituents(),
            self.pending.boundary_metadata_bytes(),
        )
    }

    /// Transfers every accepted local constituent back to the caller.
    pub fn take_pending(&mut self) -> PendingInput {
        PendingInput::from_pending(std::mem::take(&mut self.pending))
    }

    pub async fn set_policy(&mut self, policy: BackpressurePolicy) -> Result<(), SendError> {
        if self.policy == policy {
            return Ok(());
        }
        self.flush().await?;
        self.policy = policy;
        Ok(())
    }

    /// Sends borrowed canonical input. Until this method returns `Buffered` or
    /// `Sent`, cancellation leaves the new delta with the caller.
    pub async fn send(&mut self, delta: &str) -> Result<SendOutcome, SendError> {
        match self.policy {
            BackpressurePolicy::Block => self.send_block(delta).await,
            BackpressurePolicy::CoalesceLocal => self.send_coalesced(delta).await,
        }
    }

    pub async fn flush(&mut self) -> Result<SendOutcome, SendError> {
        if self.pending.is_empty() {
            self.pending.clear_empty_messages();
            return Ok(SendOutcome::Sent);
        }
        let permit = self.tx.reserve().await.map_err(|_| SendError::Closed)?;
        let (text, messages) = self.pending.take_text(&mut self.work);
        self.work.record_output(text.len(), messages, None);
        permit.send(text);
        Ok(SendOutcome::Sent)
    }

    async fn send_block(&mut self, delta: &str) -> Result<SendOutcome, SendError> {
        self.work.record_input(0);
        self.send_direct(delta).await
    }

    async fn send_coalesced(&mut self, delta: &str) -> Result<SendOutcome, SendError> {
        let (has_newline, scanned_bytes) = scan_newline(delta.as_bytes());
        self.work.record_input(scanned_bytes);

        let reaches_byte_limit = !self.pending.is_empty()
            && self.pending.bytes().saturating_add(delta.len()) >= self.local_max_bytes;
        let exceeds_chunk_limit = !delta.is_empty()
            && !self.pending.is_empty()
            && self.pending.constituents().saturating_add(1) > self.local_max_chunks;
        if reaches_byte_limit || exceeds_chunk_limit {
            self.flush().await?;
        }

        if delta.len() >= self.local_max_bytes {
            return self.send_direct(delta).await;
        }

        if has_newline {
            match self.tx.try_reserve() {
                Ok(permit) => {
                    let chunk = ScannedChunk::scan_without_recording(delta.to_string(), true);
                    self.pending.accept(chunk, Instant::now());
                    let (text, messages) = self.pending.take_text(&mut self.work);
                    self.work.record_output(text.len(), messages, None);
                    permit.send(text);
                    return Ok(SendOutcome::Sent);
                }
                Err(mpsc::error::TrySendError::Full(())) => {}
                Err(mpsc::error::TrySendError::Closed(())) => return Err(SendError::Closed),
            }
        } else if self.tx.is_closed() {
            return Err(SendError::Closed);
        }

        let chunk = ScannedChunk::scan_without_recording(delta.to_string(), has_newline);
        self.pending.accept(chunk, Instant::now());
        Ok(SendOutcome::Buffered)
    }

    async fn send_direct(&mut self, delta: &str) -> Result<SendOutcome, SendError> {
        let permit = self.tx.reserve().await.map_err(|_| SendError::Closed)?;
        let text = delta.to_string();
        self.work.record_output(text.len(), 1, None);
        permit.send(text);
        Ok(SendOutcome::Sent)
    }
}
