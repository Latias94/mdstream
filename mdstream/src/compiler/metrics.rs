use super::{
    definitions::SemanticWork,
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
    pub(super) semantic: SemanticWork,
}

pub(super) fn compile_metrics(
    previous: CompilerMetrics,
    observation: CompileObservation,
) -> Result<CompilerMetrics, CompilerError> {
    let semantic = observation.semantic;
    let metrics = CompilerMetrics {
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
        semantic_definition_visits: previous.semantic_definition_visits,
        semantic_state_key_visits: previous.semantic_state_key_visits,
        semantic_state_edge_visits: previous.semantic_state_edge_visits,
        semantic_candidate_node_visits: previous.semantic_candidate_node_visits,
        semantic_candidate_dependency_visits: previous.semantic_candidate_dependency_visits,
        semantic_dependent_visits: previous.semantic_dependent_visits,
        semantic_corrections_emitted: previous.semantic_corrections_emitted,
        retained_semantic_definitions: previous.retained_semantic_definitions,
        retained_semantic_dependencies: previous.retained_semantic_dependencies,
        retained_semantic_metadata_bytes: previous.retained_semantic_metadata_bytes,
        frontier_bytes: observation.frontier_bytes,
        next_checkpoint: observation.next_checkpoint,
    };
    add_semantic_metrics(metrics, semantic)
}

pub(super) fn add_semantic_metrics(
    mut metrics: CompilerMetrics,
    semantic: SemanticWork,
) -> Result<CompilerMetrics, CompilerError> {
    metrics.semantic_definition_visits = metrics
        .semantic_definition_visits
        .checked_add(semantic.definition_visits)
        .ok_or(CompilerError::MetricsOverflow("semantic definitions"))?;
    metrics.semantic_state_key_visits = metrics
        .semantic_state_key_visits
        .checked_add(semantic.state_key_visits)
        .ok_or(CompilerError::MetricsOverflow("semantic state key visits"))?;
    metrics.semantic_state_edge_visits = metrics
        .semantic_state_edge_visits
        .checked_add(semantic.state_edge_visits)
        .ok_or(CompilerError::MetricsOverflow("semantic state edge visits"))?;
    metrics.semantic_candidate_node_visits = metrics
        .semantic_candidate_node_visits
        .checked_add(semantic.candidate_node_visits)
        .ok_or(CompilerError::MetricsOverflow("semantic candidate nodes"))?;
    metrics.semantic_candidate_dependency_visits = metrics
        .semantic_candidate_dependency_visits
        .checked_add(semantic.candidate_dependency_visits)
        .ok_or(CompilerError::MetricsOverflow(
            "semantic candidate dependencies",
        ))?;
    metrics.semantic_dependent_visits = metrics
        .semantic_dependent_visits
        .checked_add(semantic.dependent_visits)
        .ok_or(CompilerError::MetricsOverflow("semantic dependent visits"))?;
    metrics.semantic_corrections_emitted = metrics
        .semantic_corrections_emitted
        .checked_add(semantic.corrections_emitted)
        .ok_or(CompilerError::MetricsOverflow("semantic corrections"))?;
    metrics.retained_semantic_definitions = semantic.retained_definitions;
    metrics.retained_semantic_dependencies = semantic.retained_dependencies;
    metrics.retained_semantic_metadata_bytes = semantic.retained_metadata_bytes;
    Ok(metrics)
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
