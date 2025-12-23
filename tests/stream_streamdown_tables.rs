mod support;

use mdstream::{BlockKind, Options};

#[test]
fn streamdown_benchmark_simple_table_chunking_invariance() {
    // From Streamdown's parse-blocks benchmark ("Tables").
    let markdown = include_str!("fixtures/streamdown_bench/table_simple.md");

    let opts = Options::default();
    let blocks_whole = support::collect_final_blocks(support::chunk_whole(markdown), opts.clone());
    let blocks_lines = support::collect_final_blocks(support::chunk_lines(markdown), opts.clone());
    let blocks_rand = support::collect_final_blocks(
        support::chunk_pseudo_random(
            markdown,
            "streamdown_benchmark_simple_table_chunking_invariance",
            0,
            40,
        ),
        opts,
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    assert_eq!(blocks_whole.len(), 1);
    assert_eq!(blocks_whole[0].0, BlockKind::Table);
    assert!(blocks_whole[0].1.contains("| Header 1 | Header 2 |"));
}

#[test]
fn streamdown_benchmark_large_table_chunking_invariance() {
    // From Streamdown's parse-blocks benchmark ("Tables").
    let markdown = include_str!("fixtures/streamdown_bench/table_large_100_rows.md");

    let opts = Options::default();
    let blocks_whole = support::collect_final_blocks(support::chunk_whole(markdown), opts.clone());
    let blocks_lines = support::collect_final_blocks(support::chunk_lines(markdown), opts.clone());
    let blocks_rand = support::collect_final_blocks(
        support::chunk_pseudo_random(
            markdown,
            "streamdown_benchmark_large_table_chunking_invariance",
            0,
            40,
        ),
        opts,
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    assert_eq!(blocks_whole.len(), 1);
    assert_eq!(blocks_whole[0].0, BlockKind::Table);
    assert!(
        blocks_whole[0]
            .1
            .contains("| C991 | C992 | C993 | C994 | C995 |")
    );
}
