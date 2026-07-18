use mdstream_bindings_core::{BINDING_OPTIONS_SCHEMA, BindingPayloadKind, EngineSession};
use mdstream_protocol::{ContentKind, Snapshot};

#[test]
fn omitted_and_explicit_custom_block_booleans_match_the_canonical_defaults() {
    let omitted = format!(
        r#"{{
          "schema":"{BINDING_OPTIONS_SCHEMA}",
          "custom_blocks":[{{"namespace":"app.default/1","name":"note"}}]
        }}"#
    );
    let mut defaulted = EngineSession::new(omitted.as_bytes()).unwrap();
    defaulted.append(b"<NOTE>\nbody\n</NOTE>\n").unwrap();
    defaulted.finish().unwrap();
    let defaulted_snapshot = decode_snapshot(&mut defaulted);
    assert!(defaulted_snapshot.nodes().iter().any(|node| {
        matches!(
            &node.content,
            ContentKind::Custom {
                namespace,
                opaque: true,
                ..
            } if namespace == "app.default/1"
        )
    }));

    let explicit = format!(
        r#"{{
          "schema":"{BINDING_OPTIONS_SCHEMA}",
          "custom_blocks":[{{
            "namespace":"app.explicit/1",
            "name":"note",
            "opaque":false,
            "case_insensitive":false
          }}]
        }}"#
    );
    let mut configured = EngineSession::new(explicit.as_bytes()).unwrap();
    configured.append(b"<note>\nbody\n</note>\n").unwrap();
    configured.finish().unwrap();
    let configured_snapshot = decode_snapshot(&mut configured);
    assert!(configured_snapshot.nodes().iter().any(|node| {
        matches!(
            &node.content,
            ContentKind::Custom {
                namespace,
                opaque: false,
                ..
            } if namespace == "app.explicit/1"
        )
    }));
}

fn decode_snapshot(engine: &mut EngineSession) -> Snapshot {
    let output = engine.snapshot().unwrap();
    let bytes = output
        .payloads()
        .iter()
        .find_map(|payload| {
            (payload.kind() == BindingPayloadKind::Snapshot).then_some(payload.bytes())
        })
        .unwrap();
    serde_json::from_slice(bytes).unwrap()
}
