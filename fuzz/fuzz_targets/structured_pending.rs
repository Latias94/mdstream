#![no_main]

use std::ops::Range;

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use mdstream::{EngineLimits, EngineOutput, StreamEngine};
use mdstream_conformance::{
    CanonicalPendingScenario, NormalizedSnapshot, PendingScenarioShape, PendingScenarioSize,
    ProtocolTrace, TraceInputEvent, replay_protocol_trace, utf8_ranges_from_target_widths,
};
use mdstream_protocol::{
    ApplyOutcome, ChangeSet, DocumentLifecycle, ProtocolLimits, Reducer, Snapshot,
};

const WIDTH_SELECTORS: usize = 16;
const MAX_CHUNKS: usize = 256;

#[derive(Arbitrary, Debug)]
struct StructuredPendingCase {
    shape: u8,
    size: u8,
    widths: [u16; WIDTH_SELECTORS],
    fallback_width: u16,
}

fn selected_scenario(case: &StructuredPendingCase) -> CanonicalPendingScenario {
    let shape =
        PendingScenarioShape::ALL[usize::from(case.shape) % PendingScenarioShape::ALL.len()];
    let size = PendingScenarioSize::ALL[usize::from(case.size) % PendingScenarioSize::ALL.len()];
    CanonicalPendingScenario::new(shape, size)
}

fn bounded_split_ranges(source: &str, case: &StructuredPendingCase) -> Vec<Range<usize>> {
    let minimum_width = source.len().div_ceil(MAX_CHUNKS);
    let maximum_width = (source.len() / 4).max(minimum_width);
    let width_span = maximum_width - minimum_width + 1;
    let map_width = |width: u16| minimum_width + usize::from(width) % width_span;

    let ranges = utf8_ranges_from_target_widths(
        source,
        case.widths.into_iter().map(map_width),
        map_width(case.fallback_width),
    )
    .expect("bounded fuzz widths are always non-zero");
    assert!(ranges.len() >= 2);
    assert!(ranges.len() <= MAX_CHUNKS);
    ranges
}

fn reducer_snapshot(reducer: &Reducer) -> Option<Snapshot> {
    reducer.document().map(|document| document.snapshot())
}

fn assert_transition_limits(
    engine: &StreamEngine,
    output: &EngineOutput,
    protocol_limits: ProtocolLimits,
    engine_limits: EngineLimits,
) {
    let metrics = engine.metrics();
    assert!(metrics.storage.canonical_source_bytes <= protocol_limits.max_source_bytes);
    assert!(metrics.storage.frontier_bytes <= protocol_limits.max_source_bytes);
    assert!(metrics.retained_input_bytes <= protocol_limits.max_source_bytes);
    assert!(metrics.compiler.retained_semantic_definitions <= protocol_limits.max_definitions);
    assert!(
        metrics.compiler.retained_semantic_dependencies <= protocol_limits.max_definition_edges
    );
    assert!(
        metrics.compiler.retained_semantic_metadata_bytes
            <= protocol_limits.max_definition_metadata_bytes
    );
    assert!(metrics.work.last_change_bytes <= engine_limits.max_change_bytes);
    assert!(metrics.work.last_transaction_bytes <= engine_limits.max_transaction_bytes);
    assert!(metrics.work.peak_transaction_bytes <= engine_limits.max_transaction_bytes);
    assert!(
        output
            .changes()
            .iter()
            .all(|change| change.operations().len() <= protocol_limits.max_operations)
    );
}

fn consume_output(
    engine: &StreamEngine,
    reducer: &mut Reducer,
    changes: &mut Vec<ChangeSet>,
    output: EngineOutput,
    protocol_limits: ProtocolLimits,
    engine_limits: EngineLimits,
) -> usize {
    assert_transition_limits(engine, &output, protocol_limits, engine_limits);

    let mut emitted_source_bytes = 0usize;
    for change in output.into_changes() {
        emitted_source_bytes = emitted_source_bytes
            .checked_add(change.source().suffix.len())
            .expect("canonical pending sources fit in usize");
        let replay_change = change.clone();
        assert!(matches!(
            reducer
                .apply(change)
                .expect("canonical engine output must apply to a consumer reducer"),
            ApplyOutcome::Applied { .. } | ApplyOutcome::Recovered { .. }
        ));
        changes.push(replay_change);
    }
    emitted_source_bytes
}

fn run_schedule(
    scenario_id: &str,
    schedule_id: &str,
    source: &str,
    ranges: &[Range<usize>],
) -> NormalizedSnapshot {
    let protocol_limits = ProtocolLimits::default();
    let engine_limits = EngineLimits::default();
    let mut engine = StreamEngine::builder()
        .protocol_limits(protocol_limits)
        .engine_limits(engine_limits)
        .build()
        .expect("default limits accept canonical pending scenarios");
    let mut reducer = Reducer::with_limits(protocol_limits);
    let mut changes = Vec::new();
    let mut input_events = Vec::with_capacity(ranges.len() + 1);
    let mut emitted_source_bytes = 0usize;

    for range in ranges {
        let chunk = &source[range.clone()];
        let output = engine
            .append(chunk)
            .expect("canonical pending chunks fit default engine limits");
        emitted_source_bytes += consume_output(
            &engine,
            &mut reducer,
            &mut changes,
            output,
            protocol_limits,
            engine_limits,
        );
        input_events.push(TraceInputEvent::Append {
            chunk: chunk.to_string(),
            change_end: changes.len(),
        });
    }

    let output = engine
        .finish()
        .expect("canonical pending scenarios finalize within default limits");
    emitted_source_bytes += consume_output(
        &engine,
        &mut reducer,
        &mut changes,
        output,
        protocol_limits,
        engine_limits,
    );
    input_events.push(TraceInputEvent::Finish {
        change_end: changes.len(),
    });

    let snapshot = engine
        .snapshot()
        .expect("finishing a canonical pending scenario installs a document");
    assert_eq!(Some(snapshot.clone()), reducer_snapshot(&reducer));
    assert_eq!(snapshot.lifecycle(), DocumentLifecycle::Finalized);
    assert_eq!(snapshot.source(), source);
    assert_eq!(emitted_source_bytes, source.len());
    assert_eq!(
        snapshot.coordinate().source_cursor.get(),
        u64::try_from(source.len()).unwrap()
    );
    assert_eq!(
        snapshot.projection_cursor(),
        snapshot.coordinate().source_cursor
    );
    assert_eq!(
        engine.metrics().work.raw_source_bytes,
        u64::try_from(source.len()).unwrap()
    );
    assert_eq!(
        reducer.metrics().source_bytes_appended,
        u64::try_from(source.len()).unwrap()
    );

    let trace = ProtocolTrace {
        id: format!("{scenario_id}-{schedule_id}"),
        schedule: schedule_id.to_string(),
        setup_changes: 0,
        input_events,
        changes,
    };
    let replay = replay_protocol_trace(&trace).expect("structured engine trace must replay");
    let normalized = NormalizedSnapshot::from(snapshot);
    assert_eq!(replay.normalized_final_snapshot(), normalized);
    normalized
}

fuzz_target!(|case: StructuredPendingCase| {
    let scenario = selected_scenario(&case);
    let source = scenario.source();
    let whole_range = 0..source.len();
    let split_ranges = bounded_split_ranges(&source, &case);

    let whole = run_schedule(
        scenario.id(),
        "whole",
        &source,
        std::slice::from_ref(&whole_range),
    );
    let split = run_schedule(scenario.id(), "bounded-split", &source, &split_ranges);
    assert_eq!(split.roots, whole.roots, "root identity changed");
    assert_eq!(split.nodes, whole.nodes, "node identity changed");
    assert_eq!(
        split.resources, whole.resources,
        "resource identity changed"
    );
    assert_eq!(split, whole, "final snapshot changed with chunk schedule");
});
