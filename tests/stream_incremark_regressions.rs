use mdstream::{MdStream, Options};

fn collect_final_blocks(chunks: impl IntoIterator<Item = String>, opts: Options) -> Vec<String> {
    let mut s = MdStream::new(opts);
    let mut out = Vec::new();

    for chunk in chunks {
        let u = s.append(&chunk);
        out.extend(u.committed.into_iter().map(|b| b.raw));
    }
    let u = s.finalize();
    out.extend(u.committed.into_iter().map(|b| b.raw));
    out
}

fn chunk_whole(text: &str) -> Vec<String> {
    vec![text.to_string()]
}

fn chunk_lines(text: &str) -> Vec<String> {
    text.split_inclusive('\n').map(|s| s.to_string()).collect()
}

fn chunk_chars(text: &str) -> Vec<String> {
    text.chars().map(|c| c.to_string()).collect()
}

fn chunk_pseudo_random(text: &str, mut seed: u32) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < text.len() {
        // LCG
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
fn newline_normalization_crlf_across_chunk_boundary() {
    let mut s = MdStream::new(Options::default());
    s.append("a\r");
    s.append("\n");
    s.finalize();
    assert_eq!(s.buffer(), "a\n");

    let mut s = MdStream::new(Options::default());
    s.append("a\r");
    s.finalize();
    assert_eq!(s.buffer(), "a\n");

    let mut s = MdStream::new(Options::default());
    s.append("a\r\nb");
    s.finalize();
    assert_eq!(s.buffer(), "a\nb");
}

#[test]
fn chunking_invariance_for_block_splitting() {
    let markdown = r#"# H1

Paragraph with **bold** and *italic*.

```rust
fn main() {
    println!("hi");
}
```

~~~txt
fenced with tildes
~~~

| A | B |
|---|---|
| 1 | 2 |

> Quote line 1
> Quote line 2

- item 1
- item 2

$$
E = mc^2
$$

<div>
HTML block
</div>

Final paragraph."#;

    let opts = Options::default();
    let blocks_whole = collect_final_blocks(chunk_whole(markdown), opts.clone());
    let blocks_lines = collect_final_blocks(chunk_lines(markdown), opts.clone());
    let blocks_chars = collect_final_blocks(chunk_chars(markdown), opts.clone());
    let blocks_rand = collect_final_blocks(chunk_pseudo_random(markdown, 1), opts.clone());

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_chars, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);
}

#[test]
fn reset_clears_state() {
    let mut s = MdStream::new(Options::default());
    s.append("# Title\n\nBody");
    s.finalize();
    assert!(!s.buffer().is_empty());

    s.reset();
    assert_eq!(s.buffer(), "");
    assert!(s.snapshot_blocks().is_empty());
}

#[test]
fn footnote_detection_across_chunk_boundaries_enters_single_block_mode() {
    let mut s = MdStream::new(Options::default());

    let u1 = s.append("This is a footnote ref [^");
    assert!(u1.committed.is_empty());
    assert!(u1.pending.is_some());

    let u2 = s.append("1] and more.\n");
    assert!(u2.committed.is_empty());
    let pending = u2.pending.expect("pending");
    assert_eq!(pending.id.0, 1);
    assert!(pending.raw.contains("[^1]"));

    let u3 = s.append("\n[^1]: definition\n");
    assert!(u3.committed.is_empty());
    let pending = u3.pending.expect("pending");
    assert_eq!(pending.id.0, 1);
    assert!(pending.raw.contains("[^1]:"));

    let u4 = s.finalize();
    assert_eq!(u4.committed.len(), 1);
    assert!(u4.pending.is_none());
}

#[test]
fn invalid_footnote_syntax_does_not_trigger_single_block_mode() {
    let mut s = MdStream::new(Options::default());

    let u = s.append("This is not a footnote[^ 1].\n\nAfter\n");
    assert!(u.committed.iter().any(|b| b.raw == "This is not a footnote[^ 1].\n\n"));
    assert_eq!(u.pending.as_ref().unwrap().raw, "After\n");
}
