mod support;

use proptest::prelude::*;
use proptest::test_runner::TestCaseResult;

fn plain_fragment() -> impl Strategy<Value = String> {
    let mut characters = Vec::new();
    characters.extend('a'..='z');
    characters.extend('A'..='Z');
    characters.extend('0'..='9');
    characters.extend([
        ' ', '_', '-', '*', '`', '~', '[', ']', '(', ')', '|', ':', '.', ',', '!', '?', '/', '\\',
        '<', '>', '=', '$', '#', '+', '中', '文', 'é', 'Ω', '🙂',
    ]);
    prop::collection::vec(prop::sample::select(characters), 0..24)
        .prop_map(|characters| characters.into_iter().collect())
}

fn markdown_token() -> impl Strategy<Value = String> {
    prop_oneof![
        plain_fragment(),
        prop::sample::select(vec![
            "\n",
            "\r\n",
            "\n\n",
            "# Heading\n",
            "> quoted line\n",
            "- list item\n",
            "1. numbered item\n",
            "- [ ] task item\n",
            "| A | B |\n|---|---|\n| 1 | 2 |\n",
            "```rust\n",
            "```\n",
            "$$\n",
            "[^note]: footnote body\n",
            "[ref]: https://example.test\n",
            "<div>\n",
            "</div>\n",
            "<!-- comment\n",
            "-->\n",
            "[link",
            "![alt",
            "**",
            "~~",
            "`",
        ])
        .prop_map(str::to_string),
    ]
}

fn markdownish_document() -> impl Strategy<Value = String> {
    prop::collection::vec(markdown_token(), 0..40)
        .prop_map(|tokens| tokens.concat())
        .prop_map(|document| document.chars().take(2048).collect())
}

fn assert_chunking_invariant(markdown: &str, seed: u64, max_bytes: usize) -> TestCaseResult {
    let expected = support::replay(support::chunk_whole(markdown));
    prop_assert_eq!(&support::replay(support::chunk_lines(markdown)), &expected);
    prop_assert_eq!(&support::replay(support::chunk_chars(markdown)), &expected);
    prop_assert_eq!(
        support::replay(support::chunk_pseudo_random(
            markdown,
            "proptest.canonical",
            seed,
            max_bytes,
        )),
        expected
    );
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 96,
        max_shrink_iters: 4096,
        ..ProptestConfig::default()
    })]

    #[test]
    fn generated_markdown_reaches_one_canonical_snapshot(
        markdown in markdownish_document(),
        seed in any::<u64>(),
        max_bytes in 1usize..32,
    ) {
        assert_chunking_invariant(&markdown, seed, max_bytes)?;
    }
}
