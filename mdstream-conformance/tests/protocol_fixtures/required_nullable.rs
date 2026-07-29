use super::*;

#[test]
fn required_nullable_protocol_fields_match_schema_and_serde() {
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

    let content_validator = validator_for("contentKind");
    let content_cases = [
        (
            "link.target",
            "target",
            ContentKind::Link {
                target: None,
                reference_label: None,
                style: LinkStyle::Inline,
            },
        ),
        (
            "link.reference_label",
            "reference_label",
            ContentKind::Link {
                target: None,
                reference_label: None,
                style: LinkStyle::Inline,
            },
        ),
        (
            "image.target",
            "target",
            ContentKind::Image {
                target: None,
                reference_label: None,
                style: LinkStyle::Inline,
                alt: SemanticText::Source {},
            },
        ),
        (
            "image.reference_label",
            "reference_label",
            ContentKind::Image {
                target: None,
                reference_label: None,
                style: LinkStyle::Inline,
                alt: SemanticText::Source {},
            },
        ),
        (
            "code_block.info",
            "info",
            ContentKind::CodeBlock {
                syntax: CodeBlockSyntax::Indented,
                info: None,
                text: SemanticText::Source {},
            },
        ),
        (
            "list.start",
            "start",
            ContentKind::List {
                ordered: false,
                start: None,
                tight: false,
            },
        ),
        (
            "list_item.checked",
            "checked",
            ContentKind::ListItem { checked: None },
        ),
        (
            "footnote_reference.target",
            "target",
            ContentKind::FootnoteReference {
                label: "note".to_string(),
                target: None,
            },
        ),
        (
            "citation_reference.target",
            "target",
            ContentKind::CitationReference {
                key: "paper".to_string(),
                target: None,
            },
        ),
    ];
    for (label, field, content) in content_cases {
        let mut value = serde_json::to_value(content).unwrap();
        assert_eq!(value[field], serde_json::Value::Null, "{label}");
        assert!(
            content_validator.is_valid(&value),
            "schema rejected {label}"
        );
        assert!(serde_json::from_value::<ContentKind>(value.clone()).is_ok());

        value.as_object_mut().unwrap().remove(field);
        assert!(
            !content_validator.is_valid(&value),
            "schema accepted missing {label}"
        );
        assert!(
            serde_json::from_value::<ContentKind>(value).is_err(),
            "Serde accepted missing {label}"
        );
    }

    let resource_validator = validator_for("semanticResourceKind");
    for (label, resource) in [
        (
            "link.title",
            SemanticResourceKind::Link {
                destination: "https://example.test".to_string(),
                title: None,
            },
        ),
        (
            "citation.title",
            SemanticResourceKind::Citation {
                protocol: CitationProtocol::V1,
                key: "paper".to_string(),
                destination: "https://example.test".to_string(),
                title: None,
            },
        ),
    ] {
        let mut value = serde_json::to_value(resource).unwrap();
        assert_eq!(value["title"], serde_json::Value::Null, "{label}");
        assert!(
            resource_validator.is_valid(&value),
            "schema rejected {label}"
        );
        assert!(serde_json::from_value::<SemanticResourceKind>(value.clone()).is_ok());

        value.as_object_mut().unwrap().remove("title");
        assert!(
            !resource_validator.is_valid(&value),
            "schema accepted missing {label}"
        );
        assert!(
            serde_json::from_value::<SemanticResourceKind>(value).is_err(),
            "Serde accepted missing {label}"
        );
    }

    let change_validator = validator_for("changeSet");
    let mut epoch_start = serde_json::to_value(start(1, "epoch:nullable", "")).unwrap();
    assert_eq!(
        epoch_start["epoch_start"]["predecessor"],
        serde_json::Value::Null
    );
    assert!(change_validator.is_valid(&epoch_start));
    assert!(serde_json::from_value::<ChangeSet>(epoch_start.clone()).is_ok());
    epoch_start["epoch_start"]
        .as_object_mut()
        .unwrap()
        .remove("predecessor");
    assert!(!change_validator.is_valid(&epoch_start));
    assert!(serde_json::from_value::<ChangeSet>(epoch_start).is_err());

    let mut ordinary = serde_json::to_value(next(1, 1, 0, "change:nullable", "x")).unwrap();
    assert_eq!(ordinary["epoch_start"], serde_json::Value::Null);
    assert!(change_validator.is_valid(&ordinary));
    assert!(serde_json::from_value::<ChangeSet>(ordinary.clone()).is_ok());
    ordinary.as_object_mut().unwrap().remove("epoch_start");
    assert!(!change_validator.is_valid(&ordinary));
    assert!(serde_json::from_value::<ChangeSet>(ordinary).is_err());
}
