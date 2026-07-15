use std::{
    fs,
    path::{Path, PathBuf},
};

use mdstream_conformance::{
    ArtifactStatus, BindingBudgets, StreamingBudget, U7_BASELINE_COMMIT, load_binding_budgets,
    load_streaming_budget,
};
use serde_json::{Value, json};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("conformance crate must live below the repository root")
        .to_path_buf()
}

#[test]
fn checked_in_budget_artifacts_exist() {
    let root = repository_root();
    for relative in [
        "conformance/budgets/streaming.json",
        "bindings/budgets.json",
    ] {
        let path = root.join(relative);
        assert!(
            path.is_file(),
            "required budget artifact is missing: {relative}"
        );
    }
}

#[test]
fn budget_schema_is_valid_and_checked_in_artifacts_conform() {
    let root = repository_root();
    let schema = read_json(root.join("conformance/schemas/budget.schema.json"));
    jsonschema::meta::validate(&schema).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();

    for relative in [
        "conformance/budgets/streaming.json",
        "bindings/budgets.json",
    ] {
        let value = read_json(root.join(relative));
        let errors = validator
            .iter_errors(&value)
            .map(|error| format!("{}: {error}", error.instance_path))
            .collect::<Vec<_>>();
        assert!(
            errors.is_empty(),
            "{relative} failed budget schema validation:\n{}",
            errors.join("\n")
        );
    }

    let streaming = load_streaming_budget(root.join("conformance/budgets/streaming.json")).unwrap();
    assert_eq!(streaming.provenance.source_commit, U7_BASELINE_COMMIT);
    load_binding_budgets(root.join("bindings/budgets.json")).unwrap();
}

#[test]
fn budget_contract_rejects_missing_calibration_provenance() {
    let root = repository_root();
    let schema = read_json(root.join("conformance/schemas/budget.schema.json"));
    let validator = jsonschema::validator_for(&schema).unwrap();
    let mut value = read_json(root.join("conformance/budgets/streaming.json"));
    value.as_object_mut().unwrap().remove("provenance");

    assert!(!validator.is_valid(&value));
    assert!(serde_json::from_value::<StreamingBudget>(value).is_err());
}

#[test]
fn budget_contract_rejects_relative_only_binding_limits() {
    let root = repository_root();
    let schema = read_json(root.join("conformance/schemas/budget.schema.json"));
    let validator = jsonschema::validator_for(&schema).unwrap();
    let mut value = read_json(root.join("bindings/budgets.json"));
    let first = value["artifacts"][0].as_object_mut().unwrap();
    first.remove("ceiling_bytes");
    first.insert("relative_limit_percent".to_string(), json!(15));

    assert!(!validator.is_valid(&value));
    assert!(serde_json::from_value::<BindingBudgets>(value).is_err());
}

#[test]
fn budget_contract_rejects_default_merman_artifacts() {
    let root = repository_root();
    let schema = read_json(root.join("conformance/schemas/budget.schema.json"));
    let validator = jsonschema::validator_for(&schema).unwrap();
    let mut value = read_json(root.join("bindings/budgets.json"));
    value["policy"]["default_artifacts_allow_merman"] = json!(true);

    assert!(!validator.is_valid(&value));
    let contract: BindingBudgets = serde_json::from_value(value).unwrap();
    assert!(contract.validate().is_err());
}

#[test]
fn budget_contract_rejects_a_missing_absolute_artifact_ceiling() {
    let root = repository_root();
    let schema = read_json(root.join("conformance/schemas/budget.schema.json"));
    let validator = jsonschema::validator_for(&schema).unwrap();
    let mut value = read_json(root.join("bindings/budgets.json"));
    value["artifacts"].as_array_mut().unwrap().pop();

    assert!(!validator.is_valid(&value));
    let contract: BindingBudgets = serde_json::from_value(value).unwrap();
    assert!(contract.validate().is_err());
}

#[test]
fn budget_schema_rejects_duplicate_artifact_discriminators() {
    let root = repository_root();
    let schema = read_json(root.join("conformance/schemas/budget.schema.json"));
    let mut value = read_json(root.join("bindings/budgets.json"));
    let mut duplicate = value["artifacts"][0].clone();
    duplicate["status"] = json!("measured");
    duplicate["measurement"] = json!({
        "measured_bytes": 1,
        "artifact_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
        "command": "test measurement"
    });
    value["artifacts"][1] = duplicate;

    assert_binding_rejected(&schema, value, "duplicate artifact discriminator");
}

#[test]
fn budget_contract_rejects_missing_required_measurement() {
    let root = repository_root();
    let schema = read_json(root.join("conformance/schemas/budget.schema.json"));
    let mut value = read_json(root.join("bindings/budgets.json"));
    value["artifacts"][0]
        .as_object_mut()
        .unwrap()
        .remove("measurement");

    assert_binding_rejected(&schema, value, "missing required measurement");
}

#[test]
fn streaming_budget_rejects_noncanonical_provenance() {
    let root = repository_root();
    let schema = read_json(root.join("conformance/schemas/budget.schema.json"));
    let value = read_json(root.join("conformance/budgets/streaming.json"));

    for (pointer, replacement) in [
        ("/provenance/profile", "debug"),
        ("/provenance/command", "cargo run --example calibration"),
        ("/provenance/fixture/schedule", "bytes"),
    ] {
        let mut mutated = value.clone();
        *mutated.pointer_mut(pointer).unwrap() = json!(replacement);
        assert_streaming_rejected(&schema, mutated, pointer);
    }
}

#[test]
fn streaming_budget_rejects_inconsistent_transport_counts() {
    let root = repository_root();
    let schema = read_json(root.join("conformance/schemas/budget.schema.json"));
    let value = read_json(root.join("conformance/budgets/streaming.json"));

    for (left, right) in [
        ("change_count", "applied_changes"),
        ("operation_count", "operations_visited"),
    ] {
        let mut mutated = value.clone();
        mutated["minimal_transport"]["counts"][right] = json!(1);
        assert_streaming_rejected(&schema, mutated, &format!("{left} must match {right}"));
    }
}

#[test]
fn streaming_budget_rejects_every_frozen_measurement_mutation() {
    const POINTERS: &[&str] = &[
        "/provenance/fixture/bytes",
        "/provenance/fixture/chunks",
        "/legacy_0_3/input_bytes",
        "/legacy_0_3/counts/append_calls",
        "/legacy_0_3/counts/update_count",
        "/legacy_0_3/counts/committed_blocks_emitted",
        "/legacy_0_3/counts/pending_observations",
        "/legacy_0_3/counts/reset_count",
        "/legacy_0_3/counts/invalidated_block_ids",
        "/legacy_0_3/counts/observed_text_bytes",
        "/legacy_0_3/counts/final_block_count",
        "/legacy_0_3/counts/retained_buffer_bytes",
        "/minimal_transport/input_bytes",
        "/minimal_transport/counts/chunk_count",
        "/minimal_transport/counts/change_count",
        "/minimal_transport/counts/operation_count",
        "/minimal_transport/counts/applied_changes",
        "/minimal_transport/counts/operations_visited",
        "/minimal_transport/counts/nodes_validated",
        "/minimal_transport/counts/relationship_steps",
        "/minimal_transport/counts/child_ids_copied",
        "/minimal_transport/counts/snapshots_validated",
        "/minimal_transport/wire/encoded_change_bytes",
        "/minimal_transport/wire/encoded_snapshot_bytes",
    ];

    let root = repository_root();
    let schema = read_json(root.join("conformance/schemas/budget.schema.json"));
    let value = read_json(root.join("conformance/budgets/streaming.json"));
    for pointer in POINTERS {
        let mut mutated = value.clone();
        let measurement = mutated.pointer_mut(pointer).unwrap();
        *measurement = json!(measurement.as_u64().unwrap().checked_add(1).unwrap());
        assert_streaming_rejected(&schema, mutated, pointer);
    }
}

#[test]
fn deterministic_replay_comparison_ignores_host_provenance() {
    let root = repository_root();
    let expected = load_streaming_budget(root.join("conformance/budgets/streaming.json")).unwrap();
    let mut replay = expected.clone();
    replay.provenance.os = "linux".to_string();
    replay.provenance.os_version = "different host".to_string();
    replay.provenance.arch = "x86_64".to_string();
    replay.provenance.cpu = "different CPU".to_string();
    replay.provenance.rustc.host = "x86_64-unknown-linux-gnu".to_string();
    replay.provenance.cargo_version = "cargo 1.85.0 (different host)".to_string();

    replay.verify_deterministic_match(&expected).unwrap();
    replay.minimal_transport.wire.encoded_change_bytes += 1;
    assert!(replay.verify_deterministic_match(&expected).is_err());
}

#[test]
fn future_binding_artifacts_remain_pending_without_fake_measurements() {
    let root = repository_root();
    let contract = load_binding_budgets(root.join("bindings/budgets.json")).unwrap();

    assert_eq!(contract.artifacts.len(), 8);
    assert!(contract.artifacts.iter().all(|artifact| {
        artifact.status == ArtifactStatus::Pending && artifact.measurement.is_none()
    }));
}

fn read_json(path: impl AsRef<Path>) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn assert_streaming_rejected(schema: &Value, value: Value, mutation: &str) {
    let validator = jsonschema::validator_for(schema).unwrap();
    assert!(
        !validator.is_valid(&value),
        "schema accepted streaming mutation {mutation}"
    );
    let contract: StreamingBudget = serde_json::from_value(value).unwrap();
    assert!(
        contract.validate().is_err(),
        "Rust accepted streaming mutation {mutation}"
    );
}

fn assert_binding_rejected(schema: &Value, value: Value, mutation: &str) {
    let validator = jsonschema::validator_for(schema).unwrap();
    assert!(
        !validator.is_valid(&value),
        "schema accepted binding mutation {mutation}"
    );
    if let Ok(contract) = serde_json::from_value::<BindingBudgets>(value) {
        assert!(
            contract.validate().is_err(),
            "Rust accepted binding mutation {mutation}"
        );
    }
}
