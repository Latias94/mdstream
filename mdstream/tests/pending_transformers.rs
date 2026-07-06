use mdstream::{FnPendingTransformer, MdStream, Options, TerminatorOptions};

#[test]
fn pending_transformer_can_override_display() {
    let mut s = MdStream::new(Options::default());
    s.push_pending_transformer(FnPendingTransformer(
        |input: mdstream::PendingTransformInput<'_>| Some(format!("{}<<t>>", input.display)),
    ));

    let u1 = s.append("hi");
    let p1 = u1.pending.expect("pending");
    assert_eq!(p1.raw, "hi");
    assert_eq!(p1.display.as_deref(), Some("hi<<t>>"));

    let u2 = s.append(" there");
    let p2 = u2.pending.expect("pending");
    assert_eq!(p2.raw, "hi there");
    assert_eq!(p2.display.as_deref(), Some("hi there<<t>>"));
}

#[test]
fn default_pending_repair_is_observable_through_stream() {
    let mut link_stream = MdStream::new(Options::default());
    let link_update = link_stream.append("See [docs](");
    let link_pending = link_update.pending.expect("pending");
    assert_eq!(link_pending.raw, "See [docs](");
    assert_eq!(
        link_pending.display.as_deref(),
        Some("See [docs](streamdown:incomplete-link)")
    );

    let mut formatting_stream = MdStream::new(Options::default());
    let formatting_update = formatting_stream.append("Text with **bold and `code");
    let formatting_pending = formatting_update.pending.expect("pending");
    assert_eq!(formatting_pending.raw, "Text with **bold and `code");
    assert_eq!(
        formatting_pending.display.as_deref(),
        Some("Text with **bold and `code**`")
    );
}

#[test]
fn built_in_incomplete_link_placeholder_transformer_can_be_enabled() {
    // Disable built-in terminator handling so this test exercises the transformer.
    let opts = Options {
        terminator: TerminatorOptions {
            links: false,
            images: false,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut s = MdStream::new(opts)
        .with_pending_transformer(mdstream::IncompleteLinkPlaceholderTransformer::default());

    let u = s.append("See [docs](");
    let p = u.pending.expect("pending");
    assert_eq!(p.raw, "See [docs](");
    assert_eq!(
        p.display.as_deref(),
        Some("See [docs](streamdown:incomplete-link)")
    );
}

#[test]
fn built_in_incomplete_image_drop_transformer_can_be_enabled() {
    let opts = Options {
        terminator: TerminatorOptions {
            links: false,
            images: false,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut s = MdStream::new(opts)
        .with_pending_transformer(mdstream::IncompleteImageDropTransformer::default());

    let u = s.append("Before ![alt](");
    let p = u.pending.expect("pending");
    assert_eq!(p.raw, "Before ![alt](");
    assert_eq!(p.display.as_deref(), Some("Before "));
}
