use mdstream::{MdStream, Options};

#[test]
fn heading_commits_on_newline() {
    let mut s = MdStream::new(Options::default());
    let u = s.append("# Title\n");
    assert_eq!(u.committed.len(), 1);
    assert!(u.pending.is_none() || u.pending.as_ref().unwrap().raw.is_empty());
}

#[test]
fn fenced_code_is_committed_as_a_whole() {
    let mut s = MdStream::new(Options::default());

    let u1 = s.append("```js\n");
    assert!(u1.committed.is_empty());
    assert!(u1.pending.is_some());

    let u2 = s.append("console.log(1)\n");
    assert!(u2.committed.is_empty());
    assert!(u2.pending.is_some());

    let u3 = s.append("```\n");
    assert_eq!(u3.committed.len(), 1);
    assert!(u3.committed[0].raw.contains("console.log(1)"));
}

#[test]
fn footnotes_single_block_mode() {
    let mut s = MdStream::new(Options::default());
    let u1 = s.append("Testing this[^1] out.\n");
    assert!(u1.committed.is_empty());
    assert!(u1.pending.is_some());
    assert_eq!(u1.pending.as_ref().unwrap().raw, "Testing this[^1] out.\n");

    let u2 = s.append("\n[^1]: Footnote.\n");
    assert!(u2.committed.is_empty());
    assert_eq!(
        u2.pending.as_ref().unwrap().raw,
        "Testing this[^1] out.\n\n[^1]: Footnote.\n"
    );

    let u3 = s.finalize();
    assert_eq!(u3.committed.len(), 1);
    assert!(u3.pending.is_none());
}

