use mdstream_protocol::{LinkStyle, SemanticText, SourceCursor};

use crate::compiler::{DraftContentKind, DraftOriginHint, DraftResourceRole, SyntheticRole};

use super::parser::compile_markdown;

#[test]
fn compiles_blocks_and_nested_phrasing() {
    let source = "# Hello *em* **strong** ~~gone~~\n\nParagraph.";
    let forest = compile_markdown(source, SourceCursor::new(0)).unwrap();

    assert_eq!(forest.roots.len(), 2);
    assert!(matches!(
        forest.roots[0].content,
        DraftContentKind::Heading { level: 1 }
    ));
    assert!(
        forest.roots[0]
            .children
            .iter()
            .any(|node| matches!(node.content, DraftContentKind::Emphasis))
    );
    assert!(
        forest.roots[0]
            .children
            .iter()
            .any(|node| matches!(node.content, DraftContentKind::Strong))
    );
    assert!(
        forest.roots[0]
            .children
            .iter()
            .any(|node| matches!(node.content, DraftContentKind::Strikethrough))
    );
}

#[test]
fn synthesizes_tight_paragraphs_and_records_task_markers() {
    let forest = compile_markdown("- [x] done\n- [ ] later\n", SourceCursor::new(0)).unwrap();
    let list = &forest.roots[0];

    assert!(matches!(
        list.content,
        DraftContentKind::List { tight: true, .. }
    ));
    assert!(matches!(
        list.children[0].content,
        DraftContentKind::ListItem {
            checked: Some(true)
        }
    ));
    assert_eq!(
        list.children[0].children[0].origin,
        DraftOriginHint::Synthetic(SyntheticRole::TightParagraph)
    );
}

#[test]
fn synthesizes_protocol_table_sections_and_columns() {
    let source = "a | b\n--|:--:\n1 | 2\n";
    let forest = compile_markdown(source, SourceCursor::new(0)).unwrap();
    let table = &forest.roots[0];

    assert!(matches!(table.content, DraftContentKind::Table { .. }));
    assert!(matches!(
        table.children[0].content,
        DraftContentKind::TableHead
    ));
    assert_eq!(
        table.children[0].children[0].origin,
        DraftOriginHint::Synthetic(SyntheticRole::TableHeaderRow)
    );
    assert_eq!(
        table.children[1].origin,
        DraftOriginHint::Synthetic(SyntheticRole::TableBody)
    );
    assert!(matches!(
        table.children[1].children[0].children[1].content,
        DraftContentKind::TableCell { column: 1 }
    ));
}

#[test]
fn code_and_html_collectors_are_leaf_nodes() {
    let source = "> ```mermaid theme=dark\n> graph TD\n> A-->B\n> ```\n\n<div>\nraw\n</div>\n";
    let forest = compile_markdown(source, SourceCursor::new(0)).unwrap();
    let code = &forest.roots[0].children[0];
    let html = &forest.roots[1];

    assert!(
        matches!(
            &code.content,
            DraftContentKind::CodeBlock {
                info: Some(info),
                text: SemanticText::Normalized { value },
                ..
            } if info == "mermaid theme=dark" && value == "graph TD\nA-->B\n"
        ),
        "{code:#?}"
    );
    assert!(code.children.is_empty());
    assert!(matches!(
        html.content,
        DraftContentKind::Html { block: true, .. }
    ));
    assert!(html.children.is_empty());
}

#[test]
fn unresolved_and_collapsed_references_remain_typed() {
    let source = "[missing][] and [known][]\n\n[known]: /target \"title\"\n";
    let forest = compile_markdown(source, SourceCursor::new(0)).unwrap();
    let links = forest.roots[0]
        .children
        .iter()
        .filter(|node| matches!(node.content, DraftContentKind::Link { .. }))
        .collect::<Vec<_>>();

    assert!(matches!(
        links[0].content,
        DraftContentKind::Link {
            target: None,
            style: LinkStyle::CollapsedUnknown,
            ..
        }
    ));
    assert!(matches!(
        links[1].content,
        DraftContentKind::Link {
            target: Some(_),
            style: LinkStyle::Collapsed,
            ..
        }
    ));
    assert_eq!(forest.resources.len(), 1);
    assert_eq!(forest.resources[0].key.role, DraftResourceRole::Link);
    assert!(
        source
            .get(
                usize::try_from(links[0].source.start.get()).unwrap()
                    ..usize::try_from(links[0].source.end.get()).unwrap()
            )
            .unwrap()
            .ends_with("[]")
    );
}

#[test]
fn fragment_resources_preserve_usage_labels_before_document_semantics() {
    let source = concat!(
        "[a][Straße] [b][STRASSE] [c](https://example.test) ",
        "[d](https://example.test)\n\n",
        "[straße]: https://example.test\n",
    );
    let forest = compile_markdown(source, SourceCursor::new(0)).unwrap();

    assert_eq!(forest.resources.len(), 4);
    assert_eq!(
        forest.resources[0].key.reference_label.as_deref(),
        Some("Straße")
    );
    assert_eq!(
        forest.resources[1].key.reference_label.as_deref(),
        Some("STRASSE")
    );
    assert!(forest.resources[2].key.reference_label.is_none());
    assert!(forest.resources[3].key.reference_label.is_none());
}

#[test]
fn semantic_text_distinguishes_source_from_normalized_values() {
    let forest = compile_markdown("plain &amp; \\*star", SourceCursor::new(0)).unwrap();
    let paragraph = &forest.roots[0];
    let texts = paragraph
        .children
        .iter()
        .filter_map(|node| match &node.content {
            DraftContentKind::Text { text } => Some(text),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(texts
        .iter()
        .any(|text| matches!(text, SemanticText::Normalized { value } if value.contains('&') || value.contains('*'))));
}

#[test]
fn maps_math_footnotes_breaks_rules_and_images() {
    let source = "![a *b*](img.png) $x$  \nnext[^n]\n\n---\n\n[^n]: note\n";
    let forest = compile_markdown(source, SourceCursor::new(0)).unwrap();
    let paragraph = &forest.roots[0];

    assert!(matches!(
        paragraph.children[0].content,
        DraftContentKind::Image { .. }
    ));
    assert!(
        paragraph
            .children
            .iter()
            .any(|node| matches!(node.content, DraftContentKind::Math { display: false, .. }))
    );
    assert!(
        paragraph
            .children
            .iter()
            .any(|node| matches!(node.content, DraftContentKind::HardBreak))
    );
    assert!(
        forest
            .roots
            .iter()
            .any(|node| matches!(node.content, DraftContentKind::ThematicBreak))
    );
    assert!(
        forest
            .roots
            .iter()
            .any(|node| matches!(node.content, DraftContentKind::FootnoteDefinition { .. }))
    );
}

#[test]
fn offsets_are_absolute_utf8_byte_ranges() {
    let source = "# cafe\u{301} 世界";
    let base = SourceCursor::new(900);
    let forest = compile_markdown(source, base).unwrap();
    let heading = &forest.roots[0];

    assert_eq!(heading.source.start, base);
    assert_eq!(
        heading.source.end,
        SourceCursor::new(900 + u64::try_from(source.len()).unwrap())
    );
    assert_eq!(heading.children[0].source.start, SourceCursor::new(902));
}

#[test]
fn heading_ranges_exclude_only_the_terminal_line_ending() {
    let cases = [
        ("# h\n", "# h"),
        ("# h", "# h"),
        ("###\n", "###"),
        ("# h ###\n", "# h ###"),
        ("h\n===\n", "h\n==="),
        ("# h\r\n", "# h"),
    ];

    for (source, expected) in cases {
        let forest = compile_markdown(source, SourceCursor::new(0)).unwrap();
        let heading = &forest.roots[0];
        let end = usize::try_from(heading.source.end.get()).unwrap();
        assert_eq!(&source[..end], expected, "source: {source:?}");
    }
}
