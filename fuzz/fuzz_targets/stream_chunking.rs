#![no_main]

use std::fmt::Write as _;

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use mdstream::{
    CompilerError, EngineError, EngineLimits, EngineOutput, StreamEngine,
};
use mdstream_conformance::{
    NormalizedSnapshot, ProtocolTrace, TraceInputEvent, assert_last_retry_idempotent,
    replay_protocol_trace, utf8_ranges_from_target_widths,
};
use mdstream_protocol::{
    ApplyOutcome, ChangeSet, ContentKind, NodeId, NodeVersion, ProjectionOp, ProtocolError,
    ProtocolLimits, Reducer, ResourceRef, Snapshot,
};

const MAX_INPUT_BYTES: usize = 4096;
const PREFIX: &str = "seed\n\n\r";
const RECOVERY: [&str; 3] = ["x\n\n", "[probe][late]\n\n", "[late]: /accepted\n"];

#[derive(Arbitrary, Debug)]
struct StreamCase {
    input: String,
    split_bytes: Vec<u8>,
    limit_plane: u8,
    limit_cut: u16,
    compaction_blocks: u8,
}

#[derive(Clone, Copy, Debug)]
enum LimitPlane {
    SourceBytes,
    ChangeBytes,
    TransactionBytes,
    Operations,
    DefinitionEdges,
}

impl LimitPlane {
    fn from_byte(value: u8) -> Self {
        match value % 5 {
            0 => Self::SourceBytes,
            1 => Self::ChangeBytes,
            2 => Self::TransactionBytes,
            3 => Self::Operations,
            _ => Self::DefinitionEdges,
        }
    }
}

fn bounded_input(input: &str) -> String {
    let mut end = input.len().min(MAX_INPUT_BYTES);
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    input[..end].to_string()
}

fn chunks<'a>(input: &'a str, split_bytes: &[u8]) -> Vec<&'a str> {
    utf8_ranges_from_target_widths(
        input,
        split_bytes
            .iter()
            .copied()
            .map(usize::from)
            .map(|width| width.max(1)),
        16,
    )
    .expect("fuzz widths and fallback are non-zero")
    .into_iter()
    .map(|range| &input[range])
    .collect()
}

fn consume_output(
    reducer: &mut Reducer,
    changes: &mut Vec<ChangeSet>,
    output: EngineOutput,
) -> usize {
    let mut emitted_source_bytes = 0usize;
    for change in output.into_changes() {
        emitted_source_bytes = emitted_source_bytes
            .checked_add(change.source().suffix.len())
            .expect("bounded fuzz source accounting cannot overflow");
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

fn reducer_snapshot(reducer: &Reducer) -> Option<Snapshot> {
    reducer.document().map(|document| document.snapshot())
}

fn run_canonical(id: &str, input_chunks: &[&str]) -> NormalizedSnapshot {
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    let mut changes = Vec::new();
    let mut input_events = Vec::with_capacity(input_chunks.len() + 1);
    let mut emitted_source_bytes = 0usize;

    for chunk in input_chunks {
        emitted_source_bytes += consume_output(
            &mut reducer,
            &mut changes,
            engine.append(chunk).expect("arbitrary UTF-8 is valid Markdown input"),
        );
        input_events.push(TraceInputEvent::Append {
            chunk: (*chunk).to_string(),
            change_end: changes.len(),
        });
        assert_eq!(engine.snapshot(), reducer_snapshot(&reducer));
    }

    emitted_source_bytes += consume_output(
        &mut reducer,
        &mut changes,
        engine.finish().expect("default limits accept bounded fuzz input"),
    );
    input_events.push(TraceInputEvent::Finish {
        change_end: changes.len(),
    });

    let snapshot = engine
        .snapshot()
        .expect("finish installs a canonical document even for empty input");
    assert_eq!(Some(snapshot.clone()), reducer_snapshot(&reducer));
    assert_eq!(emitted_source_bytes, snapshot.source().len());
    assert_snapshot_integrity(&snapshot);

    let trace = ProtocolTrace {
        id: id.to_string(),
        schedule: id.to_string(),
        setup_changes: 0,
        input_events,
        changes,
    };
    let replay = replay_protocol_trace(&trace).expect("engine trace must satisfy replay laws");
    assert_last_retry_idempotent(&trace).expect("last engine change retry must be idempotent");
    let normalized = NormalizedSnapshot::from(snapshot);
    assert_eq!(replay.normalized_final_snapshot(), normalized);
    normalized
}

fn assert_snapshot_integrity(snapshot: &Snapshot) {
    let source = snapshot.source();
    for node in snapshot.nodes() {
        let source_start = usize::try_from(node.source.start.get()).unwrap();
        let source_end = usize::try_from(node.source.end.get()).unwrap();
        let body_start = usize::try_from(node.body.start.get()).unwrap();
        let body_end = usize::try_from(node.body.end.get()).unwrap();
        assert!(source_start <= body_start && body_end <= source_end);
        assert!(source_end <= source.len());
        assert!(source.is_char_boundary(source_start));
        assert!(source.is_char_boundary(source_end));
        assert!(source.is_char_boundary(body_start));
        assert!(source.is_char_boundary(body_end));
        assert_eq!(node.version, node.derived_version());
        assert_eq!(node.children.version(), &node.children.derived_version());
    }
    for resource in snapshot.resources() {
        assert_eq!(resource.version, resource.derived_version());
    }
}

fn heavy_transition(block_count: usize) -> String {
    let mut source = String::new();
    for index in 0..block_count {
        writeln!(source, "[dependent-{index}][late]\n").unwrap();
    }
    source
}

fn plane_cost(engine: &StreamEngine, output: &EngineOutput, plane: LimitPlane) -> usize {
    match plane {
        LimitPlane::SourceBytes => engine
            .snapshot()
            .map_or(0, |snapshot| snapshot.source().len()),
        LimitPlane::ChangeBytes => engine.metrics().work.last_change_bytes,
        LimitPlane::TransactionBytes => engine.metrics().work.last_transaction_bytes,
        LimitPlane::Operations => output
            .changes()
            .iter()
            .map(|change| change.operations().len())
            .sum(),
        LimitPlane::DefinitionEdges => engine.metrics().compiler.retained_semantic_dependencies,
    }
}

fn measured_limit_bounds(plane: LimitPlane, heavy: &str) -> (usize, usize) {
    let mut control = StreamEngine::new();
    let mut floor = 0usize;
    for chunk in std::iter::once(PREFIX).chain(RECOVERY) {
        let output = control.append(chunk).unwrap();
        floor = floor.max(plane_cost(&control, &output, plane));
    }
    let output = control.finish().unwrap();
    floor = floor.max(plane_cost(&control, &output, plane));

    let mut measured = StreamEngine::new();
    measured.append(PREFIX).unwrap();
    let output = measured.append(heavy).unwrap();
    let actual = plane_cost(&measured, &output, plane);
    assert!(
        actual > floor,
        "heavy transition must exceed the clean control for {plane:?}: floor={floor}, actual={actual}"
    );
    (floor, actual)
}

fn configured_engine(
    plane: LimitPlane,
    limit: usize,
) -> (StreamEngine, ProtocolLimits, EngineLimits) {
    let mut protocol_limits = ProtocolLimits::default();
    let mut engine_limits = EngineLimits::default();
    match plane {
        LimitPlane::SourceBytes => protocol_limits.max_source_bytes = limit,
        LimitPlane::ChangeBytes => engine_limits.max_change_bytes = limit,
        LimitPlane::TransactionBytes => engine_limits.max_transaction_bytes = limit,
        LimitPlane::Operations => protocol_limits.max_operations = limit,
        LimitPlane::DefinitionEdges => protocol_limits.max_definition_edges = limit,
    }
    let engine = StreamEngine::builder()
        .protocol_limits(protocol_limits)
        .engine_limits(engine_limits)
        .build()
        .unwrap();
    (engine, protocol_limits, engine_limits)
}

fn assert_limit_error(error: &EngineError, plane: LimitPlane, limit: usize) {
    let matches = match (plane, error) {
        (
            LimitPlane::SourceBytes,
            EngineError::Protocol(ProtocolError::SourceTooLarge {
                limit: seen,
                actual,
            }),
        ) => *seen == limit && *actual > limit,
        (
            LimitPlane::ChangeBytes,
            EngineError::LimitExceeded {
                field: "engine.change_bytes",
                limit: seen,
                actual,
            },
        ) => *seen == limit && *actual > limit,
        (
            LimitPlane::TransactionBytes,
            EngineError::LimitExceeded {
                field: "engine.transaction_bytes",
                limit: seen,
                actual,
            },
        ) => *seen == limit && *actual > limit,
        (
            LimitPlane::Operations,
            EngineError::Compiler(CompilerError::LimitExceeded {
                field: "change.operations",
                limit: seen,
                actual,
            }),
        ) => *seen == limit && *actual > limit,
        (
            LimitPlane::DefinitionEdges,
            EngineError::Compiler(CompilerError::LimitExceeded {
                field: "definition.dependencies",
                limit: seen,
                actual,
            }),
        ) => *seen == limit && *actual > limit,
        _ => false,
    };
    assert!(matches, "unexpected {plane:?} rejection at {limit}: {error:?}");
}

fn assert_engine_equivalent(left: &StreamEngine, right: &StreamEngine) {
    assert_eq!(left.snapshot(), right.snapshot());
    assert_eq!(left.coordinate(), right.coordinate());
    assert_eq!(left.lifecycle(), right.lifecycle());
    assert_eq!(left.metrics(), right.metrics());
}

fn assert_reducer_equivalent(left: &Reducer, right: &Reducer) {
    assert_eq!(reducer_snapshot(left), reducer_snapshot(right));
    assert_eq!(left.metrics(), right.metrics());
}

fn assert_success_within_limits(
    engine: &StreamEngine,
    output: &EngineOutput,
    protocol_limits: ProtocolLimits,
    engine_limits: EngineLimits,
) {
    assert!(engine.metrics().work.last_change_bytes <= engine_limits.max_change_bytes);
    assert!(engine.metrics().work.peak_transaction_bytes <= engine_limits.max_transaction_bytes);
    assert!(
        output
            .changes()
            .iter()
            .all(|change| change.operations().len() <= protocol_limits.max_operations)
    );
    assert!(
        engine.metrics().compiler.retained_semantic_dependencies
            <= protocol_limits.max_definition_edges
    );
    assert!(
        engine
            .snapshot()
            .is_none_or(|snapshot| snapshot.source().len() <= protocol_limits.max_source_bytes)
    );
}

fn assert_random_limit_atomicity(case: &StreamCase) {
    let plane = LimitPlane::from_byte(case.limit_plane);
    let block_count = 32 + usize::from(case.compaction_blocks % 32);
    let heavy = heavy_transition(block_count);
    let (floor, actual) = measured_limit_bounds(plane, &heavy);
    let limit = floor + usize::from(case.limit_cut) % (actual - floor);
    let (mut candidate, protocol_limits, engine_limits) = configured_engine(plane, limit);
    let (mut control, _, _) = configured_engine(plane, limit);
    let mut candidate_reducer = Reducer::new();
    let mut control_reducer = Reducer::new();
    let mut scratch = Vec::new();

    let candidate_prefix = candidate.append(PREFIX).unwrap();
    let control_prefix = control.append(PREFIX).unwrap();
    assert_eq!(candidate_prefix, control_prefix);
    assert_success_within_limits(
        &candidate,
        &candidate_prefix,
        protocol_limits,
        engine_limits,
    );
    consume_output(&mut candidate_reducer, &mut scratch, candidate_prefix);
    scratch.clear();
    consume_output(&mut control_reducer, &mut scratch, control_prefix);
    scratch.clear();
    assert_engine_equivalent(&candidate, &control);
    assert_reducer_equivalent(&candidate_reducer, &control_reducer);

    let error = candidate
        .append(&heavy)
        .expect_err("measured hard limit must reject the heavy transition");
    assert_limit_error(&error, plane, limit);
    assert_engine_equivalent(&candidate, &control);
    assert_reducer_equivalent(&candidate_reducer, &control_reducer);
    let retry_error = candidate
        .append(&heavy)
        .expect_err("retrying the rejected transition must reject identically");
    assert_eq!(retry_error, error);
    assert_engine_equivalent(&candidate, &control);

    for chunk in RECOVERY {
        let candidate_output = candidate.append(chunk).unwrap();
        let control_output = control.append(chunk).unwrap();
        assert_eq!(candidate_output, control_output);
        assert_success_within_limits(
            &candidate,
            &candidate_output,
            protocol_limits,
            engine_limits,
        );
        consume_output(&mut candidate_reducer, &mut scratch, candidate_output);
        scratch.clear();
        consume_output(&mut control_reducer, &mut scratch, control_output);
        scratch.clear();
        assert_engine_equivalent(&candidate, &control);
        assert_reducer_equivalent(&candidate_reducer, &control_reducer);
    }

    let candidate_finish = candidate.finish().unwrap();
    let control_finish = control.finish().unwrap();
    assert_eq!(candidate_finish, control_finish);
    assert_success_within_limits(
        &candidate,
        &candidate_finish,
        protocol_limits,
        engine_limits,
    );
    consume_output(&mut candidate_reducer, &mut scratch, candidate_finish);
    scratch.clear();
    consume_output(&mut control_reducer, &mut scratch, control_finish);
    assert_engine_equivalent(&candidate, &control);
    assert_reducer_equivalent(&candidate_reducer, &control_reducer);
}

fn late_reference(snapshot: &Snapshot) -> (NodeId, NodeVersion, Option<ResourceRef>) {
    snapshot
        .nodes()
        .iter()
        .find_map(|node| match &node.content {
            ContentKind::Link {
                reference_label,
                target,
                ..
            } if reference_label.as_deref() == Some("late") => {
                Some((node.id, node.version.clone(), target.clone()))
            }
            _ => None,
        })
        .expect("late reference must remain in the canonical projection")
}

fn assert_compaction_semantics(case: &StreamCase) {
    let block_count = 8 + usize::from(case.compaction_blocks % 16);
    let mut prefix = String::from("[late]\n\n");
    for index in 0..block_count {
        writeln!(prefix, "# H{index}\n").unwrap();
    }

    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    let mut scratch = Vec::new();
    for chunk in chunks(&prefix, &case.split_bytes) {
        consume_output(
            &mut reducer,
            &mut scratch,
            engine.append(chunk).unwrap(),
        );
        scratch.clear();
    }
    let before = engine.snapshot().unwrap();
    assert_snapshot_integrity(&before);
    assert!(engine.metrics().retained_source_base > 0);
    assert_eq!(engine.metrics().compiler.retained_semantic_dependencies, 1);
    let (before_id, before_version, before_target) = late_reference(&before);
    assert_eq!(before_target, None);

    let definition = "[late]: /target\n";
    let correction = engine.append(definition).unwrap();
    assert!(correction.changes().iter().any(|change| {
        change.operations().iter().any(|operation| {
            matches!(
                operation,
                ProjectionOp::ReplaceNode { node_id, .. } if *node_id == before_id
            )
        })
    }));
    consume_output(&mut reducer, &mut scratch, correction);
    scratch.clear();
    consume_output(&mut reducer, &mut scratch, engine.finish().unwrap());
    let after = engine.snapshot().unwrap();
    assert_eq!(Some(after.clone()), reducer_snapshot(&reducer));
    assert_snapshot_integrity(&after);
    let (after_id, after_version, after_target) = late_reference(&after);
    assert_eq!(after_id, before_id);
    assert_ne!(after_version, before_version);
    assert!(after_target.is_some());

    let source = format!("{prefix}{definition}");
    let expected = run_canonical("compaction-whole", &[source.as_str()]);
    assert_eq!(NormalizedSnapshot::from(after), expected);
}

fuzz_target!(|case: StreamCase| {
    let input = bounded_input(&case.input);
    let whole = [input.as_str()];
    let split = chunks(&input, &case.split_bytes);
    assert_eq!(
        run_canonical("fuzz-split", &split),
        run_canonical("fuzz-whole", &whole)
    );
    assert_random_limit_atomicity(&case);
    assert_compaction_semantics(&case);
});
