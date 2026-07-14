use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

use mdstream_conformance::{
    ChunkSchedule, FIXTURE_SCHEMA, ProtocolTrace, TraceInputEvent, assert_epoch_reset_isolation,
    assert_fixture_protocol, assert_fork_snapshot_recovery, assert_gap_snapshot_recovery,
    assert_last_retry_idempotent, assert_older_change_stale, exhaustive_utf8_partitions,
    load_fixture_dir, replay_protocol_trace, source_only_trace,
};
use mdstream_protocol::{
    ApplyOutcome, BlockQuoteKind, ChangeId, ChangeSet, ChildList, ChildListOwner, CitationProtocol,
    ContentKind, ContentNode, Epoch, LinkStyle, NodeId, NodeStability, NodeVersion, ProjectionOp,
    ProtocolLimits, RecoveryReason, Reducer, ResourceId, ResourceRef, ResourceVersion,
    SemanticResource, SemanticResourceKind, SemanticText, Sequence, SourceCursor, SourceDelta,
    SourceRange, StructureVersion, TableAlignment,
};

fn corpus_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance")
}

fn change_id(value: &str) -> ChangeId {
    ChangeId::new(value).unwrap()
}

fn start(epoch: u64, id: &str, source: &str) -> ChangeSet {
    ChangeSet::start_epoch(
        Epoch::new(epoch),
        change_id(id),
        None,
        SourceDelta::append(SourceCursor::new(0), source),
        vec![],
    )
    .unwrap()
}

fn next(epoch: u64, sequence: u64, cursor: u64, id: &str, suffix: &str) -> ChangeSet {
    ChangeSet::new(
        Epoch::new(epoch),
        Sequence::new(sequence),
        change_id(id),
        SourceDelta::append(SourceCursor::new(cursor), suffix),
        vec![],
    )
    .unwrap()
}

fn finish(epoch: u64, sequence: u64, cursor: u64, id: &str) -> ChangeSet {
    ChangeSet::new(
        Epoch::new(epoch),
        Sequence::new(sequence),
        change_id(id),
        SourceDelta::unchanged(SourceCursor::new(cursor)),
        vec![ProjectionOp::FinishDocument],
    )
    .unwrap()
}

fn character_trace() -> ProtocolTrace {
    ProtocolTrace {
        id: "characters".to_string(),
        schedule: "characters".to_string(),
        setup_changes: 0,
        input_events: vec![
            TraceInputEvent::Append {
                chunk: "a".to_string(),
                change_end: 1,
            },
            TraceInputEvent::Append {
                chunk: "b".to_string(),
                change_end: 2,
            },
            TraceInputEvent::Append {
                chunk: "c".to_string(),
                change_end: 3,
            },
            TraceInputEvent::Finish { change_end: 4 },
        ],
        changes: vec![
            start(1, "chars:start", "a"),
            next(1, 1, 1, "chars:1", "b"),
            next(1, 2, 2, "chars:2", "c"),
            finish(1, 3, 3, "chars:finish"),
        ],
    }
}

fn reset_trace() -> ProtocolTrace {
    let first = start(1, "reset:old:start", "old");
    let second = next(1, 1, 3, "reset:old:1", "!");
    let mut producer = Reducer::new();
    producer.apply(first.clone()).unwrap();
    producer.apply(second.clone()).unwrap();
    let predecessor = producer.document().unwrap().coordinate().clone();
    let reset = ChangeSet::start_epoch(
        Epoch::new(2),
        change_id("reset:new:start"),
        Some(predecessor),
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![],
    )
    .unwrap();
    let appended = next(2, 1, 0, "reset:new:1", "new");
    ProtocolTrace {
        id: "reset".to_string(),
        schedule: "whole".to_string(),
        setup_changes: 2,
        input_events: vec![
            TraceInputEvent::Reset { change_end: 3 },
            TraceInputEvent::Append {
                chunk: "new".to_string(),
                change_end: 4,
            },
            TraceInputEvent::Finish { change_end: 5 },
        ],
        changes: vec![
            first,
            second,
            reset,
            appended,
            finish(2, 2, 3, "reset:new:finish"),
        ],
    }
}

fn checked_in_reset_fixture() -> mdstream_conformance::Fixture {
    load_fixture_dir(corpus_root().join("fixtures"))
        .unwrap()
        .into_iter()
        .find(|fixture| fixture.id == "protocol.epoch-reset")
        .unwrap()
}

#[test]
fn corpus_schema_is_valid_and_every_fixture_conforms() {
    let root = corpus_root();
    let schema_path = root.join("schemas/fixture.schema.json");
    let schema: serde_json::Value =
        serde_json::from_slice(&fs::read(&schema_path).unwrap()).unwrap();
    jsonschema::meta::validate(&schema).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();

    let fixture_dir = root.join("fixtures");
    let mut paths = fs::read_dir(&fixture_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    paths.sort();
    assert!(!paths.is_empty(), "conformance corpus must not be empty");

    for path in paths {
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let errors = validator
            .iter_errors(&value)
            .map(|error| format!("{}: {error}", error.instance_path))
            .collect::<Vec<_>>();
        assert!(
            errors.is_empty(),
            "{} failed JSON Schema validation:\n{}",
            path.display(),
            errors.join("\n")
        );
    }

    let fixtures = load_fixture_dir(fixture_dir).unwrap();
    assert!(
        fixtures
            .iter()
            .all(|fixture| fixture.schema == FIXTURE_SCHEMA)
    );
}

#[test]
fn checked_in_protocol_traces_replay_to_their_normalized_goldens() {
    let fixtures = load_fixture_dir(corpus_root().join("fixtures")).unwrap();
    let protocol_fixtures = fixtures
        .iter()
        .filter(|fixture| !fixture.traces.is_empty())
        .collect::<Vec<_>>();
    assert!(
        protocol_fixtures.len() >= 2,
        "corpus must include linear and reset protocol fixtures"
    );
    for fixture in protocol_fixtures {
        assert_fixture_protocol(fixture).unwrap();
    }
}

#[test]
fn synthetic_trace_exercises_retry_stale_gap_fork_and_recovery_laws() {
    let trace = character_trace();
    assert_eq!(
        replay_protocol_trace(&trace)
            .unwrap()
            .final_snapshot
            .source(),
        "abc"
    );
    assert_last_retry_idempotent(&trace).unwrap();
    assert_older_change_stale(&trace).unwrap();
    assert_gap_snapshot_recovery(&trace, 1).unwrap();
    assert_fork_snapshot_recovery(&trace, 1).unwrap();

    let mut reducer = Reducer::new();
    for change in trace.changes {
        reducer.apply(change).unwrap();
    }
    let document = reducer.document().unwrap();
    let unannounced = ChangeSet::new(
        Epoch::new(2),
        Sequence::new(1),
        change_id("unannounced:future"),
        SourceDelta::unchanged(document.coordinate().source_cursor),
        vec![ProjectionOp::FinishDocument],
    )
    .unwrap();
    assert!(matches!(
        reducer.apply(unannounced).unwrap(),
        ApplyOutcome::RecoveryRequired {
            reason: RecoveryReason::UnannouncedEpoch { .. },
            ..
        }
    ));
}

#[test]
fn reset_trace_separates_epochs_and_rejects_delayed_prior_epoch_changes() {
    let trace = reset_trace();
    assert_epoch_reset_isolation(&trace).unwrap();
    let report = replay_protocol_trace(&trace).unwrap();
    assert_eq!(report.final_snapshot.coordinate().epoch, Epoch::new(2));
    assert_eq!(report.final_snapshot.source(), "new");
}

#[test]
fn reset_fixture_rejects_nonempty_source_inexact_predecessor_and_multiple_changes() {
    let fixture = checked_in_reset_fixture();
    let reset_index = fixture.traces[0].setup_changes;
    let reset = &fixture.traces[0].changes[reset_index];
    let predecessor = reset.epoch_start().unwrap().predecessor.clone();

    let mut nonempty_source = fixture.clone();
    nonempty_source.traces[0].changes[reset_index] = ChangeSet::start_epoch(
        Epoch::new(2),
        change_id("reset:nonempty-source"),
        predecessor.clone(),
        SourceDelta::append(SourceCursor::new(0), "unexpected"),
        vec![],
    )
    .unwrap();
    let error = nonempty_source.validate().unwrap_err().to_string();
    assert!(error.contains("reset epoch start must be empty"), "{error}");

    let mut inexact_predecessor = fixture.clone();
    inexact_predecessor.traces[0].changes[reset_index] = ChangeSet::start_epoch(
        Epoch::new(2),
        change_id("reset:inexact-predecessor"),
        None,
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![],
    )
    .unwrap();
    let error = inexact_predecessor.validate().unwrap_err().to_string();
    assert!(error.contains("predecessor"), "{error}");

    let mut multiple_changes = fixture;
    multiple_changes.traces[0].input_events[0] = TraceInputEvent::Reset {
        change_end: reset_index + 2,
    };
    let error = multiple_changes.validate().unwrap_err().to_string();
    assert!(
        error.contains("reset must emit exactly one change"),
        "{error}"
    );
}

#[test]
fn compatibility_profiles_are_narrow_and_pinned_to_upstream_versions() {
    let fixtures = load_fixture_dir(corpus_root().join("fixtures")).unwrap();
    let profiles = fixtures
        .iter()
        .map(|fixture| fixture.profile.id.as_str())
        .collect::<BTreeSet<_>>();
    for expected in [
        "streamdown.block-framing/2.5.0",
        "remend.pending-repair/1.3.0",
        "incremark.final-ast/0.3.10+marked-default",
    ] {
        assert!(profiles.contains(expected), "missing profile {expected}");
    }
}

#[test]
fn fixture_contract_cannot_skip_claimed_protocol_schedules_or_goldens() {
    let fixtures = load_fixture_dir(corpus_root().join("fixtures")).unwrap();
    let fixture = fixtures
        .into_iter()
        .find(|fixture| fixture.id == "protocol.linear-source")
        .unwrap();

    let mut no_traces = fixture.clone();
    no_traces.traces.clear();
    assert!(no_traces.validate().is_err());

    let mut missing_schedule = fixture.clone();
    missing_schedule.traces.pop();
    assert!(missing_schedule.validate().is_err());

    let mut mislabeled_input = fixture.clone();
    mislabeled_input.traces[0].input_events[0] = TraceInputEvent::Append {
        chunk: "a".to_string(),
        change_end: 1,
    };
    assert!(mislabeled_input.validate().is_err());

    let mut append_owns_finish = fixture.clone();
    append_owns_finish.traces[0].input_events[0] = TraceInputEvent::Append {
        chunk: "abc".to_string(),
        change_end: 2,
    };
    append_owns_finish.traces[0].input_events[1] = TraceInputEvent::Finish { change_end: 2 };
    assert!(append_owns_finish.validate().is_err());

    let mut setup_owns_reset = checked_in_reset_fixture();
    setup_owns_reset.traces[0].setup_changes = 3;
    assert!(setup_owns_reset.validate().is_err());

    let mut no_normalized_golden = fixture;
    no_normalized_golden.expected.normalized_snapshot = None;
    no_normalized_golden.expected.pending_projection = Some("not a protocol oracle".to_string());
    assert!(no_normalized_golden.validate().is_err());
}

#[test]
fn empty_append_events_do_not_advance_the_change_boundary() {
    let trace = source_only_trace(
        "crlf-with-empty-event",
        "byte-cuts",
        Epoch::new(1),
        ["A\r", "", "\n"],
    )
    .unwrap();
    assert_eq!(
        trace
            .input_events
            .iter()
            .map(TraceInputEvent::change_end)
            .collect::<Vec<_>>(),
        vec![1, 1, 2, 3]
    );
    assert_eq!(trace.changes.len(), 3);

    let mut fixture = load_fixture_dir(corpus_root().join("fixtures"))
        .unwrap()
        .into_iter()
        .find(|fixture| fixture.id == "protocol.linear-source")
        .unwrap();
    let trace_index = fixture
        .traces
        .iter()
        .position(|trace| trace.schedule == "characters")
        .unwrap();
    fixture.traces[trace_index].input_events.insert(
        1,
        TraceInputEvent::Append {
            chunk: String::new(),
            change_end: 1,
        },
    );
    fixture.validate().unwrap();

    fixture.traces[trace_index].input_events[1] = TraceInputEvent::Append {
        chunk: String::new(),
        change_end: 2,
    };
    let error = fixture.validate().unwrap_err().to_string();
    assert!(error.contains("empty append emitted a change"), "{error}");
}

#[test]
fn json_schema_and_serde_reject_the_same_malformed_protocol_shapes() {
    let schema: serde_json::Value = serde_json::from_slice(
        &fs::read(corpus_root().join("schemas/fixture.schema.json")).unwrap(),
    )
    .unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let fixture_value: serde_json::Value = serde_json::from_slice(
        &fs::read(corpus_root().join("fixtures/protocol-linear-source.json")).unwrap(),
    )
    .unwrap();

    let mut malformed_operation = fixture_value.clone();
    malformed_operation["traces"][0]["changes"][1]["operations"] = serde_json::json!([{}]);
    assert!(!validator.is_valid(&malformed_operation));
    assert!(serde_json::from_value::<mdstream_conformance::Fixture>(malformed_operation).is_err());

    let mut bypassed_contract = fixture_value.clone();
    bypassed_contract["traces"] = serde_json::json!([]);
    bypassed_contract["expected"] =
        serde_json::json!({"pending_projection": "not a protocol oracle"});
    assert!(!validator.is_valid(&bypassed_contract));
    assert!(
        serde_json::from_value::<mdstream_conformance::Fixture>(bypassed_contract)
            .unwrap()
            .validate()
            .is_err()
    );

    let mut malformed_node = fixture_value.clone();
    malformed_node["expected"]["normalized_snapshot"]["nodes"] = serde_json::json!([{}]);
    assert!(!validator.is_valid(&malformed_node));
    assert!(serde_json::from_value::<mdstream_conformance::Fixture>(malformed_node).is_err());

    let mut overflowing_epoch = fixture_value.clone();
    overflowing_epoch["traces"][0]["changes"][0]["epoch"] =
        serde_json::json!("18446744073709551616");
    assert!(!validator.is_valid(&overflowing_epoch));
    assert!(serde_json::from_value::<mdstream_conformance::Fixture>(overflowing_epoch).is_err());

    let mut invalid_change_id = fixture_value.clone();
    invalid_change_id["traces"][0]["changes"][0]["change_id"] = serde_json::json!("!");
    assert!(!validator.is_valid(&invalid_change_id));
    assert!(serde_json::from_value::<mdstream_conformance::Fixture>(invalid_change_id).is_err());

    for (claim, oracle, expected) in [
        (
            "legacy_block_framing",
            "exact_pending_projection",
            serde_json::json!({"pending_projection": "pending"}),
        ),
        (
            "pending_repair",
            "upstream_predicate",
            serde_json::json!({"upstream_predicates": ["unrelated predicate"]}),
        ),
        (
            "final_ast_characterization",
            "exact_pending_projection",
            serde_json::json!({"pending_projection": "pending"}),
        ),
        (
            "lifecycle_characterization",
            "exact_pending_projection",
            serde_json::json!({"pending_projection": "pending"}),
        ),
    ] {
        let mut unsupported_claim = fixture_value.clone();
        unsupported_claim["profile"]["claim_scope"] = serde_json::json!([claim]);
        unsupported_claim["provenance"]["oracle_kind"] = serde_json::json!(oracle);
        unsupported_claim["traces"] = serde_json::json!([]);
        unsupported_claim["expected"] = expected;
        unsupported_claim["required_checkpoints"] = serde_json::json!([]);
        assert!(
            !validator.is_valid(&unsupported_claim),
            "schema accepted `{claim}` without its required evidence"
        );
        assert!(
            serde_json::from_value::<mdstream_conformance::Fixture>(unsupported_claim)
                .unwrap()
                .validate()
                .is_err(),
            "Rust accepted `{claim}` without its required evidence"
        );
    }

    let mut checkpoint_without_protocol = fixture_value.clone();
    checkpoint_without_protocol["profile"]["claim_scope"] =
        serde_json::json!(["lifecycle_characterization"]);
    checkpoint_without_protocol["provenance"]["oracle_kind"] =
        serde_json::json!("exact_pending_projection");
    checkpoint_without_protocol["traces"] = serde_json::json!([]);
    checkpoint_without_protocol["expected"] = serde_json::json!({"pending_projection": "pending"});
    checkpoint_without_protocol["required_checkpoints"] = serde_json::json!([{
        "id": "orphan",
        "trace": "missing",
        "after_change": 0
    }]);
    assert!(!validator.is_valid(&checkpoint_without_protocol));
    assert!(
        serde_json::from_value::<mdstream_conformance::Fixture>(checkpoint_without_protocol)
            .unwrap()
            .validate()
            .is_err()
    );

    let mut canonical_without_protocol = fixture_value.clone();
    canonical_without_protocol["profile"]["claim_scope"] = serde_json::json!(["pending_repair"]);
    canonical_without_protocol["provenance"]["oracle_kind"] =
        serde_json::json!("canonical_protocol");
    canonical_without_protocol["traces"] = serde_json::json!([]);
    canonical_without_protocol["expected"] = serde_json::json!({"pending_projection": "pending"});
    canonical_without_protocol["required_checkpoints"] = serde_json::json!([]);
    assert!(!validator.is_valid(&canonical_without_protocol));
    assert!(
        serde_json::from_value::<mdstream_conformance::Fixture>(canonical_without_protocol)
            .unwrap()
            .validate()
            .is_err()
    );

    let mut maximum_epoch = fixture_value;
    maximum_epoch["traces"][0]["changes"][0]["epoch"] = serde_json::json!("18446744073709551615");
    assert!(validator.is_valid(&maximum_epoch));
    assert!(serde_json::from_value::<mdstream_conformance::Fixture>(maximum_epoch).is_ok());
}

#[test]
fn protocol_schema_accepts_every_serde_operation_and_content_variant() {
    let schema: serde_json::Value = serde_json::from_slice(
        &fs::read(corpus_root().join("schemas/fixture.schema.json")).unwrap(),
    )
    .unwrap();
    let validator_for = |name: &str| {
        let definition = serde_json::json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$ref": format!("#/$defs/{name}"),
            "$defs": schema["$defs"].clone()
        });
        jsonschema::validator_for(&definition).unwrap()
    };

    let reference = ResourceRef {
        id: ResourceId::new(1),
        version: ResourceVersion::new("resource:v1").unwrap(),
    };
    let mut attributes = BTreeMap::new();
    attributes.insert("role".to_string(), "note".to_string());
    let contents = vec![
        ContentKind::Paragraph {},
        ContentKind::Heading { level: 2 },
        ContentKind::Text {
            text: SemanticText::Source {},
        },
        ContentKind::Emphasis {},
        ContentKind::Strong {},
        ContentKind::Strikethrough {},
        ContentKind::Link {
            target: Some(reference.clone()),
            reference_label: Some("ref".to_string()),
            style: LinkStyle::Reference,
        },
        ContentKind::Image {
            target: Some(reference.clone()),
            reference_label: None,
            style: LinkStyle::Inline,
            alt: SemanticText::Normalized {
                value: "diagram".to_string(),
            },
        },
        ContentKind::InlineCode {
            text: SemanticText::Normalized {
                value: "code".to_string(),
            },
        },
        ContentKind::CodeBlock {
            fenced: true,
            language: Some("rust".to_string()),
            meta: None,
            mermaid: false,
            text: SemanticText::Source {},
        },
        ContentKind::List {
            ordered: true,
            start: Some(1),
            tight: false,
        },
        ContentKind::ListItem {
            checked: Some(true),
        },
        ContentKind::BlockQuote {
            style: BlockQuoteKind::Warning,
        },
        ContentKind::ThematicBreak {},
        ContentKind::Table {
            alignments: vec![TableAlignment::Left, TableAlignment::Right],
        },
        ContentKind::TableHead {},
        ContentKind::TableBody {},
        ContentKind::TableRow {},
        ContentKind::TableCell { column: 1 },
        ContentKind::Html {
            block: true,
            opaque: true,
        },
        ContentKind::Math {
            display: true,
            text: SemanticText::Source {},
        },
        ContentKind::FootnoteDefinition {
            label: "note".to_string(),
        },
        ContentKind::FootnoteReference {
            label: "note".to_string(),
        },
        ContentKind::CitationDefinition {
            key: "paper".to_string(),
            target: reference.clone(),
        },
        ContentKind::CitationReference {
            key: "paper".to_string(),
            target: Some(reference),
        },
        ContentKind::SoftBreak {},
        ContentKind::HardBreak {},
        ContentKind::Custom {
            namespace: "example.rich/1".to_string(),
            name: "aside".to_string(),
            opaque: false,
            attributes,
        },
    ];
    let node_validator = validator_for("contentNode");
    let range = SourceRange::new(SourceCursor::new(0), SourceCursor::new(0));
    for (index, content) in contents.into_iter().enumerate() {
        let node = ContentNode::leaf(
            NodeId::new(u128::try_from(index).unwrap()),
            NodeStability::Stable,
            range,
            content,
        );
        let value = serde_json::to_value(&node).unwrap();
        assert!(
            node_validator.is_valid(&value),
            "schema rejected content kind {}",
            value["content"]["kind"]
        );
        assert_eq!(serde_json::from_value::<ContentNode>(value).unwrap(), node);
    }

    let node = ContentNode::leaf(
        NodeId::new(0),
        NodeStability::Stable,
        range,
        ContentKind::Paragraph {},
    );
    let replacement = ContentNode::leaf(
        NodeId::new(0),
        NodeStability::Stable,
        range,
        ContentKind::Heading { level: 1 },
    );
    let resource = SemanticResource::new(
        ResourceId::new(0),
        SemanticResourceKind::Citation {
            protocol: CitationProtocol::V1,
            key: "paper".to_string(),
            destination: "https://example.test".to_string(),
            title: None,
        },
    );
    let operations = vec![
        ProjectionOp::InsertNode { node: node.clone() },
        ProjectionOp::ReplaceNode {
            node_id: node.id,
            expected_version: node.version.clone(),
            projection: replacement.projection(),
        },
        ProjectionOp::StabilizeNode {
            node_id: node.id,
            expected_version: node.version.clone(),
            new_version: NodeVersion::new("stable").unwrap(),
        },
        ProjectionOp::RemoveNode {
            node_id: node.id,
            expected_version: node.version,
        },
        ProjectionOp::SpliceChildren {
            owner: ChildListOwner::Node {
                node_id: NodeId::new(1),
            },
            expected_version: StructureVersion::new("old").unwrap(),
            start: 0,
            delete_count: 0,
            insert: vec![NodeId::new(2)],
            new_version: StructureVersion::new("new").unwrap(),
        },
        ProjectionOp::InsertResource {
            resource: resource.clone(),
        },
        ProjectionOp::ReplaceResource {
            resource_id: resource.id,
            expected_version: resource.version.clone(),
            resource: resource.clone(),
        },
        ProjectionOp::RemoveResource {
            resource_id: resource.id,
            expected_version: resource.version,
        },
        ProjectionOp::FinishDocument,
    ];
    let operation_validator = validator_for("projectionOp");
    for operation in operations {
        let value = serde_json::to_value(&operation).unwrap();
        assert!(
            operation_validator.is_valid(&value),
            "schema rejected operation {}",
            value["kind"]
        );
        assert_eq!(
            serde_json::from_value::<ProjectionOp>(value).unwrap(),
            operation
        );
    }
}

#[test]
fn content_id_schema_and_wire_cover_the_full_u128_domain() {
    let schema: serde_json::Value = serde_json::from_slice(
        &fs::read(corpus_root().join("schemas/fixture.schema.json")).unwrap(),
    )
    .unwrap();
    let definition = serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": "#/$defs/contentId",
        "$defs": schema["$defs"].clone()
    });
    let validator = jsonschema::validator_for(&definition).unwrap();

    for value in [
        "0",
        "18446744073709551616",
        "340282366920938463463374607431768211455",
    ] {
        assert!(validator.is_valid(&serde_json::json!(value)), "{value}");
    }
    for value in [
        "00",
        "340282366920938463463374607431768211456",
        "999999999999999999999999999999999999999",
    ] {
        assert!(!validator.is_valid(&serde_json::json!(value)), "{value}");
    }

    let node_id = NodeId::new(u128::MAX);
    let resource_id = ResourceId::new(u128::MAX);
    let maximum = serde_json::json!("340282366920938463463374607431768211455");
    assert_eq!(serde_json::to_value(node_id).unwrap(), maximum);
    assert_eq!(serde_json::to_value(resource_id).unwrap(), maximum);
    assert_eq!(
        serde_json::from_value::<NodeId>(maximum.clone()).unwrap(),
        node_id
    );
    assert_eq!(
        serde_json::from_value::<ResourceId>(maximum).unwrap(),
        resource_id
    );
}

#[test]
fn large_snapshot_and_mixed_trace_report_snapshot_and_delta_work_separately() {
    const NODE_COUNT: usize = 10_000;
    const ROUNDS: usize = 10;
    const OPERATION_COUNT: u64 = (NODE_COUNT * ROUNDS) as u64;

    let limits = ProtocolLimits {
        max_nodes: NODE_COUNT,
        max_operations: NODE_COUNT + 1,
        max_children_per_list: NODE_COUNT,
        ..ProtocolLimits::default()
    };
    let empty_range = SourceRange::new(SourceCursor::new(0), SourceCursor::new(0));
    let nodes = (0..NODE_COUNT)
        .map(|index| {
            ContentNode::leaf(
                NodeId::new(u128::try_from(index).unwrap()),
                NodeStability::Stable,
                empty_range,
                ContentKind::Paragraph {},
            )
        })
        .collect::<Vec<_>>();
    let roots = nodes.iter().map(|node| node.id).collect::<Vec<_>>();
    let root_list = ChildList::new(roots.clone());
    let mut bootstrap_operations = nodes
        .into_iter()
        .map(|node| ProjectionOp::InsertNode { node })
        .collect::<Vec<_>>();
    bootstrap_operations.push(ProjectionOp::SpliceChildren {
        owner: ChildListOwner::Document,
        expected_version: ChildList::empty().version,
        start: 0,
        delete_count: 0,
        insert: roots,
        new_version: root_list.version,
    });
    let bootstrap = ChangeSet::start_epoch(
        Epoch::new(1),
        change_id("workload:start"),
        None,
        SourceDelta::unchanged(SourceCursor::new(0)),
        bootstrap_operations,
    )
    .unwrap();

    let mut producer = Reducer::with_limits(limits);
    producer.apply(bootstrap).unwrap();
    let producer_delta_baseline = producer.metrics();
    let checkpoint = producer.document().unwrap().snapshot();
    assert_eq!(checkpoint.nodes().len(), NODE_COUNT);

    let mut consumer = Reducer::with_limits(limits);
    consumer.recover_snapshot(checkpoint).unwrap();
    let snapshot_load = consumer.metrics();
    assert_eq!(snapshot_load.snapshots_validated, 1);
    assert_eq!(snapshot_load.nodes_validated, NODE_COUNT as u64);
    let consumer_delta_baseline = consumer.metrics();

    for round in 0..ROUNDS {
        let operations = (0..NODE_COUNT)
            .map(|index| {
                let id = NodeId::new(u128::try_from(index).unwrap());
                let current = producer.document().unwrap().node(id).unwrap();
                let content = if round % 2 == 0 {
                    ContentKind::Heading { level: 1 }
                } else {
                    ContentKind::Paragraph {}
                };
                let replacement =
                    ContentNode::leaf(id, NodeStability::Stable, empty_range, content);
                ProjectionOp::ReplaceNode {
                    node_id: id,
                    expected_version: current.version.clone(),
                    projection: replacement.projection(),
                }
            })
            .collect::<Vec<_>>();
        let sequence = u64::try_from(round + 1).unwrap();
        let change = ChangeSet::new(
            Epoch::new(1),
            Sequence::new(sequence),
            change_id(&format!("workload:{sequence}")),
            SourceDelta::append(SourceCursor::new(u64::try_from(round).unwrap()), "x"),
            operations,
        )
        .unwrap();
        producer.apply(change.clone()).unwrap();
        consumer.apply(change).unwrap();
    }

    for (name, metrics, baseline) in [
        ("producer", producer.metrics(), producer_delta_baseline),
        ("consumer", consumer.metrics(), consumer_delta_baseline),
    ] {
        assert_eq!(
            metrics.operations_visited - baseline.operations_visited,
            OPERATION_COUNT,
            "{name} operation work"
        );
        assert!(
            metrics.nodes_validated - baseline.nodes_validated <= OPERATION_COUNT,
            "{name} node validation work must be proportional to operations"
        );
        assert!(
            metrics.relationship_steps - baseline.relationship_steps <= OPERATION_COUNT * 5,
            "{name} relationship work must not rescan the 10k-node forest per operation"
        );
    }
    assert_eq!(
        mdstream_conformance::NormalizedSnapshot::from(producer.document().unwrap().snapshot()),
        mdstream_conformance::NormalizedSnapshot::from(consumer.document().unwrap().snapshot())
    );
}

#[test]
fn bounded_short_sources_enumerate_every_utf8_partition() {
    for source in ["A\nB", "A\r\n", "中é🙂"] {
        let schedules = exhaustive_utf8_partitions(source).unwrap();
        let boundary_count = source.char_indices().skip(1).count();
        assert_eq!(schedules.len(), 1usize << boundary_count);
        for schedule in schedules {
            assert_eq!(schedule.slices(source).unwrap().concat(), source);
        }
    }

    let explicit_crlf_cut = ChunkSchedule::ByteCuts { cuts: vec![2] };
    assert_eq!(explicit_crlf_cut.slices("A\r\n").unwrap(), ["A\r", "\n"]);
}
