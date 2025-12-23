use mdstream::{BlockKind, MdStream, Options};

#[test]
fn streamdown_benchmark_streaming_text_50_steps() {
    let mut s = MdStream::new(Options::default());

    let u0 = s.append("# Heading\n\n");
    assert_eq!(u0.committed.len(), 1);
    assert_eq!(u0.committed[0].raw, "# Heading\n");

    let mut acc = String::new();
    for _ in 0..50 {
        let u = s.append("This is streaming text. ");
        assert!(u.committed.is_empty());
        acc.push_str("This is streaming text. ");
        assert_eq!(u.pending.as_ref().unwrap().kind, BlockKind::Paragraph);
        assert_eq!(u.pending.as_ref().unwrap().raw, acc);
    }

    let uf = s.finalize();
    assert_eq!(uf.committed.len(), 1);
    assert_eq!(uf.committed[0].raw, acc);
}

#[test]
fn streamdown_benchmark_streaming_code_block_9_steps() {
    let mut s = MdStream::new(Options::default());

    // Deltas between Streamdown's "full text per step" examples.
    let deltas = ["```javascript", "\n", "const", " x", " =", " 1", ";", "\n", "```"];

    let mut acc = String::new();
    for (i, d) in deltas.iter().enumerate() {
        let u = s.append(d);
        acc.push_str(d);
        assert!(u.committed.is_empty(), "step {i} should not commit");
        assert_eq!(u.pending.as_ref().unwrap().raw, acc, "step {i} raw mismatch");
    }

    // The closing fence line may not end with a newline; we only commit at finalize.
    let uf = s.finalize();
    assert_eq!(uf.committed.len(), 1);
    assert_eq!(uf.committed[0].kind, BlockKind::CodeFence);
    assert_eq!(uf.committed[0].raw, acc);
}

