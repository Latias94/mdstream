use mdstream_bindings_core::{
    BINDING_OPTIONS_SCHEMA, BINDING_SCHEMA, BindingPayloadKind, ReducerSession,
};
use mdstream_protocol::{
    ChangeId, ChangeSet, ChildList, ChildListOwner, ContentKind, ContentNode, Epoch, NodeId,
    NodeStability, ProjectionOp, ProtocolLimits, SourceCursor, SourceDelta, SourceRange,
    encode_change_json,
};

fn range(start: u64, end: u64) -> SourceRange {
    SourceRange::new(SourceCursor::new(start), SourceCursor::new(end))
}

fn start_change(epoch: u64, predecessor: Option<mdstream_protocol::Coordinate>) -> ChangeSet {
    let node_id = NodeId::new(1);
    let roots = ChildList::empty();
    ChangeSet::start_epoch(
        Epoch::new(epoch),
        ChangeId::new(format!("bindings:transition:{epoch}")).unwrap(),
        predecessor,
        SourceDelta::append(SourceCursor::new(0), "A"),
        vec![
            ProjectionOp::AdvanceProjection {
                expected_cursor: SourceCursor::new(0),
                new_cursor: SourceCursor::new(1),
            },
            ProjectionOp::InsertNode {
                node: ContentNode::leaf(
                    node_id,
                    NodeStability::Provisional,
                    range(0, 1),
                    ContentKind::Paragraph {},
                ),
            },
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
    .unwrap()
}

fn transition_options() -> Vec<u8> {
    format!(
        r#"{{
          "schema":"{BINDING_OPTIONS_SCHEMA}",
          "capture_transitions":true,
          "protocol":{{
            "max_source_bytes":"1024",
            "max_nodes":"16",
            "max_resources":"16",
            "max_operations":"32",
            "max_change_structural_items":"64",
            "max_children_per_list":"16"
          }},
          "wire":{{"max_reducer_update_bytes":"1048576"}}
        }}"#
    )
    .into_bytes()
}

fn apply(session: &mut ReducerSession, change: &ChangeSet) -> serde_json::Value {
    let encoded = encode_change_json(change, usize::MAX, ProtocolLimits::default()).unwrap();
    let output = session.apply_change(&encoded).unwrap();
    let update = output
        .payloads()
        .iter()
        .find(|payload| payload.kind() == BindingPayloadKind::ReducerUpdate)
        .unwrap();
    serde_json::from_slice(update.bytes()).unwrap()
}

#[test]
fn transition_capture_is_opt_in_and_disabled_updates_keep_the_old_shape() {
    let mut session = ReducerSession::new(b"").unwrap();
    let start = start_change(1, None);
    let update = apply(&mut session, &start);
    assert_eq!(update["schema"], BINDING_SCHEMA);
    assert!(update.get("transition").is_none());

    let encoded = encode_change_json(&start, usize::MAX, ProtocolLimits::default()).unwrap();
    let retry = session.apply_change(&encoded).unwrap();
    let retry = retry
        .payloads()
        .iter()
        .find(|payload| payload.kind() == BindingPayloadKind::ReducerUpdate)
        .unwrap();
    assert_eq!(
        std::str::from_utf8(retry.bytes()).unwrap(),
        r#"{"schema":"mdstream.bindings/0.4","kind":"reducer_update","outcome":{"kind":"idempotent"},"status":{"kind":"ready"},"impact":{"changed_node_ids":[],"removed_node_ids":[],"changed_resource_ids":[],"removed_resource_ids":[],"source_changed":false,"projection_changed":false,"lifecycle_changed":false,"roots_changed":false,"full_replace":false},"document":{"coordinate":{"epoch":"1","sequence":"0","change_id":"bindings:transition:1","source_cursor":"1"},"lifecycle":"open","projection_cursor":"1"}}"#
    );
}

#[test]
fn enabled_updates_carry_stable_facts_without_a_new_payload_kind() {
    let mut session = ReducerSession::new(&transition_options()).unwrap();
    let start = start_change(1, None);
    let update = apply(&mut session, &start);
    assert_eq!(update["transition"]["schema"], "mdstream.transitions/1");
    assert_eq!(update["transition"]["facts"]["scope"], "continuous");
    assert_eq!(
        update["transition"]["facts"]["after"]["continuity_generation"],
        "0"
    );

    let retry = apply(&mut session, &start);
    assert_eq!(retry["outcome"]["kind"], "idempotent");
    assert!(retry.get("transition").is_none());
}

#[test]
fn enabled_epoch_reset_is_a_coarse_generation_barrier() {
    let mut session = ReducerSession::new(&transition_options()).unwrap();
    let initial = apply(&mut session, &start_change(1, None));
    let predecessor = serde_json::from_value(initial["outcome"]["coordinate"].clone()).unwrap();
    let update = apply(&mut session, &start_change(2, Some(predecessor)));
    assert_eq!(update["transition"]["facts"]["scope"], "full_replace");
    assert_eq!(
        update["transition"]["facts"]["after"]["continuity_generation"],
        "1"
    );
    assert!(update["transition"]["facts"].get("nodes").is_none());
}

#[test]
fn accepted_exact_update_bound_encodes_a_maximum_escaped_append_after_commit() {
    let options = |max_update_bytes: usize| {
        format!(
            r#"{{
              "schema":"{BINDING_OPTIONS_SCHEMA}",
              "capture_transitions":true,
              "protocol":{{
                "max_source_bytes":"64",
                "max_nodes":"2",
                "max_resources":"0",
                "max_operations":"4",
                "max_change_structural_items":"2",
                "max_children_per_list":"1"
              }},
              "wire":{{"max_reducer_update_bytes":"{max_update_bytes}"}}
            }}"#
        )
    };
    let error = ReducerSession::new(options(1).as_bytes()).unwrap_err();
    let required = error
        .message()
        .split("at least ")
        .nth(1)
        .and_then(|value| value.split_whitespace().next())
        .unwrap()
        .parse::<usize>()
        .unwrap();
    let mut session = ReducerSession::new(options(required).as_bytes()).unwrap();

    let text_id = NodeId::new(1);
    let paragraph_id = NodeId::new(2);
    let empty = ChildList::empty();
    let start = ChangeSet::start_epoch(
        Epoch::new(1),
        ChangeId::new("bindings:transition:escaped-start").unwrap(),
        None,
        SourceDelta::unchanged(SourceCursor::new(0)),
        vec![
            ProjectionOp::InsertNode {
                node: ContentNode::leaf(
                    text_id,
                    NodeStability::Provisional,
                    range(0, 0),
                    ContentKind::Text {
                        text: mdstream_protocol::SemanticText::Source {},
                    },
                ),
            },
            ProjectionOp::InsertNode {
                node: ContentNode::leaf(
                    paragraph_id,
                    NodeStability::Provisional,
                    range(0, 0),
                    ContentKind::Paragraph {},
                ),
            },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Node {
                    node_id: paragraph_id,
                },
                expected_version: empty.version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![text_id],
                new_version: empty.version_after_append(&[text_id]),
            },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Document,
                expected_version: empty.version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![paragraph_id],
                new_version: empty.version_after_append(&[paragraph_id]),
            },
        ],
    )
    .unwrap();
    apply(&mut session, &start);
    let text = ContentNode::leaf(
        text_id,
        NodeStability::Provisional,
        range(0, 64),
        ContentKind::Text {
            text: mdstream_protocol::SemanticText::Source {},
        },
    );
    let paragraph = ContentNode::leaf(
        paragraph_id,
        NodeStability::Provisional,
        range(0, 64),
        ContentKind::Paragraph {},
    );
    let append = ChangeSet::new(
        Epoch::new(1),
        mdstream_protocol::Sequence::new(1),
        ChangeId::new("bindings:transition:escaped-append").unwrap(),
        SourceDelta::append(SourceCursor::new(0), "\0".repeat(64)),
        vec![
            ProjectionOp::AdvanceProjection {
                expected_cursor: SourceCursor::new(0),
                new_cursor: SourceCursor::new(64),
            },
            ProjectionOp::ReplaceNode {
                node_id: text_id,
                expected_version: session_node_version(&mut session, text_id),
                projection: text.projection(),
            },
            ProjectionOp::ReplaceNode {
                node_id: paragraph_id,
                expected_version: session_node_version(&mut session, paragraph_id),
                projection: paragraph.projection(),
            },
        ],
    )
    .unwrap();
    let update = apply(&mut session, &append);
    assert_eq!(
        update["transition"]["facts"]["nodes"][0]["text"]["text"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
}

fn session_node_version(
    session: &mut ReducerSession,
    node_id: NodeId,
) -> mdstream_protocol::NodeVersion {
    let output = session.node_view(node_id).unwrap();
    let payload = output
        .payloads()
        .iter()
        .find(|payload| payload.kind() == BindingPayloadKind::NodeView)
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(payload.bytes()).unwrap();
    serde_json::from_value(value["node"]["version"].clone()).unwrap()
}

#[test]
fn snapshot_recovery_omits_same_floor_facts_and_encodes_advanced_full_replace() {
    let start = start_change(1, None);
    let mut consumer = ReducerSession::new(&transition_options()).unwrap();
    apply(&mut consumer, &start);
    let same_floor = snapshot_bytes(&mut consumer);
    let gap = ChangeSet::new(
        Epoch::new(1),
        mdstream_protocol::Sequence::new(2),
        ChangeId::new("bindings:transition:gap").unwrap(),
        SourceDelta::append(SourceCursor::new(1), "gap"),
        Vec::new(),
    )
    .unwrap();
    let gap_update = apply(&mut consumer, &gap);
    assert_eq!(gap_update["outcome"]["kind"], "recovery_required");
    assert!(gap_update.get("transition").is_none());
    let recovered = consumer.recover_snapshot(&same_floor).unwrap();
    let recovered = reducer_update(&recovered);
    assert_eq!(recovered["outcome"]["kind"], "recovered");
    assert!(recovered.get("transition").is_none());

    let mut producer = ReducerSession::new(b"").unwrap();
    apply(&mut producer, &start);
    apply(
        &mut producer,
        &ChangeSet::new(
            Epoch::new(1),
            mdstream_protocol::Sequence::new(1),
            ChangeId::new("bindings:transition:advanced").unwrap(),
            SourceDelta::append(SourceCursor::new(1), "B"),
            Vec::new(),
        )
        .unwrap(),
    );
    let advanced = snapshot_bytes(&mut producer);
    apply(&mut consumer, &gap);
    let recovered = consumer.recover_snapshot(&advanced).unwrap();
    let recovered = reducer_update(&recovered);
    assert_eq!(recovered["transition"]["facts"]["scope"], "full_replace");
    assert_eq!(
        recovered["transition"]["facts"]["after"]["continuity_generation"],
        "1"
    );
}

fn snapshot_bytes(session: &mut ReducerSession) -> Vec<u8> {
    let output = session.snapshot().unwrap();
    output
        .payloads()
        .iter()
        .find(|payload| payload.kind() == BindingPayloadKind::Snapshot)
        .unwrap()
        .bytes()
        .to_vec()
}

fn reducer_update(output: &mdstream_bindings_core::BindingOutput) -> serde_json::Value {
    let payload = output
        .payloads()
        .iter()
        .find(|payload| payload.kind() == BindingPayloadKind::ReducerUpdate)
        .unwrap();
    serde_json::from_slice(payload.bytes()).unwrap()
}
