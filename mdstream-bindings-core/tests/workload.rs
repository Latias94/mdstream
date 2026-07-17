use mdstream_bindings_core::{BindingPayloadKind, ReducerSession};
use mdstream_protocol::{
    ChangeId, ChangeSet, ChildList, ChildListOwner, ContentKind, ContentNode, Epoch, NodeId,
    NodeProjection, NodeStability, ProjectionOp, ProtocolLimits, SemanticText, Sequence,
    SourceCursor, SourceDelta, SourceRange, encode_change_json,
};

const NODE_COUNT: usize = 10_000;
const REPLACEMENT_ROUNDS: usize = 10;
const INSERT_BATCH: usize = 1_000;

fn empty_range() -> SourceRange {
    SourceRange::new(SourceCursor::new(0), SourceCursor::new(0))
}

fn projection(round: usize) -> NodeProjection {
    NodeProjection::new(
        NodeStability::Stable,
        empty_range(),
        empty_range(),
        ContentKind::Html {
            block: true,
            text: SemanticText::Normalized {
                value: round.to_string(),
            },
        },
    )
}

fn apply(session: &mut ReducerSession, change: &ChangeSet) -> serde_json::Value {
    let encoded = encode_change_json(change, usize::MAX, ProtocolLimits::default()).unwrap();
    let output = session.apply_change(&encoded).unwrap();
    assert_eq!(output.count(BindingPayloadKind::Snapshot), 0);
    let update = output
        .payloads()
        .iter()
        .find(|payload| payload.kind() == BindingPayloadKind::ReducerUpdate)
        .unwrap();
    serde_json::from_slice(update.bytes()).unwrap()
}

#[test]
fn large_reducer_workload_materializes_only_explicit_node_views() {
    let mut session = ReducerSession::new(b"").unwrap();
    let epoch = Epoch::new(1);
    let mut sequence = 0u64;
    let mut roots = Vec::with_capacity(NODE_COUNT);
    let initial_projection = projection(0);

    for batch_start in (0..NODE_COUNT).step_by(INSERT_BATCH) {
        let batch_end = (batch_start + INSERT_BATCH).min(NODE_COUNT);
        let inserted = (batch_start..batch_end)
            .map(|index| NodeId::new(index as u128 + 1))
            .collect::<Vec<_>>();
        let mut operations = inserted
            .iter()
            .map(|id| ProjectionOp::InsertNode {
                node: ContentNode::new(
                    *id,
                    initial_projection.stability,
                    initial_projection.source,
                    initial_projection.body,
                    Vec::new(),
                    initial_projection.content.clone(),
                ),
            })
            .collect::<Vec<_>>();
        let previous = ChildList::new(roots.clone());
        roots.extend(inserted.iter().copied());
        let next = ChildList::new(roots.clone());
        operations.push(ProjectionOp::SpliceChildren {
            owner: ChildListOwner::Document,
            expected_version: previous.version().clone(),
            start: batch_start as u32,
            delete_count: 0,
            insert: inserted,
            new_version: next.version().clone(),
        });
        let change_id = ChangeId::new(format!("workload:insert:{sequence}")).unwrap();
        let change = if sequence == 0 {
            ChangeSet::start_epoch(
                epoch,
                change_id,
                None,
                SourceDelta::unchanged(SourceCursor::new(0)),
                operations,
            )
            .unwrap()
        } else {
            ChangeSet::new(
                epoch,
                Sequence::new(sequence),
                change_id,
                SourceDelta::unchanged(SourceCursor::new(0)),
                operations,
            )
            .unwrap()
        };
        apply(&mut session, &change);
        sequence += 1;
    }

    let mut expected_version = initial_projection.version.clone();
    for round in 1..=REPLACEMENT_ROUNDS {
        let next = projection(round);
        let operations = roots
            .iter()
            .map(|id| ProjectionOp::ReplaceNode {
                node_id: *id,
                expected_version: expected_version.clone(),
                projection: next.clone(),
            })
            .collect::<Vec<_>>();
        let change = ChangeSet::new(
            epoch,
            Sequence::new(sequence),
            ChangeId::new(format!("workload:replace:{round}")).unwrap(),
            SourceDelta::unchanged(SourceCursor::new(0)),
            operations,
        )
        .unwrap();
        let update = apply(&mut session, &change);
        assert_eq!(
            update["impact"]["changed_node_ids"]
                .as_array()
                .unwrap()
                .len(),
            NODE_COUNT
        );
        assert!(update.get("nodes").is_none());
        expected_version = next.version;
        sequence += 1;
    }

    assert_eq!(session.metrics().materialized_node_views, 0);
    assert_eq!(session.metrics().snapshot_payloads, 0);
    assert_eq!(session.processor_metrics().store_entry_visits, 0);
    for id in roots.iter().take(16) {
        let output = session.node_view(*id).unwrap();
        assert_eq!(output.count(BindingPayloadKind::NodeView), 1);
        let view: serde_json::Value = serde_json::from_slice(output.payloads()[0].bytes()).unwrap();
        assert_eq!(view["body_text"], "");
        assert!(view.get("source_text").is_none());
    }
    assert_eq!(session.metrics().materialized_node_views, 16);
    assert_eq!(session.metrics().snapshot_payloads, 0);

    session.snapshot().unwrap();
    assert_eq!(session.metrics().snapshot_payloads, 1);
    assert_eq!(session.metrics().decoded_change_payloads, sequence);
}
