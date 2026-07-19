use mdstream::{EngineOutput, StreamEngine};
use mdstream_conformance::{
    ChunkSchedule, NormalizedSnapshot, ProtocolTrace, TraceInputEvent, assert_trace_laws,
};
use mdstream_protocol::{ApplyOutcome, ChangeSet, Reducer, TransitionReducer};

fn apply_output(
    reducer: &mut Reducer,
    transition_reducer: &mut TransitionReducer,
    output: EngineOutput,
) -> usize {
    output
        .into_changes()
        .into_iter()
        .map(|change| {
            let source_bytes = change.source().suffix.len();
            let outcome = reducer.apply(change.clone()).unwrap();
            assert!(matches!(outcome, ApplyOutcome::Applied { .. }));
            let observed = transition_reducer.apply(change).unwrap();
            assert_eq!(observed.outcome, outcome);
            assert!(observed.facts.is_some());
            source_bytes
        })
        .sum()
}

fn replay(source: &str, schedule: ChunkSchedule) -> NormalizedSnapshot {
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    let mut transition_reducer = TransitionReducer::new();
    for chunk in schedule.slices(source).unwrap() {
        apply_output(
            &mut reducer,
            &mut transition_reducer,
            engine.append(chunk).unwrap(),
        );
    }
    apply_output(
        &mut reducer,
        &mut transition_reducer,
        engine.finish().unwrap(),
    );
    let snapshot = reducer.document().unwrap().snapshot();
    assert_eq!(
        Some(snapshot.clone()),
        transition_reducer
            .document()
            .map(|document| document.snapshot())
    );
    NormalizedSnapshot::from(snapshot)
}

#[test]
fn every_utf8_schedule_replays_through_the_canonical_reducer() {
    let source = "# H\r\n\r\nText `代码`\r\n";
    let whole = replay(source, ChunkSchedule::Whole);
    for schedule in [
        ChunkSchedule::Lines,
        ChunkSchedule::Characters,
        ChunkSchedule::Seeded {
            label: "u3.delta-stream".to_string(),
            seed: 7,
            trial: 11,
            max_bytes: 5,
        },
    ] {
        assert_eq!(replay(source, schedule), whole);
    }
    assert_eq!(whole.source, "# H\n\nText `代码`\n");
}

#[test]
fn one_byte_appends_emit_only_linear_normalized_source_suffixes() {
    let source = "x".repeat(4096);
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    let mut transition_reducer = TransitionReducer::new();
    let mut emitted_source_bytes = 0usize;

    for byte in source.as_bytes() {
        emitted_source_bytes += apply_output(
            &mut reducer,
            &mut transition_reducer,
            engine
                .append(std::str::from_utf8(std::slice::from_ref(byte)).unwrap())
                .unwrap(),
        );
    }
    emitted_source_bytes += apply_output(
        &mut reducer,
        &mut transition_reducer,
        engine.finish().unwrap(),
    );

    assert_eq!(emitted_source_bytes, source.len());
    assert_eq!(reducer.document().unwrap().source(), source);
    assert_eq!(
        transition_reducer.document().unwrap().snapshot(),
        reducer.document().unwrap().snapshot()
    );
}

fn extend_trace(changes: &mut Vec<ChangeSet>, output: EngineOutput) -> usize {
    changes.extend(output.into_changes());
    changes.len()
}

#[test]
fn engine_generated_trace_satisfies_conformance_laws() {
    let mut engine = StreamEngine::new();
    let mut changes = Vec::new();
    let mut input_events = Vec::new();

    for chunk in ["# H\r", "", "\n", "Body"] {
        let change_end = extend_trace(&mut changes, engine.append(chunk).unwrap());
        input_events.push(TraceInputEvent::Append {
            chunk: chunk.to_string(),
            change_end,
        });
    }
    let change_end = extend_trace(&mut changes, engine.finish().unwrap());
    input_events.push(TraceInputEvent::Finish { change_end });

    let trace = ProtocolTrace {
        id: "u3-engine".to_string(),
        schedule: "mixed".to_string(),
        setup_changes: 0,
        input_events,
        changes,
    };
    assert_trace_laws(&trace).unwrap();
}

#[test]
fn engine_reset_trace_satisfies_epoch_isolation_laws() {
    let mut engine = StreamEngine::new();
    let mut changes = Vec::new();
    extend_trace(&mut changes, engine.append("old").unwrap());
    let setup_changes = changes.len();

    let mut input_events = Vec::new();
    let change_end = extend_trace(&mut changes, engine.reset().unwrap());
    input_events.push(TraceInputEvent::Reset { change_end });
    let change_end = extend_trace(&mut changes, engine.append("new").unwrap());
    input_events.push(TraceInputEvent::Append {
        chunk: "new".to_string(),
        change_end,
    });
    let change_end = extend_trace(&mut changes, engine.finish().unwrap());
    input_events.push(TraceInputEvent::Finish { change_end });

    let trace = ProtocolTrace {
        id: "u3-engine-reset".to_string(),
        schedule: "whole".to_string(),
        setup_changes,
        input_events,
        changes,
    };
    assert_trace_laws(&trace).unwrap();
}
