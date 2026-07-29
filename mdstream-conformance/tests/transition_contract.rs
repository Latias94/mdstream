use std::path::PathBuf;

use mdstream_conformance::{
    HostReconstruction, HostReconstructionOutcome, HostReconstructionTrace, TextReconstruction,
    cross_check_transition_trace, load_fixture_dir, reconstruct_host_trace,
};
use mdstream_protocol::{
    ChangeId, ChangeSet, ChildList, ChildListOwner, CodeBlockSyntax, ContentKind, ContentNode,
    Epoch, NodeId, NodeProjection, NodeStability, ProjectionOp, SemanticText, Sequence,
    SourceCursor, SourceDelta, SourceRange,
};

fn corpus_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance")
}

fn fixture_trace(fixture_id: &str, trace_id: &str) -> mdstream_conformance::ProtocolTrace {
    load_fixture_dir(corpus_root().join("fixtures"))
        .unwrap()
        .into_iter()
        .find(|fixture| fixture.id == fixture_id)
        .unwrap()
        .traces
        .into_iter()
        .find(|trace| trace.id == trace_id)
        .unwrap()
}

#[test]
fn transition_facts_match_host_reconstruction_for_adversarial_adoption_trace() {
    let trace = fixture_trace("adoption.headless-rich-content", "adversarial");

    let report = cross_check_transition_trace(&trace).unwrap();

    assert_eq!(report.checked_steps, trace.changes.len());
}

#[test]
fn transition_cross_check_treats_epoch_reset_as_a_coarse_full_replace() {
    let trace = fixture_trace("protocol.epoch-reset", "reset");

    let report = cross_check_transition_trace(&trace).unwrap();

    assert!(report.full_replacements > 0);
}

#[test]
fn transition_cross_check_requires_no_facts_for_an_idempotent_retry() {
    let fixture = fixture_trace("adoption.headless-rich-content", "whole");
    let initial = fixture.changes[0].clone();
    let trace = mdstream_conformance::ProtocolTrace {
        id: "idempotent-retry".to_string(),
        schedule: "retry".to_string(),
        setup_changes: 0,
        input_events: Vec::new(),
        changes: vec![initial.clone(), initial],
    };

    let report = cross_check_transition_trace(&trace).unwrap();

    assert_eq!(report.no_facts, 1);
}

#[test]
fn reconstruction_trace_is_deterministic_and_quantifies_host_bookkeeping() {
    let trace = fixture_trace("adoption.headless-rich-content", "adversarial");
    let first = reconstruct_host_trace(&trace).unwrap();
    let second = reconstruct_host_trace(&trace).unwrap();

    assert_eq!(first, second);
    assert_eq!(first.steps.len(), trace.changes.len());
    assert_eq!(first.setup_changes, trace.setup_changes);
    assert_eq!(first.input_events, trace.input_events);
    assert!(first.max_retained.node_views > 0);
    assert!(first.max_retained.parent_entries > 0);
    assert!(first.total_work.node_views_materialized >= first.max_retained.node_views);
    assert!(first.steps.iter().any(|step| !step.structures.is_empty()));
    assert!(first.steps.iter().any(|step| !step.resources.is_empty()));
    assert!(first.steps.iter().any(|step| {
        step.nodes
            .iter()
            .any(|node| node.before.is_none() && node.after.is_some())
    }));
    assert!(first.steps.iter().any(|step| {
        step.nodes
            .iter()
            .any(|node| node.after.is_none() && node.before.is_some())
    }));
    assert!(first.steps.iter().any(|step| {
        step.nodes.iter().any(|node| {
            node.before.as_ref().is_some_and(|before| {
                before.stability == NodeStability::Provisional
                    && node
                        .after
                        .as_ref()
                        .is_some_and(|after| after.stability == NodeStability::Stable)
            })
        })
    }));
    assert!(first.steps.iter().any(|step| {
        step.nodes.iter().any(|node| {
            node.before.as_ref().is_some_and(|before| {
                node.after
                    .as_ref()
                    .is_some_and(|after| before.version != after.version)
            })
        })
    }));
    assert!(first.steps.iter().any(|step| {
        step.pending_source
            .as_ref()
            .is_some_and(|pending| !pending.text.is_empty())
    }));
    assert!(
        first
            .steps
            .last()
            .unwrap()
            .pending_source
            .as_ref()
            .is_some_and(|pending| pending.text.is_empty())
    );
    assert!(first.steps.last().unwrap().impact.lifecycle_changed);

    let first_json = serde_json::to_string_pretty(&first).unwrap();
    let second_json = serde_json::to_string_pretty(&second).unwrap();
    assert_eq!(first_json, second_json);
    assert_eq!(
        serde_json::from_str::<HostReconstructionTrace>(&first_json).unwrap(),
        first
    );
}

#[test]
fn chunk_schedules_converge_but_reconstruction_work_is_schedule_local() {
    let whole_trace = fixture_trace("adoption.headless-rich-content", "whole");
    let adversarial_trace = fixture_trace("adoption.headless-rich-content", "adversarial");
    let whole_snapshot = mdstream_conformance::replay_protocol_trace(&whole_trace)
        .unwrap()
        .normalized_final_snapshot();
    let adversarial_snapshot = mdstream_conformance::replay_protocol_trace(&adversarial_trace)
        .unwrap()
        .normalized_final_snapshot();
    let whole = reconstruct_host_trace(&whole_trace).unwrap();
    let adversarial = reconstruct_host_trace(&adversarial_trace).unwrap();

    assert_eq!(whole_snapshot, adversarial_snapshot);
    assert_eq!(whole.final_retained, adversarial.final_retained);
    assert_ne!(whole.steps.len(), adversarial.steps.len());
    assert_ne!(whole.total_work, adversarial.total_work);
}

#[test]
fn host_observes_reorder_reparent_and_subtree_removal() {
    let range = SourceRange::new(SourceCursor::new(0), SourceCursor::new(0));
    let parent_a = NodeId::new(1);
    let parent_b = NodeId::new(2);
    let child_c = NodeId::new(3);
    let child_d = NodeId::new(4);
    let a = ContentNode::new(
        parent_a,
        NodeStability::Stable,
        range,
        range,
        Vec::new(),
        ContentKind::Paragraph {},
    );
    let b = ContentNode::new(
        parent_b,
        NodeStability::Stable,
        range,
        range,
        Vec::new(),
        ContentKind::Paragraph {},
    );
    let c = ContentNode::leaf(
        child_c,
        NodeStability::Stable,
        range,
        ContentKind::Text {
            text: SemanticText::Normalized {
                value: "C".to_string(),
            },
        },
    );
    let d = ContentNode::leaf(
        child_d,
        NodeStability::Stable,
        range,
        ContentKind::Text {
            text: SemanticText::Normalized {
                value: "D".to_string(),
            },
        },
    );
    let roots = ChildList::empty();
    let initial_children = ChildList::new(vec![child_c, child_d]);
    let start = ChangeSet::start_epoch(
        Epoch::new(1),
        ChangeId::new("host-structure:start").unwrap(),
        None,
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![
            ProjectionOp::InsertNode { node: a.clone() },
            ProjectionOp::InsertNode { node: b.clone() },
            ProjectionOp::InsertNode { node: c.clone() },
            ProjectionOp::InsertNode { node: d },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Node { node_id: parent_a },
                expected_version: a.children.version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![child_c, child_d],
                new_version: initial_children.version().clone(),
            },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Document,
                expected_version: roots.version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![parent_a, parent_b],
                new_version: roots.version_after_append(&[parent_a, parent_b]),
            },
        ],
    )
    .unwrap();

    let reordered = ChildList::new(vec![child_d, child_c]);
    let reorder = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(1),
        ChangeId::new("host-structure:reorder").unwrap(),
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![ProjectionOp::SpliceChildren {
            owner: ChildListOwner::Node { node_id: parent_a },
            expected_version: initial_children.version().clone(),
            start: 0,
            delete_count: 2,
            insert: vec![child_d, child_c],
            new_version: reordered.version().clone(),
        }],
    )
    .unwrap();

    let a_after_move = ChildList::new(vec![child_d]);
    let b_after_move = ChildList::new(vec![child_c]);
    let reparent = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(2),
        ChangeId::new("host-structure:reparent").unwrap(),
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Node { node_id: parent_a },
                expected_version: reordered.version().clone(),
                start: 1,
                delete_count: 1,
                insert: Vec::new(),
                new_version: a_after_move.version().clone(),
            },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Node { node_id: parent_b },
                expected_version: b.children.version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![child_c],
                new_version: b_after_move.version().clone(),
            },
        ],
    )
    .unwrap();

    let remove = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(3),
        ChangeId::new("host-structure:remove").unwrap(),
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Node { node_id: parent_b },
                expected_version: b_after_move.version().clone(),
                start: 0,
                delete_count: 1,
                insert: Vec::new(),
                new_version: ChildList::empty().version().clone(),
            },
            ProjectionOp::RemoveNode {
                node_id: child_c,
                expected_version: c.version.clone(),
            },
        ],
    )
    .unwrap();

    let mut host = HostReconstruction::new();
    host.apply(start).unwrap();
    let reorder = host.apply(reorder).unwrap();
    let reparent = host.apply(reparent).unwrap();
    let remove = host.apply(remove).unwrap();

    let splice = reorder.structures[0].splice.as_ref().unwrap();
    assert_eq!(splice.start, 0);
    assert_eq!(splice.delete_count, 2);
    assert_eq!(splice.insert, vec![child_d, child_c]);
    assert!(reparent.nodes.iter().any(|node| {
        node.node_id == child_c
            && node.previous_parent == Some(ChildListOwner::Node { node_id: parent_a })
            && node.parent == Some(ChildListOwner::Node { node_id: parent_b })
    }));
    assert_eq!(reparent.structures.len(), 2);
    assert!(remove.impact.removed_nodes.contains(&child_c));
    assert!(remove.nodes.iter().any(|node| {
        node.node_id == child_c
            && node.before.is_some()
            && node.after.is_none()
            && node.text == Some(TextReconstruction::Removed)
    }));
}

#[test]
fn reset_is_a_barrier_but_same_floor_recovery_reuses_retained_host_state() {
    let reset = reconstruct_host_trace(&fixture_trace("protocol.epoch-reset", "reset")).unwrap();
    let replacement = reset
        .steps
        .iter()
        .find(|step| step.impact.full_replace)
        .unwrap();
    assert!(replacement.continuity_barrier);
    assert!(replacement.continuity_generation > 0);

    let trace = fixture_trace("adoption.headless-rich-content", "adversarial");
    let mut host = HostReconstruction::new();
    for change in trace.changes.iter().take(3).cloned() {
        host.apply(change).unwrap();
    }
    let snapshot = host.snapshot().unwrap();
    let generation = host.continuity_generation();
    let retained = host.retained_bookkeeping();

    let gap = host.apply(trace.changes[4].clone()).unwrap();
    assert_eq!(gap.outcome, HostReconstructionOutcome::RecoveryRequired);
    let recovered = host.recover_snapshot(snapshot).unwrap();
    assert_eq!(recovered.outcome, HostReconstructionOutcome::Recovered);
    assert!(!recovered.document_changed);
    assert!(!recovered.continuity_barrier);
    assert_eq!(recovered.continuity_generation, generation);
    assert_eq!(recovered.work.node_views_materialized, 0);
    assert_eq!(host.retained_bookkeeping(), retained);
}

#[test]
fn deterministic_versions_do_not_collapse_a_b_a_observations() {
    let range = SourceRange::new(SourceCursor::new(0), SourceCursor::new(0));
    let a = NodeProjection::new(NodeStability::Stable, range, range, code_block("A"));
    let b = NodeProjection::new(NodeStability::Stable, range, range, code_block("AB"));
    let node_id = NodeId::new(1);
    let node = ContentNode::new(
        node_id,
        a.stability,
        a.source,
        a.body,
        Vec::new(),
        a.content.clone(),
    );
    let roots = ChildList::empty();
    let start = ChangeSet::start_epoch(
        Epoch::new(1),
        ChangeId::new("host-baseline:a").unwrap(),
        None,
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![
            ProjectionOp::InsertNode { node },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Document,
                expected_version: roots.version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![node_id],
                new_version: roots.version_after_append(&[node_id]),
            },
        ],
    )
    .unwrap();
    let to_b = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(1),
        ChangeId::new("host-baseline:b").unwrap(),
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![ProjectionOp::ReplaceNode {
            node_id,
            expected_version: a.version.clone(),
            projection: b.clone(),
        }],
    )
    .unwrap();
    let back_to_a = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(2),
        ChangeId::new("host-baseline:a-again").unwrap(),
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![ProjectionOp::ReplaceNode {
            node_id,
            expected_version: b.version.clone(),
            projection: a.clone(),
        }],
    )
    .unwrap();

    let mut host = HostReconstruction::new();
    let initial = host.apply(start).unwrap().clone();
    let changed = host.apply(to_b).unwrap().clone();
    let restored = host.apply(back_to_a).unwrap().clone();

    let initial_version = initial.nodes[0].after.as_ref().unwrap().version.clone();
    let changed_version = changed.nodes[0].after.as_ref().unwrap().version.clone();
    let restored_version = restored.nodes[0].after.as_ref().unwrap().version.clone();
    assert_ne!(changed_version, initial_version);
    assert_eq!(restored_version, initial_version);
    assert_eq!(changed.nodes[0].text, Some(TextReconstruction::Replaced));
    assert_eq!(restored.nodes[0].text, Some(TextReconstruction::Replaced));
}

fn code_block(value: &str) -> ContentKind {
    ContentKind::CodeBlock {
        syntax: CodeBlockSyntax::Indented,
        info: None,
        text: SemanticText::Normalized {
            value: value.to_string(),
        },
    }
}
