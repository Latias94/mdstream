use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::CoalesceOptions;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlushReason {
    Newline,
    MaxDelay,
    MaxBytes,
    ChannelClosed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoalescedChunk {
    pub text: String,
    pub reason: FlushReason,
    /// Number of input messages merged into this output chunk.
    pub merged_messages: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CoalesceStats {
    pub total_in_messages: u64,
    pub total_out_chunks: u64,
    pub total_out_bytes: u64,
    pub last_reason: Option<FlushReason>,
    pub last_merged_messages: usize,
    pub last_bytes: usize,
}

/// A receiver wrapper that merges high-frequency deltas into fewer, larger chunks.
pub struct CoalescingReceiver {
    rx: mpsc::Receiver<String>,
    opts: CoalesceOptions,
    buf: String,
    deadline: Option<Instant>,
    stats: CoalesceStats,
}

impl CoalescingReceiver {
    pub fn new(rx: mpsc::Receiver<String>, opts: CoalesceOptions) -> Self {
        Self {
            rx,
            opts,
            buf: String::new(),
            deadline: None,
            stats: CoalesceStats::default(),
        }
    }

    pub fn set_options(&mut self, opts: CoalesceOptions) {
        self.opts = opts;
        // Keep any buffered text; refresh the deadline based on the new policy.
        if !self.buf.is_empty() {
            self.deadline = Some(Instant::now() + self.opts.max_delay);
        }
    }

    pub fn options(&self) -> CoalesceOptions {
        self.opts
    }

    pub fn stats(&self) -> CoalesceStats {
        self.stats
    }

    /// Receive the next coalesced chunk.
    ///
    /// - Returns `None` when the underlying channel is closed and the internal buffer is empty.
    /// - Returns a final buffered chunk before finishing, if any.
    pub async fn recv(&mut self) -> Option<String> {
        self.recv_with_meta().await.map(|c| c.text)
    }

    pub async fn recv_with_meta(&mut self) -> Option<CoalescedChunk> {
        let mut merged_messages = 0usize;

        if self.buf.is_empty() {
            let first = self.rx.recv().await?;
            self.buf.push_str(&first);
            merged_messages += 1;
            self.deadline = Some(Instant::now() + self.opts.max_delay);
        }

        loop {
            if let Some(reason) = self.should_flush_reason() {
                return Some(self.flush_buffer(reason, merged_messages));
            }

            let Some(deadline) = self.deadline else {
                self.deadline = Some(Instant::now() + self.opts.max_delay);
                continue;
            };

            let next = tokio::time::timeout_at(deadline, self.rx.recv()).await;
            match next {
                Ok(Some(s)) => {
                    self.buf.push_str(&s);
                    merged_messages += 1;
                }
                Ok(None) => {
                    // Channel closed: flush remaining buffer once.
                    if self.buf.is_empty() {
                        return None;
                    }
                    return Some(self.flush_buffer(FlushReason::ChannelClosed, merged_messages));
                }
                Err(_) => {
                    // Timeout: flush for progress.
                    return Some(self.flush_buffer(FlushReason::MaxDelay, merged_messages));
                }
            }
        }
    }

    fn should_flush_reason(&self) -> Option<FlushReason> {
        if self.buf.len() >= self.opts.max_bytes {
            return Some(FlushReason::MaxBytes);
        }
        if self.opts.flush_on_newline && self.buf.contains('\n') {
            return Some(FlushReason::Newline);
        }
        None
    }

    fn flush_buffer(&mut self, reason: FlushReason, merged_messages: usize) -> CoalescedChunk {
        let text = self.take_buf();
        self.stats.total_in_messages = self
            .stats
            .total_in_messages
            .saturating_add(merged_messages as u64);
        self.stats.total_out_chunks = self.stats.total_out_chunks.saturating_add(1);
        self.stats.total_out_bytes = self.stats.total_out_bytes.saturating_add(text.len() as u64);
        self.stats.last_reason = Some(reason);
        self.stats.last_merged_messages = merged_messages;
        self.stats.last_bytes = text.len();
        CoalescedChunk {
            text,
            reason,
            merged_messages,
        }
    }

    fn take_buf(&mut self) -> String {
        self.deadline = None;
        std::mem::take(&mut self.buf)
    }
}
