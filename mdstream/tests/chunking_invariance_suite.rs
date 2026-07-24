mod support;

use mdstream::StreamEngine;
use mdstream_conformance::{ChunkSchedule, NormalizedSnapshot, exhaustive_utf8_partitions};

fn assert_invariant(case: &str, source: &str) {
    let expected = support::replay(support::chunk_whole(source));
    for schedule in [
        ChunkSchedule::Lines,
        ChunkSchedule::Characters,
        ChunkSchedule::Seeded {
            label: case.to_string(),
            seed: 0x5eed,
            trial: 7,
            max_bytes: 31,
        },
    ] {
        let chunks = schedule
            .slices(source)
            .unwrap()
            .into_iter()
            .map(str::to_string);
        assert_eq!(support::replay(chunks), expected, "case={case}");
    }
}

fn assert_cut_invariant(source: &str, cuts: Vec<usize>) {
    let chunks = ChunkSchedule::ByteCuts { cuts: cuts.clone() }
        .slices(source)
        .unwrap()
        .into_iter()
        .map(str::to_string);
    assert_eq!(
        support::replay(chunks),
        support::replay(support::chunk_whole(source)),
        "cuts={cuts:?}"
    );
}

fn assert_schedule_invariant(
    case: &str,
    source: &str,
    schedule: &ChunkSchedule,
    expected: &NormalizedSnapshot,
) {
    let chunks = schedule
        .slices(source)
        .unwrap()
        .into_iter()
        .map(str::to_string);
    assert_eq!(
        &support::replay(chunks),
        expected,
        "case={case} schedule={schedule:?}"
    );
}

#[test]
fn reference_documents_are_chunk_invariant_under_the_canonical_engine() {
    for (case, source) in [
        (
            "basic-many-blocks",
            include_str!("fixtures/streamdown_bench/basic_many_blocks_100.md"),
        ),
        (
            "code",
            include_str!("fixtures/streamdown_bench/code_multiple_code_blocks.md"),
        ),
        (
            "footnotes",
            include_str!("fixtures/streamdown_bench/footnotes_with_footnotes.md"),
        ),
        (
            "html",
            include_str!("fixtures/streamdown_bench/html_multiple_blocks.md"),
        ),
        (
            "math",
            include_str!("fixtures/streamdown_bench/math_with_split_delimiters.md"),
        ),
        (
            "mixed",
            include_str!("fixtures/streamdown_bench/mixed_content_realistic.md"),
        ),
        (
            "table",
            include_str!("fixtures/streamdown_bench/table_simple.md"),
        ),
    ] {
        assert_invariant(case, source);
    }
}

#[test]
fn split_crlf_and_ambiguous_table_text_are_chunk_invariant() {
    let source = "A\r\n\r\nB\r\n";
    assert_cut_invariant(source, vec![2, 4, 7]);
    assert_invariant("ambiguous-table", "a# Heading\n|# Heading\n-->\n");
}

#[test]
fn bounded_short_sources_are_invariant_under_every_utf8_partition() {
    for (case, source) in [
        ("ascii", "A\nB"),
        ("split-crlf", "A\r\nB"),
        ("unicode", "中é🙂"),
        ("ambiguous-setext", "a\n---\n"),
        ("nested-autolink-reference", "[<aa:>]a"),
    ] {
        let expected = support::replay(support::chunk_whole(source));
        for schedule in exhaustive_utf8_partitions(source).unwrap() {
            assert_schedule_invariant(case, source, &schedule, &expected);
        }
    }
}

#[test]
fn an_incomplete_line_after_a_table_remains_in_the_mutable_frontier() {
    let source = "$$\n| A | B |\n|---|---|\n| 1 | 2 |\n";
    assert_cut_invariant(source, vec!["$$\n| A | B |\n|---|---|\n|".len()]);
}

#[test]
fn incomplete_lazy_block_quote_continuations_keep_the_container_frontier() {
    let source = "> quoted line\n-->\n";
    assert_cut_invariant(source, vec!["> quoted line\n-".len()]);
}

#[test]
fn a_partial_heading_marker_can_become_a_lazy_block_quote_continuation() {
    let source = "> quoted line\n#x\n";
    assert_cut_invariant(source, vec!["> quoted line\n#".len()]);
}

#[test]
fn partial_block_markers_can_return_to_paragraph_and_list_continuations() {
    for (prefix, suffix) in [
        ("plain line\n#", "x\n"),
        ("- listed line\n#", "x\n"),
        ("> - nested line\n#", "x\n"),
    ] {
        let source = format!("{prefix}{suffix}");
        assert_cut_invariant(&source, vec![prefix.len()]);
    }
}

#[test]
fn a_closed_non_paragraph_container_tail_is_not_retained() {
    let mut engine = StreamEngine::new();
    engine.append("> # heading\n#").unwrap();

    assert_eq!(engine.metrics().compiler.frontier_bytes, 1);
}
