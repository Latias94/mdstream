use mdstream::{MdStream, Options};

#[test]
fn splits_paragraphs_on_blank_line() {
    let mut s = MdStream::new(Options::default());
    let u1 = s.append("A\n\nB");
    assert_eq!(u1.committed.len(), 1);
    assert_eq!(u1.committed[0].raw, "A\n\n");
    assert_eq!(u1.pending.as_ref().unwrap().raw, "B");
}

#[test]
fn commits_list_as_single_block() {
    let mut s = MdStream::new(Options::default());
    s.append("- a\n- b\n");
    let u = s.append("\nC\n");
    assert!(u.committed.iter().any(|b| b.raw.contains("- a\n- b\n")));
}

#[test]
fn commits_blockquote_as_single_block() {
    let mut s = MdStream::new(Options::default());
    s.append("> a\n> b\n");
    let u = s.append("\nC\n");
    assert!(u.committed.iter().any(|b| b.raw.contains("> a\n> b\n")));
}

#[test]
fn commits_table_as_single_block() {
    let mut s = MdStream::new(Options::default());
    s.append("| A | B |\n|---|---|\n| 1 | 2 |\n");
    let u = s.append("\nAfter\n");
    assert!(u.committed.iter().any(|b| b.raw.contains("| A | B |\n|---|---|\n| 1 | 2 |\n")));
}

#[test]
fn table_after_paragraph_is_separate_block() {
    let mut s = MdStream::new(Options::default());
    let u1 = s.append("Intro\n\n| A | B |\n|---|---|\n| 1 | 2 |\n");
    assert!(u1.committed.iter().any(|b| b.raw == "Intro\n\n"));
    assert!(!u1.committed.iter().any(|b| b.raw.contains("| A | B |")));
    // Header line should not be committed as a standalone paragraph.
    assert!(!u1.committed.iter().any(|b| b.raw == "| A | B |\n"));

    let u2 = s.append("\nAfter\n");
    assert!(u2
        .committed
        .iter()
        .any(|b| b.raw.contains("| A | B |\n|---|---|\n| 1 | 2 |\n")));
}

#[test]
fn commits_html_block_until_blank_line() {
    let mut s = MdStream::new(Options::default());
    s.append("<div>\nhello\n</div>\n");
    let u = s.append("\nAfter\n");
    assert!(u.committed.iter().any(|b| b.raw.contains("<div>\nhello\n</div>\n")));
}

#[test]
fn commits_math_block_as_single_block() {
    let mut s = MdStream::new(Options::default());
    s.append("$$\nx = 1\n");
    let u1 = s.append("y = 2\n");
    assert!(u1.committed.is_empty());
    let u2 = s.append("$$\n\nAfter\n");
    assert!(u2.committed.iter().any(|b| b.raw.contains("$$\nx = 1\ny = 2\n$$\n")));
}
