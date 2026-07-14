use mdstream::{CustomBlockSpec, StreamEngine};
use mdstream_conformance::NormalizedSnapshot;
use mdstream_protocol::{ContentKind, ProjectionOp};

#[test]
fn late_footnote_definition_corrects_a_typed_unresolved_reference() {
    let mut engine = StreamEngine::new();
    engine.append("note[^Straße]\n\n").unwrap();
    let before = engine.snapshot().unwrap();
    let unresolved = before
        .nodes()
        .iter()
        .find(|node| {
            matches!(
                node.content,
                ContentKind::FootnoteReference { target: None, .. }
            )
        })
        .expect("an unresolved GFM footnote must remain typed");
    let reference_id = unresolved.id;
    let before_version = unresolved.version.clone();

    let output = engine.append("[^STRASSE]: body\n").unwrap();
    assert!(output.changes().iter().any(|change| {
        change.operations().iter().any(|operation| {
            matches!(
                operation,
                ProjectionOp::ReplaceNode { node_id, .. } if *node_id == reference_id
            )
        })
    }));
    let after = engine.snapshot().unwrap();
    let resolved = after
        .nodes()
        .iter()
        .find(|node| node.id == reference_id)
        .expect("the footnote reference must keep its identity");
    assert_ne!(resolved.version, before_version);
    assert!(matches!(
        resolved.content,
        ContentKind::FootnoteReference {
            target: Some(_),
            ..
        }
    ));
}

#[test]
fn unresolved_footnote_overlay_is_chunk_invariant_across_inline_events() {
    let source = "note[^a*b*]\n\n[^a*b*]: body\n";
    let mut whole = StreamEngine::new();
    whole.append(source).unwrap();
    whole.finish().unwrap();

    let mut split = StreamEngine::new();
    split.append("note[^a*b*]\n\n").unwrap();
    split.append("[^a*b*]: body\n").unwrap();
    split.finish().unwrap();

    assert_eq!(
        NormalizedSnapshot::from(split.snapshot().unwrap()),
        NormalizedSnapshot::from(whole.snapshot().unwrap())
    );
}

#[test]
fn unresolved_footnote_overlay_accepts_only_classified_single_line_inline_spans() {
    for label in ["a*b*", "a`b`", "a&amp;b", "a\\*b", "<i>x</i>", "$x$"] {
        let prefix = format!("note[^{label}]\n\n");
        let definition = format!("[^{label}]: body\n");
        let source = format!("{prefix}{definition}");

        let mut whole = StreamEngine::new();
        whole.append(&source).unwrap();
        whole.finish().unwrap();
        assert!(
            whole
                .snapshot()
                .unwrap()
                .nodes()
                .iter()
                .any(|node| { matches!(node.content, ContentKind::FootnoteReference { .. }) }),
            "label {label:?} must be classified by the pinned GFM parser"
        );

        let mut split = StreamEngine::new();
        split.append(&prefix).unwrap();
        split.append(&definition).unwrap();
        split.finish().unwrap();
        assert_eq!(
            NormalizedSnapshot::from(split.snapshot().unwrap()),
            NormalizedSnapshot::from(whole.snapshot().unwrap()),
            "label {label:?}"
        );
    }

    let mut multiline = StreamEngine::new();
    multiline.append("note[^a\nb]\n\n").unwrap();
    multiline.finish().unwrap();
    assert!(
        !multiline
            .snapshot()
            .unwrap()
            .nodes()
            .iter()
            .any(|node| { matches!(node.content, ContentKind::FootnoteReference { .. }) })
    );
}

#[test]
fn unresolved_footnote_overlay_preserves_transparent_custom_topology() {
    let prefix = "<thinking>\nnote[^a*b*]\n</thinking>\n\n";
    let definition = "[^a*b*]: body\n";
    let source = format!("{prefix}{definition}");
    let build = || {
        StreamEngine::builder()
            .custom_block(
                CustomBlockSpec::try_new("app.custom/1", "thinking")
                    .unwrap()
                    .opaque(false),
            )
            .build()
            .unwrap()
    };

    let mut whole = build();
    whole.append(&source).unwrap();
    whole.finish().unwrap();
    let mut split = build();
    split.append(prefix).unwrap();
    split.append(definition).unwrap();
    split.finish().unwrap();

    assert_eq!(
        NormalizedSnapshot::from(split.snapshot().unwrap()),
        NormalizedSnapshot::from(whole.snapshot().unwrap())
    );
    let snapshot = whole.snapshot().unwrap();
    let custom = snapshot
        .nodes()
        .iter()
        .find(|node| matches!(node.content, ContentKind::Custom { .. }))
        .unwrap();
    let reference = snapshot
        .nodes()
        .iter()
        .find(|node| matches!(node.content, ContentKind::FootnoteReference { .. }))
        .unwrap();
    let paragraph = snapshot
        .nodes()
        .iter()
        .find(|node| node.children.iter().any(|id| *id == reference.id))
        .unwrap();
    assert!(custom.children.iter().any(|id| *id == paragraph.id));
}
