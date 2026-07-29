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
fn compiler_limits_are_independent_from_the_parser_neutral_protocol_group() {
    let compiler_group = format!(
        r#"{{
          "schema":"{BINDING_OPTIONS_SCHEMA}",
          "compiler":{{
            "max_markdown_events":"8",
            "max_markdown_overlap_work":"16",
            "max_definitions":"32",
            "max_definition_edges":"64",
            "max_definition_metadata_bytes":"128"
          }}
        }}"#
    );
    EngineSession::new(compiler_group.as_bytes()).unwrap();

    for legacy_field in [
        r#""max_markdown_events":"8""#,
        r#""max_definitions":"32""#,
        r#""max_definition_edges":"64""#,
        r#""max_definition_metadata_bytes":"128""#,
    ] {
        let legacy_protocol_field = format!(
            r#"{{
              "schema":"{BINDING_OPTIONS_SCHEMA}",
              "protocol":{{{legacy_field}}}
            }}"#
        );
        assert_eq!(
            EngineSession::new(legacy_protocol_field.as_bytes())
                .unwrap_err()
                .status(),
            BindingStatus::Options,
            "protocol unexpectedly accepted compiler field {legacy_field}"
        );
    }
}

#[test]
fn compiler_definition_limit_is_forwarded_through_engine_session() {
    let options = format!(
        r#"{{
          "schema":"{BINDING_OPTIONS_SCHEMA}",
          "compiler":{{"max_definitions":"1"}}
        }}"#
    );
    let mut engine = EngineSession::new(options.as_bytes()).unwrap();
    engine.append(b"[a]: /a\n\n").unwrap();
    let error = engine.append(b"[b]: /b\n").unwrap_err();

    assert_eq!(error.status(), BindingStatus::ResourceLimit);
    assert_eq!(error.detail_code(), "bindings.resource_limit");
    assert_eq!(error.message(), "definitions uses 2 items, limit is 1");
}

fn unresolved_footnote_workload() -> String {
    let mut source = String::new();
    source.push_str("*open ");
    for index in 0..20 {
        source.push_str(&format!("[^note-{index}] "));
    }
    source.push_str("close*\n\n");
    source
}

#[test]
fn compiler_event_limit_is_forwarded_through_engine_session() {
    let source = unresolved_footnote_workload();

    let mut default_engine = EngineSession::new(b"").unwrap();
    default_engine.append(source.as_bytes()).unwrap();
    default_engine.finish().unwrap();

    let options = format!(
        r#"{{
          "schema":"{BINDING_OPTIONS_SCHEMA}",
          "compiler":{{"max_markdown_events":"32"}}
        }}"#
    );
    let mut bounded_engine = EngineSession::new(options.as_bytes()).unwrap();
    let error = bounded_engine.append(source.as_bytes()).unwrap_err();

    assert_eq!(error.status(), BindingStatus::ResourceLimit);
    assert_eq!(error.detail_code(), "bindings.resource_limit");
    assert!(error.message().contains("markdown.events"));
    assert!(error.message().contains("uses 33 events"));
    assert!(error.message().contains("limit is 32"));
}

#[test]
fn compiler_overlap_limit_reports_bounded_work_units() {
    let options = format!(
        r#"{{
          "schema":"{BINDING_OPTIONS_SCHEMA}",
          "compiler":{{
            "max_markdown_events":"1024",
            "max_markdown_overlap_work":"1"
          }}
        }}"#
    );
    let mut engine = EngineSession::new(options.as_bytes()).unwrap();
    let error = engine
        .append(unresolved_footnote_workload().as_bytes())
        .unwrap_err();

    assert_eq!(error.status(), BindingStatus::ResourceLimit);
    assert!(error.message().contains("markdown.footnote_overlap_work"));
    assert!(error.message().contains("uses 2 work units"));
    assert!(error.message().contains("limit is 1"));
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
