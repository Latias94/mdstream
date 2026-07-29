use mdstream::StreamEngine;
use mdstream_protocol::ContentKind;

#[test]
fn typed_citation_definitions_preserve_markdown_container_topology() {
    let mut engine = StreamEngine::new();
    engine.append("> [@paper]: /paper\n").unwrap();
    engine.finish().unwrap();
    let snapshot = engine.snapshot().unwrap();
    let blockquote = snapshot
        .nodes()
        .iter()
        .find(|node| matches!(node.content, ContentKind::BlockQuote { .. }))
        .unwrap();
    let definition = snapshot
        .nodes()
        .iter()
        .find(|node| matches!(node.content, ContentKind::CitationDefinition { .. }))
        .unwrap();
    assert!(blockquote.children.iter().any(|id| *id == definition.id));
    assert!(!snapshot.roots().iter().any(|id| *id == definition.id));
    assert_eq!(blockquote.body, definition.source);
}

#[test]
fn typed_citation_definitions_stay_inside_list_items() {
    let mut engine = StreamEngine::new();
    engine.append("- [@paper]: /paper\n").unwrap();
    engine.finish().unwrap();
    let snapshot = engine.snapshot().unwrap();
    let item = snapshot
        .nodes()
        .iter()
        .find(|node| matches!(node.content, ContentKind::ListItem { .. }))
        .unwrap();
    let definition = snapshot
        .nodes()
        .iter()
        .find(|node| matches!(node.content, ContentKind::CitationDefinition { .. }))
        .unwrap();

    assert!(item.children.iter().any(|id| *id == definition.id));
    assert!(!snapshot.roots().iter().any(|id| *id == definition.id));
    assert_eq!(item.body, definition.source);
}

#[test]
fn typed_citation_definitions_set_footnote_body_to_the_child_hull() {
    let mut engine = StreamEngine::new();
    engine.append("[^note]:\n    [@paper]: /paper\n").unwrap();
    engine.finish().unwrap();
    let snapshot = engine.snapshot().unwrap();
    let footnote = snapshot
        .nodes()
        .iter()
        .find(|node| matches!(node.content, ContentKind::FootnoteDefinition { .. }))
        .unwrap();
    let definition = snapshot
        .nodes()
        .iter()
        .find(|node| matches!(node.content, ContentKind::CitationDefinition { .. }))
        .unwrap();

    assert_eq!(footnote.children.as_slice(), &[definition.id]);
    assert_eq!(footnote.body, definition.source);
}
