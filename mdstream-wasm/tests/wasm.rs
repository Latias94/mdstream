#![cfg(target_arch = "wasm32")]

use mdstream_conformance::{Fixture, NormalizedSnapshot};
use mdstream_protocol::{
    ApplyOutcome, ChangeId, ChangeSet, ChildList, ChildListOwner, ContentKind, ContentNode,
    DocumentLifecycle, Epoch, NodeId, NodeStability, ProjectionOp, ProtocolLimits, Reducer,
    SourceCursor, SourceDelta, SourceRange, decode_snapshot_json, encode_change_json,
    encode_snapshot_json,
};
use mdstream_wasm::{
    MdstreamEngineSession, MdstreamOutput, MdstreamPayloadKind, MdstreamReducerSession,
    abi_version, binding_options_schema, binding_schema, package_version,
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

fn initialize_single_stable_node(reducer: &mut MdstreamReducerSession) {
    let node_id = NodeId::new(1);
    let range = SourceRange::new(SourceCursor::new(0), SourceCursor::new(0));
    let roots = ChildList::new(vec![node_id]);
    let change = ChangeSet::start_epoch(
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
    .unwrap();
    reducer.apply_change(&encode(&change)).unwrap();
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

#[wasm_bindgen_test]
fn metadata_payload_kinds_and_consumption_are_stable() {
    assert_eq!(abi_version(), 1);
    assert_eq!(package_version(), "0.4.0");
    assert_eq!(binding_schema(), "mdstream.bindings/0.4");
    assert_eq!(binding_options_schema(), "mdstream.bindings-options/0.4");
    assert_eq!(MdstreamPayloadKind::Change as u32, 1);
    assert_eq!(MdstreamPayloadKind::ArtifactView as u32, 9);

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
