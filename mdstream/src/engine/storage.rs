use mdstream_protocol::{Document, ReducerMetrics};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
/// Deterministic retained-text and canonical-source storage measurements.
pub struct EngineStorageMetrics {
    pub canonical_source_bytes: usize,
    pub canonical_source_capacity: usize,
    pub canonical_ir_text_bytes: usize,
    pub canonical_ir_text_capacity: usize,
    pub frontier_bytes: usize,
    pub normalized_input_debt_bytes: usize,
    pub retained_text_bytes: usize,
    pub retained_text_capacity: usize,
    pub duplicated_source_body_bytes: usize,
    pub source_reallocation_copied_bytes: usize,
}

impl EngineStorageMetrics {
    pub(super) fn measure(
        document: Option<&Document>,
        frontier_bytes: usize,
        normalized_input_debt_bytes: usize,
        reducer: ReducerMetrics,
    ) -> Self {
        let canonical_source_bytes = document.map_or(0, |document| document.source().len());
        let canonical_source_capacity = document.map_or(0, Document::source_capacity);
        let canonical_ir_text_bytes = document.map_or(0, Document::retained_ir_text_bytes);
        let canonical_ir_text_capacity = document.map_or(0, Document::retained_ir_text_capacity);
        Self {
            canonical_source_bytes,
            canonical_source_capacity,
            canonical_ir_text_bytes,
            canonical_ir_text_capacity,
            frontier_bytes,
            normalized_input_debt_bytes,
            retained_text_bytes: canonical_source_bytes
                .saturating_add(canonical_ir_text_bytes)
                .saturating_add(normalized_input_debt_bytes),
            retained_text_capacity: canonical_source_capacity
                .saturating_add(canonical_ir_text_capacity)
                .saturating_add(normalized_input_debt_bytes),
            duplicated_source_body_bytes: 0,
            source_reallocation_copied_bytes: usize::try_from(
                reducer.source_reallocation_copied_bytes,
            )
            .unwrap_or(usize::MAX),
        }
    }
}
