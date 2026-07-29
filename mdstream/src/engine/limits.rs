#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Engine-owned limits for one staged source/projection transition.
///
/// Canonical document and operation limits remain in
/// `mdstream_protocol::ProtocolLimits`, while parser work is bounded by
/// [`crate::CompilerLimits`]. These limits bound the additional transaction
/// and change-set plane owned by [`crate::StreamEngine`].
pub struct EngineLimits {
    /// Maximum deterministic logical bytes retained by one emitted change.
    pub max_change_bytes: usize,
    /// Maximum deterministic logical bytes live while staging one transition.
    pub max_transaction_bytes: usize,
}

impl EngineLimits {
    /// Returns the minimum JSON output budget that safely covers any change
    /// admitted by `max_change_bytes`.
    ///
    /// The factor covers worst-case JSON string escaping together with the
    /// structural and envelope accounting included in the logical change
    /// budget. Binding facades must reject configurations where this bound
    /// cannot be represented or provided before accepting input.
    pub fn minimum_encoded_change_bytes(self) -> Option<usize> {
        self.max_change_bytes.checked_mul(6)
    }
}

impl Default for EngineLimits {
    fn default() -> Self {
        Self {
            max_change_bytes: 64 * 1024 * 1024,
            max_transaction_bytes: 128 * 1024 * 1024,
        }
    }
}
