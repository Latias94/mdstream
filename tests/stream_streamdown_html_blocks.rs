mod support;

use mdstream::{BlockKind, Options};

#[test]
fn streamdown_benchmark_simple_html_block_chunking_invariance() {
    // From Streamdown's parse-blocks benchmark ("HTML Blocks").
    let markdown = include_str!("fixtures/streamdown_bench/html_simple.md");

    let opts = Options::default();
    let blocks_whole = support::collect_final_blocks(support::chunk_whole(markdown), opts.clone());
    let blocks_lines = support::collect_final_blocks(support::chunk_lines(markdown), opts.clone());
    let blocks_rand = support::collect_final_blocks(
        support::chunk_pseudo_random(
            markdown,
            "streamdown_benchmark_simple_html_block_chunking_invariance",
            0,
            40,
        ),
        opts,
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    assert_eq!(blocks_whole.len(), 1);
    assert_eq!(blocks_whole[0].0, BlockKind::HtmlBlock);
    assert!(blocks_whole[0].1.contains("<div>\n  <p>HTML content</p>\n</div>\n"));
}

#[test]
fn streamdown_benchmark_nested_html_block_chunking_invariance() {
    // From Streamdown's parse-blocks benchmark ("HTML Blocks").
    let markdown = include_str!("fixtures/streamdown_bench/html_nested.md");

    let opts = Options::default();
    let blocks_whole = support::collect_final_blocks(support::chunk_whole(markdown), opts.clone());
    let blocks_lines = support::collect_final_blocks(support::chunk_lines(markdown), opts.clone());
    let blocks_rand = support::collect_final_blocks(
        support::chunk_pseudo_random(
            markdown,
            "streamdown_benchmark_nested_html_block_chunking_invariance",
            0,
            40,
        ),
        opts,
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    assert_eq!(blocks_whole.len(), 1);
    assert_eq!(blocks_whole[0].0, BlockKind::HtmlBlock);
    assert!(blocks_whole[0].1.contains("<p>Nested content</p>"));
    assert!(blocks_whole[0].1.trim_end().ends_with("</div>"));
}

#[test]
fn streamdown_benchmark_multiple_html_blocks_chunking_invariance() {
    // From Streamdown's parse-blocks benchmark ("HTML Blocks").
    let markdown = include_str!("fixtures/streamdown_bench/html_multiple_blocks.md");

    let opts = Options::default();
    let blocks_whole = support::collect_final_blocks(support::chunk_whole(markdown), opts.clone());
    let blocks_lines = support::collect_final_blocks(support::chunk_lines(markdown), opts.clone());
    let blocks_rand = support::collect_final_blocks(
        support::chunk_pseudo_random(
            markdown,
            "streamdown_benchmark_multiple_html_blocks_chunking_invariance",
            0,
            40,
        ),
        opts,
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    let kinds: Vec<BlockKind> = blocks_whole.iter().map(|(k, _)| *k).collect();
    assert_eq!(
        kinds,
        vec![
            BlockKind::HtmlBlock,
            BlockKind::Paragraph,
            BlockKind::HtmlBlock,
            BlockKind::Paragraph,
        ]
    );

    assert!(blocks_whole[0].1.contains("<div>First block</div>"));
    assert!(blocks_whole[1].1.contains("Some markdown"));
    assert!(blocks_whole[2].1.contains("<section>"));
    assert!(blocks_whole[2].1.contains("</section>"));
    assert!(blocks_whole[3].1.contains("More markdown"));
}
