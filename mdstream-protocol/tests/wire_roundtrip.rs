use std::collections::BTreeMap;

use mdstream_protocol::{
    BlockQuoteKind, ChangeId, ChangeSet, ChildList, ChildListOwner, CitationProtocol, ContentKind,
    ContentNode, DocumentLifecycle, Epoch, LinkStyle, NodeId, NodeStability, NodeVersion,
    ProjectionOp, ProtocolError, ProtocolErrorCode, ProtocolLimits, ProtocolMaturity,
    RecoveryReason, Reducer, RequestGeneration, ResourceId, ResourceRef, ResourceVersion,
    SemanticResource, SemanticResourceKind, SemanticText, Sequence, Snapshot, SourceCursor,
    SourceDelta, SourceRange, StructureVersion, TableAlignment, decode_change_json,
    decode_snapshot_json, encode_change_json, encode_snapshot_json,
};

fn change_id(value: &str) -> ChangeId {
    ChangeId::new(value).unwrap()
}

fn empty_range() -> SourceRange {
    SourceRange::new(SourceCursor::new(0), SourceCursor::new(0))
}

fn leaf(id: u64, content: ContentKind) -> ContentNode {
    ContentNode::leaf(
        NodeId::new(id),
        NodeStability::Stable,
        empty_range(),
        content,
    )
}

fn resource_ref(id: u64) -> ResourceRef {
    ResourceRef {
        id: ResourceId::new(id),
        version: ResourceVersion::new("resource:v1").unwrap(),
    }
}

fn splice_roots(current: &ChildList, ids: Vec<NodeId>) -> ProjectionOp {
    let replacement = ChildList::new(ids.clone());
    ProjectionOp::SpliceChildren {
        owner: ChildListOwner::Document,
        expected_version: current.version.clone(),
        start: 0,
        delete_count: u32::try_from(current.children.len()).unwrap(),
        insert: ids,
        new_version: replacement.version,
    }
}

fn rooted_change(node: ContentNode) -> ChangeSet {
    let id = node.id;
    ChangeSet::start_epoch(
        Epoch::new(9),
        change_id("epoch:9"),
        None,
        SourceDelta::append(SourceCursor::new(0), ""),
        vec![
            ProjectionOp::InsertNode { node },
            splice_roots(&ChildList::empty(), vec![id]),
        ],
    )
    .unwrap()
}

fn refresh_snapshot_digest(value: &mut serde_json::Value) {
    let snapshot: Snapshot = serde_json::from_value(value.clone()).unwrap();
    value["digest"] = serde_json::to_value(snapshot.derived_digest()).unwrap();
}

#[test]
fn js_unsafe_identifiers_are_canonical_decimal_strings() {
    let value = serde_json::to_value((
        Epoch::new(u64::MAX),
        NodeId::new(u64::MAX),
        ResourceId::new(u64::MAX),
        Sequence::new(u64::MAX),
        SourceCursor::new(u64::MAX),
        RequestGeneration::new(u64::MAX),
    ))
    .unwrap();

    assert_eq!(
        value,
        serde_json::json!([
            "18446744073709551615",
            "18446744073709551615",
            "18446744073709551615",
            "18446744073709551615",
            "18446744073709551615",
            "18446744073709551615"
        ])
    );
}

#[test]
fn decimal_identifiers_reject_noncanonical_numbers_and_negative_values() {
    for invalid in [
        serde_json::json!(1),
        serde_json::json!(-1),
        serde_json::json!("-1"),
        serde_json::json!(""),
        serde_json::json!("01"),
        serde_json::json!("+1"),
        serde_json::json!(" 1"),
        serde_json::json!("1 "),
        serde_json::json!("18446744073709551616"),
    ] {
        assert!(serde_json::from_value::<Epoch>(invalid).is_err());
    }
    assert_eq!(
        serde_json::from_value::<Epoch>(serde_json::json!("9007199254740993")).unwrap(),
        Epoch::new(9_007_199_254_740_993)
    );
}

#[test]
fn opaque_identifiers_validate_length_character_set_and_string_type() {
    assert!(ChangeId::new("").is_err());
    assert!(ChangeId::new("contains whitespace").is_err());
    assert!(ChangeId::new("x".repeat(129)).is_err());
    assert!(ChangeId::new("sha256:abcdef").is_ok());
    assert!(NodeVersion::new("version:one").is_ok());
    assert!(serde_json::from_value::<ChangeId>(serde_json::json!(42)).is_err());
}

#[test]
fn envelope_has_explicit_draft_schema_and_precise_operation_tags() {
    let change = rooted_change(leaf(0, ContentKind::Paragraph {}));
    let value = serde_json::to_value(&change).unwrap();
    assert_eq!(value["schema"], "mdstream.content/0.4-draft.1");
    assert_eq!(value["maturity"], "draft");
    assert_eq!(value["epoch"], "9");
    assert_eq!(value["sequence"], "0");
    assert_eq!(value["operations"][0]["kind"], "insert_node");
    assert_eq!(value["operations"][1]["kind"], "splice_children");
    assert_eq!(value["operations"][1]["owner"]["kind"], "document");
    assert_eq!(serde_json::from_value::<ChangeSet>(value).unwrap(), change);
}

#[test]
fn all_operation_and_nested_enum_tags_are_stable() {
    let node = leaf(0, ContentKind::Paragraph {});
    let replacement = leaf(0, ContentKind::Heading { level: 1 }).projection();
    let resource = SemanticResource::new(
        ResourceId::new(0),
        SemanticResourceKind::Link {
            destination: "https://example.test".to_string(),
            title: None,
        },
    );
    let operation_tags = vec![
        (
            "insert_node",
            ProjectionOp::InsertNode { node: node.clone() },
        ),
        (
            "replace_node",
            ProjectionOp::ReplaceNode {
                node_id: node.id,
                expected_version: node.version.clone(),
                projection: replacement,
            },
        ),
        (
            "stabilize_node",
            ProjectionOp::StabilizeNode {
                node_id: node.id,
                expected_version: node.version.clone(),
                new_version: NodeVersion::new("stable").unwrap(),
            },
        ),
        (
            "remove_node",
            ProjectionOp::RemoveNode {
                node_id: node.id,
                expected_version: node.version,
            },
        ),
        (
            "splice_children",
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Document,
                expected_version: StructureVersion::new("old").unwrap(),
                start: 0,
                delete_count: 0,
                insert: vec![],
                new_version: StructureVersion::new("new").unwrap(),
            },
        ),
        (
            "insert_resource",
            ProjectionOp::InsertResource {
                resource: resource.clone(),
            },
        ),
        (
            "replace_resource",
            ProjectionOp::ReplaceResource {
                resource_id: resource.id,
                expected_version: resource.version.clone(),
                resource: resource.clone(),
            },
        ),
        (
            "remove_resource",
            ProjectionOp::RemoveResource {
                resource_id: resource.id,
                expected_version: resource.version,
            },
        ),
        ("finish_document", ProjectionOp::FinishDocument),
    ];
    for (tag, operation) in operation_tags {
        assert_eq!(serde_json::to_value(operation).unwrap()["kind"], tag);
    }

    let owners = [
        ("document", ChildListOwner::Document),
        (
            "node",
            ChildListOwner::Node {
                node_id: NodeId::new(1),
            },
        ),
    ];
    for (tag, owner) in owners {
        assert_eq!(serde_json::to_value(owner).unwrap()["kind"], tag);
    }

    let link_styles = [
        ("inline", LinkStyle::Inline),
        ("reference", LinkStyle::Reference),
        ("reference_unknown", LinkStyle::ReferenceUnknown),
        ("collapsed", LinkStyle::Collapsed),
        ("collapsed_unknown", LinkStyle::CollapsedUnknown),
        ("shortcut", LinkStyle::Shortcut),
        ("shortcut_unknown", LinkStyle::ShortcutUnknown),
        ("autolink", LinkStyle::Autolink),
        ("email", LinkStyle::Email),
    ];
    for (tag, style) in link_styles {
        assert_eq!(serde_json::to_value(style).unwrap(), tag);
    }

    let alignments = [
        ("none", TableAlignment::None),
        ("left", TableAlignment::Left),
        ("center", TableAlignment::Center),
        ("right", TableAlignment::Right),
    ];
    for (tag, alignment) in alignments {
        assert_eq!(serde_json::to_value(alignment).unwrap(), tag);
    }

    let quote_kinds = [
        ("plain", BlockQuoteKind::Plain),
        ("note", BlockQuoteKind::Note),
        ("tip", BlockQuoteKind::Tip),
        ("important", BlockQuoteKind::Important),
        ("warning", BlockQuoteKind::Warning),
        ("caution", BlockQuoteKind::Caution),
    ];
    for (tag, kind) in quote_kinds {
        assert_eq!(serde_json::to_value(kind).unwrap(), tag);
    }

    for (tag, text) in [
        ("source", SemanticText::Source {}),
        (
            "normalized",
            SemanticText::Normalized {
                value: "value".to_string(),
            },
        ),
    ] {
        assert_eq!(serde_json::to_value(text).unwrap()["kind"], tag);
    }

    for (tag, resource) in [
        (
            "link",
            SemanticResourceKind::Link {
                destination: "https://example.test".to_string(),
                title: None,
            },
        ),
        (
            "citation",
            SemanticResourceKind::Citation {
                protocol: CitationProtocol::V1,
                key: "paper".to_string(),
                destination: "https://example.test/paper".to_string(),
                title: None,
            },
        ),
    ] {
        assert_eq!(serde_json::to_value(resource).unwrap()["kind"], tag);
    }

    let recovery_reasons = vec![
        (
            "sequence_gap",
            RecoveryReason::SequenceGap {
                expected: Sequence::new(1),
                received: Sequence::new(2),
            },
        ),
        (
            "sequence_fork",
            RecoveryReason::SequenceFork {
                sequence: Sequence::new(1),
            },
        ),
        (
            "unannounced_epoch",
            RecoveryReason::UnannouncedEpoch {
                current: Epoch::new(1),
                received: Epoch::new(2),
            },
        ),
        ("source_divergence", RecoveryReason::SourceDivergence),
        ("version_divergence", RecoveryReason::VersionDivergence),
        ("structure_divergence", RecoveryReason::StructureDivergence),
        ("resource_divergence", RecoveryReason::ResourceDivergence),
    ];
    for (tag, reason) in recovery_reasons {
        assert_eq!(serde_json::to_value(reason).unwrap()["kind"], tag);
    }

    for (expected, value) in [
        (
            "open",
            serde_json::to_value(DocumentLifecycle::Open).unwrap(),
        ),
        (
            "finalized",
            serde_json::to_value(DocumentLifecycle::Finalized).unwrap(),
        ),
        (
            "provisional",
            serde_json::to_value(NodeStability::Provisional).unwrap(),
        ),
        (
            "stable",
            serde_json::to_value(NodeStability::Stable).unwrap(),
        ),
        (
            "draft",
            serde_json::to_value(ProtocolMaturity::Draft).unwrap(),
        ),
        (
            "candidate",
            serde_json::to_value(ProtocolMaturity::Candidate).unwrap(),
        ),
        (
            "final",
            serde_json::to_value(ProtocolMaturity::Final).unwrap(),
        ),
    ] {
        assert_eq!(value, expected);
    }

    let error_codes = [
        ("unsupported_schema", ProtocolErrorCode::UnsupportedSchema),
        ("invalid_change", ProtocolErrorCode::InvalidChange),
        ("invalid_snapshot", ProtocolErrorCode::InvalidSnapshot),
        ("invalid_range", ProtocolErrorCode::InvalidRange),
        ("cursor_overflow", ProtocolErrorCode::CursorOverflow),
        ("metadata_overflow", ProtocolErrorCode::MetadataOverflow),
        ("sequence_overflow", ProtocolErrorCode::SequenceOverflow),
        ("source_too_large", ProtocolErrorCode::SourceTooLarge),
        ("too_many_nodes", ProtocolErrorCode::TooManyNodes),
        ("too_many_operations", ProtocolErrorCode::TooManyOperations),
        ("value_too_large", ProtocolErrorCode::ValueTooLarge),
        ("missing_node", ProtocolErrorCode::MissingNode),
        ("missing_resource", ProtocolErrorCode::MissingResource),
        ("duplicate_node", ProtocolErrorCode::DuplicateNode),
        ("duplicate_resource", ProtocolErrorCode::DuplicateResource),
        ("reused_node_id", ProtocolErrorCode::ReusedNodeId),
        ("reused_resource_id", ProtocolErrorCode::ReusedResourceId),
        ("version_mismatch", ProtocolErrorCode::VersionMismatch),
        (
            "resource_version_mismatch",
            ProtocolErrorCode::ResourceVersionMismatch,
        ),
        ("illegal_lifecycle", ProtocolErrorCode::IllegalLifecycle),
        ("needs_snapshot", ProtocolErrorCode::NeedsSnapshot),
        (
            "snapshot_not_allowed",
            ProtocolErrorCode::SnapshotNotAllowed,
        ),
        ("invalid_epoch_start", ProtocolErrorCode::InvalidEpochStart),
        ("stale_snapshot", ProtocolErrorCode::StaleSnapshot),
    ];
    for (tag, code) in error_codes {
        assert_eq!(serde_json::to_value(code).unwrap(), tag);
    }
}

#[test]
fn every_content_ir_variant_has_an_exact_stable_tag() {
    let mut attributes = BTreeMap::new();
    attributes.insert("role".to_string(), "note".to_string());
    let variants = vec![
        ("paragraph", ContentKind::Paragraph {}),
        ("heading", ContentKind::Heading { level: 2 }),
        (
            "text",
            ContentKind::Text {
                text: SemanticText::Source {},
            },
        ),
        ("emphasis", ContentKind::Emphasis {}),
        ("strong", ContentKind::Strong {}),
        ("strikethrough", ContentKind::Strikethrough {}),
        (
            "link",
            ContentKind::Link {
                target: Some(resource_ref(1)),
                reference_label: Some("ref".to_string()),
                style: LinkStyle::Reference,
            },
        ),
        (
            "image",
            ContentKind::Image {
                target: Some(resource_ref(1)),
                reference_label: None,
                style: LinkStyle::Inline,
                alt: SemanticText::Normalized {
                    value: "diagram & details".to_string(),
                },
            },
        ),
        (
            "inline_code",
            ContentKind::InlineCode {
                text: SemanticText::Normalized {
                    value: "code".to_string(),
                },
            },
        ),
        (
            "code_block",
            ContentKind::CodeBlock {
                fenced: true,
                language: Some("rust".to_string()),
                meta: Some("linenos".to_string()),
                mermaid: false,
                text: SemanticText::Source {},
            },
        ),
        (
            "list",
            ContentKind::List {
                ordered: true,
                start: Some(3),
                tight: false,
            },
        ),
        (
            "list_item",
            ContentKind::ListItem {
                checked: Some(true),
            },
        ),
        (
            "block_quote",
            ContentKind::BlockQuote {
                style: BlockQuoteKind::Warning,
            },
        ),
        ("thematic_break", ContentKind::ThematicBreak {}),
        (
            "table",
            ContentKind::Table {
                alignments: vec![TableAlignment::Left, TableAlignment::Right],
            },
        ),
        ("table_head", ContentKind::TableHead {}),
        ("table_body", ContentKind::TableBody {}),
        ("table_row", ContentKind::TableRow {}),
        ("table_cell", ContentKind::TableCell { column: 1 }),
        (
            "html",
            ContentKind::Html {
                block: true,
                opaque: true,
            },
        ),
        (
            "math",
            ContentKind::Math {
                display: true,
                text: SemanticText::Source {},
            },
        ),
        (
            "footnote_definition",
            ContentKind::FootnoteDefinition {
                label: "note".to_string(),
            },
        ),
        (
            "footnote_reference",
            ContentKind::FootnoteReference {
                label: "note".to_string(),
            },
        ),
        (
            "citation_definition",
            ContentKind::CitationDefinition {
                key: "paper".to_string(),
                target: resource_ref(2),
            },
        ),
        (
            "citation_reference",
            ContentKind::CitationReference {
                key: "paper".to_string(),
                target: Some(resource_ref(2)),
            },
        ),
        ("soft_break", ContentKind::SoftBreak {}),
        ("hard_break", ContentKind::HardBreak {}),
        (
            "custom",
            ContentKind::Custom {
                namespace: "example.rich/1".to_string(),
                name: "aside".to_string(),
                opaque: true,
                attributes,
            },
        ),
    ];

    for (index, (tag, content)) in variants.into_iter().enumerate() {
        let node = leaf(index as u64, content);
        let value = serde_json::to_value(&node).unwrap();
        assert_eq!(value["content"]["kind"], tag, "variant {tag}");
        assert_eq!(serde_json::from_value::<ContentNode>(value).unwrap(), node);
    }
}

#[test]
fn semantic_text_roundtrips_entity_escape_and_code_whitespace_normalization() {
    let fixtures = [
        ContentKind::Text {
            text: SemanticText::Normalized {
                value: "&".to_string(),
            },
        },
        ContentKind::Text {
            text: SemanticText::Normalized {
                value: "*".to_string(),
            },
        },
        ContentKind::InlineCode {
            text: SemanticText::Normalized {
                value: "alpha beta".to_string(),
            },
        },
        ContentKind::CodeBlock {
            fenced: false,
            language: None,
            meta: None,
            mermaid: false,
            text: SemanticText::Normalized {
                value: "line one\nline two\n".to_string(),
            },
        },
    ];

    for (id, content) in fixtures.into_iter().enumerate() {
        let node = leaf(u64::try_from(id).unwrap(), content);
        let bytes = serde_json::to_vec(&node).unwrap();
        assert_eq!(serde_json::from_slice::<ContentNode>(&bytes).unwrap(), node);
    }
}

#[test]
fn citation_resources_carry_the_mdstream_citation_protocol_marker() {
    let resource = SemanticResource::new(
        ResourceId::new(3),
        SemanticResourceKind::Citation {
            protocol: CitationProtocol::V1,
            key: "paper".to_string(),
            destination: "https://example.test".to_string(),
            title: None,
        },
    );
    let value = serde_json::to_value(&resource).unwrap();
    assert_eq!(value["content"]["kind"], "citation");
    assert_eq!(value["content"]["protocol"], "mdstream.citation/1");
    assert_eq!(mdstream_protocol::CITATION_PROTOCOL, "mdstream.citation/1");
}

#[test]
fn list_start_is_js_safe_task_state_is_explicit_and_body_is_required() {
    let list = leaf(
        0,
        ContentKind::List {
            ordered: true,
            start: Some(999_999_999),
            tight: true,
        },
    );
    let list_value = serde_json::to_value(&list).unwrap();
    assert_eq!(list_value["content"]["start"], 999_999_999u64);
    let mut oversized = list_value.clone();
    oversized["content"]["start"] = serde_json::json!(u64::MAX);
    assert!(serde_json::from_value::<ContentNode>(oversized).is_err());

    let item = leaf(1, ContentKind::ListItem { checked: None });
    let item_value = serde_json::to_value(&item).unwrap();
    assert!(
        item_value["content"]
            .as_object()
            .unwrap()
            .contains_key("checked")
    );

    let mut missing_body = serde_json::to_value(&list).unwrap();
    missing_body.as_object_mut().unwrap().remove("body");
    assert!(serde_json::from_value::<ContentNode>(missing_body).is_err());
}

#[test]
fn decoders_fail_closed_on_unknown_or_missing_nested_fields_and_size() {
    let change = rooted_change(leaf(0, ContentKind::Paragraph {}));
    let limits = ProtocolLimits::default();
    let bytes = encode_change_json(&change, usize::MAX, limits).unwrap();
    assert_eq!(
        decode_change_json(&bytes, bytes.len(), limits).unwrap(),
        change
    );
    assert!(matches!(
        decode_change_json(&bytes, bytes.len() - 1, limits),
        Err(ProtocolError::ValueTooLarge { .. })
    ));

    let mut unknown_envelope = serde_json::to_value(&change).unwrap();
    unknown_envelope["future"] = serde_json::json!(true);
    let bytes = serde_json::to_vec(&unknown_envelope).unwrap();
    assert!(matches!(
        decode_change_json(&bytes, bytes.len(), limits),
        Err(ProtocolError::InvalidChange(_))
    ));

    let mut unknown_source = serde_json::to_value(&change).unwrap();
    unknown_source["source"]["future"] = serde_json::json!(true);
    let bytes = serde_json::to_vec(&unknown_source).unwrap();
    assert!(decode_change_json(&bytes, bytes.len(), limits).is_err());

    let mut unknown_content = serde_json::to_value(&change).unwrap();
    unknown_content["operations"][0]["node"]["content"]["future"] = serde_json::json!(true);
    let bytes = serde_json::to_vec(&unknown_content).unwrap();
    assert!(decode_change_json(&bytes, bytes.len(), limits).is_err());

    let mut missing_body = serde_json::to_value(&change).unwrap();
    missing_body["operations"][0]["node"]
        .as_object_mut()
        .unwrap()
        .remove("body");
    let bytes = serde_json::to_vec(&missing_body).unwrap();
    assert!(decode_change_json(&bytes, bytes.len(), limits).is_err());
}

#[test]
fn encoded_output_limit_stops_at_the_first_byte_over_budget() {
    let source = "x".repeat(4_096);
    let change = ChangeSet::start_epoch(
        Epoch::new(1),
        change_id("bounded"),
        None,
        SourceDelta::append(SourceCursor::new(0), source),
        vec![],
    )
    .unwrap();
    let limit = 128;

    let complete = encode_change_json(&change, usize::MAX, ProtocolLimits::default()).unwrap();
    assert_eq!(
        encode_change_json(&change, complete.len(), ProtocolLimits::default()).unwrap(),
        complete
    );
    assert!(matches!(
        encode_change_json(&change, complete.len() - 1, ProtocolLimits::default()),
        Err(ProtocolError::ValueTooLarge {
            field: "encoded_change",
            limit: actual_limit,
            actual,
        }) if actual_limit + 1 == actual && actual == complete.len()
    ));

    assert!(matches!(
        encode_change_json(&change, limit, ProtocolLimits::default()),
        Err(ProtocolError::ValueTooLarge {
            field: "encoded_change",
            limit: 128,
            actual: 129,
        })
    ));
}

#[test]
fn local_protocol_limits_accept_the_boundary_and_reject_first_excess() {
    fn custom_node(
        id: u64,
        namespace: &str,
        name: &str,
        attributes: BTreeMap<String, String>,
    ) -> ContentNode {
        leaf(
            id,
            ContentKind::Custom {
                namespace: namespace.to_string(),
                name: name.to_string(),
                opaque: true,
                attributes,
            },
        )
    }

    let source_limits = ProtocolLimits {
        max_source_bytes: 1,
        ..ProtocolLimits::default()
    };
    let source_at_limit = ChangeSet::start_epoch(
        Epoch::new(1),
        change_id("limit:source:at"),
        None,
        SourceDelta::append(SourceCursor::new(0), "x"),
        vec![],
    )
    .unwrap();
    encode_change_json(&source_at_limit, usize::MAX, source_limits).unwrap();
    let source_over_limit = ChangeSet::start_epoch(
        Epoch::new(1),
        change_id("limit:source:over"),
        None,
        SourceDelta::append(SourceCursor::new(0), "xx"),
        vec![],
    )
    .unwrap();
    assert!(matches!(
        encode_change_json(&source_over_limit, usize::MAX, source_limits),
        Err(ProtocolError::SourceTooLarge {
            limit: 1,
            actual: 2,
        })
    ));

    let operation_limits = ProtocolLimits {
        max_operations: 1,
        ..ProtocolLimits::default()
    };
    let one_operation = ChangeSet::start_epoch(
        Epoch::new(1),
        change_id("limit:operations:at"),
        None,
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![ProjectionOp::FinishDocument],
    )
    .unwrap();
    encode_change_json(&one_operation, usize::MAX, operation_limits).unwrap();
    let two_operations = ChangeSet::start_epoch(
        Epoch::new(1),
        change_id("limit:operations:over"),
        None,
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![ProjectionOp::FinishDocument, ProjectionOp::FinishDocument],
    )
    .unwrap();
    assert!(matches!(
        encode_change_json(&two_operations, usize::MAX, operation_limits),
        Err(ProtocolError::TooManyOperations {
            limit: 1,
            actual: 2,
        })
    ));

    let one_child = ContentNode::new(
        NodeId::new(0),
        NodeStability::Stable,
        empty_range(),
        empty_range(),
        vec![NodeId::new(1)],
        ContentKind::BlockQuote {
            style: BlockQuoteKind::Plain,
        },
    );
    let child_limits = ProtocolLimits {
        max_children_per_list: 1,
        ..ProtocolLimits::default()
    };
    let one_child_change = ChangeSet::start_epoch(
        Epoch::new(1),
        change_id("limit:children:at"),
        None,
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![ProjectionOp::InsertNode { node: one_child }],
    )
    .unwrap();
    encode_change_json(&one_child_change, usize::MAX, child_limits).unwrap();
    let two_children = ContentNode::new(
        NodeId::new(0),
        NodeStability::Stable,
        empty_range(),
        empty_range(),
        vec![NodeId::new(1), NodeId::new(2)],
        ContentKind::BlockQuote {
            style: BlockQuoteKind::Plain,
        },
    );
    let two_child_change = ChangeSet::start_epoch(
        Epoch::new(1),
        change_id("limit:children:over"),
        None,
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![ProjectionOp::InsertNode { node: two_children }],
    )
    .unwrap();
    assert!(matches!(
        encode_change_json(&two_child_change, usize::MAX, child_limits),
        Err(ProtocolError::ValueTooLarge {
            field: "child_list.children",
            limit: 1,
            actual: 2,
        })
    ));

    let mut one_attribute = BTreeMap::new();
    one_attribute.insert("k".to_string(), "v".to_string());
    let attribute_limits = ProtocolLimits {
        max_attributes_per_node: 1,
        ..ProtocolLimits::default()
    };
    encode_change_json(
        &rooted_change(custom_node(0, "n", "x", one_attribute.clone())),
        usize::MAX,
        attribute_limits,
    )
    .unwrap();
    let mut two_attributes = one_attribute;
    two_attributes.insert("q".to_string(), "z".to_string());
    assert!(matches!(
        encode_change_json(
            &rooted_change(custom_node(0, "n", "x", two_attributes)),
            usize::MAX,
            attribute_limits,
        ),
        Err(ProtocolError::ValueTooLarge {
            field: "custom.attributes",
            limit: 1,
            actual: 2,
        })
    ));

    let value_limits = ProtocolLimits {
        max_metadata_value_bytes: 1,
        ..ProtocolLimits::default()
    };
    encode_change_json(
        &rooted_change(custom_node(0, "n", "x", BTreeMap::new())),
        usize::MAX,
        value_limits,
    )
    .unwrap();
    assert!(matches!(
        encode_change_json(
            &rooted_change(custom_node(0, "nn", "x", BTreeMap::new())),
            usize::MAX,
            value_limits,
        ),
        Err(ProtocolError::ValueTooLarge {
            field: "custom.namespace",
            limit: 1,
            actual: 2,
        })
    ));

    let node_limits = ProtocolLimits {
        max_metadata_value_bytes: 1,
        max_node_metadata_bytes: 2,
        ..ProtocolLimits::default()
    };
    let metadata_node = custom_node(0, "n", "x", BTreeMap::new());
    encode_change_json(
        &rooted_change(metadata_node.clone()),
        usize::MAX,
        node_limits,
    )
    .unwrap();
    let node_over_limit = ProtocolLimits {
        max_node_metadata_bytes: 1,
        ..node_limits
    };
    assert!(matches!(
        encode_change_json(&rooted_change(metadata_node), usize::MAX, node_over_limit,),
        Err(ProtocolError::ValueTooLarge {
            field: "node.metadata",
            limit: 1,
            actual: 2,
        })
    ));

    let metadata_nodes = [
        custom_node(0, "n", "x", BTreeMap::new()),
        custom_node(1, "a", "b", BTreeMap::new()),
    ];
    let metadata_change = ChangeSet::start_epoch(
        Epoch::new(1),
        change_id("limit:change-metadata"),
        None,
        SourceDelta::unchanged(SourceCursor::new(0)),
        metadata_nodes
            .into_iter()
            .map(|node| ProjectionOp::InsertNode { node })
            .collect(),
    )
    .unwrap();
    let change_at_limit = ProtocolLimits {
        max_metadata_value_bytes: 1,
        max_node_metadata_bytes: 2,
        max_change_metadata_bytes: 4,
        ..ProtocolLimits::default()
    };
    encode_change_json(&metadata_change, usize::MAX, change_at_limit).unwrap();
    let change_over_limit = ProtocolLimits {
        max_change_metadata_bytes: 3,
        ..change_at_limit
    };
    assert!(matches!(
        encode_change_json(&metadata_change, usize::MAX, change_over_limit),
        Err(ProtocolError::ValueTooLarge {
            field: "change.metadata",
            limit: 3,
            actual: 4,
        })
    ));
}

#[test]
fn duplicate_custom_attribute_keys_are_rejected() {
    let mut attributes = BTreeMap::new();
    attributes.insert("role".to_string(), "second".to_string());
    let change = rooted_change(leaf(
        0,
        ContentKind::Custom {
            namespace: "example.rich/1".to_string(),
            name: "aside".to_string(),
            opaque: false,
            attributes,
        },
    ));
    let canonical = String::from_utf8(
        encode_change_json(&change, usize::MAX, ProtocolLimits::default()).unwrap(),
    )
    .unwrap();
    let duplicate = canonical.replace(
        "\"attributes\":{\"role\":\"second\"}",
        "\"attributes\":{\"role\":\"first\",\"role\":\"second\"}",
    );
    assert_ne!(duplicate, canonical);

    assert!(matches!(
        decode_change_json(
            duplicate.as_bytes(),
            duplicate.len(),
            ProtocolLimits::default(),
        ),
        Err(ProtocolError::InvalidChange(_))
    ));
}

#[test]
fn protocol_errors_expose_stable_machine_codes() {
    assert_eq!(
        ProtocolError::InvalidChange("first message".to_string()).code(),
        ProtocolErrorCode::InvalidChange
    );
    assert_eq!(
        ProtocolError::InvalidChange("different message".to_string()).code(),
        ProtocolErrorCode::InvalidChange
    );
    assert_eq!(
        serde_json::to_value(ProtocolErrorCode::InvalidChange).unwrap(),
        serde_json::json!("invalid_change")
    );
}

#[test]
fn direct_and_json_decoded_replay_produce_identical_snapshots() {
    let first = rooted_change(leaf(0, ContentKind::Paragraph {}));
    let second = ChangeSet::new(
        Epoch::new(9),
        Sequence::new(1),
        change_id("finish"),
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![ProjectionOp::FinishDocument],
    )
    .unwrap();
    let limits = ProtocolLimits::default();
    let mut direct = Reducer::new();
    let mut decoded = Reducer::new();
    for change in [first, second] {
        direct.apply(change.clone()).unwrap();
        let bytes = encode_change_json(&change, usize::MAX, limits).unwrap();
        decoded
            .apply(decode_change_json(&bytes, bytes.len(), limits).unwrap())
            .unwrap();
    }
    assert_eq!(
        direct.document().unwrap().snapshot(),
        decoded.document().unwrap().snapshot()
    );
}

#[test]
fn snapshot_wire_has_one_source_body_roundtrips_and_resumes() {
    let source = "ONLY_ONE_CANONICAL_SOURCE_BODY";
    let span = SourceRange::new(SourceCursor::new(0), SourceCursor::new(source.len() as u64));
    let node = ContentNode::leaf(
        NodeId::new(0),
        NodeStability::Stable,
        span,
        ContentKind::Paragraph {},
    );
    let mut producer = Reducer::new();
    producer
        .apply(
            ChangeSet::start_epoch(
                Epoch::new(1),
                change_id("epoch:source"),
                None,
                SourceDelta::append(SourceCursor::new(0), source),
                vec![
                    ProjectionOp::InsertNode { node },
                    splice_roots(&ChildList::empty(), vec![NodeId::new(0)]),
                ],
            )
            .unwrap(),
        )
        .unwrap();
    let snapshot = producer.document().unwrap().snapshot();
    let limits = ProtocolLimits::default();
    let bytes = encode_snapshot_json(&snapshot, usize::MAX, limits).unwrap();
    assert_eq!(String::from_utf8_lossy(&bytes).matches(source).count(), 1);
    let decoded = decode_snapshot_json(&bytes, bytes.len(), limits).unwrap();
    assert_eq!(decoded, snapshot);

    let mut consumer = Reducer::new();
    consumer.recover_snapshot(decoded).unwrap();
    let next = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(1),
        change_id("append"),
        SourceDelta::append(SourceCursor::new(source.len() as u64), "!"),
        vec![],
    )
    .unwrap();
    consumer.apply(next).unwrap();
    assert_eq!(consumer.document().unwrap().source(), format!("{source}!"));
}

#[test]
fn snapshot_decoder_rejects_structural_and_version_matrix() {
    let mut reducer = Reducer::new();
    reducer
        .apply(rooted_change(leaf(0, ContentKind::Paragraph {})))
        .unwrap();
    let base = serde_json::to_value(reducer.document().unwrap().snapshot()).unwrap();
    let limits = ProtocolLimits::default();

    let mut cases = Vec::new();
    let mut cursor = base.clone();
    cursor["coordinate"]["source_cursor"] = serde_json::json!("1");
    refresh_snapshot_digest(&mut cursor);
    cases.push(cursor);

    let mut epoch = base.clone();
    epoch["coordinate"]["epoch"] = serde_json::json!("10");
    cases.push(epoch);

    let mut sequence = base.clone();
    sequence["coordinate"]["sequence"] = serde_json::json!("1");
    cases.push(sequence);

    let mut change_id = base.clone();
    change_id["coordinate"]["change_id"] = serde_json::json!("forged:change");
    cases.push(change_id);

    let mut payload_digest = base.clone();
    payload_digest["last_payload_digest"] = serde_json::json!("forged:digest");
    cases.push(payload_digest);

    let mut orphan = base.clone();
    orphan["roots"] = serde_json::to_value(ChildList::empty()).unwrap();
    refresh_snapshot_digest(&mut orphan);
    cases.push(orphan);

    let mut duplicate = base.clone();
    duplicate["nodes"]
        .as_array_mut()
        .unwrap()
        .push(base["nodes"][0].clone());
    refresh_snapshot_digest(&mut duplicate);
    cases.push(duplicate);

    let mut high_water = base.clone();
    high_water["next_node_id"] = serde_json::json!("0");
    refresh_snapshot_digest(&mut high_water);
    cases.push(high_water);

    let mut provisional_final = base.clone();
    provisional_final["lifecycle"] = serde_json::json!("finalized");
    let provisional = leaf(0, ContentKind::Paragraph {});
    let mut provisional = serde_json::to_value(provisional).unwrap();
    provisional["stability"] = serde_json::json!("provisional");
    let parsed: ContentNode = serde_json::from_value(provisional).unwrap();
    let mut parsed = parsed;
    parsed.version = parsed.derived_version();
    provisional_final["nodes"][0] = serde_json::to_value(parsed).unwrap();
    refresh_snapshot_digest(&mut provisional_final);
    cases.push(provisional_final);

    let mut forged_version = base.clone();
    forged_version["nodes"][0]["version"] = serde_json::json!("forged");
    refresh_snapshot_digest(&mut forged_version);
    cases.push(forged_version);

    let mut forged_structure = base.clone();
    forged_structure["roots"]["version"] = serde_json::json!("forged");
    refresh_snapshot_digest(&mut forged_structure);
    cases.push(forged_structure);

    let mut unknown = base;
    unknown["roots"]["future"] = serde_json::json!(true);
    cases.push(unknown);

    for value in cases {
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(
            decode_snapshot_json(&bytes, bytes.len(), limits).is_err(),
            "accepted invalid snapshot: {}",
            String::from_utf8_lossy(&bytes)
        );
    }
}

#[test]
fn snapshot_decoder_rejects_tampered_last_change_digest() {
    let mut reducer = Reducer::new();
    reducer
        .apply(rooted_change(leaf(0, ContentKind::Paragraph {})))
        .unwrap();
    let mut value = serde_json::to_value(reducer.document().unwrap().snapshot()).unwrap();
    value["last_payload_digest"] = serde_json::json!("sha256:tampered");
    let bytes = serde_json::to_vec(&value).unwrap();

    assert!(decode_snapshot_json(&bytes, bytes.len(), ProtocolLimits::default()).is_err());
}

#[test]
fn json_object_key_order_does_not_change_decoded_payload_or_digest() {
    let canonical = rooted_change(leaf(0, ContentKind::Paragraph {}));
    let value = serde_json::to_value(&canonical).unwrap();
    let reordered = format!(
        concat!(
            "{{\"operations\":{},",
            "\"source\":{{\"suffix\":{},\"expected_cursor\":{}}},",
            "\"epoch_start\":{},\"change_id\":{},\"sequence\":{},",
            "\"epoch\":{},\"maturity\":{},\"schema\":{}}}"
        ),
        serde_json::to_string(&value["operations"]).unwrap(),
        serde_json::to_string(&value["source"]["suffix"]).unwrap(),
        serde_json::to_string(&value["source"]["expected_cursor"]).unwrap(),
        serde_json::to_string(&value["epoch_start"]).unwrap(),
        serde_json::to_string(&value["change_id"]).unwrap(),
        serde_json::to_string(&value["sequence"]).unwrap(),
        serde_json::to_string(&value["epoch"]).unwrap(),
        serde_json::to_string(&value["maturity"]).unwrap(),
        serde_json::to_string(&value["schema"]).unwrap(),
    );
    let decoded: ChangeSet = serde_json::from_str(&reordered).unwrap();
    assert_eq!(decoded, canonical);
    assert_eq!(decoded.payload_digest(), canonical.payload_digest());
}
