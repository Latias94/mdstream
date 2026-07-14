use mdstream::{EngineOutput, StreamEngine};
use mdstream_conformance::NormalizedSnapshot;
use mdstream_protocol::{
    ApplyOutcome, ContentKind, ContentNode, NodeId, NodeStability, NodeVersion, Reducer, Snapshot,
};

fn apply_output(reducer: &mut Reducer, output: EngineOutput) {
    for change in output.into_changes() {
        let outcome = reducer.apply(change).expect("engine output must replay");
        assert!(matches!(outcome, ApplyOutcome::Applied { .. }));
    }
}

fn compile(chunks: &[&str]) -> Snapshot {
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    for chunk in chunks {
        apply_output(
            &mut reducer,
            engine.append(chunk).expect("append must succeed"),
        );
    }
    apply_output(&mut reducer, engine.finish().expect("finish must succeed"));
    reducer.document().unwrap().snapshot()
}

fn node(snapshot: &Snapshot, id: NodeId) -> &ContentNode {
    snapshot.nodes().iter().find(|node| node.id == id).unwrap()
}

#[test]
fn one_open_frontier_can_project_multiple_top_level_nodes() {
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    apply_output(
        &mut reducer,
        engine
            .append("---\n# Still streaming")
            .expect("append must succeed"),
    );

    let document = reducer.document().expect("append must start a document");
    assert_eq!(document.roots().len(), 2);
    let first = document
        .node(document.roots().as_slice()[0])
        .expect("first root must exist");
    let second = document
        .node(document.roots().as_slice()[1])
        .expect("second root must exist");
    assert!(matches!(first.content, ContentKind::ThematicBreak {}));
    assert!(matches!(second.content, ContentKind::Heading { level: 1 }));
    assert_eq!(second.stability, NodeStability::Provisional);
}

#[test]
fn a_stable_prefix_keeps_identity_while_the_frontier_grows_and_finishes() {
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    apply_output(
        &mut reducer,
        engine
            .append("first paragraph\n\nsecond")
            .expect("append must succeed"),
    );

    let document = reducer.document().expect("append must start a document");
    assert_eq!(document.roots().len(), 2);
    let stable_id = document.roots().as_slice()[0];
    let stable_version: NodeVersion = document
        .node(stable_id)
        .expect("stable root must exist")
        .version
        .clone();
    assert_eq!(
        document
            .node(stable_id)
            .expect("stable root must exist")
            .stability,
        NodeStability::Stable
    );

    apply_output(
        &mut reducer,
        engine.append(" grows").expect("append must succeed"),
    );
    let growing = reducer.document().expect("document must remain available");
    assert_eq!(growing.roots().as_slice()[0], stable_id);
    assert_eq!(
        growing
            .node(stable_id)
            .expect("stable root must remain")
            .version,
        stable_version
    );

    apply_output(&mut reducer, engine.finish().expect("finish must succeed"));
    let finalized = reducer.document().expect("document must remain available");
    assert_eq!(finalized.roots().as_slice()[0], stable_id);
    assert_eq!(
        finalized
            .node(stable_id)
            .expect("stable root must remain")
            .version,
        stable_version
    );
}

#[test]
fn blank_line_does_not_prematurely_stabilize_a_continuing_list() {
    let source = "- foo\n\n  bar\n";

    assert_eq!(
        NormalizedSnapshot::from(compile(&["- foo\n\n", "  bar\n"])),
        NormalizedSnapshot::from(compile(&[source]))
    );
}

#[test]
fn one_append_can_close_a_fence_and_start_the_next_root() {
    let snapshot = compile(&["```text\n", "body\n```\nafter"]);
    let roots = snapshot.roots().as_slice();

    assert_eq!(roots.len(), 2);
    assert!(matches!(
        node(&snapshot, roots[0]).content,
        ContentKind::CodeBlock { .. }
    ));
    assert!(matches!(
        node(&snapshot, roots[1]).content,
        ContentKind::Paragraph {}
    ));
}

#[test]
fn eof_fence_prefix_is_not_stabilized_before_the_line_is_complete() {
    let source = "```\nx\n```x";

    assert_eq!(
        NormalizedSnapshot::from(compile(&["```\nx\n```", "x"])),
        NormalizedSnapshot::from(compile(&[source]))
    );
}

#[test]
fn code_suffixes_stay_pending_without_explicit_fence_state() {
    let mut info_engine = StreamEngine::new();
    let mut info_reducer = Reducer::new();
    apply_output(&mut info_reducer, info_engine.append("```").unwrap());
    let info_suffix = info_engine.append("rust").unwrap();
    assert!(info_suffix.changes()[0].operations().is_empty());
    apply_output(&mut info_reducer, info_suffix);
    assert_eq!(info_reducer.document().unwrap().pending_source(), "rust");

    let mut close_engine = StreamEngine::new();
    let mut close_reducer = Reducer::new();
    apply_output(
        &mut close_reducer,
        close_engine.append("```\nbody\n").unwrap(),
    );
    let closing_suffix = close_engine.append("```").unwrap();
    assert!(closing_suffix.changes()[0].operations().is_empty());
    apply_output(&mut close_reducer, closing_suffix);
    assert_eq!(close_reducer.document().unwrap().pending_source(), "```");
}

#[test]
fn raw_html_blank_lines_do_not_stabilize_before_the_type_specific_closer() {
    let source = "<script>\n\nmore\n</script>";

    assert_eq!(
        NormalizedSnapshot::from(compile(&["<script>\n", "\n", "more\n</script>"])),
        NormalizedSnapshot::from(compile(&[source]))
    );
}

#[test]
fn blank_line_after_type_six_html_is_not_claimed_by_the_html_projection() {
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    apply_output(&mut reducer, engine.append("<div>\n").unwrap());

    apply_output(&mut reducer, engine.append("\noutside").unwrap());
    assert_eq!(reducer.document().unwrap().pending_source(), "\noutside");

    apply_output(&mut reducer, engine.finish().unwrap());
    let document = reducer.document().unwrap();
    let roots = document.roots().as_slice();
    assert_eq!(roots.len(), 2);
    assert!(matches!(
        document.node(roots[0]).unwrap().content,
        ContentKind::Html { block: true, .. }
    ));
    assert!(matches!(
        document.node(roots[1]).unwrap().content,
        ContentKind::Paragraph {}
    ));
}
