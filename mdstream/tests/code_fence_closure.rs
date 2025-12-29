mod support;

use mdstream::{BlockKind, Options};

#[test]
fn code_fence_opening_line_does_not_close_itself() {
    let markdown = "```\ncode\n```\n";
    let blocks = support::collect_final_blocks(support::chunk_whole(markdown), Options::default());
    assert_eq!(blocks.len(), 1, "expected 1 block, got {blocks:?}");
    assert_eq!(blocks[0].0, BlockKind::CodeFence);
    assert_eq!(blocks[0].1, markdown);
}

#[test]
fn code_fence_chunking_invariance_no_language() {
    let markdown = "```\ncode\n```\n";
    let opts = Options::default();
    let whole = support::collect_final_blocks(support::chunk_whole(markdown), opts.clone());
    let lines = support::collect_final_blocks(support::chunk_lines(markdown), opts.clone());
    let chars = support::collect_final_blocks(support::chunk_chars(markdown), opts);
    assert_eq!(lines, whole);
    assert_eq!(chars, whole);
}

#[test]
fn code_fence_with_inner_backticks_is_single_block() {
    let markdown = "````\nState: Normal\n  → see ``` → State: Fence\n````\n";
    let blocks = support::collect_final_blocks(support::chunk_whole(markdown), Options::default());
    assert_eq!(blocks.len(), 1, "expected 1 block, got {blocks:?}");
    assert_eq!(blocks[0].0, BlockKind::CodeFence);
    assert_eq!(blocks[0].1, markdown);
}

