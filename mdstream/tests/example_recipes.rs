use std::{fs, path::PathBuf};

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
