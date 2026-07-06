use tokio::sync::mpsc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackpressurePolicy {
    /// Await capacity. Never drops.
    ///
    /// Recommended when:
    /// - you need reliable delivery (no content loss)
    /// - your producer can tolerate waiting (e.g. network stream on a background task)
    ///
    /// Trade-off: the producer task may stall when the UI falls behind.
    Block,
    /// Drop the new delta when the channel is full.
    ///
    /// Recommended when:
    /// - deltas are replaceable / "best effort" (typing indicators, progress, ephemeral status)
    /// - you prefer keeping the UI responsive over preserving every update
    ///
    /// Trade-off: content loss is expected when the UI is slow.
    DropNew,
    /// Buffer locally and try to flush opportunistically (keeps content, reduces producer stalls).
    ///
    /// This is useful when producers are very "bursty" and you prefer UI smoothness over strict
    /// per-token delivery. It combines well with a receiver-side coalescer.
    ///
    /// Recommended when:
    /// - deltas are very high-frequency (LLM token streams)
    /// - you still want to preserve content, but avoid stalling producers on every small chunk
    ///
    /// Trade-off: memory is bounded by `local_max_bytes`; flushing becomes "chunky" under load.
    CoalesceLocal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendOutcome {
    Sent,
    Dropped,
    Buffered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    Closed,
}

/// Producer-side helper for bounded channels.
///
/// In many streaming setups, the producer runs in an async task and the UI thread drains updates.
/// This wrapper provides a few practical backpressure strategies without forcing users to build
/// their own channel policies.
pub struct DeltaSender {
    pub(crate) tx: mpsc::Sender<String>,
    policy: BackpressurePolicy,
    local_buf: String,
    local_max_bytes: usize,
}

impl DeltaSender {
    pub fn new(tx: mpsc::Sender<String>, policy: BackpressurePolicy) -> Self {
        Self {
            tx,
            policy,
            local_buf: String::new(),
            local_max_bytes: 16 * 1024,
        }
    }

    pub fn set_local_max_bytes(&mut self, max: usize) {
        self.local_max_bytes = max.max(1);
    }

    pub fn policy(&self) -> BackpressurePolicy {
        self.policy
    }

    pub fn set_policy(&mut self, policy: BackpressurePolicy) {
        self.policy = policy;
    }

    pub async fn send(&mut self, delta: &str) -> Result<SendOutcome, SendError> {
        match self.policy {
            BackpressurePolicy::Block => self.send_block(delta).await,
            BackpressurePolicy::DropNew => self.send_drop_new(delta),
            BackpressurePolicy::CoalesceLocal => self.send_coalesce_local(delta).await,
        }
    }

    pub async fn flush(&mut self) -> Result<SendOutcome, SendError> {
        if self.local_buf.is_empty() {
            return Ok(SendOutcome::Sent);
        }
        let buf = std::mem::take(&mut self.local_buf);
        self.tx.send(buf).await.map_err(|_| SendError::Closed)?;
        Ok(SendOutcome::Sent)
    }

    async fn send_block(&mut self, delta: &str) -> Result<SendOutcome, SendError> {
        self.tx
            .send(delta.to_string())
            .await
            .map_err(|_| SendError::Closed)?;
        Ok(SendOutcome::Sent)
    }

    fn send_drop_new(&mut self, delta: &str) -> Result<SendOutcome, SendError> {
        match self.tx.try_send(delta.to_string()) {
            Ok(()) => Ok(SendOutcome::Sent),
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => Ok(SendOutcome::Dropped),
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => Err(SendError::Closed),
        }
    }

    async fn send_coalesce_local(&mut self, delta: &str) -> Result<SendOutcome, SendError> {
        self.local_buf.push_str(delta);

        let should_try_flush =
            self.local_buf.len() >= self.local_max_bytes || self.local_buf.contains('\n');

        if should_try_flush {
            match self.tx.try_send(std::mem::take(&mut self.local_buf)) {
                Ok(()) => return Ok(SendOutcome::Sent),
                Err(tokio::sync::mpsc::error::TrySendError::Full(s)) => {
                    self.local_buf = s;
                    return Ok(SendOutcome::Buffered);
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    return Err(SendError::Closed);
                }
            }
        }

        Ok(SendOutcome::Buffered)
    }
}
