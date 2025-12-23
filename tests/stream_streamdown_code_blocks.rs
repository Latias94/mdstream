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
fn streamdown_benchmark_single_code_block() {
    // From Streamdown's parse-blocks benchmark ("single code block").
    let markdown = "Some text\n\n```javascript\nconst x = 1;\nconst y = 2;\n```\n\nMore text\n";

    let opts = Options::default();
    let blocks_whole = collect_final_blocks(chunk_whole(markdown), opts.clone());
    let blocks_lines = collect_final_blocks(chunk_lines(markdown), opts.clone());
    let blocks_rand = collect_final_blocks(chunk_pseudo_random(markdown, 1), opts.clone());

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
    let markdown = "```javascript\nconst x = 1;\n```\n\n```python\ny = 2\n```\n\n```rust\nlet z = 3;\n```\n";

    let opts = Options::default();
    let blocks_whole = collect_final_blocks(chunk_whole(markdown), opts.clone());
    let blocks_lines = collect_final_blocks(chunk_lines(markdown), opts.clone());
    let blocks_rand = collect_final_blocks(chunk_pseudo_random(markdown, 1), opts.clone());

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
    let blocks_whole = collect_final_blocks(chunk_whole(&markdown), opts.clone());
    let blocks_lines = collect_final_blocks(chunk_lines(&markdown), opts.clone());
    let blocks_rand = collect_final_blocks(chunk_pseudo_random(&markdown, 1), opts.clone());

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    assert_eq!(blocks_whole.len(), 1);
    assert_eq!(blocks_whole[0].0, BlockKind::CodeFence);
    assert!(blocks_whole[0].1.starts_with("```javascript\n"));
    assert!(blocks_whole[0].1.ends_with("```"));
}

