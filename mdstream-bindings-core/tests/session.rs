use mdstream_bindings_core::{
    BINDING_OPTIONS_SCHEMA, BINDING_SCHEMA, BindingPayloadKind, BindingStatus, EngineSession,
    ReducerSession, error_payload_json_bytes,
};
use mdstream_conformance::load_fixture;
use mdstream_processors::{
    CitationProcessor, CompletionOutcome, ConfigurationVersion, ContentProcessor, ProcessingPolicy,
    ProcessorArtifact, ProcessorCapabilities, ProcessorDescriptor, ProcessorFailureCode,
    ProcessorResult, run_catching,
};
use mdstream_protocol::{
    ApplyOutcome, ChangeId, ChangeSet, ChildList, ChildListOwner, ContentKind, ContentNode, Epoch,
    NodeId, NodeProjection, NodeStability, ProjectionOp, ProtocolLimits, Reducer, ReducerStatus,
    SemanticText, Sequence, SourceCursor, SourceDelta, SourceRange, encode_change_json,
    encode_snapshot_json,
};

fn payload(output: &mdstream_bindings_core::BindingOutput, kind: BindingPayloadKind) -> &[u8] {
    output
        .payloads()
        .iter()
        .find(|payload| payload.kind() == kind)
        .unwrap_or_else(|| panic!("missing {kind:?} payload"))
        .bytes()
}

fn apply_engine_output(
    reducer: &mut ReducerSession,
    output: &mdstream_bindings_core::BindingOutput,
) {
    for change in output
        .payloads()
        .iter()
        .filter(|payload| payload.kind() == BindingPayloadKind::Change)
    {
        reducer.apply_change(change.bytes()).unwrap();
    }
}

fn initialize_single_stable_node(reducer: &mut ReducerSession) -> NodeId {
    let epoch = Epoch::new(1);
    let node_id = NodeId::new(1);
    let range = SourceRange::new(SourceCursor::new(0), SourceCursor::new(0));
    let roots = ChildList::new(vec![node_id]);
    let change = ChangeSet::start_epoch(
        epoch,
        ChangeId::new("bindings:single-node:start").unwrap(),
        None,
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![
            ProjectionOp::InsertNode {
                node: ContentNode::leaf(
                    node_id,
                    NodeStability::Stable,
                    range,
                    ContentKind::Paragraph {},
                ),
            },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Document,
                expected_version: ChildList::new(Vec::new()).version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![node_id],
                new_version: roots.version().clone(),
            },
        ],
    )
    .unwrap();
    let encoded = encode_change_json(&change, usize::MAX, ProtocolLimits::default()).unwrap();
    reducer.apply_change(&encoded).unwrap();
    node_id
}

fn foreign_text_completion(request_id: u64, protocol: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema": BINDING_SCHEMA,
        "kind": "complete_processor",
        "request_id": request_id.to_string(),
        "outcome": {
            "kind": "text",
            "protocol": protocol,
            "media_type": "text/plain",
            "text": "complete"
        }
    }))
    .unwrap()
}

#[test]
fn lifecycle_commands_are_delta_first_terminal_and_resettable() {
    let mut engine = EngineSession::new(b"").unwrap();

    assert!(engine.append(b"\r").unwrap().is_empty());
    assert!(engine.append(b"").unwrap().is_empty());
    let finish = engine.finish().unwrap();
    assert_eq!(finish.count(BindingPayloadKind::Change), 1);
    assert_eq!(finish.count(BindingPayloadKind::Snapshot), 0);
    assert!(engine.finish().unwrap().is_empty());

    let before = engine.snapshot().unwrap();
    let error = engine.append(b"late").unwrap_err();
    assert_eq!(error.status(), BindingStatus::Terminal);
    assert_eq!(engine.snapshot().unwrap(), before);

    let reset = engine.reset().unwrap();
    assert_eq!(reset.count(BindingPayloadKind::Change), 1);
    assert_eq!(reset.count(BindingPayloadKind::Snapshot), 0);
    assert_eq!(engine.metrics().snapshot_payloads, 2);
}

#[test]
fn reducer_gap_recovers_only_through_an_explicit_snapshot() {
    let fixture = load_fixture(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../conformance/fixtures/protocol-linear-source.json"
    ))
    .unwrap();
    let trace = fixture
        .traces
        .iter()
        .find(|trace| trace.id == "characters")
        .unwrap();
    let encode = |index: usize| {
        encode_change_json(&trace.changes[index], usize::MAX, ProtocolLimits::default()).unwrap()
    };

    let mut facade = ReducerSession::new(b"").unwrap();
    facade.apply_change(&encode(0)).unwrap();
    let gap = facade.apply_change(&encode(2)).unwrap();
    let gap_json: serde_json::Value =
        serde_json::from_slice(payload(&gap, BindingPayloadKind::ReducerUpdate)).unwrap();
    assert_eq!(gap_json["outcome"]["kind"], "recovery_required");
    assert!(matches!(
        facade.status(),
        ReducerStatus::NeedsSnapshot { .. }
    ));

    let blocked = facade.apply_change(&encode(3)).unwrap_err();
    assert_eq!(blocked.status(), BindingStatus::NeedsSnapshot);

    let mut native = Reducer::new();
    for index in 0..=2 {
        assert!(matches!(
            native.apply(trace.changes[index].clone()).unwrap(),
            ApplyOutcome::Applied { .. } | ApplyOutcome::Recovered { .. }
        ));
    }
    let recovery = encode_snapshot_json(
        &native.document().unwrap().snapshot(),
        usize::MAX,
        ProtocolLimits::default(),
    )
    .unwrap();
    facade.recover_snapshot(&recovery).unwrap();
    facade.apply_change(&encode(3)).unwrap();
    assert!(matches!(facade.status(), ReducerStatus::Ready));
    assert_eq!(facade.metrics().decoded_snapshot_payloads, 1);
    assert_eq!(facade.metrics().snapshot_payloads, 0);
}

#[test]
fn pending_source_view_is_bounded_on_demand_and_empty_when_covered() {
    let mut reducer = ReducerSession::new(b"").unwrap();
    assert!(reducer.pending_source_view().unwrap().is_empty());

    let pending = ChangeSet::start_epoch(
        Epoch::new(1),
        ChangeId::new("bindings:pending-source:start").unwrap(),
        None,
        SourceDelta::append(SourceCursor::new(0), "aé".to_string()),
        Vec::new(),
    )
    .unwrap();
    let encoded = encode_change_json(&pending, usize::MAX, ProtocolLimits::default()).unwrap();
    reducer.apply_change(&encoded).unwrap();

    let command = serde_json::json!({
        "schema": BINDING_SCHEMA,
        "kind": "pending_source_view"
    });
    let output = reducer
        .execute(&serde_json::to_vec(&command).unwrap())
        .unwrap();
    assert_eq!(output.count(BindingPayloadKind::PendingSourceView), 1);
    let view: serde_json::Value =
        serde_json::from_slice(payload(&output, BindingPayloadKind::PendingSourceView)).unwrap();
    assert_eq!(view["schema"], BINDING_SCHEMA);
    assert_eq!(view["kind"], "pending_source_view");
    assert_eq!(view["range"]["start"], "0");
    assert_eq!(view["range"]["end"], "3");
    assert_eq!(view["text"], "aé");

    let covered = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(1),
        ChangeId::new("bindings:pending-source:covered").unwrap(),
        SourceDelta::unchanged(SourceCursor::new(3)),
        vec![ProjectionOp::AdvanceProjection {
            expected_cursor: SourceCursor::new(0),
            new_cursor: SourceCursor::new(3),
        }],
    )
    .unwrap();
    let encoded = encode_change_json(&covered, usize::MAX, ProtocolLimits::default()).unwrap();
    reducer.apply_change(&encoded).unwrap();
    assert!(reducer.pending_source_view().unwrap().is_empty());
}

#[test]
fn invalid_options_commands_utf8_and_encoded_inputs_are_atomic() {
    let invalid_options = br#"{
        "schema":"mdstream.bindings-options/0.4",
        "unknown":true
    }"#;
    assert_eq!(
        EngineSession::new(invalid_options).unwrap_err().status(),
        BindingStatus::Options
    );

    let unsafe_wire_bound = format!(
        r#"{{
          "schema":"{BINDING_OPTIONS_SCHEMA}",
          "engine":{{"max_change_bytes":"1024"}},
          "wire":{{"max_encoded_change_bytes":"6143"}}
        }}"#
    );
    assert_eq!(
        EngineSession::new(unsafe_wire_bound.as_bytes())
            .unwrap_err()
            .status(),
        BindingStatus::Options
    );

    let exact_wire_bound = format!(
        r#"{{
          "schema":"{BINDING_OPTIONS_SCHEMA}",
          "engine":{{"max_change_bytes":"2048"}},
          "wire":{{"max_encoded_change_bytes":"12288"}}
        }}"#
    );
    let mut bounded_engine = EngineSession::new(exact_wire_bound.as_bytes()).unwrap();
    assert_eq!(
        bounded_engine
            .append("x".repeat(4096).as_bytes())
            .unwrap_err()
            .status(),
        BindingStatus::ResourceLimit
    );
    assert!(bounded_engine.snapshot().unwrap().is_empty());
    assert_eq!(
        bounded_engine
            .append(b"retry")
            .unwrap()
            .count(BindingPayloadKind::Change),
        1
    );

    let mut engine = EngineSession::new(b"").unwrap();
    assert_eq!(
        engine.append(&[0xff]).unwrap_err().status(),
        BindingStatus::Utf8
    );
    assert!(engine.snapshot().unwrap().is_empty());
    assert_eq!(engine.metrics().change_payloads, 0);

    let reducer_options = format!(
        r#"{{
          "schema":"{BINDING_OPTIONS_SCHEMA}",
          "wire":{{"max_encoded_change_bytes":"16"}}
        }}"#
    );
    let mut reducer = ReducerSession::new(reducer_options.as_bytes()).unwrap();
    let oversized = vec![b' '; 17];
    assert_eq!(
        reducer.apply_change(&oversized).unwrap_err().status(),
        BindingStatus::ResourceLimit
    );
    assert!(matches!(reducer.status(), ReducerStatus::Uninitialized));

    let invalid_command = format!(r#"{{"schema":"{BINDING_SCHEMA}","kind":"unknown"}}"#);
    assert_eq!(
        reducer
            .execute(invalid_command.as_bytes())
            .unwrap_err()
            .status(),
        BindingStatus::Command
    );
    assert!(matches!(reducer.status(), ReducerStatus::Uninitialized));

    let numeric_limit = format!(
        r#"{{
          "schema":"{BINDING_OPTIONS_SCHEMA}",
          "wire":{{"max_encoded_change_bytes":16}}
        }}"#
    );
    assert_eq!(
        ReducerSession::new(numeric_limit.as_bytes())
            .unwrap_err()
            .status(),
        BindingStatus::Options
    );

    let undersized_reducer_update = format!(
        r#"{{
          "schema":"{BINDING_OPTIONS_SCHEMA}",
          "wire":{{"max_reducer_update_bytes":"1"}}
        }}"#
    );
    assert_eq!(
        ReducerSession::new(undersized_reducer_update.as_bytes())
            .unwrap_err()
            .status(),
        BindingStatus::Options
    );

    let undersized_view = format!(
        r#"{{
          "schema":"{BINDING_OPTIONS_SCHEMA}",
          "wire":{{"max_view_bytes":"1"}}
        }}"#
    );
    assert_eq!(
        ReducerSession::new(undersized_view.as_bytes())
            .unwrap_err()
            .status(),
        BindingStatus::Options
    );

    let oversized_embedded_change = serde_json::json!({
        "schema": BINDING_SCHEMA,
        "kind": "apply_change",
        "change": { "padding": "x".repeat(64) }
    });
    assert_eq!(
        reducer
            .execute(&serde_json::to_vec(&oversized_embedded_change).unwrap())
            .unwrap_err()
            .status(),
        BindingStatus::ResourceLimit
    );
    assert!(matches!(reducer.status(), ReducerStatus::Uninitialized));
}

#[test]
fn command_and_error_envelopes_are_versioned_and_transport_neutral() {
    let mut engine = EngineSession::new(b"").unwrap();
    let append = serde_json::json!({
        "schema": BINDING_SCHEMA,
        "kind": "append",
        "chunk": "command path"
    });
    let changes = engine
        .execute(&serde_json::to_vec(&append).unwrap())
        .unwrap();
    let change: serde_json::Value =
        serde_json::from_slice(payload(&changes, BindingPayloadKind::Change)).unwrap();

    let mut reducer = ReducerSession::new(b"").unwrap();
    let apply = serde_json::json!({
        "schema": BINDING_SCHEMA,
        "kind": "apply_change",
        "change": change
    });
    let update = reducer
        .execute(&serde_json::to_vec(&apply).unwrap())
        .unwrap();
    assert_eq!(update.count(BindingPayloadKind::ReducerUpdate), 1);

    let unknown_field = serde_json::json!({
        "schema": BINDING_SCHEMA,
        "kind": "finish",
        "unexpected": true
    });
    let error = engine
        .execute(&serde_json::to_vec(&unknown_field).unwrap())
        .unwrap_err();
    assert_eq!(error.status(), BindingStatus::Command);
    let envelope: serde_json::Value =
        serde_json::from_slice(&error_payload_json_bytes(&error)).unwrap();
    assert_eq!(envelope["schema"], BINDING_SCHEMA);
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["status"], BindingStatus::Command.code());
    assert_eq!(envelope["status_name"], BindingStatus::Command.code_name());

    let wrong_schema = serde_json::json!({
        "schema": "mdstream.bindings/999",
        "kind": "snapshot"
    });
    assert_eq!(
        reducer
            .execute(&serde_json::to_vec(&wrong_schema).unwrap())
            .unwrap_err()
            .status(),
        BindingStatus::UnsupportedSchema
    );
}

#[test]
fn custom_block_options_are_validated_once_before_streaming() {
    let options = format!(
        r#"{{
          "schema":"{BINDING_OPTIONS_SCHEMA}",
          "custom_blocks":[{{
            "namespace":"app.note/1",
            "name":"note",
            "opaque":false,
            "case_insensitive":true
          }}]
        }}"#
    );
    let mut engine = EngineSession::new(options.as_bytes()).unwrap();
    engine.append(b"<note>\n**body**\n</note>\n").unwrap();
    engine.finish().unwrap();
    let snapshot: mdstream_protocol::Snapshot = serde_json::from_slice(payload(
        &engine.snapshot().unwrap(),
        BindingPayloadKind::Snapshot,
    ))
    .unwrap();
    assert!(snapshot.nodes().iter().any(|node| {
        matches!(
            &node.content,
            ContentKind::Custom { namespace, .. } if namespace == "app.note/1"
        )
    }));

    let overlapping = format!(
        r#"{{
          "schema":"{BINDING_OPTIONS_SCHEMA}",
          "custom_blocks":[
            {{"namespace":"app.one/1","name":"note"}},
            {{"namespace":"app.two/1","name":"NOTE","case_insensitive":false}}
          ]
        }}"#
    );
    assert_eq!(
        EngineSession::new(overlapping.as_bytes())
            .unwrap_err()
            .status(),
        BindingStatus::Options
    );
}

#[test]
fn native_and_generic_processor_results_use_a_separate_artifact_plane() {
    let mut engine = EngineSession::new(b"").unwrap();
    let mut reducer = ReducerSession::new(b"").unwrap();
    for chunk in [
        "A citation [@rust].\n\n",
        "[@rust]: https://rust-lang.org \"Rust\"\n",
    ] {
        let output = engine.append(chunk.as_bytes()).unwrap();
        apply_engine_output(&mut reducer, &output);
    }
    let finish = engine.finish().unwrap();
    apply_engine_output(&mut reducer, &finish);

    let snapshot: mdstream_protocol::Snapshot = serde_json::from_slice(payload(
        &reducer.snapshot().unwrap(),
        BindingPayloadKind::Snapshot,
    ))
    .unwrap();
    let citation_id = snapshot
        .nodes()
        .iter()
        .find(|node| matches!(node.content, ContentKind::CitationReference { .. }))
        .unwrap()
        .id;

    let citation = CitationProcessor::new();
    let (request, begin) = reducer
        .begin_native_processor(
            citation.descriptor().clone(),
            citation_id,
            ConfigurationVersion::new("test.default").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    assert_eq!(begin.count(BindingPayloadKind::ProcessorRequest), 1);
    assert_eq!(begin.count(BindingPayloadKind::ArtifactChange), 1);
    assert_eq!(reducer.processor_metrics().pending_changes, 0);
    let slot = request.key().slot().clone();
    let result = run_catching(&citation, &request);
    let (outcome, completed) = reducer.complete_native_processor(result).unwrap();
    assert_eq!(outcome, CompletionOutcome::Applied);
    assert_eq!(completed.count(BindingPayloadKind::ProcessorCompletion), 1);
    assert_eq!(completed.count(BindingPayloadKind::ArtifactChange), 1);
    assert_eq!(reducer.processor_metrics().pending_changes, 0);
    let artifact: serde_json::Value = serde_json::from_slice(payload(
        &reducer.artifact_view(&slot).unwrap(),
        BindingPayloadKind::ArtifactView,
    ))
    .unwrap();
    assert_eq!(artifact["artifact"]["payload"]["kind"], "citation");

    let generic =
        ProcessorDescriptor::new("test.svg", "v1", ProcessorCapabilities::stable_only()).unwrap();
    let (request, _) = reducer
        .begin_native_processor(
            generic,
            citation_id,
            ConfigurationVersion::new("test.svg.default").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let request_id = request.key().generation().get().to_string();
    let generic_slot = request.key().slot().clone();
    let invalid_completion = serde_json::json!({
        "schema": BINDING_SCHEMA,
        "kind": "complete_processor",
        "request_id": request_id,
        "outcome": {
            "kind": "text",
            "protocol": "invalid protocol",
            "media_type": "image/svg+xml",
            "text": "<svg/>"
        }
    });
    assert_eq!(
        reducer
            .execute(&serde_json::to_vec(&invalid_completion).unwrap())
            .unwrap_err()
            .status(),
        BindingStatus::InvalidArgument
    );
    assert_eq!(reducer.processor_metrics().in_flight_jobs, 1);
    let valid_completion = serde_json::json!({
        "schema": BINDING_SCHEMA,
        "kind": "complete_processor",
        "request_id": request.key().generation().get().to_string(),
        "outcome": {
            "kind": "text",
            "protocol": "test.svg/1",
            "media_type": "image/svg+xml",
            "text": "<svg/>"
        }
    });
    let completed = reducer
        .execute(&serde_json::to_vec(&valid_completion).unwrap())
        .unwrap();
    let completion: serde_json::Value =
        serde_json::from_slice(payload(&completed, BindingPayloadKind::ProcessorCompletion))
            .unwrap();
    assert_eq!(completion["outcome"], "applied");
    assert_eq!(reducer.processor_metrics().in_flight_jobs, 0);
    assert_eq!(reducer.processor_metrics().pending_changes, 0);
    let artifact: serde_json::Value = serde_json::from_slice(payload(
        &reducer.artifact_view(&generic_slot).unwrap(),
        BindingPayloadKind::ArtifactView,
    ))
    .unwrap();
    assert_eq!(artifact["artifact"]["payload"]["kind"], "text");
    assert_eq!(artifact["artifact"]["payload"]["text"], "<svg/>");

    let late_descriptor =
        ProcessorDescriptor::new("test.late", "v1", ProcessorCapabilities::stable_only()).unwrap();
    let (late_request, _) = reducer
        .begin_native_processor(
            late_descriptor,
            citation_id,
            ConfigurationVersion::new("test.late.default").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let late_result = ProcessorResult::success(
        late_request.key().clone(),
        ProcessorArtifact::text("test.late/1", "text/plain", "late").unwrap(),
    );

    let reset = engine.reset().unwrap();
    let update = reset
        .payloads()
        .iter()
        .find(|payload| payload.kind() == BindingPayloadKind::Change)
        .unwrap();
    let applied = reducer.apply_change(update.bytes()).unwrap();
    assert!(applied.count(BindingPayloadKind::ArtifactChange) >= 2);
    assert_eq!(reducer.processor_metrics().pending_changes, 0);
    assert!(reducer.artifact_view(&slot).unwrap().is_empty());
    assert!(reducer.artifact_view(&generic_slot).unwrap().is_empty());
    let (outcome, _) = reducer.complete_native_processor(late_result).unwrap();
    assert_eq!(outcome, CompletionOutcome::Stale);
}

#[test]
fn conditional_processor_begin_rejects_stale_coordinates_without_a_lease() {
    let mut reducer = ReducerSession::new(b"").unwrap();
    let node_id = initialize_single_stable_node(&mut reducer);
    let node: serde_json::Value = serde_json::from_slice(payload(
        &reducer.node_view(node_id).unwrap(),
        BindingPayloadKind::NodeView,
    ))
    .unwrap();
    let input_version = node["processor_input_version"].as_str().unwrap();

    let stale = serde_json::json!({
        "schema": BINDING_SCHEMA,
        "kind": "begin_processor_if_current",
        "expected_epoch": "2",
        "node_id": node_id.get().to_string(),
        "expected_input_version": input_version,
        "processor_id": "test.conditional",
        "processor_version": "v1",
        "configuration_version": "test.conditional.default"
    });
    let output = reducer
        .execute(&serde_json::to_vec(&stale).unwrap())
        .unwrap();
    assert!(output.is_empty());
    assert_eq!(reducer.metrics().pending_processor_requests, 0);
    assert_eq!(reducer.processor_metrics().issued_requests, 0);

    let current = serde_json::json!({
        "schema": BINDING_SCHEMA,
        "kind": "begin_processor_if_current",
        "expected_epoch": "1",
        "node_id": node_id.get().to_string(),
        "expected_input_version": input_version,
        "processor_id": "test.conditional",
        "processor_version": "v1",
        "configuration_version": "test.conditional.default"
    });
    let output = reducer
        .execute(&serde_json::to_vec(&current).unwrap())
        .unwrap();
    assert_eq!(output.count(BindingPayloadKind::ProcessorRequest), 1);
    assert_eq!(reducer.metrics().pending_processor_requests, 1);
    assert_eq!(reducer.processor_metrics().issued_requests, 1);
}

#[test]
fn typed_foreign_processor_completion_reuses_the_canonical_lease_path() {
    let mut reducer = ReducerSession::new(b"").unwrap();
    let node_id = initialize_single_stable_node(&mut reducer);
    let descriptor =
        ProcessorDescriptor::new("test.typed", "v1", ProcessorCapabilities::stable_only()).unwrap();
    let (request, _) = reducer
        .begin_native_processor(
            descriptor,
            node_id,
            ConfigurationVersion::new("test.typed.default").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let slot = request.key().slot().clone();

    let invalid = reducer
        .complete_processor_text(
            request.key().generation(),
            "invalid protocol".to_string(),
            "text/plain".to_string(),
            "retriable".to_string(),
        )
        .unwrap_err();
    assert_eq!(invalid.status(), BindingStatus::InvalidArgument);
    assert_eq!(reducer.metrics().pending_processor_requests, 1);
    assert_eq!(reducer.processor_metrics().in_flight_jobs, 1);

    let completed = reducer
        .complete_processor_text(
            request.key().generation(),
            "test.typed/1".to_string(),
            "text/plain".to_string(),
            "typed completion".to_string(),
        )
        .unwrap();
    let completion: serde_json::Value =
        serde_json::from_slice(payload(&completed, BindingPayloadKind::ProcessorCompletion))
            .unwrap();
    assert_eq!(completion["outcome"], "applied");
    assert_eq!(reducer.metrics().pending_processor_requests, 0);
    assert_eq!(reducer.processor_metrics().in_flight_jobs, 0);

    let artifact: serde_json::Value = serde_json::from_slice(payload(
        &reducer.artifact_view(&slot).unwrap(),
        BindingPayloadKind::ArtifactView,
    ))
    .unwrap();
    assert_eq!(artifact["artifact"]["payload"]["text"], "typed completion");

    let replay = reducer
        .complete_processor_text(
            request.key().generation(),
            "test.typed/1".to_string(),
            "text/plain".to_string(),
            "late".to_string(),
        )
        .unwrap();
    let replay: serde_json::Value =
        serde_json::from_slice(payload(&replay, BindingPayloadKind::ProcessorCompletion)).unwrap();
    assert_eq!(replay["outcome"], "stale");

    let binary_descriptor = ProcessorDescriptor::new(
        "test.typed.binary",
        "v1",
        ProcessorCapabilities::stable_only(),
    )
    .unwrap();
    let (binary_request, _) = reducer
        .begin_native_processor(
            binary_descriptor,
            node_id,
            ConfigurationVersion::new("test.typed.binary.default").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let binary_slot = binary_request.key().slot().clone();
    reducer
        .complete_processor_binary(
            binary_request.key().generation(),
            "test.typed.binary/1".to_string(),
            "application/octet-stream".to_string(),
            vec![0, 127, 255],
        )
        .unwrap();
    let binary_artifact: serde_json::Value = serde_json::from_slice(payload(
        &reducer.artifact_view(&binary_slot).unwrap(),
        BindingPayloadKind::ArtifactView,
    ))
    .unwrap();
    assert_eq!(
        binary_artifact["artifact"]["payload"]["bytes"],
        serde_json::json!([0, 127, 255])
    );

    let failure_descriptor = ProcessorDescriptor::new(
        "test.typed.failure",
        "v1",
        ProcessorCapabilities::stable_only(),
    )
    .unwrap();
    let (failure_request, _) = reducer
        .begin_native_processor(
            failure_descriptor,
            node_id,
            ConfigurationVersion::new("test.typed.failure.default").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let failure_slot = failure_request.key().slot().clone();
    reducer
        .fail_processor(
            failure_request.key().generation(),
            ProcessorFailureCode::Panic,
            "processor threw".to_string(),
        )
        .unwrap();
    let failed_artifact: serde_json::Value = serde_json::from_slice(payload(
        &reducer.artifact_view(&failure_slot).unwrap(),
        BindingPayloadKind::ArtifactView,
    ))
    .unwrap();
    assert_eq!(failed_artifact["state"], "failed");
    assert_eq!(failed_artifact["failure"]["code"], "panic");
    assert_eq!(failed_artifact["failure"]["message"], "processor threw");

    let cancel_descriptor = ProcessorDescriptor::new(
        "test.typed.cancel",
        "v1",
        ProcessorCapabilities::stable_only(),
    )
    .unwrap();
    let (cancel_request, _) = reducer
        .begin_native_processor(
            cancel_descriptor,
            node_id,
            ConfigurationVersion::new("test.typed.cancel.default").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let cancel_slot = cancel_request.key().slot().clone();
    let cancelled = reducer
        .cancel_processor(cancel_request.key().generation())
        .unwrap();
    let cancelled: serde_json::Value =
        serde_json::from_slice(payload(&cancelled, BindingPayloadKind::ProcessorCompletion))
            .unwrap();
    assert_eq!(cancelled["outcome"], "applied");
    assert!(reducer.artifact_view(&cancel_slot).unwrap().is_empty());

    let repeated_cancel = reducer
        .cancel_processor(cancel_request.key().generation())
        .unwrap();
    let repeated_cancel: serde_json::Value = serde_json::from_slice(payload(
        &repeated_cancel,
        BindingPayloadKind::ProcessorCompletion,
    ))
    .unwrap();
    assert_eq!(repeated_cancel["outcome"], "stale");
    assert_eq!(reducer.metrics().pending_processor_requests, 0);
    assert_eq!(reducer.processor_metrics().in_flight_jobs, 0);
}

#[test]
fn replaced_foreign_requests_remain_registered_until_their_leases_settle() {
    let options = format!(
        r#"{{
          "schema":"{BINDING_OPTIONS_SCHEMA}",
          "processor":{{"max_in_flight_jobs":"2"}}
        }}"#
    );
    let mut reducer = ReducerSession::new(options.as_bytes()).unwrap();
    let node_id = initialize_single_stable_node(&mut reducer);
    let descriptor =
        ProcessorDescriptor::new("test.replace", "v1", ProcessorCapabilities::stable_only())
            .unwrap();
    let configuration = ConfigurationVersion::new("test.replace.default").unwrap();

    let (first, _) = reducer
        .begin_native_processor(
            descriptor.clone(),
            node_id,
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let (second, _) = reducer
        .begin_native_processor(
            descriptor.clone(),
            node_id,
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    assert_eq!(reducer.metrics().pending_processor_requests, 2);
    assert_eq!(reducer.processor_metrics().in_flight_jobs, 2);

    let completed = reducer
        .execute(&foreign_text_completion(
            first.key().generation().get(),
            "test.replace/1",
        ))
        .unwrap();
    let completion: serde_json::Value =
        serde_json::from_slice(payload(&completed, BindingPayloadKind::ProcessorCompletion))
            .unwrap();
    assert_eq!(completion["outcome"], "stale");
    assert_eq!(reducer.metrics().pending_processor_requests, 1);
    assert_eq!(reducer.processor_metrics().in_flight_jobs, 1);

    let (third, _) = reducer
        .begin_native_processor(
            descriptor,
            node_id,
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    assert_eq!(reducer.metrics().pending_processor_requests, 2);
    assert_eq!(reducer.processor_metrics().in_flight_jobs, 2);

    for request in [second, third] {
        reducer
            .execute(&foreign_text_completion(
                request.key().generation().get(),
                "test.replace/1",
            ))
            .unwrap();
    }
    assert_eq!(reducer.metrics().pending_processor_requests, 0);
    assert_eq!(reducer.processor_metrics().in_flight_jobs, 0);
}

#[test]
fn reducer_reconcile_retires_every_binding_request_whose_native_lease_was_removed() {
    let options = format!(
        r#"{{
          "schema":"{BINDING_OPTIONS_SCHEMA}",
          "processor":{{"max_in_flight_jobs":"2"}}
        }}"#
    );
    let mut reducer = ReducerSession::new(options.as_bytes()).unwrap();
    let node_id = initialize_single_stable_node(&mut reducer);
    let descriptor =
        ProcessorDescriptor::new("test.reconcile", "v1", ProcessorCapabilities::stable_only())
            .unwrap();
    let configuration = ConfigurationVersion::new("test.reconcile.default").unwrap();
    let (first, _) = reducer
        .begin_native_processor(
            descriptor.clone(),
            node_id,
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let (second, _) = reducer
        .begin_native_processor(
            descriptor,
            node_id,
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    reducer.cancel_processor(second.key().generation()).unwrap();
    assert_eq!(reducer.metrics().pending_processor_requests, 1);
    assert_eq!(reducer.processor_metrics().in_flight_jobs, 1);

    let range = SourceRange::new(SourceCursor::new(0), SourceCursor::new(0));
    let node_version = ContentNode::leaf(
        node_id,
        NodeStability::Stable,
        range,
        ContentKind::Paragraph {},
    )
    .version;
    let roots_version = ChildList::new(vec![node_id]).version().clone();
    let removal = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(1),
        ChangeId::new("bindings:single-node:remove").unwrap(),
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Document,
                expected_version: roots_version,
                start: 0,
                delete_count: 1,
                insert: Vec::new(),
                new_version: ChildList::empty().version().clone(),
            },
            ProjectionOp::RemoveNode {
                node_id,
                expected_version: node_version,
            },
        ],
    )
    .unwrap();
    let encoded = encode_change_json(&removal, usize::MAX, ProtocolLimits::default()).unwrap();
    reducer.apply_change(&encoded).unwrap();

    assert_eq!(reducer.processor_metrics().in_flight_jobs, 0);
    assert_eq!(reducer.metrics().pending_processor_requests, 0);
    let stale = reducer
        .execute(&foreign_text_completion(
            first.key().generation().get(),
            "test.reconcile/1",
        ))
        .unwrap();
    let stale: serde_json::Value =
        serde_json::from_slice(payload(&stale, BindingPayloadKind::ProcessorCompletion)).unwrap();
    assert_eq!(stale["outcome"], "stale");
}

#[test]
fn foreign_native_result_with_colliding_generation_preserves_the_local_request() {
    let mut local = ReducerSession::new(b"").unwrap();
    let mut foreign = ReducerSession::new(b"").unwrap();
    let local_node = initialize_single_stable_node(&mut local);
    let foreign_node = initialize_single_stable_node(&mut foreign);

    let (local_request, _) = local
        .begin_native_processor(
            ProcessorDescriptor::new("test.local", "v1", ProcessorCapabilities::stable_only())
                .unwrap(),
            local_node,
            ConfigurationVersion::new("test.local.default").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let (foreign_request, _) = foreign
        .begin_native_processor(
            ProcessorDescriptor::new("test.foreign", "v1", ProcessorCapabilities::stable_only())
                .unwrap(),
            foreign_node,
            ConfigurationVersion::new("test.foreign.default").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    assert_eq!(
        local_request.key().generation(),
        foreign_request.key().generation()
    );

    let wrong_result = ProcessorResult::success(
        foreign_request.key().clone(),
        ProcessorArtifact::text("test.foreign/1", "text/plain", "wrong session").unwrap(),
    );
    let (outcome, _) = local.complete_native_processor(wrong_result).unwrap();
    assert_eq!(outcome, CompletionOutcome::Stale);
    assert_eq!(local.metrics().pending_processor_requests, 1);
    assert_eq!(local.processor_metrics().in_flight_jobs, 1);

    let completed = local
        .execute(&foreign_text_completion(
            local_request.key().generation().get(),
            "test.local/1",
        ))
        .unwrap();
    let completion: serde_json::Value =
        serde_json::from_slice(payload(&completed, BindingPayloadKind::ProcessorCompletion))
            .unwrap();
    assert_eq!(completion["outcome"], "applied");
    assert_eq!(local.metrics().pending_processor_requests, 0);
    assert_eq!(local.processor_metrics().in_flight_jobs, 0);
}

#[test]
fn corrected_nodes_retire_foreign_processor_requests_without_registry_growth() {
    const CORRECTION_ROUNDS: u64 = 1_000;

    fn range() -> SourceRange {
        SourceRange::new(SourceCursor::new(0), SourceCursor::new(0))
    }

    fn projection(round: u64) -> NodeProjection {
        NodeProjection::new(
            NodeStability::Stable,
            range(),
            range(),
            ContentKind::Html {
                block: true,
                text: SemanticText::Normalized {
                    value: round.to_string(),
                },
            },
        )
    }

    let mut reducer = ReducerSession::new(b"").unwrap();
    let epoch = Epoch::new(1);
    let node_id = NodeId::new(1);
    let initial = projection(0);
    let roots = ChildList::new(vec![node_id]);
    let start = ChangeSet::start_epoch(
        epoch,
        ChangeId::new("bindings:processor-registry:start").unwrap(),
        None,
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![
            ProjectionOp::InsertNode {
                node: ContentNode::new(
                    node_id,
                    initial.stability,
                    initial.source,
                    initial.body,
                    Vec::new(),
                    initial.content.clone(),
                ),
            },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Document,
                expected_version: ChildList::new(Vec::new()).version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![node_id],
                new_version: roots.version().clone(),
            },
        ],
    )
    .unwrap();
    let start = encode_change_json(&start, usize::MAX, ProtocolLimits::default()).unwrap();
    reducer.apply_change(&start).unwrap();

    let descriptor =
        ProcessorDescriptor::new("test.registry", "v1", ProcessorCapabilities::stable_only())
            .unwrap();
    let configuration = ConfigurationVersion::new("test.registry.default").unwrap();
    let mut expected_version = initial.version;
    let mut last_request_id = 0;

    for round in 1..=CORRECTION_ROUNDS {
        let (request, _) = reducer
            .begin_native_processor(
                descriptor.clone(),
                node_id,
                configuration.clone(),
                ProcessingPolicy::StableOnly,
            )
            .unwrap();
        last_request_id = request.key().generation().get();
        assert_eq!(reducer.metrics().pending_processor_requests, 1);

        let next = projection(round);
        let change = ChangeSet::new(
            epoch,
            Sequence::new(round),
            ChangeId::new(format!("bindings:processor-registry:{round}")).unwrap(),
            SourceDelta::unchanged(SourceCursor::new(0)),
            vec![ProjectionOp::ReplaceNode {
                node_id,
                expected_version,
                projection: next.clone(),
            }],
        )
        .unwrap();
        let encoded = encode_change_json(&change, usize::MAX, ProtocolLimits::default()).unwrap();
        let output = reducer.apply_change(&encoded).unwrap();
        assert_eq!(output.count(BindingPayloadKind::ArtifactChange), 1);
        assert_eq!(reducer.metrics().pending_processor_requests, 0);
        assert_eq!(reducer.processor_metrics().in_flight_jobs, 0);
        expected_version = next.version;
    }

    let late = serde_json::json!({
        "schema": BINDING_SCHEMA,
        "kind": "complete_processor",
        "request_id": last_request_id.to_string(),
        "outcome": {
            "kind": "text",
            "protocol": "test.registry/1",
            "media_type": "text/plain",
            "text": "late"
        }
    });
    let output = reducer
        .execute(&serde_json::to_vec(&late).unwrap())
        .unwrap();
    let completion: serde_json::Value =
        serde_json::from_slice(payload(&output, BindingPayloadKind::ProcessorCompletion)).unwrap();
    assert_eq!(completion["outcome"], "stale");
}
