use mdstream::{FootnotesMode, MdStream, Options};

fn opts_invalidate() -> Options {
    Options {
        footnotes: FootnotesMode::Invalidate,
        ..Default::default()
    }
}

#[test]
fn footnote_definition_does_not_commit_after_first_line() {
    let mut s = MdStream::new(opts_invalidate());

    let u1 = s.append("[^1]: 第一行\n");
    assert!(u1.committed.is_empty());
    assert!(u1.pending.is_some());

    let u2 = s.append("    第二行（缩进）\n");
    assert!(u2.committed.is_empty());
    assert!(u2.pending.is_some());

    // Non-indented line ends the footnote definition even without a blank line.
    let u3 = s.append("普通段落\n");
    assert!(u3.committed.iter().any(|b| {
        b.raw == "[^1]: 第一行\n    第二行（缩进）\n"
            && b.kind == mdstream::BlockKind::FootnoteDefinition
    }));
    assert_eq!(u3.pending.as_ref().unwrap().raw, "普通段落\n");
}

#[test]
fn footnote_definition_allows_blank_line_then_indented_paragraph() {
    let mut s = MdStream::new(opts_invalidate());

    s.append("[^test]: 第一段\n");
    let u2 = s.append("\n    第二段（缩进）\n");
    assert!(u2.committed.is_empty());

    let u3 = s.append("After\n");
    assert!(u3.committed.iter().any(|b| {
        b.raw == "[^test]: 第一段\n\n    第二段（缩进）\n"
            && b.kind == mdstream::BlockKind::FootnoteDefinition
    }));
    assert_eq!(u3.pending.as_ref().unwrap().raw, "After\n");
}

#[test]
fn new_footnote_definition_ends_previous_one() {
    let mut s = MdStream::new(opts_invalidate());

    s.append("[^1]: line1\n");
    s.append("    line2\n");
    let u3 = s.append("[^2]: new\n");

    assert!(u3.committed.iter().any(|b| {
        b.raw == "[^1]: line1\n    line2\n" && b.kind == mdstream::BlockKind::FootnoteDefinition
    }));
    assert!(u3.pending.is_some());
    assert_eq!(u3.pending.as_ref().unwrap().raw, "[^2]: new\n");
}
