mod support;

use mdstream::{BlockKind, Options};

#[test]
fn streamdown_benchmark_single_code_block() {
    // From Streamdown's parse-blocks benchmark ("single code block").
    let markdown = include_str!("fixtures/streamdown_bench/code_single_code_block.md");

    let opts = Options::default();
    let blocks_whole = support::collect_final_blocks(support::chunk_whole(markdown), opts.clone());
    let blocks_lines = support::collect_final_blocks(support::chunk_lines(markdown), opts.clone());
    let blocks_rand = support::collect_final_blocks(
        support::chunk_pseudo_random(markdown, "streamdown_benchmark_single_code_block", 0, 40),
        opts.clone(),
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    assert_eq!(blocks_whole.len(), 3);
    assert_eq!(blocks_whole[0].0, BlockKind::Paragraph);
    assert_eq!(blocks_whole[0].1, "Some text\n\n");
    assert_eq!(blocks_whole[1].0, BlockKind::CodeFence);
    assert!(blocks_whole[1].1.contains("```javascript\n"));
    assert!(blocks_whole[1].1.contains("const y = 2;\n"));
    assert!(blocks_whole[1].1.contains("```\n"));
    assert_eq!(blocks_whole[2].0, BlockKind::Paragraph);
    assert_eq!(blocks_whole[2].1, "More text\n");
}

#[test]
fn streamdown_benchmark_multiple_code_blocks() {
    // From Streamdown's parse-blocks benchmark ("multiple code blocks").
    let markdown = include_str!("fixtures/streamdown_bench/code_multiple_code_blocks.md");

    let opts = Options::default();
    let blocks_whole = support::collect_final_blocks(support::chunk_whole(markdown), opts.clone());
    let blocks_lines = support::collect_final_blocks(support::chunk_lines(markdown), opts.clone());
    let blocks_rand = support::collect_final_blocks(
        support::chunk_pseudo_random(markdown, "streamdown_benchmark_multiple_code_blocks", 0, 40),
        opts.clone(),
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    assert_eq!(blocks_whole.len(), 3);
    assert!(blocks_whole.iter().all(|(k, _)| *k == BlockKind::CodeFence));
    assert!(blocks_whole[0].1.contains("```javascript\n"));
    assert!(blocks_whole[1].1.contains("```python\n"));
    assert!(blocks_whole[2].1.contains("```rust\n"));
}

#[test]
fn streamdown_benchmark_large_code_block_1000_lines() {
    // From Streamdown's parse-blocks benchmark ("large code block (1000 lines)").
    let mut markdown = String::new();
    markdown.push_str("```javascript\n");
    for _ in 0..1000 {
        markdown.push_str("const x = 1;\n");
    }
    markdown.push_str("```");

    let opts = Options::default();
    let blocks_whole = support::collect_final_blocks(support::chunk_whole(&markdown), opts.clone());
    let blocks_lines = support::collect_final_blocks(support::chunk_lines(&markdown), opts.clone());
    let blocks_rand = support::collect_final_blocks(
        support::chunk_pseudo_random(
            &markdown,
            "streamdown_benchmark_large_code_block_1000_lines",
            0,
            40,
        ),
        opts.clone(),
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    assert_eq!(blocks_whole.len(), 1);
    assert_eq!(blocks_whole[0].0, BlockKind::CodeFence);
    assert!(blocks_whole[0].1.starts_with("```javascript\n"));
    assert!(blocks_whole[0].1.ends_with("```"));
}
