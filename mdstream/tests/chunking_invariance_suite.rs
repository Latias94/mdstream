mod support;

use mdstream_conformance::ChunkSchedule;

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
    assert_eq!(
        support::replay([
            "A\r".to_string(),
            "\n\r".to_string(),
            "\nB\r".to_string(),
            "\n".to_string(),
        ]),
        support::replay(support::chunk_whole(source))
    );
    assert_invariant("ambiguous-table", "a# Heading\n|# Heading\n-->\n");
}
