use std::fmt;

use crate::{HostError, result::MAX_REMOVED_CHANGE_BYTES};

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
    /// Must be at least one.
    pub max_in_flight_jobs: usize,
    /// Maximum aggregate logical input bytes held by unsettled job leases.
    pub max_in_flight_input_bytes: usize,
    /// Maximum number of stable epoch/node/processor slots. Must be at least one.
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
    /// Maximum number of undrained derived-state change records.
    ///
    /// This must be at least [`Self::max_slots`] so every slot can emit a
    /// cleanup record during an epoch reset.
    pub max_pending_changes: usize,
    /// Maximum deterministic logical bytes in undrained change records.
    ///
    /// This must accommodate one worst-case removal record per slot so
    /// cleanup can complete without exceeding the queue budget.
    pub max_pending_change_bytes: usize,
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
            max_pending_changes: 1024,
            max_pending_change_bytes: 1024 * 1024,
        }
    }
}

impl ProcessorLimits {
    /// Validates that the change queue can record cleanup for every slot.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessorLimitsError`] when no job or slot can be admitted,
    /// either queue budget is too small, or required byte arithmetic overflows.
    pub fn validate(&self) -> Result<(), ProcessorLimitsError> {
        if self.max_in_flight_jobs == 0 {
            return Err(ProcessorLimitsError::InFlightJobsTooSmall);
        }
        if self.max_slots == 0 {
            return Err(ProcessorLimitsError::SlotsTooSmall);
        }
        if self.max_pending_changes < self.max_slots {
            return Err(ProcessorLimitsError::PendingChangesTooSmall {
                required: self.max_slots,
                actual: self.max_pending_changes,
            });
        }
        let required_change_bytes = self.max_slots.checked_mul(MAX_REMOVED_CHANGE_BYTES).ok_or(
            ProcessorLimitsError::PendingChangeBytesOverflow {
                max_slots: self.max_slots,
                bytes_per_slot: MAX_REMOVED_CHANGE_BYTES,
            },
        )?;
        if self.max_pending_change_bytes < required_change_bytes {
            return Err(ProcessorLimitsError::PendingChangeBytesTooSmall {
                required: required_change_bytes,
                actual: self.max_pending_change_bytes,
            });
        }
        Ok(())
    }
}

/// Describes a processor limit configuration that cannot guarantee cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessorLimitsError {
    /// At least one processor job must be admissible.
    InFlightJobsTooSmall,
    /// At least one processor slot must be admissible.
    SlotsTooSmall,
    /// The change queue cannot hold one cleanup record per slot.
    PendingChangesTooSmall { required: usize, actual: usize },
    /// The change queue byte budget cannot hold worst-case cleanup records.
    PendingChangeBytesTooSmall { required: usize, actual: usize },
    /// The required cleanup byte budget cannot be represented by `usize`.
    PendingChangeBytesOverflow {
        max_slots: usize,
        bytes_per_slot: usize,
    },
}

impl fmt::Display for ProcessorLimitsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InFlightJobsTooSmall => {
                formatter.write_str("processor.max_in_flight_jobs must be at least 1")
            }
            Self::SlotsTooSmall => formatter.write_str("processor.max_slots must be at least 1"),
            Self::PendingChangesTooSmall { required, actual } => write!(
                formatter,
                "processor.pending_changes must be at least {required}, found {actual}"
            ),
            Self::PendingChangeBytesTooSmall { required, actual } => write!(
                formatter,
                "processor.pending_change_bytes must be at least {required}, found {actual}"
            ),
            Self::PendingChangeBytesOverflow {
                max_slots,
                bytes_per_slot,
            } => write!(
                formatter,
                "processor pending change cleanup capacity overflows for {max_slots} slots at {bytes_per_slot} bytes per slot"
            ),
        }
    }
}

impl std::error::Error for ProcessorLimitsError {}

pub(crate) fn check_limit(
    field: &'static str,
    limit: usize,
    actual: usize,
) -> Result<(), HostError> {
    if actual > limit {
        Err(HostError::LimitExceeded {
            field,
            limit,
            actual,
        })
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProcessorMetrics {
    pub slots: usize,
    pub in_flight_jobs: usize,
    pub in_flight_input_bytes: usize,
    pub retained_artifacts: usize,
    pub retained_artifact_bytes: usize,
    pub pending_changes: usize,
    pub pending_change_bytes: usize,
    pub issued_requests: u64,
    pub accepted_results: u64,
    pub stale_results: u64,
    pub released_artifacts: u64,
    /// Deterministic store records visited by successful host mutations.
    pub store_entry_visits: u64,
    /// Number of owned processor inputs materialized after all preflight checks.
    pub input_materializations: u64,
}
