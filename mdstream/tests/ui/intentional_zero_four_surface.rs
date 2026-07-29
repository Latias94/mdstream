use mdstream::{CustomBlockSpec, EngineOutput, StreamEngine};
use mdstream_protocol::{ApplyOutcome, Reducer};

fn apply(reducer: &mut Reducer, output: EngineOutput) {
    for change in output.into_changes() {
        assert!(matches!(
            reducer.apply(change),
            Ok(ApplyOutcome::Applied { .. } | ApplyOutcome::Recovered { .. })
        ));
    }
}

fn main() {
    let mut engine = StreamEngine::builder()
        .custom_block(CustomBlockSpec::try_new("app.note/1", "note").unwrap())
        .build()
        .unwrap();
    let mut reducer = Reducer::new();
    apply(&mut reducer, engine.append("<note>streaming").unwrap());
    apply(&mut reducer, engine.finish().unwrap());
    let _ = reducer.document().unwrap().snapshot();
}
