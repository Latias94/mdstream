use mdstream::{
    CompilerError, CompilerLimits, CompilerMetrics, CustomBlockSpec, EngineError, StreamEngine,
};
use mdstream_protocol::{ChildListOwner, ContentKind, ProjectionOp, ProtocolLimits};
use pulldown_cmark::{Options, Parser};

fn deterministic_work(metrics: CompilerMetrics) -> u64 {
    metrics
        .structural_source_bytes
        .saturating_add(metrics.deferred_source_bytes)
        .saturating_add(metrics.parsed_source_bytes)
        .saturating_add(metrics.custom_scan_source_bytes)
        .saturating_add(metrics.reconcile_node_visits)
        .saturating_add(structural_reconcile_work(metrics))
        .saturating_add(metrics.reconcile_resource_visits)
        .saturating_add(metrics.incremental_projection_visits)
        .saturating_add(metrics.semantic_definition_visits)
        .saturating_add(metrics.semantic_state_key_visits)
        .saturating_add(metrics.semantic_state_edge_visits)
        .saturating_add(metrics.semantic_candidate_node_visits)
        .saturating_add(metrics.semantic_candidate_dependency_visits)
        .saturating_add(metrics.semantic_dependent_visits)
        .saturating_add(metrics.semantic_corrections_emitted)
}

fn structural_reconcile_work(metrics: CompilerMetrics) -> u64 {
    metrics
        .reconcile_structure_owners
        .saturating_add(metrics.reconcile_structure_id_comparisons)
        .saturating_add(metrics.reconcile_structure_version_steps)
        .saturating_add(metrics.reconcile_structure_ids_emitted)
}

fn repeated_fixture(prefix: &str, row: &str, minimum_bytes: usize) -> String {
    let mut source = String::from(prefix);
    while source.len() < minimum_bytes {
        source.push_str(row);
    }
    source
}

fn finished_bytewise_metrics(source: &str) -> CompilerMetrics {
    assert!(source.is_ascii());
    let mut engine = StreamEngine::new();
    for index in 0..source.len() {
        engine.append(&source[index..index + 1]).unwrap();
    }
    engine.finish().unwrap();
    engine.metrics().compiler
}

fn finished_whole_metrics(source: &str) -> CompilerMetrics {
    let mut engine = StreamEngine::new();
    engine.append(source).unwrap();
    engine.finish().unwrap();
    engine.metrics().compiler
}

fn finished_custom_whole_metrics(source: &str) -> CompilerMetrics {
    let mut engine = StreamEngine::builder()
        .custom_block(CustomBlockSpec::try_new("app.custom/1", "thinking").unwrap())
        .build()
        .unwrap();
    engine.append(source).unwrap();
    engine.finish().unwrap();
    engine.metrics().compiler
}

fn finished_nonopaque_custom_whole_metrics(source: &str, max_depth: usize) -> CompilerMetrics {
    let limits = mdstream_protocol::ProtocolLimits {
        max_tree_depth: max_depth,
        ..mdstream_protocol::ProtocolLimits::default()
    };
    let mut engine = StreamEngine::builder()
        .protocol_limits(limits)
        .custom_block(
            CustomBlockSpec::try_new("app.custom/1", "thinking")
                .unwrap()
                .opaque(false),
        )
        .build()
        .unwrap();
    engine.append(source).unwrap();
    engine.finish().unwrap();
    engine.metrics().compiler
}

fn nested_custom_fixture(depth: usize, minimum_bytes: usize) -> String {
    const OPEN: &str = "<thinking>\n\n";
    const CLOSE: &str = "</thinking>\n";
    let tag_bytes = depth.saturating_mul(OPEN.len().saturating_add(CLOSE.len()));
    let body_bytes = minimum_bytes.saturating_sub(tag_bytes).max(1);
    let mut source = String::with_capacity(tag_bytes.saturating_add(body_bytes));
    for _ in 0..depth {
        source.push_str(OPEN);
    }
    source.extend(std::iter::repeat_n('x', body_bytes));
    source.push('\n');
    for _ in 0..depth {
        source.push_str(CLOSE);
    }
    source
}

fn streamed_raw_false_closer_metrics(false_closers: usize) -> CompilerMetrics {
    let mut engine = StreamEngine::builder()
        .custom_block(CustomBlockSpec::try_new("app.custom/1", "thinking").unwrap())
        .build()
        .unwrap();
    engine.append("<thinking>\n<script>\n").unwrap();
    for _ in 0..false_closers {
        engine.append("</thinking>\n").unwrap();
    }
    engine.append("</script>\n</thinking>\n").unwrap();
    engine.metrics().compiler
}

fn streamed_guarded_false_closer_metrics(
    opening: &str,
    closing: &str,
    false_closers: usize,
) -> CompilerMetrics {
    let mut engine = StreamEngine::builder()
        .custom_block(CustomBlockSpec::try_new("app.custom/1", "thinking").unwrap())
        .build()
        .unwrap();
    engine.append(&format!("<thinking>\n{opening}\n")).unwrap();
    for _ in 0..false_closers {
        engine.append("</thinking>\n").unwrap();
    }
    engine.append(&format!("{closing}\n</thinking>\n")).unwrap();
    engine.metrics().compiler
}

fn streamed_raw_growing_line_metrics(chunks: usize) -> CompilerMetrics {
    let mut engine = StreamEngine::builder()
        .custom_block(CustomBlockSpec::try_new("app.custom/1", "thinking").unwrap())
        .build()
        .unwrap();
    engine.append("<thinking>\n<script>").unwrap();
    for _ in 0..chunks {
        engine.append("x>").unwrap();
    }
    engine.append("</script>\n</thinking>\n").unwrap();
    engine.metrics().compiler
}

fn streamed_opaque_nested_metrics(depth: usize) -> CompilerMetrics {
    const OPEN: &str = "<thinking>\n\n";
    const CLOSE: &str = "</thinking>\n";
    let mut engine = StreamEngine::builder()
        .custom_block(CustomBlockSpec::try_new("app.custom/1", "thinking").unwrap())
        .build()
        .unwrap();

    engine.append(OPEN).unwrap();
    for _ in 0..depth {
        engine.append(OPEN).unwrap();
    }
    for _ in 0..=depth {
        engine.append(CLOSE).unwrap();
    }
    engine.finish().unwrap();
    engine.metrics().compiler
}

fn streamed_top_level_raw_text_metrics(custom_looking_lines: usize) -> CompilerMetrics {
    let mut engine = StreamEngine::builder()
        .custom_block(CustomBlockSpec::try_new("app.custom/1", "thinking").unwrap())
        .build()
        .unwrap();

    engine.append("<script>\n").unwrap();
    for _ in 0..custom_looking_lines {
        engine.append("<thinking>\n\n").unwrap();
    }
    engine.metrics().compiler
}

fn assert_stage_doubling(
    label: &str,
    small: CompilerMetrics,
    large: CompilerMetrics,
    stage: impl Fn(CompilerMetrics) -> u64,
) {
    let small_work = stage(small);
    let large_work = stage(large);
    assert!(
        small_work > 0,
        "{label} counter was not exercised: {small:?}"
    );
    assert!(
        large_work.saturating_mul(100) <= small_work.saturating_mul(225),
        "{label} doubling grew from {small_work} to {large_work}: small={small:?}, large={large:?}"
    );
}

fn unique_link_fixture(count: usize) -> String {
    let mut source = String::new();
    for index in 0..count {
        source.push_str(&format!("[x](https://example.test/{index}) "));
    }
    source
}

fn nested_formatting_footnotes(depth: usize, candidates: usize) -> String {
    let mut source = String::new();
    for index in 0..depth {
        source.push(if index % 2 == 0 { '*' } else { '_' });
        source.push_str("open ");
    }
    for index in 0..candidates {
        source.push_str(&format!("[^note-{index}] "));
    }
    for index in (0..depth).rev() {
        source.push_str(" close");
        source.push(if index % 2 == 0 { '*' } else { '_' });
    }
    source
}

fn seeded_irregular_leaps(total: usize, seed: u64) -> Vec<usize> {
    let mut chunks = vec![17, 239, 769, 4_099];
    let mut consumed = chunks.iter().sum::<usize>();
    let mut state = seed.max(1);
    while consumed < total {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let requested = 1 + usize::try_from(state % 8_191).unwrap();
        let chunk = requested.min(total - consumed);
        chunks.push(chunk);
        consumed += chunk;
    }
    chunks
}

#[test]
fn unresolved_footnote_parser_events_use_a_bounded_preflight_budget() {
    let limits = CompilerLimits {
        max_markdown_events: 32,
        ..CompilerLimits::default()
    };
    let source = format!("{}\n\n", nested_formatting_footnotes(1, 20));
    let mut engine = StreamEngine::builder()
        .compiler_limits(limits)
        .build()
        .unwrap();
    engine.append("accepted\n\n").unwrap();
    let before_snapshot = engine.snapshot().unwrap();
    let before_coordinate = engine.coordinate().cloned().unwrap();
    let before_metrics = engine.metrics();

    assert!(matches!(
        engine.append(&source),
        Err(EngineError::Compiler(CompilerError::LimitExceeded {
            field: "markdown.events",
            limit: 32,
            actual: 33,
        }))
    ));
    assert_eq!(engine.snapshot().unwrap(), before_snapshot);
    assert_eq!(engine.coordinate(), Some(&before_coordinate));
    assert_eq!(engine.metrics(), before_metrics);

    let retry = engine
        .append("retry\n\n")
        .expect("a valid retry must succeed after parser-limit rejection");
    assert_eq!(retry.changes().len(), 1);
    assert_eq!(
        retry.changes()[0].sequence(),
        before_coordinate.sequence.checked_add(1).unwrap()
    );
}

#[test]
fn old_footnote_classification_has_an_independent_event_budget() {
    let source = "[^note]:    [^note]\ncontinuation";
    let canonical_options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_MATH
        | Options::ENABLE_GFM;
    let canonical_events = Parser::new_ext(source, canonical_options).count();
    let old_footnote_events =
        Parser::new_ext(source, canonical_options | Options::ENABLE_OLD_FOOTNOTES).count();
    let event_limit = canonical_events;

    assert_eq!(canonical_events, 7);
    assert_eq!(old_footnote_events, 8);
    let mut engine = StreamEngine::builder()
        .compiler_limits(CompilerLimits {
            max_markdown_events: event_limit,
            ..CompilerLimits::default()
        })
        .build()
        .unwrap();

    assert!(matches!(
        engine.append(source),
        Err(EngineError::Compiler(CompilerError::LimitExceeded {
            field: "markdown.events",
            limit,
            actual,
        })) if limit == event_limit && actual == event_limit.checked_add(1).unwrap()
    ));
}

#[test]
fn nested_unresolved_footnotes_stop_at_the_overlap_work_budget() {
    let limits = CompilerLimits::default();
    let source = nested_formatting_footnotes(128, 4_096);
    let mut engine = StreamEngine::builder()
        .compiler_limits(limits)
        .build()
        .unwrap();

    assert!(matches!(
        engine.append(&source),
        Err(EngineError::Compiler(CompilerError::LimitExceeded {
            field: "markdown.footnote_overlap_work",
            limit,
            actual,
        })) if limit == limits.max_markdown_overlap_work
            && actual == limit.checked_add(1).unwrap()
    ));
}

#[test]
fn unresolved_footnote_preflight_applies_the_tree_depth_limit() {
    let limits = ProtocolLimits {
        max_tree_depth: 8,
        ..ProtocolLimits::default()
    };
    let source = nested_formatting_footnotes(8, 1_024);
    let mut engine = StreamEngine::builder()
        .protocol_limits(limits)
        .build()
        .unwrap();

    assert!(matches!(
        engine.append(&source),
        Err(EngineError::Compiler(CompilerError::LimitExceeded {
            field: "tree.depth",
            limit: 8,
            actual: 9,
        }))
    ));
}

fn assert_linear_work(label: &str, source: &str, bytes_per_input: u64) {
    const SIZES: [usize; 4] = [8 * 1024, 16 * 1024, 32 * 1024, 64 * 1024];

    assert!(source.is_ascii(), "bytewise fixtures must be ASCII");
    assert!(source.len() >= SIZES[SIZES.len() - 1]);

    let mut engine = StreamEngine::new();
    let mut previous: Option<(u64, CompilerMetrics)> = None;

    for index in 0..SIZES[SIZES.len() - 1] {
        engine.append(&source[index..index + 1]).unwrap();

        let size = index + 1;
        if !SIZES.contains(&size) {
            continue;
        }

        let metrics = engine.metrics().compiler;
        let work = deterministic_work(metrics);
        assert!(
            work <= (size as u64).saturating_mul(bytes_per_input) + 4_096,
            "{label} visited {work} units for {size} source bytes: {metrics:?}"
        );
        if let Some((previous_work, previous_metrics)) = previous {
            assert!(
                work.saturating_mul(100) <= previous_work.saturating_mul(225),
                "{label} doubling to {size} bytes grew deterministic work from \
                 {previous_work} to {work}: previous={previous_metrics:?}, current={metrics:?}"
            );
        }
        previous = Some((work, metrics));
    }

    let before_finish = engine.metrics().compiler;
    engine.finish().unwrap();
    let finished = engine.metrics().compiler;
    assert!(
        finished.parse_passes <= before_finish.parse_passes.saturating_add(1),
        "{label} stabilization performed more than one final parse: \
         before={before_finish:?}, after={finished:?}"
    );
    let finished_work = deterministic_work(finished);
    assert!(
        finished_work <= (SIZES[SIZES.len() - 1] as u64).saturating_mul(bytes_per_input) + 4_096,
        "{label} visited {finished_work} units after stabilization: {finished:?}"
    );
}

#[test]
fn one_append_consumes_every_crossed_checkpoint_with_one_parse() {
    let mut engine = StreamEngine::new();
    let source = "x".repeat(64 * 1024);

    engine.append(&source).unwrap();

    let metrics = engine.metrics().compiler;
    assert_eq!(metrics.parse_passes, 1);
    assert_eq!(metrics.parsed_source_bytes, source.len() as u64);
    assert!(metrics.next_checkpoint > source.len());

    engine.finish().unwrap();
    assert_eq!(engine.metrics().compiler.parse_passes, 1);
}

#[test]
fn consecutive_custom_blocks_count_all_parser_work_and_scale_linearly() {
    const SIZES: [usize; 4] = [8 * 1024, 16 * 1024, 32 * 1024, 64 * 1024];
    const BLOCK: &str = concat!(
        "<thinking>\n",
        "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n",
        "</thinking>\n\n",
    );
    let mut previous = None;

    for minimum_bytes in SIZES {
        let source = repeated_fixture("", BLOCK, minimum_bytes);
        let metrics = finished_custom_whole_metrics(&source);
        let source_bytes = u64::try_from(source.len()).unwrap();

        assert!(
            metrics.parse_passes > 2,
            "fragment parser passes must be observable: {metrics:?}"
        );
        assert!(
            metrics.parsed_source_bytes > 0,
            "Markdown gaps must be counted: {metrics:?}"
        );
        assert!(
            metrics.parsed_source_bytes <= source_bytes,
            "custom parser work exceeded its linear byte bound: {metrics:?}"
        );
        assert_eq!(
            metrics.custom_scan_source_bytes, source_bytes,
            "the standalone topology scanner must visit the source once: {metrics:?}"
        );

        if let Some(previous_metrics) = previous {
            assert_stage_doubling("custom parse passes", previous_metrics, metrics, |m| {
                m.parse_passes
            });
            assert_stage_doubling("custom parsed source", previous_metrics, metrics, |m| {
                m.parsed_source_bytes
            });
            assert_stage_doubling("custom scanned source", previous_metrics, metrics, |m| {
                m.custom_scan_source_bytes
            });
        }
        previous = Some(metrics);
    }
}

#[test]
fn nested_nonopaque_custom_blocks_do_not_reparse_ancestor_bodies() {
    for (depth, minimum_bytes) in [
        (32, 8 * 1024),
        (64, 16 * 1024),
        (128, 32 * 1024),
        (256, 64 * 1024),
    ] {
        let source = nested_custom_fixture(depth, minimum_bytes);
        let metrics = finished_nonopaque_custom_whole_metrics(&source, depth + 4);
        let source_bytes = source.len() as u64;

        assert!(
            metrics.parsed_source_bytes <= source_bytes.saturating_mul(4),
            "depth={depth} reparsed nested ancestor bodies: source={} metrics={metrics:?}",
            source.len()
        );
    }
}

#[test]
fn pending_raw_text_suppresses_false_custom_close_recompilation() {
    let small = streamed_raw_false_closer_metrics(64);
    let large = streamed_raw_false_closer_metrics(128);
    let work = |metrics: CompilerMetrics| {
        metrics
            .parsed_source_bytes
            .saturating_add(metrics.custom_scan_source_bytes)
    };

    assert!(
        work(large).saturating_mul(100) <= work(small).saturating_mul(225),
        "raw-text false closer work grew too quickly: small={small:?}, large={large:?}"
    );
}

#[test]
fn every_pending_block_literal_suppresses_false_custom_closers() {
    for (label, opening, closing) in [
        ("fence", "```text", "```"),
        ("comment", "<!--", "-->"),
        ("cdata", "<![CDATA[", "]]>"),
        ("processing-instruction", "<?instruction", "?>"),
    ] {
        let small = streamed_guarded_false_closer_metrics(opening, closing, 64);
        let large = streamed_guarded_false_closer_metrics(opening, closing, 128);
        let work = |metrics: CompilerMetrics| {
            metrics
                .parsed_source_bytes
                .saturating_add(metrics.custom_scan_source_bytes)
        };
        assert!(
            work(large).saturating_mul(100) <= work(small).saturating_mul(225),
            "{label} false closer work grew too quickly: small={small:?}, large={large:?}"
        );
    }
}

#[test]
fn pending_raw_text_growing_line_never_rescans_the_historical_tail() {
    let small = streamed_raw_growing_line_metrics(2_048);
    let large = streamed_raw_growing_line_metrics(4_096);

    assert_stage_doubling("raw-text growing line", small, large, deterministic_work);
}

#[test]
fn streamed_opaque_nesting_does_not_rescan_the_outer_body_per_delimiter() {
    let small = streamed_opaque_nested_metrics(128);
    let large = streamed_opaque_nested_metrics(256);

    assert_stage_doubling("opaque nested custom scan", small, large, |metrics| {
        metrics.custom_scan_source_bytes
    });
    assert_stage_doubling("opaque nested total work", small, large, deterministic_work);
}

#[test]
fn top_level_raw_text_state_prevents_custom_looking_line_rescans() {
    let small = streamed_top_level_raw_text_metrics(128);
    let large = streamed_top_level_raw_text_metrics(256);

    assert_stage_doubling("top-level raw-text custom scan", small, large, |metrics| {
        metrics.custom_scan_source_bytes
    });
    assert_stage_doubling(
        "top-level raw-text total work",
        small,
        large,
        deterministic_work,
    );
}

#[test]
fn delimiter_dense_single_lines_have_linear_custom_scan_work() {
    let source = |count: usize| "<thinking>".repeat(count);
    let small_source = source(1_024);
    let large_source = source(2_048);
    let small = finished_custom_whole_metrics(&small_source);
    let large = finished_custom_whole_metrics(&large_source);

    assert_eq!(small.custom_scan_source_bytes, small_source.len() as u64);
    assert_eq!(large.custom_scan_source_bytes, large_source.len() as u64);
    assert_stage_doubling("delimiter-dense scan", small, large, |metrics| {
        metrics.custom_scan_source_bytes
    });
}

#[test]
fn seeded_irregular_leaps_consume_checkpoints_once_and_keep_plain_work_bounded() {
    const SOURCE_BYTES: usize = 64 * 1024;
    const ABSOLUTE_WORK_BOUND: u64 = (SOURCE_BYTES as u64) * 8 + 4_096;
    let source = "x".repeat(SOURCE_BYTES);
    let bytewise = finished_bytewise_metrics(&source);
    assert!(deterministic_work(bytewise) <= ABSOLUTE_WORK_BOUND);

    for seed in [0x5eed_u64, 0xdecafbad, 0x1234_5678_9abc_def0] {
        let mut engine = StreamEngine::new();
        let mut cursor = 0usize;
        for chunk_len in seeded_irregular_leaps(SOURCE_BYTES, seed) {
            let before = engine.metrics().compiler;
            let end = cursor + chunk_len;
            engine.append(&source[cursor..end]).unwrap();
            cursor = end;

            let after = engine.metrics().compiler;
            assert!(
                after.parse_passes <= before.parse_passes.saturating_add(1),
                "seed={seed:#x} chunk={chunk_len} parsed more than once: before={before:?}, after={after:?}"
            );
            assert!(
                after.next_checkpoint > after.frontier_bytes,
                "seed={seed:#x} left checkpoint {} at/before frontier {}: {after:?}",
                after.next_checkpoint,
                after.frontier_bytes
            );
        }
        assert_eq!(cursor, SOURCE_BYTES);

        let before_finish = engine.metrics().compiler;
        assert!(deterministic_work(before_finish) <= ABSOLUTE_WORK_BOUND);
        engine.finish().unwrap();
        let finished = engine.metrics().compiler;
        assert_eq!(
            finished.parse_passes, before_finish.parse_passes,
            "seed={seed:#x} reparsed a revision already compiled at 64 KiB"
        );
        assert!(deterministic_work(finished) <= ABSOLUTE_WORK_BOUND);
    }
}

#[test]
fn compiler_stage_counters_have_exact_small_trace_calibration() {
    let mut engine = StreamEngine::new();
    engine.append("a").unwrap();
    let initial = engine.metrics().compiler;
    assert_eq!(initial.structural_source_bytes, 1);
    assert_eq!(initial.deferred_source_bytes, 0);
    assert_eq!(initial.parse_passes, 1);
    assert_eq!(initial.parsed_source_bytes, 1);
    assert_eq!(initial.reconcile_node_visits, 2);
    assert_eq!(initial.reconcile_structure_owners, 3);
    assert_eq!(initial.reconcile_structure_id_comparisons, 0);
    assert_eq!(initial.reconcile_structure_version_steps, 2);
    assert_eq!(initial.reconcile_structure_ids_emitted, 2);
    assert_eq!(initial.reconcile_resource_visits, 0);
    assert_eq!(initial.incremental_projection_visits, 0);
    assert_eq!(initial.semantic_candidate_node_visits, 2);
    assert_eq!(initial.semantic_candidate_dependency_visits, 0);

    engine.append("b").unwrap();
    let incremental = engine.metrics().compiler;
    assert_eq!(incremental.structural_source_bytes, 2);
    assert_eq!(incremental.deferred_source_bytes, 0);
    assert_eq!(incremental.parse_passes, 1);
    assert_eq!(incremental.parsed_source_bytes, 1);
    assert_eq!(incremental.reconcile_node_visits, 2);
    assert_eq!(incremental.incremental_projection_visits, 2);

    engine.append("*").unwrap();
    let deferred = engine.metrics().compiler;
    assert_eq!(deferred.structural_source_bytes, 3);
    assert_eq!(deferred.deferred_source_bytes, 1);
    assert_eq!(deferred.parse_passes, 1);
    assert_eq!(deferred.incremental_projection_visits, 2);

    let linked = finished_whole_metrics("[x](https://example.test)");
    assert_eq!(linked.parse_passes, 1);
    assert_eq!(linked.reconcile_resource_visits, 1);
}

#[test]
fn individual_compiler_stage_counters_are_near_linear() {
    let plain_small = finished_bytewise_metrics(&"x".repeat(8 * 1024));
    let plain_large = finished_bytewise_metrics(&"x".repeat(16 * 1024));
    assert_stage_doubling("structural source", plain_small, plain_large, |m| {
        m.structural_source_bytes
    });
    assert_stage_doubling("parsed source", plain_small, plain_large, |m| {
        m.parsed_source_bytes
    });
    assert_stage_doubling("reconcile nodes", plain_small, plain_large, |m| {
        m.reconcile_node_visits
    });
    assert_stage_doubling("reconcile owners", plain_small, plain_large, |m| {
        m.reconcile_structure_owners
    });
    assert_stage_doubling("reconcile comparisons", plain_small, plain_large, |m| {
        m.reconcile_structure_id_comparisons
    });
    assert_stage_doubling("reconcile version steps", plain_small, plain_large, |m| {
        m.reconcile_structure_version_steps
    });
    assert_stage_doubling("reconcile IDs emitted", plain_small, plain_large, |m| {
        m.reconcile_structure_ids_emitted
    });
    assert_stage_doubling("incremental projections", plain_small, plain_large, |m| {
        m.incremental_projection_visits
    });
    assert_stage_doubling("semantic candidate nodes", plain_small, plain_large, |m| {
        m.semantic_candidate_node_visits
    });

    let deferred_small = finished_bytewise_metrics(&"[".repeat(8 * 1024));
    let deferred_large = finished_bytewise_metrics(&"[".repeat(16 * 1024));
    assert_stage_doubling("deferred source", deferred_small, deferred_large, |m| {
        m.deferred_source_bytes
    });

    let resources_small = finished_whole_metrics(&unique_link_fixture(128));
    let resources_large = finished_whole_metrics(&unique_link_fixture(256));
    assert_eq!(resources_small.reconcile_resource_visits, 128);
    assert_eq!(resources_large.reconcile_resource_visits, 256);
    assert_stage_doubling(
        "reconcile resources",
        resources_small,
        resources_large,
        |m| m.reconcile_resource_visits,
    );
}

#[test]
fn appends_between_geometric_checkpoints_do_not_reparse_the_frontier() {
    let mut engine = StreamEngine::new();

    engine.append("x").unwrap();
    assert_eq!(engine.metrics().compiler.parse_passes, 1);

    for _ in 1..255 {
        engine.append("x").unwrap();
    }
    assert_eq!(engine.metrics().compiler.parse_passes, 1);
    assert_eq!(engine.metrics().compiler.next_checkpoint, 256);

    engine.append("x").unwrap();
    assert_eq!(engine.metrics().compiler.parse_passes, 2);
    assert_eq!(engine.metrics().compiler.next_checkpoint, 512);

    for _ in 256..511 {
        engine.append("x").unwrap();
    }
    assert_eq!(engine.metrics().compiler.parse_passes, 2);

    engine.append("x").unwrap();
    assert_eq!(engine.metrics().compiler.parse_passes, 3);
    assert_eq!(engine.metrics().compiler.next_checkpoint, 1024);
}

#[test]
fn geometric_frontier_parse_work_is_near_linear() {
    let source = "x".repeat(64 * 1024);
    assert_linear_work("plain", &source, 8);
}

#[test]
fn fenced_frontier_work_is_near_linear() {
    let source = repeated_fixture("```text\n", "code line\n", 64 * 1024);
    assert_linear_work("fence", &source, 8);
}

#[test]
fn container_frontier_work_is_near_linear() {
    let row = format!("> {}\n", "x".repeat(252));
    let source = repeated_fixture("", &row, 64 * 1024);
    assert_linear_work("container", &source, 32);
}

#[test]
fn table_frontier_work_is_near_linear() {
    let row = format!("{} | {}\n", "x".repeat(124), "y".repeat(124));
    let source = repeated_fixture("a | b\n--|--\n", &row, 64 * 1024);
    assert_linear_work("table", &source, 32);
}

#[test]
fn loose_list_frontier_work_is_near_linear() {
    let small = repeated_fixture("", "- item\n\n", 8 * 1024);
    let large = repeated_fixture("", "- item\n\n", 16 * 1024);
    let small_metrics = finished_bytewise_metrics(&small);
    let large_metrics = finished_bytewise_metrics(&large);
    let small_work = deterministic_work(small_metrics);
    let large_work = deterministic_work(large_metrics);
    assert!(
        large_work.saturating_mul(100) <= small_work.saturating_mul(225),
        "loose list doubling grew deterministic work from {small_work} to {large_work}: \
         small={small_metrics:?}, large={large_metrics:?}"
    );
}

#[test]
fn unresolved_whitespace_does_not_reparse_every_revision() {
    let mut engine = StreamEngine::new();

    engine.append(" ").unwrap();
    assert_eq!(engine.metrics().compiler.parse_passes, 1);

    for _ in 1..255 {
        engine.append(" ").unwrap();
    }

    assert_eq!(engine.metrics().compiler.parse_passes, 1);
    assert_eq!(engine.metrics().compiler.next_checkpoint, 256);
}

#[test]
fn completed_blank_whitespace_releases_the_frontier() {
    let mut engine = StreamEngine::new();

    engine.append(" \n").unwrap();

    assert_eq!(engine.metrics().compiler.parse_passes, 1);
    assert_eq!(engine.metrics().compiler.frontier_bytes, 0);
}

#[test]
fn blank_lines_inside_fenced_code_do_not_trigger_structural_compilation() {
    let mut engine = StreamEngine::new();

    engine.append("```text\n").unwrap();
    assert_eq!(engine.metrics().compiler.parse_passes, 1);

    engine.append("\n").unwrap();
    assert_eq!(engine.metrics().compiler.parse_passes, 1);

    engine.append("```\n").unwrap();
    assert_eq!(engine.metrics().compiler.parse_passes, 2);
}

#[test]
fn indented_closing_fence_triggers_structural_compilation() {
    let mut engine = StreamEngine::new();

    engine.append("```text\nbody\n").unwrap();
    assert_eq!(engine.metrics().compiler.parse_passes, 1);

    engine.append(" ```\n").unwrap();
    assert_eq!(engine.metrics().compiler.parse_passes, 2);
}

#[test]
fn stable_root_growth_emits_only_frontier_sized_splices() {
    const PARAGRAPHS: usize = 128;
    let mut engine = StreamEngine::new();
    let mut root_ids_transmitted = 0usize;

    for index in 0..PARAGRAPHS {
        let output = engine.append(&format!("paragraph {index}\n\n")).unwrap();
        for change in output.changes() {
            for operation in change.operations() {
                if let ProjectionOp::SpliceChildren {
                    owner: ChildListOwner::Document,
                    delete_count,
                    insert,
                    ..
                } = operation
                {
                    root_ids_transmitted = root_ids_transmitted
                        .saturating_add(*delete_count as usize)
                        .saturating_add(insert.len());
                }
            }
        }
    }

    assert!(
        root_ids_transmitted <= PARAGRAPHS * 2,
        "stable root growth retransmitted {root_ids_transmitted} IDs for {PARAGRAPHS} roots"
    );
}

#[test]
fn growing_list_emits_only_the_new_child_id() {
    let mut engine = StreamEngine::new();
    let initial = engine.append("- first").unwrap();
    let list_id = initial
        .changes()
        .iter()
        .flat_map(|change| change.operations())
        .find_map(|operation| match operation {
            ProjectionOp::InsertNode { node }
                if matches!(node.content, ContentKind::List { .. }) =>
            {
                Some(node.id)
            }
            _ => None,
        })
        .expect("the initial projection should insert a list node");

    engine.append("\n- second").unwrap();
    let appended = engine.finish().unwrap();
    let splice = appended
        .changes()
        .iter()
        .flat_map(|change| change.operations())
        .find_map(|operation| match operation {
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Node { node_id },
                start,
                delete_count,
                insert,
                ..
            } if *node_id == list_id => Some((*start, *delete_count, insert.as_slice())),
            _ => None,
        })
        .expect("the existing list should receive a child-list splice");

    assert_eq!(splice.0, 1);
    assert_eq!(splice.1, 0);
    assert_eq!(splice.2.len(), 1);
}

fn growing_frontier_metrics(paragraphs: usize) -> CompilerMetrics {
    let mut engine = StreamEngine::new();
    engine.append("paragraph 0").unwrap();
    for index in 1..paragraphs {
        engine.append(&format!("\n\nparagraph {index}")).unwrap();
    }
    engine.metrics().compiler
}

#[test]
fn growing_frontier_root_version_work_is_near_linear() {
    let small = growing_frontier_metrics(128);
    let large = growing_frontier_metrics(256);
    assert!(
        structural_reconcile_work(large).saturating_mul(100)
            <= structural_reconcile_work(small).saturating_mul(225),
        "frontier root growth expanded structure work too quickly: small={small:?}, large={large:?}"
    );
}

#[test]
fn html_blank_lines_do_not_force_reparse_before_a_checkpoint() {
    let mut engine = StreamEngine::new();
    engine.append("<div>\n").unwrap();
    for _ in 0..1_024 {
        engine.append("\n").unwrap();
    }

    let metrics = engine.metrics().compiler;
    assert!(
        metrics.parsed_source_bytes <= 8 * 1_024,
        "released HTML source was reparsed for each blank line: {metrics:?}"
    );
}

#[test]
fn footnote_blank_line_waits_for_classified_continuation() {
    let mut engine = StreamEngine::new();
    engine.append("[^note]: first\n").unwrap();
    let before = engine.metrics().compiler.parse_passes;

    engine.append("\n").unwrap();

    assert_eq!(engine.metrics().compiler.parse_passes, before);
}
