use mdstream::pending::{terminate_markdown, TerminatorOptions};

#[test]
fn setext_heading_protection() {
    let opts = TerminatorOptions::default();
    assert_eq!(terminate_markdown("here is a list\n-", &opts), "here is a list\n-\u{200B}");
    assert_eq!(terminate_markdown("Some text\n--", &opts), "Some text\n--\u{200B}");
    assert_eq!(terminate_markdown("Some text\n=", &opts), "Some text\n=\u{200B}");
    assert_eq!(terminate_markdown("Some text\n==", &opts), "Some text\n==\u{200B}");
    assert_eq!(terminate_markdown("Some text\n---", &opts), "Some text\n---");
    assert_eq!(terminate_markdown("Heading\n===", &opts), "Heading\n===");
}

#[test]
fn incomplete_links_and_images() {
    let opts = TerminatorOptions::default();
    assert_eq!(
        terminate_markdown("Text with [incomplete link", &opts),
        "Text with [incomplete link](streamdown:incomplete-link)"
    );
    assert_eq!(
        terminate_markdown("Visit [our site](https://exa", &opts),
        "Visit [our site](streamdown:incomplete-link)"
    );
    assert_eq!(
        terminate_markdown("Text [foo [bar] baz](", &opts),
        "Text [foo [bar] baz](streamdown:incomplete-link)"
    );
    assert_eq!(
        terminate_markdown("[outer [nested] text](incomplete", &opts),
        "[outer [nested] text](streamdown:incomplete-link)"
    );
}

#[test]
fn no_incomplete_link_markers_inside_code_fences() {
    let opts = TerminatorOptions::default();
    let text = "```js\nconst arr = [1, 2, 3];\nconsole.log(arr[0]);\n```\n";
    assert_eq!(terminate_markdown(text, &opts), text);
}

#[test]
fn incomplete_link_outside_code_fences_is_fixed() {
    let opts = TerminatorOptions::default();
    let text = "```bash\necho \"test\"\n```\nAnd here's an [incomplete link";
    let expected = "```bash\necho \"test\"\n```\nAnd here's an [incomplete link](streamdown:incomplete-link)";
    assert_eq!(terminate_markdown(text, &opts), expected);
}

#[test]
fn streaming_nested_formatting_examples() {
    let opts = TerminatorOptions::default();
    assert_eq!(
        terminate_markdown("This is **bold with *ital", &opts),
        "This is **bold with *ital*"
    );
    assert_eq!(
        terminate_markdown("**bold _und", &opts),
        "**bold _und_**"
    );
    assert_eq!(
        terminate_markdown("To use this function, call `getData(", &opts),
        "To use this function, call `getData(`"
    );
}
