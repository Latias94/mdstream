mod support;

use mdstream::{BlockKind, Options};

#[test]
fn list_allows_blank_line_between_items_chunking_invariance() {
    let markdown = "- item 1\n\n- item 2\n\nAfter\n";

    let opts = Options::default();
    let blocks_whole = support::collect_final_blocks(support::chunk_whole(markdown), opts.clone());
    let blocks_lines = support::collect_final_blocks(support::chunk_lines(markdown), opts.clone());
    let blocks_rand = support::collect_final_blocks(
        support::chunk_pseudo_random(markdown, "list_blank_line_between_items", 0, 40),
        opts.clone(),
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    assert_eq!(blocks_whole.len(), 2);
    assert_eq!(blocks_whole[0].0, BlockKind::List);
    assert!(blocks_whole[0].1.contains("- item 1\n\n- item 2\n\n"));
    assert_eq!(blocks_whole[1].0, BlockKind::Paragraph);
    assert_eq!(blocks_whole[1].1, "After\n");
}

#[test]
fn list_allows_multiline_item_then_next_item_chunking_invariance() {
    let markdown = "- item 1\n  continued line\n- item 2\n\nAfter\n";

    let opts = Options::default();
    let blocks_whole = support::collect_final_blocks(support::chunk_whole(markdown), opts.clone());
    let blocks_lines = support::collect_final_blocks(support::chunk_lines(markdown), opts.clone());
    let blocks_rand = support::collect_final_blocks(
        support::chunk_pseudo_random(markdown, "list_multiline_item_then_next_item", 0, 40),
        opts.clone(),
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    assert_eq!(blocks_whole.len(), 2);
    assert_eq!(blocks_whole[0].0, BlockKind::List);
    assert!(
        blocks_whole[0]
            .1
            .contains("- item 1\n  continued line\n- item 2\n\n")
    );
    assert_eq!(blocks_whole[1].1, "After\n");
}

#[test]
fn list_allows_blank_line_then_indented_continuation_chunking_invariance() {
    let markdown = "- item 1\n\n  continuation after blank\n- item 2\n\nAfter\n";

    let opts = Options::default();
    let blocks_whole = support::collect_final_blocks(support::chunk_whole(markdown), opts.clone());
    let blocks_lines = support::collect_final_blocks(support::chunk_lines(markdown), opts.clone());
    let blocks_rand = support::collect_final_blocks(
        support::chunk_pseudo_random(
            markdown,
            "list_blank_line_then_indented_continuation",
            0,
            40,
        ),
        opts.clone(),
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    assert_eq!(blocks_whole.len(), 2);
    assert_eq!(blocks_whole[0].0, BlockKind::List);
    assert!(
        blocks_whole[0]
            .1
            .contains("- item 1\n\n  continuation after blank\n- item 2\n\n")
    );
    assert_eq!(blocks_whole[1].1, "After\n");
}

#[test]
fn task_list_items_are_stable_chunking_invariance() {
    let markdown = "- [x] done\n  more\n- [ ] todo\n\nAfter\n";

    let opts = Options::default();
    let blocks_whole = support::collect_final_blocks(support::chunk_whole(markdown), opts.clone());
    let blocks_lines = support::collect_final_blocks(support::chunk_lines(markdown), opts.clone());
    let blocks_rand = support::collect_final_blocks(
        support::chunk_pseudo_random(markdown, "task_list_items_are_stable", 0, 40),
        opts.clone(),
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    assert_eq!(blocks_whole.len(), 2);
    assert_eq!(blocks_whole[0].0, BlockKind::List);
    assert!(
        blocks_whole[0]
            .1
            .contains("- [x] done\n  more\n- [ ] todo\n\n")
    );
    assert_eq!(blocks_whole[1].1, "After\n");
}

#[test]
fn blockquote_lazy_continuation_is_stable_chunking_invariance() {
    let markdown = "> quote line 1\nlazy continuation\n> quote line 2\n\nAfter\n";

    let opts = Options::default();
    let blocks_whole = support::collect_final_blocks(support::chunk_whole(markdown), opts.clone());
    let blocks_lines = support::collect_final_blocks(support::chunk_lines(markdown), opts.clone());
    let blocks_rand = support::collect_final_blocks(
        support::chunk_pseudo_random(markdown, "blockquote_lazy_continuation_is_stable", 0, 40),
        opts.clone(),
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    assert_eq!(blocks_whole.len(), 2);
    assert_eq!(blocks_whole[0].0, BlockKind::BlockQuote);
    assert!(
        blocks_whole[0]
            .1
            .contains("> quote line 1\nlazy continuation\n> quote line 2\n\n")
    );
    assert_eq!(blocks_whole[1].1, "After\n");
}
