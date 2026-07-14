use mdstream::{EngineOutput, StreamEngine};
use mdstream_protocol::{
    ApplyOutcome, ContentKind, ContentNode, NodeId, Reducer, SemanticText, Snapshot, SourceCursor,
    SourceRange,
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
    reducer
        .document()
        .expect("finish must produce a document")
        .snapshot()
}

fn node(snapshot: &Snapshot, id: NodeId) -> &ContentNode {
    snapshot
        .nodes()
        .iter()
        .find(|node| node.id == id)
        .expect("child identity must resolve")
}

fn slice(source: &str, range: SourceRange) -> &str {
    let start = usize::try_from(range.start.get()).unwrap();
    let end = usize::try_from(range.end.get()).unwrap();
    &source[start..end]
}

fn semantic_value(snapshot: &Snapshot, owner: &ContentNode, text: &SemanticText) -> String {
    match text {
        SemanticText::Source {} => slice(snapshot.source(), owner.body).to_string(),
        SemanticText::Normalized { value } => value.clone(),
    }
}

#[test]
fn canonical_ranges_are_utf8_byte_aligned_after_split_crlf_normalization() {
    let snapshot = compile(&["# cafe\u{301} 世界 &amp;\r", "\n\r", "\n尾巴"]);
    assert_eq!(snapshot.source(), "# cafe\u{301} 世界 &amp;\n\n尾巴");

    for content in snapshot.nodes() {
        assert!(content.source.contains(content.body));
        for range in [content.source, content.body] {
            let start = usize::try_from(range.start.get()).unwrap();
            let end = usize::try_from(range.end.get()).unwrap();
            assert!(snapshot.source().is_char_boundary(start));
            assert!(snapshot.source().is_char_boundary(end));
        }
    }

    let heading = node(&snapshot, snapshot.roots().as_slice()[0]);
    assert!(matches!(heading.content, ContentKind::Heading { level: 1 }));
    assert_eq!(
        slice(snapshot.source(), heading.source),
        "# cafe\u{301} 世界 &amp;"
    );
    assert_eq!(
        slice(snapshot.source(), heading.body),
        "cafe\u{301} 世界 &amp;"
    );
    let heading_text = node(&snapshot, heading.children.as_slice()[0]);
    let ContentKind::Text { text } = &heading_text.content else {
        panic!("heading must own text");
    };
    assert_eq!(
        slice(snapshot.source(), heading_text.source),
        "cafe\u{301} 世界 &amp;"
    );
    assert_eq!(
        semantic_value(&snapshot, heading_text, text),
        "cafe\u{301} 世界 &"
    );
    assert!(matches!(text, SemanticText::Normalized { .. }));

    let paragraph = node(&snapshot, snapshot.roots().as_slice()[1]);
    assert_eq!(slice(snapshot.source(), paragraph.source), "尾巴");
    assert_eq!(paragraph.source, paragraph.body);
}

#[test]
fn syntax_ranges_and_semantic_text_distinguish_raw_from_normalized_values() {
    let snapshot = compile(&[
        "前 ` 代码 ` &amp; $α + β$\n\n",
        "<div>中🙂</div>\n\n",
        "~~~~rust linenos\nfn 中() {}\n~~~~",
    ]);
    let source = snapshot.source();

    let paragraph = node(&snapshot, snapshot.roots().as_slice()[0]);
    let phrasing = paragraph
        .children
        .iter()
        .map(|id| node(&snapshot, *id))
        .collect::<Vec<_>>();

    let code = phrasing
        .iter()
        .find(|node| matches!(node.content, ContentKind::InlineCode { .. }))
        .expect("inline code must exist");
    assert_eq!(slice(source, code.source), "` 代码 `");
    assert_eq!(slice(source, code.body), " 代码 ");
    let ContentKind::InlineCode { text } = &code.content else {
        unreachable!();
    };
    assert_eq!(semantic_value(&snapshot, code, text), "代码");
    assert!(matches!(text, SemanticText::Normalized { .. }));

    let entity = phrasing
        .iter()
        .find(|node| {
            matches!(
                &node.content,
                ContentKind::Text {
                    text: SemanticText::Normalized { value }
                } if value.contains('&')
            )
        })
        .expect("entity text must remain typed");
    assert!(slice(source, entity.source).contains("&amp;"));

    let math = phrasing
        .iter()
        .find(|node| matches!(node.content, ContentKind::Math { display: false, .. }))
        .expect("inline math must exist");
    assert_eq!(slice(source, math.source), "$α + β$");
    assert_eq!(slice(source, math.body), "α + β");

    let html = node(&snapshot, snapshot.roots().as_slice()[1]);
    let ContentKind::Html { block: true, text } = &html.content else {
        panic!("second root must be block HTML");
    };
    assert_eq!(slice(source, html.source), "<div>中🙂</div>\n");
    assert_eq!(semantic_value(&snapshot, html, text), "<div>中🙂</div>\n");

    let fenced = node(&snapshot, snapshot.roots().as_slice()[2]);
    assert!(matches!(fenced.content, ContentKind::CodeBlock { .. }));
    assert_eq!(
        slice(source, fenced.source),
        "~~~~rust linenos\nfn 中() {}\n~~~~"
    );
    assert_eq!(slice(source, fenced.body), "fn 中() {}\n");
}

#[test]
fn closed_inline_nodes_leave_later_source_in_the_explicit_projection_frontier() {
    for inline in ["[x](u)", "*x*", "`x`"] {
        let mut engine = StreamEngine::new();
        let mut reducer = Reducer::new();
        apply_output(&mut reducer, engine.append(inline).unwrap());
        let deferred = engine.append("\n").unwrap();
        assert!(deferred.changes()[0].operations().is_empty());
        apply_output(&mut reducer, deferred);

        let document = reducer.document().unwrap();
        let compiled = SourceCursor::new(inline.len() as u64);
        assert_eq!(document.projection_cursor(), compiled);
        assert_eq!(
            &document.source()[compiled.get() as usize..],
            "\n",
            "uncompiled source must remain directly observable"
        );
        let paragraph = document.node(document.roots().as_slice()[0]).unwrap();
        assert_eq!(slice(document.source(), paragraph.source), inline);
        assert_eq!(slice(document.source(), paragraph.body), inline);
        let inline_node = document.node(paragraph.children.as_slice()[0]).unwrap();
        assert_eq!(slice(document.source(), inline_node.source), inline);

        apply_output(&mut reducer, engine.finish().unwrap());
        let finalized = reducer.document().unwrap();
        assert_eq!(
            finalized.projection_cursor(),
            finalized.coordinate().source_cursor
        );
    }
}

#[test]
fn nested_table_growth_stays_in_the_explicit_projection_frontier_between_checkpoints() {
    let prefix = format!("a | b\n--|--\n{} | {}", "x".repeat(124), "y".repeat(124));
    assert!(prefix.len() >= 256);

    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    apply_output(&mut reducer, engine.append(&prefix).unwrap());

    let deferred = engine.append("z").unwrap();
    assert!(deferred.changes()[0].operations().is_empty());
    apply_output(&mut reducer, deferred);

    let document = reducer.document().unwrap();
    assert_eq!(
        document.projection_cursor(),
        SourceCursor::new(prefix.len() as u64)
    );
    assert_eq!(document.pending_source(), "z");

    apply_output(&mut reducer, engine.finish().unwrap());
    let finalized = reducer.document().unwrap();
    assert_eq!(
        finalized.projection_cursor(),
        finalized.coordinate().source_cursor
    );
}

#[test]
fn markdown_structure_suffix_stays_pending_until_recompiled() {
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    apply_output(&mut reducer, engine.append("a *b").unwrap());

    let deferred = engine.append("*").unwrap();
    assert!(deferred.changes()[0].operations().is_empty());
    apply_output(&mut reducer, deferred);

    let document = reducer.document().unwrap();
    assert_eq!(document.projection_cursor(), SourceCursor::new(4));
    assert_eq!(document.pending_source(), "*");

    apply_output(&mut reducer, engine.finish().unwrap());
    assert!(
        reducer
            .document()
            .unwrap()
            .nodes()
            .any(|node| matches!(node.content, ContentKind::Emphasis {}))
    );
}

#[test]
fn first_non_whitespace_after_an_empty_projection_is_compiled_immediately() {
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    apply_output(&mut reducer, engine.append(" ").unwrap());
    apply_output(&mut reducer, engine.append("a").unwrap());

    let document = reducer.document().unwrap();
    assert_eq!(document.projection_cursor(), SourceCursor::new(2));
    assert_eq!(document.pending_source(), "");
    let root = document.node(document.roots().as_slice()[0]).unwrap();
    assert!(matches!(root.content, ContentKind::Paragraph {}));
}
