#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlushReason {
    Newline,
    MaxDelay,
    MaxBytes,
    MaxPendingChunks,
    ChannelClosed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CoalesceStats {
    pub input_attempts: u64,
    pub output_chunks: u64,
    pub output_bytes: u64,
    /// Logical bytes inspected while searching each input for its first
    /// newline. Bytes after that newline are not inspected or counted.
    pub scan_bytes: u64,
    pub join_copy_bytes: u64,
    pub pending_bytes: usize,
    pub pending_constituents: usize,
    /// Logical bytes occupied by live per-constituent records. Allocator spare
    /// capacity is excluded so this value remains deterministic.
    pub boundary_metadata_bytes: usize,
    pub last_reason: Option<FlushReason>,
    pub last_merged_messages: usize,
    pub last_bytes: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActorStats {
    pub input_attempts: u64,
    pub append_attempts: u64,
    pub successful_appends: u64,
    pub committed_bytes: u64,
    pub published_results: u64,
    pub pending_bytes: usize,
    pub pending_constituents: usize,
    /// Logical bytes occupied by live per-constituent records. Allocator spare
    /// capacity is excluded so this value remains deterministic.
    pub boundary_metadata_bytes: usize,
    /// Logical bytes inspected while searching each input for its first
    /// newline. Bytes after that newline are not inspected or counted.
    pub scan_bytes: u64,
    pub join_copy_bytes: u64,
    pub replay_count: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CoalesceWork {
    pub(crate) input_attempts: u64,
    pub(crate) output_chunks: u64,
    pub(crate) output_bytes: u64,
    pub(crate) scan_bytes: u64,
    pub(crate) join_copy_bytes: u64,
    pub(crate) last_reason: Option<FlushReason>,
    pub(crate) last_merged_messages: usize,
    pub(crate) last_bytes: usize,
}

impl CoalesceWork {
    pub(crate) fn record_input(&mut self, scanned_bytes: usize) {
        self.input_attempts = self.input_attempts.saturating_add(1);
        self.scan_bytes = self
            .scan_bytes
            .saturating_add(u64::try_from(scanned_bytes).unwrap_or(u64::MAX));
    }

    pub(crate) fn record_join_copy(&mut self, bytes: usize) {
        self.join_copy_bytes = self
            .join_copy_bytes
            .saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
    }

    pub(crate) fn record_output(
        &mut self,
        bytes: usize,
        messages: usize,
        reason: Option<FlushReason>,
    ) {
        self.output_chunks = self.output_chunks.saturating_add(1);
        self.output_bytes = self
            .output_bytes
            .saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
        self.last_reason = reason;
        self.last_merged_messages = messages;
        self.last_bytes = bytes;
    }

    pub(crate) fn snapshot(
        self,
        pending_bytes: usize,
        pending_constituents: usize,
        boundary_metadata_bytes: usize,
    ) -> CoalesceStats {
        CoalesceStats {
            input_attempts: self.input_attempts,
            output_chunks: self.output_chunks,
            output_bytes: self.output_bytes,
            scan_bytes: self.scan_bytes,
            join_copy_bytes: self.join_copy_bytes,
            pending_bytes,
            pending_constituents,
            boundary_metadata_bytes,
            last_reason: self.last_reason,
            last_merged_messages: self.last_merged_messages,
            last_bytes: self.last_bytes,
        }
    }
}
