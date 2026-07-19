use mdstream_protocol::{
    ApplyOutcome, ChangeId, ChangeSet, ChildList, ChildListOwner, CitationProtocol, ContentKind,
    ContentNode, Coordinate, Epoch, NodeId, NodeProjection, NodeStability, ProjectionOp,
    ProtocolLimits, Reducer, ResourceId, SemanticResource, SemanticResourceKind, SemanticText,
    Sequence, SourceCursor, SourceDelta, SourceRange, TextTransition, TransitionFacts,
    TransitionReducer,
};

fn range(start: u64, end: u64) -> SourceRange {
    SourceRange::new(SourceCursor::new(start), SourceCursor::new(end))
}

fn source_text(id: u128, end: u64, stability: NodeStability) -> ContentNode {
    ContentNode::leaf(
        NodeId::new(id),
        stability,
        range(0, end),
        ContentKind::Text {
            text: SemanticText::Source {},
        },
    )
}

fn start_text(source: &str, stability: NodeStability) -> ChangeSet {
    text_epoch(Epoch::new(1), None, source, stability)
}

fn text_epoch(
    epoch: Epoch,
    predecessor: Option<Coordinate>,
    source: &str,
    stability: NodeStability,
) -> ChangeSet {
    let node = source_text(1, source.len() as u64, stability);
    let roots = ChildList::empty();
    let children = ChildList::empty();
    let paragraph = ContentNode::leaf(
        NodeId::new(2),
        stability,
        range(0, source.len() as u64),
        ContentKind::Paragraph {},
    );
    ChangeSet::start_epoch(
        epoch,
        ChangeId::new(format!("transition:start:{epoch}")).unwrap(),
        predecessor,
        SourceDelta::append(SourceCursor::new(0), source),
        vec![
            ProjectionOp::AdvanceProjection {
                expected_cursor: SourceCursor::new(0),
                new_cursor: SourceCursor::new(source.len() as u64),
            },
            ProjectionOp::InsertNode { node },
            ProjectionOp::InsertNode { node: paragraph },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Node {
                    node_id: NodeId::new(2),
                },
                expected_version: children.version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![NodeId::new(1)],
                new_version: children.version_after_append(&[NodeId::new(1)]),
            },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Document,
                expected_version: roots.version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![NodeId::new(2)],
                new_version: roots.version_after_append(&[NodeId::new(2)]),
            },
        ],
    )
    .unwrap()
}

fn normalized_children_start(count: usize) -> ChangeSet {
    let paragraph_id = NodeId::new((count + 1) as u128);
    let child_ids = (1..=count)
        .map(|id| NodeId::new(id as u128))
        .collect::<Vec<_>>();
    let mut operations = child_ids
        .iter()
        .enumerate()
        .map(|(index, id)| ProjectionOp::InsertNode {
            node: ContentNode::leaf(
                *id,
                NodeStability::Stable,
                range(0, 0),
                ContentKind::Text {
                    text: SemanticText::Normalized {
                        value: index.to_string(),
                    },
                },
            ),
        })
        .collect::<Vec<_>>();
    operations.push(ProjectionOp::InsertNode {
        node: ContentNode::leaf(
            paragraph_id,
            NodeStability::Stable,
            range(0, 0),
            ContentKind::Paragraph {},
        ),
    });
    let empty = ChildList::empty();
    operations.push(ProjectionOp::SpliceChildren {
        owner: ChildListOwner::Node {
            node_id: paragraph_id,
        },
        expected_version: empty.version().clone(),
        start: 0,
        delete_count: 0,
        new_version: empty.version_after_append(&child_ids),
        insert: child_ids,
    });
    operations.push(ProjectionOp::SpliceChildren {
        owner: ChildListOwner::Document,
        expected_version: empty.version().clone(),
        start: 0,
        delete_count: 0,
        new_version: empty.version_after_append(&[paragraph_id]),
        insert: vec![paragraph_id],
    });
    ChangeSet::start_epoch(
        Epoch::new(1),
        ChangeId::new("transition:normalized-children").unwrap(),
        None,
        SourceDelta::unchanged(SourceCursor::new(0)),
        operations,
    )
    .unwrap()
}

#[test]
fn capture_is_opt_in_and_initial_install_has_detailed_facts() {
    let mut ordinary = Reducer::new();
    assert!(matches!(
        ordinary.apply(start_text("A", NodeStability::Provisional)),
        Ok(ApplyOutcome::Applied { .. })
    ));
    assert_eq!(ordinary.transition_metrics(), Default::default());
    let mut observed = TransitionReducer::new();
    let report = observed
        .apply(start_text("A", NodeStability::Provisional))
        .unwrap();
    let TransitionFacts::Continuous {
        before,
        after,
        nodes,
        structures,
        ..
    } = report.facts.unwrap()
    else {
        panic!("initial install must expose detailed transition facts");
    };
    assert!(before.is_none());
    assert_eq!(after.continuity_generation.get(), 0);
    assert_eq!(nodes.len(), 2);
    assert!(nodes.iter().all(|node| node.before.is_none()));
    assert!(nodes.iter().all(|node| node.after.is_some()));
    assert_eq!(structures.len(), 2);
    assert!(structures.iter().all(|structure| structure.start == 0));
    assert!(
        structures
            .iter()
            .all(|structure| structure.removed.is_empty())
    );
    assert!(
        structures
            .iter()
            .all(|structure| structure.inserted.len() == 1)
    );

    let metrics = observed.transition_metrics();
    assert_eq!(metrics.facts_built, 1);
    assert!(metrics.entity_visits > 0);
    assert_eq!(metrics.splice_ids_copied, 4);
}

#[test]
fn source_projection_extension_owns_the_exact_utf8_delta() {
    for (case, suffix) in [("ascii", "B"), ("emoji", "🙂"), ("combining", "\u{301}")] {
        let mut reducer = TransitionReducer::new();
        reducer
            .apply(start_text("A", NodeStability::Provisional))
            .unwrap();
        let current = reducer.document().unwrap().node(NodeId::new(1)).unwrap();
        let paragraph = reducer.document().unwrap().node(NodeId::new(2)).unwrap();
        let end = 1 + suffix.len() as u64;
        let change = ChangeSet::new(
            Epoch::new(1),
            Sequence::new(1),
            ChangeId::new(format!("transition:{case}")).unwrap(),
            SourceDelta::append(SourceCursor::new(1), suffix),
            vec![
                ProjectionOp::AdvanceProjection {
                    expected_cursor: SourceCursor::new(1),
                    new_cursor: SourceCursor::new(end),
                },
                ProjectionOp::ReplaceNode {
                    node_id: NodeId::new(1),
                    expected_version: current.version.clone(),
                    projection: NodeProjection::new(
                        NodeStability::Provisional,
                        range(0, end),
                        range(0, end),
                        ContentKind::Text {
                            text: SemanticText::Source {},
                        },
                    ),
                },
                ProjectionOp::ReplaceNode {
                    node_id: NodeId::new(2),
                    expected_version: paragraph.version.clone(),
                    projection: NodeProjection::new(
                        NodeStability::Provisional,
                        range(0, end),
                        range(0, end),
                        ContentKind::Paragraph {},
                    ),
                },
            ],
        )
        .unwrap();

        let report = reducer.apply(change).unwrap();
        let TransitionFacts::Continuous { nodes, .. } = report.facts.unwrap() else {
            panic!("ordinary changes must expose continuous facts");
        };
        let text = nodes
            .iter()
            .find(|node| node.key.node_id == NodeId::new(1))
            .and_then(|node| node.text.clone())
            .unwrap();
        assert_eq!(
            text,
            TextTransition::ProjectionAppend {
                range: range(1, end),
                text: suffix.to_string(),
            },
            "{case}"
        );
    }
}

#[test]
fn idempotent_changes_do_not_create_transition_facts() {
    let start = start_text("A", NodeStability::Provisional);
    let mut reducer = TransitionReducer::new();
    reducer.apply(start.clone()).unwrap();
    let report = reducer.apply(start).unwrap();
    assert_eq!(report.outcome, ApplyOutcome::Idempotent);
    assert!(report.facts.is_none());
}

#[test]
fn same_floor_recovery_is_control_only_but_advanced_recovery_is_a_barrier() {
    let start = start_text("A", NodeStability::Provisional);
    let gap = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(2),
        ChangeId::new("transition:gap").unwrap(),
        SourceDelta::append(SourceCursor::new(1), "gap"),
        Vec::new(),
    )
    .unwrap();

    let mut same_floor = TransitionReducer::new();
    same_floor.apply(start.clone()).unwrap();
    let snapshot = same_floor.document().unwrap().snapshot();
    let gap_report = same_floor.apply(gap.clone()).unwrap();
    assert!(matches!(
        gap_report.outcome,
        ApplyOutcome::RecoveryRequired { .. }
    ));
    assert!(gap_report.facts.is_none());
    let generation = same_floor.continuity_generation();
    let recovered = same_floor.recover_snapshot(snapshot).unwrap();
    assert!(recovered.facts.is_none());
    assert_eq!(same_floor.continuity_generation(), generation);

    let mut producer = Reducer::new();
    producer.apply(start).unwrap();
    producer
        .apply(
            ChangeSet::new(
                Epoch::new(1),
                Sequence::new(1),
                ChangeId::new("transition:advanced").unwrap(),
                SourceDelta::append(SourceCursor::new(1), "B"),
                Vec::new(),
            )
            .unwrap(),
        )
        .unwrap();

    let mut advanced = TransitionReducer::new();
    advanced
        .apply(start_text("A", NodeStability::Provisional))
        .unwrap();
    advanced.apply(gap).unwrap();
    let recovered = advanced
        .recover_snapshot(producer.document().unwrap().snapshot())
        .unwrap();
    let TransitionFacts::FullReplace { before, after } = recovered.facts.unwrap() else {
        panic!("advanced snapshot recovery must be a full-replace barrier");
    };
    assert_eq!(before.unwrap().continuity_generation.get(), 0);
    assert_eq!(after.continuity_generation.get(), 1);
    assert_eq!(advanced.continuity_generation().get(), 1);
}

#[test]
fn an_owned_append_fact_survives_a_later_a_b_a_correction() {
    let mut reducer = TransitionReducer::new();
    reducer
        .apply(start_text("A", NodeStability::Provisional))
        .unwrap();
    let current = reducer.document().unwrap().node(NodeId::new(1)).unwrap();
    let paragraph = reducer.document().unwrap().node(NodeId::new(2)).unwrap();
    let append = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(1),
        ChangeId::new("transition:a-to-ab").unwrap(),
        SourceDelta::append(SourceCursor::new(1), "B"),
        vec![
            ProjectionOp::AdvanceProjection {
                expected_cursor: SourceCursor::new(1),
                new_cursor: SourceCursor::new(2),
            },
            ProjectionOp::ReplaceNode {
                node_id: NodeId::new(1),
                expected_version: current.version.clone(),
                projection: NodeProjection::new(
                    NodeStability::Provisional,
                    range(0, 2),
                    range(0, 2),
                    ContentKind::Text {
                        text: SemanticText::Source {},
                    },
                ),
            },
            ProjectionOp::ReplaceNode {
                node_id: NodeId::new(2),
                expected_version: paragraph.version.clone(),
                projection: NodeProjection::new(
                    NodeStability::Provisional,
                    range(0, 2),
                    range(0, 2),
                    ContentKind::Paragraph {},
                ),
            },
        ],
    )
    .unwrap();
    let append_report = reducer.apply(append).unwrap();

    let current = reducer.document().unwrap().node(NodeId::new(1)).unwrap();
    let paragraph = reducer.document().unwrap().node(NodeId::new(2)).unwrap();
    let correction = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(2),
        ChangeId::new("transition:ab-to-a").unwrap(),
        SourceDelta::unchanged(SourceCursor::new(2)),
        vec![
            ProjectionOp::ReplaceNode {
                node_id: NodeId::new(1),
                expected_version: current.version.clone(),
                projection: NodeProjection::new(
                    NodeStability::Provisional,
                    range(0, 1),
                    range(0, 1),
                    ContentKind::Text {
                        text: SemanticText::Source {},
                    },
                ),
            },
            ProjectionOp::ReplaceNode {
                node_id: NodeId::new(2),
                expected_version: paragraph.version.clone(),
                projection: NodeProjection::new(
                    NodeStability::Provisional,
                    range(0, 1),
                    range(0, 1),
                    ContentKind::Paragraph {},
                ),
            },
        ],
    )
    .unwrap();
    let correction_report = reducer.apply(correction).unwrap();

    let text_for = |facts: TransitionFacts| match facts {
        TransitionFacts::Continuous { nodes, .. } => nodes
            .into_iter()
            .find(|node| node.key.node_id == NodeId::new(1))
            .and_then(|node| node.text),
        TransitionFacts::FullReplace { .. } => None,
    };
    assert_eq!(
        text_for(append_report.facts.unwrap()),
        Some(TextTransition::ProjectionAppend {
            range: range(1, 2),
            text: "B".to_string(),
        })
    );
    assert_eq!(
        text_for(correction_report.facts.unwrap()),
        Some(TextTransition::Replacement)
    );
}

#[test]
fn structure_facts_normalize_only_the_authored_edit_window() {
    let mut reducer = TransitionReducer::new();
    reducer.apply(normalized_children_start(4)).unwrap();
    let paragraph_id = NodeId::new(5);
    let current = &reducer
        .document()
        .unwrap()
        .node(paragraph_id)
        .unwrap()
        .children;
    let replacement = vec![
        NodeId::new(1),
        NodeId::new(3),
        NodeId::new(2),
        NodeId::new(4),
    ];
    let change = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(1),
        ChangeId::new("transition:normalized-splice").unwrap(),
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![ProjectionOp::SpliceChildren {
            owner: ChildListOwner::Node {
                node_id: paragraph_id,
            },
            expected_version: current.version().clone(),
            start: 0,
            delete_count: 4,
            new_version: ChildList::new(replacement.clone()).version().clone(),
            insert: replacement,
        }],
    )
    .unwrap();

    let before_metrics = reducer.transition_metrics();
    let report = reducer.apply(change).unwrap();
    let TransitionFacts::Continuous {
        nodes, structures, ..
    } = report.facts.unwrap()
    else {
        panic!("a reorder is continuous");
    };
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].key.node_id, paragraph_id);
    assert_ne!(
        nodes[0].before.as_ref().unwrap().children_version,
        nodes[0].after.as_ref().unwrap().children_version
    );
    assert_eq!(structures.len(), 1);
    let structure = &structures[0];
    assert_eq!(structure.start, 1);
    assert_eq!(
        structure
            .removed
            .iter()
            .map(|key| key.node_id)
            .collect::<Vec<_>>(),
        vec![NodeId::new(2), NodeId::new(3)]
    );
    assert_eq!(
        structure
            .inserted
            .iter()
            .map(|key| key.node_id)
            .collect::<Vec<_>>(),
        vec![NodeId::new(3), NodeId::new(2)]
    );
    assert_eq!(
        reducer
            .transition_metrics()
            .splice_ids_copied
            .saturating_sub(before_metrics.splice_ids_copied),
        8
    );
}

#[test]
fn facts_work_for_a_local_splice_does_not_scale_with_the_child_list() {
    const CHILDREN: usize = 10_000;
    let limits = ProtocolLimits {
        max_operations: CHILDREN + 4,
        ..ProtocolLimits::default()
    };
    let mut reducer = TransitionReducer::with_limits(limits);
    reducer.apply(normalized_children_start(CHILDREN)).unwrap();
    let paragraph_id = NodeId::new((CHILDREN + 1) as u128);
    let current = &reducer
        .document()
        .unwrap()
        .node(paragraph_id)
        .unwrap()
        .children;
    let middle = CHILDREN / 2;
    let swapped = vec![
        NodeId::new((middle + 2) as u128),
        NodeId::new((middle + 1) as u128),
    ];
    let mut replacement = current.as_slice().to_vec();
    replacement.splice(middle..middle + 2, swapped.iter().copied());
    let change = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(1),
        ChangeId::new("transition:large-local-splice").unwrap(),
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![ProjectionOp::SpliceChildren {
            owner: ChildListOwner::Node {
                node_id: paragraph_id,
            },
            expected_version: current.version().clone(),
            start: middle as u32,
            delete_count: 2,
            insert: swapped,
            new_version: ChildList::new(replacement).version().clone(),
        }],
    )
    .unwrap();

    let before = reducer.transition_metrics();
    reducer.apply(change).unwrap();
    let after = reducer.transition_metrics();
    assert_eq!(after.splice_ids_copied - before.splice_ids_copied, 8);
    assert!(after.entity_visits - before.entity_visits <= 3);
}

#[test]
fn resource_corrections_report_versions_and_the_complete_affected_fanout() {
    let resource = SemanticResource::new(
        ResourceId::new(1),
        SemanticResourceKind::Citation {
            protocol: CitationProtocol::V1,
            key: "paper".to_string(),
            destination: "https://example.test/old".to_string(),
            title: None,
        },
    );
    let reference = ContentNode::leaf(
        NodeId::new(1),
        NodeStability::Stable,
        range(0, 0),
        ContentKind::CitationReference {
            key: "paper".to_string(),
            target: Some(resource.reference()),
        },
    );
    let paragraph = ContentNode::leaf(
        NodeId::new(2),
        NodeStability::Stable,
        range(0, 0),
        ContentKind::Paragraph {},
    );
    let empty = ChildList::empty();
    let start = ChangeSet::start_epoch(
        Epoch::new(1),
        ChangeId::new("transition:resource-start").unwrap(),
        None,
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![
            ProjectionOp::InsertResource {
                resource: resource.clone(),
            },
            ProjectionOp::InsertNode { node: reference },
            ProjectionOp::InsertNode { node: paragraph },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Node {
                    node_id: NodeId::new(2),
                },
                expected_version: empty.version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![NodeId::new(1)],
                new_version: empty.version_after_append(&[NodeId::new(1)]),
            },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Document,
                expected_version: empty.version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![NodeId::new(2)],
                new_version: empty.version_after_append(&[NodeId::new(2)]),
            },
        ],
    )
    .unwrap();
    let mut reducer = TransitionReducer::new();
    reducer.apply(start).unwrap();

    let corrected = SemanticResource::new(
        ResourceId::new(1),
        SemanticResourceKind::Citation {
            protocol: CitationProtocol::V1,
            key: "paper".to_string(),
            destination: "https://example.test/new".to_string(),
            title: Some("Revised".to_string()),
        },
    );
    let report = reducer
        .apply(
            ChangeSet::new(
                Epoch::new(1),
                Sequence::new(1),
                ChangeId::new("transition:resource-correction").unwrap(),
                SourceDelta::unchanged(SourceCursor::new(0)),
                vec![ProjectionOp::ReplaceResource {
                    resource_id: ResourceId::new(1),
                    expected_version: resource.version.clone(),
                    resource: corrected.clone(),
                }],
            )
            .unwrap(),
        )
        .unwrap();
    let TransitionFacts::Continuous {
        nodes, resources, ..
    } = report.facts.unwrap()
    else {
        panic!("resource correction is continuous");
    };
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].key.node_id, NodeId::new(1));
    assert!(nodes[0].text.is_none());
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].before_version, Some(resource.version));
    assert_eq!(resources[0].after_version, Some(corrected.version));
    assert_eq!(resources[0].affected_nodes.len(), 1);
    assert_eq!(resources[0].affected_nodes[0].node_id, NodeId::new(1));
}

#[test]
fn resource_facts_scale_with_a_ten_thousand_node_fanout() {
    const USERS: u128 = 10_000;
    let resource = SemanticResource::new(
        ResourceId::new(1),
        SemanticResourceKind::Citation {
            protocol: CitationProtocol::V1,
            key: "shared".to_string(),
            destination: "https://example.test/old".to_string(),
            title: None,
        },
    );
    let paragraph_id = NodeId::new(USERS + 1);
    let paragraph = ContentNode::leaf(
        paragraph_id,
        NodeStability::Stable,
        range(0, 0),
        ContentKind::Paragraph {},
    );
    let reference = |id| {
        ContentNode::leaf(
            NodeId::new(id),
            NodeStability::Stable,
            range(0, 0),
            ContentKind::CitationReference {
                key: "shared".to_string(),
                target: Some(resource.reference()),
            },
        )
    };
    let first_ids = (1..=USERS / 2).map(NodeId::new).collect::<Vec<_>>();
    let empty = ChildList::empty();
    let mut first = vec![
        ProjectionOp::InsertResource {
            resource: resource.clone(),
        },
        ProjectionOp::InsertNode {
            node: paragraph.clone(),
        },
    ];
    first.extend(
        (1..=USERS / 2)
            .map(reference)
            .map(|node| ProjectionOp::InsertNode { node }),
    );
    first.push(ProjectionOp::SpliceChildren {
        owner: ChildListOwner::Node {
            node_id: paragraph_id,
        },
        expected_version: empty.version().clone(),
        start: 0,
        delete_count: 0,
        insert: first_ids.clone(),
        new_version: empty.version_after_append(&first_ids),
    });
    first.push(ProjectionOp::SpliceChildren {
        owner: ChildListOwner::Document,
        expected_version: empty.version().clone(),
        start: 0,
        delete_count: 0,
        insert: vec![paragraph_id],
        new_version: empty.version_after_append(&[paragraph_id]),
    });
    let mut reducer = TransitionReducer::new();
    reducer
        .apply(
            ChangeSet::start_epoch(
                Epoch::new(1),
                ChangeId::new("transition:fanout-start").unwrap(),
                None,
                SourceDelta::unchanged(SourceCursor::new(0)),
                first,
            )
            .unwrap(),
        )
        .unwrap();

    let second_ids = (USERS / 2 + 1..=USERS).map(NodeId::new).collect::<Vec<_>>();
    let current = &reducer
        .document()
        .unwrap()
        .node(paragraph_id)
        .unwrap()
        .children;
    let mut second = (USERS / 2 + 1..=USERS)
        .map(reference)
        .map(|node| ProjectionOp::InsertNode { node })
        .collect::<Vec<_>>();
    second.push(ProjectionOp::SpliceChildren {
        owner: ChildListOwner::Node {
            node_id: paragraph_id,
        },
        expected_version: current.version().clone(),
        start: (USERS / 2) as u32,
        delete_count: 0,
        insert: second_ids.clone(),
        new_version: current.version_after_append(&second_ids),
    });
    reducer
        .apply(
            ChangeSet::new(
                Epoch::new(1),
                Sequence::new(1),
                ChangeId::new("transition:fanout-second").unwrap(),
                SourceDelta::unchanged(SourceCursor::new(0)),
                second,
            )
            .unwrap(),
        )
        .unwrap();

    let corrected = SemanticResource::new(
        ResourceId::new(1),
        SemanticResourceKind::Citation {
            protocol: CitationProtocol::V1,
            key: "shared".to_string(),
            destination: "https://example.test/new".to_string(),
            title: None,
        },
    );
    let before = reducer.transition_metrics();
    let report = reducer
        .apply(
            ChangeSet::new(
                Epoch::new(1),
                Sequence::new(2),
                ChangeId::new("transition:fanout-correction").unwrap(),
                SourceDelta::unchanged(SourceCursor::new(0)),
                vec![ProjectionOp::ReplaceResource {
                    resource_id: ResourceId::new(1),
                    expected_version: resource.version,
                    resource: corrected,
                }],
            )
            .unwrap(),
        )
        .unwrap();
    let TransitionFacts::Continuous {
        nodes, resources, ..
    } = report.facts.unwrap()
    else {
        panic!("resource fanout correction is continuous");
    };
    assert_eq!(nodes.len(), USERS as usize);
    assert!(nodes.iter().all(|node| node.text.is_none()));
    assert_eq!(resources[0].affected_nodes.len(), USERS as usize);
    assert!(reducer.transition_metrics().entity_visits - before.entity_visits <= 30_001);
}

#[test]
fn pending_source_catch_up_is_an_append_fact_without_a_freshness_claim() {
    let text = source_text(1, 1, NodeStability::Provisional);
    let paragraph = ContentNode::leaf(
        NodeId::new(2),
        NodeStability::Provisional,
        range(0, 1),
        ContentKind::Paragraph {},
    );
    let empty = ChildList::empty();
    let start = ChangeSet::start_epoch(
        Epoch::new(1),
        ChangeId::new("transition:pending-start").unwrap(),
        None,
        SourceDelta::append(SourceCursor::new(0), "AB"),
        vec![
            ProjectionOp::AdvanceProjection {
                expected_cursor: SourceCursor::new(0),
                new_cursor: SourceCursor::new(1),
            },
            ProjectionOp::InsertNode { node: text },
            ProjectionOp::InsertNode { node: paragraph },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Node {
                    node_id: NodeId::new(2),
                },
                expected_version: empty.version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![NodeId::new(1)],
                new_version: empty.version_after_append(&[NodeId::new(1)]),
            },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Document,
                expected_version: empty.version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![NodeId::new(2)],
                new_version: empty.version_after_append(&[NodeId::new(2)]),
            },
        ],
    )
    .unwrap();
    let mut reducer = TransitionReducer::new();
    reducer.apply(start).unwrap();
    assert_eq!(reducer.document().unwrap().pending_source(), "B");
    let pending_snapshot = reducer.document().unwrap().snapshot();
    let text = reducer.document().unwrap().node(NodeId::new(1)).unwrap();
    let paragraph = reducer.document().unwrap().node(NodeId::new(2)).unwrap();
    let catch_up = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(1),
        ChangeId::new("transition:pending-catch-up").unwrap(),
        SourceDelta::unchanged(SourceCursor::new(2)),
        vec![
            ProjectionOp::AdvanceProjection {
                expected_cursor: SourceCursor::new(1),
                new_cursor: SourceCursor::new(2),
            },
            ProjectionOp::ReplaceNode {
                node_id: NodeId::new(1),
                expected_version: text.version.clone(),
                projection: NodeProjection::new(
                    NodeStability::Provisional,
                    range(0, 2),
                    range(0, 2),
                    ContentKind::Text {
                        text: SemanticText::Source {},
                    },
                ),
            },
            ProjectionOp::ReplaceNode {
                node_id: NodeId::new(2),
                expected_version: paragraph.version.clone(),
                projection: NodeProjection::new(
                    NodeStability::Provisional,
                    range(0, 2),
                    range(0, 2),
                    ContentKind::Paragraph {},
                ),
            },
        ],
    )
    .unwrap();

    let report = reducer.apply(catch_up).unwrap();
    let TransitionFacts::Continuous { before, nodes, .. } = report.facts.unwrap() else {
        panic!("pending catch-up is continuous");
    };
    assert_eq!(before.unwrap().projection_cursor, SourceCursor::new(1));
    assert_eq!(
        nodes
            .into_iter()
            .find(|node| node.key.node_id == NodeId::new(1))
            .and_then(|node| node.text),
        Some(TextTransition::ProjectionAppend {
            range: range(1, 2),
            text: "B".to_string(),
        })
    );

    let mut mixed = TransitionReducer::new();
    mixed.recover_snapshot(pending_snapshot).unwrap();
    let text = mixed.document().unwrap().node(NodeId::new(1)).unwrap();
    let paragraph = mixed.document().unwrap().node(NodeId::new(2)).unwrap();
    let mixed_report = mixed
        .apply(
            ChangeSet::new(
                Epoch::new(1),
                Sequence::new(1),
                ChangeId::new("transition:pending-plus-fresh").unwrap(),
                SourceDelta::append(SourceCursor::new(2), "C"),
                vec![
                    ProjectionOp::AdvanceProjection {
                        expected_cursor: SourceCursor::new(1),
                        new_cursor: SourceCursor::new(3),
                    },
                    ProjectionOp::ReplaceNode {
                        node_id: NodeId::new(1),
                        expected_version: text.version.clone(),
                        projection: NodeProjection::new(
                            NodeStability::Provisional,
                            range(0, 3),
                            range(0, 3),
                            ContentKind::Text {
                                text: SemanticText::Source {},
                            },
                        ),
                    },
                    ProjectionOp::ReplaceNode {
                        node_id: NodeId::new(2),
                        expected_version: paragraph.version.clone(),
                        projection: NodeProjection::new(
                            NodeStability::Provisional,
                            range(0, 3),
                            range(0, 3),
                            ContentKind::Paragraph {},
                        ),
                    },
                ],
            )
            .unwrap(),
        )
        .unwrap();
    let TransitionFacts::Continuous { nodes, .. } = mixed_report.facts.unwrap() else {
        panic!("mixed pending and fresh projection is continuous");
    };
    assert_eq!(
        nodes
            .into_iter()
            .find(|node| node.key.node_id == NodeId::new(1))
            .and_then(|node| node.text),
        Some(TextTransition::ProjectionAppend {
            range: range(1, 3),
            text: "BC".to_string(),
        })
    );
}

#[test]
fn normalized_text_replaces_while_finish_only_changes_document_state() {
    let mut reducer = TransitionReducer::new();
    reducer.apply(normalized_children_start(1)).unwrap();
    let current = reducer.document().unwrap().node(NodeId::new(1)).unwrap();
    let report = reducer
        .apply(
            ChangeSet::new(
                Epoch::new(1),
                Sequence::new(1),
                ChangeId::new("transition:normalized-replace").unwrap(),
                SourceDelta::unchanged(SourceCursor::new(0)),
                vec![ProjectionOp::ReplaceNode {
                    node_id: NodeId::new(1),
                    expected_version: current.version.clone(),
                    projection: NodeProjection::new(
                        NodeStability::Stable,
                        range(0, 0),
                        range(0, 0),
                        ContentKind::Text {
                            text: SemanticText::Normalized {
                                value: "replacement".to_string(),
                            },
                        },
                    ),
                }],
            )
            .unwrap(),
        )
        .unwrap();
    let TransitionFacts::Continuous { nodes, .. } = report.facts.unwrap() else {
        panic!("normalized correction is continuous");
    };
    assert_eq!(nodes[0].text, Some(TextTransition::Replacement));

    let finish = reducer
        .apply(
            ChangeSet::new(
                Epoch::new(1),
                Sequence::new(2),
                ChangeId::new("transition:finish").unwrap(),
                SourceDelta::unchanged(SourceCursor::new(0)),
                vec![ProjectionOp::FinishDocument],
            )
            .unwrap(),
        )
        .unwrap();
    let TransitionFacts::Continuous {
        before,
        after,
        nodes,
        structures,
        resources,
    } = finish.facts.unwrap()
    else {
        panic!("finish is a continuous lifecycle transition");
    };
    assert_ne!(before.unwrap().lifecycle, after.lifecycle);
    assert!(nodes.is_empty());
    assert!(structures.is_empty());
    assert!(resources.is_empty());
}

#[test]
fn stabilization_changes_node_state_without_claiming_text_replacement() {
    let mut reducer = TransitionReducer::new();
    reducer
        .apply(start_text("A", NodeStability::Provisional))
        .unwrap();
    let text = reducer.document().unwrap().node(NodeId::new(1)).unwrap();
    let paragraph = reducer.document().unwrap().node(NodeId::new(2)).unwrap();
    let stable_text = NodeProjection::new(
        NodeStability::Stable,
        text.source,
        text.body,
        text.content.clone(),
    );
    let stable_paragraph = NodeProjection::new(
        NodeStability::Stable,
        paragraph.source,
        paragraph.body,
        paragraph.content.clone(),
    );
    let report = reducer
        .apply(
            ChangeSet::new(
                Epoch::new(1),
                Sequence::new(1),
                ChangeId::new("transition:stabilize").unwrap(),
                SourceDelta::unchanged(SourceCursor::new(1)),
                vec![
                    ProjectionOp::StabilizeNode {
                        node_id: NodeId::new(1),
                        expected_version: text.version.clone(),
                        new_version: stable_text.version,
                    },
                    ProjectionOp::StabilizeNode {
                        node_id: NodeId::new(2),
                        expected_version: paragraph.version.clone(),
                        new_version: stable_paragraph.version,
                    },
                ],
            )
            .unwrap(),
        )
        .unwrap();
    let TransitionFacts::Continuous { nodes, .. } = report.facts.unwrap() else {
        panic!("stabilization is continuous");
    };
    assert_eq!(nodes.len(), 2);
    assert!(nodes.iter().all(|node| {
        node.before.as_ref().unwrap().stability == NodeStability::Provisional
            && node.after.as_ref().unwrap().stability == NodeStability::Stable
            && node.text.is_none()
    }));
}

#[test]
fn reparenting_reports_both_structure_edits_and_the_node_parent_change() {
    let text = |id| {
        ContentNode::leaf(
            NodeId::new(id),
            NodeStability::Stable,
            range(0, 0),
            ContentKind::Text {
                text: SemanticText::Normalized {
                    value: id.to_string(),
                },
            },
        )
    };
    let paragraph = |id| {
        ContentNode::leaf(
            NodeId::new(id),
            NodeStability::Stable,
            range(0, 0),
            ContentKind::Paragraph {},
        )
    };
    let empty = ChildList::empty();
    let start = ChangeSet::start_epoch(
        Epoch::new(1),
        ChangeId::new("transition:reparent-start").unwrap(),
        None,
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![
            ProjectionOp::InsertNode { node: text(1) },
            ProjectionOp::InsertNode { node: text(2) },
            ProjectionOp::InsertNode { node: paragraph(3) },
            ProjectionOp::InsertNode { node: paragraph(4) },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Node {
                    node_id: NodeId::new(3),
                },
                expected_version: empty.version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![NodeId::new(1)],
                new_version: empty.version_after_append(&[NodeId::new(1)]),
            },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Node {
                    node_id: NodeId::new(4),
                },
                expected_version: empty.version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![NodeId::new(2)],
                new_version: empty.version_after_append(&[NodeId::new(2)]),
            },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Document,
                expected_version: empty.version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![NodeId::new(3), NodeId::new(4)],
                new_version: empty.version_after_append(&[NodeId::new(3), NodeId::new(4)]),
            },
        ],
    )
    .unwrap();
    let mut reducer = TransitionReducer::new();
    reducer.apply(start).unwrap();
    let left = &reducer
        .document()
        .unwrap()
        .node(NodeId::new(3))
        .unwrap()
        .children;
    let right = &reducer
        .document()
        .unwrap()
        .node(NodeId::new(4))
        .unwrap()
        .children;
    let report = reducer
        .apply(
            ChangeSet::new(
                Epoch::new(1),
                Sequence::new(1),
                ChangeId::new("transition:reparent").unwrap(),
                SourceDelta::unchanged(SourceCursor::new(0)),
                vec![
                    ProjectionOp::SpliceChildren {
                        owner: ChildListOwner::Node {
                            node_id: NodeId::new(3),
                        },
                        expected_version: left.version().clone(),
                        start: 0,
                        delete_count: 1,
                        insert: Vec::new(),
                        new_version: ChildList::empty().version().clone(),
                    },
                    ProjectionOp::SpliceChildren {
                        owner: ChildListOwner::Node {
                            node_id: NodeId::new(4),
                        },
                        expected_version: right.version().clone(),
                        start: 1,
                        delete_count: 0,
                        insert: vec![NodeId::new(1)],
                        new_version: right.version_after_append(&[NodeId::new(1)]),
                    },
                ],
            )
            .unwrap(),
        )
        .unwrap();
    let TransitionFacts::Continuous {
        nodes, structures, ..
    } = report.facts.unwrap()
    else {
        panic!("reparenting is continuous");
    };
    assert_eq!(structures.len(), 2);
    let moved = nodes
        .iter()
        .find(|node| node.key.node_id == NodeId::new(1))
        .unwrap();
    let parent_id = |stamp: &mdstream_protocol::NodeStateStamp| match stamp.parent.unwrap() {
        mdstream_protocol::TransitionChildListOwner::Node { key } => key.node_id,
        mdstream_protocol::TransitionChildListOwner::Document => panic!("expected node owner"),
    };
    assert_eq!(parent_id(moved.before.as_ref().unwrap()), NodeId::new(3));
    assert_eq!(parent_id(moved.after.as_ref().unwrap()), NodeId::new(4));
}

#[test]
fn duplicate_splices_for_one_owner_fail_atomically_without_transition_work() {
    let mut reducer = TransitionReducer::new();
    reducer.apply(normalized_children_start(4)).unwrap();
    let paragraph_id = NodeId::new(5);
    let current = &reducer
        .document()
        .unwrap()
        .node(paragraph_id)
        .unwrap()
        .children;
    let operation = ProjectionOp::SpliceChildren {
        owner: ChildListOwner::Node {
            node_id: paragraph_id,
        },
        expected_version: current.version().clone(),
        start: 0,
        delete_count: 1,
        insert: vec![NodeId::new(1)],
        new_version: current.version().clone(),
    };
    let invalid = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(1),
        ChangeId::new("transition:duplicate-owner").unwrap(),
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![operation.clone(), operation],
    )
    .unwrap();
    let before = reducer.document().unwrap().snapshot();
    let metrics = reducer.transition_metrics();

    assert!(matches!(
        reducer.apply(invalid),
        Err(mdstream_protocol::TransitionError::Protocol(
            mdstream_protocol::ProtocolError::InvalidChange(_)
        ))
    ));
    assert_eq!(reducer.document().unwrap().snapshot(), before);
    assert_eq!(reducer.transition_metrics(), metrics);
}

#[test]
fn a_removed_owner_does_not_publish_an_intermediate_child_list_fact() {
    let parent = ContentNode::leaf(
        NodeId::new(1),
        NodeStability::Stable,
        range(0, 1),
        ContentKind::BlockQuote {
            style: Default::default(),
        },
    );
    let child = ContentNode::leaf(
        NodeId::new(2),
        NodeStability::Stable,
        range(0, 1),
        ContentKind::Paragraph {},
    );
    let empty = ChildList::empty();
    let start = ChangeSet::start_epoch(
        Epoch::new(1),
        ChangeId::new("transition:remove-owner-start").unwrap(),
        None,
        SourceDelta::append(SourceCursor::new(0), "a"),
        vec![
            ProjectionOp::AdvanceProjection {
                expected_cursor: SourceCursor::new(0),
                new_cursor: SourceCursor::new(1),
            },
            ProjectionOp::InsertNode {
                node: parent.clone(),
            },
            ProjectionOp::InsertNode { node: child },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Node {
                    node_id: NodeId::new(1),
                },
                expected_version: empty.version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![NodeId::new(2)],
                new_version: empty.version_after_append(&[NodeId::new(2)]),
            },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Document,
                expected_version: empty.version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![NodeId::new(1)],
                new_version: empty.version_after_append(&[NodeId::new(1)]),
            },
        ],
    )
    .unwrap();
    let mut reducer = TransitionReducer::new();
    reducer.apply(start).unwrap();
    let roots = reducer.document().unwrap().roots();
    let children = &reducer
        .document()
        .unwrap()
        .node(NodeId::new(1))
        .unwrap()
        .children;
    let report = reducer
        .apply(
            ChangeSet::new(
                Epoch::new(1),
                Sequence::new(1),
                ChangeId::new("transition:extract-and-remove").unwrap(),
                SourceDelta::unchanged(SourceCursor::new(1)),
                vec![
                    ProjectionOp::SpliceChildren {
                        owner: ChildListOwner::Node {
                            node_id: NodeId::new(1),
                        },
                        expected_version: children.version().clone(),
                        start: 0,
                        delete_count: 1,
                        insert: Vec::new(),
                        new_version: ChildList::empty().version().clone(),
                    },
                    ProjectionOp::SpliceChildren {
                        owner: ChildListOwner::Document,
                        expected_version: roots.version().clone(),
                        start: 0,
                        delete_count: 1,
                        insert: vec![NodeId::new(2)],
                        new_version: ChildList::new(vec![NodeId::new(2)]).version().clone(),
                    },
                    ProjectionOp::RemoveNode {
                        node_id: NodeId::new(1),
                        expected_version: parent.version,
                    },
                ],
            )
            .unwrap(),
        )
        .unwrap();
    let TransitionFacts::Continuous { structures, .. } = report.facts.unwrap() else {
        panic!("extract-and-remove is continuous");
    };
    assert_eq!(structures.len(), 1);
    assert!(matches!(
        structures[0].owner,
        mdstream_protocol::TransitionChildListOwner::Document
    ));
    assert_eq!(
        reducer.document().unwrap().roots().as_slice(),
        &[NodeId::new(2)]
    );
}

#[test]
fn bootstrap_snapshot_is_an_advanced_full_replace_generation() {
    let mut producer = Reducer::new();
    producer
        .apply(start_text("A", NodeStability::Provisional))
        .unwrap();
    let mut consumer = TransitionReducer::new();
    let report = consumer
        .recover_snapshot(producer.document().unwrap().snapshot())
        .unwrap();
    let TransitionFacts::FullReplace { before, after } = report.facts.unwrap() else {
        panic!("snapshot bootstrap is a full replacement");
    };
    assert!(before.is_none());
    assert_eq!(after.continuity_generation.get(), 1);
    assert_eq!(consumer.continuity_generation().get(), 1);
}

#[test]
fn predecessor_linked_epoch_reset_increments_continuity_and_has_no_details() {
    let mut reducer = TransitionReducer::new();
    reducer
        .apply(start_text("A", NodeStability::Provisional))
        .unwrap();
    let predecessor = reducer.document().unwrap().coordinate().clone();
    let report = reducer
        .apply(text_epoch(
            Epoch::new(2),
            Some(predecessor),
            "Z",
            NodeStability::Provisional,
        ))
        .unwrap();
    let TransitionFacts::FullReplace { before, after } = report.facts.unwrap() else {
        panic!("epoch reset must be a coarse full replacement");
    };
    assert_eq!(before.unwrap().continuity_generation.get(), 0);
    assert_eq!(after.continuity_generation.get(), 1);
    assert_eq!(after.coordinate.epoch, Epoch::new(2));
    assert_eq!(reducer.continuity_generation().get(), 1);
}

#[test]
fn transition_wire_shapes_are_closed_and_root_and_node_owners_are_distinct() {
    let mut reducer = TransitionReducer::new();
    let facts = reducer
        .apply(start_text("A", NodeStability::Provisional))
        .unwrap()
        .facts
        .unwrap();
    let value = serde_json::to_value(&facts).unwrap();
    let structures = value["structures"].as_array().unwrap();
    let owner_kinds = structures
        .iter()
        .map(|structure| structure["owner"]["kind"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(owner_kinds.contains(&"document"));
    assert!(owner_kinds.contains(&"node"));

    let mut unknown = value.clone();
    unknown["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<TransitionFacts>(unknown).is_err());
    let mut nested_unknown = value.clone();
    nested_unknown["nodes"][0]["after"]["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<TransitionFacts>(nested_unknown).is_err());
    let mut missing_nullable = value.clone();
    missing_nullable.as_object_mut().unwrap().remove("before");
    assert!(serde_json::from_value::<TransitionFacts>(missing_nullable).is_err());
    let mut missing_parent = value.clone();
    missing_parent["nodes"][0]["after"]
        .as_object_mut()
        .unwrap()
        .remove("parent");
    assert!(serde_json::from_value::<TransitionFacts>(missing_parent).is_err());
    let mut numeric_generation = value;
    numeric_generation["after"]["continuity_generation"] = serde_json::json!(0);
    assert!(serde_json::from_value::<TransitionFacts>(numeric_generation).is_err());
}
