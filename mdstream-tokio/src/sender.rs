use tokio::sync::mpsc;

/// Lossless backpressure strategies for canonical document input.
///
/// Lossy policies are deliberately absent from this API:
///
/// ```compile_fail
/// use mdstream_tokio::BackpressurePolicy;
///
/// let _ = BackpressurePolicy::DropNew;
/// ```
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
    /// Buffer locally and try to flush opportunistically (keeps content, reduces producer stalls).
    ///
    /// This is useful when producers are very "bursty" and you prefer UI smoothness over strict
    /// per-token delivery. It combines well with a receiver-side coalescer.
    ///
    /// Recommended when:
    /// - deltas are very high-frequency (LLM token streams)
    /// - you still want to preserve content, but avoid stalling producers on every small chunk
    ///
    /// Trade-off: backpressure begins at `local_max_bytes`; one input delta may
    /// itself be larger than that threshold, and flushing becomes chunky under load.
    CoalesceLocal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendOutcome {
    Sent,
    /// Accepted into the sender's local buffer, but not yet admitted to the channel.
    ///
    /// Call [`DeltaSender::flush`] before dropping the sender. `Drop` cannot
    /// asynchronously deliver buffered content.
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
///
/// A [`SendOutcome::Buffered`] result is retained locally. Producers must call
/// [`Self::flush`] before dropping the sender to complete delivery.
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

    /// Change policy without allowing buffered content to be overtaken.
    pub async fn set_policy(&mut self, policy: BackpressurePolicy) -> Result<(), SendError> {
        if self.policy == policy {
            return Ok(());
        }
        self.flush().await?;
        self.policy = policy;
        Ok(())
    }

    pub async fn send(&mut self, delta: &str) -> Result<SendOutcome, SendError> {
        match self.policy {
            BackpressurePolicy::Block => self.send_block(delta).await,
            BackpressurePolicy::CoalesceLocal => self.send_coalesce_local(delta).await,
        }
    }

    pub async fn flush(&mut self) -> Result<SendOutcome, SendError> {
        if self.local_buf.is_empty() {
            return Ok(SendOutcome::Sent);
        }
        let permit = self.tx.reserve().await.map_err(|_| SendError::Closed)?;
        permit.send(std::mem::take(&mut self.local_buf));
        Ok(SendOutcome::Sent)
    }

    async fn send_block(&mut self, delta: &str) -> Result<SendOutcome, SendError> {
        let permit = self.tx.reserve().await.map_err(|_| SendError::Closed)?;
        permit.send(delta.to_string());
        Ok(SendOutcome::Sent)
    }

    async fn send_coalesce_local(&mut self, delta: &str) -> Result<SendOutcome, SendError> {
        self.local_buf.push_str(delta);

        let should_try_flush =
            self.local_buf.len() >= self.local_max_bytes || self.local_buf.contains('\n');

        if should_try_flush {
            match self.tx.try_reserve() {
                Ok(permit) => {
                    permit.send(std::mem::take(&mut self.local_buf));
                    return Ok(SendOutcome::Sent);
                }
                Err(tokio::sync::mpsc::error::TrySendError::Full(())) => {
                    if self.local_buf.len() < self.local_max_bytes {
                        return Ok(SendOutcome::Buffered);
                    }

                    let permit = self.tx.reserve().await.map_err(|_| SendError::Closed)?;
                    permit.send(std::mem::take(&mut self.local_buf));
                    return Ok(SendOutcome::Sent);
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(())) => {
                    return Err(SendError::Closed);
                }
            }
        }

        Ok(SendOutcome::Buffered)
    }
}
