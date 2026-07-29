use super::reference;
use mdstream::{CustomBlockSpec, EngineOutput, StreamEngine};
use mdstream_conformance::{ChunkSchedule, NormalizedSnapshot};
use mdstream_protocol::{
    ApplyOutcome, ContentKind, LinkStyle, ProjectionOp, ProtocolLimits, Reducer, TransitionReducer,
};
use std::collections::BTreeSet;

fn apply_with_transition_mirror(
    reducer: &mut Reducer,
    transition_reducer: &mut TransitionReducer,
    output: EngineOutput,
) {
    for change in output.into_changes() {
        let outcome = reducer.apply(change.clone()).unwrap();
        assert!(matches!(outcome, ApplyOutcome::Applied { .. }));
        let observed = transition_reducer.apply(change).unwrap();
        assert_eq!(observed.outcome, outcome);
        assert!(observed.facts.is_some());
    }
}

#[test]
fn late_reference_definition_corrects_only_the_stable_dependent() {
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    let mut transition_reducer = TransitionReducer::new();
    let initial = engine.append("[shared]\n\nunrelated\n\n").unwrap();
    apply_with_transition_mirror(&mut reducer, &mut transition_reducer, initial);
    let before = engine.snapshot().unwrap();
    let (reference_id, before_version, before_target) = reference(&before, 0);
    assert_eq!(before_target, None);
    let unrelated = before
        .nodes()
        .iter()
        .find(|node| {
            node.source.start.get() == 10 && matches!(node.content, ContentKind::Paragraph {})
        })
        .expect("unrelated paragraph must exist");
    let unrelated_id = unrelated.id;
    let unrelated_version = unrelated.version.clone();

    let output = engine.append("[shared]: /target\n").unwrap();
    let replacements = output
        .changes()
        .iter()
        .flat_map(|change| change.operations())
        .filter_map(|operation| match operation {
            ProjectionOp::ReplaceNode { node_id, .. } => Some(*node_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(replacements, vec![reference_id]);
    apply_with_transition_mirror(&mut reducer, &mut transition_reducer, output);

    let after = engine.snapshot().unwrap();
    let (after_id, after_version, after_target) = reference(&after, 0);
    assert_eq!(after_id, reference_id);
    assert_ne!(after_version, before_version);
    assert!(after_target.is_some());
    let unrelated = after
        .nodes()
        .iter()
        .find(|node| node.id == unrelated_id)
        .expect("unrelated node must survive");
    assert_eq!(unrelated.version, unrelated_version);
    assert_eq!(
        transition_reducer.document().unwrap().snapshot(),
        reducer.document().unwrap().snapshot()
    );
}

#[test]
fn reference_labels_use_commonmark_whitespace_and_unicode_case_folding() {
    let mut engine = StreamEngine::new();
    engine
        .append("[space][Ref \t Name] [unicode][Straße]\n\n")
        .unwrap();
    engine
        .append("[ref name]: /space\n[STRASSE]: /unicode\n")
        .unwrap();
    let snapshot = engine.snapshot().unwrap();
    let destinations = snapshot
        .resources()
        .iter()
        .filter_map(|resource| match &resource.content {
            mdstream_protocol::SemanticResourceKind::Link { destination, .. } => {
                Some(destination.as_str())
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(destinations, BTreeSet::from(["/space", "/unicode"]));
    assert!(snapshot.nodes().iter().all(|node| {
        !matches!(
            node.content,
            ContentKind::Link {
                reference_label: Some(_),
                target: None,
                ..
            }
        )
    }));
}

#[test]
fn first_definition_wins_across_custom_markdown_regions_without_an_early_use() {
    let source = concat!(
        "[shared]: /one\n\n",
        "<thinking>\nbody\n</thinking>\n\n",
        "[b][shared]\n\n[shared]: /two\n",
    );
    let mut engine = StreamEngine::builder()
        .custom_block(CustomBlockSpec::try_new("app.custom/1", "thinking").unwrap())
        .build()
        .unwrap();

    engine.append(source).unwrap();
    let snapshot = engine.snapshot().unwrap();
    let reference_start = u64::try_from(source.find("[b][shared]").unwrap()).unwrap();
    let (_, _, target) = reference(&snapshot, reference_start);
    let target = target.expect("the later reference must resolve");
    let resource = snapshot
        .resources()
        .iter()
        .find(|resource| resource.id == target)
        .expect("resolved resource must exist");
    assert!(matches!(
        &resource.content,
        mdstream_protocol::SemanticResourceKind::Link { destination, .. }
            if destination == "/one"
    ));
}

#[test]
fn a_stable_reference_resource_is_reused_across_frontiers_and_budgets() {
    let mut engine = StreamEngine::builder()
        .protocol_limits(ProtocolLimits {
            max_resources: 1,
            ..ProtocolLimits::default()
        })
        .build()
        .unwrap();
    engine.append("[a][shared]\n\n[shared]: /one\n\n").unwrap();
    let first = engine.snapshot().unwrap();
    let (_, _, first_target) = reference(&first, 0);
    let first_target = first_target.expect("first reference must resolve");

    engine.append("[b][shared]").unwrap();
    let second = engine.snapshot().unwrap();
    let second_start = u64::try_from("[a][shared]\n\n[shared]: /one\n\n".len()).unwrap();
    let (_, _, second_target) = reference(&second, second_start);
    assert_eq!(second_target, Some(first_target));
    assert_eq!(second.resources().len(), 1);
}

#[test]
fn provisional_shortcut_does_not_promote_a_stable_definition_resource() {
    let prefix = "[label]: /definition\n\n[label]";
    let suffix = "(/inline)\n";

    let mut split = StreamEngine::new();
    split.append(prefix).unwrap();
    assert!(
        split
            .snapshot()
            .unwrap()
            .resources()
            .iter()
            .any(|resource| {
                matches!(
                    &resource.content,
                    mdstream_protocol::SemanticResourceKind::Link { destination, .. }
                        if destination == "/definition"
                )
            })
    );
    split.append(suffix).unwrap();
    split.finish().unwrap();

    let mut whole = StreamEngine::new();
    whole.append(&format!("{prefix}{suffix}")).unwrap();
    whole.finish().unwrap();

    assert_eq!(
        NormalizedSnapshot::from(split.snapshot().unwrap()),
        NormalizedSnapshot::from(whole.snapshot().unwrap())
    );
}

#[test]
fn frontier_definition_promotion_keeps_its_resource_alive() {
    let mut engine = StreamEngine::new();
    engine.append("[label]\n\n").unwrap();
    engine.append("[label]: /target").unwrap();
    let before = engine.snapshot().unwrap();
    let (_, _, target) = reference(&before, 0);
    let target = target.expect("the provisional definition must resolve the stable reference");

    engine.append("\n\n").unwrap();
    let after = engine.snapshot().unwrap();
    let (_, _, promoted_target) = reference(&after, 0);
    assert_eq!(promoted_target, Some(target));
    assert!(
        after
            .resources()
            .iter()
            .any(|resource| resource.id == target)
    );
}

#[test]
fn finish_promotes_semantic_frontiers_without_reparse_and_counts_the_work() {
    let mut dependency = StreamEngine::new();
    dependency.append("[missing]").unwrap();
    let before_dependency = dependency.metrics().compiler;
    dependency.finish().unwrap();
    let after_dependency = dependency.metrics().compiler;
    assert_eq!(
        after_dependency.parse_passes,
        before_dependency.parse_passes
    );
    assert_eq!(
        after_dependency.semantic_state_edge_visits - before_dependency.semantic_state_edge_visits,
        1
    );
    assert_eq!(after_dependency.retained_semantic_dependencies, 1);

    let mut definition = StreamEngine::new();
    definition.append("[label]: /target").unwrap();
    let before_definition = definition.metrics().compiler;
    definition.finish().unwrap();
    let after_definition = definition.metrics().compiler;
    assert_eq!(
        after_definition.parse_passes,
        before_definition.parse_passes
    );
    assert_eq!(
        after_definition.semantic_state_key_visits - before_definition.semantic_state_key_visits,
        1
    );
    assert_eq!(after_definition.retained_semantic_definitions, 1);
}

#[test]
fn late_citation_definition_emits_a_typed_definition_and_targeted_correction() {
    let mut engine = StreamEngine::new();
    engine.append("See [@Paper]\n\n").unwrap();
    let before = engine.snapshot().unwrap();
    let unresolved = before
        .nodes()
        .iter()
        .find(|node| {
            matches!(
                node.content,
                ContentKind::CitationReference { target: None, .. }
            )
        })
        .unwrap();
    let reference_id = unresolved.id;

    engine
        .append("[@paper]: https://example.test/paper \"Paper\"\n")
        .unwrap();
    let after = engine.snapshot().unwrap();
    assert!(after.nodes().iter().any(|node| {
        matches!(
            node.content,
            ContentKind::CitationDefinition { ref key, .. } if key == "paper"
        )
    }));
    assert!(matches!(
        after
            .nodes()
            .iter()
            .find(|node| node.id == reference_id)
            .map(|node| &node.content),
        Some(ContentKind::CitationReference {
            key,
            target: Some(_)
        }) if key == "paper"
    ));
}

#[test]
fn citation_extension_does_not_capture_non_shortcut_reference_forms() {
    let mut engine = StreamEngine::new();
    engine
        .append("![alt][@paper] [display][@paper] [@paper][]\n\n[@paper]: /paper\n")
        .unwrap();
    engine.finish().unwrap();
    let snapshot = engine.snapshot().unwrap();

    assert!(
        !snapshot
            .nodes()
            .iter()
            .any(|node| matches!(node.content, ContentKind::CitationReference { .. }))
    );
    let ordinary_targets = snapshot
        .nodes()
        .iter()
        .filter_map(|node| match &node.content {
            ContentKind::Link {
                target: Some(target),
                ..
            }
            | ContentKind::Image {
                target: Some(target),
                ..
            } => Some(target.id),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(ordinary_targets.len(), 1);
    let ordinary = snapshot
        .resources()
        .iter()
        .find(|resource| ordinary_targets.contains(&resource.id))
        .unwrap();
    assert!(matches!(
        ordinary.content,
        mdstream_protocol::SemanticResourceKind::Link { .. }
    ));
    assert!(
        snapshot
            .nodes()
            .iter()
            .any(|node| { matches!(node.content, ContentKind::CitationDefinition { .. }) })
    );
}

#[test]
fn late_definitions_correct_every_reference_form_and_no_unrelated_node() {
    let mut engine = StreamEngine::new();
    let source = "[shortcut] [collapsed][] [full][target] ![image][target]\n\nunrelated\n\n";
    engine.append(source).unwrap();
    let before = engine.snapshot().unwrap();
    let dependents = before
        .nodes()
        .iter()
        .filter(|node| {
            matches!(
                node.content,
                ContentKind::Link {
                    target: None,
                    reference_label: Some(_),
                    ..
                } | ContentKind::Image {
                    target: None,
                    reference_label: Some(_),
                    ..
                }
            )
        })
        .map(|node| node.id)
        .collect::<BTreeSet<_>>();
    assert_eq!(dependents.len(), 4);
    let unrelated = before
        .nodes()
        .iter()
        .find(|node| {
            node.source.start.get() == u64::try_from(source.find("unrelated").unwrap()).unwrap()
                && matches!(node.content, ContentKind::Paragraph {})
        })
        .unwrap();
    let unrelated_version = unrelated.version.clone();

    let output = engine
        .append("[shortcut]: /shortcut\n[collapsed]: /collapsed\n[target]: /target\n")
        .unwrap();
    let replaced = output
        .changes()
        .iter()
        .flat_map(|change| change.operations())
        .filter_map(|operation| match operation {
            ProjectionOp::ReplaceNode { node_id, .. } => Some(*node_id),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(replaced, dependents);

    let after = engine.snapshot().unwrap();
    assert!(
        after
            .nodes()
            .iter()
            .filter(|node| dependents.contains(&node.id))
            .all(|node| {
                matches!(
                    node.content,
                    ContentKind::Link {
                        target: Some(_),
                        style: LinkStyle::Shortcut | LinkStyle::Collapsed | LinkStyle::Reference,
                        ..
                    } | ContentKind::Image {
                        target: Some(_),
                        style: LinkStyle::Reference,
                        ..
                    }
                )
            })
    );
    assert_eq!(
        after
            .nodes()
            .iter()
            .find(|node| node.id == unrelated.id)
            .unwrap()
            .version,
        unrelated_version
    );
}

#[test]
fn duplicate_definition_changes_source_only_and_preserves_first_winner() {
    let mut engine = StreamEngine::new();
    engine
        .append("[use][label]\n\n[label]: /first\n\n")
        .unwrap();
    let before = engine.snapshot().unwrap();
    let (_, before_version, before_target) = reference(&before, 0);

    let output = engine.append("[label]: /ignored\n").unwrap();
    assert!(output.changes().iter().all(|change| {
        change.operations().iter().all(|operation| {
            !matches!(
                operation,
                ProjectionOp::InsertResource { .. }
                    | ProjectionOp::ReplaceResource { .. }
                    | ProjectionOp::RemoveResource { .. }
                    | ProjectionOp::ReplaceNode { .. }
            )
        })
    }));
    let after = engine.snapshot().unwrap();
    let (_, after_version, after_target) = reference(&after, 0);
    assert_eq!(after_version, before_version);
    assert_eq!(after_target, before_target);
    let resource = after
        .resources()
        .iter()
        .find(|resource| Some(resource.id) == after_target)
        .unwrap();
    assert!(matches!(
        &resource.content,
        mdstream_protocol::SemanticResourceKind::Link { destination, .. }
            if destination == "/first"
    ));
}

#[test]
fn disappearing_provisional_definition_removes_its_resource() {
    let mut engine = StreamEngine::new();
    engine.append("See [@paper]\n\n[@paper]: /paper").unwrap();
    let before = engine.snapshot().unwrap();
    let resource_id = before.resources()[0].id;
    assert!(before.nodes().iter().any(|node| {
        matches!(
            node.content,
            ContentKind::CitationReference {
                target: Some(_),
                ..
            }
        )
    }));

    engine.append(" \"title\" trailing\n\n").unwrap();
    let output = engine.finish().unwrap();
    assert!(
        output.changes().iter().any(|change| {
            change.operations().iter().any(|operation| {
                matches!(
                    operation,
                    ProjectionOp::RemoveResource { resource_id: removed, .. }
                        if *removed == resource_id
                )
            })
        }),
        "changes: {:#?}",
        output.changes()
    );
    let after = engine.snapshot().unwrap();
    assert!(after.resources().is_empty());
    assert!(after.nodes().iter().any(|node| {
        matches!(
            node.content,
            ContentKind::CitationReference { target: None, .. }
        )
    }));

    let mut whole = StreamEngine::new();
    whole
        .append("See [@paper]\n\n[@paper]: /paper \"title\" trailing\n\n")
        .unwrap();
    whole.finish().unwrap();
    assert_eq!(
        NormalizedSnapshot::from(after),
        NormalizedSnapshot::from(whole.snapshot().unwrap())
    );
}

#[test]
fn code_html_and_opaque_custom_content_never_contribute_semantic_facts() {
    let source = concat!(
        "outside [real]\n\n",
        "`[^inline] [real]: /inline`\n\n",
        "[wrapped [^link]](/target)\n\n",
        "![alt [^image]](/image)\n\n",
        "```text\n[real]: /code\n[^code]\n```\n\n",
        "<div>\n[real]: /html\n[^html]\n</div>\n\n",
        "<thinking>\n[real]: /custom\n[^custom]\n[@fake]\n</thinking>\n",
    );
    let mut engine = StreamEngine::builder()
        .custom_block(CustomBlockSpec::try_new("app.custom/1", "thinking").unwrap())
        .build()
        .unwrap();
    engine.append(source).unwrap();
    engine.finish().unwrap();
    let snapshot = engine.snapshot().unwrap();
    let (_, _, target) = reference(&snapshot, 8);
    assert_eq!(target, None);
    assert!(!snapshot.nodes().iter().any(|node| {
        matches!(
            node.content,
            ContentKind::FootnoteReference { .. } | ContentKind::CitationReference { .. }
        )
    }));
    assert_eq!(snapshot.resources().len(), 2);
    assert!(snapshot.resources().iter().all(|resource| {
        matches!(
            resource.content,
            mdstream_protocol::SemanticResourceKind::Link { .. }
        )
    }));
}

#[test]
fn reset_clears_definition_registry_and_reverse_dependencies() {
    let mut engine = StreamEngine::new();
    engine.append("[label]: /old\n").unwrap();
    engine.reset().unwrap();
    engine.append("[new][label]").unwrap();
    let snapshot = engine.snapshot().unwrap();
    let (_, _, target) = reference(&snapshot, 0);
    assert_eq!(target, None);
    assert!(snapshot.resources().is_empty());
}

#[test]
fn semantic_registry_and_corrections_are_chunk_schedule_invariant() {
    let source = concat!(
        "[link][Shared] [^Note] [@Paper]\n\n",
        "<thinking>\nclassified **body**\n</thinking>\n\n",
        "[shared]: /target\n",
        "[^note]: footnote body\n",
        "[@paper]: /paper \"Paper\"\n",
    );
    let schedules = [
        ChunkSchedule::Whole,
        ChunkSchedule::Characters,
        ChunkSchedule::ByteCuts {
            cuts: vec![1, 12, 35, 64, source.len() - 1],
        },
        ChunkSchedule::Seeded {
            label: "u5.semantic-correction".to_string(),
            seed: 0x05e1_a71c_u64,
            trial: 3,
            max_bytes: 13,
        },
    ];
    let compile = |schedule: &ChunkSchedule| {
        let mut engine = StreamEngine::builder()
            .custom_block(CustomBlockSpec::try_new("app.custom/1", "thinking").unwrap())
            .build()
            .unwrap();
        for chunk in schedule.slices(source).unwrap() {
            engine.append(chunk).unwrap();
        }
        engine.finish().unwrap();
        NormalizedSnapshot::from(engine.snapshot().unwrap())
    };
    let expected = compile(&schedules[0]);
    for schedule in &schedules[1..] {
        assert_eq!(compile(schedule), expected, "schedule {schedule:?}");
    }
}
