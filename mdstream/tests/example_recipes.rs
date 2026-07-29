use std::{fs, path::PathBuf};

#[allow(dead_code)]
#[path = "../examples/minimal.rs"]
mod minimal;

use mdstream::StreamEngine;
use mdstream_protocol::{ApplyOutcome, Reducer};

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")
}

fn read_example(name: &str) -> String {
    fs::read_to_string(examples_dir().join(name))
        .unwrap_or_else(|error| panic!("example `{name}` must exist: {error}"))
}

#[test]
fn rust_learning_ladder_has_truthful_recipe_names_and_assertion_modes() {
    let minimal = read_example("minimal.rs");
    let headless = read_example("headless_state.rs");
    let processor = read_example("processor_lifecycle.rs");
    let recovery = read_example("replica_recovery.rs");
    let trace = read_example("transition_trace.rs");
    let custom = read_example("custom_blocks.rs");
    let readme = read_example("README.md");

    assert!(minimal.contains("--assert"));
    assert!(headless.contains("changed_nodes"));
    assert!(headless.contains("removed_nodes"));
    assert!(processor.contains("CompletionOutcome::Stale"));
    assert!(processor.contains("canonical"));
    assert!(recovery.contains("retained_same_floor"));
    assert!(recovery.contains("replaced_advanced"));
    assert!(trace.contains("schedule-local"));
    assert!(custom.contains("canonical"));

    for recipe in [
        "minimal",
        "headless_state",
        "processor_lifecycle",
        "replica_recovery",
        "transition_trace",
        "custom_blocks",
    ] {
        assert!(
            readme.contains(recipe),
            "the Rust example guide must name `{recipe}`"
        );
    }

    assert!(!examples_dir().join("egui_adapter.rs").exists());
    assert!(!examples_dir().join("gpui_adapter.rs").exists());
}

#[test]
fn minimal_provisional_observations_cannot_masquerade_as_each_other() {
    let mermaid = reduce_append("```mermaid\nflowchart LR\n  A --> B\n```\n\n");
    let mermaid = mermaid.document().unwrap();
    assert!(minimal::provisional_observation_matches(
        mermaid,
        "provisional_mermaid_block"
    ));
    assert!(!minimal::provisional_observation_matches(
        mermaid,
        "provisional_citation_definition"
    ));

    let citation = reduce_append("[@engine]: https://docs.rs/mdstream \"mdstream engine\"\n");
    let citation = citation.document().unwrap();
    assert!(minimal::provisional_observation_matches(
        citation,
        "provisional_citation_definition"
    ));
    assert!(!minimal::provisional_observation_matches(
        citation,
        "provisional_mermaid_block"
    ));
}

fn reduce_append(source: &str) -> Reducer {
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    for change in engine.append(source).unwrap().into_changes() {
        assert!(matches!(
            reducer.apply(change).unwrap(),
            ApplyOutcome::Applied { .. } | ApplyOutcome::Recovered { .. }
        ));
    }
    reducer
}
