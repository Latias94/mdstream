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
fn splits_streamdown_benchmark_document_with_footnotes_as_single_pending_block() {
    let mut s = MdStream::new(Options::default());
    let input = "This is text with a footnote[^1].\n\nHere's another footnote[^note].\n\n[^1]: This is the first footnote.\n[^note]: This is a named footnote.\n";
    let u = s.append(input);

    assert!(u.committed.is_empty());
    let pending = u.pending.expect("pending");
    assert_eq!(pending.id.0, 1);
    assert_eq!(pending.raw, input);
}

#[test]
fn splits_streamdown_benchmark_document_with_many_footnotes_as_single_pending_block() {
    let mut s = MdStream::new(Options::default());
    let mut input = String::new();
    input.push_str("Text[^1] with[^2] many[^3] footnotes[^4].\n\n");
    for i in 0..10 {
        input.push_str(&format!("[^{}]: Footnote {}\n", i + 1, i + 1));
    }
    let u = s.append(&input);

    assert!(u.committed.is_empty());
    let pending = u.pending.expect("pending");
    assert_eq!(pending.id.0, 1);
    assert_eq!(pending.raw, input);
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
fn splits_streamdown_benchmark_simple_table() {
    let mut s = MdStream::new(Options::default());
    let input = "| Header 1 | Header 2 |\n|----------|----------|\n| Cell 1   | Cell 2   |\n| Cell 3   | Cell 4   |\n\nAfter\n";
    let u = s.append(input);
    assert!(u.committed.iter().any(|b| {
        b.kind == mdstream::BlockKind::Table
            && b.raw.contains("| Header 1 | Header 2 |\n|----------|----------|\n")
            && b.raw.contains("| Cell 3   | Cell 4   |\n")
    }));
    assert_eq!(u.pending.as_ref().unwrap().raw, "After\n");
}

#[test]
fn splits_streamdown_benchmark_large_table() {
    let mut s = MdStream::new(Options::default());
    let mut input = String::new();
    input.push_str("| H1 | H2 | H3 | H4 | H5 |\n");
    input.push_str("|----|----|----|----|-------|\n");
    for i in 0..100 {
        input.push_str(&format!(
            "| C{i}1 | C{i}2 | C{i}3 | C{i}4 | C{i}5 |\n"
        ));
    }
    input.push_str("\nAfter\n");

    let u = s.append(&input);
    assert!(u.committed.iter().any(|b| {
        b.kind == mdstream::BlockKind::Table
            && b.raw.contains("| H1 | H2 | H3 | H4 | H5 |\n")
            && b.raw.contains("| C991 | C992 | C993 | C994 | C995 |\n")
    }));
    assert_eq!(u.pending.as_ref().unwrap().raw, "After\n");
}

#[test]
fn commits_html_block_until_blank_line() {
    let mut s = MdStream::new(Options::default());
    let u1 = s.append("<div>\nhello\n</div>\n");
    assert!(u1.committed.iter().any(|b| b.raw.contains("<div>\nhello\n</div>\n")));
    let _ = s.append("\nAfter\n");
}

#[test]
fn commits_html_block_when_tag_stack_closes_without_blank_line() {
    let mut s = MdStream::new(Options::default());
    let u = s.append("<div>\nhello\n</div>\nAfter");
    assert!(u.committed.iter().any(|b| b.raw == "<div>\nhello\n</div>\n"));
    assert_eq!(u.pending.as_ref().unwrap().raw, "After");
}

#[test]
fn commits_nested_html_block_when_stack_closes() {
    let mut s = MdStream::new(Options::default());
    let u = s.append("<div>\n<span>\nhi\n</span>\n</div>\nAfter");
    assert!(u
        .committed
        .iter()
        .any(|b| b.raw.contains("<div>\n<span>\nhi\n</span>\n</div>\n")));
    assert_eq!(u.pending.as_ref().unwrap().raw, "After");
}

#[test]
fn treats_html_comments_as_html_blocks() {
    let mut s = MdStream::new(Options::default());
    let u = s.append("<!--\nhello\n-->\nAfter");
    assert!(u.committed.iter().any(|b| b.raw == "<!--\nhello\n-->\n"));
    assert_eq!(u.pending.as_ref().unwrap().raw, "After");
}

#[test]
fn does_not_treat_autolink_as_html_block() {
    let mut s = MdStream::new(Options::default());
    let u = s.append("<https://example.com>\n\nAfter");
    // Should behave as normal paragraph split, not HTML block.
    assert!(u.committed.iter().any(|b| b.raw == "<https://example.com>\n\n"));
}

#[test]
fn splits_streamdown_benchmark_html_blocks() {
    let mut s = MdStream::new(Options::default());
    let input = "<div>\n  <p>HTML content</p>\n</div>\n\nAfter\n";
    let u = s.append(input);
    assert!(u
        .committed
        .iter()
        .any(|b| b.raw.contains("<div>\n  <p>HTML content</p>\n</div>\n")));
    assert_eq!(u.pending.as_ref().unwrap().raw, "After\n");
}

#[test]
fn splits_streamdown_benchmark_nested_html_block() {
    let mut s = MdStream::new(Options::default());
    let input = "<div>\n  <div>\n    <div>\n      <p>Nested content</p>\n    </div>\n  </div>\n</div>\n\nAfter\n";
    let u = s.append(input);
    assert!(u
        .committed
        .iter()
        .any(|b| b.raw.contains("<div>\n  <div>\n    <div>\n      <p>Nested content</p>\n    </div>\n  </div>\n</div>\n")));
    assert_eq!(u.pending.as_ref().unwrap().raw, "After\n");
}

#[test]
fn splits_streamdown_benchmark_multiple_html_blocks() {
    let mut s = MdStream::new(Options::default());
    let input = "<div>First block</div>\n\nSome markdown\n\n<section>\n  <p>Second block</p>\n</section>\n\nMore markdown\n";
    let u = s.append(input);

    assert!(u.committed.iter().any(|b| b.raw == "<div>First block</div>\n"));
    assert!(u.committed.iter().any(|b| b.raw == "Some markdown\n\n"));
    assert!(u.committed.iter().any(|b| {
        b.raw.contains("<section>\n  <p>Second block</p>\n</section>\n") && b.kind == mdstream::BlockKind::HtmlBlock
    }));
    assert_eq!(u.pending.as_ref().unwrap().raw, "More markdown\n");
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

#[test]
fn commits_math_block_with_split_delimiters_as_single_block() {
    let mut s = MdStream::new(Options::default());

    let u1 = s.append("Some text\n\n$$\n\nx^2 + y^2 = z^2\n\n");
    assert!(u1.committed.iter().any(|b| b.raw == "Some text\n\n"));
    assert!(u1.pending.is_some());
    assert_eq!(
        u1.pending.as_ref().unwrap().raw,
        "$$\n\nx^2 + y^2 = z^2\n\n"
    );

    let u2 = s.append("$$\n\nMore text\n");
    assert!(u2.committed.iter().any(|b| {
        b.raw == "$$\n\nx^2 + y^2 = z^2\n\n$$\n\n"
            || b.raw == "$$\n\nx^2 + y^2 = z^2\n\n$$\n"
    }));
    assert_eq!(u2.pending.as_ref().unwrap().raw, "More text\n");
}

#[test]
fn commits_simple_math_block_like_streamdown_bench() {
    let mut s = MdStream::new(Options::default());
    let input = "Some text\n\n$$\nE = mc^2\n$$\n\nMore text\n";
    let u = s.append(input);

    assert!(u.committed.iter().any(|b| b.raw == "Some text\n\n"));
    assert!(u
        .committed
        .iter()
        .any(|b| b.raw == "$$\nE = mc^2\n$$\n\n" || b.raw == "$$\nE = mc^2\n$$\n"));
    assert_eq!(u.pending.as_ref().unwrap().raw, "More text\n");
}

#[test]
fn commits_complex_math_blocks_like_streamdown_bench() {
    let mut s = MdStream::new(Options::default());
    let input = "$$\n\\begin{bmatrix}\na & b \\\\\nc & d\n\\end{bmatrix}\n$$\n\nText\n\n$$\n\\int_0^\\infty x^2 dx\n$$\n";
    let u = s.append(input);

    assert!(u.committed.iter().any(|b| {
        b.raw.contains("$$\n\\begin{bmatrix}\n")
            && b.raw.contains("\\end{bmatrix}\n$$\n")
            && b.kind == mdstream::BlockKind::MathBlock
    }));
    assert!(u.committed.iter().any(|b| b.raw == "Text\n\n"));
    assert!(u.committed.iter().any(|b| {
        b.raw.contains("$$\n\\int_0^\\infty x^2 dx\n$$\n") && b.kind == mdstream::BlockKind::MathBlock
    }));
    assert!(u.pending.is_none() || u.pending.as_ref().unwrap().raw.trim().is_empty());
}
