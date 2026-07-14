use mdstream::{CompilerError, CustomBlockSpec, EngineError, EngineOutput, StreamEngine};
use mdstream_protocol::{
    ApplyOutcome, CodeBlockSyntax, CodeFenceMarker, ContentKind, ContentNode, NodeId,
    NodeStability, ProtocolLimits, Reducer, SemanticText, Snapshot, TableAlignment,
};

fn apply_output(reducer: &mut Reducer, output: EngineOutput) {
    for change in output.into_changes() {
        let outcome = reducer.apply(change).expect("engine output must replay");
        assert!(matches!(outcome, ApplyOutcome::Applied { .. }));
    }
}

fn compile(source: &str) -> Snapshot {
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    let output = engine.append(source).expect("append must succeed");
    apply_output(&mut reducer, output);
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

fn roots(snapshot: &Snapshot) -> Vec<&ContentNode> {
    snapshot
        .roots()
        .iter()
        .map(|id| node(snapshot, *id))
        .collect()
}

fn children<'snapshot>(
    snapshot: &'snapshot Snapshot,
    owner: &ContentNode,
) -> Vec<&'snapshot ContentNode> {
    owner
        .children
        .iter()
        .map(|id| node(snapshot, *id))
        .collect()
}

fn semantic_value(snapshot: &Snapshot, owner: &ContentNode, text: &SemanticText) -> String {
    match text {
        SemanticText::Source {} => {
            let start = usize::try_from(owner.body.start.get()).unwrap();
            let end = usize::try_from(owner.body.end.get()).unwrap();
            snapshot.source()[start..end].to_string()
        }
        SemanticText::Normalized { value } => value.clone(),
    }
}

#[test]
fn heading_phrasing_and_resources_compile_to_typed_topology() {
    let snapshot = compile("# Hello *world* and [docs](https://example.test \"Docs\")");
    let document_roots = roots(&snapshot);
    assert_eq!(document_roots.len(), 1);
    let heading = document_roots[0];
    assert!(matches!(heading.content, ContentKind::Heading { level: 1 }));

    let phrasing = children(&snapshot, heading);
    assert_eq!(phrasing.len(), 4);
    assert!(matches!(phrasing[0].content, ContentKind::Text { .. }));
    assert!(matches!(phrasing[1].content, ContentKind::Emphasis {}));
    assert!(matches!(phrasing[2].content, ContentKind::Text { .. }));
    assert!(matches!(phrasing[3].content, ContentKind::Link { .. }));

    let emphasis = children(&snapshot, phrasing[1]);
    assert_eq!(emphasis.len(), 1);
    let ContentKind::Text { text } = &emphasis[0].content else {
        panic!("emphasis must own text");
    };
    assert_eq!(semantic_value(&snapshot, emphasis[0], text), "world");

    let link = phrasing[3];
    let ContentKind::Link { target, .. } = &link.content else {
        panic!("fourth phrasing node must be a link");
    };
    let target = target.as_ref().expect("inline link must resolve");
    let resource = snapshot
        .resources()
        .iter()
        .find(|resource| resource.id == target.id)
        .expect("link resource must exist");
    assert_eq!(resource.version, target.version);
    assert!(matches!(
        &resource.content,
        mdstream_protocol::SemanticResourceKind::Link { destination, title }
            if destination == "https://example.test" && title.as_deref() == Some("Docs")
    ));
}

#[test]
fn tight_lists_and_tables_expose_canonical_synthetic_containers() {
    let snapshot = compile("- alpha\n- beta\n\n| Name | Score |\n| --- | ---: |\n| Ada | 10 |\n");
    let document_roots = roots(&snapshot);
    assert_eq!(document_roots.len(), 2);

    let list = document_roots[0];
    assert!(matches!(
        list.content,
        ContentKind::List {
            ordered: false,
            start: None,
            tight: true
        }
    ));
    let items = children(&snapshot, list);
    assert_eq!(items.len(), 2);
    for item in items {
        assert!(matches!(item.content, ContentKind::ListItem { .. }));
        let item_blocks = children(&snapshot, item);
        assert_eq!(item_blocks.len(), 1);
        assert!(matches!(item_blocks[0].content, ContentKind::Paragraph {}));
        assert!(matches!(
            children(&snapshot, item_blocks[0]).as_slice(),
            [ContentNode {
                content: ContentKind::Text { .. },
                ..
            }]
        ));
    }

    let table = document_roots[1];
    assert!(matches!(
        &table.content,
        ContentKind::Table { alignments }
            if alignments == &[TableAlignment::None, TableAlignment::Right]
    ));
    let sections = children(&snapshot, table);
    assert_eq!(sections.len(), 2);
    assert!(matches!(sections[0].content, ContentKind::TableHead {}));
    assert!(matches!(sections[1].content, ContentKind::TableBody {}));

    let head_rows = children(&snapshot, sections[0]);
    let body_rows = children(&snapshot, sections[1]);
    assert_eq!(head_rows.len(), 1, "table heads synthesize one row");
    assert_eq!(body_rows.len(), 1);
    for row in [head_rows[0], body_rows[0]] {
        assert!(matches!(row.content, ContentKind::TableRow {}));
        let cells = children(&snapshot, row);
        assert_eq!(cells.len(), 2);
        assert!(matches!(
            cells[0].content,
            ContentKind::TableCell { column: 0 }
        ));
        assert!(matches!(
            cells[1].content,
            ContentKind::TableCell { column: 1 }
        ));
    }
}

#[test]
fn html_code_and_display_math_are_renderer_neutral_typed_leaves() {
    let snapshot =
        compile("<div>raw</div>\n\n~~~~rust linenos\nfn main() {}\n~~~~\n\nbefore $$x + y$$ after");
    let document_roots = roots(&snapshot);
    assert_eq!(document_roots.len(), 3);

    let html = document_roots[0];
    let ContentKind::Html { block, text } = &html.content else {
        panic!("first root must be HTML");
    };
    assert!(*block);
    assert_eq!(semantic_value(&snapshot, html, text), "<div>raw</div>\n");
    assert!(html.children.is_empty());

    let code = document_roots[1];
    let ContentKind::CodeBlock { syntax, info, text } = &code.content else {
        panic!("second root must be a code block");
    };
    assert_eq!(
        *syntax,
        CodeBlockSyntax::Fenced {
            marker: CodeFenceMarker::Tilde,
            length: 4
        }
    );
    assert_eq!(code.content.code_language(), Some("rust"));
    assert_eq!(info.as_deref(), Some("rust linenos"));
    assert_eq!(semantic_value(&snapshot, code, text), "fn main() {}\n");
    assert!(code.children.is_empty());

    let paragraph = document_roots[2];
    assert!(matches!(paragraph.content, ContentKind::Paragraph {}));
    let phrasing = children(&snapshot, paragraph);
    assert!(phrasing.iter().any(|node| {
        matches!(
            &node.content,
            ContentKind::Math { display: true, text }
                if semantic_value(&snapshot, node, text) == "x + y"
        )
    }));
}

#[test]
fn configured_html_blocks_compile_to_namespaced_custom_nodes() {
    let mut engine = StreamEngine::builder()
        .custom_block(CustomBlockSpec::try_new("app.custom/1", "thinking").unwrap())
        .build()
        .unwrap();
    let mut reducer = Reducer::new();
    apply_output(
        &mut reducer,
        engine
            .append("<thinking role=analysis enabled=\"true\">\nsecret\n</thinking>")
            .unwrap(),
    );
    apply_output(&mut reducer, engine.finish().unwrap());

    let snapshot = reducer.document().unwrap().snapshot();
    let custom = snapshot
        .nodes()
        .iter()
        .find(|node| matches!(node.content, ContentKind::Custom { .. }))
        .expect("configured block must become a custom node");
    let ContentKind::Custom {
        namespace,
        name,
        opaque,
        attributes,
    } = &custom.content
    else {
        unreachable!();
    };
    assert_eq!(namespace, "app.custom/1");
    assert_eq!(name, "thinking");
    assert!(*opaque);
    assert_eq!(attributes.get("role").map(String::as_str), Some("analysis"));
    assert_eq!(attributes.get("enabled").map(String::as_str), Some("true"));
    assert!(custom.children.is_empty());
}

#[test]
fn custom_attribute_limits_fail_atomically_and_allow_a_valid_retry() {
    let limits = ProtocolLimits {
        max_attributes_per_node: 1,
        ..ProtocolLimits::default()
    };
    let mut engine = StreamEngine::builder()
        .protocol_limits(limits)
        .custom_block(CustomBlockSpec::try_new("x", "thinking").unwrap())
        .build()
        .unwrap();

    engine.append("# stable\n\n").unwrap();
    let before = engine.snapshot().unwrap();
    let before_metrics = engine.metrics();

    assert!(matches!(
        engine.append("<thinking a=1 b=2>\nrejected\n</thinking>"),
        Err(EngineError::Compiler(CompilerError::LimitExceeded {
            field: "custom.attributes",
            limit: 1,
            actual: 2,
        }))
    ));
    assert_eq!(engine.snapshot().unwrap(), before);
    assert_eq!(engine.metrics(), before_metrics);

    engine
        .append("<thinking a=1>\naccepted\n</thinking>")
        .expect("a retry within the attribute budget must succeed");
}

#[test]
fn malformed_custom_attributes_remain_machine_classifiable() {
    for (source, expected) in [
        (
            "<thinking =x>\nbody\n</thinking>",
            mdstream::MarkdownDiagnostic::InvalidCustomAttributeName,
        ),
        (
            "<thinking x=>\nbody\n</thinking>",
            mdstream::MarkdownDiagnostic::InvalidCustomAttributeValue,
        ),
        (
            "<thinking x=1 x=2>\nbody\n</thinking>",
            mdstream::MarkdownDiagnostic::DuplicateCustomAttribute,
        ),
    ] {
        let mut engine = StreamEngine::builder()
            .custom_block(CustomBlockSpec::try_new("x", "thinking").unwrap())
            .build()
            .unwrap();
        engine.append("# stable\n\n").unwrap();
        let before = engine.snapshot().unwrap();
        let before_metrics = engine.metrics();

        let error = engine.append(source).unwrap_err();
        let EngineError::Compiler(CompilerError::Markdown(actual)) = error else {
            panic!("malformed attributes must return a Markdown diagnostic: {error:?}");
        };
        assert_eq!(actual, expected, "source={source:?}");
        assert_eq!(engine.snapshot().unwrap(), before);
        assert_eq!(engine.metrics(), before_metrics);

        engine
            .append("<thinking x=valid>\nbody\n</thinking>")
            .expect("a valid retry after a rejected attribute must succeed");
    }
}

#[test]
fn configured_html_blocks_preserve_blank_lines_inside_one_custom_node() {
    let source = "<thinking>\nfirst\n\nsecond\n</thinking>";
    let mut engine = StreamEngine::builder()
        .custom_block(CustomBlockSpec::try_new("app.custom/1", "thinking").unwrap())
        .build()
        .unwrap();
    let mut reducer = Reducer::new();
    apply_output(&mut reducer, engine.append(source).unwrap());
    apply_output(&mut reducer, engine.finish().unwrap());

    let snapshot = reducer.document().unwrap().snapshot();
    let custom_nodes = snapshot
        .nodes()
        .iter()
        .filter(|node| matches!(node.content, ContentKind::Custom { .. }))
        .collect::<Vec<_>>();
    assert_eq!(custom_nodes.len(), 1);
    assert_eq!(custom_nodes[0].source.start.get(), 0);
    assert_eq!(
        custom_nodes[0].source.end.get(),
        u64::try_from(source.len()).unwrap()
    );
}

#[test]
fn configured_html_blocks_parse_quoted_delimiters_and_empty_attributes() {
    let mut engine = StreamEngine::builder()
        .custom_block(CustomBlockSpec::try_new("app.custom/1", "thinking").unwrap())
        .build()
        .unwrap();
    let mut reducer = Reducer::new();
    apply_output(
        &mut reducer,
        engine
            .append("<thinking title=\"a > b\" empty=\"\">\nsecret\n</thinking>")
            .unwrap(),
    );
    apply_output(&mut reducer, engine.finish().unwrap());

    let snapshot = reducer.document().unwrap().snapshot();
    let attributes = snapshot
        .nodes()
        .iter()
        .find_map(|node| match &node.content {
            ContentKind::Custom { attributes, .. } => Some(attributes),
            _ => None,
        })
        .expect("configured block must become a custom node");
    assert_eq!(attributes.get("title").map(String::as_str), Some("a > b"));
    assert_eq!(attributes.get("empty").map(String::as_str), Some(""));
}

#[test]
fn custom_block_case_policy_and_normalized_utf8_body_ranges_are_exact() {
    let exact_spec = || {
        CustomBlockSpec::try_new("app.custom/1", "thinking")
            .unwrap()
            .case_insensitive(false)
    };
    let mut mixed_case = StreamEngine::builder()
        .custom_block(exact_spec())
        .build()
        .unwrap();
    mixed_case
        .append("<THINKING>\nplain HTML\n</THINKING>")
        .unwrap();
    mixed_case.finish().unwrap();
    assert!(
        !mixed_case
            .snapshot()
            .unwrap()
            .nodes()
            .iter()
            .any(|node| matches!(node.content, ContentKind::Custom { .. }))
    );

    let mut exact_case = StreamEngine::builder()
        .custom_block(exact_spec())
        .build()
        .unwrap();
    for chunk in [
        "<thinking role=\"\u{3bb}\">\r",
        "\n\u{3c0}\r",
        "\n</thinking>",
    ] {
        exact_case.append(chunk).unwrap();
    }
    exact_case.finish().unwrap();

    let snapshot = exact_case.snapshot().unwrap();
    let custom = snapshot
        .nodes()
        .iter()
        .find(|node| matches!(node.content, ContentKind::Custom { .. }))
        .expect("exact-case custom block must be classified");
    let body_start = usize::try_from(custom.body.start.get()).unwrap();
    let body_end = usize::try_from(custom.body.end.get()).unwrap();
    assert_eq!(&snapshot.source()[body_start..body_end], "\n\u{3c0}\n");
    assert_eq!(custom.source.start.get(), 0);
    assert_eq!(custom.source.end.get(), snapshot.source().len() as u64);
}

#[test]
fn non_opaque_custom_blocks_compile_their_markdown_children() {
    let mut engine = StreamEngine::builder()
        .custom_block(
            CustomBlockSpec::try_new("app.custom/1", "thinking")
                .unwrap()
                .opaque(false),
        )
        .build()
        .unwrap();
    let mut reducer = Reducer::new();
    apply_output(
        &mut reducer,
        engine
            .append("<thinking>\n**visible**\n</thinking>")
            .unwrap(),
    );
    apply_output(&mut reducer, engine.finish().unwrap());

    let snapshot = reducer.document().unwrap().snapshot();
    let custom = snapshot
        .nodes()
        .iter()
        .find(|node| matches!(node.content, ContentKind::Custom { opaque: false, .. }))
        .expect("configured block must become a non-opaque custom node");
    let descendants = children(&snapshot, custom);
    assert!(
        descendants
            .iter()
            .any(|node| matches!(node.content, ContentKind::Paragraph {}))
    );
    assert!(snapshot.nodes().iter().any(|node| {
        matches!(node.content, ContentKind::Strong {})
            && node.source.start.get() >= custom.body.start.get()
            && node.source.end.get() <= custom.body.end.get()
    }));
}

#[test]
fn non_opaque_custom_recursion_respects_the_effective_tree_depth_limit() {
    let limits = ProtocolLimits {
        max_tree_depth: 2,
        ..ProtocolLimits::default()
    };
    let spec = || {
        CustomBlockSpec::try_new("app.custom/1", "thinking")
            .unwrap()
            .opaque(false)
    };
    let mut exact = StreamEngine::builder()
        .protocol_limits(limits)
        .custom_block(spec())
        .build()
        .unwrap();
    exact
        .append("<thinking>\n\n<thinking>\n</thinking>\n</thinking>")
        .expect("custom depth exactly at the limit must be accepted");

    let mut engine = StreamEngine::builder()
        .protocol_limits(limits)
        .custom_block(spec())
        .build()
        .unwrap();
    let source = concat!(
        "<thinking>\n",
        "\n",
        "<thinking>\n",
        "\n",
        "<thinking>\n",
        "inside\n",
        "</thinking>\n",
        "</thinking>\n",
        "</thinking>",
    );

    assert!(matches!(
        engine.append(source),
        Err(EngineError::Compiler(CompilerError::LimitExceeded {
            field: "tree.depth",
            limit: 2,
            actual: 3,
        }))
    ));
    assert!(engine.snapshot().is_none());
}

#[test]
fn custom_block_closing_tags_inside_fenced_code_are_opaque_to_pairing() {
    for source in [
        "<thinking>\n```text\n</thinking>\n```\nafter\n</thinking>",
        "<thinking>\n`</thinking>`\nafter\n</thinking>",
        "<thinking>\n\n    </thinking>\nafter\n</thinking>",
        "<thinking>\n> ```text\n> </thinking>\n> ```\nafter\n</thinking>",
        "<thinking>\n<!--\n```\n</thinking>\n-->\nafter\n</thinking>",
    ] {
        let mut engine = StreamEngine::builder()
            .custom_block(CustomBlockSpec::try_new("app.custom/1", "thinking").unwrap())
            .build()
            .unwrap();
        let mut reducer = Reducer::new();
        apply_output(&mut reducer, engine.append(source).unwrap());
        apply_output(&mut reducer, engine.finish().unwrap());

        let snapshot = reducer.document().unwrap().snapshot();
        let custom = snapshot
            .nodes()
            .iter()
            .find(|node| matches!(node.content, ContentKind::Custom { .. }))
            .expect("outer configured block must remain paired");
        assert_eq!(custom.source.start.get(), 0, "source={source:?}");
        assert_eq!(
            custom.source.end.get(),
            source.len() as u64,
            "source={source:?}"
        );
        assert_eq!(
            custom.body.end.get(),
            source.rfind("</thinking>").unwrap() as u64,
            "source={source:?}"
        );
    }
}

#[test]
fn split_fence_closing_line_does_not_reopen_inside_a_custom_block() {
    for opaque in [true, false] {
        let build = || {
            StreamEngine::builder()
                .custom_block(
                    CustomBlockSpec::try_new("app.custom/1", "thinking")
                        .unwrap()
                        .opaque(opaque),
                )
                .build()
                .unwrap()
        };
        let mut engine = build();
        let prefix = "<thinking>\n```text\nbody\n````";

        engine.append(prefix).unwrap();
        engine.append("\n").unwrap();
        engine.append("</thinking>\n").unwrap();

        let snapshot = engine.snapshot().unwrap();
        let custom = snapshot
            .nodes()
            .iter()
            .find(|node| matches!(node.content, ContentKind::Custom { .. }))
            .expect("the custom block must remain projected");
        assert_eq!(custom.stability, NodeStability::Stable, "opaque={opaque}");
        assert_eq!(custom.body.end.get(), (prefix.len() + 1) as u64);
        assert_eq!(
            custom.source.end.get(),
            (prefix.len() + 1 + "</thinking>".len()) as u64,
        );

        let mut bytewise = build();
        let prefix = "<thinking>\n```text\nbody\n";
        bytewise.append(prefix).unwrap();
        for marker in ["`", "`", "`"] {
            bytewise.append(marker).unwrap();
        }
        bytewise.append("\n</thinking>\n").unwrap();

        let snapshot = bytewise.snapshot().unwrap();
        let custom = snapshot
            .nodes()
            .iter()
            .find(|node| matches!(node.content, ContentKind::Custom { .. }))
            .expect("the bytewise fence must retain the custom block");
        assert_eq!(custom.stability, NodeStability::Stable, "opaque={opaque}");
        assert_eq!(custom.body.end.get(), (prefix.len() + 4) as u64);
    }
}

#[test]
fn inline_html_cannot_turn_embedded_custom_text_into_a_delimiter() {
    for tag in ["script", "style", "pre", "textarea"] {
        for opaque in [true, false] {
            let source = format!(
                "<thinking>\nprefix <{tag}>text </thinking></{tag}> suffix\nafter\n</thinking>"
            );
            let mut engine = StreamEngine::builder()
                .custom_block(
                    CustomBlockSpec::try_new("app.custom/1", "thinking")
                        .unwrap()
                        .opaque(opaque),
                )
                .build()
                .unwrap();

            engine.append(&source).unwrap();
            engine.finish().unwrap();

            let snapshot = engine.snapshot().unwrap();
            let custom = snapshot
                .nodes()
                .iter()
                .find(|node| matches!(node.content, ContentKind::Custom { .. }))
                .expect("the outer custom block must remain classified");
            assert_eq!(
                custom.source.end.get(),
                source.len() as u64,
                "tag={tag} opaque={opaque}"
            );
            assert_eq!(
                custom.body.end.get(),
                source.rfind("</thinking>").unwrap() as u64,
                "tag={tag} opaque={opaque}"
            );
        }
    }
}

#[test]
fn inline_script_does_not_hide_a_following_standalone_custom_closer() {
    let prefix = "<thinking>\nprefix <script>\n";
    for opaque in [true, false] {
        let mut engine = StreamEngine::builder()
            .custom_block(
                CustomBlockSpec::try_new("app.custom/1", "thinking")
                    .unwrap()
                    .opaque(opaque),
            )
            .build()
            .unwrap();

        engine.append(prefix).unwrap();
        engine.append("</thinking>\n").unwrap();
        let snapshot = engine.snapshot().unwrap();
        let custom = snapshot
            .nodes()
            .iter()
            .find(|node| matches!(node.content, ContentKind::Custom { .. }))
            .expect("the standalone closer must retain the outer custom block");

        assert_eq!(custom.stability, NodeStability::Stable, "opaque={opaque}");
        assert_eq!(
            custom.body.end.get(),
            prefix.len() as u64,
            "opaque={opaque}"
        );
        assert_eq!(
            custom.source.end.get(),
            (prefix.len() + "</thinking>".len()) as u64,
            "opaque={opaque}",
        );
    }
}

#[test]
fn configured_custom_blocks_do_not_escape_markdown_or_html_containers() {
    for source in [
        "- item\n\n  <thinking>\n  secret\n  </thinking>\n",
        "<!--\n<thinking>\nsecret\n</thinking>\n-->\n",
    ] {
        let mut engine = StreamEngine::builder()
            .custom_block(CustomBlockSpec::try_new("app.custom/1", "thinking").unwrap())
            .build()
            .unwrap();
        let mut reducer = Reducer::new();
        apply_output(&mut reducer, engine.append(source).unwrap());
        apply_output(&mut reducer, engine.finish().unwrap());

        assert!(
            !reducer
                .document()
                .unwrap()
                .nodes()
                .any(|node| matches!(node.content, ContentKind::Custom { .. })),
            "nested tag must remain owned by its CommonMark container: {source:?}"
        );
    }
}

#[test]
fn unclosed_custom_block_is_provisional_and_keeps_identity_when_closed() {
    let mut engine = StreamEngine::builder()
        .custom_block(CustomBlockSpec::try_new("app.custom/1", "thinking").unwrap())
        .build()
        .unwrap();
    let mut reducer = Reducer::new();
    apply_output(
        &mut reducer,
        engine.append("<thinking>\nsecret\n\n").unwrap(),
    );

    let pending = reducer
        .document()
        .unwrap()
        .nodes()
        .find(|node| matches!(node.content, ContentKind::Custom { .. }))
        .expect("complete opening tag must identify a provisional custom block");
    let pending_id = pending.id;
    assert_eq!(pending.stability, NodeStability::Provisional);

    apply_output(&mut reducer, engine.append("more\n</thinking>").unwrap());
    let tentative = reducer
        .document()
        .unwrap()
        .nodes()
        .find(|node| matches!(node.content, ContentKind::Custom { .. }))
        .expect("closing tag must keep the custom projection");
    assert_eq!(tentative.id, pending_id);
    assert_eq!(tentative.stability, NodeStability::Provisional);

    apply_output(&mut reducer, engine.append("\n").unwrap());
    let closed = reducer
        .document()
        .unwrap()
        .nodes()
        .find(|node| matches!(node.content, ContentKind::Custom { .. }))
        .expect("a physical line ending must confirm the custom closing tag");
    assert_eq!(closed.id, pending_id);
    assert_eq!(closed.stability, NodeStability::Stable);
    assert_eq!(
        closed.source.end.get(),
        "<thinking>\nsecret\n\nmore\n</thinking>".len() as u64
    );
    assert_eq!(
        closed.body.end.get(),
        "<thinking>\nsecret\n\nmore\n".len() as u64
    );

    let document = reducer.document().unwrap();
    assert_eq!(
        document.projection_cursor(),
        document.coordinate().source_cursor
    );
    assert!(document.pending_source().is_empty());
}

#[test]
fn different_name_nested_custom_closers_follow_the_authoritative_stack() {
    let mut engine = StreamEngine::builder()
        .custom_block(
            CustomBlockSpec::try_new("app.outer/1", "outer")
                .unwrap()
                .opaque(false),
        )
        .custom_block(
            CustomBlockSpec::try_new("app.inner/1", "inner")
                .unwrap()
                .opaque(false),
        )
        .build()
        .unwrap();

    engine.append("<outer>\n\n").unwrap();
    engine.append("<inner>\nbody\n").unwrap();
    let before_mismatch = engine.metrics().compiler.parse_passes;
    engine.append("</outer>\n").unwrap();
    assert_eq!(
        engine.metrics().compiler.parse_passes,
        before_mismatch,
        "a non-stack-top closer must not force canonical recompilation"
    );
    engine.append("</inner>\n").unwrap();
    engine.append("</outer>\n").unwrap();

    let snapshot = engine.snapshot().unwrap();
    let custom = snapshot
        .nodes()
        .iter()
        .filter(|node| matches!(node.content, ContentKind::Custom { .. }))
        .collect::<Vec<_>>();
    assert_eq!(custom.len(), 2);
    assert!(
        custom
            .iter()
            .all(|node| node.stability == NodeStability::Stable)
    );
}

#[test]
fn citation_shortcuts_compile_to_typed_references_and_resources() {
    let unresolved = compile("See [@Key]");
    assert!(unresolved.nodes().iter().any(|node| {
        matches!(
            node.content,
            ContentKind::CitationReference {
                ref key,
                target: None
            } if key == "key"
        )
    }));

    let resolved = compile("[@Key]: https://example.test/paper \"Paper\"\n\nSee [@Key]");
    let citation = resolved
        .nodes()
        .iter()
        .find_map(|node| match &node.content {
            ContentKind::CitationReference {
                key,
                target: Some(target),
            } => Some((key, target)),
            _ => None,
        })
        .expect("defined citation must carry a resource reference");
    assert_eq!(citation.0, "key");
    let resource = resolved
        .resources()
        .iter()
        .find(|resource| resource.id == citation.1.id)
        .expect("citation resource must be present");
    assert!(matches!(
        &resource.content,
        mdstream_protocol::SemanticResourceKind::Citation {
            key,
            destination,
            title: Some(title),
            ..
        } if key == "key" && destination == "https://example.test/paper" && title == "Paper"
    ));
}
