use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoalesceOptions {
    flush_on_newline: bool,
    max_delay: Duration,
    max_bytes: usize,
    max_pending_chunks: usize,
}

impl CoalesceOptions {
    /// Creates a bounded coalescing policy.
    ///
    /// Zero byte and constituent limits are normalized to one so every policy
    /// can accept a standalone non-empty chunk before applying backpressure.
    pub fn new(max_delay: Duration, max_bytes: usize, max_pending_chunks: usize) -> Self {
        Self {
            flush_on_newline: true,
            max_delay,
            max_bytes: max_bytes.max(1),
            max_pending_chunks: max_pending_chunks.max(1),
        }
    }

    pub const fn flush_on_newline(self) -> bool {
        self.flush_on_newline
    }

    pub const fn max_delay(self) -> Duration {
        self.max_delay
    }

    pub const fn max_bytes(self) -> usize {
        self.max_bytes
    }

    pub const fn max_pending_chunks(self) -> usize {
        self.max_pending_chunks
    }

    pub const fn with_newline_flush(mut self, enabled: bool) -> Self {
        self.flush_on_newline = enabled;
        self
    }

    pub fn with_max_delay(mut self, max_delay: Duration) -> Self {
        self.max_delay = max_delay;
        self
    }

    pub fn with_max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes.max(1);
        self
    }

    pub fn with_max_pending_chunks(mut self, max_pending_chunks: usize) -> Self {
        self.max_pending_chunks = max_pending_chunks.max(1);
        self
    }
}

impl Default for CoalesceOptions {
    fn default() -> Self {
        Self::new(Duration::from_millis(60), 8 * 1024, 1024)
    }
}
