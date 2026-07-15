use mdstream::{CustomBlockSpec, EngineOutput, StreamEngine};
use mdstream_protocol::{ApplyOutcome, ContentKind, Reducer};

fn apply(reducer: &mut Reducer, output: EngineOutput) {
    for change in output.into_changes() {
        assert!(matches!(
            reducer.apply(change).unwrap(),
            ApplyOutcome::Applied { .. } | ApplyOutcome::Recovered { .. }
        ));
    }
}

fn main() {
    let mut engine = StreamEngine::builder()
        .custom_block(CustomBlockSpec::try_new("app.thinking/1", "thinking").unwrap())
        .build()
        .unwrap();
    let mut reducer = Reducer::new();

    apply(
        &mut reducer,
        engine
            .append("<thinking>\nprivate reasoning\n</thinking>\n")
            .unwrap(),
    );
    apply(&mut reducer, engine.finish().unwrap());

    assert!(reducer.document().unwrap().nodes().any(|node| {
        matches!(
            &node.content,
            ContentKind::Custom { namespace, .. } if namespace == "app.thinking/1"
        )
    }));
}
