use mdstream::{FootnotesMode, MdStream, Options, ReferenceDefinitionsMode, Update};

fn assert_owned_and_borrowed_updates_match(case_name: &str, opts: Options, chunks: &[&str]) {
    let mut owned_stream = MdStream::new(opts.clone());
    let mut borrowed_stream = MdStream::new(opts);

    for (index, chunk) in chunks.iter().enumerate() {
        let owned = owned_stream.append(chunk);
        let borrowed = borrowed_stream.append_ref(chunk).to_owned();
        assert_eq!(
            borrowed, owned,
            "case={case_name} chunk_index={index} chunk={chunk:?}"
        );
    }

    let owned = owned_stream.finalize();
    let borrowed = borrowed_stream.finalize_ref().to_owned();
    assert_eq!(borrowed, owned, "case={case_name} finalize");
}

fn apply_updates(opts: Options, chunks: &[&str], borrowed: bool) -> Vec<Update> {
    let mut stream = MdStream::new(opts);
    let mut updates = Vec::new();

    for chunk in chunks {
        let update = if borrowed {
            stream.append_ref(chunk).to_owned()
        } else {
            stream.append(chunk)
        };
        updates.push(update);
    }

    let final_update = if borrowed {
        stream.finalize_ref().to_owned()
    } else {
        stream.finalize()
    };
    updates.push(final_update);
    updates
}

#[test]
fn append_and_append_ref_emit_equivalent_updates() {
    let cases = vec![
        (
            "plain_blocks",
            Options::default(),
            vec!["Hello", "\n\n", "World\n"],
        ),
        (
            "code_fence_pending_display",
            Options::default(),
            vec!["```rs\n", "fn main() {\n", "}\n"],
        ),
        (
            "reference_definition_invalidation",
            Options {
                reference_definitions: ReferenceDefinitionsMode::Invalidate,
                ..Default::default()
            },
            vec![
                "See [ref].\n\n",
                "[ref]: https://example.com\n",
                "\n",
                "Next\n",
            ],
        ),
        (
            "footnote_single_block_reset",
            Options {
                footnotes: FootnotesMode::SingleBlock,
                ..Default::default()
            },
            vec!["Intro\n\n", "[^1]: note\n", "    continued\n"],
        ),
    ];

    for (case_name, opts, chunks) in cases {
        assert_owned_and_borrowed_updates_match(case_name, opts, &chunks);
    }
}

#[test]
fn append_ref_trace_can_be_replayed_as_owned_updates() {
    let opts = Options {
        reference_definitions: ReferenceDefinitionsMode::Invalidate,
        ..Default::default()
    };
    let chunks = [
        "# Title\n\n",
        "See [Ref].\n\n",
        "[ref]: https://example.com\n",
        "\n",
        "```text\npartial",
    ];

    let owned_updates = apply_updates(opts.clone(), &chunks, false);
    let borrowed_updates = apply_updates(opts, &chunks, true);

    assert_eq!(borrowed_updates, owned_updates);
    assert!(
        borrowed_updates
            .iter()
            .any(|update| !update.invalidated.is_empty()),
        "trace should characterize reference invalidation"
    );
    assert!(
        borrowed_updates
            .iter()
            .any(|update| update.pending.as_ref().is_some_and(|pending| {
                pending
                    .display
                    .as_deref()
                    .is_some_and(|display| display.ends_with("```\n") && pending.raw != display)
            })),
        "trace should characterize incomplete fence display repair"
    );
}
