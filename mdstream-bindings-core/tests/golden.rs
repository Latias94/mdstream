use mdstream_bindings_core::{BindingPayloadKind, EngineSession, ReducerSession};
use mdstream_conformance::{NormalizedSnapshot, load_fixture, replay_protocol_trace};
use mdstream_protocol::{
    ApplyOutcome, ProtocolLimits, Reducer, decode_change_json, decode_snapshot_json,
    encode_change_json,
};

fn payload(output: &mdstream_bindings_core::BindingOutput, kind: BindingPayloadKind) -> &[u8] {
    let matching = output
        .payloads()
        .iter()
        .filter(|payload| payload.kind() == kind)
        .collect::<Vec<_>>();
    assert_eq!(matching.len(), 1, "expected one {kind:?} payload");
    matching[0].bytes()
}

#[test]
fn shared_protocol_goldens_match_the_native_reducer() {
    let fixture = load_fixture(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../conformance/fixtures/protocol-linear-source.json"
    ))
    .unwrap();

    for trace in &fixture.traces {
        let native = replay_protocol_trace(trace).unwrap();
        let mut facade = ReducerSession::new(b"").unwrap();

        for change in &trace.changes {
            let encoded =
                encode_change_json(change, usize::MAX, ProtocolLimits::default()).unwrap();
            let output = facade.apply_change(&encoded).unwrap();
            assert_eq!(
                output.count(BindingPayloadKind::ReducerUpdate),
                1,
                "every applied canonical change has one compact update"
            );
        }

        let snapshot_output = facade.snapshot().unwrap();
        let snapshot = decode_snapshot_json(
            payload(&snapshot_output, BindingPayloadKind::Snapshot),
            usize::MAX,
            ProtocolLimits::default(),
        )
        .unwrap();
        assert_eq!(
            NormalizedSnapshot::from(snapshot),
            native.normalized_final_snapshot()
        );
        assert_eq!(facade.metrics().snapshot_payloads, 1);
    }
}

#[test]
fn engine_changes_are_canonical_and_normal_append_never_serializes_a_snapshot() {
    let mut facade = EngineSession::new(b"").unwrap();
    let mut native_reducer = Reducer::new();

    for chunk in [
        "# Binding golden\n\n",
        "A [link][target].\n\n",
        "[target]: /ok\n",
    ] {
        let output = facade.append(chunk.as_bytes()).unwrap();
        assert_eq!(output.count(BindingPayloadKind::Snapshot), 0);
        for payload in output
            .payloads()
            .iter()
            .filter(|payload| payload.kind() == BindingPayloadKind::Change)
        {
            let change =
                decode_change_json(payload.bytes(), usize::MAX, ProtocolLimits::default()).unwrap();
            assert!(matches!(
                native_reducer.apply(change).unwrap(),
                ApplyOutcome::Applied { .. } | ApplyOutcome::Recovered { .. }
            ));
        }
    }

    let finish = facade.finish().unwrap();
    assert_eq!(finish.count(BindingPayloadKind::Snapshot), 0);
    for payload in finish
        .payloads()
        .iter()
        .filter(|payload| payload.kind() == BindingPayloadKind::Change)
    {
        let change =
            decode_change_json(payload.bytes(), usize::MAX, ProtocolLimits::default()).unwrap();
        native_reducer.apply(change).unwrap();
    }

    let facade_snapshot = decode_snapshot_json(
        payload(&facade.snapshot().unwrap(), BindingPayloadKind::Snapshot),
        usize::MAX,
        ProtocolLimits::default(),
    )
    .unwrap();
    assert_eq!(
        NormalizedSnapshot::from(facade_snapshot),
        NormalizedSnapshot::from(native_reducer.document().unwrap().snapshot())
    );
    assert_eq!(
        facade.metrics().change_payloads,
        native_reducer.metrics().applied_changes
    );
    assert_eq!(facade.metrics().snapshot_payloads, 1);
}
