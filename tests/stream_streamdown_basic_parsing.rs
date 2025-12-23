mod support;

use mdstream::Options;

#[test]
fn streamdown_benchmark_single_block() {
    let markdown = include_str!("fixtures/streamdown_bench/basic_single_block.md")
        .trim_end_matches('\n');

    let opts = Options::default();
    let blocks_whole = support::collect_final_raw(support::chunk_whole(markdown), opts.clone());
    let blocks_lines = support::collect_final_raw(support::chunk_lines(markdown), opts.clone());
    let blocks_rand = support::collect_final_raw(
        support::chunk_pseudo_random(markdown, "streamdown_benchmark_single_block", 0, 40),
        opts.clone(),
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    assert_eq!(
        blocks_whole,
        vec![
            "# Heading\n".to_string(),
            "This is a paragraph.".to_string()
        ]
    );
}

#[test]
fn streamdown_benchmark_multiple_blocks_10() {
    // From Streamdown's parse-blocks benchmark ("multiple blocks (10)").
    let markdown = include_str!("fixtures/streamdown_bench/basic_multiple_blocks_10.md");

    let opts = Options::default();
    let blocks_whole = support::collect_final_raw(support::chunk_whole(markdown), opts.clone());
    let blocks_lines = support::collect_final_raw(support::chunk_lines(markdown), opts.clone());
    let blocks_rand = support::collect_final_raw(
        support::chunk_pseudo_random(markdown, "streamdown_benchmark_multiple_blocks_10", 0, 40),
        opts.clone(),
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    // Sanity checks: key blocks exist and order is stable.
    assert!(blocks_whole.iter().any(|b| b == "# Heading 1\n"));
    assert!(
        blocks_whole
            .iter()
            .any(|b| b.contains("This is paragraph 1."))
    );
    assert!(blocks_whole.iter().any(|b| b == "## Heading 2\n"));
    assert!(blocks_whole.iter().any(|b| b.contains("- List item 1")));
    assert!(blocks_whole.iter().any(|b| b.contains("> Blockquote text")));
}

#[test]
fn streamdown_benchmark_many_blocks_100() {
    // From Streamdown's parse-blocks benchmark ("many blocks (100)").
    let markdown = include_str!("fixtures/streamdown_bench/basic_many_blocks_100.md");

    let opts = Options::default();
    let blocks_whole = support::collect_final_raw(support::chunk_whole(markdown), opts.clone());
    let blocks_lines = support::collect_final_raw(support::chunk_lines(markdown), opts.clone());
    let blocks_rand = support::collect_final_raw(
        support::chunk_pseudo_random(markdown, "streamdown_benchmark_many_blocks_100", 0, 40),
        opts.clone(),
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    // Each section yields one heading block and one paragraph block.
    assert_eq!(blocks_whole.len(), 200);
    assert_eq!(blocks_whole[0], "## Section 0\n");
    assert!(blocks_whole[1].starts_with("Paragraph 0"));
    assert!(blocks_whole[blocks_whole.len() - 2].starts_with("## Section 99"));
    assert!(blocks_whole[blocks_whole.len() - 1].starts_with("Paragraph 99"));
}
