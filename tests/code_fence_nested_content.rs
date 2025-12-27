mod support;

use mdstream::{BlockKind, Options};

/// Bug fix: CodeFence blocks include all content between fences in `raw`.
///
/// Previously, the opening fence line was incorrectly committed as its own block
/// because `fence_end()` matched on the first line. Fix: skip fence_end check
/// on the block's start line.
#[test]
fn code_fence_with_inner_backticks_is_single_block() {
    let markdown = "````\nState: Normal\n  → see ``` → State: Fence\n````\n";

    let opts = Options::default();
    let blocks = support::collect_final_blocks(support::chunk_whole(markdown), opts);

    assert_eq!(blocks.len(), 1, "Expected 1 block, got {:?}", blocks);
    assert_eq!(blocks[0].0, BlockKind::CodeFence);
    assert_eq!(blocks[0].1, markdown);
}

/// Chunking invariance: same result whether fed whole, by lines, or by chars
#[test]
fn code_fence_chunking_invariance() {
    let markdown = "````\nState: Normal\n  → see ``` → State: Fence\n````\n";

    let opts = Options::default();
    let blocks_whole = support::collect_final_blocks(support::chunk_whole(markdown), opts.clone());
    let blocks_lines = support::collect_final_blocks(support::chunk_lines(markdown), opts.clone());
    let blocks_chars = support::collect_final_blocks(support::chunk_chars(markdown), opts);

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_chars, blocks_whole);
}

/// 4-backtick fence containing 3-backtick content works correctly
#[test]
fn code_fence_nested_inner_fence() {
    let markdown = "````\n```rust\nfn main() {}\n```\n````\n";

    let opts = Options::default();
    let blocks = support::collect_final_blocks(support::chunk_whole(markdown), opts);

    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].0, BlockKind::CodeFence);
    assert!(blocks[0].1.contains("```rust"));
}

/// CommonMark spec: closing fence must be >= opening fence length.
/// So 4 backticks WILL close a 3-backtick fence. This is correct behavior.
#[test]
fn commonmark_fence_closing_behavior() {
    // 3-backtick outer, 4-backtick inner: the inner CLOSES the outer per spec
    let markdown = "```markdown\n````\ncontent\n````\n```\n";

    let opts = Options::default();
    let blocks = support::collect_final_blocks(support::chunk_whole(markdown), opts);

    // This produces multiple blocks - correct per CommonMark
    assert!(blocks.len() > 1, "4 backticks close 3-backtick fence per spec");
}

/// To embed N-backtick examples, use N+1 backticks on outer fence
#[test]
fn proper_fence_nesting() {
    // 5-backtick outer can contain 4-backtick content
    let markdown = "`````markdown\n````\ninner\n````\n`````\n";

    let opts = Options::default();
    let blocks = support::collect_final_blocks(support::chunk_whole(markdown), opts);

    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].0, BlockKind::CodeFence);
    assert!(blocks[0].1.contains("````"));
}

/// Tilde fences work the same way
#[test]
fn code_fence_tilde() {
    let markdown = "~~~~\ncode with ~~not strikethrough~~\n~~~~\n";

    let opts = Options::default();
    let blocks = support::collect_final_blocks(support::chunk_whole(markdown), opts);

    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].0, BlockKind::CodeFence);
}
