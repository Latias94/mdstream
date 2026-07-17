use super::*;

#[test]
fn envelope_has_explicit_final_schema_and_precise_operation_tags() {
    let change = rooted_change(leaf(0, ContentKind::Paragraph {}));
    let value = serde_json::to_value(&change).unwrap();
    assert_eq!(value["schema"], "mdstream.content/0.4");
    assert_eq!(value["maturity"], "final");
    assert_eq!(value["epoch"], "9");
    assert_eq!(value["sequence"], "0");
    assert_eq!(value["operations"][0]["kind"], "insert_node");
    assert_eq!(value["operations"][1]["kind"], "splice_children");
    assert_eq!(value["operations"][1]["owner"]["kind"], "document");
    assert_eq!(serde_json::from_value::<ChangeSet>(value).unwrap(), change);
}

#[test]
fn final_gate_rejects_draft_and_candidate_envelopes() {
    let change = rooted_change(leaf(0, ContentKind::Paragraph {}));
    for maturity in ["draft", "candidate"] {
        let mut value = serde_json::to_value(&change).unwrap();
        value["maturity"] = serde_json::json!(maturity);
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(matches!(
            mdstream_protocol::decode_change_json(
                &bytes,
                1024 * 1024,
                ProtocolLimits::default()
            ),
            Err(ProtocolError::UnsupportedSchema(message))
                if message.contains("maturity")
        ));
    }
}

#[test]
fn final_gate_rejects_the_superseded_candidate_schema() {
    let change = rooted_change(leaf(0, ContentKind::Paragraph {}));
    let mut value = serde_json::to_value(change).unwrap();
    value["schema"] = serde_json::json!("mdstream.content/0.4-candidate.1");
    let bytes = serde_json::to_vec(&value).unwrap();
    assert!(matches!(
        decode_change_json(&bytes, bytes.len(), ProtocolLimits::default()),
        Err(ProtocolError::UnsupportedSchema(schema))
            if schema == "mdstream.content/0.4-candidate.1"
    ));
}

#[test]
fn final_gate_accepts_snapshot_and_rejects_digest_consistent_draft_and_candidate_snapshots() {
    let mut producer = Reducer::new();
    producer
        .apply(
            ChangeSet::start_epoch(
                Epoch::new(1),
                change_id("epoch:snapshot-maturity"),
                None,
                SourceDelta::append(SourceCursor::new(0), ""),
                vec![],
            )
            .unwrap(),
        )
        .unwrap();
    let snapshot = producer.document().unwrap().snapshot();
    let limits = ProtocolLimits::default();
    let final_bytes = serde_json::to_vec(&snapshot).unwrap();
    assert_eq!(
        decode_snapshot_json(&final_bytes, final_bytes.len(), limits).unwrap(),
        snapshot
    );

    for maturity in ["draft", "candidate"] {
        let mut value = serde_json::to_value(&snapshot).unwrap();
        value["maturity"] = serde_json::json!(maturity);
        refresh_snapshot_digest(&mut value);
        let unchecked: Snapshot = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(unchecked.digest(), &unchecked.derived_digest());

        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(matches!(
            decode_snapshot_json(&bytes, bytes.len(), limits),
            Err(ProtocolError::UnsupportedSchema(message))
                if message.contains("maturity")
        ));
    }
}

#[test]
fn nullable_wire_fields_are_required_and_accept_explicit_null() {
    let content_cases = [
        (
            "link.target",
            "target",
            serde_json::json!({
                "kind": "link",
                "target": null,
                "reference_label": null,
                "style": "inline"
            }),
        ),
        (
            "link.reference_label",
            "reference_label",
            serde_json::json!({
                "kind": "link",
                "target": null,
                "reference_label": null,
                "style": "inline"
            }),
        ),
        (
            "image.target",
            "target",
            serde_json::json!({
                "kind": "image",
                "target": null,
                "reference_label": null,
                "style": "inline",
                "alt": {"kind": "source"}
            }),
        ),
        (
            "image.reference_label",
            "reference_label",
            serde_json::json!({
                "kind": "image",
                "target": null,
                "reference_label": null,
                "style": "inline",
                "alt": {"kind": "source"}
            }),
        ),
        (
            "code_block.info",
            "info",
            serde_json::json!({
                "kind": "code_block",
                "syntax": {"kind": "indented"},
                "info": null,
                "text": {"kind": "source"}
            }),
        ),
        (
            "list.start",
            "start",
            serde_json::json!({
                "kind": "list",
                "ordered": false,
                "start": null,
                "tight": false
            }),
        ),
        (
            "list_item.checked",
            "checked",
            serde_json::json!({"kind": "list_item", "checked": null}),
        ),
        (
            "footnote_reference.target",
            "target",
            serde_json::json!({
                "kind": "footnote_reference",
                "label": "note",
                "target": null
            }),
        ),
        (
            "citation_reference.target",
            "target",
            serde_json::json!({
                "kind": "citation_reference",
                "key": "paper",
                "target": null
            }),
        ),
    ];
    for (label, field, value) in content_cases {
        assert!(
            serde_json::from_value::<ContentKind>(value.clone()).is_ok(),
            "explicit null rejected for {label}"
        );
        let mut missing = value;
        missing.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<ContentKind>(missing).is_err(),
            "missing field accepted for {label}"
        );
    }

    for (label, mut value) in [
        (
            "link.title",
            serde_json::json!({
                "kind": "link",
                "destination": "https://example.test",
                "title": null
            }),
        ),
        (
            "citation.title",
            serde_json::json!({
                "kind": "citation",
                "protocol": "mdstream.citation/1",
                "key": "paper",
                "destination": "https://example.test",
                "title": null
            }),
        ),
    ] {
        assert!(
            serde_json::from_value::<SemanticResourceKind>(value.clone()).is_ok(),
            "explicit null rejected for {label}"
        );
        value.as_object_mut().unwrap().remove("title");
        assert!(
            serde_json::from_value::<SemanticResourceKind>(value).is_err(),
            "missing field accepted for {label}"
        );
    }

    let start = ChangeSet::start_epoch(
        Epoch::new(1),
        change_id("epoch:required-predecessor"),
        None,
        SourceDelta::append(SourceCursor::new(0), ""),
        vec![],
    )
    .unwrap();
    let mut start_value = serde_json::to_value(start).unwrap();
    assert_eq!(
        start_value["epoch_start"]["predecessor"],
        serde_json::Value::Null
    );
    assert!(serde_json::from_value::<ChangeSet>(start_value.clone()).is_ok());
    start_value["epoch_start"]
        .as_object_mut()
        .unwrap()
        .remove("predecessor");
    assert!(serde_json::from_value::<ChangeSet>(start_value).is_err());

    let ordinary = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(1),
        change_id("change:required-epoch-start"),
        SourceDelta::append(SourceCursor::new(0), "x"),
        vec![],
    )
    .unwrap();
    let mut ordinary_value = serde_json::to_value(ordinary).unwrap();
    assert_eq!(ordinary_value["epoch_start"], serde_json::Value::Null);
    assert!(serde_json::from_value::<ChangeSet>(ordinary_value.clone()).is_ok());
    ordinary_value
        .as_object_mut()
        .unwrap()
        .remove("epoch_start");
    assert!(serde_json::from_value::<ChangeSet>(ordinary_value).is_err());
}

#[test]
fn footnote_wire_schema_distinguishes_definitions_and_unresolved_references() {
    let resource = SemanticResource::new(
        ResourceId::new(3),
        SemanticResourceKind::Footnote {
            label: "note".to_string(),
        },
    );
    let definition = leaf(
        0,
        ContentKind::FootnoteDefinition {
            label: "note".to_string(),
            target: resource.reference(),
        },
    );
    let resolved = leaf(
        1,
        ContentKind::FootnoteReference {
            label: "note".to_string(),
            target: Some(resource.reference()),
        },
    );
    let unresolved = leaf(
        2,
        ContentKind::FootnoteReference {
            label: "missing".to_string(),
            target: None,
        },
    );

    let resource_value = serde_json::to_value(&resource).unwrap();
    assert_eq!(
        resource_value["content"],
        serde_json::json!({ "kind": "footnote", "label": "note" })
    );
    assert_eq!(
        serde_json::to_value(&definition).unwrap()["content"],
        serde_json::json!({
            "kind": "footnote_definition",
            "label": "note",
            "target": {
                "id": "3",
                "version": resource.version.as_str(),
            },
        })
    );
    assert_eq!(
        serde_json::to_value(&resolved).unwrap()["content"],
        serde_json::json!({
            "kind": "footnote_reference",
            "label": "note",
            "target": {
                "id": "3",
                "version": resource.version.as_str(),
            },
        })
    );
    assert_eq!(
        serde_json::to_value(&unresolved).unwrap()["content"],
        serde_json::json!({
            "kind": "footnote_reference",
            "label": "missing",
            "target": null,
        })
    );

    let mut missing_target = serde_json::to_value(definition).unwrap();
    missing_target["content"]
        .as_object_mut()
        .unwrap()
        .remove("target");
    assert!(serde_json::from_value::<ContentNode>(missing_target).is_err());
    assert_eq!(
        serde_json::from_value::<ContentNode>(serde_json::to_value(&unresolved).unwrap()).unwrap(),
        unresolved
    );
}
