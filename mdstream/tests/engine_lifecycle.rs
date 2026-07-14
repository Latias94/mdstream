use mdstream::{EngineError, EngineOutput, Options, StreamEngine};
use mdstream_protocol::{
    ApplyOutcome, DocumentLifecycle, ProjectionOp, Reducer, Sequence, SourceCursor,
};

fn apply_output(reducer: &mut Reducer, output: &EngineOutput) {
    for change in output.changes().iter().cloned() {
        let outcome = reducer.apply(change).unwrap();
        assert!(
            matches!(
                outcome,
                ApplyOutcome::Applied { .. } | ApplyOutcome::Recovered { .. }
            ),
            "engine output must be canonical: {outcome:?}"
        );
    }
}

fn assert_engine_matches_reducer(engine: &StreamEngine, reducer: &Reducer) {
    assert_eq!(
        engine.snapshot(),
        reducer
            .document()
            .map(mdstream_protocol::Document::snapshot)
    );
}

#[test]
fn finish_is_terminal_idempotent_and_append_after_finish_is_typed() {
    let mut engine = StreamEngine::new(Options::default());
    let mut reducer = Reducer::new();

    let appended = engine.append("# Hello").unwrap();
    assert_eq!(appended.changes().len(), 1);
    apply_output(&mut reducer, &appended);

    let finished = engine.finish().unwrap();
    assert_eq!(finished.changes().len(), 1);
    assert!(
        finished.changes()[0]
            .operations()
            .contains(&ProjectionOp::FinishDocument)
    );
    apply_output(&mut reducer, &finished);
    assert_eq!(engine.lifecycle(), DocumentLifecycle::Finalized);
    assert_eq!(
        reducer.document().unwrap().lifecycle(),
        DocumentLifecycle::Finalized
    );
    assert_eq!(reducer.document().unwrap().source(), "# Hello");

    let coordinate = engine.coordinate().cloned();
    let snapshot = reducer.document().unwrap().snapshot();
    let engine_snapshot = engine.snapshot();
    assert!(engine.finish().unwrap().is_empty());
    assert_eq!(engine.coordinate(), coordinate.as_ref());

    assert_eq!(engine.append(""), Err(EngineError::Finished));
    assert_eq!(engine.append("late"), Err(EngineError::Finished));
    assert_eq!(engine.append("\n"), Err(EngineError::Finished));
    assert_eq!(engine.coordinate(), coordinate.as_ref());
    assert_eq!(engine.snapshot(), engine_snapshot);
    assert_eq!(reducer.document().unwrap().snapshot(), snapshot);
}

#[test]
fn finishing_an_empty_document_starts_and_finalizes_one_epoch() {
    let mut engine = StreamEngine::new(Options::default());
    let mut reducer = Reducer::new();

    let output = engine.finish().unwrap();
    assert_eq!(output.changes().len(), 1);
    let change = &output.changes()[0];
    assert_eq!(change.sequence(), Sequence::new(0));
    assert!(change.epoch_start().is_some());
    assert!(change.source().suffix.is_empty());
    assert_eq!(change.operations(), &[ProjectionOp::FinishDocument]);
    apply_output(&mut reducer, &output);

    assert_eq!(engine.lifecycle(), DocumentLifecycle::Finalized);
    assert!(engine.finish().unwrap().is_empty());
}

#[test]
fn provisional_frame_identity_survives_growth_and_finalization() {
    let mut engine = StreamEngine::new(Options::default());
    engine.append("a").unwrap();
    let first = engine.snapshot().unwrap().nodes()[0].clone();
    assert_eq!(
        first.stability,
        mdstream_protocol::NodeStability::Provisional
    );

    engine.append("b").unwrap();
    let grown = engine.snapshot().unwrap().nodes()[0].clone();
    assert_eq!(grown.id, first.id);
    assert_ne!(grown.version, first.version);

    let finish = engine.finish().unwrap();
    assert!(matches!(
        finish.changes()[0].operations(),
        [
            ProjectionOp::StabilizeNode { .. },
            ProjectionOp::FinishDocument
        ]
    ));
    let stable = engine.snapshot().unwrap().nodes()[0].clone();
    assert_eq!(stable.id, first.id);
    assert_ne!(stable.version, grown.version);
    assert_eq!(stable.stability, mdstream_protocol::NodeStability::Stable);
}

#[test]
fn reset_from_open_and_finalized_states_emits_empty_linked_epoch_starts() {
    let mut engine = StreamEngine::new(Options::default());
    let mut reducer = Reducer::new();

    apply_output(&mut reducer, &engine.append("old").unwrap());
    let open_predecessor = engine.coordinate().unwrap().clone();
    let open_reset = engine.reset().unwrap();
    assert_linked_empty_reset(&open_reset, &open_predecessor);
    apply_output(&mut reducer, &open_reset);
    assert_eq!(engine.lifecycle(), DocumentLifecycle::Open);
    assert_eq!(engine.coordinate().unwrap().sequence, Sequence::new(0));
    assert_eq!(
        engine.coordinate().unwrap().source_cursor,
        SourceCursor::new(0)
    );

    apply_output(&mut reducer, &engine.append("middle").unwrap());
    apply_output(&mut reducer, &engine.finish().unwrap());
    let finalized_predecessor = engine.coordinate().unwrap().clone();
    let finalized_reset = engine.reset().unwrap();
    assert_linked_empty_reset(&finalized_reset, &finalized_predecessor);
    apply_output(&mut reducer, &finalized_reset);

    apply_output(&mut reducer, &engine.append("new").unwrap());
    apply_output(&mut reducer, &engine.finish().unwrap());
    let document = reducer.document().unwrap();
    assert_eq!(document.source(), "new");
    assert_eq!(document.lifecycle(), DocumentLifecycle::Finalized);
}

fn assert_linked_empty_reset(output: &EngineOutput, predecessor: &mdstream_protocol::Coordinate) {
    assert_eq!(output.changes().len(), 1);
    let reset = &output.changes()[0];
    assert_eq!(reset.sequence(), Sequence::new(0));
    assert_eq!(
        reset
            .epoch_start()
            .and_then(|start| start.predecessor.as_ref()),
        Some(predecessor)
    );
    assert!(reset.source().suffix.is_empty());
    assert!(reset.operations().is_empty());
}

#[test]
fn empty_append_preserves_pending_cr_and_finish_resolves_trailing_cr() {
    let mut split = StreamEngine::new(Options::default());
    let mut split_reducer = Reducer::new();
    apply_output(&mut split_reducer, &split.append("A\r").unwrap());
    let before_empty = split.coordinate().cloned();
    assert!(split.append("").unwrap().is_empty());
    assert_eq!(split.coordinate(), before_empty.as_ref());
    apply_output(&mut split_reducer, &split.append("\nB").unwrap());
    apply_output(&mut split_reducer, &split.finish().unwrap());

    let mut whole = StreamEngine::new(Options::default());
    let mut whole_reducer = Reducer::new();
    apply_output(&mut whole_reducer, &whole.append("A\r\nB").unwrap());
    apply_output(&mut whole_reducer, &whole.finish().unwrap());

    assert_eq!(split_reducer.document().unwrap().source(), "A\nB");
    assert_eq!(
        split_reducer.document().unwrap().source(),
        whole_reducer.document().unwrap().source()
    );

    let mut trailing = StreamEngine::new(Options::default());
    let mut trailing_reducer = Reducer::new();
    apply_output(&mut trailing_reducer, &trailing.append("A\r").unwrap());
    let finish = trailing.finish().unwrap();
    assert_eq!(finish.changes()[0].source().suffix, "\n");
    assert!(matches!(
        finish.changes()[0].operations(),
        [
            ProjectionOp::ReplaceNode { .. },
            ProjectionOp::FinishDocument
        ]
    ));
    apply_output(&mut trailing_reducer, &finish);
    assert_eq!(trailing_reducer.document().unwrap().source(), "A\n");
}

#[test]
fn producer_snapshot_matches_an_independent_reducer_after_every_transition() {
    let mut engine = StreamEngine::new(Options::default());
    let mut reducer = Reducer::new();

    for chunk in ["A\r", "", "\n", "B"] {
        let output = engine.append(chunk).unwrap();
        apply_output(&mut reducer, &output);
        assert_engine_matches_reducer(&engine, &reducer);
    }

    let reset = engine.reset().unwrap();
    apply_output(&mut reducer, &reset);
    assert_engine_matches_reducer(&engine, &reducer);

    let appended = engine.append("new").unwrap();
    apply_output(&mut reducer, &appended);
    assert_engine_matches_reducer(&engine, &reducer);

    let finished = engine.finish().unwrap();
    apply_output(&mut reducer, &finished);
    assert_engine_matches_reducer(&engine, &reducer);
}
