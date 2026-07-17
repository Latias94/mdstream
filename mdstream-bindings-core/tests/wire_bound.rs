use mdstream::{CustomBlockSpec, EngineLimits, EngineOutput, StreamEngine};
use mdstream_bindings_core::{BINDING_OPTIONS_SCHEMA, BindingPayloadKind, ReducerSession};
use mdstream_protocol::{
    ChangeId, ChangeSet, ChildList, ChildListOwner, ContentKind, ContentNode, Epoch, NodeId,
    NodeStability, ProjectionOp, ProtocolLimits, SourceCursor, SourceDelta, SourceRange,
    encode_change_json,
};
use proptest::prelude::*;

fn assert_wire_bound(engine: &StreamEngine, output: &EngineOutput) {
    if output.is_empty() {
        return;
    }
    assert_eq!(output.changes().len(), 1);
    let measured_change_bytes = engine.metrics().work.last_change_bytes;
    let encoded =
        encode_change_json(&output.changes()[0], usize::MAX, ProtocolLimits::default()).unwrap();
    let bound = measured_change_bytes.checked_mul(6).unwrap();
    assert!(
        encoded.len() <= bound,
        "encoded={} bound={} measured={} change={:?}",
        encoded.len(),
        bound,
        measured_change_bytes,
        output.changes()[0]
    );
}

fn append_checked(engine: &mut StreamEngine, chunk: &str) {
    let output = engine.append(chunk).unwrap();
    assert_wire_bound(engine, &output);
}

fn finish_checked(engine: &mut StreamEngine) {
    let output = engine.finish().unwrap();
    assert_wire_bound(engine, &output);
}

#[test]
fn engine_limit_exposes_the_checked_post_commit_encoding_budget() {
    assert_eq!(
        EngineLimits {
            max_change_bytes: 123,
            max_transaction_bytes: 456,
        }
        .minimum_encoded_change_bytes(),
        Some(738)
    );
    assert_eq!(
        EngineLimits {
            max_change_bytes: usize::MAX,
            max_transaction_bytes: usize::MAX,
        }
        .minimum_encoded_change_bytes(),
        None
    );
}

#[test]
fn mixed_ir_correction_and_structural_growth_stay_inside_the_wire_bound() {
    let mut engine = StreamEngine::builder()
        .custom_block(CustomBlockSpec::try_new("test.note/1", "note").unwrap())
        .build()
        .unwrap();
    append_checked(
        &mut engine,
        "# Heading\n\nParagraph *em* **strong** ~~strike~~ `code` [link][ref] ![alt](img).  \nnext\n\n> quote\n\n- [x] item\n- item 2\n\n---\n\n| a | b |\n| - | -: |\n| 1 | 2 |\n\n<div>html</div>\n\n$x$\n\n$$\ny\n$$\n\n[^note] [@cite]\n\n<note mode=\"x\">\ncustom **body**\n</note>\n",
    );
    append_checked(
        &mut engine,
        "\n[ref]: https://example.test \"Example\"\n[^note]: footnote body\n[@cite]: https://cite.test \"Citation\"\n",
    );
    finish_checked(&mut engine);

    let reset = engine.reset().unwrap();
    assert_wire_bound(&engine, &reset);

    let mut list = String::new();
    for index in 0..1_000 {
        list.push_str(&format!("- item {index}\n"));
    }
    append_checked(&mut engine, &list);
    finish_checked(&mut engine);
}

#[test]
fn configured_view_bound_covers_a_maximum_escaped_node_body() {
    const SOURCE_BYTES: usize = 64;
    const NODE_STRUCTURAL_ITEMS: usize = 3;
    const REQUIRED_VIEW_BYTES: usize =
        (SOURCE_BYTES + 1 + NODE_STRUCTURAL_ITEMS * 64 + 4 * 1024) * 6;

    let options = format!(
        r#"{{
          "schema":"{BINDING_OPTIONS_SCHEMA}",
          "protocol":{{
            "max_source_bytes":"{SOURCE_BYTES}",
            "max_children_per_list":"1",
            "max_attributes_per_node":"1",
            "max_node_metadata_bytes":"1"
          }},
          "processor":{{
            "max_input_bytes":"0",
            "max_artifact_bytes":"0",
            "max_error_bytes":"0"
          }},
          "wire":{{"max_view_bytes":"{REQUIRED_VIEW_BYTES}"}}
        }}"#
    );
    let mut session = ReducerSession::new(options.as_bytes()).unwrap();
    let source = "\0".repeat(SOURCE_BYTES);
    let node_id = NodeId::new(1);
    let range = SourceRange::new(SourceCursor::new(0), SourceCursor::new(SOURCE_BYTES as u64));
    let roots = ChildList::new(vec![node_id]);
    let change = ChangeSet::start_epoch(
        Epoch::new(1),
        ChangeId::new("bindings:view-bound:start").unwrap(),
        None,
        SourceDelta::append(SourceCursor::new(0), source.clone()),
        vec![
            ProjectionOp::InsertNode {
                node: ContentNode::leaf(
                    node_id,
                    NodeStability::Stable,
                    range,
                    ContentKind::ThematicBreak {},
                ),
            },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Document,
                expected_version: ChildList::new(Vec::new()).version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![node_id],
                new_version: roots.version().clone(),
            },
            ProjectionOp::AdvanceProjection {
                expected_cursor: SourceCursor::new(0),
                new_cursor: SourceCursor::new(SOURCE_BYTES as u64),
            },
        ],
    )
    .unwrap();
    let encoded = encode_change_json(&change, usize::MAX, ProtocolLimits::default()).unwrap();
    session.apply_change(&encoded).unwrap();

    let output = session.node_view(node_id).unwrap();
    let view = output
        .payloads()
        .iter()
        .find(|payload| payload.kind() == BindingPayloadKind::NodeView)
        .unwrap();
    let view: serde_json::Value = serde_json::from_slice(view.bytes()).unwrap();
    assert_eq!(view["body_text"], source);
    assert!(view.get("source_text").is_none());
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 96,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn arbitrary_utf8_and_json_escaping_stay_inside_the_wire_bound(
        characters in proptest::collection::vec(any::<char>(), 0..512),
        split_seed in any::<u64>(),
    ) {
        let source = characters.into_iter().collect::<String>();
        let mut engine = StreamEngine::new();
        if source.is_empty() {
            finish_checked(&mut engine);
            return Ok(());
        }

        let mut cursor = 0usize;
        let mut state = split_seed | 1;
        while cursor < source.len() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let width = (state as usize % 31).saturating_add(1);
            let mut end = cursor.saturating_add(width).min(source.len());
            while end < source.len() && !source.is_char_boundary(end) {
                end += 1;
            }
            append_checked(&mut engine, &source[cursor..end]);
            cursor = end;
        }
        finish_checked(&mut engine);
    }
}
