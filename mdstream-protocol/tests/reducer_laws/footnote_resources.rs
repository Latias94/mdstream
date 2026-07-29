use super::*;

#[test]
fn footnote_resources_support_atomic_late_resolution_and_dependency_tracking() {
    let paragraph = leaf(0, NodeStability::Stable, (0, 0), ContentKind::Paragraph {});
    let unresolved = leaf(
        1,
        NodeStability::Stable,
        (0, 0),
        ContentKind::FootnoteReference {
            label: "note".to_string(),
            target: None,
        },
    );
    let mut reducer = Reducer::new();
    reducer
        .apply(
            start(
                1,
                "",
                vec![
                    ProjectionOp::InsertNode {
                        node: paragraph.clone(),
                    },
                    ProjectionOp::InsertNode {
                        node: unresolved.clone(),
                    },
                    append_splice(
                        ChildListOwner::Node {
                            node_id: paragraph.id,
                        },
                        &paragraph.children,
                        vec![unresolved.id],
                    ),
                    append_splice(
                        ChildListOwner::Document,
                        &ChildList::empty(),
                        vec![paragraph.id],
                    ),
                ],
            )
            .unwrap(),
        )
        .unwrap();

    let resource = SemanticResource::new(
        ResourceId::new(0),
        SemanticResourceKind::Footnote {
            label: "note".to_string(),
        },
    );
    let definition = leaf(
        2,
        NodeStability::Stable,
        (0, 0),
        ContentKind::FootnoteDefinition {
            label: "note".to_string(),
            target: resource.reference(),
        },
    );
    let resolved = leaf(
        1,
        NodeStability::Stable,
        (0, 0),
        ContentKind::FootnoteReference {
            label: "note".to_string(),
            target: Some(resource.reference()),
        },
    );
    let append_definition = append_splice(
        ChildListOwner::Document,
        reducer.document().unwrap().roots(),
        vec![definition.id],
    );
    reducer
        .apply(next_change(
            &reducer,
            1,
            "footnote:late-definition",
            "",
            vec![
                ProjectionOp::InsertResource {
                    resource: resource.clone(),
                },
                ProjectionOp::InsertNode {
                    node: definition.clone(),
                },
                ProjectionOp::ReplaceNode {
                    node_id: unresolved.id,
                    expected_version: unresolved.version.clone(),
                    projection: resolved.projection(),
                },
                append_definition,
            ],
        ))
        .unwrap();

    let document = reducer.document().unwrap();
    let resolved = document.node(unresolved.id).unwrap();
    assert_ne!(resolved.version, unresolved.version);
    assert_eq!(
        resolved.content.resource_ref().unwrap(),
        &resource.reference()
    );
    assert_eq!(
        document
            .node(definition.id)
            .unwrap()
            .content
            .resource_ref()
            .unwrap(),
        &resource.reference()
    );

    let before = document.clone();
    let removal = next_change(
        &reducer,
        2,
        "footnote:remove-in-use-resource",
        "",
        vec![ProjectionOp::RemoveResource {
            resource_id: resource.id,
            expected_version: resource.version.clone(),
        }],
    );
    assert_eq!(
        reducer.apply(removal),
        Err(ProtocolError::MissingResource(resource.id))
    );
    assert_eq!(reducer.document().unwrap(), &before);
}

#[test]
fn footnote_nodes_reject_resources_from_a_different_label() {
    let resource = SemanticResource::new(
        ResourceId::new(0),
        SemanticResourceKind::Footnote {
            label: "other".to_string(),
        },
    );
    let definition = leaf(
        0,
        NodeStability::Stable,
        (0, 0),
        ContentKind::FootnoteDefinition {
            label: "note".to_string(),
            target: resource.reference(),
        },
    );
    let mut reducer = Reducer::new();
    assert!(matches!(
        reducer.apply(
            start(
                1,
                "",
                vec![
                    ProjectionOp::InsertResource { resource },
                    ProjectionOp::InsertNode {
                        node: definition.clone(),
                    },
                    append_splice(
                        ChildListOwner::Document,
                        &ChildList::empty(),
                        vec![definition.id],
                    ),
                ],
            )
            .unwrap(),
        ),
        Err(ProtocolError::InvalidChange(_))
    ));
    assert!(reducer.document().is_none());
}

#[test]
fn unused_footnote_resource_replacement_preserves_label_identity_atomically() {
    let resource = SemanticResource::new(
        ResourceId::new(0),
        SemanticResourceKind::Footnote {
            label: "note".to_string(),
        },
    );
    let mut reducer = Reducer::new();
    reducer
        .apply(
            start(
                1,
                "",
                vec![ProjectionOp::InsertResource {
                    resource: resource.clone(),
                }],
            )
            .unwrap(),
        )
        .unwrap();
    let before = reducer.document().unwrap().clone();
    let before_coordinate = before.coordinate().clone();

    let same_label = next_change(
        &reducer,
        1,
        "footnote:replace-same-label",
        "",
        vec![ProjectionOp::ReplaceResource {
            resource_id: resource.id,
            expected_version: resource.version.clone(),
            resource: resource.clone(),
        }],
    );
    assert!(matches!(
        reducer.apply(same_label),
        Err(ProtocolError::InvalidChange(message))
            if message.contains("must change content")
    ));
    assert_eq!(reducer.document().unwrap(), &before);
    assert_eq!(reducer.document().unwrap().coordinate(), &before_coordinate);

    let different_label = SemanticResource::new(
        resource.id,
        SemanticResourceKind::Footnote {
            label: "other".to_string(),
        },
    );
    let replacement = next_change(
        &reducer,
        1,
        "footnote:replace-different-label",
        "",
        vec![ProjectionOp::ReplaceResource {
            resource_id: resource.id,
            expected_version: resource.version.clone(),
            resource: different_label.clone(),
        }],
    );
    assert!(matches!(
        reducer.apply(replacement),
        Err(ProtocolError::InvalidChange(message))
            if message.contains("semantic kind and identity")
    ));
    assert_eq!(reducer.document().unwrap(), &before);
    assert_eq!(reducer.document().unwrap().coordinate(), &before_coordinate);

    let mut value = serde_json::to_value(before.snapshot()).unwrap();
    value["coordinate"]["sequence"] = serde_json::json!("1");
    value["coordinate"]["change_id"] = serde_json::json!("footnote:snapshot-replacement");
    value["resources"][0] = serde_json::to_value(different_label).unwrap();
    let replacement_snapshot = snapshot_from_value(value);
    let gap = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(2),
        change_id("footnote:snapshot-gap"),
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![ProjectionOp::FinishDocument],
    )
    .unwrap();
    assert!(matches!(
        reducer.apply(gap).unwrap(),
        ApplyOutcome::RecoveryRequired {
            reason: RecoveryReason::SequenceGap { .. },
            ..
        }
    ));
    assert!(matches!(
        reducer.recover_snapshot(replacement_snapshot),
        Err(ProtocolError::InvalidSnapshot(message))
            if message.contains("semantic identity")
    ));
    assert_eq!(reducer.document().unwrap(), &before);
    assert_eq!(reducer.document().unwrap().coordinate(), &before_coordinate);
}
