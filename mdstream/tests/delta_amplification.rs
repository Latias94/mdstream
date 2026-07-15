use mdstream::{CompilerMetrics, EngineOutput, StreamEngine};
use mdstream_protocol::{
    ApplyOutcome, ChangePayloadCost, ProtocolLimits, Reducer, ReducerMetrics, encode_change_json,
};

const SIZES: [usize; 4] = [8 * 1024, 16 * 1024, 32 * 1024, 64 * 1024];

#[derive(Debug, Clone, Copy, Default)]
struct PipelineObservation {
    raw_source_bytes: usize,
    projection_text_bytes: usize,
    wire_text_bytes: usize,
    encoded_wire_bytes: usize,
    change_sets: usize,
    operations: usize,
}

fn compiler_work(metrics: CompilerMetrics) -> u64 {
    metrics
        .structural_source_bytes
        .saturating_add(metrics.deferred_source_bytes)
        .saturating_add(metrics.parsed_source_bytes)
        .saturating_add(metrics.custom_scan_source_bytes)
        .saturating_add(metrics.reconcile_node_visits)
        .saturating_add(metrics.reconcile_structure_owners)
        .saturating_add(metrics.reconcile_structure_id_comparisons)
        .saturating_add(metrics.reconcile_structure_version_steps)
        .saturating_add(metrics.reconcile_structure_ids_emitted)
        .saturating_add(metrics.reconcile_resource_visits)
        .saturating_add(metrics.incremental_projection_visits)
        .saturating_add(metrics.semantic_definition_visits)
        .saturating_add(metrics.semantic_state_key_visits)
        .saturating_add(metrics.semantic_state_edge_visits)
        .saturating_add(metrics.semantic_candidate_node_visits)
        .saturating_add(metrics.semantic_candidate_dependency_visits)
        .saturating_add(metrics.semantic_dependent_visits)
        .saturating_add(metrics.semantic_corrections_emitted)
}

fn reducer_work(metrics: ReducerMetrics) -> u64 {
    metrics
        .applied_changes
        .saturating_add(metrics.operations_visited)
        .saturating_add(metrics.nodes_validated)
        .saturating_add(metrics.relationship_steps)
        .saturating_add(metrics.child_ids_copied)
}

fn observe_output(
    reducer: &mut Reducer,
    output: EngineOutput,
    observation: &mut PipelineObservation,
) {
    let limits = ProtocolLimits::default();
    for change in output.into_changes() {
        observation.raw_source_bytes = observation
            .raw_source_bytes
            .checked_add(change.source().suffix.len())
            .unwrap();
        observation.change_sets = observation.change_sets.checked_add(1).unwrap();
        observation.operations = observation
            .operations
            .checked_add(change.operations().len())
            .unwrap();
        let payload = change
            .operations()
            .iter()
            .try_fold(ChangePayloadCost::ZERO, |total, operation| {
                total.checked_add(operation.payload_cost(limits)?)
            })
            .unwrap();
        let wire_text_bytes = payload.wire_text_bytes;
        observation.projection_text_bytes = observation
            .projection_text_bytes
            .checked_add(payload.metadata_bytes)
            .unwrap();
        observation.wire_text_bytes = observation
            .wire_text_bytes
            .checked_add(wire_text_bytes)
            .unwrap();
        observation.encoded_wire_bytes = observation
            .encoded_wire_bytes
            .checked_add(
                encode_change_json(&change, usize::MAX, limits)
                    .unwrap()
                    .len(),
            )
            .unwrap();
        assert!(matches!(
            reducer.apply(change).unwrap(),
            ApplyOutcome::Applied { .. } | ApplyOutcome::Recovered { .. }
        ));
    }
}

fn fixture(prefix: &str, row: &str) -> String {
    let mut source = String::with_capacity(SIZES[SIZES.len() - 1]);
    source.push_str(prefix);
    while source.len() < SIZES[SIZES.len() - 1] {
        source.push_str(row);
    }
    source.truncate(SIZES[SIZES.len() - 1]);
    source
}

fn assert_pipeline_bounds(label: &str, source: &str) {
    assert_eq!(source.len(), SIZES[SIZES.len() - 1]);
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    let mut observation = PipelineObservation::default();
    let mut previous_work: Option<u64> = None;

    for chunk in source.chars().map(String::from) {
        observe_output(
            &mut reducer,
            engine.append(&chunk).unwrap(),
            &mut observation,
        );
        let size = reducer
            .document()
            .map_or(0, |document| document.source().len());
        if !SIZES.contains(&size) {
            continue;
        }

        assert_eq!(observation.raw_source_bytes, size, "{label} raw source");
        let engine_work = engine.metrics().work;
        assert_eq!(
            engine_work.raw_source_bytes,
            u64::try_from(observation.raw_source_bytes).unwrap()
        );
        assert_eq!(
            engine_work.projection_text_bytes,
            u64::try_from(observation.projection_text_bytes).unwrap()
        );
        assert_eq!(
            engine_work.wire_text_bytes,
            u64::try_from(observation.wire_text_bytes).unwrap()
        );
        assert_eq!(
            engine_work.change_sets,
            u64::try_from(observation.change_sets).unwrap()
        );
        assert_eq!(
            engine_work.operations,
            u64::try_from(observation.operations).unwrap()
        );
        assert!(
            observation.projection_text_bytes <= size.saturating_mul(8),
            "{label} projection text amplified {} bytes of source to {} bytes",
            size,
            observation.projection_text_bytes
        );
        let wire_limit = observation
            .raw_source_bytes
            .checked_add(observation.wire_text_bytes)
            .unwrap()
            .checked_mul(6)
            .unwrap()
            .checked_add(observation.change_sets.checked_mul(512).unwrap())
            .unwrap()
            .checked_add(observation.operations.checked_mul(128).unwrap())
            .unwrap();
        assert!(
            observation.encoded_wire_bytes <= wire_limit,
            "{label} wire amplification at {size} bytes: observed={}, limit={wire_limit}, observation={observation:?}",
            observation.encoded_wire_bytes
        );

        let work = compiler_work(engine.metrics().compiler)
            .saturating_add(reducer_work(reducer.metrics()))
            .saturating_add(u64::try_from(observation.encoded_wire_bytes).unwrap());
        if let Some(previous) = previous_work {
            assert!(
                work.saturating_mul(100) <= previous.saturating_mul(225),
                "{label} full-pipeline work grew from {previous} to {work} at {size} bytes"
            );
        }
        previous_work = Some(work);
    }

    assert_eq!(reducer.document().unwrap().source(), source);
}

#[test]
fn pending_paragraph_pipeline_is_bounded() {
    assert_pipeline_bounds("paragraph", &"x".repeat(SIZES[SIZES.len() - 1]));
}

#[test]
fn pending_fence_pipeline_is_bounded() {
    assert_pipeline_bounds("fence", &fixture("```text\n", "code line\n"));
}

#[test]
fn pending_container_pipeline_is_bounded() {
    let row = format!("> {}\n", "x".repeat(252));
    assert_pipeline_bounds("container", &fixture("", &row));
}

#[test]
fn pending_table_pipeline_is_bounded() {
    let row = format!("{} | {}\n", "x".repeat(124), "y".repeat(124));
    assert_pipeline_bounds("table", &fixture("a | b\n--|--\n", &row));
}

#[test]
fn pending_unicode_pipeline_is_bounded() {
    assert_pipeline_bounds("unicode", &"界x".repeat(SIZES[SIZES.len() - 1] / 4));
}

#[test]
fn normalized_source_payload_is_exact_per_epoch() {
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    let mut first = PipelineObservation::default();
    for chunk in ["a\r", "", "\n", "界"] {
        observe_output(&mut reducer, engine.append(chunk).unwrap(), &mut first);
    }
    assert_eq!(first.raw_source_bytes, "a\n界".len());
    assert_eq!(reducer.document().unwrap().source(), "a\n界");

    observe_output(
        &mut reducer,
        engine.reset().unwrap(),
        &mut PipelineObservation::default(),
    );
    let mut second = PipelineObservation::default();
    observe_output(&mut reducer, engine.append("next").unwrap(), &mut second);
    assert_eq!(second.raw_source_bytes, 4);
    assert_eq!(reducer.document().unwrap().source(), "next");
}
