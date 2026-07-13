mod support;

use std::path::PathBuf;

use mdstream::{BlockKind, FootnotesMode, Options};
use mdstream_conformance::{LegacyBlock, load_fixture_dir};

fn assert_invariant(case_name: &str, markdown: &str, opts: Options, trials: u64, max_bytes: usize) {
    let expected = support::collect_final_blocks(support::chunk_whole(markdown), opts.clone());

    let blocks_lines = support::collect_final_blocks(support::chunk_lines(markdown), opts.clone());
    assert_eq!(blocks_lines, expected, "case={case_name} chunker=lines");

    let blocks_chars = support::collect_final_blocks(support::chunk_chars(markdown), opts.clone());
    assert_eq!(blocks_chars, expected, "case={case_name} chunker=chars");

    for t in 0..trials {
        let blocks_rand = support::collect_final_blocks(
            support::chunk_pseudo_random(markdown, case_name, t, max_bytes),
            opts.clone(),
        );
        assert_eq!(blocks_rand, expected, "case={case_name} chunker=rand t={t}");
    }
}

#[test]
fn streamdown_benchmark_suite_chunking_invariance() {
    // Inputs sourced from Streamdown's `__benchmarks__/parse-blocks.bench.ts`.
    let single_block = include_str!("fixtures/streamdown_bench/basic_single_block.md")
        .trim_end_matches(['\r', '\n']);
    let multiple_blocks_10 = include_str!("fixtures/streamdown_bench/basic_multiple_blocks_10.md");
    let single_code_block = include_str!("fixtures/streamdown_bench/code_single_code_block.md");
    let math_with_split_delimiters =
        include_str!("fixtures/streamdown_bench/math_with_split_delimiters.md");
    let multiple_html_blocks = include_str!("fixtures/streamdown_bench/html_multiple_blocks.md");
    let with_footnotes = include_str!("fixtures/streamdown_bench/footnotes_with_footnotes.md");
    let simple_table = include_str!("fixtures/streamdown_bench/table_simple.md");

    let opts = Options::default();
    assert_invariant("single_block", single_block, opts.clone(), 16, 64);
    assert_invariant(
        "multiple_blocks_10",
        multiple_blocks_10,
        opts.clone(),
        16,
        64,
    );
    assert_invariant("single_code_block", single_code_block, opts.clone(), 16, 64);
    assert_invariant(
        "math_with_split_delimiters",
        math_with_split_delimiters,
        opts.clone(),
        16,
        64,
    );
    assert_invariant(
        "multiple_html_blocks",
        multiple_html_blocks,
        opts.clone(),
        16,
        64,
    );
    assert_invariant("with_footnotes", with_footnotes, opts.clone(), 16, 64);
    assert_invariant("simple_table", simple_table, opts.clone(), 16, 64);
}

#[test]
fn incremark_inspired_suite_chunking_invariance() {
    // Inputs inspired by Incremark's `IncremarkParser.*.test.ts`.
    let paragraph = "Hello, World!";
    let multiple_paragraphs = "第一段\n\n第二段";
    let headings = "# 标题一\n\n## 标题二\n\n内容";
    let code_block = "```js\nconsole.log(\"hi\")\n```\n\n段落";
    let gfm_table = "| A | B |\n|---|---|\n| 1 | 2 |";

    let opts = Options::default();
    assert_invariant("incremark_paragraph", paragraph, opts.clone(), 8, 32);
    assert_invariant(
        "incremark_multiple_paragraphs",
        multiple_paragraphs,
        opts.clone(),
        8,
        32,
    );
    assert_invariant("incremark_headings", headings, opts.clone(), 8, 32);
    assert_invariant("incremark_code_block", code_block, opts.clone(), 8, 32);
    assert_invariant("incremark_gfm_table", gfm_table, opts.clone(), 8, 32);
}

#[test]
fn chunking_invariance_handles_crlf_split_across_chunks() {
    let opts = Options::default();
    let markdown = "A\r\n\r\nB\r\n";

    let expected = support::collect_final_blocks(support::chunk_whole(markdown), opts.clone());
    let blocks_split_crlf = support::collect_final_blocks(
        vec![
            "A\r".to_string(),
            "\n\r".to_string(),
            "\nB\r".to_string(),
            "\n".to_string(),
        ],
        opts,
    );
    assert_eq!(blocks_split_crlf, expected);
}

#[test]
fn incomplete_table_delimiter_candidate_waits_for_newline() {
    let opts = Options::default();
    let markdown = "a# Heading\n|# Heading\n-->\n";

    let expected = support::collect_final_blocks(support::chunk_whole(markdown), opts.clone());
    let blocks_chars = support::collect_final_blocks(support::chunk_chars(markdown), opts);

    assert_eq!(blocks_chars, expected);
}

#[test]
fn checked_in_legacy_framing_goldens_hold_for_every_declared_schedule() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/fixtures");
    let fixtures = load_fixture_dir(fixture_dir).unwrap();
    let fixtures = fixtures
        .iter()
        .filter(|fixture| fixture.expected.legacy_framing.is_some())
        .collect::<Vec<_>>();
    assert!(
        !fixtures.is_empty(),
        "legacy golden corpus must not be empty"
    );

    for fixture in fixtures {
        let expected = fixture.expected.legacy_framing.as_ref().unwrap();
        let options = Options {
            footnotes: match fixture
                .options
                .get("footnotes")
                .and_then(|value| value.as_str())
            {
                Some("invalidate") => FootnotesMode::Invalidate,
                _ => FootnotesMode::SingleBlock,
            },
            ..Options::default()
        };
        for named in &fixture.schedules {
            let chunks = named
                .schedule
                .slices(&fixture.source)
                .unwrap()
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>();
            let actual = support::collect_final_blocks(chunks, options.clone())
                .into_iter()
                .map(|(kind, raw)| LegacyBlock {
                    kind: block_kind_name(kind).to_string(),
                    raw,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                &actual, expected,
                "fixture={} schedule={}",
                fixture.id, named.id
            );
        }
    }
}

fn block_kind_name(kind: BlockKind) -> &'static str {
    match kind {
        BlockKind::Paragraph => "paragraph",
        BlockKind::Heading => "heading",
        BlockKind::ThematicBreak => "thematic_break",
        BlockKind::CodeFence => "code_fence",
        BlockKind::List => "list",
        BlockKind::BlockQuote => "block_quote",
        BlockKind::Table => "table",
        BlockKind::HtmlBlock => "html_block",
        BlockKind::MathBlock => "math_block",
        BlockKind::FootnoteDefinition => "footnote_definition",
        BlockKind::Unknown => "unknown",
    }
}
