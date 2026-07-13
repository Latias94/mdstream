// Upstream-derived characterization for `remend.pending-repair/1.3.0`, pinned
// to Streamdown commit e5deed330aa4231751a106445d93d62e4716a22f. These tests
// claim only exact pending-text repair, never finalized Markdown semantics.
use mdstream::{TerminatorOptions, terminate_markdown};

fn remend(text: &str) -> String {
    terminate_markdown(text, &TerminatorOptions::default())
}

#[test]
fn katex_block_formatting() {
    assert_eq!(remend("Text with $$formula"), "Text with $$formula$$");
    assert_eq!(remend("$$incomplete"), "$$incomplete$$");

    let text = "Text with $$E = mc^2$$";
    assert_eq!(remend(text), text);

    let text = "$$formula1$$ and $$formula2$$";
    assert_eq!(remend(text), text);

    assert_eq!(remend("$$first$$ and $$second"), "$$first$$ and $$second$$");
    assert_eq!(remend("$$x + y = z"), "$$x + y = z$$");
    assert_eq!(remend("$$\nx = 1\ny = 2"), "$$\nx = 1\ny = 2\n$$");
}

#[test]
fn katex_inline_dollars_are_not_completed() {
    assert_eq!(remend("Text with $formula"), "Text with $formula");
    assert_eq!(remend("$incomplete"), "$incomplete");

    let text = "Text with $x^2 + y^2 = z^2$";
    assert_eq!(remend(text), text);

    let text = "$a = 1$ and $b = 2$";
    assert_eq!(remend(text), text);

    assert_eq!(remend("$first$ and $second"), "$first$ and $second");
    assert_eq!(remend("$$block$$ and $inline"), "$$block$$ and $inline");
    assert_eq!(remend("$x + y = z"), "$x + y = z");

    let text = r"Price is \$100";
    assert_eq!(remend(text), text);

    assert_eq!(remend("$$$"), "$$$$$");
    assert_eq!(remend("$$$$"), "$$$$");
}

#[test]
fn math_blocks_with_underscores_and_asterisks() {
    let text = "The variable $x_1$ represents the first element";
    assert_eq!(remend(text), text);
    let text = "Formula: $a_b + c_d = e_f$";
    assert_eq!(remend(text), text);

    let text = "$$x_1 + y_2 = z_3$$";
    assert_eq!(remend(text), text);
    let text = "$$\na_1 + b_2\nc_3 + d_4\n$$";
    assert_eq!(remend(text), text);

    assert_eq!(remend("Math expression $x_"), "Math expression $x_");
    assert_eq!(remend("$$formula_"), "$$formula_$$");

    let text = "Text with _italic_ and math $x_1$";
    assert_eq!(remend(text), text);
    let text = "_italic text_ followed by $a_b$";
    assert_eq!(remend(text), text);

    assert_eq!(
        remend("Start _italic with $x_1$"),
        "Start _italic with $x_1$_"
    );

    let text = "$x_1 + x_2 + x_3 = y_1$";
    assert_eq!(remend(text), text);
    let text = "$$\\sum_{i=1}^{n} x_i = \\prod_{j=1}^{m} y_j$$";
    assert_eq!(remend(text), text);

    let text = r"Price is \$50 and _this is italic_";
    assert_eq!(remend(text), text);
    assert_eq!(
        remend(r"Cost \$100 with _incomplete"),
        r"Cost \$100 with _incomplete_"
    );

    let text = "Inline $x_1$ and block $$y_2$$ math";
    assert_eq!(remend(text), text);

    let text = "_italic start $x_1$ italic end_";
    assert_eq!(remend(text), text);

    let str_ = "Streamdown uses double dollar signs (`$$`) to delimit mathematical expressions.";
    assert_eq!(remend(str_), str_);
    let str_ = "Use `$$` for math blocks and `$$formula$$` for inline.";
    assert_eq!(remend(str_), str_);
    assert_eq!(
        remend("Math: $$x+y and code: `$$`"),
        "Math: $$x+y and code: `$$`$$"
    );
    assert_eq!(
        remend("$$formula$$ and code `$$` and $$incomplete"),
        "$$formula$$ and code `$$` and $$incomplete$$"
    );

    let text = "$$\\mathbf{w}^{*}$$";
    assert_eq!(remend(text), text);
    let text = "$$\n\\mathbf{w}^{*} = \\underset{\\|\\mathbf{w}\\|=1}{\\arg\\max} \\;\\; \\mathbf{w}^T S \\mathbf{w}\n$$";
    assert_eq!(remend(text), text);
    let text = "Text with *italic* and math $$x^{*}$$";
    assert_eq!(remend(text), text);
    assert_eq!(
        remend("Start *italic with $$x^{*}$$"),
        "Start *italic with $$x^{*}$$*"
    );
}

#[test]
fn mixed_formatting() {
    let text = "**bold** and *italic* and `code` and ~~strike~~";
    assert_eq!(remend(text), text);

    assert_eq!(remend("**bold and *italic"), "**bold and *italic*");

    let text = "**bold with *italic* inside**";
    assert_eq!(remend(text), text);

    assert_eq!(
        remend("Text with [link and **bold"),
        "Text with [link and **bold](streamdown:incomplete-link)"
    );

    let text = "# Heading\n\n**Bold text** with *italic* and `code`.\n\n- List item\n- Another item with ~~strike~~";
    assert_eq!(remend(text), text);

    assert_eq!(remend("*italic with **bold"), "*italic with **bold***");
    assert_eq!(remend("**bold with `code"), "**bold with `code**`");
    assert_eq!(remend("~~strike with **bold"), "~~strike with **bold**~~");
    assert_eq!(remend("**bold with $x^2"), "**bold with $x^2**");
    assert_eq!(
        remend("**bold *italic `code ~~strike"),
        "**bold *italic `code ~~strike*`~~"
    );

    let text = "**bold *italic* text** and `code`";
    assert_eq!(remend(text), text);

    assert_eq!(
        remend("**bold and *bold-italic***"),
        "**bold and *bold-italic***"
    );

    assert_eq!(
        remend("combined **_bold and italic"),
        "combined **_bold and italic_**"
    );
    assert_eq!(remend("**_text"), "**_text_**");
    assert_eq!(remend("_italic and **bold"), "_italic and **bold**_");
}

#[test]
fn streaming_scenarios() {
    assert_eq!(
        remend("# Main Title\n## Subtitle with **emph"),
        "# Main Title\n## Subtitle with **emph**"
    );
    assert_eq!(remend("> Quote with **bold"), "> Quote with **bold**");
    assert_eq!(
        remend("| Col1 | Col2 |\n|------|------|\n| **dat"),
        "| Col1 | Col2 |\n|------|------|\n| **dat**"
    );
    assert_eq!(
        remend("1. First item\n   - Nested with `code\n2. Second"),
        "1. First item\n   - Nested with `code\n2. Second`"
    );
    assert_eq!(remend("Text **bold `code"), "Text **bold `code**`");

    let chunks = [
        "Here is",
        "Here is a **bold",
        "Here is a **bold statement",
        "Here is a **bold statement** about",
        "Here is a **bold statement** about `code",
        "Here is a **bold statement** about `code`.",
    ];
    assert_eq!(remend(chunks[0]), chunks[0]);
    assert_eq!(remend(chunks[1]), "Here is a **bold**");
    assert_eq!(remend(chunks[2]), "Here is a **bold statement**");
    assert_eq!(remend(chunks[3]), chunks[3]);
    assert_eq!(
        remend(chunks[4]),
        "Here is a **bold statement** about `code`"
    );
    assert_eq!(remend(chunks[5]), chunks[5]);

    let chunks = [
        "To use this function",
        "To use this function, call `getData(",
        "To use this function, call `getData()` with",
    ];
    assert_eq!(remend(chunks[0]), chunks[0]);
    assert_eq!(remend(chunks[1]), "To use this function, call `getData(`");
    assert_eq!(remend(chunks[2]), chunks[2]);
}

#[test]
fn edge_cases_and_horizontal_rules() {
    assert_eq!(remend("Text ending with *"), "Text ending with *");
    assert_eq!(remend("Text ending with **"), "Text ending with **");

    assert_eq!(remend("****"), "****");
    assert_eq!(remend("``"), "``");

    assert_eq!(remend("**"), "**");
    assert_eq!(remend("__"), "__");
    assert_eq!(remend("***"), "***");
    assert_eq!(remend("*"), "*");
    assert_eq!(remend("_"), "_");
    assert_eq!(remend("~~"), "~~");
    assert_eq!(remend("`"), "`");

    assert_eq!(remend("** __"), "** __");
    assert_eq!(remend("\n** __\n"), "\n** __\n");
    assert_eq!(remend("* _ ~~ `"), "* _ ~~ `");

    assert_eq!(remend("** "), "**");
    assert_eq!(remend(" **"), " **");
    assert_eq!(remend("  **  "), "  **  ");

    assert_eq!(remend("**text"), "**text**");
    assert_eq!(remend("__text"), "__text__");
    assert_eq!(remend("*text"), "*text*");
    assert_eq!(remend("_text"), "_text_");
    assert_eq!(remend("~~text"), "~~text~~");
    assert_eq!(remend("`text"), "`text`");

    assert_eq!(remend("---"), "---");
    assert_eq!(remend("----"), "----");
    assert_eq!(remend("-----"), "-----");
    assert_eq!(remend("***"), "***");
    assert_eq!(remend("****"), "****");
    assert_eq!(remend("*****"), "*****");
    assert_eq!(remend("___"), "___");
    assert_eq!(remend("____"), "____");
    assert_eq!(remend("_____"), "_____");
    assert_eq!(remend("- - -"), "- - -");
    assert_eq!(remend("* * *"), "* * *");
    assert_eq!(remend("_ _ _"), "_ _ _");
    assert_eq!(remend("-  -  -"), "-  -  -");
    assert_eq!(remend("*   *   *"), "*   *   *");
    assert_eq!(remend("_    _    _"), "_    _    _");

    let text = "Text before\n***\nText after";
    assert_eq!(remend(text), text);
    let text = "Text before\n___\nText after";
    assert_eq!(remend(text), text);

    assert_eq!(remend("Some text\n\n---"), "Some text\n\n---");
    assert_eq!(remend("Some text\n\n***"), "Some text\n\n***");
    assert_eq!(remend("Some text\n\n___"), "Some text\n\n___");

    assert_eq!(remend("--"), "--");
    assert_eq!(remend("**"), "**");
    assert_eq!(remend("__"), "__");
    assert_eq!(remend("Text\n\n--"), "Text\n\n--");

    assert_eq!(remend("   ---"), "   ---");
    assert_eq!(remend("  ***"), "  ***");
    assert_eq!(remend(" ___"), " ___");
}

#[test]
fn word_internal_underscores_and_identifiers() {
    assert_eq!(remend("hello_world"), "hello_world");
    assert_eq!(remend("hello_world_test"), "hello_world_test");
    assert_eq!(remend("MAX_VALUE"), "MAX_VALUE");
    assert_eq!(
        remend("The user_name and user_email are required"),
        "The user_name and user_email are required"
    );
    assert_eq!(
        remend("Visit https://example.com/path_with_underscore"),
        "Visit https://example.com/path_with_underscore"
    );
    assert_eq!(remend("The value is 1_000_000"), "The value is 1_000_000");

    assert_eq!(remend("_italic text"), "_italic text_");
    assert_eq!(remend("This is _italic"), "This is _italic_");
    assert_eq!(remend("_italic\n"), "_italic_\n");

    assert_eq!(remend("word_"), "word_");
    assert_eq!(remend("_privateVariable"), "_privateVariable_");
    assert_eq!(
        remend("Use `variable_name` in your code"),
        "Use `variable_name` in your code"
    );
    assert_eq!(
        remend("The variable_name is _important"),
        "The variable_name is _important_"
    );
    assert_eq!(
        remend("_complete italic_ and some_other_text"),
        "_complete italic_ and some_other_text"
    );
    assert_eq!(
        remend("```\nfunction_name()\n```"),
        "```\nfunction_name()\n```"
    );
    assert_eq!(
        remend(r#"<div data_attribute="value">"#),
        r#"<div data_attribute="value">"#
    );

    assert_eq!(
        remend("__init__ and __main__ are special"),
        "__init__ and __main__ are special"
    );
    assert_eq!(
        remend("The user_id field stores the _unique identifier"),
        "The user_id field stores the _unique identifier_"
    );
    let input = "hello_world\n\n<a href=\"example_link\"/>";
    assert_eq!(remend(input), input);
}

#[test]
fn inline_code_and_code_blocks() {
    assert_eq!(remend("Text with `code"), "Text with `code`");
    assert_eq!(remend("`incomplete"), "`incomplete`");

    let text = "Text with `inline code`";
    assert_eq!(remend(text), text);
    let text = "`code1` and `code2`";
    assert_eq!(remend(text), text);

    let text = "```\ncode block with `backtick\n```";
    assert_eq!(remend(text), text);

    let text = "```javascript\nconst x = `template";
    assert_eq!(remend(text), text);

    let text = "```python print(\"Hello, Sunnyvale!\")```";
    assert_eq!(remend(text), text);
    let text = "```python print(\"Hello, Sunnyvale!\")``";
    assert_eq!(remend(text), "```python print(\"Hello, Sunnyvale!\")```");

    let text = "```code```";
    assert_eq!(remend(text), text);
    let text = "```code```\n";
    assert_eq!(remend(text), text);
    let text = "```\ncode\n```";
    assert_eq!(remend(text), text);
    let text = "``````";
    assert_eq!(remend(text), text);
    let text = "text``````";
    assert_eq!(remend(text), text);

    assert_eq!(
        remend("```\nblock\n```\n`inline"),
        "```\nblock\n```\n`inline`"
    );
}

#[test]
fn setext_heading_protection_with_indentation() {
    assert_eq!(remend("here is a list\n-"), "here is a list\n-\u{200B}");
    assert_eq!(remend("Some text\n--"), "Some text\n--\u{200B}");
    assert_eq!(remend("Some text\n="), "Some text\n=\u{200B}");
    assert_eq!(remend("Some text\n=="), "Some text\n==\u{200B}");

    assert_eq!(remend("Some text\n---"), "Some text\n---");
    assert_eq!(remend("Heading\n==="), "Heading\n===");

    assert_eq!(remend("-"), "-");
    assert_eq!(remend("\n-"), "\n-");

    assert_eq!(remend("Line 1\nLine 2\n-"), "Line 1\nLine 2\n-\u{200B}");

    // Leading whitespace is allowed.
    assert_eq!(remend("Some text\n  -"), "Some text\n  -\u{200B}");

    assert_eq!(remend("Some text\n-x"), "Some text\n-x");
    assert_eq!(remend("Some text\n----"), "Some text\n----");

    // Trailing single space removed before applying ZWSP.
    assert_eq!(remend("Some text\n- "), "Some text\n-\u{200B}");
}

#[test]
fn list_handling() {
    let text = "* Item 1\n* Item 2\n* Item 3";
    assert_eq!(remend(text), text);

    let text = "* Single item";
    assert_eq!(remend(text), text);

    let text = "* Parent item\n  * Nested item 1\n  * Nested item 2";
    assert_eq!(remend(text), text);

    let text = "* Item with *italic* text\n* Another item";
    assert_eq!(remend(text), text);

    assert_eq!(
        remend("* Item with *incomplete italic\n* Another item"),
        "* Item with *incomplete italic\n* Another item*"
    );

    let text = "* First item\n* Second *italic* item\n* Third item";
    assert_eq!(remend(text), text);

    let text = "*\tItem with tab\n*\tAnother item";
    assert_eq!(remend(text), text);

    let text = "- Item 1\n- Item 2 with *italic*\n- Item 3";
    assert_eq!(remend(text), text);

    let text = "* user123\n* user456\n* user789";
    assert_eq!(remend(text), text);

    assert_eq!(
        remend("- Item 1\n- Item 2 with **bol"),
        "- Item 1\n- Item 2 with **bol**"
    );

    // Lists with just emphasis markers should not be auto-completed.
    assert_eq!(remend("- __"), "- __");
    assert_eq!(remend("- **"), "- **");
    assert_eq!(remend("- __\n- **"), "- __\n- **");
    assert_eq!(remend("\n- __\n- **"), "\n- __\n- **");

    assert_eq!(remend("* __\n* **"), "* __\n* **");
    assert_eq!(remend("+ __\n+ **"), "+ __\n+ **");

    assert_eq!(remend("- __ text after"), "- __ text after__");
    assert_eq!(remend("- ** text after"), "- ** text after**");

    assert_eq!(
        remend("- __\n- Normal item\n- **"),
        "- __\n- Normal item\n- **"
    );

    assert_eq!(remend("- ***"), "- ***");
    assert_eq!(remend("- *"), "- *");
    assert_eq!(remend("- _"), "- _");
    assert_eq!(remend("- ~~"), "- ~~");
    assert_eq!(remend("- `"), "- `");

    // Do not close across multiline list items when marker is immediately after list prefix.
    assert_eq!(remend("- **text\nmore text"), "- **text\nmore text");
    assert_eq!(
        remend("* **content\n* Another item"),
        "* **content\n* Another item"
    );
}

#[test]
fn link_handling() {
    assert_eq!(
        remend("Text with [incomplete link"),
        "Text with [incomplete link](streamdown:incomplete-link)"
    );
    assert_eq!(
        remend("Text [partial"),
        "Text [partial](streamdown:incomplete-link)"
    );

    let text = "Text with [complete link](url)";
    assert_eq!(remend(text), text);

    let text = "[link1](url1) and [link2](url2)";
    assert_eq!(remend(text), text);

    assert_eq!(
        remend("[outer [nested] text](incomplete"),
        "[outer [nested] text](streamdown:incomplete-link)"
    );
    assert_eq!(
        remend("[link with [inner] content](http://incomplete"),
        "[link with [inner] content](streamdown:incomplete-link)"
    );
    assert_eq!(
        remend("Text [foo [bar] baz]("),
        "Text [foo [bar] baz](streamdown:incomplete-link)"
    );

    let text = "[link with [brackets] inside](https://example.com)";
    assert_eq!(remend(text), text);

    assert_eq!(
        remend("Check out [this lin"),
        "Check out [this lin](streamdown:incomplete-link)"
    );
    assert_eq!(
        remend("Visit [our site](https://exa"),
        "Visit [our site](streamdown:incomplete-link)"
    );

    assert_eq!(
        remend("Text [outer [inner"),
        "Text [outer [inner](streamdown:incomplete-link)"
    );
    assert_eq!(
        remend("[foo [bar [baz"),
        "[foo [bar [baz](streamdown:incomplete-link)"
    );
    assert_eq!(
        remend("Text [outer [inner]"),
        "Text [outer [inner]](streamdown:incomplete-link)"
    );
    assert_eq!(
        remend("[link [nested] text"),
        "[link [nested] text](streamdown:incomplete-link)"
    );
}

#[test]
fn image_handling() {
    assert_eq!(remend("Text with ![incomplete image"), "Text with ");
    assert_eq!(remend("![partial"), "");

    let text = "Text with ![alt text](image.png)";
    assert_eq!(remend(text), text);

    assert_eq!(remend("See ![the diag"), "See ");
    assert_eq!(remend("![logo](./assets/log"), "");

    assert_eq!(remend("Text ![outer [inner]"), "Text ");
    assert_eq!(remend("![nested [brackets] text"), "");
    assert_eq!(remend("Start ![foo [bar] baz"), "Start ");

    let markdown = "textContent ![image](https://img.alicdn.com/imgextra/i4/6000000003603/O1CN01ApW8bQ1cUE8LduPra_!!6000000003603-2-skyky.png)";
    assert_eq!(remend(markdown), markdown);
    let link_markdown = "textContent [link](https://example.com/path_name!!test)";
    assert_eq!(remend(link_markdown), link_markdown);
    let multiple_images = "textContent ![image1](https://example.com/path_1!!test.png) ![image2](https://example.com/path_2!!test.png)";
    assert_eq!(remend(multiple_images), multiple_images);
}

#[test]
fn bold_italic_and_strikethrough() {
    assert_eq!(remend("Text with **bold"), "Text with **bold**");
    assert_eq!(remend("**incomplete"), "**incomplete**");
    let text = "Text with **bold text**";
    assert_eq!(remend(text), text);
    let text = "**bold1** and **bold2**";
    assert_eq!(remend(text), text);
    assert_eq!(remend("**first** and **second"), "**first** and **second**");
    assert_eq!(
        remend("Here is some **bold tex"),
        "Here is some **bold tex**"
    );

    assert_eq!(
        remend("Text with ***bold-italic"),
        "Text with ***bold-italic***"
    );
    assert_eq!(remend("***incomplete"), "***incomplete***");
    let text = "Text with ***bold and italic text***";
    assert_eq!(remend(text), text);
    let text = "***first*** and ***second***";
    assert_eq!(remend(text), text);
    assert_eq!(
        remend("***first*** and ***second"),
        "***first*** and ***second***"
    );
    assert_eq!(
        remend("*italic* **bold** ***both"),
        "*italic* **bold** ***both***"
    );
    assert_eq!(
        remend("***Starting bold-italic"),
        "***Starting bold-italic***"
    );
    assert_eq!(
        remend("***bold-italic with `code"),
        "***bold-italic with `code***`"
    );

    assert_eq!(remend("Text with ~~strike"), "Text with ~~strike~~");
    assert_eq!(remend("~~incomplete"), "~~incomplete~~");
    let text = "Text with ~~strikethrough text~~";
    assert_eq!(remend(text), text);
    let text = "~~strike1~~ and ~~strike2~~";
    assert_eq!(remend(text), text);
    assert_eq!(remend("~~first~~ and ~~second"), "~~first~~ and ~~second~~");
}

#[test]
fn asterisk_and_underscore_italics() {
    assert_eq!(remend("Text with __italic"), "Text with __italic__");
    assert_eq!(remend("__incomplete"), "__incomplete__");
    let text = "Text with __italic text__";
    assert_eq!(remend(text), text);
    assert_eq!(remend("__first__ and __second"), "__first__ and __second__");

    assert_eq!(remend("Text with *italic"), "Text with *italic*");
    assert_eq!(remend("*incomplete"), "*incomplete*");
    let text = "Text with *italic text*";
    assert_eq!(remend(text), text);
    assert_eq!(remend("**bold** and *italic"), "**bold** and *italic*");

    assert_eq!(remend("234234*123"), "234234*123");
    assert_eq!(remend("hello*world"), "hello*world");
    assert_eq!(remend("test*123*test"), "test*123*test");
    assert_eq!(
        remend("*italic with some*var*name inside"),
        "*italic with some*var*name inside*"
    );
    assert_eq!(
        remend("test*var and *incomplete italic"),
        "test*var and *incomplete italic*"
    );

    assert_eq!(
        remend(r"\*escaped asterisk and *italic"),
        r"\*escaped asterisk and *italic*"
    );
    assert_eq!(
        remend(r"*start \* middle \* end"),
        r"*start \* middle \* end*"
    );

    assert_eq!(remend("abc*123"), "abc*123");
    assert_eq!(remend("123*abc"), "123*abc");

    assert_eq!(remend("This is *italic"), "This is *italic*");
    assert_eq!(remend("*word* and more text"), "*word* and more text");
}

#[test]
fn basic_input() {
    assert_eq!(remend(""), "");
    let text = "This is plain text without any markdown";
    assert_eq!(remend(text), text);
}

#[test]
fn code_block_edge_cases() {
    assert_eq!(
        remend("```javascript\nconst x = 5;"),
        "```javascript\nconst x = 5;"
    );
    assert_eq!(remend("```\ncode here"), "```\ncode here");

    let text = "```javascript\nconst x = 5;\n```";
    assert_eq!(remend(text), text);

    let text = "```\ncode\n```\nMore text";
    assert_eq!(remend(text), text);

    let grok_output = "```python def greet(name): return f\"Hello, {name}!\"\n```";
    assert_eq!(remend(grok_output), grok_output);

    let text = r#"Here's some code:
```javascript
const arr = [1, 2, 3];
console.log(arr[0]);
```
Done with code block."#;
    assert_eq!(remend(text), text);

    let text = r#"Here's a code block:
```bash
echo "test"
```
And here's an [incomplete link"#;
    assert_eq!(
        remend(text),
        r#"Here's a code block:
```bash
echo "test"
```
And here's an [incomplete link](streamdown:incomplete-link)"#
    );

    let text_content = r#"Precisely.

When full-screen TUI applications like **Vim**, **less**, or **htop** start, they switch the terminal into what's called the **alternate screen buffer**—a second, temporary display area separate from the main scrollback buffer.

### How it works
They send ANSI escape sequences such as:
```bash
# Enter alternate screen buffer
echo -e "\\e[?1049h"

# Exit (back to normal buffer)
echo -e "\\e[?1049l"
```

- `\\e[?1049h` — activates the alternate screen.
- `\\e[?1049l` — deactivates it and restores the previous view.

While in this mode:
- The "scrollback" (your regular terminal history) is hidden.
- The program gets a fresh, empty screen to draw on.
- When the program exits, the screen restores exactly as it was before.

### tmux behavior
`tmux` respects these escape sequences by default. When apps use the alternate buffer, tmux holds that screen separately from the main one. That's why, when you scroll in tmux during Vim, you don't see your shell history—you have to leave Vim first.

If someone wants to **disable** this behavior (so the app draws on the main screen and you can scroll back freely), they can set:
```bash
set -g terminal-overrides 'xterm*:smcup@:rmcup@'
```
in their `~/.tmux.conf`, which disables use of the alternate buffer entirely.

Would you like me to show how to conditionally toggle that behavior per app or session?"#;
    assert_eq!(remend(text_content), text_content);

    let input = r#"```css
/* Commentary */

[class*="WidgetTitle__Header"] {
  font-size: 18px !important;
}
```

Notes and tips:
* Use !important only where necessary in CSS."#;
    assert_eq!(remend(input), input);
    assert!(!remend(input).ends_with("__"));

    let input = r#"```python
def __init__(self):
    pass
```

* List item"#;
    assert_eq!(remend(input), input);
    assert!(!remend(input).ends_with("__"));

    let text = r#"```css
code here
```

**incomplete bold"#;
    assert_eq!(
        remend(text),
        r#"```css
code here
```

**incomplete bold**"#
    );
}
