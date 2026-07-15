use mdstream::{EngineOutput, StreamEngine};
use mdstream_protocol::{ApplyOutcome, Reducer};

fn apply(reducer: &mut Reducer, output: EngineOutput) {
    for change in output.into_changes() {
        assert!(matches!(
            reducer.apply(change).unwrap(),
            ApplyOutcome::Applied { .. } | ApplyOutcome::Recovered { .. }
        ));
    }
}

fn main() {
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();

    for chunk in ["# mdstream\n\n", "A **streaming** document."] {
        apply(&mut reducer, engine.append(chunk).unwrap());
    }
    apply(&mut reducer, engine.finish().unwrap());

    let document = reducer.document().unwrap();
    println!(
        "lifecycle={:?} roots={} nodes={} source_bytes={}",
        document.lifecycle(),
        document.roots().len(),
        document.nodes().len(),
        document.source().len()
    );
}
