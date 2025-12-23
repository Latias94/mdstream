use mdstream::{BlockKind, MdStream, Options};

fn collect_final_blocks(chunks: impl IntoIterator<Item = String>, opts: Options) -> Vec<(BlockKind, String)> {
    let mut s = MdStream::new(opts);
    let mut out = Vec::new();

    for chunk in chunks {
        let u = s.append(&chunk);
        out.extend(u.committed.into_iter().map(|b| (b.kind, b.raw)));
    }
    let u = s.finalize();
    out.extend(u.committed.into_iter().map(|b| (b.kind, b.raw)));
    out
}

fn chunk_whole(text: &str) -> Vec<String> {
    vec![text.to_string()]
}

fn chunk_lines(text: &str) -> Vec<String> {
    text.split_inclusive('\n').map(|s| s.to_string()).collect()
}

fn chunk_pseudo_random(text: &str, mut seed: u32) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < text.len() {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let want = (seed % 40 + 1) as usize; // 1..=40 bytes
        let mut end = (start + want).min(text.len());
        while end < text.len() && !text.is_char_boundary(end) {
            end += 1;
        }
        out.push(text[start..end].to_string());
        start = end;
    }
    out
}

#[test]
fn list_allows_blank_line_between_items_chunking_invariance() {
    let markdown = "- item 1\n\n- item 2\n\nAfter\n";

    let opts = Options::default();
    let blocks_whole = collect_final_blocks(chunk_whole(markdown), opts.clone());
    let blocks_lines = collect_final_blocks(chunk_lines(markdown), opts.clone());
    let blocks_rand = collect_final_blocks(chunk_pseudo_random(markdown, 1), opts.clone());

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
    let blocks_whole = collect_final_blocks(chunk_whole(markdown), opts.clone());
    let blocks_lines = collect_final_blocks(chunk_lines(markdown), opts.clone());
    let blocks_rand = collect_final_blocks(chunk_pseudo_random(markdown, 1), opts.clone());

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    assert_eq!(blocks_whole.len(), 2);
    assert_eq!(blocks_whole[0].0, BlockKind::List);
    assert!(blocks_whole[0].1.contains("- item 1\n  continued line\n- item 2\n\n"));
    assert_eq!(blocks_whole[1].1, "After\n");
}

#[test]
fn list_allows_blank_line_then_indented_continuation_chunking_invariance() {
    let markdown = "- item 1\n\n  continuation after blank\n- item 2\n\nAfter\n";

    let opts = Options::default();
    let blocks_whole = collect_final_blocks(chunk_whole(markdown), opts.clone());
    let blocks_lines = collect_final_blocks(chunk_lines(markdown), opts.clone());
    let blocks_rand = collect_final_blocks(chunk_pseudo_random(markdown, 1), opts.clone());

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    assert_eq!(blocks_whole.len(), 2);
    assert_eq!(blocks_whole[0].0, BlockKind::List);
    assert!(blocks_whole[0]
        .1
        .contains("- item 1\n\n  continuation after blank\n- item 2\n\n"));
    assert_eq!(blocks_whole[1].1, "After\n");
}

#[test]
fn task_list_items_are_stable_chunking_invariance() {
    let markdown = "- [x] done\n  more\n- [ ] todo\n\nAfter\n";

    let opts = Options::default();
    let blocks_whole = collect_final_blocks(chunk_whole(markdown), opts.clone());
    let blocks_lines = collect_final_blocks(chunk_lines(markdown), opts.clone());
    let blocks_rand = collect_final_blocks(chunk_pseudo_random(markdown, 1), opts.clone());

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    assert_eq!(blocks_whole.len(), 2);
    assert_eq!(blocks_whole[0].0, BlockKind::List);
    assert!(blocks_whole[0].1.contains("- [x] done\n  more\n- [ ] todo\n\n"));
    assert_eq!(blocks_whole[1].1, "After\n");
}

#[test]
fn blockquote_lazy_continuation_is_stable_chunking_invariance() {
    let markdown = "> quote line 1\nlazy continuation\n> quote line 2\n\nAfter\n";

    let opts = Options::default();
    let blocks_whole = collect_final_blocks(chunk_whole(markdown), opts.clone());
    let blocks_lines = collect_final_blocks(chunk_lines(markdown), opts.clone());
    let blocks_rand = collect_final_blocks(chunk_pseudo_random(markdown, 1), opts.clone());

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    assert_eq!(blocks_whole.len(), 2);
    assert_eq!(blocks_whole[0].0, BlockKind::BlockQuote);
    assert!(blocks_whole[0]
        .1
        .contains("> quote line 1\nlazy continuation\n> quote line 2\n\n"));
    assert_eq!(blocks_whole[1].1, "After\n");
}

