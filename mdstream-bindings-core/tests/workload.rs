use mdstream_bindings_core::{BINDING_OPTIONS_SCHEMA, BindingPayloadKind, ReducerSession};
use mdstream_protocol::{
    ApplyOutcome, ChangeId, ChangeSet, ChildList, ChildListOwner, ContentKind, ContentNode,
    Coordinate, Epoch, NodeId, NodeProjection, NodeStability, ProjectionOp, ProtocolLimits,
    Reducer, SemanticText, Sequence, SourceCursor, SourceDelta, SourceRange, TransitionFacts,
    TransitionReducer, encode_change_json, encode_snapshot_json,
};
use serde_json::{Value, json};

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

fn reducer_update_bytes(
    session: &mut ReducerSession,
    change: &ChangeSet,
    limits: ProtocolLimits,
) -> Vec<u8> {
    let encoded = encode_change_json(change, usize::MAX, limits).unwrap();
    let output = session.apply_change(&encoded).unwrap();
    assert_eq!(output.count(BindingPayloadKind::ReducerUpdate), 1);
    output
        .payloads()
        .iter()
        .find(|payload| payload.kind() == BindingPayloadKind::ReducerUpdate)
        .unwrap()
        .bytes()
        .to_vec()
}

fn recovered_update_bytes(session: &mut ReducerSession, snapshot: &[u8]) -> Vec<u8> {
    let output = session.recover_snapshot(snapshot).unwrap();
    assert_eq!(output.count(BindingPayloadKind::ReducerUpdate), 1);
    output
        .payloads()
        .iter()
        .find(|payload| payload.kind() == BindingPayloadKind::ReducerUpdate)
        .unwrap()
        .bytes()
        .to_vec()
}

fn reducer_options(
    limits: ProtocolLimits,
    capture_transitions: bool,
    max_reducer_update_bytes: usize,
) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema": BINDING_OPTIONS_SCHEMA,
        "capture_transitions": capture_transitions,
        "protocol": {
            "max_source_bytes": limits.max_source_bytes.to_string(),
            "max_nodes": limits.max_nodes.to_string(),
            "max_resources": limits.max_resources.to_string(),
            "max_operations": limits.max_operations.to_string(),
            "max_change_structural_items": limits.max_change_structural_items.to_string(),
            "max_children_per_list": limits.max_children_per_list.to_string(),
        },
        "wire": {
            "max_reducer_update_bytes": max_reducer_update_bytes.to_string(),
        },
    }))
    .unwrap()
}

#[derive(Debug)]
struct EncodedTransitionEvidence {
    overhead_bytes: usize,
    facts_json_bytes: usize,
    plain_update_bytes: usize,
    facts: Value,
}

fn encoded_transition_evidence(plain: &[u8], captured: &[u8]) -> EncodedTransitionEvidence {
    let plain_value: Value = serde_json::from_slice(plain).unwrap();
    assert!(plain_value.get("transition").is_none());

    let mut captured_without_transition: Value = serde_json::from_slice(captured).unwrap();
    let transition = captured_without_transition
        .as_object_mut()
        .unwrap()
        .remove("transition")
        .expect("capture-enabled state changes include transition facts");
    assert_eq!(captured_without_transition, plain_value);

    let transition_json_bytes = serde_json::to_vec(&transition).unwrap().len();
    let overhead_bytes = captured.len().checked_sub(plain.len()).unwrap();
    assert_eq!(
        overhead_bytes,
        b",\"transition\":".len() + transition_json_bytes,
        "the exact wire delta is the trailing transition member"
    );
    let facts = transition.get("facts").unwrap().clone();
    let facts_json_bytes = serde_json::to_vec(&facts).unwrap().len();

    EncodedTransitionEvidence {
        overhead_bytes,
        facts_json_bytes,
        plain_update_bytes: plain.len(),
        facts,
    }
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

#[test]
fn continuous_append_reports_exact_owned_text_and_wire_overhead() {
    let suffix = "B\u{1f642}\u{301}";
    let limits = ProtocolLimits {
        max_source_bytes: 64,
        max_nodes: 2,
        max_resources: 0,
        max_operations: 8,
        max_change_structural_items: 4,
        max_children_per_list: 1,
        ..ProtocolLimits::default()
    };
    let start = source_text_start();
    let append = source_text_append(suffix);
    let plain_options = reducer_options(limits, false, 1024 * 1024);
    let captured_options = reducer_options(limits, true, 1024 * 1024);
    let mut plain = ReducerSession::new(&plain_options).unwrap();
    let mut captured = ReducerSession::new(&captured_options).unwrap();

    reducer_update_bytes(&mut plain, &start, limits);
    reducer_update_bytes(&mut captured, &start, limits);
    let plain_update = reducer_update_bytes(&mut plain, &append, limits);
    let captured_update = reducer_update_bytes(&mut captured, &append, limits);
    let wire = encoded_transition_evidence(&plain_update, &captured_update);
    assert!(wire.overhead_bytes > wire.facts_json_bytes);

    let mut reducer = TransitionReducer::with_limits(limits);
    reducer.apply(start).unwrap();
    let before = reducer.transition_metrics();
    let report = reducer.apply(append).unwrap();
    let after = reducer.transition_metrics();
    let facts = report.facts.unwrap();

    assert_eq!(after.facts_built - before.facts_built, 1);
    assert_eq!(after.entity_visits - before.entity_visits, 4);
    assert_eq!(after.splice_ids_copied - before.splice_ids_copied, 0);
    assert_eq!(
        after.owned_text_bytes_copied - before.owned_text_bytes_copied,
        suffix.len() as u64
    );
    assert_eq!(wire.facts, serde_json::to_value(facts).unwrap());
}

#[test]
fn advanced_full_replace_facts_and_wire_overhead_are_constant_size() {
    let small = advanced_full_replace_evidence(1);
    let large = advanced_full_replace_evidence(NODE_COUNT);

    assert_eq!(large.facts_json_bytes, small.facts_json_bytes);
    assert_eq!(large.overhead_bytes, small.overhead_bytes);
    assert!(
        large.plain_update_bytes > small.plain_update_bytes,
        "whole-update impact and roots retain their explicit document-sized cost"
    );
}

fn source_text_start() -> ChangeSet {
    let text_id = NodeId::new(1);
    let paragraph_id = NodeId::new(2);
    let source = SourceRange::new(SourceCursor::new(0), SourceCursor::new(1));
    let text_children = ChildList::new(vec![text_id]);
    let roots = ChildList::new(vec![paragraph_id]);
    ChangeSet::start_epoch(
        Epoch::new(1),
        ChangeId::new("workload:text:start").unwrap(),
        None,
        SourceDelta::append(SourceCursor::new(0), "A"),
        vec![
            ProjectionOp::AdvanceProjection {
                expected_cursor: SourceCursor::new(0),
                new_cursor: SourceCursor::new(1),
            },
            ProjectionOp::InsertNode {
                node: ContentNode::leaf(
                    text_id,
                    NodeStability::Provisional,
                    source,
                    ContentKind::Text {
                        text: SemanticText::Source {},
                    },
                ),
            },
            ProjectionOp::InsertNode {
                node: ContentNode::leaf(
                    paragraph_id,
                    NodeStability::Provisional,
                    source,
                    ContentKind::Paragraph {},
                ),
            },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Node {
                    node_id: paragraph_id,
                },
                expected_version: ChildList::empty().version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![text_id],
                new_version: text_children.version().clone(),
            },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Document,
                expected_version: ChildList::empty().version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![paragraph_id],
                new_version: roots.version().clone(),
            },
        ],
    )
    .unwrap()
}

fn source_text_append(suffix: &str) -> ChangeSet {
    let text_id = NodeId::new(1);
    let paragraph_id = NodeId::new(2);
    let start = source_text_start();
    let current_text = start
        .operations()
        .iter()
        .find_map(|operation| match operation {
            ProjectionOp::InsertNode { node } if node.id == text_id => Some(node),
            _ => None,
        })
        .unwrap();
    let current_paragraph = start
        .operations()
        .iter()
        .find_map(|operation| match operation {
            ProjectionOp::InsertNode { node } if node.id == paragraph_id => Some(node),
            _ => None,
        })
        .unwrap();
    let end = 1 + suffix.len() as u64;
    ChangeSet::new(
        Epoch::new(1),
        Sequence::new(1),
        ChangeId::new("workload:text:append").unwrap(),
        SourceDelta::append(SourceCursor::new(1), suffix),
        vec![
            ProjectionOp::AdvanceProjection {
                expected_cursor: SourceCursor::new(1),
                new_cursor: SourceCursor::new(end),
            },
            ProjectionOp::ReplaceNode {
                node_id: text_id,
                expected_version: current_text.version.clone(),
                projection: NodeProjection::new(
                    NodeStability::Provisional,
                    SourceRange::new(SourceCursor::new(0), SourceCursor::new(end)),
                    SourceRange::new(SourceCursor::new(0), SourceCursor::new(end)),
                    ContentKind::Text {
                        text: SemanticText::Source {},
                    },
                ),
            },
            ProjectionOp::ReplaceNode {
                node_id: paragraph_id,
                expected_version: current_paragraph.version.clone(),
                projection: NodeProjection::new(
                    NodeStability::Provisional,
                    SourceRange::new(SourceCursor::new(0), SourceCursor::new(end)),
                    SourceRange::new(SourceCursor::new(0), SourceCursor::new(end)),
                    ContentKind::Paragraph {},
                ),
            },
        ],
    )
    .unwrap()
}

fn population_limits(node_count: usize) -> ProtocolLimits {
    ProtocolLimits {
        max_source_bytes: 1,
        max_nodes: node_count,
        max_resources: 0,
        max_operations: node_count + 2,
        max_change_structural_items: node_count * 2 + 1,
        max_children_per_list: node_count,
        ..ProtocolLimits::default()
    }
}

fn population_start(epoch: u64, predecessor: Option<Coordinate>, node_count: usize) -> ChangeSet {
    let ids = (1..=node_count)
        .map(|id| NodeId::new(id as u128))
        .collect::<Vec<_>>();
    let range = empty_range();
    let mut operations = ids
        .iter()
        .map(|id| ProjectionOp::InsertNode {
            node: ContentNode::leaf(*id, NodeStability::Stable, range, ContentKind::Paragraph {}),
        })
        .collect::<Vec<_>>();
    let roots = ChildList::new(ids.clone());
    operations.push(ProjectionOp::SpliceChildren {
        owner: ChildListOwner::Document,
        expected_version: ChildList::empty().version().clone(),
        start: 0,
        delete_count: 0,
        insert: ids,
        new_version: roots.version().clone(),
    });
    ChangeSet::start_epoch(
        Epoch::new(epoch),
        ChangeId::new(format!("workload:population:epoch-{epoch}")).unwrap(),
        predecessor,
        SourceDelta::unchanged(SourceCursor::new(0)),
        operations,
    )
    .unwrap()
}

fn recovery_gap() -> ChangeSet {
    ChangeSet::new(
        Epoch::new(1),
        Sequence::new(2),
        ChangeId::new("workload:population:gap").unwrap(),
        SourceDelta::append(SourceCursor::new(0), "x"),
        Vec::new(),
    )
    .unwrap()
}

fn advanced_full_replace_evidence(node_count: usize) -> EncodedTransitionEvidence {
    const MAX_REDUCER_UPDATE_BYTES: usize = 128 * 1024 * 1024;

    let limits = population_limits(node_count);
    let start = population_start(1, None, node_count);
    let mut producer = Reducer::with_limits(limits);
    assert!(matches!(
        producer.apply(start.clone()).unwrap(),
        ApplyOutcome::Applied { .. }
    ));
    let predecessor = producer.document().unwrap().coordinate().clone();
    let replacement = population_start(2, Some(predecessor), node_count);
    assert!(matches!(
        producer.apply(replacement).unwrap(),
        ApplyOutcome::Applied { .. } | ApplyOutcome::Recovered { .. }
    ));
    let snapshot = producer.document().unwrap().snapshot();
    let snapshot_bytes = encode_snapshot_json(&snapshot, usize::MAX, limits).unwrap();
    let gap = recovery_gap();

    let plain_options = reducer_options(limits, false, MAX_REDUCER_UPDATE_BYTES);
    let captured_options = reducer_options(limits, true, MAX_REDUCER_UPDATE_BYTES);
    let mut plain = ReducerSession::new(&plain_options).unwrap();
    let mut captured = ReducerSession::new(&captured_options).unwrap();
    reducer_update_bytes(&mut plain, &start, limits);
    reducer_update_bytes(&mut captured, &start, limits);
    let plain_gap = reducer_update_bytes(&mut plain, &gap, limits);
    let captured_gap = reducer_update_bytes(&mut captured, &gap, limits);
    assert_eq!(captured_gap, plain_gap);

    let plain_update = recovered_update_bytes(&mut plain, &snapshot_bytes);
    let captured_update = recovered_update_bytes(&mut captured, &snapshot_bytes);
    let wire = encoded_transition_evidence(&plain_update, &captured_update);
    assert_eq!(wire.facts["scope"], "full_replace");
    assert_eq!(wire.facts.as_object().unwrap().len(), 3);

    let mut reducer = TransitionReducer::with_limits(limits);
    reducer.apply(start).unwrap();
    let gap_report = reducer.apply(gap).unwrap();
    assert!(matches!(
        gap_report.outcome,
        ApplyOutcome::RecoveryRequired { .. }
    ));
    assert!(gap_report.facts.is_none());
    let before = reducer.transition_metrics();
    let report = reducer.recover_snapshot(snapshot).unwrap();
    let after = reducer.transition_metrics();
    assert!(matches!(report.outcome, ApplyOutcome::Recovered { .. }));
    assert_eq!(after.facts_built - before.facts_built, 1);
    assert_eq!(after.entity_visits - before.entity_visits, 1);
    assert_eq!(after.splice_ids_copied - before.splice_ids_copied, 0);
    assert_eq!(
        after.owned_text_bytes_copied - before.owned_text_bytes_copied,
        0
    );
    let facts = report.facts.unwrap();
    assert!(matches!(facts, TransitionFacts::FullReplace { .. }));
    assert_eq!(wire.facts, serde_json::to_value(facts).unwrap());

    wire
}
