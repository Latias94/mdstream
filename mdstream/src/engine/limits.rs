#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Engine-owned limits for one staged source/projection transition.
///
/// Canonical document, parser, and operation limits remain in
/// `mdstream_protocol::ProtocolLimits`. These limits bound the additional
/// transaction and change-set plane owned by [`crate::StreamEngine`].
pub struct EngineLimits {
    /// Maximum deterministic logical bytes retained by one emitted change.
    pub max_change_bytes: usize,
    /// Maximum deterministic logical bytes live while staging one transition.
    pub max_transaction_bytes: usize,
}

impl Default for EngineLimits {
    fn default() -> Self {
        Self {
            max_change_bytes: 64 * 1024 * 1024,
            max_transaction_bytes: 128 * 1024 * 1024,
        }
    }
}
