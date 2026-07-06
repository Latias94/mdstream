use mdstream::{TerminatorOptions, terminate_markdown};

#[test]
fn setext_heading_protection() {
    let opts = TerminatorOptions::default();
    assert_eq!(
        terminate_markdown("here is a list\n-", &opts),
        "here is a list\n-\u{200B}"
    );
    assert_eq!(
        terminate_markdown("Some text\n--", &opts),
        "Some text\n--\u{200B}"
    );
    assert_eq!(
        terminate_markdown("Some text\n=", &opts),
        "Some text\n=\u{200B}"
    );
    assert_eq!(
        terminate_markdown("Some text\n==", &opts),
        "Some text\n==\u{200B}"
    );
    assert_eq!(
        terminate_markdown("Some text\n---", &opts),
        "Some text\n---"
    );
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
    let expected =
        "```bash\necho \"test\"\n```\nAnd here's an [incomplete link](streamdown:incomplete-link)";
    assert_eq!(terminate_markdown(text, &opts), expected);
}

#[test]
fn streaming_nested_formatting_examples() {
    let opts = TerminatorOptions::default();
    assert_eq!(
        terminate_markdown("This is **bold with *ital", &opts),
        "This is **bold with *ital*"
    );
    assert_eq!(terminate_markdown("**bold _und", &opts), "**bold _und_**");
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

#[test]
fn latex_begin_block_does_not_duplicate_katex_delimiters_streamdown_issue_54() {
    let opts = TerminatorOptions::default();
    let content = "$$\n\\begin{pmatrix}\nx \\\\\ny\n\\end{pmatrix}\n=\n$$";
    let out = terminate_markdown(content, &opts);
    assert!(!out.contains("$$$$"));
    assert_eq!(out, content);
}

#[test]
fn latex_begin_block_missing_closing_delimiter_is_balanced_once() {
    let opts = TerminatorOptions::default();
    let content = "$$\n\\begin{bmatrix}\n1 & 2 \\\\\n3 & 4\n\\end{bmatrix}\n=";
    let out = terminate_markdown(content, &opts);
    assert!(!out.contains("$$$$"));
    assert!(out.ends_with("$$"));
}

#[test]
fn streamdown_component_incomplete_markdown_enabled_by_default() {
    // Streamdown: `parseIncompleteMarkdown` defaults to `true` in streaming mode.
    let opts = TerminatorOptions::default();
    assert_eq!(
        terminate_markdown("Text with **incomplete bold", &opts),
        "Text with **incomplete bold**"
    );
}

#[test]
fn streamdown_component_incomplete_markdown_disabled() {
    // Streamdown: `parseIncompleteMarkdown={false}` means "do not remend".
    let opts = TerminatorOptions {
        setext_headings: false,
        links: false,
        images: false,
        emphasis: false,
        inline_code: false,
        strikethrough: false,
        katex_block: false,
        incomplete_link_url: TerminatorOptions::default().incomplete_link_url,
        window_bytes: TerminatorOptions::default().window_bytes,
    };
    assert_eq!(
        terminate_markdown("Text with **incomplete bold", &opts),
        "Text with **incomplete bold"
    );
}

#[test]
fn streamdown_cjk_friendly_emphasis_examples_are_stable() {
    let opts = TerminatorOptions::default();

    assert_eq!(
        terminate_markdown("**（）【】「」**", &opts),
        "**（）【】「」**"
    );
    assert_eq!(
        terminate_markdown("**外側（*内側【ネスト】*）。**", &opts),
        "**外側（*内側【ネスト】*）。**"
    );
    assert_eq!(
        terminate_markdown(
            "**太字（説明）**と*斜体【補足】*と~~削除「古い」~~。",
            &opts
        ),
        "**太字（説明）**と*斜体【補足】*と~~削除「古い」~~。"
    );
    assert_eq!(
        terminate_markdown("**テキスト**（説明）", &opts),
        "**テキスト**（説明）"
    );
    assert_eq!(
        terminate_markdown("**This is English（これは日本語）mixed content。**", &opts),
        "**This is English（これは日本語）mixed content。**"
    );
}

#[test]
fn streamdown_cjk_friendly_incomplete_emphasis_is_completed() {
    let opts = TerminatorOptions::default();
    assert_eq!(
        terminate_markdown("**未完了のテキスト（括弧付き", &opts),
        "**未完了のテキスト（括弧付き**"
    );
    assert_eq!(
        terminate_markdown("**ストリーミング", &opts),
        "**ストリーミング**"
    );
}

#[test]
fn streamdown_email_addresses_are_not_modified() {
    let opts = TerminatorOptions::default();
    assert_eq!(
        terminate_markdown("example@gmail.com", &opts),
        "example@gmail.com"
    );
    assert_eq!(
        terminate_markdown(
            "Please contact me at john.doe@example.com for more info",
            &opts
        ),
        "Please contact me at john.doe@example.com for more info"
    );
    assert_eq!(
        terminate_markdown("Contact admin@site.com or support@site.com", &opts),
        "Contact admin@site.com or support@site.com"
    );
    assert_eq!(
        terminate_markdown("user+test@example-domain.co.uk", &opts),
        "user+test@example-domain.co.uk"
    );
}
