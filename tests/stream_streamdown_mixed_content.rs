mod support;

use mdstream::{BlockKind, Options};

#[test]
fn streamdown_benchmark_realistic_ai_response_chunking_invariance() {
    // From Streamdown's parse-blocks benchmark ("Mixed Content").
    let markdown = include_str!("fixtures/streamdown_bench/mixed_content_realistic.md");

    let opts = Options::default();
    let blocks_whole = support::collect_final_raw(support::chunk_whole(markdown), opts.clone());
    let blocks_lines = support::collect_final_raw(support::chunk_lines(markdown), opts.clone());
    let blocks_rand = support::collect_final_raw(
        support::chunk_pseudo_random(
            markdown,
            "streamdown_benchmark_realistic_ai_response_chunking_invariance",
            0,
            40,
        ),
        opts.clone(),
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    // Minimal structural assertions: keep the benchmark behavior locked in without overfitting raw
    // whitespace details.
    let blocks_whole = support::collect_final_blocks(support::chunk_whole(markdown), opts);
    let kinds: Vec<BlockKind> = blocks_whole.iter().map(|(k, _)| *k).collect();
    assert_eq!(
        kinds,
        vec![
            BlockKind::Heading,
            BlockKind::Paragraph,
            BlockKind::Heading,
            BlockKind::CodeFence,
            BlockKind::Heading,
            BlockKind::Paragraph,
            BlockKind::MathBlock,
            BlockKind::Heading,
            BlockKind::Table,
            BlockKind::Heading,
            BlockKind::List,
            BlockKind::BlockQuote,
            BlockKind::Paragraph,
        ]
    );
}
