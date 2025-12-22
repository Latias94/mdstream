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

#[test]
fn inline_code_and_triple_backticks() {
    let opts = TerminatorOptions::default();
    assert_eq!(
        terminate_markdown("Text with `code", &opts),
        "Text with `code`"
    );
    assert_eq!(
        terminate_markdown("```python print(\"Hello!\")``", &opts),
        "```python print(\"Hello!\")```"
    );
    // Incomplete multiline code block should not be modified.
    assert_eq!(
        terminate_markdown("```javascript\nconst x = `template", &opts),
        "```javascript\nconst x = `template"
    );
}

#[test]
fn strikethrough_and_katex() {
    let opts = TerminatorOptions::default();
    assert_eq!(
        terminate_markdown("Text with ~~strike", &opts),
        "Text with ~~strike~~"
    );
    assert_eq!(
        terminate_markdown("Text with $$formula", &opts),
        "Text with $$formula$$"
    );
    assert_eq!(
        terminate_markdown("$$\nx = 1\ny = 2", &opts),
        "$$\nx = 1\ny = 2\n$$"
    );
}

#[test]
fn mixed_formatting_order_matches_streamdown() {
    let opts = TerminatorOptions::default();
    // Bold closed first, then code.
    assert_eq!(
        terminate_markdown("**bold with `code", &opts),
        "**bold with `code**`"
    );
    // Italic closed after bold if italic opened before bold.
    assert_eq!(
        terminate_markdown("*italic with **bold", &opts),
        "*italic with **bold***"
    );
    // $$ outside inline code is completed; $$ inside inline code is ignored.
    assert_eq!(
        terminate_markdown("Math: $$x+y and code: `$$`", &opts),
        "Math: $$x+y and code: `$$`$$"
    );
    // Underscore inside $$ should not be treated as italic.
    assert_eq!(terminate_markdown("$$formula_", &opts), "$$formula_$$");
}
