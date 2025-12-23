mod support;

use mdstream::{MdStream, Options};

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
    let blocks_whole = support::collect_final_blocks(support::chunk_whole(markdown), opts.clone());
    let blocks_lines = support::collect_final_blocks(support::chunk_lines(markdown), opts.clone());
    let blocks_chars = support::collect_final_blocks(support::chunk_chars(markdown), opts.clone());
    let blocks_rand = support::collect_final_blocks(
        support::chunk_pseudo_random(markdown, "chunking_invariance_for_block_splitting", 0, 40),
        opts.clone(),
    );

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
    assert!(
        u.committed
            .iter()
            .any(|b| b.raw == "This is not a footnote[^ 1].\n\n")
    );
    assert_eq!(u.pending.as_ref().unwrap().raw, "After\n");
}
