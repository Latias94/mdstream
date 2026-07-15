use mdstream::{CustomBlockSpec, EngineOutput, StreamEngine};
use mdstream_conformance::{ChunkSchedule, NormalizedSnapshot};
use mdstream_protocol::{ApplyOutcome, ContentKind, Document, NodeId, NodeStability, Reducer};

fn apply_output(reducer: &mut Reducer, output: EngineOutput) {
    for change in output.into_changes() {
        let outcome = reducer.apply(change).expect("engine output must replay");
        assert!(matches!(outcome, ApplyOutcome::Applied { .. }));
    }
}

fn replay(source: &str, schedule: &ChunkSchedule) -> NormalizedSnapshot {
    replay_with_engine(source, schedule, StreamEngine::new())
}

fn replay_with_engine(
    source: &str,
    schedule: &ChunkSchedule,
    mut engine: StreamEngine,
) -> NormalizedSnapshot {
    let mut reducer = Reducer::new();
    for chunk in schedule.slices(source).expect("valid UTF-8 schedule") {
        apply_output(
            &mut reducer,
            engine.append(chunk).expect("append must succeed"),
        );
    }
    apply_output(&mut reducer, engine.finish().expect("finish must succeed"));
    NormalizedSnapshot::from(
        reducer
            .document()
            .expect("finish must produce a document")
            .snapshot(),
    )
}

#[test]
fn trailing_blank_lines_do_not_change_footnote_identity_by_chunk_schedule() {
    let source = "\n[^note]: footnote body\n\n\n";
    assert_eq!(
        replay(source, &ChunkSchedule::Characters),
        replay(source, &ChunkSchedule::Whole),
    );
}

#[test]
fn html_emphasis_and_link_identity_is_invariant_across_the_first_checkpoint() {
    let source = format!(
        "<aside>streamed html</aside>\n\n{} *important* [docs](https://example.test/docs)\n",
        "checkpoint ".repeat(28)
    );
    assert!(
        source.len() > 256,
        "fixture must cross the first checkpoint"
    );

    let schedules = [
        ChunkSchedule::Whole,
        ChunkSchedule::Characters,
        ChunkSchedule::ByteCuts {
            cuts: vec![1, 255, 256, source.len() - 1],
        },
        ChunkSchedule::Seeded {
            label: "u4.content-identity".to_string(),
            seed: 17,
            trial: 4,
            max_bytes: 73,
        },
    ];
    let baseline = replay(&source, &schedules[0]);

    assert!(
        baseline
            .nodes
            .iter()
            .any(|node| matches!(node.content, ContentKind::Html { .. })),
        "HTML must be represented by typed IR"
    );
    assert!(
        baseline
            .nodes
            .iter()
            .any(|node| matches!(node.content, ContentKind::Emphasis { .. })),
        "emphasis must be represented by typed IR"
    );
    assert!(
        baseline
            .nodes
            .iter()
            .any(|node| matches!(node.content, ContentKind::Link { .. })),
        "links must be represented by typed IR"
    );

    for schedule in &schedules[1..] {
        assert_eq!(
            replay(&source, schedule),
            baseline,
            "final identity and versions must not depend on {schedule:?}"
        );
    }
}

#[test]
fn custom_and_citation_identity_is_invariant_across_chunk_schedules() {
    let source =
        "<thinking role=analysis title=\"a > b\">\nsecret\n\nmore\n</thinking>\n\nSee [@Paper]";
    let schedules = [
        ChunkSchedule::Whole,
        ChunkSchedule::Characters,
        ChunkSchedule::ByteCuts {
            cuts: vec![1, 17, 32, source.len() - 1],
        },
        ChunkSchedule::Seeded {
            label: "u4.custom-citation".to_string(),
            seed: 23,
            trial: 7,
            max_bytes: 11,
        },
    ];
    let engine = || {
        StreamEngine::builder()
            .custom_block(CustomBlockSpec::try_new("app.custom/1", "thinking").unwrap())
            .build()
            .unwrap()
    };
    let baseline = replay_with_engine(source, &schedules[0], engine());
    assert!(
        baseline
            .nodes
            .iter()
            .any(|node| matches!(node.content, ContentKind::Custom { .. }))
    );
    assert!(
        baseline
            .nodes
            .iter()
            .any(|node| { matches!(node.content, ContentKind::CitationReference { .. }) })
    );

    for schedule in &schedules[1..] {
        assert_eq!(
            replay_with_engine(source, schedule, engine()),
            baseline,
            "custom/citation identity must not depend on {schedule:?}"
        );
    }
}

#[test]
fn nested_custom_and_inline_raw_text_identity_is_chunk_invariant() {
    let source = concat!(
        "<thinking>\n",
        "\n",
        "<thinking>\n**nested**\n</thinking>\n",
        "prefix <ScRiPt>fake </thinking></sCrIpT> suffix\n",
        "</thinking>",
    );
    let false_close = source.find("fake </thinking>").unwrap() + "fake ".len();
    let raw_close = source.find("</sCrIpT>").unwrap();
    let schedules = [
        ChunkSchedule::Whole,
        ChunkSchedule::Characters,
        ChunkSchedule::ByteCuts {
            cuts: vec![
                1,
                false_close,
                false_close + "</thinking>".len(),
                raw_close + 4,
                source.len() - 1,
            ],
        },
        ChunkSchedule::Seeded {
            label: "u4.nested-custom-raw-text".to_string(),
            seed: 0x00c0_570a_u64,
            trial: 11,
            max_bytes: 7,
        },
    ];
    let engine = || {
        StreamEngine::builder()
            .custom_block(
                CustomBlockSpec::try_new("app.custom/1", "thinking")
                    .unwrap()
                    .opaque(false),
            )
            .build()
            .unwrap()
    };
    let baseline = replay_with_engine(source, &schedules[0], engine());
    assert_eq!(
        baseline
            .nodes
            .iter()
            .filter(|node| matches!(node.content, ContentKind::Custom { .. }))
            .count(),
        2
    );

    for schedule in &schedules[1..] {
        assert_eq!(
            replay_with_engine(source, schedule, engine()),
            baseline,
            "nested custom/raw-text identity must not depend on {schedule:?}"
        );
    }
}

#[test]
fn markdown_protected_html_openers_inside_custom_are_chunk_invariant() {
    let engine = || {
        StreamEngine::builder()
            .custom_block(CustomBlockSpec::try_new("app.custom/1", "thinking").unwrap())
            .build()
            .unwrap()
    };

    for protected_markdown in ["`<script>`", r"\<script>"] {
        let source = format!("<thinking>\n{protected_markdown}\n</thinking>");
        let html_start = source.find("<script>").unwrap();
        let line_start = source[..html_start].rfind('\n').unwrap() + 1;
        let closer_start = source.rfind("</thinking>").unwrap();
        let schedules = [
            ChunkSchedule::Whole,
            ChunkSchedule::Characters,
            ChunkSchedule::ByteCuts {
                cuts: vec![
                    1,
                    line_start,
                    html_start,
                    html_start + 1,
                    closer_start,
                    source.len() - 1,
                ],
            },
        ];
        let baseline = replay_with_engine(&source, &schedules[0], engine());
        let custom = baseline
            .nodes
            .iter()
            .find(|node| matches!(node.content, ContentKind::Custom { .. }))
            .expect("the outer custom block must remain classified");
        assert_eq!(custom.body.end.get(), closer_start as u64);

        for schedule in &schedules[1..] {
            assert_eq!(
                replay_with_engine(&source, schedule, engine()),
                baseline,
                "Markdown-protected HTML classification changed for {protected_markdown:?} under {schedule:?}",
            );
        }
    }
}

#[test]
fn custom_literal_edge_cases_are_chunk_invariant() {
    let engine = || {
        StreamEngine::builder()
            .custom_block(CustomBlockSpec::try_new("app.custom/1", "thinking").unwrap())
            .custom_block(CustomBlockSpec::try_new("app.script/1", "script").unwrap())
            .build()
            .unwrap()
    };
    let cases = [
        concat!(
            "<thinking>\n<script\n",
            "</script >\n</thinking>\n",
            "</script>\n</thinking>",
        ),
        concat!(
            "<thinking>\n\n<script>\n",
            "</thinking>\n</script>\n</thinking>",
        ),
        "<thinking>\n    <!--\n    code\n</thinking>",
        "<thinking>\n<!-- --> <!--\n</thinking>",
        "<thinking>\n``` info`\n</thinking>",
    ];

    for source in cases {
        let closer_start = source.rfind("</thinking>").unwrap();
        let baseline = replay_with_engine(source, &ChunkSchedule::Whole, engine());
        let custom = baseline
            .nodes
            .iter()
            .find(|node| matches!(node.content, ContentKind::Custom { .. }))
            .expect("the outer custom block must remain classified");
        assert_eq!(
            custom.body.end.get(),
            closer_start as u64,
            "source={source:?}"
        );

        for schedule in [
            ChunkSchedule::Characters,
            ChunkSchedule::ByteCuts {
                cuts: vec![1, source.len() / 3, source.len() / 2, source.len() - 1],
            },
        ] {
            assert_eq!(
                replay_with_engine(source, &schedule, engine()),
                baseline,
                "custom literal state changed for {source:?} under {schedule:?}",
            );
        }
    }
}

#[test]
fn tentative_custom_lines_do_not_leak_chunk_dependent_identity() {
    let engine = || {
        StreamEngine::builder()
            .custom_block(CustomBlockSpec::try_new("app.custom/1", "thinking").unwrap())
            .build()
            .unwrap()
    };

    let invalid_open = "<thinking> trailing\nparagraph";
    let invalid_open_baseline = replay_with_engine(invalid_open, &ChunkSchedule::Whole, engine());
    assert!(
        !invalid_open_baseline
            .nodes
            .iter()
            .any(|node| matches!(node.content, ContentKind::Custom { .. }))
    );
    for schedule in [
        ChunkSchedule::Characters,
        ChunkSchedule::ByteCuts {
            cuts: vec!["<thinking>".len(), invalid_open.len() - 1],
        },
    ] {
        assert_eq!(
            replay_with_engine(invalid_open, &schedule, engine()),
            invalid_open_baseline,
            "invalid tentative opening diverged for {schedule:?}"
        );
    }

    let invalid_close = concat!(
        "<thinking>\n",
        "body\n",
        "</thinking> trailing\n",
        "\n",
        "</thinking>",
    );
    let invalid_close_baseline = replay_with_engine(invalid_close, &ChunkSchedule::Whole, engine());
    let tentative_cut = invalid_close.find("</thinking>").unwrap() + "</thinking>".len();
    for schedule in [
        ChunkSchedule::Characters,
        ChunkSchedule::ByteCuts {
            cuts: vec![1, tentative_cut, invalid_close.len() - 1],
        },
    ] {
        assert_eq!(
            replay_with_engine(invalid_close, &schedule, engine()),
            invalid_close_baseline,
            "invalid tentative closing diverged for {schedule:?}"
        );
    }
}

#[test]
fn custom_opening_context_survives_a_stable_frontier_boundary() {
    struct Case {
        label: &'static str,
        source: &'static str,
        cut: usize,
        expected_custom_nodes: usize,
    }

    let cases = [
        Case {
            label: "heading",
            source: "# heading\n<thinking>\nbody\n</thinking>",
            cut: "# heading\n".len(),
            expected_custom_nodes: 0,
        },
        Case {
            label: "heading-after-blank",
            source: "# heading\n\n<thinking>\nbody\n</thinking>",
            cut: "# heading\n\n".len(),
            expected_custom_nodes: 1,
        },
        Case {
            label: "fence",
            source: "```text\nbody\n```\n<thinking>\nbody\n</thinking>",
            cut: "```text\nbody\n```\n".len(),
            expected_custom_nodes: 0,
        },
        Case {
            label: "closed-custom",
            source: concat!(
                "<thinking>\nfirst\n</thinking>\n",
                "<thinking>\nsecond\n</thinking>",
            ),
            cut: "<thinking>\nfirst\n</thinking>\n".len(),
            expected_custom_nodes: 1,
        },
        Case {
            label: "partial-whitespace-line",
            source: "\n <thinking>\nbody\n</thinking>",
            cut: "\n ".len(),
            expected_custom_nodes: 0,
        },
    ];
    let engine = || {
        StreamEngine::builder()
            .custom_block(CustomBlockSpec::try_new("app.custom/1", "thinking").unwrap())
            .build()
            .unwrap()
    };

    for case in cases {
        let baseline = replay_with_engine(case.source, &ChunkSchedule::Whole, engine());
        assert_eq!(
            baseline
                .nodes
                .iter()
                .filter(|node| matches!(node.content, ContentKind::Custom { .. }))
                .count(),
            case.expected_custom_nodes,
            "{} whole-source classification",
            case.label,
        );
        assert_eq!(
            replay_with_engine(case.source, &ChunkSchedule::Characters, engine()),
            baseline,
            "{} classification changed under character chunks",
            case.label,
        );
        assert_eq!(
            replay_with_engine(
                case.source,
                &ChunkSchedule::ByteCuts {
                    cuts: vec![case.cut],
                },
                engine(),
            ),
            baseline,
            "{} classification changed after frontier advancement",
            case.label,
        );
    }
}

#[test]
fn unfinished_unclaimed_physical_lines_are_chunk_invariant() {
    for (source, cut) in [
        ("# heading\n ", "# heading\n".len()),
        ("---\n ", "---\n".len()),
        ("[shared]: /url", "[shared]".len()),
        ("# heading\n[shared]: /url", "# heading\n".len()),
    ] {
        let baseline = replay(source, &ChunkSchedule::Whole);
        assert_eq!(
            replay(source, &ChunkSchedule::ByteCuts { cuts: vec![cut] }),
            baseline,
        );
        assert_eq!(replay(source, &ChunkSchedule::Characters), baseline);
    }

    let custom_source = "<thinking>\nbody\n</thinking>\n ";
    let engine = || {
        StreamEngine::builder()
            .custom_block(CustomBlockSpec::try_new("app.custom/1", "thinking").unwrap())
            .build()
            .unwrap()
    };
    let baseline = replay_with_engine(custom_source, &ChunkSchedule::Whole, engine());
    assert_eq!(
        replay_with_engine(
            custom_source,
            &ChunkSchedule::ByteCuts {
                cuts: vec!["<thinking>\nbody\n</thinking>\n".len()],
            },
            engine(),
        ),
        baseline,
    );
    assert_eq!(
        replay_with_engine(custom_source, &ChunkSchedule::Characters, engine()),
        baseline,
    );
}

#[test]
fn opaque_same_name_balance_is_chunk_invariant() {
    let source = concat!(
        "<thinking>\n",
        "\n",
        "<thinking>\n",
        "\n",
        "<thinking>\n",
        "inner\n",
        "</thinking>\n",
        "</thinking>\n",
        "</thinking>\n",
        "\n",
        "after",
    );
    let engine = || {
        StreamEngine::builder()
            .custom_block(CustomBlockSpec::try_new("app.custom/1", "thinking").unwrap())
            .build()
            .unwrap()
    };
    let baseline = replay_with_engine(source, &ChunkSchedule::Whole, engine());
    assert_eq!(
        baseline
            .nodes
            .iter()
            .filter(|node| matches!(node.content, ContentKind::Custom { .. }))
            .count(),
        1,
    );

    let first_inner_end = source["<thinking>\n\n".len()..].find("<thinking>").unwrap()
        + "<thinking>\n\n".len()
        + "<thinking>".len();
    for schedule in [
        ChunkSchedule::Characters,
        ChunkSchedule::ByteCuts {
            cuts: vec![
                "<thinking>".len(),
                first_inner_end,
                source.rfind("</thinking>").unwrap() + "</thinking>".len(),
            ],
        },
        ChunkSchedule::Seeded {
            label: "u4.opaque-balance".to_string(),
            seed: 0x000a_11ce_u64,
            trial: 5,
            max_bytes: 9,
        },
    ] {
        assert_eq!(
            replay_with_engine(source, &schedule, engine()),
            baseline,
            "opaque balance changed for {schedule:?}",
        );
    }
}

#[test]
fn tight_to_loose_list_correction_preserves_paragraph_and_text_identity() {
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    apply_output(&mut reducer, engine.append("- item").unwrap());

    let before = reducer.document().unwrap();
    let paragraph_id = before
        .nodes()
        .find(|node| matches!(node.content, ContentKind::Paragraph {}))
        .map(|node| node.id)
        .expect("tight list must synthesize a paragraph");
    let text_id = before
        .nodes()
        .find(|node| matches!(node.content, ContentKind::Text { .. }))
        .map(|node| node.id)
        .expect("list item must contain text");

    let correction = format!("\n\n  {}\n\n  second", "x".repeat(260));
    apply_output(&mut reducer, engine.append(&correction).unwrap());
    let after = reducer.document().unwrap();
    assert!(
        after
            .nodes()
            .any(|node| { matches!(node.content, ContentKind::List { tight: false, .. }) })
    );
    let current_ids = after.nodes().map(|node| node.id).collect::<Vec<NodeId>>();
    assert!(current_ids.contains(&paragraph_id));
    assert!(current_ids.contains(&text_id));
}

#[derive(Clone, Copy, Debug)]
enum AmbiguousFrontierKind {
    Setext,
    Table,
    ContinuingList,
    Emphasis,
}

fn assert_corrected_semantics(document: &Document, kind: AmbiguousFrontierKind, label: &str) {
    match kind {
        AmbiguousFrontierKind::Setext => assert!(
            document
                .nodes()
                .any(|node| matches!(node.content, ContentKind::Heading { level: 2 })),
            "{label} must resolve to a setext heading"
        ),
        AmbiguousFrontierKind::Table => assert!(
            document
                .nodes()
                .any(|node| matches!(node.content, ContentKind::Table { .. })),
            "{label} must resolve to a table"
        ),
        AmbiguousFrontierKind::ContinuingList => {
            let list = document
                .nodes()
                .find(|node| matches!(node.content, ContentKind::List { .. }))
                .expect("continuing-list fixture must resolve to a list");
            assert_eq!(list.children.len(), 2, "{label} must contain two items");
        }
        AmbiguousFrontierKind::Emphasis => assert!(
            document
                .nodes()
                .any(|node| matches!(node.content, ContentKind::Emphasis {})),
            "{label} must resolve to emphasis"
        ),
    }
}

#[test]
fn ambiguous_frontiers_preserve_unrelated_stable_identity_and_final_replay() {
    struct Case {
        label: &'static str,
        prefix: &'static str,
        correction: &'static str,
        kind: AmbiguousFrontierKind,
        seed: u64,
    }

    let cases = [
        Case {
            label: "setext",
            prefix: "# stable sibling\n\ncandidate",
            correction: "\n---\n\n",
            kind: AmbiguousFrontierKind::Setext,
            seed: 11,
        },
        Case {
            label: "table",
            prefix: "# stable sibling\n\n| A | B |",
            correction: "\n| - | - |\n| x | y |\n\n",
            kind: AmbiguousFrontierKind::Table,
            seed: 13,
        },
        Case {
            label: "continuing-list",
            prefix: "# stable sibling\n\n- first",
            correction: "\n- second\n\n",
            kind: AmbiguousFrontierKind::ContinuingList,
            seed: 17,
        },
        Case {
            label: "emphasis",
            prefix: "# stable sibling\n\n*open",
            correction: " emphasis*\n\n",
            kind: AmbiguousFrontierKind::Emphasis,
            seed: 19,
        },
    ];

    for case in cases {
        let mut engine = StreamEngine::new();
        let mut reducer = Reducer::new();
        apply_output(&mut reducer, engine.append(case.prefix).unwrap());

        let before = reducer.document().expect("prefix must produce a document");
        assert!(
            before.roots().len() >= 2,
            "{} must retain a stable sibling and an ambiguous frontier",
            case.label
        );
        let roots = before.roots().as_slice();
        let stable_id = roots[0];
        let stable = before
            .node(stable_id)
            .expect("stable sibling identity must resolve");
        assert!(matches!(stable.content, ContentKind::Heading { level: 1 }));
        assert_eq!(stable.stability, NodeStability::Stable, "{}", case.label);
        let stable_version = stable.version.clone();
        assert!(
            roots[1..].iter().all(|id| {
                before
                    .node(*id)
                    .is_some_and(|node| node.stability == NodeStability::Provisional)
            }),
            "{} ambiguous frontier must remain provisional",
            case.label
        );

        apply_output(&mut reducer, engine.append(case.correction).unwrap());
        let corrected = reducer
            .document()
            .expect("correction must retain the document");
        let unchanged = corrected
            .node(stable_id)
            .expect("correction must retain the stable sibling");
        assert_eq!(unchanged.version, stable_version, "{}", case.label);

        apply_output(&mut reducer, engine.finish().unwrap());
        let finalized = reducer
            .document()
            .expect("finish must project the corrected document");
        let finalized_sibling = finalized
            .node(stable_id)
            .expect("finish must retain the stable sibling");
        assert_eq!(finalized_sibling.version, stable_version, "{}", case.label);
        assert_corrected_semantics(finalized, case.kind, case.label);

        let source = format!("{}{}", case.prefix, case.correction);
        let baseline = replay(&source, &ChunkSchedule::Whole);
        for schedule in [
            ChunkSchedule::Characters,
            ChunkSchedule::Seeded {
                label: format!("u4.ambiguous-frontier.{}", case.label),
                seed: case.seed,
                trial: 3,
                max_bytes: 7,
            },
        ] {
            assert_eq!(
                replay(&source, &schedule),
                baseline,
                "{} final identity must not depend on {schedule:?}",
                case.label
            );
        }
    }
}
