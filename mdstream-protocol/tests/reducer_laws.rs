use std::collections::BTreeMap;

use mdstream_protocol::{
    ApplyOutcome, ChangeId, ChangeImpact, ChangeSet, ChildList, ChildListOwner, CitationProtocol,
    ContentKind, ContentNode, DocumentLifecycle, Epoch, LinkStyle, NodeId, NodeStability,
    NodeVersion, ProjectionOp, ProtocolError, ProtocolLimits, RecoveryReason, Reducer,
    ReducerStatus, ResourceId, SemanticResource, SemanticResourceKind, Sequence, Snapshot,
    SourceCursor, SourceDelta, SourceRange, encode_change_json, encode_snapshot_json,
};

#[path = "reducer_laws/footnote_resources.rs"]
mod footnote_resources;

fn change_id(value: &str) -> ChangeId {
    ChangeId::new(value).unwrap()
}

fn range(start: u64, end: u64) -> SourceRange {
    SourceRange::new(SourceCursor::new(start), SourceCursor::new(end))
}

fn leaf(id: u64, stability: NodeStability, span: (u64, u64), content: ContentKind) -> ContentNode {
    ContentNode::leaf(
        NodeId::new(u128::from(id)),
        stability,
        range(span.0, span.1),
        content,
    )
}

fn container(
    id: u64,
    stability: NodeStability,
    span: (u64, u64),
    children: Vec<NodeId>,
    content: ContentKind,
) -> ContentNode {
    ContentNode::new(
        NodeId::new(u128::from(id)),
        stability,
        range(span.0, span.1),
        range(span.0, span.1),
        children,
        content,
    )
}

fn splice(
    owner: ChildListOwner,
    current: &ChildList,
    start: usize,
    delete_count: usize,
    insert: Vec<NodeId>,
) -> ProjectionOp {
    let mut children = current.as_slice().to_vec();
    children.splice(start..start + delete_count, insert.iter().copied());
    let replacement = ChildList::new(children);
    ProjectionOp::SpliceChildren {
        owner,
        expected_version: current.version().clone(),
        start: u32::try_from(start).unwrap(),
        delete_count: u32::try_from(delete_count).unwrap(),
        insert,
        new_version: replacement.version().clone(),
    }
}

fn append_splice(owner: ChildListOwner, current: &ChildList, insert: Vec<NodeId>) -> ProjectionOp {
    ProjectionOp::SpliceChildren {
        owner,
        expected_version: current.version().clone(),
        start: u32::try_from(current.len()).unwrap(),
        delete_count: 0,
        new_version: current.version_after_append(&insert),
        insert,
    }
}

fn advance_projection(expected: u64, new: u64) -> ProjectionOp {
    ProjectionOp::AdvanceProjection {
        expected_cursor: SourceCursor::new(expected),
        new_cursor: SourceCursor::new(new),
    }
}

fn start(
    epoch: u64,
    source: &str,
    mut operations: Vec<ProjectionOp>,
) -> Result<ChangeSet, ProtocolError> {
    let projects_nodes = operations.iter().any(|operation| {
        matches!(
            operation,
            ProjectionOp::InsertNode { .. } | ProjectionOp::ReplaceNode { .. }
        )
    });
    let advances_projection = operations
        .iter()
        .any(|operation| matches!(operation, ProjectionOp::AdvanceProjection { .. }));
    if !source.is_empty() && projects_nodes && !advances_projection {
        operations.push(advance_projection(0, source.len() as u64));
    }
    ChangeSet::start_epoch(
        Epoch::new(epoch),
        change_id(&format!("epoch:{epoch}")),
        None,
        SourceDelta::append(SourceCursor::new(0), source),
        operations,
    )
}

fn rooted_start(epoch: u64, source: &str, nodes: Vec<ContentNode>) -> ChangeSet {
    let roots = nodes.iter().map(|node| node.id).collect::<Vec<_>>();
    let mut operations = nodes
        .into_iter()
        .map(|node| ProjectionOp::InsertNode { node })
        .collect::<Vec<_>>();
    operations.push(splice(
        ChildListOwner::Document,
        &ChildList::empty(),
        0,
        0,
        roots,
    ));
    if !source.is_empty() {
        operations.push(advance_projection(0, source.len() as u64));
    }
    start(epoch, source, operations).unwrap()
}

fn stable_table_start(epoch: u64, alignments: Vec<mdstream_protocol::TableAlignment>) -> ChangeSet {
    let table = leaf(
        0,
        NodeStability::Stable,
        (0, 0),
        ContentKind::Table {
            alignments: alignments.clone(),
        },
    );
    let head = leaf(1, NodeStability::Stable, (0, 0), ContentKind::TableHead {});
    let body = leaf(2, NodeStability::Stable, (0, 0), ContentKind::TableBody {});
    let row = leaf(3, NodeStability::Stable, (0, 0), ContentKind::TableRow {});
    let cells = alignments
        .iter()
        .enumerate()
        .map(|(column, _)| {
            leaf(
                u64::try_from(column + 4).unwrap(),
                NodeStability::Stable,
                (0, 0),
                ContentKind::TableCell {
                    column: u32::try_from(column).unwrap(),
                },
            )
        })
        .collect::<Vec<_>>();
    let mut operations = vec![
        ProjectionOp::InsertNode {
            node: table.clone(),
        },
        ProjectionOp::InsertNode { node: head.clone() },
        ProjectionOp::InsertNode { node: body.clone() },
        ProjectionOp::InsertNode { node: row.clone() },
    ];
    operations.extend(
        cells
            .iter()
            .cloned()
            .map(|node| ProjectionOp::InsertNode { node }),
    );
    operations.extend([
        append_splice(
            ChildListOwner::Node { node_id: row.id },
            &row.children,
            cells.iter().map(|cell| cell.id).collect(),
        ),
        append_splice(
            ChildListOwner::Node { node_id: head.id },
            &head.children,
            vec![row.id],
        ),
        append_splice(
            ChildListOwner::Node { node_id: table.id },
            &table.children,
            vec![head.id, body.id],
        ),
        append_splice(
            ChildListOwner::Document,
            &ChildList::empty(),
            vec![table.id],
        ),
    ]);
    start(epoch, "", operations).unwrap()
}

fn next_change(
    reducer: &Reducer,
    sequence: u64,
    id: &str,
    suffix: &str,
    mut operations: Vec<ProjectionOp>,
) -> ChangeSet {
    let document = reducer.document().unwrap();
    let projects_nodes = operations.iter().any(|operation| {
        matches!(
            operation,
            ProjectionOp::InsertNode { .. } | ProjectionOp::ReplaceNode { .. }
        )
    });
    let advances_projection = operations
        .iter()
        .any(|operation| matches!(operation, ProjectionOp::AdvanceProjection { .. }));
    if !suffix.is_empty() && projects_nodes && !advances_projection {
        operations.push(ProjectionOp::AdvanceProjection {
            expected_cursor: document.projection_cursor(),
            new_cursor: SourceCursor::new(
                document.coordinate().source_cursor.get() + suffix.len() as u64,
            ),
        });
    }
    ChangeSet::new(
        document.coordinate().epoch,
        Sequence::new(sequence),
        change_id(id),
        SourceDelta::append(document.coordinate().source_cursor, suffix),
        operations,
    )
    .unwrap()
}

fn snapshot_from_value(mut value: serde_json::Value) -> Snapshot {
    let candidate: Snapshot = serde_json::from_value(value.clone()).unwrap();
    value["digest"] = serde_json::to_value(candidate.derived_digest()).unwrap();
    serde_json::from_value(value).unwrap()
}

fn impact(outcome: ApplyOutcome) -> ChangeImpact {
    match outcome {
        ApplyOutcome::Applied { impact, .. } | ApplyOutcome::Recovered { impact, .. } => impact,
        other => panic!("expected a state-changing outcome, got {other:?}"),
    }
}

#[test]
fn bootstrap_owns_one_source_and_explicit_root_order() {
    let mut reducer = Reducer::new();
    let change = rooted_start(
        7,
        "hello world",
        vec![
            leaf(
                4,
                NodeStability::Provisional,
                (0, 5),
                ContentKind::Paragraph {},
            ),
            leaf(9, NodeStability::Stable, (6, 11), ContentKind::Paragraph {}),
        ],
    );

    let outcome = reducer.apply(change).unwrap();
    let impact = impact(outcome);
    let document = reducer.document().unwrap();
    assert_eq!(document.source(), "hello world");
    assert_eq!(document.coordinate().source_cursor, SourceCursor::new(11));
    assert_eq!(
        document.roots().as_slice(),
        &[NodeId::new(4), NodeId::new(9)]
    );
    assert!(impact.roots_changed);
    assert_eq!(impact.changed_nodes, vec![NodeId::new(4), NodeId::new(9)]);
}

#[test]
fn projection_coverage_advances_explicitly_and_source_only_changes_preserve_it() {
    let mut reducer = Reducer::new();
    let outcome = reducer.apply(start(1, "abc", vec![]).unwrap()).unwrap();
    let document = reducer.document().unwrap();
    assert_eq!(document.coordinate().source_cursor, SourceCursor::new(3));
    assert_eq!(document.projection_cursor(), SourceCursor::new(0));
    assert_eq!(document.pending_source(), "abc");
    assert_eq!(document.pending_source_range(), range(0, 3));
    assert!(!impact(outcome).projection_changed);

    let outcome = reducer
        .apply(next_change(&reducer, 1, "source-only", "def", vec![]))
        .unwrap();
    assert_eq!(
        reducer.document().unwrap().projection_cursor(),
        SourceCursor::new(0)
    );
    assert_eq!(reducer.document().unwrap().pending_source(), "abcdef");
    let source_impact = impact(outcome);
    assert!(source_impact.source_changed);
    assert!(!source_impact.projection_changed);

    let outcome = reducer
        .apply(next_change(
            &reducer,
            2,
            "projection:4",
            "",
            vec![advance_projection(0, 4)],
        ))
        .unwrap();
    let document = reducer.document().unwrap();
    assert_eq!(document.coordinate().source_cursor, SourceCursor::new(6));
    assert_eq!(document.projection_cursor(), SourceCursor::new(4));
    assert_eq!(document.pending_source(), "ef");
    let snapshot = document.snapshot();
    assert_eq!(snapshot.pending_source().unwrap(), "ef");
    assert_eq!(snapshot.pending_source_range().unwrap(), range(4, 6));
    let impact = impact(outcome);
    assert!(!impact.source_changed);
    assert!(impact.projection_changed);
}

#[test]
fn projection_coverage_is_monotonic_bounded_and_compare_and_set() {
    fn seeded() -> Reducer {
        let mut reducer = Reducer::new();
        reducer.apply(start(1, "abc", vec![]).unwrap()).unwrap();
        reducer
    }

    let mut stale = seeded();
    let before = stale.document().unwrap().clone();
    let change = next_change(
        &stale,
        1,
        "projection:stale",
        "",
        vec![advance_projection(1, 2)],
    );
    assert!(matches!(
        stale.apply(change).unwrap(),
        ApplyOutcome::RecoveryRequired {
            reason: RecoveryReason::ProjectionDivergence,
            ..
        }
    ));
    assert_eq!(stale.document().unwrap(), &before);

    for (id, operations) in [
        ("projection:no-op", vec![advance_projection(0, 0)]),
        ("projection:beyond-source", vec![advance_projection(0, 4)]),
        (
            "projection:duplicate",
            vec![advance_projection(0, 1), advance_projection(1, 2)],
        ),
    ] {
        let mut reducer = seeded();
        let before = reducer.document().unwrap().clone();
        assert!(matches!(
            reducer.apply(next_change(&reducer, 1, id, "", operations)),
            Err(ProtocolError::InvalidChange(_))
        ));
        assert_eq!(reducer.document().unwrap(), &before);
    }

    let mut backward = seeded();
    backward
        .apply(next_change(
            &backward,
            1,
            "projection:2",
            "",
            vec![advance_projection(0, 2)],
        ))
        .unwrap();
    let before = backward.document().unwrap().clone();
    assert!(matches!(
        backward.apply(next_change(
            &backward,
            2,
            "projection:backward",
            "",
            vec![advance_projection(2, 1)],
        )),
        Err(ProtocolError::InvalidChange(_))
    ));
    assert_eq!(backward.document().unwrap(), &before);
}

#[test]
fn projection_coverage_requires_utf8_boundaries_in_changes_and_snapshots() {
    let mut initial = Reducer::new();
    assert!(matches!(
        initial.apply(start(1, "é", vec![advance_projection(0, 1)]).unwrap()),
        Err(ProtocolError::InvalidChange(_))
    ));
    assert!(initial.document().is_none());

    let mut appended = Reducer::new();
    appended.apply(start(1, "a", vec![]).unwrap()).unwrap();
    let before = appended.document().unwrap().clone();
    assert!(matches!(
        appended.apply(next_change(
            &appended,
            1,
            "projection:utf8-suffix",
            "é",
            vec![advance_projection(0, 2)],
        )),
        Err(ProtocolError::InvalidChange(_))
    ));
    assert_eq!(appended.document().unwrap(), &before);

    let mut producer = Reducer::new();
    producer.apply(start(1, "é", vec![]).unwrap()).unwrap();
    let mut invalid = serde_json::to_value(producer.document().unwrap().snapshot()).unwrap();
    invalid["projection_cursor"] = serde_json::json!("1");
    let invalid = snapshot_from_value(invalid);
    let mut consumer = Reducer::new();
    assert!(matches!(
        consumer.recover_snapshot(invalid),
        Err(ProtocolError::InvalidSnapshot(_))
    ));
    assert!(consumer.document().is_none());
}

#[test]
fn node_ranges_and_finalization_cannot_exceed_projection_coverage() {
    let node = leaf(0, NodeStability::Stable, (0, 3), ContentKind::Paragraph {});
    let mut uncovered = Reducer::new();
    let uncovered_roots = splice(
        ChildListOwner::Document,
        &ChildList::empty(),
        0,
        0,
        vec![node.id],
    );
    assert!(matches!(
        uncovered.apply(
            ChangeSet::start_epoch(
                Epoch::new(1),
                change_id("uncovered"),
                None,
                SourceDelta::append(SourceCursor::new(0), "abc"),
                vec![
                    ProjectionOp::InsertNode { node: node.clone() },
                    uncovered_roots,
                ],
            )
            .unwrap()
        ),
        Err(ProtocolError::InvalidChange(_))
    ));
    assert!(uncovered.document().is_none());

    let roots = splice(
        ChildListOwner::Document,
        &ChildList::empty(),
        0,
        0,
        vec![node.id],
    );
    let mut covered = Reducer::new();
    covered
        .apply(
            start(
                1,
                "abc",
                vec![
                    ProjectionOp::InsertNode { node },
                    roots,
                    advance_projection(0, 3),
                ],
            )
            .unwrap(),
        )
        .unwrap();
    let mut uncovered_snapshot =
        serde_json::to_value(covered.document().unwrap().snapshot()).unwrap();
    uncovered_snapshot["projection_cursor"] = serde_json::json!("2");
    let uncovered_snapshot = snapshot_from_value(uncovered_snapshot);
    let mut snapshot_consumer = Reducer::new();
    assert!(matches!(
        snapshot_consumer.recover_snapshot(uncovered_snapshot),
        Err(ProtocolError::InvalidSnapshot(_))
    ));
    covered
        .apply(next_change(
            &covered,
            1,
            "finish",
            "",
            vec![ProjectionOp::FinishDocument],
        ))
        .unwrap();
    assert_eq!(
        covered.document().unwrap().lifecycle(),
        DocumentLifecycle::Finalized
    );

    let mut unfinished = Reducer::new();
    unfinished.apply(start(1, "abc", vec![]).unwrap()).unwrap();
    let before = unfinished.document().unwrap().clone();
    assert!(matches!(
        unfinished.apply(next_change(
            &unfinished,
            1,
            "finish:uncovered",
            "",
            vec![ProjectionOp::FinishDocument],
        )),
        Err(ProtocolError::IllegalLifecycle(_))
    ));
    assert_eq!(unfinished.document().unwrap(), &before);
}

#[test]
fn snapshot_projection_coverage_is_digest_bound_validated_and_recovered() {
    let mut producer = Reducer::new();
    producer.apply(start(1, "abc", vec![]).unwrap()).unwrap();
    producer
        .apply(next_change(
            &producer,
            1,
            "projection:2",
            "",
            vec![advance_projection(0, 2)],
        ))
        .unwrap();
    let snapshot = producer.document().unwrap().snapshot();
    assert_eq!(snapshot.projection_cursor(), SourceCursor::new(2));
    let original_digest = snapshot.digest().clone();

    let mut tampered = serde_json::to_value(&snapshot).unwrap();
    tampered["projection_cursor"] = serde_json::json!("3");
    let tampered: Snapshot = serde_json::from_value(tampered).unwrap();
    assert_ne!(tampered.derived_digest(), original_digest);

    let mut invalid = serde_json::to_value(&snapshot).unwrap();
    invalid["projection_cursor"] = serde_json::json!("4");
    let invalid = snapshot_from_value(invalid);
    let mut consumer = Reducer::new();
    assert!(matches!(
        consumer.recover_snapshot(invalid),
        Err(ProtocolError::InvalidSnapshot(_))
    ));

    consumer.recover_snapshot(snapshot).unwrap();
    assert_eq!(
        consumer.document().unwrap().projection_cursor(),
        SourceCursor::new(2)
    );

    let gap = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(3),
        change_id("projection:gap"),
        SourceDelta::unchanged(SourceCursor::new(3)),
        vec![ProjectionOp::FinishDocument],
    )
    .unwrap();
    assert!(matches!(
        consumer.apply(gap).unwrap(),
        ApplyOutcome::RecoveryRequired {
            reason: RecoveryReason::SequenceGap { .. },
            ..
        }
    ));
    let mut rollback = serde_json::to_value(producer.document().unwrap().snapshot()).unwrap();
    rollback["coordinate"]["sequence"] = serde_json::json!("2");
    rollback["coordinate"]["change_id"] = serde_json::json!("projection:rollback");
    rollback["projection_cursor"] = serde_json::json!("1");
    let rollback = snapshot_from_value(rollback);
    assert!(matches!(
        consumer.recover_snapshot(rollback),
        Err(ProtocolError::InvalidSnapshot(_))
    ));
}

#[test]
fn canonical_parent_child_grammar_is_atomic_for_changes_and_snapshots() {
    let mut invalid_root_reducer = Reducer::new();
    let invalid_root = leaf(
        0,
        NodeStability::Stable,
        (0, 1),
        ContentKind::Text {
            text: mdstream_protocol::SemanticText::Source {},
        },
    );
    assert!(matches!(
        invalid_root_reducer.apply(rooted_start(1, "x", vec![invalid_root])),
        Err(ProtocolError::InvalidChange(_))
    ));
    assert!(invalid_root_reducer.document().is_none());

    let paragraph = leaf(0, NodeStability::Stable, (0, 1), ContentKind::Paragraph {});
    let text = leaf(
        1,
        NodeStability::Stable,
        (0, 1),
        ContentKind::Text {
            text: mdstream_protocol::SemanticText::Source {},
        },
    );
    let operations = vec![
        ProjectionOp::InsertNode {
            node: paragraph.clone(),
        },
        ProjectionOp::InsertNode { node: text },
        splice(
            ChildListOwner::Node {
                node_id: paragraph.id,
            },
            &paragraph.children,
            0,
            0,
            vec![NodeId::new(1)],
        ),
        splice(
            ChildListOwner::Document,
            &ChildList::empty(),
            0,
            0,
            vec![paragraph.id],
        ),
    ];
    let mut reducer = Reducer::new();
    reducer.apply(start(1, "x", operations).unwrap()).unwrap();
    let before = reducer.document().unwrap().clone();

    let invalid_parent = container(
        0,
        NodeStability::Stable,
        (0, 1),
        vec![NodeId::new(1)],
        ContentKind::BlockQuote {
            style: Default::default(),
        },
    );
    let invalid_change = next_change(
        &reducer,
        1,
        "grammar:invalid-parent",
        "",
        vec![ProjectionOp::ReplaceNode {
            node_id: NodeId::new(0),
            expected_version: paragraph.version,
            projection: invalid_parent.projection(),
        }],
    );
    assert!(matches!(
        reducer.apply(invalid_change),
        Err(ProtocolError::InvalidChange(_))
    ));
    assert_eq!(reducer.document().unwrap(), &before);

    let mut snapshot_value = serde_json::to_value(before.snapshot()).unwrap();
    snapshot_value["nodes"][0]["content"] = serde_json::to_value(&invalid_parent.content).unwrap();
    snapshot_value["nodes"][0]["version"] = serde_json::to_value(&invalid_parent.version).unwrap();
    let mut consumer = Reducer::new();
    assert!(matches!(
        consumer.recover_snapshot(snapshot_from_value(snapshot_value)),
        Err(ProtocolError::InvalidSnapshot(_))
    ));
    assert!(consumer.document().is_none());
}

#[test]
fn paragraph_accepts_display_math_between_text_children() {
    let paragraph = leaf(0, NodeStability::Stable, (0, 0), ContentKind::Paragraph {});
    let left = leaf(
        1,
        NodeStability::Stable,
        (0, 0),
        ContentKind::Text {
            text: mdstream_protocol::SemanticText::Normalized {
                value: "before".to_string(),
            },
        },
    );
    let math = leaf(
        2,
        NodeStability::Stable,
        (0, 0),
        ContentKind::Math {
            display: true,
            text: mdstream_protocol::SemanticText::Normalized {
                value: "x + y".to_string(),
            },
        },
    );
    let right = leaf(
        3,
        NodeStability::Stable,
        (0, 0),
        ContentKind::Text {
            text: mdstream_protocol::SemanticText::Normalized {
                value: "after".to_string(),
            },
        },
    );
    let operations = vec![
        ProjectionOp::InsertNode {
            node: paragraph.clone(),
        },
        ProjectionOp::InsertNode { node: left },
        ProjectionOp::InsertNode { node: math },
        ProjectionOp::InsertNode { node: right },
        splice(
            ChildListOwner::Node {
                node_id: paragraph.id,
            },
            &paragraph.children,
            0,
            0,
            vec![NodeId::new(1), NodeId::new(2), NodeId::new(3)],
        ),
        splice(
            ChildListOwner::Document,
            &ChildList::empty(),
            0,
            0,
            vec![paragraph.id],
        ),
    ];

    let mut reducer = Reducer::new();
    reducer.apply(start(1, "", operations).unwrap()).unwrap();

    let document = reducer.document().unwrap();
    assert_eq!(
        document.node(NodeId::new(0)).unwrap().children.as_slice(),
        &[NodeId::new(1), NodeId::new(2), NodeId::new(3)]
    );
}

#[test]
fn forest_grammar_rejects_nested_links_and_noncanonical_tables() {
    let unresolved_link = || ContentKind::Link {
        target: None,
        reference_label: Some("ref".to_string()),
        style: LinkStyle::ReferenceUnknown,
    };
    let paragraph = leaf(0, NodeStability::Stable, (0, 0), ContentKind::Paragraph {});
    let outer = leaf(1, NodeStability::Stable, (0, 0), unresolved_link());
    let emphasis = leaf(2, NodeStability::Stable, (0, 0), ContentKind::Emphasis {});
    let inner = leaf(3, NodeStability::Stable, (0, 0), unresolved_link());
    let nested_link = start(
        1,
        "",
        vec![
            ProjectionOp::InsertNode {
                node: paragraph.clone(),
            },
            ProjectionOp::InsertNode {
                node: outer.clone(),
            },
            ProjectionOp::InsertNode {
                node: emphasis.clone(),
            },
            ProjectionOp::InsertNode { node: inner },
            splice(
                ChildListOwner::Node {
                    node_id: emphasis.id,
                },
                &emphasis.children,
                0,
                0,
                vec![NodeId::new(3)],
            ),
            splice(
                ChildListOwner::Node { node_id: outer.id },
                &outer.children,
                0,
                0,
                vec![emphasis.id],
            ),
            splice(
                ChildListOwner::Node {
                    node_id: paragraph.id,
                },
                &paragraph.children,
                0,
                0,
                vec![outer.id],
            ),
            splice(
                ChildListOwner::Document,
                &ChildList::empty(),
                0,
                0,
                vec![paragraph.id],
            ),
        ],
    )
    .unwrap();
    let mut reducer = Reducer::new();
    assert!(matches!(
        reducer.apply(nested_link),
        Err(ProtocolError::InvalidChange(_))
    ));
    assert!(reducer.document().is_none());

    let table = leaf(
        0,
        NodeStability::Stable,
        (0, 0),
        ContentKind::Table {
            alignments: vec![mdstream_protocol::TableAlignment::Left],
        },
    );
    let head = leaf(1, NodeStability::Stable, (0, 0), ContentKind::TableHead {});
    let body = leaf(2, NodeStability::Stable, (0, 0), ContentKind::TableBody {});
    let reversed_sections = start(
        1,
        "",
        vec![
            ProjectionOp::InsertNode {
                node: table.clone(),
            },
            ProjectionOp::InsertNode { node: head },
            ProjectionOp::InsertNode { node: body },
            splice(
                ChildListOwner::Node { node_id: table.id },
                &table.children,
                0,
                0,
                vec![NodeId::new(2), NodeId::new(1)],
            ),
            splice(
                ChildListOwner::Document,
                &ChildList::empty(),
                0,
                0,
                vec![table.id],
            ),
        ],
    )
    .unwrap();
    let mut reducer = Reducer::new();
    assert!(matches!(
        reducer.apply(reversed_sections),
        Err(ProtocolError::InvalidChange(_))
    ));

    let table = leaf(
        0,
        NodeStability::Stable,
        (0, 0),
        ContentKind::Table {
            alignments: vec![mdstream_protocol::TableAlignment::Left],
        },
    );
    let body = leaf(1, NodeStability::Stable, (0, 0), ContentKind::TableBody {});
    let row = leaf(2, NodeStability::Stable, (0, 0), ContentKind::TableRow {});
    let cell = leaf(
        3,
        NodeStability::Stable,
        (0, 0),
        ContentKind::TableCell { column: 1 },
    );
    let invalid_column = start(
        1,
        "",
        vec![
            ProjectionOp::InsertNode {
                node: table.clone(),
            },
            ProjectionOp::InsertNode { node: body.clone() },
            ProjectionOp::InsertNode { node: row.clone() },
            ProjectionOp::InsertNode { node: cell },
            splice(
                ChildListOwner::Node { node_id: row.id },
                &row.children,
                0,
                0,
                vec![NodeId::new(3)],
            ),
            splice(
                ChildListOwner::Node { node_id: body.id },
                &body.children,
                0,
                0,
                vec![row.id],
            ),
            splice(
                ChildListOwner::Node { node_id: table.id },
                &table.children,
                0,
                0,
                vec![body.id],
            ),
            splice(
                ChildListOwner::Document,
                &ChildList::empty(),
                0,
                0,
                vec![table.id],
            ),
        ],
    )
    .unwrap();
    let mut reducer = Reducer::new();
    assert!(matches!(
        reducer.apply(invalid_column),
        Err(ProtocolError::InvalidChange(_))
    ));
}

#[test]
fn forest_grammar_enforces_table_completeness_and_internal_roles() {
    let stable_table = leaf(
        0,
        NodeStability::Stable,
        (0, 0),
        ContentKind::Table {
            alignments: vec![mdstream_protocol::TableAlignment::Left],
        },
    );
    let mut reducer = Reducer::new();
    assert!(matches!(
        reducer.apply(rooted_start(1, "", vec![stable_table])),
        Err(ProtocolError::InvalidChange(_))
    ));
    assert!(reducer.document().is_none());

    let custom = leaf(
        0,
        NodeStability::Stable,
        (0, 0),
        ContentKind::Custom {
            namespace: "example.test/1".to_string(),
            name: "container".to_string(),
            opaque: false,
            attributes: BTreeMap::new(),
        },
    );
    let detached_row = leaf(
        1,
        NodeStability::Provisional,
        (0, 0),
        ContentKind::TableRow {},
    );
    let invalid_role = start(
        1,
        "",
        vec![
            ProjectionOp::InsertNode {
                node: custom.clone(),
            },
            ProjectionOp::InsertNode { node: detached_row },
            append_splice(
                ChildListOwner::Node { node_id: custom.id },
                &custom.children,
                vec![NodeId::new(1)],
            ),
            append_splice(
                ChildListOwner::Document,
                &ChildList::empty(),
                vec![custom.id],
            ),
        ],
    )
    .unwrap();
    let mut reducer = Reducer::new();
    assert!(matches!(
        reducer.apply(invalid_role),
        Err(ProtocolError::InvalidChange(_))
    ));

    let table = leaf(
        0,
        NodeStability::Provisional,
        (0, 0),
        ContentKind::Table {
            alignments: vec![
                mdstream_protocol::TableAlignment::Left,
                mdstream_protocol::TableAlignment::Right,
            ],
        },
    );
    let head = leaf(
        1,
        NodeStability::Provisional,
        (0, 0),
        ContentKind::TableHead {},
    );
    let row = leaf(
        2,
        NodeStability::Provisional,
        (0, 0),
        ContentKind::TableRow {},
    );
    let cell = leaf(
        3,
        NodeStability::Stable,
        (0, 0),
        ContentKind::TableCell { column: 0 },
    );
    let mut reducer = Reducer::new();
    reducer
        .apply(
            start(
                1,
                "",
                vec![
                    ProjectionOp::InsertNode {
                        node: table.clone(),
                    },
                    ProjectionOp::InsertNode { node: head.clone() },
                    ProjectionOp::InsertNode { node: row.clone() },
                    ProjectionOp::InsertNode { node: cell },
                    append_splice(
                        ChildListOwner::Node { node_id: row.id },
                        &row.children,
                        vec![NodeId::new(3)],
                    ),
                    append_splice(
                        ChildListOwner::Node { node_id: head.id },
                        &head.children,
                        vec![row.id],
                    ),
                    append_splice(
                        ChildListOwner::Node { node_id: table.id },
                        &table.children,
                        vec![head.id],
                    ),
                    append_splice(
                        ChildListOwner::Document,
                        &ChildList::empty(),
                        vec![table.id],
                    ),
                ],
            )
            .unwrap(),
        )
        .unwrap();
    let before = reducer.document().unwrap().clone();
    let current_row = before.node(row.id).unwrap();
    let stable_row = container(
        2,
        NodeStability::Stable,
        (0, 0),
        current_row.children.as_slice().to_vec(),
        ContentKind::TableRow {},
    );
    assert!(matches!(
        reducer.apply(next_change(
            &reducer,
            1,
            "table:stabilize-incomplete-row",
            "",
            vec![ProjectionOp::StabilizeNode {
                node_id: row.id,
                expected_version: current_row.version.clone(),
                new_version: stable_row.version,
            }],
        )),
        Err(ProtocolError::InvalidChange(_))
    ));
    assert_eq!(reducer.document().unwrap(), &before);
}

#[test]
fn append_fast_path_revalidates_a_changed_sequence_prefix() {
    let table = leaf(
        0,
        NodeStability::Provisional,
        (0, 0),
        ContentKind::Table {
            alignments: vec![
                mdstream_protocol::TableAlignment::Left,
                mdstream_protocol::TableAlignment::Right,
            ],
        },
    );
    let head = leaf(
        1,
        NodeStability::Provisional,
        (0, 0),
        ContentKind::TableHead {},
    );
    let row = leaf(
        2,
        NodeStability::Provisional,
        (0, 0),
        ContentKind::TableRow {},
    );
    let first = leaf(
        3,
        NodeStability::Stable,
        (0, 0),
        ContentKind::TableCell { column: 0 },
    );
    let mut reducer = Reducer::new();
    reducer
        .apply(
            start(
                1,
                "",
                vec![
                    ProjectionOp::InsertNode {
                        node: table.clone(),
                    },
                    ProjectionOp::InsertNode { node: head.clone() },
                    ProjectionOp::InsertNode { node: row.clone() },
                    ProjectionOp::InsertNode {
                        node: first.clone(),
                    },
                    append_splice(
                        ChildListOwner::Node { node_id: row.id },
                        &row.children,
                        vec![first.id],
                    ),
                    append_splice(
                        ChildListOwner::Node { node_id: head.id },
                        &head.children,
                        vec![row.id],
                    ),
                    append_splice(
                        ChildListOwner::Node { node_id: table.id },
                        &table.children,
                        vec![head.id],
                    ),
                    append_splice(
                        ChildListOwner::Document,
                        &ChildList::empty(),
                        vec![table.id],
                    ),
                ],
            )
            .unwrap(),
        )
        .unwrap();

    let before = reducer.document().unwrap().clone();
    let changed_first = leaf(
        3,
        NodeStability::Stable,
        (0, 0),
        ContentKind::TableCell { column: 1 },
    );
    let second = leaf(
        4,
        NodeStability::Stable,
        (0, 0),
        ContentKind::TableCell { column: 1 },
    );
    assert!(matches!(
        reducer.apply(next_change(
            &reducer,
            1,
            "table:dirty-prefix-append",
            "",
            vec![
                ProjectionOp::ReplaceNode {
                    node_id: first.id,
                    expected_version: first.version,
                    projection: changed_first.projection(),
                },
                ProjectionOp::InsertNode { node: second },
                append_splice(
                    ChildListOwner::Node { node_id: row.id },
                    &before.node(row.id).unwrap().children,
                    vec![NodeId::new(4)],
                ),
            ],
        )),
        Err(ProtocolError::InvalidChange(_))
    ));
    assert_eq!(reducer.document().unwrap(), &before);
}

#[test]
fn append_fast_path_rejects_a_removed_prefix_child() {
    let parent = leaf(
        0,
        NodeStability::Stable,
        (0, 0),
        ContentKind::BlockQuote {
            style: Default::default(),
        },
    );
    let child = leaf(1, NodeStability::Stable, (0, 0), ContentKind::Paragraph {});
    let mut reducer = Reducer::new();
    reducer
        .apply(
            start(
                1,
                "",
                vec![
                    ProjectionOp::InsertNode {
                        node: parent.clone(),
                    },
                    ProjectionOp::InsertNode {
                        node: child.clone(),
                    },
                    append_splice(
                        ChildListOwner::Node { node_id: parent.id },
                        &parent.children,
                        vec![child.id],
                    ),
                    append_splice(
                        ChildListOwner::Document,
                        &ChildList::empty(),
                        vec![parent.id],
                    ),
                ],
            )
            .unwrap(),
        )
        .unwrap();
    let before = reducer.document().unwrap().clone();
    let replacement = leaf(2, NodeStability::Stable, (0, 0), ContentKind::Paragraph {});
    assert!(matches!(
        reducer.apply(next_change(
            &reducer,
            1,
            "append:removed-prefix",
            "",
            vec![
                ProjectionOp::RemoveNode {
                    node_id: child.id,
                    expected_version: child.version,
                },
                ProjectionOp::InsertNode { node: replacement },
                append_splice(
                    ChildListOwner::Node { node_id: parent.id },
                    &before.node(parent.id).unwrap().children,
                    vec![NodeId::new(2)],
                ),
            ],
        )),
        Err(ProtocolError::MissingNode(id)) if id == child.id
    ));
    assert_eq!(reducer.document().unwrap(), &before);
}

#[test]
fn nested_links_are_checked_on_replace_move_and_snapshot() {
    let unresolved_link = || ContentKind::Link {
        target: None,
        reference_label: Some("ref".to_string()),
        style: LinkStyle::ReferenceUnknown,
    };

    let paragraph = leaf(0, NodeStability::Stable, (0, 0), ContentKind::Paragraph {});
    let outer = leaf(1, NodeStability::Stable, (0, 0), unresolved_link());
    let emphasis = leaf(2, NodeStability::Stable, (0, 0), ContentKind::Emphasis {});
    let text = leaf(
        3,
        NodeStability::Stable,
        (0, 0),
        ContentKind::Text {
            text: mdstream_protocol::SemanticText::Source {},
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
                        node: outer.clone(),
                    },
                    ProjectionOp::InsertNode {
                        node: emphasis.clone(),
                    },
                    ProjectionOp::InsertNode { node: text.clone() },
                    append_splice(
                        ChildListOwner::Node {
                            node_id: emphasis.id,
                        },
                        &emphasis.children,
                        vec![text.id],
                    ),
                    append_splice(
                        ChildListOwner::Node { node_id: outer.id },
                        &outer.children,
                        vec![emphasis.id],
                    ),
                    append_splice(
                        ChildListOwner::Node {
                            node_id: paragraph.id,
                        },
                        &paragraph.children,
                        vec![outer.id],
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
    let before = reducer.document().unwrap().clone();
    let inner = leaf(3, NodeStability::Stable, (0, 0), unresolved_link());
    assert!(matches!(
        reducer.apply(next_change(
            &reducer,
            1,
            "link:replace-descendant",
            "",
            vec![ProjectionOp::ReplaceNode {
                node_id: text.id,
                expected_version: text.version,
                projection: inner.projection(),
            }],
        )),
        Err(ProtocolError::InvalidChange(_))
    ));
    assert_eq!(reducer.document().unwrap(), &before);

    let mut snapshot = serde_json::to_value(before.snapshot()).unwrap();
    snapshot["nodes"][3] = serde_json::to_value(inner).unwrap();
    let mut consumer = Reducer::new();
    assert!(matches!(
        consumer.recover_snapshot(snapshot_from_value(snapshot)),
        Err(ProtocolError::InvalidSnapshot(_))
    ));

    let paragraph = leaf(0, NodeStability::Stable, (0, 0), ContentKind::Paragraph {});
    let outer = leaf(1, NodeStability::Stable, (0, 0), unresolved_link());
    let emphasis = leaf(2, NodeStability::Stable, (0, 0), ContentKind::Emphasis {});
    let inner = leaf(3, NodeStability::Stable, (0, 0), unresolved_link());
    let mut reducer = Reducer::new();
    reducer
        .apply(
            start(
                2,
                "",
                vec![
                    ProjectionOp::InsertNode {
                        node: paragraph.clone(),
                    },
                    ProjectionOp::InsertNode {
                        node: outer.clone(),
                    },
                    ProjectionOp::InsertNode {
                        node: emphasis.clone(),
                    },
                    ProjectionOp::InsertNode { node: inner },
                    append_splice(
                        ChildListOwner::Node {
                            node_id: emphasis.id,
                        },
                        &emphasis.children,
                        vec![NodeId::new(3)],
                    ),
                    append_splice(
                        ChildListOwner::Node {
                            node_id: paragraph.id,
                        },
                        &paragraph.children,
                        vec![outer.id, emphasis.id],
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
    let before = reducer.document().unwrap().clone();
    assert!(matches!(
        reducer.apply(next_change(
            &reducer,
            1,
            "link:move-subtree",
            "",
            vec![
                splice(
                    ChildListOwner::Node {
                        node_id: paragraph.id,
                    },
                    &before.node(paragraph.id).unwrap().children,
                    1,
                    1,
                    vec![],
                ),
                append_splice(
                    ChildListOwner::Node { node_id: outer.id },
                    &before.node(outer.id).unwrap().children,
                    vec![emphasis.id],
                ),
            ],
        )),
        Err(ProtocolError::InvalidChange(_))
    ));
    assert_eq!(reducer.document().unwrap(), &before);
}

#[test]
fn table_sequence_and_stable_width_hold_across_changes_and_recovery() {
    let mut reducer = Reducer::new();
    reducer
        .apply(stable_table_start(
            1,
            vec![mdstream_protocol::TableAlignment::Left],
        ))
        .unwrap();
    let snapshot = reducer.document().unwrap().snapshot();
    let mut consumer = Reducer::new();
    consumer.recover_snapshot(snapshot.clone()).unwrap();
    assert_eq!(consumer.document().unwrap(), reducer.document().unwrap());
    let mut reversed = serde_json::to_value(snapshot).unwrap();
    reversed["nodes"][0]["children"] =
        serde_json::to_value(ChildList::new(vec![NodeId::new(2), NodeId::new(1)])).unwrap();
    let mut invalid_consumer = Reducer::new();
    assert!(matches!(
        invalid_consumer.recover_snapshot(snapshot_from_value(reversed)),
        Err(ProtocolError::InvalidSnapshot(_))
    ));

    let current = reducer.document().unwrap().node(NodeId::new(0)).unwrap();
    let widened = mdstream_protocol::NodeProjection::new(
        NodeStability::Stable,
        current.source,
        current.body,
        ContentKind::Table {
            alignments: vec![
                mdstream_protocol::TableAlignment::Left,
                mdstream_protocol::TableAlignment::Right,
            ],
        },
    );
    let before = reducer.document().unwrap().clone();
    assert!(matches!(
        reducer.apply(next_change(
            &reducer,
            1,
            "table:widen-stable",
            "",
            vec![ProjectionOp::ReplaceNode {
                node_id: NodeId::new(0),
                expected_version: current.version.clone(),
                projection: widened,
            }],
        )),
        Err(ProtocolError::InvalidChange(_))
    ));
    assert_eq!(reducer.document().unwrap(), &before);

    let current = reducer.document().unwrap().node(NodeId::new(0)).unwrap();
    let realigned = mdstream_protocol::NodeProjection::new(
        NodeStability::Stable,
        current.source,
        current.body,
        ContentKind::Table {
            alignments: vec![mdstream_protocol::TableAlignment::Right],
        },
    );
    reducer
        .apply(next_change(
            &reducer,
            1,
            "table:realign-stable",
            "",
            vec![ProjectionOp::ReplaceNode {
                node_id: NodeId::new(0),
                expected_version: current.version.clone(),
                projection: realigned,
            }],
        ))
        .unwrap();

    let retained = reducer.document().unwrap().clone();
    let mut future = serde_json::to_value(retained.snapshot()).unwrap();
    future["coordinate"]["sequence"] = serde_json::json!("2");
    future["coordinate"]["change_id"] = serde_json::json!("table:future-width");
    future["nodes"][0] = serde_json::to_value(container(
        0,
        NodeStability::Stable,
        (0, 0),
        vec![NodeId::new(1), NodeId::new(2)],
        ContentKind::Table {
            alignments: vec![
                mdstream_protocol::TableAlignment::Right,
                mdstream_protocol::TableAlignment::Left,
            ],
        },
    ))
    .unwrap();
    future["nodes"][3] = serde_json::to_value(container(
        3,
        NodeStability::Stable,
        (0, 0),
        vec![NodeId::new(4), NodeId::new(5)],
        ContentKind::TableRow {},
    ))
    .unwrap();
    future["nodes"].as_array_mut().unwrap().push(
        serde_json::to_value(leaf(
            5,
            NodeStability::Stable,
            (0, 0),
            ContentKind::TableCell { column: 1 },
        ))
        .unwrap(),
    );
    let future = snapshot_from_value(future);
    let mut standalone = Reducer::new();
    standalone.recover_snapshot(future.clone()).unwrap();

    let gap = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(3),
        change_id("table:gap"),
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![ProjectionOp::FinishDocument],
    )
    .unwrap();
    reducer.apply(gap).unwrap();
    assert!(matches!(
        reducer.recover_snapshot(future),
        Err(ProtocolError::InvalidSnapshot(_))
    ));
    assert_eq!(reducer.document().unwrap(), &retained);

    let provisional = leaf(
        0,
        NodeStability::Provisional,
        (0, 0),
        ContentKind::Table {
            alignments: vec![mdstream_protocol::TableAlignment::Left],
        },
    );
    let mut reducer = Reducer::new();
    reducer
        .apply(rooted_start(9, "", vec![provisional.clone()]))
        .unwrap();
    let widened = leaf(
        0,
        NodeStability::Provisional,
        (0, 0),
        ContentKind::Table {
            alignments: vec![
                mdstream_protocol::TableAlignment::Left,
                mdstream_protocol::TableAlignment::Right,
            ],
        },
    );
    reducer
        .apply(next_change(
            &reducer,
            1,
            "table:widen-provisional",
            "",
            vec![ProjectionOp::ReplaceNode {
                node_id: provisional.id,
                expected_version: provisional.version,
                projection: widened.projection(),
            }],
        ))
        .unwrap();
    let mut incomplete = serde_json::to_value(reducer.document().unwrap().snapshot()).unwrap();
    incomplete["nodes"][0] = serde_json::to_value(leaf(
        0,
        NodeStability::Stable,
        (0, 0),
        ContentKind::Table {
            alignments: vec![
                mdstream_protocol::TableAlignment::Left,
                mdstream_protocol::TableAlignment::Right,
            ],
        },
    ))
    .unwrap();
    let mut invalid_consumer = Reducer::new();
    assert!(matches!(
        invalid_consumer.recover_snapshot(snapshot_from_value(incomplete)),
        Err(ProtocolError::InvalidSnapshot(_))
    ));

    let table = leaf(
        0,
        NodeStability::Provisional,
        (0, 0),
        ContentKind::Table {
            alignments: vec![mdstream_protocol::TableAlignment::Left],
        },
    );
    let head = leaf(
        1,
        NodeStability::Provisional,
        (0, 0),
        ContentKind::TableHead {},
    );
    let mut reducer = Reducer::new();
    reducer
        .apply(
            start(
                10,
                "",
                vec![
                    ProjectionOp::InsertNode {
                        node: table.clone(),
                    },
                    ProjectionOp::InsertNode { node: head.clone() },
                    append_splice(
                        ChildListOwner::Node { node_id: table.id },
                        &table.children,
                        vec![head.id],
                    ),
                    append_splice(
                        ChildListOwner::Document,
                        &ChildList::empty(),
                        vec![table.id],
                    ),
                ],
            )
            .unwrap(),
        )
        .unwrap();
    let body = leaf(
        2,
        NodeStability::Provisional,
        (0, 0),
        ContentKind::TableBody {},
    );
    let stable_table = leaf(
        0,
        NodeStability::Stable,
        (0, 0),
        ContentKind::Table {
            alignments: vec![mdstream_protocol::TableAlignment::Left],
        },
    );
    reducer
        .apply(next_change(
            &reducer,
            1,
            "table:complete-and-stabilize",
            "",
            vec![
                ProjectionOp::InsertNode { node: body },
                append_splice(
                    ChildListOwner::Node { node_id: table.id },
                    &reducer.document().unwrap().node(table.id).unwrap().children,
                    vec![NodeId::new(2)],
                ),
                ProjectionOp::StabilizeNode {
                    node_id: table.id,
                    expected_version: table.version,
                    new_version: stable_table.version,
                },
            ],
        ))
        .unwrap();
    assert_eq!(
        reducer
            .document()
            .unwrap()
            .node(table.id)
            .unwrap()
            .stability,
        NodeStability::Stable
    );

    let table = leaf(
        0,
        NodeStability::Provisional,
        (0, 0),
        ContentKind::Table {
            alignments: vec![mdstream_protocol::TableAlignment::Left],
        },
    );
    let empty_stable_head = leaf(1, NodeStability::Stable, (0, 0), ContentKind::TableHead {});
    let mut reducer = Reducer::new();
    assert!(matches!(
        reducer.apply(
            start(
                11,
                "",
                vec![
                    ProjectionOp::InsertNode {
                        node: table.clone(),
                    },
                    ProjectionOp::InsertNode {
                        node: empty_stable_head.clone(),
                    },
                    append_splice(
                        ChildListOwner::Node { node_id: table.id },
                        &table.children,
                        vec![empty_stable_head.id],
                    ),
                    append_splice(
                        ChildListOwner::Document,
                        &ChildList::empty(),
                        vec![table.id],
                    ),
                ],
            )
            .unwrap()
        ),
        Err(ProtocolError::InvalidChange(_))
    ));
}

#[test]
fn sequence_distinguishes_retry_stale_fork_gap_and_recovery_absorption() {
    let mut reducer = Reducer::new();
    let epoch_start = start(1, "a", vec![]).unwrap();
    reducer.apply(epoch_start.clone()).unwrap();
    assert_eq!(
        reducer.apply(epoch_start.clone()).unwrap(),
        ApplyOutcome::Idempotent
    );
    let first = next_change(&reducer, 1, "change:1", "b", vec![]);
    reducer.apply(first.clone()).unwrap();
    assert_eq!(
        reducer.apply(first.clone()).unwrap(),
        ApplyOutcome::Idempotent
    );

    assert!(matches!(
        reducer.apply(epoch_start).unwrap(),
        ApplyOutcome::Stale {
            received_sequence,
            ..
        } if received_sequence == Sequence::new(0)
    ));

    let mut fork_value = serde_json::to_value(first).unwrap();
    fork_value["change_id"] = serde_json::json!("change:fork");
    let fork: ChangeSet = serde_json::from_value(fork_value).unwrap();
    assert!(matches!(
        reducer.apply(fork).unwrap(),
        ApplyOutcome::RecoveryRequired {
            reason: RecoveryReason::SequenceFork { .. },
            ..
        }
    ));
    let before = reducer.document().unwrap().clone();

    let malformed = serde_json::from_value::<ChangeSet>(serde_json::json!({
        "schema": "future",
        "maturity": "draft",
        "epoch": "1",
        "sequence": "2",
        "change_id": "malformed",
        "epoch_start": null,
        "source": {"expected_cursor": "2", "suffix": "c"},
        "operations": []
    }))
    .unwrap();
    assert_eq!(reducer.apply(malformed), Err(ProtocolError::NeedsSnapshot));
    assert_eq!(reducer.document().unwrap(), &before);
}

#[test]
fn producer_apply_rolls_back_noncanonical_routing_state() {
    let mut reducer = Reducer::new();
    reducer.apply(start(1, "a", vec![]).unwrap()).unwrap();
    let before = reducer.document().unwrap().snapshot();
    let metrics = reducer.metrics();
    let gap = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(2),
        change_id("producer:gap"),
        SourceDelta::append(SourceCursor::new(1), "b"),
        vec![],
    )
    .unwrap();

    assert!(matches!(
        reducer.apply_producer_ref(&gap).unwrap(),
        ApplyOutcome::RecoveryRequired {
            reason: RecoveryReason::SequenceGap { .. },
            ..
        }
    ));
    assert_eq!(gap.sequence(), Sequence::new(2));
    assert_eq!(reducer.status(), ReducerStatus::Ready);
    assert_eq!(reducer.document().unwrap().snapshot(), before);
    assert_eq!(reducer.metrics(), metrics);

    let next = next_change(&reducer, 1, "producer:next", "b", vec![]);
    assert!(matches!(
        reducer.apply_producer_ref(&next).unwrap(),
        ApplyOutcome::Applied { .. }
    ));
    assert_eq!(reducer.document().unwrap().source(), "ab");
}

#[test]
fn routing_classification_precedes_projection_validation() {
    fn forged_change(sequence: u64, id: &str, cursor: u64) -> ChangeSet {
        let valid = ChangeSet::new(
            Epoch::new(1),
            Sequence::new(sequence),
            change_id(id),
            SourceDelta::unchanged(SourceCursor::new(cursor)),
            vec![ProjectionOp::InsertNode {
                node: leaf(99, NodeStability::Stable, (0, 0), ContentKind::Paragraph {}),
            }],
        )
        .unwrap();
        let mut value = serde_json::to_value(valid).unwrap();
        value["operations"][0]["node"]["version"] = serde_json::json!("forged");
        serde_json::from_value(value).unwrap()
    }

    let mut stale = Reducer::new();
    stale.apply(start(1, "a", vec![]).unwrap()).unwrap();
    stale
        .apply(next_change(&stale, 1, "one", "b", vec![]))
        .unwrap();
    stale
        .apply(next_change(&stale, 2, "two", "c", vec![]))
        .unwrap();
    let before = stale.document().unwrap().clone();
    assert!(matches!(
        stale.apply(forged_change(1, "old:malformed", 0)).unwrap(),
        ApplyOutcome::Stale { .. }
    ));
    assert_eq!(stale.document().unwrap(), &before);

    let mut fork = Reducer::new();
    fork.apply(start(1, "a", vec![]).unwrap()).unwrap();
    fork.apply(next_change(&fork, 1, "one", "b", vec![]))
        .unwrap();
    assert!(matches!(
        fork.apply(forged_change(1, "fork:malformed", 0)).unwrap(),
        ApplyOutcome::RecoveryRequired {
            reason: RecoveryReason::SequenceFork { .. },
            ..
        }
    ));

    let mut gap = Reducer::new();
    gap.apply(start(1, "a", vec![]).unwrap()).unwrap();
    assert!(matches!(
        gap.apply(forged_change(2, "gap:malformed", 0)).unwrap(),
        ApplyOutcome::RecoveryRequired {
            reason: RecoveryReason::SequenceGap { .. },
            ..
        }
    ));

    let mut next = Reducer::new();
    next.apply(start(1, "a", vec![]).unwrap()).unwrap();
    let before = next.document().unwrap().clone();
    assert_eq!(
        next.apply(forged_change(1, "next:malformed", 1)),
        Err(ProtocolError::VersionMismatch(NodeId::new(99)))
    );
    assert_eq!(next.status(), ReducerStatus::Ready);
    assert_eq!(next.document().unwrap(), &before);
}

#[test]
fn source_structure_and_resource_cas_divergence_have_distinct_recovery_reasons() {
    fn assert_recovery(mut reducer: Reducer, change: ChangeSet, expected: RecoveryReason) {
        let before = reducer.document().unwrap().clone();
        assert!(matches!(
            reducer.apply(change).unwrap(),
            ApplyOutcome::RecoveryRequired { reason, .. } if reason == expected
        ));
        assert_eq!(reducer.document().unwrap(), &before);
        assert!(matches!(
            reducer.status(),
            ReducerStatus::NeedsSnapshot { reason, .. } if reason == expected
        ));
    }

    let mut source = Reducer::new();
    source.apply(start(1, "a", vec![]).unwrap()).unwrap();
    let source_change = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(1),
        change_id("divergence:source"),
        SourceDelta::append(SourceCursor::new(0), "b"),
        vec![],
    )
    .unwrap();
    assert_recovery(source, source_change, RecoveryReason::SourceDivergence);

    let mut structure = Reducer::new();
    structure.apply(start(1, "", vec![]).unwrap()).unwrap();
    let structure_change = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(1),
        change_id("divergence:structure"),
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![ProjectionOp::SpliceChildren {
            owner: ChildListOwner::Document,
            expected_version: mdstream_protocol::StructureVersion::new("stale").unwrap(),
            start: 0,
            delete_count: 0,
            insert: vec![NodeId::new(99)],
            new_version: mdstream_protocol::StructureVersion::new("future").unwrap(),
        }],
    )
    .unwrap();
    assert_recovery(
        structure,
        structure_change,
        RecoveryReason::StructureDivergence,
    );

    let original_resource = SemanticResource::new(
        ResourceId::new(0),
        SemanticResourceKind::Link {
            destination: "https://example.test/old".to_string(),
            title: None,
        },
    );
    let mut resource = Reducer::new();
    resource
        .apply(
            start(
                1,
                "",
                vec![ProjectionOp::InsertResource {
                    resource: original_resource,
                }],
            )
            .unwrap(),
        )
        .unwrap();
    let replacement_resource = SemanticResource::new(
        ResourceId::new(0),
        SemanticResourceKind::Link {
            destination: "https://example.test/new".to_string(),
            title: None,
        },
    );
    let resource_change = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(1),
        change_id("divergence:resource"),
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![ProjectionOp::ReplaceResource {
            resource_id: ResourceId::new(0),
            expected_version: mdstream_protocol::ResourceVersion::new("stale").unwrap(),
            resource: replacement_resource,
        }],
    )
    .unwrap();
    assert_recovery(
        resource,
        resource_change,
        RecoveryReason::ResourceDivergence,
    );
}

#[test]
fn epoch_start_requires_an_exact_predecessor_and_a_future_epoch() {
    let mut reducer = Reducer::new();
    reducer.apply(start(7, "a", vec![]).unwrap()).unwrap();
    reducer
        .apply(next_change(&reducer, 1, "epoch:current", "b", vec![]))
        .unwrap();
    let current = reducer.document().unwrap().coordinate().clone();
    let before = reducer.document().unwrap().clone();

    let mut predecessors = vec![None];
    let mut wrong_epoch = current.clone();
    wrong_epoch.epoch = Epoch::new(6);
    predecessors.push(Some(wrong_epoch));
    let mut wrong_sequence = current.clone();
    wrong_sequence.sequence = Sequence::new(0);
    predecessors.push(Some(wrong_sequence));
    let mut wrong_change = current.clone();
    wrong_change.change_id = change_id("epoch:wrong-change");
    predecessors.push(Some(wrong_change));
    let mut wrong_cursor = current.clone();
    wrong_cursor.source_cursor = SourceCursor::new(1);
    predecessors.push(Some(wrong_cursor));

    for (index, predecessor) in predecessors.into_iter().enumerate() {
        let change = ChangeSet::start_epoch(
            Epoch::new(8),
            change_id(&format!("epoch:invalid:{index}")),
            predecessor,
            SourceDelta::append(SourceCursor::new(0), "new"),
            vec![],
        )
        .unwrap();
        assert!(matches!(
            reducer.apply(change),
            Err(ProtocolError::InvalidEpochStart { .. })
        ));
        assert_eq!(reducer.status(), ReducerStatus::Ready);
        assert_eq!(reducer.document().unwrap(), &before);
    }

    let same_epoch = ChangeSet::start_epoch(
        Epoch::new(7),
        change_id("epoch:same"),
        Some(current.clone()),
        SourceDelta::append(SourceCursor::new(0), "new"),
        vec![],
    )
    .unwrap();
    assert!(matches!(
        reducer.apply(same_epoch).unwrap(),
        ApplyOutcome::Stale {
            received_epoch,
            received_sequence,
            ..
        } if received_epoch == Epoch::new(7) && received_sequence == Sequence::new(0)
    ));
    assert_eq!(reducer.status(), ReducerStatus::Ready);
    assert_eq!(reducer.document().unwrap(), &before);

    let valid = ChangeSet::start_epoch(
        Epoch::new(8),
        change_id("epoch:valid"),
        Some(current),
        SourceDelta::append(SourceCursor::new(0), "new"),
        vec![],
    )
    .unwrap();
    assert!(matches!(
        reducer.apply(valid).unwrap(),
        ApplyOutcome::Recovered { .. }
    ));
    assert_eq!(
        reducer.document().unwrap().coordinate().epoch,
        Epoch::new(8)
    );
    assert_eq!(reducer.document().unwrap().source(), "new");
}

#[test]
fn gap_recovers_from_snapshot_and_continues_at_the_next_sequence() {
    let bootstrap = start(1, "a", vec![]).unwrap();
    let mut producer = Reducer::new();
    let mut consumer = Reducer::new();
    producer.apply(bootstrap.clone()).unwrap();
    consumer.apply(bootstrap).unwrap();
    producer
        .apply(next_change(&producer, 1, "producer:1", "b", vec![]))
        .unwrap();

    let gap = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(2),
        change_id("gap:2"),
        SourceDelta::append(SourceCursor::new(1), "c"),
        vec![],
    )
    .unwrap();
    assert!(matches!(
        consumer.apply(gap).unwrap(),
        ApplyOutcome::RecoveryRequired {
            reason: RecoveryReason::SequenceGap { .. },
            ..
        }
    ));
    consumer
        .recover_snapshot(producer.document().unwrap().snapshot())
        .unwrap();
    consumer
        .apply(next_change(&consumer, 2, "consumer:2", "c", vec![]))
        .unwrap();
    assert_eq!(consumer.document().unwrap().source(), "abc");
}

#[test]
fn epoch_and_snapshot_recovery_routing_is_explicit() {
    let bootstrap = start(1, "a", vec![]).unwrap();
    let mut bootstrap_fork = Reducer::new();
    bootstrap_fork.apply(bootstrap.clone()).unwrap();
    let mut conflicting_start = serde_json::to_value(&bootstrap).unwrap();
    conflicting_start["change_id"] = serde_json::json!("epoch:conflicting-start");
    let conflicting_start: ChangeSet = serde_json::from_value(conflicting_start).unwrap();
    assert!(matches!(
        bootstrap_fork.apply(conflicting_start).unwrap(),
        ApplyOutcome::RecoveryRequired {
            reason: RecoveryReason::SequenceFork { sequence },
            ..
        } if sequence == Sequence::new(0)
    ));

    let mut future_epoch = Reducer::new();
    future_epoch.apply(bootstrap.clone()).unwrap();
    let before = future_epoch.document().unwrap().clone();
    let future = ChangeSet::new(
        Epoch::new(2),
        Sequence::new(1),
        change_id("epoch:unannounced"),
        SourceDelta::unchanged(SourceCursor::new(1)),
        vec![ProjectionOp::FinishDocument],
    )
    .unwrap();
    assert!(matches!(
        future_epoch.apply(future).unwrap(),
        ApplyOutcome::RecoveryRequired {
            reason: RecoveryReason::UnannouncedEpoch { current, received },
            ..
        } if current == Epoch::new(1) && received == Epoch::new(2)
    ));
    assert_eq!(future_epoch.document().unwrap(), &before);
    assert!(matches!(
        future_epoch.status(),
        ReducerStatus::NeedsSnapshot {
            reason: RecoveryReason::UnannouncedEpoch { .. },
            ..
        }
    ));

    let mut below_floor = Reducer::new();
    below_floor.apply(bootstrap).unwrap();
    let old_snapshot = below_floor.document().unwrap().snapshot();
    below_floor
        .apply(next_change(&below_floor, 1, "floor:1", "b", vec![]))
        .unwrap();
    let gap = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(3),
        change_id("floor:gap"),
        SourceDelta::unchanged(SourceCursor::new(2)),
        vec![ProjectionOp::FinishDocument],
    )
    .unwrap();
    below_floor.apply(gap).unwrap();
    let before = below_floor.document().unwrap().clone();
    let status = below_floor.status();
    assert!(matches!(
        below_floor.recover_snapshot(old_snapshot),
        Err(ProtocolError::StaleSnapshot { floor, received })
            if floor == Sequence::new(1) && received == Sequence::new(0)
    ));
    assert_eq!(below_floor.status(), status);
    assert_eq!(below_floor.document().unwrap(), &before);
}

#[test]
fn snapshot_progression_is_monotonic_and_rejections_are_atomic() {
    let mut reducer = Reducer::new();
    reducer
        .apply(rooted_start(
            1,
            "a",
            vec![leaf(
                0,
                NodeStability::Stable,
                (0, 1),
                ContentKind::Paragraph {},
            )],
        ))
        .unwrap();
    let gap = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(2),
        change_id("gap"),
        SourceDelta::append(SourceCursor::new(1), "x"),
        vec![],
    )
    .unwrap();
    reducer.apply(gap).unwrap();
    let before = reducer.document().unwrap().clone();

    let mut same_floor = serde_json::to_value(before.snapshot()).unwrap();
    same_floor["source"] = serde_json::json!("b");
    let forged = snapshot_from_value(same_floor);
    assert!(matches!(
        reducer.recover_snapshot(forged),
        Err(ProtocolError::InvalidSnapshot(_))
    ));
    assert_eq!(reducer.document().unwrap(), &before);

    let mut rollback = serde_json::to_value(before.snapshot()).unwrap();
    rollback["coordinate"]["sequence"] = serde_json::json!("1");
    rollback["coordinate"]["change_id"] = serde_json::json!("forged:1");
    rollback["coordinate"]["source_cursor"] = serde_json::json!("0");
    rollback["source"] = serde_json::json!("");
    rollback["roots"] = serde_json::to_value(ChildList::empty()).unwrap();
    rollback["nodes"] = serde_json::json!([]);
    let forged = snapshot_from_value(rollback);
    assert!(matches!(
        reducer.recover_snapshot(forged),
        Err(ProtocolError::InvalidSnapshot(_))
    ));
    assert_eq!(reducer.document().unwrap(), &before);

    let mut stability = serde_json::to_value(before.snapshot()).unwrap();
    stability["coordinate"]["sequence"] = serde_json::json!("1");
    stability["coordinate"]["change_id"] = serde_json::json!("producer:1");
    let provisional = leaf(
        0,
        NodeStability::Provisional,
        (0, 1),
        ContentKind::Paragraph {},
    );
    stability["nodes"][0] = serde_json::to_value(provisional).unwrap();
    let forged = snapshot_from_value(stability);
    assert!(matches!(
        reducer.recover_snapshot(forged),
        Err(ProtocolError::InvalidSnapshot(_))
    ));
    assert_eq!(reducer.document().unwrap(), &before);
    assert!(matches!(
        reducer.status(),
        ReducerStatus::NeedsSnapshot { .. }
    ));
}

#[test]
fn same_floor_snapshot_recovery_preserves_the_retained_document() {
    let parent = leaf(
        0,
        NodeStability::Stable,
        (0, 0),
        ContentKind::BlockQuote {
            style: Default::default(),
        },
    );
    let child = leaf(1, NodeStability::Stable, (0, 0), ContentKind::Paragraph {});
    let mut reducer = Reducer::new();
    reducer
        .apply(
            start(
                1,
                "",
                vec![
                    ProjectionOp::InsertNode {
                        node: parent.clone(),
                    },
                    ProjectionOp::InsertNode { node: child },
                    append_splice(
                        ChildListOwner::Node { node_id: parent.id },
                        &parent.children,
                        vec![NodeId::new(1)],
                    ),
                    append_splice(
                        ChildListOwner::Document,
                        &ChildList::empty(),
                        vec![parent.id],
                    ),
                ],
            )
            .unwrap(),
        )
        .unwrap();
    let snapshot = reducer.document().unwrap().snapshot();
    let roots_ptr = reducer.document().unwrap().roots().as_slice().as_ptr();
    let children_ptr = reducer
        .document()
        .unwrap()
        .node(parent.id)
        .unwrap()
        .children
        .as_slice()
        .as_ptr();
    let gap = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(2),
        change_id("same-floor:gap"),
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![ProjectionOp::FinishDocument],
    )
    .unwrap();
    assert!(matches!(
        reducer.apply(gap).unwrap(),
        ApplyOutcome::RecoveryRequired { .. }
    ));

    let recovered = reducer.recover_snapshot(snapshot).unwrap();
    let recovered_impact = impact(recovered);
    assert!(recovered_impact.is_empty());
    assert_eq!(reducer.status(), ReducerStatus::Ready);
    assert_eq!(
        reducer.document().unwrap().roots().as_slice().as_ptr(),
        roots_ptr
    );
    assert_eq!(
        reducer
            .document()
            .unwrap()
            .node(parent.id)
            .unwrap()
            .children
            .as_slice()
            .as_ptr(),
        children_ptr
    );
}

#[test]
fn same_epoch_snapshot_cannot_change_resource_identity() {
    let resource = SemanticResource::new(
        ResourceId::new(0),
        SemanticResourceKind::Citation {
            protocol: CitationProtocol::V1,
            key: "paper".to_string(),
            destination: "https://example.test/paper".to_string(),
            title: None,
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
    let mut value = serde_json::to_value(reducer.document().unwrap().snapshot()).unwrap();
    value["coordinate"]["sequence"] = serde_json::json!("1");
    value["coordinate"]["change_id"] = serde_json::json!("snapshot:advanced");
    value["resources"][0] = serde_json::to_value(SemanticResource::new(
        resource.id,
        SemanticResourceKind::Link {
            destination: "https://example.test/link".to_string(),
            title: None,
        },
    ))
    .unwrap();
    let forged = snapshot_from_value(value);

    let gap = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(2),
        change_id("snapshot:gap"),
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![ProjectionOp::FinishDocument],
    )
    .unwrap();
    reducer.apply(gap).unwrap();
    let before = reducer.document().unwrap().clone();
    assert!(matches!(
        reducer.recover_snapshot(forged),
        Err(ProtocolError::InvalidSnapshot(_))
    ));
    assert_eq!(reducer.document().unwrap(), &before);
    assert!(matches!(
        reducer.status(),
        ReducerStatus::NeedsSnapshot { .. }
    ));
}

#[test]
fn snapshot_semantics_reject_utf8_overlap_cycle_and_resource_corruption() {
    let mut utf8_producer = Reducer::new();
    utf8_producer
        .apply(rooted_start(
            1,
            "é",
            vec![leaf(
                0,
                NodeStability::Stable,
                (0, 2),
                ContentKind::Paragraph {},
            )],
        ))
        .unwrap();
    let mut utf8 = serde_json::to_value(utf8_producer.document().unwrap().snapshot()).unwrap();
    utf8["nodes"][0]["source"]["start"] = serde_json::json!("1");
    utf8["nodes"][0]["body"]["start"] = serde_json::json!("1");
    let mut consumer = Reducer::new();
    assert!(matches!(
        consumer.recover_snapshot(snapshot_from_value(utf8)),
        Err(ProtocolError::InvalidRange { .. })
    ));
    assert!(consumer.document().is_none());

    let mut overlap_producer = Reducer::new();
    overlap_producer
        .apply(rooted_start(
            1,
            "abc",
            vec![
                leaf(0, NodeStability::Stable, (0, 2), ContentKind::Paragraph {}),
                leaf(1, NodeStability::Stable, (2, 3), ContentKind::Paragraph {}),
            ],
        ))
        .unwrap();
    let mut overlap =
        serde_json::to_value(overlap_producer.document().unwrap().snapshot()).unwrap();
    overlap["nodes"][1] = serde_json::to_value(leaf(
        1,
        NodeStability::Stable,
        (1, 3),
        ContentKind::Paragraph {},
    ))
    .unwrap();
    let mut consumer = Reducer::new();
    assert!(matches!(
        consumer.recover_snapshot(snapshot_from_value(overlap)),
        Err(ProtocolError::InvalidSnapshot(_))
    ));

    let parent = ContentNode::new(
        NodeId::new(0),
        NodeStability::Stable,
        range(0, 3),
        range(0, 3),
        vec![],
        ContentKind::BlockQuote {
            style: Default::default(),
        },
    );
    let child = leaf(1, NodeStability::Stable, (1, 2), ContentKind::Paragraph {});
    let tree_start = start(
        1,
        "abc",
        vec![
            ProjectionOp::InsertNode {
                node: parent.clone(),
            },
            ProjectionOp::InsertNode { node: child },
            splice(
                ChildListOwner::Node { node_id: parent.id },
                &parent.children,
                0,
                0,
                vec![NodeId::new(1)],
            ),
            splice(
                ChildListOwner::Document,
                &ChildList::empty(),
                0,
                0,
                vec![parent.id],
            ),
        ],
    )
    .unwrap();
    let mut tree_producer = Reducer::new();
    tree_producer.apply(tree_start).unwrap();
    let tree_base = serde_json::to_value(tree_producer.document().unwrap().snapshot()).unwrap();

    let mut duplicate_owner = tree_base.clone();
    duplicate_owner["roots"] =
        serde_json::to_value(ChildList::new(vec![NodeId::new(0), NodeId::new(1)])).unwrap();
    let mut consumer = Reducer::new();
    assert!(matches!(
        consumer.recover_snapshot(snapshot_from_value(duplicate_owner)),
        Err(ProtocolError::InvalidSnapshot(_))
    ));

    let mut outside_body = tree_base;
    outside_body["nodes"][0] = serde_json::to_value(ContentNode::new(
        NodeId::new(0),
        NodeStability::Stable,
        range(0, 3),
        range(2, 3),
        vec![NodeId::new(1)],
        ContentKind::BlockQuote {
            style: Default::default(),
        },
    ))
    .unwrap();
    let mut consumer = Reducer::new();
    assert!(matches!(
        consumer.recover_snapshot(snapshot_from_value(outside_body)),
        Err(ProtocolError::InvalidSnapshot(_))
    ));

    let outer = leaf(
        0,
        NodeStability::Stable,
        (0, 0),
        ContentKind::BlockQuote {
            style: Default::default(),
        },
    );
    let inner = leaf(
        1,
        NodeStability::Stable,
        (0, 0),
        ContentKind::BlockQuote {
            style: Default::default(),
        },
    );
    let cycle_start = start(
        1,
        "",
        vec![
            ProjectionOp::InsertNode {
                node: outer.clone(),
            },
            ProjectionOp::InsertNode {
                node: inner.clone(),
            },
            splice(
                ChildListOwner::Node { node_id: outer.id },
                &outer.children,
                0,
                0,
                vec![inner.id],
            ),
            splice(
                ChildListOwner::Document,
                &ChildList::empty(),
                0,
                0,
                vec![outer.id],
            ),
        ],
    )
    .unwrap();
    let mut cycle_producer = Reducer::new();
    cycle_producer.apply(cycle_start).unwrap();
    let mut cycle = serde_json::to_value(cycle_producer.document().unwrap().snapshot()).unwrap();
    cycle["roots"] = serde_json::to_value(ChildList::empty()).unwrap();
    cycle["nodes"][1]["children"] = serde_json::to_value(ChildList::new(vec![outer.id])).unwrap();
    let mut consumer = Reducer::new();
    assert!(matches!(
        consumer.recover_snapshot(snapshot_from_value(cycle)),
        Err(ProtocolError::InvalidSnapshot(_))
    ));

    let resource = SemanticResource::new(
        ResourceId::new(0),
        SemanticResourceKind::Citation {
            protocol: CitationProtocol::V1,
            key: "paper".to_string(),
            destination: "https://example.test/paper".to_string(),
            title: None,
        },
    );
    let paragraph = leaf(0, NodeStability::Stable, (0, 0), ContentKind::Paragraph {});
    let reference = leaf(
        1,
        NodeStability::Stable,
        (0, 0),
        ContentKind::CitationReference {
            key: "paper".to_string(),
            target: Some(resource.reference()),
        },
    );
    let resource_start = start(
        1,
        "",
        vec![
            ProjectionOp::InsertResource { resource },
            ProjectionOp::InsertNode {
                node: paragraph.clone(),
            },
            ProjectionOp::InsertNode { node: reference },
            splice(
                ChildListOwner::Node {
                    node_id: paragraph.id,
                },
                &paragraph.children,
                0,
                0,
                vec![NodeId::new(1)],
            ),
            splice(
                ChildListOwner::Document,
                &ChildList::empty(),
                0,
                0,
                vec![paragraph.id],
            ),
        ],
    )
    .unwrap();
    let mut resource_producer = Reducer::new();
    resource_producer.apply(resource_start).unwrap();
    let resource_base =
        serde_json::to_value(resource_producer.document().unwrap().snapshot()).unwrap();
    let mut corrupted_resource = resource_base.clone();
    corrupted_resource["nodes"][1] = serde_json::to_value(leaf(
        1,
        NodeStability::Stable,
        (0, 0),
        ContentKind::CitationReference {
            key: "paper".to_string(),
            target: Some(mdstream_protocol::ResourceRef {
                id: ResourceId::new(0),
                version: mdstream_protocol::ResourceVersion::new("stale").unwrap(),
            }),
        },
    ))
    .unwrap();

    let mut missing_resource = resource_base.clone();
    missing_resource["resources"] = serde_json::json!([]);

    let mut duplicate_resource = resource_base.clone();
    duplicate_resource["resources"]
        .as_array_mut()
        .unwrap()
        .push(resource_base["resources"][0].clone());

    let incompatible = SemanticResource::new(
        ResourceId::new(0),
        SemanticResourceKind::Link {
            destination: "https://example.test/not-a-citation".to_string(),
            title: None,
        },
    );
    let mut wrong_resource_kind = resource_base;
    wrong_resource_kind["resources"][0] = serde_json::to_value(&incompatible).unwrap();
    wrong_resource_kind["nodes"][1] = serde_json::to_value(leaf(
        1,
        NodeStability::Stable,
        (0, 0),
        ContentKind::CitationReference {
            key: "paper".to_string(),
            target: Some(incompatible.reference()),
        },
    ))
    .unwrap();

    for corrupted in [
        corrupted_resource,
        missing_resource,
        duplicate_resource,
        wrong_resource_kind,
    ] {
        let mut consumer = Reducer::new();
        assert!(
            consumer
                .recover_snapshot(snapshot_from_value(corrupted))
                .is_err()
        );
        assert!(consumer.document().is_none());
    }
}

#[test]
fn snapshot_recovery_reports_removed_nodes_and_epoch_reset_is_a_full_replace() {
    let bootstrap = rooted_start(
        1,
        "a",
        vec![leaf(
            0,
            NodeStability::Stable,
            (0, 1),
            ContentKind::Paragraph {},
        )],
    );
    let mut producer = Reducer::new();
    let mut consumer = Reducer::new();
    producer.apply(bootstrap.clone()).unwrap();
    consumer.apply(bootstrap).unwrap();

    let old_root = producer.document().unwrap().roots().clone();
    let version = producer
        .document()
        .unwrap()
        .node(NodeId::new(0))
        .unwrap()
        .version
        .clone();
    producer
        .apply(next_change(
            &producer,
            1,
            "remove:0",
            "",
            vec![
                splice(ChildListOwner::Document, &old_root, 0, 1, vec![]),
                ProjectionOp::RemoveNode {
                    node_id: NodeId::new(0),
                    expected_version: version,
                },
            ],
        ))
        .unwrap();
    let gap = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(2),
        change_id("gap"),
        SourceDelta::unchanged(SourceCursor::new(1)),
        vec![ProjectionOp::FinishDocument],
    )
    .unwrap();
    consumer.apply(gap).unwrap();
    let recovered = impact(
        consumer
            .recover_snapshot(producer.document().unwrap().snapshot())
            .unwrap(),
    );
    assert_eq!(recovered.changed_nodes, vec![NodeId::new(0)]);
    assert_eq!(recovered.removed_nodes, vec![NodeId::new(0)]);
    assert!(recovered.full_replace);

    let predecessor = consumer.document().unwrap().coordinate().clone();
    let reset = ChangeSet::start_epoch(
        Epoch::new(2),
        change_id("epoch:2"),
        Some(predecessor),
        SourceDelta::append(SourceCursor::new(0), "new"),
        vec![],
    )
    .unwrap();
    let reset_impact = impact(consumer.apply(reset).unwrap());
    assert!(reset_impact.full_replace);
    assert!(reset_impact.roots_changed);
    assert_eq!(consumer.document().unwrap().source(), "new");
}

#[test]
fn finalized_is_terminal_even_for_future_same_epoch_sequences() {
    let mut reducer = Reducer::new();
    reducer.apply(start(1, "done", vec![]).unwrap()).unwrap();
    let finish = next_change(
        &reducer,
        1,
        "finish",
        "",
        vec![advance_projection(0, 4), ProjectionOp::FinishDocument],
    );
    reducer.apply(finish.clone()).unwrap();
    assert_eq!(reducer.apply(finish).unwrap(), ApplyOutcome::Idempotent);
    let before = reducer.document().unwrap().clone();

    let future = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(3),
        change_id("future"),
        SourceDelta::append(SourceCursor::new(4), "!"),
        vec![],
    )
    .unwrap();
    assert!(matches!(
        reducer.apply(future),
        Err(ProtocolError::IllegalLifecycle(_))
    ));
    assert_eq!(reducer.status(), ReducerStatus::Ready);
    assert_eq!(reducer.document().unwrap(), &before);
}

#[test]
fn change_impact_reports_source_and_lifecycle_transitions() {
    let mut reducer = Reducer::new();
    let bootstrap = impact(reducer.apply(start(1, "a", vec![]).unwrap()).unwrap());
    assert!(bootstrap.source_changed);
    assert!(!bootstrap.lifecycle_changed);

    let appended = impact(
        reducer
            .apply(next_change(&reducer, 1, "source:append", "b", vec![]))
            .unwrap(),
    );
    assert!(appended.source_changed);
    assert!(!appended.lifecycle_changed);

    let finished = impact(
        reducer
            .apply(next_change(
                &reducer,
                2,
                "lifecycle:finish",
                "",
                vec![advance_projection(0, 2), ProjectionOp::FinishDocument],
            ))
            .unwrap(),
    );
    assert!(!finished.source_changed);
    assert!(finished.projection_changed);
    assert!(finished.lifecycle_changed);
    assert!(!finished.is_empty());
}

#[test]
fn finalization_uses_the_provisional_index_instead_of_scanning_all_nodes() {
    let node_count = 10_000usize;
    let limits = ProtocolLimits {
        max_nodes: node_count,
        max_operations: node_count + 1,
        max_children_per_list: node_count,
        ..ProtocolLimits::default()
    };
    let nodes = (0..node_count)
        .map(|id| {
            leaf(
                u64::try_from(id).unwrap(),
                NodeStability::Stable,
                (0, 0),
                ContentKind::Paragraph {},
            )
        })
        .collect();
    let mut reducer = Reducer::with_limits(limits);
    reducer.apply(rooted_start(1, "", nodes)).unwrap();
    assert_eq!(reducer.document().unwrap().provisional_node_count(), 0);
    let baseline = reducer.metrics();
    reducer
        .apply(next_change(
            &reducer,
            1,
            "finish:indexed",
            "",
            vec![ProjectionOp::FinishDocument],
        ))
        .unwrap();
    let metrics = reducer.metrics();
    assert_eq!(metrics.nodes_validated, baseline.nodes_validated);
    assert_eq!(metrics.relationship_steps, baseline.relationship_steps);

    let provisional = leaf(
        0,
        NodeStability::Provisional,
        (0, 0),
        ContentKind::Paragraph {},
    );
    let mut rejected = Reducer::new();
    rejected
        .apply(rooted_start(1, "", vec![provisional]))
        .unwrap();
    assert_eq!(rejected.document().unwrap().provisional_node_count(), 1);
    let before = rejected.document().unwrap().clone();
    assert!(matches!(
        rejected.apply(next_change(
            &rejected,
            1,
            "finish:provisional",
            "",
            vec![ProjectionOp::FinishDocument],
        )),
        Err(ProtocolError::IllegalLifecycle(_))
    ));
    assert_eq!(rejected.document().unwrap(), &before);
}

#[test]
fn structure_splice_reorders_roots_without_changing_node_versions() {
    let first = leaf(4, NodeStability::Stable, (1, 2), ContentKind::Paragraph {});
    let second = leaf(9, NodeStability::Stable, (0, 1), ContentKind::Paragraph {});
    let first_version = first.version.clone();
    let second_version = second.version.clone();
    let mut reducer = Reducer::new();
    reducer
        .apply(rooted_start(1, "ab", vec![second, first]))
        .unwrap();
    assert_eq!(
        reducer.document().unwrap().roots().as_slice(),
        &[NodeId::new(9), NodeId::new(4)]
    );

    assert_eq!(
        reducer
            .document()
            .unwrap()
            .node(NodeId::new(4))
            .unwrap()
            .version,
        first_version
    );
    assert_eq!(
        reducer
            .document()
            .unwrap()
            .node(NodeId::new(9))
            .unwrap()
            .version,
        second_version
    );
}

#[test]
fn child_splice_builds_single_owner_tree_and_identity_can_reappear() {
    let parent = leaf(
        0,
        NodeStability::Stable,
        (0, 2),
        ContentKind::BlockQuote {
            style: Default::default(),
        },
    );
    let child = leaf(1, NodeStability::Stable, (0, 2), ContentKind::Paragraph {});
    let mut operations = vec![
        ProjectionOp::InsertNode {
            node: parent.clone(),
        },
        ProjectionOp::InsertNode { node: child },
        splice(
            ChildListOwner::Node {
                node_id: NodeId::new(0),
            },
            &parent.children,
            0,
            0,
            vec![NodeId::new(1)],
        ),
        splice(
            ChildListOwner::Document,
            &ChildList::empty(),
            0,
            0,
            vec![NodeId::new(0)],
        ),
    ];
    let mut reducer = Reducer::new();
    reducer
        .apply(start(1, "ab", operations.clone()).unwrap())
        .unwrap();
    assert_eq!(
        reducer.document().unwrap().parent(NodeId::new(1)),
        Some(ChildListOwner::Node {
            node_id: NodeId::new(0)
        })
    );

    let roots = reducer.document().unwrap().roots().clone();
    let version = reducer
        .document()
        .unwrap()
        .node(NodeId::new(0))
        .unwrap()
        .version
        .clone();
    reducer
        .apply(next_change(
            &reducer,
            1,
            "remove:tree",
            "",
            vec![
                splice(ChildListOwner::Document, &roots, 0, 1, vec![]),
                ProjectionOp::RemoveNode {
                    node_id: NodeId::new(0),
                    expected_version: version,
                },
            ],
        ))
        .unwrap();
    operations.clear();
    let roots = reducer.document().unwrap().roots().clone();
    let reuse = next_change(
        &reducer,
        2,
        "reuse",
        "",
        vec![
            ProjectionOp::InsertNode {
                node: leaf(1, NodeStability::Stable, (0, 0), ContentKind::Paragraph {}),
            },
            append_splice(ChildListOwner::Document, &roots, vec![NodeId::new(1)]),
        ],
    );
    assert!(matches!(
        reducer.apply(reuse).unwrap(),
        ApplyOutcome::Applied { .. }
    ));
    assert_eq!(
        reducer.document().unwrap().roots().as_slice(),
        &[NodeId::new(1)]
    );
}

#[test]
fn deterministic_ids_apply_independently_of_numeric_discovery_order() {
    let mut reducer = Reducer::new();
    reducer
        .apply(rooted_start(
            1,
            "ab",
            vec![leaf(
                100,
                NodeStability::Provisional,
                (0, 1),
                ContentKind::Paragraph {},
            )],
        ))
        .unwrap();
    let roots = reducer.document().unwrap().roots().clone();
    let lower = leaf(
        1,
        NodeStability::Provisional,
        (1, 2),
        ContentKind::Paragraph {},
    );
    let change = next_change(
        &reducer,
        1,
        "deterministic:lower",
        "",
        vec![
            ProjectionOp::InsertNode {
                node: lower.clone(),
            },
            append_splice(ChildListOwner::Document, &roots, vec![lower.id]),
        ],
    );

    assert!(matches!(
        reducer.apply(change).unwrap(),
        ApplyOutcome::Applied { .. }
    ));
    assert_eq!(
        reducer.document().unwrap().roots().as_slice(),
        &[NodeId::new(100), NodeId::new(1)]
    );
}

#[test]
fn removed_resource_identity_can_reappear() {
    let resource = SemanticResource::new(
        ResourceId::new(7),
        SemanticResourceKind::Link {
            destination: "https://example.test/resource".to_string(),
            title: None,
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
    assert_eq!(
        reducer.apply(next_change(
            &reducer,
            1,
            "resource:duplicate-live",
            "",
            vec![ProjectionOp::InsertResource {
                resource: resource.clone(),
            }],
        )),
        Err(ProtocolError::DuplicateResource(resource.id))
    );
    reducer
        .apply(next_change(
            &reducer,
            1,
            "resource:remove",
            "",
            vec![ProjectionOp::RemoveResource {
                resource_id: resource.id,
                expected_version: resource.version.clone(),
            }],
        ))
        .unwrap();
    assert!(reducer.document().unwrap().resource(resource.id).is_none());

    reducer
        .apply(next_change(
            &reducer,
            2,
            "resource:reappear",
            "",
            vec![ProjectionOp::InsertResource {
                resource: resource.clone(),
            }],
        ))
        .unwrap();
    assert_eq!(
        reducer.document().unwrap().resource(resource.id),
        Some(&resource)
    );
}

#[test]
fn one_change_can_reparent_between_containers_and_then_remove_the_old_owner() {
    let left = leaf(
        0,
        NodeStability::Stable,
        (0, 1),
        ContentKind::BlockQuote {
            style: Default::default(),
        },
    );
    let right = leaf(
        1,
        NodeStability::Stable,
        (1, 2),
        ContentKind::BlockQuote {
            style: Default::default(),
        },
    );
    let child = leaf(2, NodeStability::Stable, (1, 1), ContentKind::Paragraph {});
    let mut reducer = Reducer::new();
    reducer
        .apply(
            start(
                1,
                "ab",
                vec![
                    ProjectionOp::InsertNode { node: left.clone() },
                    ProjectionOp::InsertNode {
                        node: right.clone(),
                    },
                    ProjectionOp::InsertNode { node: child },
                    append_splice(
                        ChildListOwner::Node {
                            node_id: NodeId::new(0),
                        },
                        &left.children,
                        vec![NodeId::new(2)],
                    ),
                    append_splice(
                        ChildListOwner::Document,
                        &ChildList::empty(),
                        vec![NodeId::new(0), NodeId::new(1)],
                    ),
                ],
            )
            .unwrap(),
        )
        .unwrap();

    let left_children = reducer
        .document()
        .unwrap()
        .node(NodeId::new(0))
        .unwrap()
        .children
        .clone();
    let right_children = reducer
        .document()
        .unwrap()
        .node(NodeId::new(1))
        .unwrap()
        .children
        .clone();
    let move_impact = impact(
        reducer
            .apply(next_change(
                &reducer,
                1,
                "move:left-to-right",
                "",
                vec![
                    splice(
                        ChildListOwner::Node {
                            node_id: NodeId::new(0),
                        },
                        &left_children,
                        0,
                        1,
                        vec![],
                    ),
                    append_splice(
                        ChildListOwner::Node {
                            node_id: NodeId::new(1),
                        },
                        &right_children,
                        vec![NodeId::new(2)],
                    ),
                ],
            ))
            .unwrap(),
    );
    assert_eq!(
        reducer.document().unwrap().parent(NodeId::new(2)),
        Some(ChildListOwner::Node {
            node_id: NodeId::new(1)
        })
    );
    assert_eq!(
        move_impact.changed_nodes,
        vec![NodeId::new(0), NodeId::new(1), NodeId::new(2)]
    );

    let roots = reducer.document().unwrap().roots().clone();
    let left_version = reducer
        .document()
        .unwrap()
        .node(NodeId::new(0))
        .unwrap()
        .version
        .clone();
    reducer
        .apply(next_change(
            &reducer,
            2,
            "remove:empty-left",
            "",
            vec![
                splice(ChildListOwner::Document, &roots, 0, 1, vec![]),
                ProjectionOp::RemoveNode {
                    node_id: NodeId::new(0),
                    expected_version: left_version,
                },
            ],
        ))
        .unwrap();
    assert!(reducer.document().unwrap().node(NodeId::new(0)).is_none());
    assert!(reducer.document().unwrap().node(NodeId::new(2)).is_some());
}

#[test]
fn move_child_out_then_splice_and_remove_old_container_does_not_panic() {
    let parent = leaf(
        0,
        NodeStability::Stable,
        (0, 1),
        ContentKind::BlockQuote {
            style: Default::default(),
        },
    );
    let child = leaf(1, NodeStability::Stable, (0, 1), ContentKind::Paragraph {});
    let mut reducer = Reducer::new();
    reducer
        .apply(
            start(
                1,
                "a",
                vec![
                    ProjectionOp::InsertNode {
                        node: parent.clone(),
                    },
                    ProjectionOp::InsertNode { node: child },
                    append_splice(
                        ChildListOwner::Node {
                            node_id: NodeId::new(0),
                        },
                        &parent.children,
                        vec![NodeId::new(1)],
                    ),
                    append_splice(
                        ChildListOwner::Document,
                        &ChildList::empty(),
                        vec![NodeId::new(0)],
                    ),
                ],
            )
            .unwrap(),
        )
        .unwrap();
    let parent = reducer.document().unwrap().node(NodeId::new(0)).unwrap();
    let roots = reducer.document().unwrap().roots().clone();
    let outcome = reducer
        .apply(next_change(
            &reducer,
            1,
            "extract-and-remove",
            "",
            vec![
                splice(
                    ChildListOwner::Node {
                        node_id: NodeId::new(0),
                    },
                    &parent.children,
                    0,
                    1,
                    vec![],
                ),
                splice(ChildListOwner::Document, &roots, 0, 1, vec![NodeId::new(1)]),
                ProjectionOp::RemoveNode {
                    node_id: NodeId::new(0),
                    expected_version: parent.version.clone(),
                },
            ],
        ))
        .unwrap();
    assert_eq!(impact(outcome).removed_nodes, vec![NodeId::new(0)]);
    assert_eq!(
        reducer.document().unwrap().roots().as_slice(),
        &[NodeId::new(1)]
    );
}

#[test]
fn deterministic_versions_reject_forgery_and_return_after_a_b_a() {
    let a = leaf(0, NodeStability::Stable, (0, 1), ContentKind::Paragraph {});
    let b = leaf(
        0,
        NodeStability::Stable,
        (0, 1),
        ContentKind::Heading { level: 1 },
    );
    let a_again = leaf(0, NodeStability::Stable, (0, 1), ContentKind::Paragraph {});
    assert_eq!(a.version, a_again.version);
    assert_ne!(a.version, b.version);

    let mut forged = serde_json::to_value(rooted_start(1, "a", vec![a.clone()])).unwrap();
    forged["operations"][0]["node"]["version"] = serde_json::json!("forged");
    let forged: ChangeSet = serde_json::from_value(forged).unwrap();
    let mut reducer = Reducer::new();
    assert_eq!(
        reducer.apply(forged),
        Err(ProtocolError::VersionMismatch(NodeId::new(0)))
    );
    assert_eq!(reducer.status(), ReducerStatus::Uninitialized);

    reducer
        .apply(rooted_start(1, "a", vec![a.clone()]))
        .unwrap();
    let before = reducer.document().unwrap().clone();
    let mut forged_replace = b.clone();
    forged_replace.version = NodeVersion::new("forged").unwrap();
    let replace = next_change(
        &reducer,
        1,
        "replace:forged",
        "",
        vec![ProjectionOp::ReplaceNode {
            node_id: NodeId::new(0),
            expected_version: a.version,
            projection: forged_replace.projection(),
        }],
    );
    assert_eq!(
        reducer.apply(replace),
        Err(ProtocolError::VersionMismatch(NodeId::new(0)))
    );
    assert_eq!(reducer.document().unwrap(), &before);
}

#[test]
fn stabilization_checks_the_derived_version_and_rolls_back_the_batch() {
    let provisional = leaf(
        0,
        NodeStability::Provisional,
        (0, 1),
        ContentKind::Paragraph {},
    );
    let expected = provisional.version.clone();
    let mut reducer = Reducer::new();
    reducer
        .apply(rooted_start(1, "a", vec![provisional]))
        .unwrap();
    let before = reducer.document().unwrap().clone();
    let change = next_change(
        &reducer,
        1,
        "stabilize:forged",
        "",
        vec![ProjectionOp::StabilizeNode {
            node_id: NodeId::new(0),
            expected_version: expected,
            new_version: NodeVersion::new("forged").unwrap(),
        }],
    );
    assert_eq!(
        reducer.apply(change),
        Err(ProtocolError::VersionMismatch(NodeId::new(0)))
    );
    assert_eq!(reducer.document().unwrap(), &before);

    let mut stable = before.node(NodeId::new(0)).unwrap().clone();
    stable.stability = NodeStability::Stable;
    stable.version = stable.derived_version();
    let outcome = reducer
        .apply(next_change(
            &reducer,
            1,
            "stabilize:valid",
            "",
            vec![ProjectionOp::StabilizeNode {
                node_id: NodeId::new(0),
                expected_version: before.node(NodeId::new(0)).unwrap().version.clone(),
                new_version: stable.version.clone(),
            }],
        ))
        .unwrap();
    assert_eq!(impact(outcome).changed_nodes, vec![NodeId::new(0)]);
    assert_eq!(
        reducer.document().unwrap().node(NodeId::new(0)).unwrap(),
        &stable
    );
}

#[test]
fn append_and_local_replace_share_one_atomic_source_view_and_stale_cas_rolls_back() {
    let original = leaf(
        0,
        NodeStability::Provisional,
        (0, 1),
        ContentKind::Paragraph {},
    );
    let mut reducer = Reducer::new();
    reducer
        .apply(rooted_start(1, "a", vec![original.clone()]))
        .unwrap();
    let replacement = container(
        0,
        NodeStability::Provisional,
        (0, 2),
        original.children.as_slice().to_vec(),
        ContentKind::Paragraph {},
    );
    reducer
        .apply(next_change(
            &reducer,
            1,
            "append-and-replace",
            "b",
            vec![ProjectionOp::ReplaceNode {
                node_id: NodeId::new(0),
                expected_version: original.version,
                projection: replacement.projection(),
            }],
        ))
        .unwrap();
    assert_eq!(reducer.document().unwrap().source(), "ab");
    assert_eq!(
        reducer
            .document()
            .unwrap()
            .node(NodeId::new(0))
            .unwrap()
            .source,
        range(0, 2)
    );

    let before = reducer.document().unwrap().clone();
    let resource = SemanticResource::new(
        ResourceId::new(0),
        SemanticResourceKind::Link {
            destination: "https://example.test".to_string(),
            title: None,
        },
    );
    let stale_cas = next_change(
        &reducer,
        2,
        "stale-cas-after-valid-op",
        "",
        vec![
            ProjectionOp::InsertResource { resource },
            ProjectionOp::ReplaceNode {
                node_id: NodeId::new(0),
                expected_version: NodeVersion::new("stale").unwrap(),
                projection: reducer
                    .document()
                    .unwrap()
                    .node(NodeId::new(0))
                    .unwrap()
                    .projection(),
            },
        ],
    );
    assert!(matches!(
        reducer.apply(stale_cas).unwrap(),
        ApplyOutcome::RecoveryRequired {
            reason: RecoveryReason::VersionDivergence,
            ..
        }
    ));
    assert_eq!(reducer.document().unwrap(), &before);
    assert!(
        reducer
            .document()
            .unwrap()
            .resource(ResourceId::new(0))
            .is_none()
    );
}

#[test]
fn semantic_resources_are_shared_versioned_and_type_checked() {
    let resource = SemanticResource::new(
        ResourceId::new(0),
        SemanticResourceKind::Citation {
            protocol: CitationProtocol::V1,
            key: "paper".to_string(),
            destination: "https://example.test/paper".to_string(),
            title: Some("Paper".to_string()),
        },
    );
    let resource_ref = resource.reference();
    let paragraph = leaf(0, NodeStability::Stable, (0, 4), ContentKind::Paragraph {});
    let reference = leaf(
        1,
        NodeStability::Stable,
        (0, 4),
        ContentKind::CitationReference {
            key: "paper".to_string(),
            target: Some(resource_ref),
        },
    );
    let operations = vec![
        ProjectionOp::InsertResource {
            resource: resource.clone(),
        },
        ProjectionOp::InsertNode {
            node: paragraph.clone(),
        },
        ProjectionOp::InsertNode { node: reference },
        splice(
            ChildListOwner::Node {
                node_id: paragraph.id,
            },
            &paragraph.children,
            0,
            0,
            vec![NodeId::new(1)],
        ),
        splice(
            ChildListOwner::Document,
            &ChildList::empty(),
            0,
            0,
            vec![paragraph.id],
        ),
    ];
    let change = start(1, "cite", operations).unwrap();
    let mut reducer = Reducer::new();
    reducer.apply(change).unwrap();
    assert_eq!(reducer.document().unwrap().resources().len(), 1);

    let corrected = SemanticResource::new(
        ResourceId::new(0),
        SemanticResourceKind::Citation {
            protocol: CitationProtocol::V1,
            key: "paper".to_string(),
            destination: "https://example.test/revised".to_string(),
            title: Some("Revised paper".to_string()),
        },
    );
    let before = reducer.document().unwrap().clone();
    let resource_only = next_change(
        &reducer,
        1,
        "resource:stale-users",
        "",
        vec![ProjectionOp::ReplaceResource {
            resource_id: ResourceId::new(0),
            expected_version: resource.version.clone(),
            resource: corrected.clone(),
        }],
    );
    let outcome = reducer.apply(resource_only).unwrap();
    let resource_only_impact = impact(outcome);
    assert_eq!(
        resource_only_impact.changed_resources,
        vec![ResourceId::new(0)]
    );
    assert!(resource_only_impact.changed_nodes.contains(&NodeId::new(1)));
    let rebound = reducer.document().unwrap().node(NodeId::new(1)).unwrap();
    assert_ne!(
        rebound.version,
        before.node(NodeId::new(1)).unwrap().version
    );
    assert_eq!(
        rebound.content.resource_ref().unwrap().version,
        corrected.version
    );

    let current_reference = reducer.document().unwrap().node(NodeId::new(1)).unwrap();
    let corrected_again = SemanticResource::new(
        ResourceId::new(0),
        SemanticResourceKind::Citation {
            protocol: CitationProtocol::V1,
            key: "paper".to_string(),
            destination: "https://example.test/final".to_string(),
            title: Some("Final paper".to_string()),
        },
    );
    let corrected_reference = leaf(
        1,
        NodeStability::Stable,
        (0, 4),
        ContentKind::CitationReference {
            key: "paper".to_string(),
            target: Some(corrected_again.reference()),
        },
    );
    let outcome = reducer
        .apply(next_change(
            &reducer,
            2,
            "resource:replace-with-users",
            "",
            vec![
                ProjectionOp::ReplaceResource {
                    resource_id: ResourceId::new(0),
                    expected_version: corrected.version,
                    resource: corrected_again.clone(),
                },
                ProjectionOp::ReplaceNode {
                    node_id: NodeId::new(1),
                    expected_version: current_reference.version.clone(),
                    projection: corrected_reference.projection(),
                },
            ],
        ))
        .unwrap();
    let impact = impact(outcome);
    assert_eq!(impact.changed_resources, vec![ResourceId::new(0)]);
    assert!(impact.changed_nodes.contains(&NodeId::new(1)));
    assert_eq!(
        reducer
            .document()
            .unwrap()
            .resource(ResourceId::new(0))
            .unwrap(),
        &corrected_again
    );
    assert_eq!(
        reducer
            .document()
            .unwrap()
            .node(NodeId::new(1))
            .unwrap()
            .version,
        corrected_reference.version
    );

    let link = SemanticResource::new(
        ResourceId::new(0),
        SemanticResourceKind::Link {
            destination: "https://other.test".to_string(),
            title: None,
        },
    );
    let change = next_change(
        &reducer,
        3,
        "resource:wrong-kind",
        "",
        vec![ProjectionOp::ReplaceResource {
            resource_id: ResourceId::new(0),
            expected_version: corrected_again.version,
            resource: link,
        }],
    );
    assert!(matches!(
        reducer.apply(change),
        Err(ProtocolError::InvalidChange(_))
    ));
}

#[test]
fn resource_replacement_bulk_rebinds_fanout_beyond_the_operation_limit() {
    let user_count = 10_000u64;
    let resource = SemanticResource::new(
        ResourceId::new(0),
        SemanticResourceKind::Citation {
            protocol: CitationProtocol::V1,
            key: "shared".to_string(),
            destination: "https://example.test/original".to_string(),
            title: None,
        },
    );
    let paragraph = leaf(0, NodeStability::Stable, (0, 0), ContentKind::Paragraph {});
    let make_reference = |id| {
        leaf(
            id,
            NodeStability::Stable,
            (0, 0),
            ContentKind::CitationReference {
                key: "shared".to_string(),
                target: Some(resource.reference()),
            },
        )
    };
    let first_ids = (1..=user_count / 2)
        .map(|id| NodeId::new(u128::from(id)))
        .collect::<Vec<_>>();
    let mut first = vec![
        ProjectionOp::InsertResource {
            resource: resource.clone(),
        },
        ProjectionOp::InsertNode {
            node: paragraph.clone(),
        },
    ];
    first.extend(
        (1..=user_count / 2)
            .map(make_reference)
            .map(|node| ProjectionOp::InsertNode { node }),
    );
    first.push(splice(
        ChildListOwner::Node {
            node_id: paragraph.id,
        },
        &paragraph.children,
        0,
        0,
        first_ids,
    ));
    first.push(splice(
        ChildListOwner::Document,
        &ChildList::empty(),
        0,
        0,
        vec![paragraph.id],
    ));

    let mut reducer = Reducer::new();
    reducer.apply(start(1, "", first).unwrap()).unwrap();
    let second_ids = (user_count / 2 + 1..=user_count)
        .map(|id| NodeId::new(u128::from(id)))
        .collect::<Vec<_>>();
    let mut second = (user_count / 2 + 1..=user_count)
        .map(make_reference)
        .map(|node| ProjectionOp::InsertNode { node })
        .collect::<Vec<_>>();
    second.push(append_splice(
        ChildListOwner::Node {
            node_id: paragraph.id,
        },
        &reducer
            .document()
            .unwrap()
            .node(paragraph.id)
            .unwrap()
            .children,
        second_ids,
    ));
    reducer
        .apply(next_change(&reducer, 1, "fanout:second-half", "", second))
        .unwrap();

    let replacement = SemanticResource::new(
        ResourceId::new(0),
        SemanticResourceKind::Citation {
            protocol: CitationProtocol::V1,
            key: "shared".to_string(),
            destination: "https://example.test/replacement".to_string(),
            title: None,
        },
    );
    let change = next_change(
        &reducer,
        2,
        "fanout:replace-resource",
        "",
        vec![ProjectionOp::ReplaceResource {
            resource_id: ResourceId::new(0),
            expected_version: resource.version,
            resource: replacement.clone(),
        }],
    );
    let encoded = encode_change_json(&change, usize::MAX, ProtocolLimits::default()).unwrap();
    let baseline = reducer.metrics();
    let outcome = reducer.apply(change).unwrap();
    let changed = impact(outcome);
    assert!(encoded.len() < 2_048);
    assert_eq!(
        reducer.metrics().operations_visited - baseline.operations_visited,
        1
    );
    assert_eq!(
        changed.changed_nodes.len(),
        usize::try_from(user_count).unwrap()
    );
    assert_eq!(changed.changed_resources, vec![ResourceId::new(0)]);
    for id in [1, user_count] {
        assert_eq!(
            reducer
                .document()
                .unwrap()
                .node(NodeId::new(u128::from(id)))
                .unwrap()
                .content
                .resource_ref()
                .unwrap()
                .version,
            replacement.version
        );
    }
}

#[test]
fn resource_replacement_composes_with_stabilization_and_preserves_identity() {
    let resource = SemanticResource::new(
        ResourceId::new(0),
        SemanticResourceKind::Citation {
            protocol: CitationProtocol::V1,
            key: "paper".to_string(),
            destination: "https://example.test/original".to_string(),
            title: None,
        },
    );
    let paragraph = leaf(0, NodeStability::Stable, (0, 0), ContentKind::Paragraph {});
    let reference = leaf(
        1,
        NodeStability::Provisional,
        (0, 0),
        ContentKind::CitationReference {
            key: "paper".to_string(),
            target: Some(resource.reference()),
        },
    );
    let mut reducer = Reducer::new();
    reducer
        .apply(
            start(
                1,
                "",
                vec![
                    ProjectionOp::InsertResource {
                        resource: resource.clone(),
                    },
                    ProjectionOp::InsertNode {
                        node: paragraph.clone(),
                    },
                    ProjectionOp::InsertNode {
                        node: reference.clone(),
                    },
                    append_splice(
                        ChildListOwner::Node {
                            node_id: paragraph.id,
                        },
                        &paragraph.children,
                        vec![reference.id],
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

    let replacement = SemanticResource::new(
        ResourceId::new(0),
        SemanticResourceKind::Citation {
            protocol: CitationProtocol::V1,
            key: "paper".to_string(),
            destination: "https://example.test/revised".to_string(),
            title: Some("Revised".to_string()),
        },
    );
    let expected = leaf(
        1,
        NodeStability::Stable,
        (0, 0),
        ContentKind::CitationReference {
            key: "paper".to_string(),
            target: Some(replacement.reference()),
        },
    );
    let wrong_final = leaf(
        1,
        NodeStability::Stable,
        (0, 0),
        ContentKind::CitationReference {
            key: "paper".to_string(),
            target: Some(resource.reference()),
        },
    );
    let before = reducer.document().unwrap().clone();
    assert_eq!(
        reducer.apply(next_change(
            &reducer,
            1,
            "resource:wrong-final-version",
            "",
            vec![
                ProjectionOp::ReplaceResource {
                    resource_id: resource.id,
                    expected_version: resource.version.clone(),
                    resource: replacement.clone(),
                },
                ProjectionOp::StabilizeNode {
                    node_id: reference.id,
                    expected_version: reference.version.clone(),
                    new_version: wrong_final.version,
                },
            ],
        )),
        Err(ProtocolError::VersionMismatch(reference.id))
    );
    assert_eq!(reducer.document().unwrap(), &before);

    reducer
        .apply(next_change(
            &reducer,
            1,
            "resource:replace-and-stabilize",
            "",
            vec![
                ProjectionOp::ReplaceResource {
                    resource_id: resource.id,
                    expected_version: resource.version.clone(),
                    resource: replacement.clone(),
                },
                ProjectionOp::StabilizeNode {
                    node_id: reference.id,
                    expected_version: reference.version,
                    new_version: expected.version.clone(),
                },
            ],
        ))
        .unwrap();
    let rebound = reducer.document().unwrap().node(reference.id).unwrap();
    assert_eq!(rebound.version, expected.version);
    assert_eq!(rebound.stability, NodeStability::Stable);
    assert_eq!(
        rebound.content.resource_ref().unwrap().version,
        replacement.version
    );

    let renamed = SemanticResource::new(
        resource.id,
        SemanticResourceKind::Citation {
            protocol: CitationProtocol::V1,
            key: "renamed".to_string(),
            destination: "https://example.test/revised".to_string(),
            title: None,
        },
    );
    let before = reducer.document().unwrap().clone();
    assert!(matches!(
        reducer.apply(next_change(
            &reducer,
            2,
            "resource:rename-identity",
            "",
            vec![ProjectionOp::ReplaceResource {
                resource_id: replacement.id,
                expected_version: replacement.version,
                resource: renamed,
            }],
        )),
        Err(ProtocolError::InvalidChange(_))
    ));
    assert_eq!(reducer.document().unwrap(), &before);

    let unreferenced = SemanticResource::new(
        ResourceId::new(0),
        SemanticResourceKind::Citation {
            protocol: CitationProtocol::V1,
            key: "identity".to_string(),
            destination: "https://example.test/citation".to_string(),
            title: None,
        },
    );
    let mut reducer = Reducer::new();
    reducer
        .apply(
            start(
                9,
                "",
                vec![ProjectionOp::InsertResource {
                    resource: unreferenced.clone(),
                }],
            )
            .unwrap(),
        )
        .unwrap();
    let renamed_unreferenced = SemanticResource::new(
        unreferenced.id,
        SemanticResourceKind::Citation {
            protocol: CitationProtocol::V1,
            key: "renamed".to_string(),
            destination: "https://example.test/citation".to_string(),
            title: None,
        },
    );
    assert!(matches!(
        reducer.apply(next_change(
            &reducer,
            1,
            "resource:rename-unreferenced",
            "",
            vec![ProjectionOp::ReplaceResource {
                resource_id: unreferenced.id,
                expected_version: unreferenced.version.clone(),
                resource: renamed_unreferenced,
            }],
        )),
        Err(ProtocolError::InvalidChange(_))
    ));
    let cross_kind = SemanticResource::new(
        unreferenced.id,
        SemanticResourceKind::Link {
            destination: "https://example.test/link".to_string(),
            title: None,
        },
    );
    assert!(matches!(
        reducer.apply(next_change(
            &reducer,
            1,
            "resource:cross-kind",
            "",
            vec![ProjectionOp::ReplaceResource {
                resource_id: unreferenced.id,
                expected_version: unreferenced.version,
                resource: cross_kind,
            }],
        )),
        Err(ProtocolError::InvalidChange(_))
    ));
}

#[test]
fn aggregate_metadata_budget_counts_shared_resources_once() {
    let limits = ProtocolLimits {
        max_metadata_value_bytes: 16,
        max_node_metadata_bytes: 16,
        max_change_metadata_bytes: 64,
        max_document_metadata_bytes: 8,
        ..ProtocolLimits::default()
    };
    let resource = SemanticResource::new(
        ResourceId::new(0),
        SemanticResourceKind::Link {
            destination: "12345678".to_string(),
            title: None,
        },
    );
    let resource_ref = resource.reference();
    let paragraph = leaf(0, NodeStability::Stable, (0, 0), ContentKind::Paragraph {});
    let link = leaf(
        1,
        NodeStability::Stable,
        (0, 0),
        ContentKind::Link {
            target: Some(resource_ref),
            reference_label: None,
            style: LinkStyle::Inline,
        },
    );
    let operations = vec![
        ProjectionOp::InsertResource { resource },
        ProjectionOp::InsertNode {
            node: paragraph.clone(),
        },
        ProjectionOp::InsertNode { node: link },
        splice(
            ChildListOwner::Node {
                node_id: paragraph.id,
            },
            &paragraph.children,
            0,
            0,
            vec![NodeId::new(1)],
        ),
        splice(
            ChildListOwner::Document,
            &ChildList::empty(),
            0,
            0,
            vec![paragraph.id],
        ),
    ];
    let mut reducer = Reducer::with_limits(limits);
    reducer.apply(start(1, "", operations).unwrap()).unwrap();
    assert_eq!(reducer.document().unwrap().metadata_bytes(), 8);

    let too_large = SemanticResource::new(
        ResourceId::new(1),
        SemanticResourceKind::Link {
            destination: "x".to_string(),
            title: None,
        },
    );
    let before = reducer.document().unwrap().clone();
    let change = next_change(
        &reducer,
        1,
        "metadata:plus1",
        "",
        vec![ProjectionOp::InsertResource {
            resource: too_large,
        }],
    );
    assert!(matches!(
        reducer.apply(change),
        Err(ProtocolError::ValueTooLarge {
            field: "document.metadata",
            ..
        })
    ));
    assert_eq!(reducer.document().unwrap(), &before);
}

#[test]
fn reducer_limits_accept_boundaries_and_reject_first_excess_atomically() {
    let source_limits = ProtocolLimits {
        max_source_bytes: 2,
        ..ProtocolLimits::default()
    };
    let mut source = Reducer::with_limits(source_limits);
    source.apply(start(1, "x", vec![]).unwrap()).unwrap();
    source
        .apply(next_change(&source, 1, "limit:source:at", "y", vec![]))
        .unwrap();
    let before = source.document().unwrap().clone();
    assert!(matches!(
        source.apply(next_change(&source, 2, "limit:source:over", "z", vec![],)),
        Err(ProtocolError::SourceTooLarge {
            limit: 2,
            actual: 3,
        })
    ));
    assert_eq!(source.status(), ReducerStatus::Ready);
    assert_eq!(source.document().unwrap(), &before);

    let node_limits = ProtocolLimits {
        max_nodes: 1,
        ..ProtocolLimits::default()
    };
    let mut one_node = Reducer::with_limits(node_limits);
    one_node
        .apply(rooted_start(
            1,
            "",
            vec![leaf(
                0,
                NodeStability::Stable,
                (0, 0),
                ContentKind::Paragraph {},
            )],
        ))
        .unwrap();
    let mut two_nodes = Reducer::with_limits(node_limits);
    assert!(matches!(
        two_nodes.apply(rooted_start(
            1,
            "",
            vec![
                leaf(0, NodeStability::Stable, (0, 0), ContentKind::Paragraph {},),
                leaf(1, NodeStability::Stable, (0, 0), ContentKind::Paragraph {},),
            ],
        )),
        Err(ProtocolError::TooManyNodes {
            limit: 1,
            actual: 2,
        })
    ));
    assert!(two_nodes.document().is_none());

    let resource_limits = ProtocolLimits {
        max_resources: 1,
        ..ProtocolLimits::default()
    };
    let make_resource = |id, suffix: &str| {
        SemanticResource::new(
            ResourceId::new(id),
            SemanticResourceKind::Link {
                destination: format!("https://example.test/{suffix}"),
                title: None,
            },
        )
    };
    let mut one_resource = Reducer::with_limits(resource_limits);
    one_resource
        .apply(
            start(
                1,
                "",
                vec![ProjectionOp::InsertResource {
                    resource: make_resource(0, "one"),
                }],
            )
            .unwrap(),
        )
        .unwrap();
    let mut two_resources = Reducer::with_limits(resource_limits);
    assert!(matches!(
        two_resources.apply(
            start(
                1,
                "",
                vec![
                    ProjectionOp::InsertResource {
                        resource: make_resource(0, "one"),
                    },
                    ProjectionOp::InsertResource {
                        resource: make_resource(1, "two"),
                    },
                ],
            )
            .unwrap(),
        ),
        Err(ProtocolError::ValueTooLarge {
            field: "document.resources",
            limit: 1,
            actual: 2,
        })
    ));
    assert!(two_resources.document().is_none());

    let parent = leaf(
        0,
        NodeStability::Stable,
        (0, 0),
        ContentKind::BlockQuote {
            style: Default::default(),
        },
    );
    let child = leaf(1, NodeStability::Stable, (0, 0), ContentKind::Paragraph {});
    let tree_change = start(
        1,
        "",
        vec![
            ProjectionOp::InsertNode {
                node: parent.clone(),
            },
            ProjectionOp::InsertNode { node: child },
            splice(
                ChildListOwner::Node { node_id: parent.id },
                &parent.children,
                0,
                0,
                vec![NodeId::new(1)],
            ),
            splice(
                ChildListOwner::Document,
                &ChildList::empty(),
                0,
                0,
                vec![parent.id],
            ),
        ],
    )
    .unwrap();
    let exact_tree_limits = ProtocolLimits {
        max_children_per_list: 1,
        max_tree_depth: 2,
        ..ProtocolLimits::default()
    };
    let mut exact_tree = Reducer::with_limits(exact_tree_limits);
    exact_tree.apply(tree_change.clone()).unwrap();
    let shallow_limits = ProtocolLimits {
        max_tree_depth: 1,
        ..exact_tree_limits
    };
    let mut shallow = Reducer::with_limits(shallow_limits);
    assert!(matches!(
        shallow.apply(tree_change),
        Err(ProtocolError::ValueTooLarge {
            field: "tree.depth",
            limit: 1,
            actual: 2,
        })
    ));
    assert!(shallow.document().is_none());
}

#[test]
fn deep_and_wide_snapshot_validation_is_linear_and_depth_bounded() {
    let depth = 128u64;
    let mut operations = (0..depth)
        .map(|id| ProjectionOp::InsertNode {
            node: leaf(
                id,
                NodeStability::Stable,
                (0, 0),
                if id + 1 == depth {
                    ContentKind::Paragraph {}
                } else {
                    ContentKind::BlockQuote {
                        style: Default::default(),
                    }
                },
            ),
        })
        .collect::<Vec<_>>();
    for id in 0..depth - 1 {
        operations.push(splice(
            ChildListOwner::Node {
                node_id: NodeId::new(u128::from(id)),
            },
            &ChildList::empty(),
            0,
            0,
            vec![NodeId::new(u128::from(id + 1))],
        ));
    }
    operations.push(splice(
        ChildListOwner::Document,
        &ChildList::empty(),
        0,
        0,
        vec![NodeId::new(0)],
    ));
    let mut producer = Reducer::new();
    producer.apply(start(1, "", operations).unwrap()).unwrap();
    let snapshot = producer.document().unwrap().snapshot();

    let mut consumer = Reducer::new();
    consumer.recover_snapshot(snapshot.clone()).unwrap();
    assert!(consumer.metrics().relationship_steps <= depth * 4 + 4);

    let low_depth = ProtocolLimits {
        max_tree_depth: 64,
        ..ProtocolLimits::default()
    };
    let mut bounded = Reducer::with_limits(low_depth);
    assert!(matches!(
        bounded.recover_snapshot(snapshot),
        Err(ProtocolError::ValueTooLarge {
            field: "tree.depth",
            ..
        })
    ));

    let width = 512u64;
    let parent = leaf(
        0,
        NodeStability::Stable,
        (0, 0),
        ContentKind::BlockQuote {
            style: Default::default(),
        },
    );
    let mut wide = vec![ProjectionOp::InsertNode {
        node: parent.clone(),
    }];
    wide.extend((1..=width).map(|id| ProjectionOp::InsertNode {
        node: leaf(id, NodeStability::Stable, (0, 0), ContentKind::Paragraph {}),
    }));
    wide.push(splice(
        ChildListOwner::Node {
            node_id: NodeId::new(0),
        },
        &parent.children,
        0,
        0,
        (1..=width).map(|id| NodeId::new(u128::from(id))).collect(),
    ));
    wide.push(splice(
        ChildListOwner::Document,
        &ChildList::empty(),
        0,
        0,
        vec![NodeId::new(0)],
    ));
    let mut producer = Reducer::new();
    producer.apply(start(2, "", wide).unwrap()).unwrap();
    let mut consumer = Reducer::new();
    consumer
        .recover_snapshot(producer.document().unwrap().snapshot())
        .unwrap();
    assert!(consumer.metrics().relationship_steps <= (width + 1) * 4 + 4);
}

#[test]
fn moving_a_subtree_revalidates_every_descendant_depth() {
    let limits = ProtocolLimits {
        max_tree_depth: 2,
        ..ProtocolLimits::default()
    };
    let ancestor = leaf(
        0,
        NodeStability::Stable,
        (1, 1),
        ContentKind::BlockQuote {
            style: Default::default(),
        },
    );
    let descendant = leaf(1, NodeStability::Stable, (1, 1), ContentKind::Paragraph {});
    let new_parent = leaf(
        2,
        NodeStability::Stable,
        (1, 2),
        ContentKind::BlockQuote {
            style: Default::default(),
        },
    );
    let mut reducer = Reducer::with_limits(limits);
    reducer
        .apply(
            start(
                1,
                "ab",
                vec![
                    ProjectionOp::InsertNode {
                        node: ancestor.clone(),
                    },
                    ProjectionOp::InsertNode { node: descendant },
                    ProjectionOp::InsertNode {
                        node: new_parent.clone(),
                    },
                    append_splice(
                        ChildListOwner::Node {
                            node_id: NodeId::new(0),
                        },
                        &ancestor.children,
                        vec![NodeId::new(1)],
                    ),
                    append_splice(
                        ChildListOwner::Document,
                        &ChildList::empty(),
                        vec![NodeId::new(0), NodeId::new(2)],
                    ),
                ],
            )
            .unwrap(),
        )
        .unwrap();
    let before = reducer.document().unwrap().clone();
    let roots = before.roots().clone();
    let change = next_change(
        &reducer,
        1,
        "move:too-deep",
        "",
        vec![
            splice(ChildListOwner::Document, &roots, 0, 1, vec![]),
            append_splice(
                ChildListOwner::Node {
                    node_id: NodeId::new(2),
                },
                &new_parent.children,
                vec![NodeId::new(0)],
            ),
        ],
    );
    assert!(matches!(
        reducer.apply(change),
        Err(ProtocolError::ValueTooLarge {
            field: "tree.depth",
            limit: 2,
            actual: 3
        })
    ));
    assert_eq!(reducer.document().unwrap(), &before);
}

#[test]
fn streaming_root_and_child_appends_have_linear_reducer_and_wire_work() {
    fn run(count: u64, nested: bool) -> (u64, u64, usize) {
        let mut reducer = Reducer::new();
        if nested {
            reducer
                .apply(rooted_start(
                    1,
                    "",
                    vec![leaf(
                        0,
                        NodeStability::Stable,
                        (0, 0),
                        ContentKind::BlockQuote {
                            style: Default::default(),
                        },
                    )],
                ))
                .unwrap();
        } else {
            reducer.apply(start(1, "", vec![]).unwrap()).unwrap();
        }
        let baseline = reducer.metrics();
        let mut encoded_bytes = 0usize;
        for offset in 0..count {
            let id = if nested { offset + 1 } else { offset };
            let owner = if nested {
                ChildListOwner::Node {
                    node_id: NodeId::new(0),
                }
            } else {
                ChildListOwner::Document
            };
            let current = match owner {
                ChildListOwner::Document => reducer.document().unwrap().roots(),
                ChildListOwner::Node { node_id } => {
                    &reducer.document().unwrap().node(node_id).unwrap().children
                }
            };
            let change = next_change(
                &reducer,
                offset + 1,
                &format!("append:{offset}"),
                "",
                vec![
                    ProjectionOp::InsertNode {
                        node: leaf(id, NodeStability::Stable, (0, 0), ContentKind::Paragraph {}),
                    },
                    append_splice(owner, current, vec![NodeId::new(u128::from(id))]),
                ],
            );
            encoded_bytes = encoded_bytes.saturating_add(
                encode_change_json(&change, usize::MAX, ProtocolLimits::default())
                    .unwrap()
                    .len(),
            );
            let changed = impact(reducer.apply(change).unwrap());
            assert!(changed.changed_nodes.len() <= 2);
        }
        let metrics = reducer.metrics();
        (
            metrics.relationship_steps - baseline.relationship_steps,
            metrics.child_ids_copied - baseline.child_ids_copied,
            encoded_bytes,
        )
    }

    for nested in [false, true] {
        let small = run(256, nested);
        let large = run(512, nested);
        assert!(
            large.0 <= small.0 * 2 + 16,
            "relationship work: {small:?} -> {large:?}"
        );
        assert!(
            large.1 <= small.1 * 2 + 16,
            "copy work: {small:?} -> {large:?}"
        );
        assert!(
            large.2 <= small.2 * 2 + 8_192,
            "wire bytes: {small:?} -> {large:?}"
        );
    }
}

#[test]
fn indexed_local_replacements_do_not_rescan_the_whole_root_list() {
    let nodes = (0..1_000)
        .map(|id| leaf(id, NodeStability::Stable, (0, 0), ContentKind::Paragraph {}))
        .collect::<Vec<_>>();
    let mut reducer = Reducer::new();
    reducer.apply(rooted_start(1, "", nodes)).unwrap();
    let baseline = reducer.metrics();

    for sequence in 1..=100 {
        let id = sequence - 1;
        let current = reducer
            .document()
            .unwrap()
            .node(NodeId::new(u128::from(id)))
            .unwrap();
        let replacement = container(
            id,
            NodeStability::Stable,
            (0, 0),
            current.children.as_slice().to_vec(),
            ContentKind::Heading { level: 1 },
        );
        reducer
            .apply(next_change(
                &reducer,
                sequence,
                &format!("replace:{sequence}"),
                "",
                vec![ProjectionOp::ReplaceNode {
                    node_id: NodeId::new(u128::from(id)),
                    expected_version: current.version.clone(),
                    projection: replacement.projection(),
                }],
            ))
            .unwrap();
    }

    let metrics = reducer.metrics();
    assert_eq!(
        metrics.operations_visited,
        baseline.operations_visited + 100
    );
    assert!(metrics.nodes_validated <= baseline.nodes_validated + 100);
    assert!(metrics.relationship_steps <= baseline.relationship_steps + 500);
}

#[test]
fn local_projection_replace_wire_and_copy_work_ignore_large_child_lists() {
    fn replace_parent(child_count: usize) -> (usize, u64, u64) {
        let limits = ProtocolLimits {
            max_nodes: child_count + 1,
            max_operations: child_count + 3,
            max_children_per_list: child_count,
            ..ProtocolLimits::default()
        };
        let parent = leaf(
            0,
            NodeStability::Stable,
            (0, 0),
            ContentKind::BlockQuote {
                style: Default::default(),
            },
        );
        let mut operations = vec![ProjectionOp::InsertNode {
            node: parent.clone(),
        }];
        operations.extend((1..=child_count).map(|id| ProjectionOp::InsertNode {
            node: leaf(
                u64::try_from(id).unwrap(),
                NodeStability::Stable,
                (0, 0),
                ContentKind::Paragraph {},
            ),
        }));
        operations.push(splice(
            ChildListOwner::Node { node_id: parent.id },
            &parent.children,
            0,
            0,
            (1..=child_count)
                .map(|id| NodeId::new(u128::try_from(id).unwrap()))
                .collect(),
        ));
        operations.push(splice(
            ChildListOwner::Document,
            &ChildList::empty(),
            0,
            0,
            vec![parent.id],
        ));

        let mut reducer = Reducer::with_limits(limits);
        reducer.apply(start(1, "", operations).unwrap()).unwrap();
        let baseline = reducer.metrics();
        let current = reducer.document().unwrap().node(parent.id).unwrap();
        let replacement = mdstream_protocol::NodeProjection::new(
            NodeStability::Stable,
            current.source,
            current.body,
            ContentKind::BlockQuote {
                style: mdstream_protocol::BlockQuoteKind::Note,
            },
        );
        let change = next_change(
            &reducer,
            1,
            "replace:local-projection",
            "",
            vec![ProjectionOp::ReplaceNode {
                node_id: parent.id,
                expected_version: current.version.clone(),
                projection: replacement,
            }],
        );
        let encoded = encode_change_json(&change, usize::MAX, limits).unwrap();
        reducer.apply(change).unwrap();

        (
            encoded.len(),
            reducer.metrics().child_ids_copied - baseline.child_ids_copied,
            reducer.metrics().relationship_steps - baseline.relationship_steps,
        )
    }

    let small = replace_parent(1);
    let large = replace_parent(10_000);
    assert_eq!(large.0, small.0);
    assert_eq!(small.1, 0);
    assert_eq!(large.1, 0);
    assert!(
        large.2 <= small.2 + 8,
        "local projection relationship work grew with child count: {small:?} -> {large:?}"
    );
}

#[test]
fn retained_snapshots_do_not_defer_topology_copies_into_later_appends() {
    let parent = leaf(
        0,
        NodeStability::Stable,
        (0, 0),
        ContentKind::BlockQuote {
            style: Default::default(),
        },
    );
    let child = leaf(1, NodeStability::Stable, (0, 0), ContentKind::Paragraph {});
    let bootstrap = start(
        1,
        "",
        vec![
            ProjectionOp::InsertNode {
                node: parent.clone(),
            },
            ProjectionOp::InsertNode { node: child },
            splice(
                ChildListOwner::Node { node_id: parent.id },
                &parent.children,
                0,
                0,
                vec![NodeId::new(1)],
            ),
            splice(
                ChildListOwner::Document,
                &ChildList::empty(),
                0,
                0,
                vec![parent.id],
            ),
        ],
    )
    .unwrap();
    let mut producer = Reducer::new();
    producer.apply(bootstrap).unwrap();

    let roots_clone = producer.document().unwrap().roots().clone();
    assert_ne!(
        producer.document().unwrap().roots().as_slice().as_ptr(),
        roots_clone.as_slice().as_ptr(),
    );
    let node_clone = producer
        .document()
        .unwrap()
        .node(NodeId::new(0))
        .unwrap()
        .clone();
    assert_ne!(
        producer
            .document()
            .unwrap()
            .node(NodeId::new(0))
            .unwrap()
            .children
            .as_slice()
            .as_ptr(),
        node_clone.children.as_slice().as_ptr(),
    );
    let reducer_clone = producer.clone();
    assert_ne!(
        producer.document().unwrap().roots().as_slice().as_ptr(),
        reducer_clone
            .document()
            .unwrap()
            .roots()
            .as_slice()
            .as_ptr(),
    );

    let retained = producer.document().unwrap().snapshot();
    assert_ne!(
        producer.document().unwrap().roots().as_slice().as_ptr(),
        retained.roots().as_slice().as_ptr(),
    );
    assert_ne!(
        producer
            .document()
            .unwrap()
            .node(NodeId::new(0))
            .unwrap()
            .children
            .as_slice()
            .as_ptr(),
        retained.nodes()[0].children.as_slice().as_ptr(),
    );

    let retained_clone = retained.clone();
    let mut consumer = Reducer::new();
    consumer.recover_snapshot(retained_clone).unwrap();
    assert_ne!(
        consumer.document().unwrap().roots().as_slice().as_ptr(),
        retained.roots().as_slice().as_ptr(),
    );
    assert_ne!(
        consumer
            .document()
            .unwrap()
            .node(NodeId::new(0))
            .unwrap()
            .children
            .as_slice()
            .as_ptr(),
        retained.nodes()[0].children.as_slice().as_ptr(),
    );

    let next = leaf(2, NodeStability::Stable, (0, 0), ContentKind::Paragraph {});
    let roots = consumer.document().unwrap().roots().clone();
    let baseline = consumer.metrics();
    consumer
        .apply(next_change(
            &consumer,
            1,
            "append:after-retained-snapshot",
            "",
            vec![
                ProjectionOp::InsertNode { node: next },
                append_splice(ChildListOwner::Document, &roots, vec![NodeId::new(2)]),
            ],
        ))
        .unwrap();
    assert_eq!(
        consumer.metrics().child_ids_copied - baseline.child_ids_copied,
        1
    );
    assert_eq!(retained.roots().as_slice(), &[NodeId::new(0)]);
}

#[test]
fn custom_attributes_use_deterministic_map_order_for_versions() {
    let mut left = BTreeMap::new();
    left.insert("b".to_string(), "2".to_string());
    left.insert("a".to_string(), "1".to_string());
    let mut right = BTreeMap::new();
    right.insert("a".to_string(), "1".to_string());
    right.insert("b".to_string(), "2".to_string());
    let left = leaf(
        0,
        NodeStability::Stable,
        (0, 0),
        ContentKind::Custom {
            namespace: "example.test/1".to_string(),
            name: "aside".to_string(),
            opaque: false,
            attributes: left,
        },
    );
    let right = leaf(
        99,
        NodeStability::Stable,
        (0, 0),
        ContentKind::Custom {
            namespace: "example.test/1".to_string(),
            name: "aside".to_string(),
            opaque: false,
            attributes: right,
        },
    );
    assert_eq!(left.version, right.version);
}

#[test]
fn processor_input_version_includes_child_structure() {
    let left = container(
        0,
        NodeStability::Stable,
        (0, 0),
        vec![NodeId::new(1)],
        ContentKind::BlockQuote {
            style: Default::default(),
        },
    );
    let right = container(
        0,
        NodeStability::Stable,
        (0, 0),
        vec![NodeId::new(2)],
        ContentKind::BlockQuote {
            style: Default::default(),
        },
    );

    assert_eq!(left.version, right.version);
    assert_ne!(
        left.processor_input_version(),
        right.processor_input_version()
    );
}

#[test]
fn processor_input_context_version_and_cost_include_body_and_resource() {
    let node = leaf(
        0,
        NodeStability::Stable,
        (0, 0),
        ContentKind::CodeBlock {
            syntax: mdstream_protocol::CodeBlockSyntax::Fenced {
                marker: mdstream_protocol::CodeFenceMarker::Backtick,
                length: 3,
            },
            info: Some("text".to_string()),
            text: mdstream_protocol::SemanticText::Source {},
        },
    );
    let resource = SemanticResource::new(
        ResourceId::new(9),
        SemanticResourceKind::Link {
            destination: "https://example.test/resource".to_string(),
            title: Some("Resource".to_string()),
        },
    );

    assert_ne!(
        node.processor_input_version_with_context("a", None),
        node.processor_input_version_with_context("b", None)
    );
    assert_ne!(
        node.processor_input_version_with_context("a", None),
        node.processor_input_version_with_context("a", Some(&resource))
    );
    let mut mutated = node.clone();
    let ContentKind::CodeBlock { info, .. } = &mut mutated.content else {
        unreachable!()
    };
    *info = Some("mutated".to_string());
    assert_eq!(node.version, mutated.version);
    assert_ne!(
        node.processor_input_version_with_context("a", None),
        mutated.processor_input_version_with_context("a", None)
    );
    let base_bytes = node
        .checked_processor_input_byte_len_with_context("", None)
        .unwrap();
    assert_eq!(
        node.checked_processor_input_byte_len_with_context("abc", None),
        Some(base_bytes + 3)
    );
    assert!(
        node.checked_processor_input_byte_len_with_context("", Some(&resource))
            .unwrap()
            > base_bytes
    );
}

#[test]
fn normalized_semantic_text_counts_toward_metadata_limits() {
    let limits = ProtocolLimits {
        max_metadata_value_bytes: 3,
        max_node_metadata_bytes: 3,
        ..ProtocolLimits::default()
    };
    let fixtures = [
        ContentKind::Text {
            text: mdstream_protocol::SemanticText::Normalized {
                value: "four".to_string(),
            },
        },
        ContentKind::Html {
            block: true,
            text: mdstream_protocol::SemanticText::Normalized {
                value: "four".to_string(),
            },
        },
    ];

    for content in fixtures {
        let node = leaf(0, NodeStability::Stable, (0, 0), content);
        let mut reducer = Reducer::with_limits(limits);
        assert!(matches!(
            reducer.apply(rooted_start(1, "", vec![node])),
            Err(ProtocolError::ValueTooLarge {
                field: "semantic_text.value",
                limit: 3,
                actual: 4,
            })
        ));
    }
}

#[test]
fn aggregate_structural_limits_preflight_changes_and_snapshots() {
    let limits = ProtocolLimits {
        max_nodes: 3,
        max_operations: 4,
        max_change_structural_items: 3,
        max_children_per_list: 4,
        ..ProtocolLimits::default()
    };
    let empty = ChildList::empty();
    let oversized = ChangeSet::start_epoch(
        Epoch::new(1),
        change_id("structural:aggregate"),
        None,
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![
            append_splice(
                ChildListOwner::Node {
                    node_id: NodeId::new(10),
                },
                &empty,
                vec![NodeId::new(0), NodeId::new(1), NodeId::new(2)],
            ),
            append_splice(
                ChildListOwner::Node {
                    node_id: NodeId::new(11),
                },
                &empty,
                vec![NodeId::new(3), NodeId::new(4), NodeId::new(5)],
            ),
        ],
    )
    .unwrap();
    assert!(matches!(
        encode_change_json(&oversized, usize::MAX, limits),
        Err(ProtocolError::ValueTooLarge {
            field: "change.structural_items",
            limit: 3,
            actual: 6,
        })
    ));

    let table_heavy = start(
        1,
        "",
        vec![ProjectionOp::InsertNode {
            node: leaf(
                0,
                NodeStability::Provisional,
                (0, 0),
                ContentKind::Table {
                    alignments: vec![
                        mdstream_protocol::TableAlignment::Left,
                        mdstream_protocol::TableAlignment::Center,
                        mdstream_protocol::TableAlignment::Right,
                        mdstream_protocol::TableAlignment::None,
                    ],
                },
            ),
        }],
    )
    .unwrap();
    assert!(matches!(
        encode_change_json(&table_heavy, usize::MAX, limits),
        Err(ProtocolError::ValueTooLarge {
            field: "change.structural_items",
            limit: 3,
            actual: 4,
        })
    ));

    let live_limits = ProtocolLimits {
        max_change_structural_items: 4,
        max_document_structural_items: 5,
        max_children_per_list: 4,
        ..ProtocolLimits::default()
    };
    let first_table = leaf(
        0,
        NodeStability::Provisional,
        (0, 0),
        ContentKind::Table {
            alignments: vec![
                mdstream_protocol::TableAlignment::Left,
                mdstream_protocol::TableAlignment::Center,
                mdstream_protocol::TableAlignment::Right,
            ],
        },
    );
    let mut live = Reducer::with_limits(live_limits);
    live.apply(rooted_start(7, "", vec![first_table])).unwrap();
    assert_eq!(live.document().unwrap().structural_items(), 4);
    encode_snapshot_json(
        &live.document().unwrap().snapshot(),
        usize::MAX,
        live_limits,
    )
    .unwrap();
    let before = live.document().unwrap().clone();
    let second_table = leaf(
        1,
        NodeStability::Provisional,
        (0, 0),
        ContentKind::Table {
            alignments: vec![mdstream_protocol::TableAlignment::Left],
        },
    );
    assert!(matches!(
        live.apply(next_change(
            &live,
            1,
            "structural:document-cumulative",
            "",
            vec![
                ProjectionOp::InsertNode { node: second_table },
                append_splice(
                    ChildListOwner::Document,
                    before.roots(),
                    vec![NodeId::new(1)],
                ),
            ],
        )),
        Err(ProtocolError::ValueTooLarge {
            field: "document.structural_items",
            limit: 5,
            actual: 6,
        })
    ));
    assert_eq!(live.document().unwrap(), &before);

    let mut producer = Reducer::new();
    producer
        .apply(rooted_start(
            2,
            "",
            vec![
                leaf(0, NodeStability::Stable, (0, 0), ContentKind::Paragraph {}),
                leaf(1, NodeStability::Stable, (0, 0), ContentKind::Paragraph {}),
            ],
        ))
        .unwrap();
    let mut value = serde_json::to_value(producer.document().unwrap().snapshot()).unwrap();
    value["digest"] = serde_json::json!("tampered");
    let snapshot: Snapshot = serde_json::from_value(value).unwrap();
    let mut consumer = Reducer::with_limits(ProtocolLimits {
        max_nodes: 1,
        ..ProtocolLimits::default()
    });
    assert!(matches!(
        consumer.recover_snapshot(snapshot),
        Err(ProtocolError::TooManyNodes {
            limit: 1,
            actual: 2,
        })
    ));

    let table = leaf(
        0,
        NodeStability::Provisional,
        (0, 0),
        ContentKind::Table {
            alignments: vec![
                mdstream_protocol::TableAlignment::Left,
                mdstream_protocol::TableAlignment::Center,
                mdstream_protocol::TableAlignment::Right,
                mdstream_protocol::TableAlignment::None,
            ],
        },
    );
    let mut table_producer = Reducer::new();
    table_producer
        .apply(rooted_start(3, "", vec![table]))
        .unwrap();
    let mut value = serde_json::to_value(table_producer.document().unwrap().snapshot()).unwrap();
    value["digest"] = serde_json::json!("tampered");
    let snapshot: Snapshot = serde_json::from_value(value).unwrap();
    let mut consumer = Reducer::with_limits(ProtocolLimits {
        max_nodes: 1,
        max_children_per_list: 4,
        max_document_structural_items: 4,
        ..ProtocolLimits::default()
    });
    assert!(matches!(
        consumer.recover_snapshot(snapshot),
        Err(ProtocolError::ValueTooLarge {
            field: "snapshot.structural_items",
            limit: 4,
            actual: 5,
        })
    ));

    let mut value = serde_json::to_value(producer.document().unwrap().snapshot()).unwrap();
    value["digest"] = serde_json::json!("tampered");
    let snapshot: Snapshot = serde_json::from_value(value).unwrap();
    let mut consumer = Reducer::with_limits(ProtocolLimits {
        max_nodes: 2,
        max_children_per_list: 1,
        ..ProtocolLimits::default()
    });
    assert!(matches!(
        consumer.recover_snapshot(snapshot),
        Err(ProtocolError::ValueTooLarge {
            field: "snapshot.roots",
            limit: 1,
            actual: 2,
        })
    ));

    let mut value = serde_json::to_value(producer.document().unwrap().snapshot()).unwrap();
    value["digest"] = serde_json::json!("tampered");
    value["roots"] = serde_json::to_value(ChildList::new(vec![NodeId::new(0)])).unwrap();
    value["nodes"][0] = serde_json::to_value(container(
        0,
        NodeStability::Stable,
        (0, 0),
        vec![NodeId::new(1)],
        ContentKind::BlockQuote {
            style: Default::default(),
        },
    ))
    .unwrap();
    value["nodes"][1] = serde_json::to_value(container(
        1,
        NodeStability::Stable,
        (0, 0),
        vec![NodeId::new(0)],
        ContentKind::BlockQuote {
            style: Default::default(),
        },
    ))
    .unwrap();
    let snapshot: Snapshot = serde_json::from_value(value).unwrap();
    let mut consumer = Reducer::with_limits(ProtocolLimits {
        max_nodes: 2,
        max_children_per_list: 2,
        ..ProtocolLimits::default()
    });
    assert!(matches!(
        consumer.recover_snapshot(snapshot),
        Err(ProtocolError::ValueTooLarge {
            field: "snapshot.attachments",
            limit: 2,
            actual: 3,
        })
    ));
}
