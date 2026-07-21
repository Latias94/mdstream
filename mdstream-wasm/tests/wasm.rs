#![cfg(target_arch = "wasm32")]

use mdstream_bindings_core::{BindingOutput, BindingPayloadKind, ReducerSession};
use mdstream_conformance::{Fixture, NormalizedSnapshot};
use mdstream_protocol::{
    ApplyOutcome, ChangeId, ChangeSet, ChildList, ChildListOwner, ContentKind, ContentNode,
    DocumentLifecycle, Epoch, NodeId, NodeStability, ProjectionOp, ProtocolLimits, Reducer,
    SourceCursor, SourceDelta, SourceRange, decode_snapshot_json, encode_change_json,
    encode_snapshot_json,
};
use mdstream_wasm::{
    MdstreamEngineSession, MdstreamOutput, MdstreamPayloadKind, MdstreamReducerSession,
    abi_version, binding_options_schema, binding_schema, package_version, transition_schema,
};
use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;

fn take(output: &mut MdstreamOutput, kind: MdstreamPayloadKind) -> Vec<u8> {
    (0..output.payload_count())
        .find(|index| output.kind(*index).unwrap() == kind)
        .map(|index| output.take(index).unwrap())
        .unwrap_or_else(|| panic!("missing {kind:?} payload"))
}

fn error_status(error: &JsValue) -> f64 {
    js_sys::Reflect::get(error, &JsValue::from_str("status"))
        .unwrap()
        .as_f64()
        .unwrap()
}

fn error_detail_code(error: &JsValue) -> String {
    js_sys::Reflect::get(error, &JsValue::from_str("detail_code"))
        .unwrap()
        .as_string()
        .unwrap()
}

fn json_payload(output: &mut MdstreamOutput, kind: MdstreamPayloadKind) -> serde_json::Value {
    serde_json::from_slice(&take(output, kind)).unwrap()
}

fn metric(payload: &[u8], kind: u8, index: usize) -> u64 {
    assert_eq!(&payload[..3], b"MDM");
    assert_eq!(payload[3], 1);
    assert_eq!(payload[4], kind);
    assert!(index < usize::from(payload[5]));
    let start = 6 + index * 8;
    u64::from_le_bytes(payload[start..start + 8].try_into().unwrap())
}

fn encode(change: &ChangeSet) -> Vec<u8> {
    encode_change_json(change, usize::MAX, ProtocolLimits::default()).unwrap()
}

fn single_stable_node_change() -> ChangeSet {
    let node_id = NodeId::new(1);
    let range = SourceRange::new(SourceCursor::new(0), SourceCursor::new(0));
    let roots = ChildList::new(vec![node_id]);
    ChangeSet::start_epoch(
        Epoch::new(1),
        ChangeId::new("wasm:processor:start").unwrap(),
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
    .unwrap()
}

fn initialize_single_stable_node(reducer: &mut MdstreamReducerSession) {
    reducer
        .apply_change(&encode(&single_stable_node_change()))
        .unwrap();
}

fn begin_processor(reducer: &mut MdstreamReducerSession, processor_id: &str) -> String {
    let mut output = reducer
        .begin_processor(
            "1",
            processor_id,
            "v1",
            &format!("{processor_id}.default"),
            false,
            false,
        )
        .unwrap();
    json_payload(&mut output, MdstreamPayloadKind::ProcessorRequest)["request_id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn artifact_view(reducer: &mut MdstreamReducerSession, processor_id: &str) -> MdstreamOutput {
    reducer.artifact_view("1", "1", processor_id).unwrap()
}

fn linear_fixture() -> Fixture {
    let fixture: Fixture = serde_json::from_str(include_str!(
        "../../conformance/fixtures/protocol-linear-source.json"
    ))
    .unwrap();
    fixture.validate().unwrap();
    fixture
}

fn transition_options_json() -> String {
    format!(
        r#"{{
          "schema":"{}",
          "capture_transitions":true,
          "protocol":{{
            "max_source_bytes":"1024",
            "max_nodes":"16",
            "max_resources":"16",
            "max_operations":"32",
            "max_change_structural_items":"64",
            "max_children_per_list":"16"
          }},
          "wire":{{"max_reducer_update_bytes":"1048576"}}
        }}"#,
        binding_options_schema()
    )
}

fn take_core_payload(output: BindingOutput, expected_kind: BindingPayloadKind) -> Vec<u8> {
    let mut payloads = output.into_payloads();
    assert_eq!(payloads.len(), 1);
    let payload = payloads.pop().unwrap();
    assert_eq!(payload.kind(), expected_kind);
    payload.into_bytes()
}

fn assert_wasm_reducer_update_parity(
    expected: BindingOutput,
    mut actual: MdstreamOutput,
) -> Vec<u8> {
    let expected = take_core_payload(expected, BindingPayloadKind::ReducerUpdate);
    assert_eq!(BindingPayloadKind::ReducerUpdate as u32, 3);
    assert_eq!(MdstreamPayloadKind::ReducerUpdate as u32, 3);
    assert_eq!(actual.payload_count(), 1);
    assert_eq!(actual.kind(0).unwrap(), MdstreamPayloadKind::ReducerUpdate);
    let actual = actual.take(0).unwrap();
    assert_eq!(actual, expected);
    actual
}

#[wasm_bindgen_test]
fn metadata_payload_kinds_and_consumption_are_stable() {
    assert_eq!(abi_version(), 1);
    assert_eq!(package_version(), "0.4.0");
    assert_eq!(binding_schema(), "mdstream.bindings/0.4");
    assert_eq!(binding_options_schema(), "mdstream.bindings-options/0.4");
    assert_eq!(transition_schema(), "mdstream.transitions/1");
    assert_eq!(MdstreamPayloadKind::Change as u32, 1);
    assert_eq!(MdstreamPayloadKind::ReducerUpdate as u32, 3);
    assert_eq!(MdstreamPayloadKind::ArtifactView as u32, 9);
    assert_eq!(MdstreamPayloadKind::PendingSourceView as u32, 10);

    let mut engine = MdstreamEngineSession::new(None).unwrap();
    let mut output = engine.append("owned").unwrap();
    assert_eq!(output.payload_count(), 1);
    assert_eq!(output.remaining(), 1);
    let bytes = output.take(0).unwrap();
    assert!(!bytes.is_empty());
    assert_eq!(output.remaining(), 0);
    assert_eq!(error_status(&output.take(0).unwrap_err()), 1.0);

    for _ in 0..32 {
        engine.append(" more").unwrap();
    }
    let change: ChangeSet = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(change.sequence().get(), 0);
}

#[wasm_bindgen_test]
fn reducer_exposes_effective_processor_scheduler_limits() {
    let defaults = MdstreamReducerSession::new(None).unwrap();
    assert_eq!(defaults.processor_max_in_flight_jobs(), 32);
    assert_eq!(defaults.processor_max_queued_candidates(), 256);

    let options = format!(
        r#"{{
          "schema":"{}",
          "processor":{{
            "max_in_flight_jobs":"2",
            "max_slots":"25"
          }}
        }}"#,
        binding_options_schema()
    );
    let custom = MdstreamReducerSession::new(Some(options)).unwrap();
    assert_eq!(custom.processor_max_in_flight_jobs(), 2);
    assert_eq!(custom.processor_max_queued_candidates(), 25);
}

#[wasm_bindgen_test]
fn transition_facts_use_the_existing_reducer_update_transport() {
    let options = transition_options_json();
    let mut reducer = MdstreamReducerSession::new(Some(options)).unwrap();
    let mut output = reducer
        .apply_change(&encode(&single_stable_node_change()))
        .unwrap();
    assert_eq!(output.payload_count(), 1);
    let update = json_payload(&mut output, MdstreamPayloadKind::ReducerUpdate);
    assert_eq!(update["transition"]["schema"], transition_schema());
    assert_eq!(update["transition"]["facts"]["scope"], "continuous");
    assert_eq!(MdstreamPayloadKind::ReducerUpdate as u32, 3);
}

#[wasm_bindgen_test]
fn wasm_reducer_updates_are_byte_identical_to_the_direct_core() {
    let fixture = linear_fixture();
    let trace = fixture
        .traces
        .iter()
        .find(|trace| trace.id == "characters")
        .unwrap();
    let changes = trace.changes.iter().map(encode).collect::<Vec<_>>();

    let mut producer = ReducerSession::new(b"").unwrap();
    for change in changes.iter().take(3) {
        producer.apply_change(change).unwrap();
    }
    let advanced_snapshot =
        take_core_payload(producer.snapshot().unwrap(), BindingPayloadKind::Snapshot);

    let options = transition_options_json();
    let mut direct = ReducerSession::new(options.as_bytes()).unwrap();
    let mut wasm = MdstreamReducerSession::new(Some(options)).unwrap();

    let continuous = assert_wasm_reducer_update_parity(
        direct.apply_change(&changes[0]).unwrap(),
        wasm.apply_change(&changes[0]).unwrap(),
    );
    let continuous: serde_json::Value = serde_json::from_slice(&continuous).unwrap();
    assert_eq!(continuous["transition"]["facts"]["scope"], "continuous");

    let gap = assert_wasm_reducer_update_parity(
        direct.apply_change(&changes[2]).unwrap(),
        wasm.apply_change(&changes[2]).unwrap(),
    );
    let gap: serde_json::Value = serde_json::from_slice(&gap).unwrap();
    assert_eq!(gap["outcome"]["kind"], "recovery_required");

    let full_replace = assert_wasm_reducer_update_parity(
        direct.recover_snapshot(&advanced_snapshot).unwrap(),
        wasm.recover_snapshot(&advanced_snapshot).unwrap(),
    );
    let full_replace: serde_json::Value = serde_json::from_slice(&full_replace).unwrap();
    assert_eq!(full_replace["transition"]["facts"]["scope"], "full_replace");
}

#[wasm_bindgen_test]
fn pending_source_view_is_an_on_demand_reducer_payload() {
    let mut reducer = MdstreamReducerSession::new(None).unwrap();
    assert_eq!(
        reducer
            .pending_source_view()
            .unwrap()
            .count(MdstreamPayloadKind::PendingSourceView),
        0
    );

    let change = ChangeSet::start_epoch(
        Epoch::new(1),
        ChangeId::new("wasm:pending-source:start").unwrap(),
        None,
        SourceDelta::append(SourceCursor::new(0), "abc".to_string()),
        Vec::new(),
    )
    .unwrap();
    reducer.apply_change(&encode(&change)).unwrap();

    let mut output = reducer.pending_source_view().unwrap();
    assert_eq!(output.count(MdstreamPayloadKind::PendingSourceView), 1);
    let view = json_payload(&mut output, MdstreamPayloadKind::PendingSourceView);
    assert_eq!(view["kind"], "pending_source_view");
    assert_eq!(view["range"]["start"], "0");
    assert_eq!(view["range"]["end"], "3");
    assert_eq!(view["text"], "abc");
}

#[wasm_bindgen_test]
fn engine_transport_is_delta_first_explicit_and_structured() {
    let mut engine = MdstreamEngineSession::new(None).unwrap();

    let append = engine.append("# WASM\n\nbody").unwrap();
    assert_eq!(append.count(MdstreamPayloadKind::Change), 1);
    assert_eq!(append.count(MdstreamPayloadKind::Snapshot), 0);

    let finish = engine.finish().unwrap();
    assert_eq!(finish.count(MdstreamPayloadKind::Change), 1);
    assert_eq!(finish.count(MdstreamPayloadKind::Snapshot), 0);

    let mut snapshot = engine.snapshot().unwrap();
    let snapshot = decode_snapshot_json(
        &take(&mut snapshot, MdstreamPayloadKind::Snapshot),
        usize::MAX,
        ProtocolLimits::default(),
    )
    .unwrap();
    assert_eq!(snapshot.lifecycle(), DocumentLifecycle::Finalized);
    assert_eq!(snapshot.source(), "# WASM\n\nbody");

    let error = engine.append("late").unwrap_err();
    assert_eq!(error_status(&error), 6.0);

    let reset = engine.reset().unwrap();
    assert_eq!(reset.count(MdstreamPayloadKind::Change), 1);
    assert_eq!(reset.count(MdstreamPayloadKind::Snapshot), 0);
    assert_eq!(metric(&engine.metrics(), 1, 4), 1);
}

#[wasm_bindgen_test]
fn shared_golden_replays_through_the_wasm_reducer() {
    let fixture = linear_fixture();
    for trace in fixture.traces {
        let mut reducer = MdstreamReducerSession::new(None).unwrap();
        for change in trace.changes {
            let output = reducer.apply_change(&encode(&change)).unwrap();
            assert_eq!(output.count(MdstreamPayloadKind::ReducerUpdate), 1);
            assert_eq!(output.count(MdstreamPayloadKind::Snapshot), 0);
        }
        let mut output = reducer.snapshot().unwrap();
        let snapshot = decode_snapshot_json(
            &take(&mut output, MdstreamPayloadKind::Snapshot),
            usize::MAX,
            ProtocolLimits::default(),
        )
        .unwrap();
        assert_eq!(
            NormalizedSnapshot::from(&snapshot),
            fixture.expected.normalized_snapshot.clone().unwrap(),
            "trace {} diverged",
            trace.id
        );
    }
}

#[wasm_bindgen_test]
fn reducer_retry_fork_gap_and_explicit_recovery_match_native_state() {
    let fixture = linear_fixture();
    let trace = fixture
        .traces
        .iter()
        .find(|trace| trace.id == "characters")
        .unwrap();
    let mut reducer = MdstreamReducerSession::new(None).unwrap();

    reducer.apply_change(&encode(&trace.changes[0])).unwrap();
    let mut retry = reducer.apply_change(&encode(&trace.changes[0])).unwrap();
    assert_eq!(
        json_payload(&mut retry, MdstreamPayloadKind::ReducerUpdate)["outcome"]["kind"],
        "idempotent"
    );

    let mut gap = reducer.apply_change(&encode(&trace.changes[2])).unwrap();
    assert_eq!(reducer.status(), "needs_snapshot");
    assert_eq!(
        json_payload(&mut gap, MdstreamPayloadKind::ReducerUpdate)["outcome"]["kind"],
        "recovery_required"
    );

    let blocked = reducer
        .apply_change(&encode(&trace.changes[3]))
        .unwrap_err();
    assert_eq!(error_status(&blocked), 9.0);

    let mut native = Reducer::new();
    for change in trace.changes.iter().take(3) {
        assert!(matches!(
            native.apply(change.clone()).unwrap(),
            ApplyOutcome::Applied { .. }
        ));
    }
    let snapshot = encode_snapshot_json(
        &native.document().unwrap().snapshot(),
        usize::MAX,
        ProtocolLimits::default(),
    )
    .unwrap();
    reducer.recover_snapshot(&snapshot).unwrap();
    assert_eq!(reducer.status(), "ready");
    reducer.apply_change(&encode(&trace.changes[3])).unwrap();
    let mut final_snapshot = reducer.snapshot().unwrap();
    let final_snapshot = decode_snapshot_json(
        &take(&mut final_snapshot, MdstreamPayloadKind::Snapshot),
        usize::MAX,
        ProtocolLimits::default(),
    )
    .unwrap();
    assert_eq!(
        NormalizedSnapshot::from(&final_snapshot),
        fixture.expected.normalized_snapshot.clone().unwrap()
    );

    let mut forked = MdstreamReducerSession::new(None).unwrap();
    forked.apply_change(&encode(&trace.changes[0])).unwrap();
    let fork = ChangeSet::start_epoch(
        Epoch::new(1),
        ChangeId::new("wasm:fork").unwrap(),
        None,
        SourceDelta::append(SourceCursor::new(0), "a"),
        Vec::new(),
    )
    .unwrap();
    let mut fork = forked.apply_change(&encode(&fork)).unwrap();
    assert_eq!(forked.status(), "needs_snapshot");
    assert_eq!(
        json_payload(&mut fork, MdstreamPayloadKind::ReducerUpdate)["status"]["kind"],
        "needs_snapshot"
    );
}

#[wasm_bindgen_test]
fn maximum_decimal_ids_and_opaque_versions_survive_js_transport() {
    let epoch = Epoch::new(u64::MAX);
    let node_id = NodeId::new(u128::MAX);
    let range = SourceRange::new(SourceCursor::new(0), SourceCursor::new(0));
    let roots = ChildList::new(vec![node_id]);
    let change = ChangeSet::start_epoch(
        epoch,
        ChangeId::new("wasm:max-identifiers").unwrap(),
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

    let mut reducer = MdstreamReducerSession::new(None).unwrap();
    let mut update = reducer.apply_change(&encode(&change)).unwrap();
    let update = json_payload(&mut update, MdstreamPayloadKind::ReducerUpdate);
    assert_eq!(
        update["document"]["coordinate"]["epoch"],
        u64::MAX.to_string()
    );
    assert_eq!(update["document"]["coordinate"]["sequence"], "0");
    assert_eq!(
        update["impact"]["changed_node_ids"][0],
        u128::MAX.to_string()
    );

    let mut view = reducer.node_view(&node_id.to_string()).unwrap();
    let view = json_payload(&mut view, MdstreamPayloadKind::NodeView);
    assert_eq!(view["node"]["id"], u128::MAX.to_string());
    assert!(
        view["node"]["version"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert!(
        view["processor_input_version"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );

    let options = format!(
        r#"{{
          "schema":"{}",
          "capture_transitions":true,
          "protocol":{{
            "max_source_bytes":"0",
            "max_nodes":"1",
            "max_resources":"0",
            "max_operations":"2",
            "max_change_structural_items":"1",
            "max_children_per_list":"1"
          }},
          "wire":{{"max_reducer_update_bytes":"1048576"}}
        }}"#,
        binding_options_schema()
    );
    let mut captured = MdstreamReducerSession::new(Some(options)).unwrap();
    let mut transition = captured.apply_change(&encode(&change)).unwrap();
    let transition = json_payload(&mut transition, MdstreamPayloadKind::ReducerUpdate);
    assert_eq!(
        transition["transition"]["facts"]["nodes"][0]["key"]["epoch"],
        u64::MAX.to_string()
    );
    assert_eq!(
        transition["transition"]["facts"]["nodes"][0]["key"]["node_id"],
        u128::MAX.to_string()
    );
}

#[wasm_bindgen_test]
fn malformed_decimal_inputs_share_the_invalid_argument_contract() {
    let mut reducer = MdstreamReducerSession::new(None).unwrap();
    for value in ["", "-1", "1.0", "18446744073709551616"] {
        let error = reducer.cancel_processor(value).unwrap_err();
        assert_eq!(error_status(&error), 1.0);
        assert_eq!(error_detail_code(&error), "bindings.decimal_id");
    }

    let error = reducer
        .node_view("340282366920938463463374607431768211456")
        .unwrap_err();
    assert_eq!(error_status(&error), 1.0);
    assert_eq!(error_detail_code(&error), "bindings.decimal_id");
}

#[wasm_bindgen_test]
fn typed_processor_transport_preserves_leases_and_owned_binary_payloads() {
    let mut reducer = MdstreamReducerSession::new(None).unwrap();
    initialize_single_stable_node(&mut reducer);

    let text_id = begin_processor(&mut reducer, "test.wasm.text");
    let invalid_id = reducer
        .complete_processor_text("01", "test.wasm.text/1", "text/plain", "invalid request id")
        .unwrap_err();
    assert_eq!(error_status(&invalid_id), 1.0);
    assert_eq!(metric(&reducer.metrics(), 1, 13), 1);

    let invalid_artifact = reducer
        .complete_processor_text(&text_id, "invalid protocol", "text/plain", "retry")
        .unwrap_err();
    assert_eq!(error_status(&invalid_artifact), 1.0);
    assert_eq!(metric(&reducer.processor_metrics(), 2, 1), 1);

    let mut completed = reducer
        .complete_processor_text(&text_id, "test.wasm.text/1", "text/plain", "complete")
        .unwrap();
    assert_eq!(
        json_payload(&mut completed, MdstreamPayloadKind::ProcessorCompletion)["outcome"],
        "applied"
    );
    let mut text_artifact = artifact_view(&mut reducer, "test.wasm.text");
    assert_eq!(
        json_payload(&mut text_artifact, MdstreamPayloadKind::ArtifactView)["artifact"]["payload"]
            ["text"],
        "complete"
    );

    let mut replay = reducer
        .complete_processor_text(&text_id, "test.wasm.text/1", "text/plain", "late")
        .unwrap();
    assert_eq!(
        json_payload(&mut replay, MdstreamPayloadKind::ProcessorCompletion)["outcome"],
        "stale"
    );

    let binary_id = begin_processor(&mut reducer, "test.wasm.binary");
    reducer
        .complete_processor_binary(
            &binary_id,
            "test.wasm.binary/1",
            "application/octet-stream",
            vec![0, 127, 255],
        )
        .unwrap();
    let mut binary_artifact = artifact_view(&mut reducer, "test.wasm.binary");
    assert_eq!(
        json_payload(&mut binary_artifact, MdstreamPayloadKind::ArtifactView)["artifact"]["payload"]
            ["bytes"],
        serde_json::json!([0, 127, 255])
    );

    let failure_id = begin_processor(&mut reducer, "test.wasm.failure");
    let invalid_code = reducer
        .fail_processor(&failure_id, "unknown", "invalid")
        .unwrap_err();
    assert_eq!(error_status(&invalid_code), 1.0);
    assert_eq!(metric(&reducer.processor_metrics(), 2, 1), 1);
    reducer
        .fail_processor(&failure_id, "panic", "processor threw")
        .unwrap();
    let mut failed_artifact = artifact_view(&mut reducer, "test.wasm.failure");
    let failed = json_payload(&mut failed_artifact, MdstreamPayloadKind::ArtifactView);
    assert_eq!(failed["state"], "failed");
    assert_eq!(failed["failure"]["code"], "panic");
    assert_eq!(failed["failure"]["message"], "processor threw");

    let cancel_id = begin_processor(&mut reducer, "test.wasm.cancel");
    let mut cancelled = reducer.cancel_processor(&cancel_id).unwrap();
    assert_eq!(
        json_payload(&mut cancelled, MdstreamPayloadKind::ProcessorCompletion)["outcome"],
        "applied"
    );
    assert_eq!(
        artifact_view(&mut reducer, "test.wasm.cancel").remaining(),
        0
    );
    let mut repeated_cancel = reducer.cancel_processor(&cancel_id).unwrap();
    assert_eq!(
        json_payload(
            &mut repeated_cancel,
            MdstreamPayloadKind::ProcessorCompletion
        )["outcome"],
        "stale"
    );
    assert_eq!(metric(&reducer.metrics(), 1, 13), 0);
    assert_eq!(metric(&reducer.processor_metrics(), 2, 1), 0);
}

#[wasm_bindgen_test]
fn invalid_options_and_oversized_input_fail_without_engine_mutation() {
    let error = MdstreamEngineSession::new(Some("{}".to_string())).unwrap_err();
    assert_eq!(error_status(&error), 3.0);

    let wrong_schema = MdstreamReducerSession::new(Some(
        r#"{"schema":"mdstream.bindings-options/999"}"#.to_string(),
    ))
    .unwrap_err();
    assert_eq!(error_status(&wrong_schema), 5.0);
    assert_eq!(
        error_detail_code(&wrong_schema),
        "bindings.unsupported_options_schema"
    );

    let old_budget = MdstreamReducerSession::new(Some(format!(
        r#"{{"schema":"{}","wire":{{"max_impact_bytes":"1048576"}}}}"#,
        binding_options_schema()
    )))
    .unwrap_err();
    assert_eq!(error_status(&old_budget), 3.0);

    let options = format!(
        r#"{{
          "schema":"{}",
          "wire":{{"max_command_bytes":"4"}}
        }}"#,
        binding_options_schema()
    );
    let mut engine = MdstreamEngineSession::new(Some(options)).unwrap();
    let error = engine.append("12345").unwrap_err();
    assert_eq!(error_status(&error), 11.0);
    assert_eq!(metric(&engine.metrics(), 1, 3), 0);
    assert_eq!(metric(&engine.metrics(), 1, 4), 0);
    assert!(engine.snapshot().unwrap().remaining() == 0);

    let retry = engine.append("ok").unwrap();
    assert_eq!(retry.count(MdstreamPayloadKind::Change), 1);
}
