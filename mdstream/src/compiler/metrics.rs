use super::{
    reconcile::ReconcileMetrics,
    types::{CompilerError, CompilerMetrics},
};

pub(super) struct CompileObservation {
    pub(super) parse_passes: u64,
    pub(super) parsed_source_bytes: u64,
    pub(super) custom_scan_source_bytes: u64,
    pub(super) structural_source_bytes: usize,
    pub(super) frontier_bytes: usize,
    pub(super) next_checkpoint: usize,
    pub(super) reconciled: ReconcileMetrics,
}

pub(super) fn compile_metrics(
    previous: CompilerMetrics,
    observation: CompileObservation,
) -> Result<CompilerMetrics, CompilerError> {
    Ok(CompilerMetrics {
        structural_source_bytes: add_metric_bytes(
            previous.structural_source_bytes,
            observation.structural_source_bytes,
            "structural source bytes",
        )?,
        deferred_source_bytes: previous.deferred_source_bytes,
        parse_passes: previous
            .parse_passes
            .checked_add(observation.parse_passes)
            .ok_or(CompilerError::MetricsOverflow("parse passes"))?,
        parsed_source_bytes: previous
            .parsed_source_bytes
            .checked_add(observation.parsed_source_bytes)
            .ok_or(CompilerError::MetricsOverflow("parsed bytes"))?,
        custom_scan_source_bytes: previous
            .custom_scan_source_bytes
            .checked_add(observation.custom_scan_source_bytes)
            .ok_or(CompilerError::MetricsOverflow("custom scan source bytes"))?,
        reconcile_node_visits: previous
            .reconcile_node_visits
            .checked_add(observation.reconciled.nodes_visited)
            .ok_or(CompilerError::MetricsOverflow("reconciled nodes"))?,
        reconcile_structure_owners: previous
            .reconcile_structure_owners
            .checked_add(observation.reconciled.structure_owners_visited)
            .ok_or(CompilerError::MetricsOverflow(
                "reconciled structure owners",
            ))?,
        reconcile_structure_id_comparisons: previous
            .reconcile_structure_id_comparisons
            .checked_add(observation.reconciled.structure_id_comparisons)
            .ok_or(CompilerError::MetricsOverflow(
                "reconciled structure ID comparisons",
            ))?,
        reconcile_structure_version_steps: previous
            .reconcile_structure_version_steps
            .checked_add(observation.reconciled.structure_version_steps)
            .ok_or(CompilerError::MetricsOverflow(
                "reconciled structure version steps",
            ))?,
        reconcile_structure_ids_emitted: previous
            .reconcile_structure_ids_emitted
            .checked_add(observation.reconciled.structure_ids_emitted)
            .ok_or(CompilerError::MetricsOverflow(
                "reconciled structure IDs emitted",
            ))?,
        reconcile_resource_visits: previous
            .reconcile_resource_visits
            .checked_add(observation.reconciled.resources_visited)
            .ok_or(CompilerError::MetricsOverflow("reconciled resources"))?,
        incremental_projection_visits: previous.incremental_projection_visits,
        frontier_bytes: observation.frontier_bytes,
        next_checkpoint: observation.next_checkpoint,
    })
}

pub(super) fn add_metric_bytes(
    previous: u64,
    bytes: usize,
    field: &'static str,
) -> Result<u64, CompilerError> {
    let bytes = u64::try_from(bytes).map_err(|_| CompilerError::MetricsOverflow(field))?;
    previous
        .checked_add(bytes)
        .ok_or(CompilerError::MetricsOverflow(field))
}
