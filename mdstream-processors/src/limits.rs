#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessorLimits {
    /// Maximum deterministic logical bytes in one owned processor input.
    ///
    /// This covers the canonical node representation, direct child identities,
    /// body bytes, directly referenced resource, and cached input version. It
    /// is not an allocator-capacity or processor peak-memory measurement.
    pub max_input_bytes: usize,
    /// Maximum logical bytes in one retained artifact envelope and payload.
    pub max_artifact_bytes: usize,
    /// Maximum number of issued jobs whose leases have not been settled.
    pub max_in_flight_jobs: usize,
    /// Maximum aggregate logical input bytes held by unsettled job leases.
    pub max_in_flight_input_bytes: usize,
    /// Maximum number of stable epoch/node/processor slots.
    pub max_slots: usize,
    /// Maximum number of artifacts retained across all ready slots.
    pub max_retained_artifacts: usize,
    /// Maximum aggregate logical bytes retained by ready artifacts.
    pub max_retained_artifact_bytes: usize,
    /// Maximum UTF-8 bytes retained in any structured failure message.
    ///
    /// The failure code remains available when this is zero. Fixed host
    /// diagnostics and processor-provided messages are normalized to this cap
    /// before entering slot state.
    pub max_error_bytes: usize,
}

impl Default for ProcessorLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 1024 * 1024,
            max_artifact_bytes: 4 * 1024 * 1024,
            max_in_flight_jobs: 32,
            max_in_flight_input_bytes: 8 * 1024 * 1024,
            max_slots: 256,
            max_retained_artifacts: 128,
            max_retained_artifact_bytes: 32 * 1024 * 1024,
            max_error_bytes: 4 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProcessorMetrics {
    pub slots: usize,
    pub in_flight_jobs: usize,
    pub in_flight_input_bytes: usize,
    pub retained_artifacts: usize,
    pub retained_artifact_bytes: usize,
    pub issued_requests: u64,
    pub accepted_results: u64,
    pub stale_results: u64,
    pub released_artifacts: u64,
}
