use mdstream::{EngineOutput, StreamEngine};
use mdstream_conformance::NormalizedSnapshot;
use mdstream_protocol::{
    ApplyOutcome, ContentKind, ContentNode, Document, NodeId, NodeStability, NodeVersion, Reducer,
    Snapshot,
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

fn chunks_by_character_widths<'a>(source: &'a str, widths: &[usize]) -> Vec<&'a str> {
    assert!(!widths.is_empty());
    assert!(widths.iter().all(|width| *width > 0));

    let mut boundaries = source
        .char_indices()
        .map(|(offset, _)| offset)
        .collect::<Vec<_>>();
    boundaries.push(source.len());

    let mut chunks = Vec::new();
    let mut start = 0;
    let character_count = boundaries.len() - 1;
    let mut width_index = 0;
    while start < character_count {
        let end = start
            .saturating_add(widths[width_index % widths.len()])
            .min(character_count);
        chunks.push(&source[boundaries[start]..boundaries[end]]);
        start = end;
        width_index += 1;
    }
    chunks
}

fn assert_subtree_stability(
    document: &Document,
    id: NodeId,
    expected: NodeStability,
    context: &str,
) {
    let current = document
        .node(id)
        .unwrap_or_else(|| panic!("{context}: node {id:?} must exist"));
    assert_eq!(
        current.stability, expected,
        "{context}: node {id:?} must match its root stability"
    );
    for child in current.children.iter().copied() {
        assert_subtree_stability(document, child, expected, context);
    }
}

fn assert_compiler_stability_frontier(document: &Document, context: &str) {
    let mut saw_provisional_root = false;
    for (root_index, root_id) in document.roots().iter().copied().enumerate() {
        let root = document
            .node(root_id)
            .unwrap_or_else(|| panic!("{context}: root {root_id:?} must exist"));
        match root.stability {
            NodeStability::Stable => assert!(
                !saw_provisional_root,
                "{context}: stable root at index {root_index} follows a provisional root"
            ),
            NodeStability::Provisional => saw_provisional_root = true,
        }
        assert_subtree_stability(document, root_id, root.stability, context);
    }
}

#[test]
fn engine_compiler_emits_uniform_subtrees_behind_a_stable_root_prefix() {
    let cases = [
        (
            "paragraph-emphasis",
            "Lead *outer **inner** text* with cafe\u{301} and \u{1f680}.\n\nTrailing paragraph",
        ),
        (
            "nested-list",
            "- first *item*\n  - nested **child**\n  - sibling\n\n- second item\n\nAfter list",
        ),
        (
            "blockquote",
            "> Quote with *emphasis*.\n>\n> 1. nested item\n>    - deeper\n\nOutside quote",
        ),
        (
            "table",
            "| Name | Value |\n| :--- | ---: |\n| *alpha* | `one` |\n| **beta** | two |\n\nAfter table",
        ),
        (
            "fenced-code",
            "Before fence.\n\n```rust\nfn main() {\n    println!(\"stream \u{4f60}\u{597d} \u{1f30a}\");\n}\n```\n\nAfter *fence*",
        ),
    ];

    for (case, source) in cases {
        let schedules = [
            ("whole", vec![source]),
            ("character", chunks_by_character_widths(source, &[1])),
            (
                "uneven-utf8",
                chunks_by_character_widths(source, &[2, 1, 7, 3, 1, 11, 5]),
            ),
        ];

        for (schedule, chunks) in schedules {
            let mut engine = StreamEngine::new();
            let mut reducer = Reducer::new();

            for (append_index, chunk) in chunks.into_iter().enumerate() {
                let output = engine.append(chunk).unwrap_or_else(|error| {
                    panic!("{case}/{schedule} append {append_index} failed: {error}")
                });
                apply_output(&mut reducer, output);
                let document = reducer
                    .document()
                    .unwrap_or_else(|| panic!("{case}/{schedule} append {append_index}: document"));
                let context = format!("{case}/{schedule} append {append_index}");
                assert_compiler_stability_frontier(document, &context);
            }

            apply_output(
                &mut reducer,
                engine
                    .finish()
                    .unwrap_or_else(|error| panic!("{case}/{schedule} finish failed: {error}")),
            );
            let document = reducer
                .document()
                .unwrap_or_else(|| panic!("{case}/{schedule} finish: document"));
            let context = format!("{case}/{schedule} finish");
            assert_compiler_stability_frontier(document, &context);
            assert!(
                !document.roots().is_empty(),
                "{context}: the non-empty source must produce roots"
            );
            assert!(
                document
                    .nodes()
                    .all(|node| node.stability == NodeStability::Stable),
                "{context}: every root subtree node must be stable"
            );
        }
    }
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
