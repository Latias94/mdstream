mod support;

use mdstream::Options;

fn assert_invariant(case_name: &str, markdown: &str, opts: Options, trials: u64, max_bytes: usize) {
    let expected = support::collect_final_blocks(support::chunk_whole(markdown), opts.clone());

    let blocks_lines = support::collect_final_blocks(support::chunk_lines(markdown), opts.clone());
    assert_eq!(blocks_lines, expected, "case={case_name} chunker=lines");

    let blocks_chars = support::collect_final_blocks(support::chunk_chars(markdown), opts.clone());
    assert_eq!(blocks_chars, expected, "case={case_name} chunker=chars");

    for t in 0..trials {
        let blocks_rand = support::collect_final_blocks(
            support::chunk_pseudo_random(markdown, case_name, t, max_bytes),
            opts.clone(),
        );
        assert_eq!(blocks_rand, expected, "case={case_name} chunker=rand t={t}");
    }
}

#[test]
fn streamdown_benchmark_suite_chunking_invariance() {
    // Inputs sourced from Streamdown's `__benchmarks__/parse-blocks.bench.ts`.
    let single_block = "# Heading\n\nThis is a paragraph.";

    let multiple_blocks_10 = r#"
# Heading 1

This is paragraph 1.

## Heading 2

This is paragraph 2.

- List item 1
- List item 2

> Blockquote text
"#;

    let single_code_block = r#"
Some text

```javascript
const x = 1;
const y = 2;
```

More text
"#;

    let math_with_split_delimiters = r#"
Some text

$$

x^2 + y^2 = z^2

$$

More text
"#;

    let multiple_html_blocks = r#"
<div>First block</div>

Some markdown

<section>
  <p>Second block</p>
</section>

More markdown
"#;

    let with_footnotes = r#"
This is text with a footnote[^1].

Here's another footnote[^note].

[^1]: This is the first footnote.
[^note]: This is a named footnote.
"#;

    let simple_table = r#"
| Header 1 | Header 2 |
|----------|----------|
| Cell 1   | Cell 2   |
| Cell 3   | Cell 4   |
"#;

    let opts = Options::default();
    assert_invariant("single_block", single_block, opts.clone(), 16, 64);
    assert_invariant(
        "multiple_blocks_10",
        multiple_blocks_10,
        opts.clone(),
        16,
        64,
    );
    assert_invariant("single_code_block", single_code_block, opts.clone(), 16, 64);
    assert_invariant(
        "math_with_split_delimiters",
        math_with_split_delimiters,
        opts.clone(),
        16,
        64,
    );
    assert_invariant(
        "multiple_html_blocks",
        multiple_html_blocks,
        opts.clone(),
        16,
        64,
    );
    assert_invariant("with_footnotes", with_footnotes, opts.clone(), 16, 64);
    assert_invariant("simple_table", simple_table, opts.clone(), 16, 64);
}

#[test]
fn incremark_inspired_suite_chunking_invariance() {
    // Inputs inspired by Incremark's `IncremarkParser.*.test.ts`.
    let paragraph = "Hello, World!";
    let multiple_paragraphs = "第一段\n\n第二段";
    let headings = "# 标题一\n\n## 标题二\n\n内容";
    let code_block = "```js\nconsole.log(\"hi\")\n```\n\n段落";
    let gfm_table = "| A | B |\n|---|---|\n| 1 | 2 |";

    let opts = Options::default();
    assert_invariant("incremark_paragraph", paragraph, opts.clone(), 8, 32);
    assert_invariant(
        "incremark_multiple_paragraphs",
        multiple_paragraphs,
        opts.clone(),
        8,
        32,
    );
    assert_invariant("incremark_headings", headings, opts.clone(), 8, 32);
    assert_invariant("incremark_code_block", code_block, opts.clone(), 8, 32);
    assert_invariant("incremark_gfm_table", gfm_table, opts.clone(), 8, 32);
}

#[test]
fn chunking_invariance_handles_crlf_split_across_chunks() {
    let opts = Options::default();
    let markdown = "A\r\n\r\nB\r\n";

    let expected = support::collect_final_blocks(support::chunk_whole(markdown), opts.clone());
    let blocks_split_crlf = support::collect_final_blocks(
        vec![
            "A\r".to_string(),
            "\n\r".to_string(),
            "\nB\r".to_string(),
            "\n".to_string(),
        ],
        opts,
    );
    assert_eq!(blocks_split_crlf, expected);
}
