mod support;

use mdstream::{FootnotesMode, Options, ReferenceDefinitionsMode};
use mdstream_conformance::{ChunkSchedule, replay_protocol_trace, source_only_trace};
use mdstream_protocol::Epoch;
use proptest::prelude::*;
use proptest::test_runner::TestCaseResult;

fn plain_fragment() -> impl Strategy<Value = String> {
    let mut chars = Vec::new();
    chars.extend('a'..='z');
    chars.extend('A'..='Z');
    chars.extend('0'..='9');
    chars.extend([
        ' ', '_', '-', '*', '`', '~', '[', ']', '(', ')', '|', ':', '.', ',', '!', '?', '/', '\\',
        '<', '>', '=', '$', '#', '+', '中', '文', 'é', 'Ω', '🙂',
    ]);
    let ch = prop::sample::select(chars);

    prop::collection::vec(ch, 0..24).prop_map(|chars| chars.into_iter().collect())
}

fn markdown_token() -> impl Strategy<Value = String> {
    prop_oneof![
        plain_fragment(),
        prop::sample::select(vec![
            "\n",
            "\r\n",
            "\n\n",
            "\r\n\r\n",
            "# Heading\n",
            "## 二级标题\r\n",
            "> quoted line\n",
            "- list item\n",
            "1. numbered item\n",
            "- [ ] task item\n",
            "| A | B |\n|---|---|\n| 1 | 2 |\n",
            "```rust\n",
            "```\n",
            "~~~\n",
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
            "__",
            "~~",
            "`",
        ])
        .prop_map(str::to_string),
    ]
}

fn markdownish_document() -> impl Strategy<Value = String> {
    prop::collection::vec(markdown_token(), 0..40)
        .prop_map(|tokens| tokens.concat())
        .prop_map(|doc| doc.chars().take(2048).collect())
}

fn assert_chunking_invariant(
    markdown: &str,
    opts: Options,
    seed: u64,
    max_bytes: usize,
) -> TestCaseResult {
    let expected = support::collect_final_blocks(support::chunk_whole(markdown), opts.clone());

    let by_line = support::collect_final_blocks(support::chunk_lines(markdown), opts.clone());
    prop_assert_eq!(&by_line, &expected);

    let by_char = support::collect_final_blocks(support::chunk_chars(markdown), opts.clone());
    prop_assert_eq!(&by_char, &expected);

    let by_random = support::collect_final_blocks(
        support::chunk_pseudo_random(markdown, "proptest_chunking", seed, max_bytes),
        opts.clone(),
    );
    prop_assert_eq!(&by_random, &expected);

    let borrowed_random = support::collect_final_blocks_borrowed(
        support::chunk_pseudo_random(markdown, "proptest_chunking_borrowed", seed, max_bytes),
        opts,
    );
    prop_assert_eq!(&borrowed_random, &expected);

    Ok(())
}

fn assert_protocol_schedule_invariant(
    markdown: &str,
    seed: u64,
    max_bytes: usize,
) -> TestCaseResult {
    let whole = ChunkSchedule::Whole.slices(markdown).unwrap();
    let seeded = ChunkSchedule::Seeded {
        label: "proptest.protocol".to_string(),
        seed,
        trial: 0,
        max_bytes,
    }
    .slices(markdown)
    .unwrap();
    let whole = source_only_trace("whole", "whole", Epoch::new(1), whole).unwrap();
    let seeded = source_only_trace("seeded", "seeded", Epoch::new(1), seeded).unwrap();
    let expected = replay_protocol_trace(&whole)
        .unwrap()
        .normalized_final_snapshot();
    let actual = replay_protocol_trace(&seeded)
        .unwrap()
        .normalized_final_snapshot();
    prop_assert_eq!(actual, expected);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 96,
        max_shrink_iters: 4096,
        ..ProptestConfig::default()
    })]

    #[test]
    fn generated_markdown_chunking_is_invariant(
        markdown in markdownish_document(),
        seed in any::<u64>(),
        max_bytes in 1usize..32,
    ) {
        assert_chunking_invariant(&markdown, Options::default(), seed, max_bytes)?;
    }

    #[test]
    fn generated_markdown_chunking_is_invariant_with_invalidations(
        markdown in markdownish_document(),
        seed in any::<u64>(),
        max_bytes in 1usize..32,
    ) {
        let opts = Options {
            footnotes: FootnotesMode::Invalidate,
            reference_definitions: ReferenceDefinitionsMode::Invalidate,
            ..Options::default()
        };

        assert_chunking_invariant(&markdown, opts, seed, max_bytes)?;
    }

    #[test]
    fn generated_utf8_schedules_replay_to_one_normalized_protocol_snapshot(
        markdown in markdownish_document(),
        seed in any::<u64>(),
        max_bytes in 1usize..32,
    ) {
        assert_protocol_schedule_invariant(&markdown, seed, max_bytes)?;
    }
}
