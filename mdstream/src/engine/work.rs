use mdstream_protocol::{ChangePayloadCost, ChangeSet};

use super::{EngineError, EngineLimits};

const CHANGE_ENVELOPE_BYTES: usize = 512;
const OPERATION_ENVELOPE_BYTES: usize = 128;
const STRUCTURAL_ITEM_BYTES: usize = 16;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
/// Deterministic cumulative work and payload measurements for the current epoch.
pub struct EngineWorkMetrics {
    pub raw_source_bytes: u64,
    pub projection_text_bytes: u64,
    pub wire_text_bytes: u64,
    pub change_sets: u64,
    pub operations: u64,
    pub wire_bound_bytes: u64,
    pub last_change_bytes: usize,
    pub last_transaction_bytes: usize,
    pub peak_transaction_bytes: usize,
}

impl EngineWorkMetrics {
    pub(super) fn check_transaction_lower_bound(
        suffix_bytes: usize,
        staging_frontier_bytes: usize,
        engine_limits: EngineLimits,
    ) -> Result<(), EngineError> {
        let transaction_bytes = CHANGE_ENVELOPE_BYTES
            .checked_add(staging_frontier_bytes)
            .and_then(|bytes| bytes.checked_add(suffix_bytes))
            .and_then(|bytes| bytes.checked_add(suffix_bytes))
            .ok_or(EngineError::MetricsOverflow("transaction bytes"))?;
        check_limit(
            "engine.transaction_bytes",
            engine_limits.max_transaction_bytes,
            transaction_bytes,
        )
    }

    pub(super) fn stage(
        self,
        change: &ChangeSet,
        payload: ChangePayloadCost,
        frontier_bytes: usize,
        engine_limits: EngineLimits,
    ) -> Result<Self, EngineError> {
        let wire_text_bytes = payload.wire_text_bytes;
        let operation_bytes = change
            .operations()
            .len()
            .checked_mul(OPERATION_ENVELOPE_BYTES)
            .ok_or(EngineError::MetricsOverflow("operation envelope bytes"))?;
        let structural_bytes = payload
            .structural_items
            .checked_mul(STRUCTURAL_ITEM_BYTES)
            .ok_or(EngineError::MetricsOverflow("structural item bytes"))?;
        let change_bytes = CHANGE_ENVELOPE_BYTES
            .checked_add(change.source().suffix.len())
            .and_then(|bytes| bytes.checked_add(wire_text_bytes))
            .and_then(|bytes| bytes.checked_add(operation_bytes))
            .and_then(|bytes| bytes.checked_add(structural_bytes))
            .ok_or(EngineError::MetricsOverflow("change bytes"))?;
        check_limit(
            "engine.change_bytes",
            engine_limits.max_change_bytes,
            change_bytes,
        )?;

        // The compiler reconstructs the mutable frontier while the normalized
        // suffix and typed change are both live. This is a deterministic,
        // conservative logical peak rather than allocator RSS.
        let transaction_bytes = change_bytes
            .checked_add(frontier_bytes)
            .and_then(|bytes| bytes.checked_add(change.source().suffix.len()))
            .ok_or(EngineError::MetricsOverflow("transaction bytes"))?;
        check_limit(
            "engine.transaction_bytes",
            engine_limits.max_transaction_bytes,
            transaction_bytes,
        )?;

        let text_payload = change
            .source()
            .suffix
            .len()
            .checked_add(wire_text_bytes)
            .ok_or(EngineError::MetricsOverflow("wire text payload"))?;
        let wire_bound = text_payload
            .checked_mul(6)
            .and_then(|bytes| bytes.checked_add(CHANGE_ENVELOPE_BYTES))
            .and_then(|bytes| bytes.checked_add(operation_bytes))
            .ok_or(EngineError::MetricsOverflow("wire bound bytes"))?;

        Ok(Self {
            raw_source_bytes: checked_add_usize(
                self.raw_source_bytes,
                change.source().suffix.len(),
                "raw source bytes",
            )?,
            projection_text_bytes: checked_add_usize(
                self.projection_text_bytes,
                payload.metadata_bytes,
                "projection text bytes",
            )?,
            wire_text_bytes: checked_add_usize(
                self.wire_text_bytes,
                wire_text_bytes,
                "wire text bytes",
            )?,
            change_sets: self
                .change_sets
                .checked_add(1)
                .ok_or(EngineError::MetricsOverflow("change sets"))?,
            operations: checked_add_usize(
                self.operations,
                change.operations().len(),
                "operations",
            )?,
            wire_bound_bytes: checked_add_usize(
                self.wire_bound_bytes,
                wire_bound,
                "wire bound bytes",
            )?,
            last_change_bytes: change_bytes,
            last_transaction_bytes: transaction_bytes,
            peak_transaction_bytes: self.peak_transaction_bytes.max(transaction_bytes),
        })
    }
}

fn checked_add_usize(current: u64, value: usize, field: &'static str) -> Result<u64, EngineError> {
    let value = u64::try_from(value).map_err(|_| EngineError::MetricsOverflow(field))?;
    current
        .checked_add(value)
        .ok_or(EngineError::MetricsOverflow(field))
}

fn check_limit(field: &'static str, limit: usize, actual: usize) -> Result<(), EngineError> {
    if actual > limit {
        Err(EngineError::LimitExceeded {
            field,
            limit,
            actual,
        })
    } else {
        Ok(())
    }
}
