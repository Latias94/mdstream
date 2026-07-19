use mdstream_bindings_core::{
    BINDING_OPTIONS_SCHEMA, BindingPayloadKind, BindingStatus, EngineSession, ReducerSession,
};
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

fn bounded_reducer_options(capture_transitions: Option<&str>, max_update_bytes: usize) -> String {
    let capture = capture_transitions
        .map(|value| format!(r#","capture_transitions":{value}"#))
        .unwrap_or_default();
    format!(
        r#"{{
          "schema":"{BINDING_OPTIONS_SCHEMA}"{capture},
          "protocol":{{
            "max_source_bytes":"1",
            "max_nodes":"1",
            "max_resources":"1",
            "max_operations":"1",
            "max_change_structural_items":"1",
            "max_children_per_list":"1"
          }},
          "wire":{{"max_reducer_update_bytes":"{max_update_bytes}"}}
        }}"#
    )
}

#[test]
fn transition_capture_is_a_strict_boolean_and_defaults_to_false() {
    let omitted = bounded_reducer_options(None, 8 * 1024);
    ReducerSession::new(omitted.as_bytes()).unwrap();

    let explicit_false = bounded_reducer_options(Some("false"), 8 * 1024);
    ReducerSession::new(explicit_false.as_bytes()).unwrap();

    for invalid in [r#""true""#, "1", "null"] {
        let options = bounded_reducer_options(Some(invalid), 32 * 1024);
        assert_eq!(
            ReducerSession::new(options.as_bytes())
                .unwrap_err()
                .status(),
            BindingStatus::Options,
            "capture_transitions accepted {invalid}"
        );
    }
}

#[test]
fn capture_enabled_preflight_requires_the_larger_reducer_update_bound() {
    let disabled = bounded_reducer_options(Some("false"), 8 * 1024);
    ReducerSession::new(disabled.as_bytes()).unwrap();

    let undersized = bounded_reducer_options(Some("true"), 8 * 1024);
    let error = ReducerSession::new(undersized.as_bytes()).unwrap_err();
    assert_eq!(error.status(), BindingStatus::Options);
    assert!(error.message().contains("wire.max_reducer_update_bytes"));

    let sufficient = bounded_reducer_options(Some("true"), 32 * 1024);
    ReducerSession::new(sufficient.as_bytes()).unwrap();
}

#[test]
fn reducer_update_budget_rename_and_options_schema_are_strict() {
    let old_name = format!(
        r#"{{
          "schema":"{BINDING_OPTIONS_SCHEMA}",
          "wire":{{"max_impact_bytes":"1048576"}}
        }}"#
    );
    assert_eq!(
        ReducerSession::new(old_name.as_bytes())
            .unwrap_err()
            .status(),
        BindingStatus::Options
    );

    let wrong_schema = bounded_reducer_options(None, 8 * 1024)
        .replace(BINDING_OPTIONS_SCHEMA, "mdstream.bindings-options/999");
    assert_eq!(
        ReducerSession::new(wrong_schema.as_bytes())
            .unwrap_err()
            .status(),
        BindingStatus::UnsupportedSchema
    );
}

#[test]
fn reducer_update_bound_arithmetic_overflow_is_rejected_during_construction() {
    let options = format!(
        r#"{{
          "schema":"{BINDING_OPTIONS_SCHEMA}",
          "capture_transitions":true,
          "protocol":{{"max_nodes":"18446744073709551615"}},
          "wire":{{"max_reducer_update_bytes":"18446744073709551615"}}
        }}"#
    );
    assert_eq!(
        ReducerSession::new(options.as_bytes())
            .unwrap_err()
            .status(),
        BindingStatus::Options
    );
}
