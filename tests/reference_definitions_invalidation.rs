use mdstream::FootnotesMode;
use mdstream::{MdStream, Options, ReferenceDefinitionsMode};

#[test]
fn emits_invalidated_when_reference_definition_is_committed() {
    let opts = Options {
        reference_definitions: ReferenceDefinitionsMode::Invalidate,
        ..Default::default()
    };

    let mut s = MdStream::new(opts);

    // First paragraph with a shortcut reference. This won't be committed until a later non-empty line arrives.
    let u1 = s.append("See [ref].\n\n");
    assert!(u1.committed.is_empty());

    // Starting the definition line will commit the previous paragraph block.
    let u2 = s.append("[ref]: https://example.com\n");
    assert_eq!(u2.committed.len(), 1);
    assert!(u2.invalidated.is_empty(), "definition not committed yet");

    // Commit the definition block by introducing a blank line then a non-empty line.
    s.append("\n");
    let u3 = s.append("Next\n");
    assert!(!u3.committed.is_empty());
    assert_eq!(u3.invalidated, vec![mdstream::BlockId(1)]);
}

#[test]
fn stability_first_mode_does_not_emit_invalidations() {
    let opts = Options {
        reference_definitions: ReferenceDefinitionsMode::StabilityFirst,
        ..Default::default()
    };

    let mut s = MdStream::new(opts);
    s.append("See [ref].\n\n");
    let u2 = s.append("[ref]: https://example.com\n");
    assert_eq!(u2.committed.len(), 1);
    s.append("\n");
    let u3 = s.append("Next\n");
    assert!(u3.invalidated.is_empty());
}

#[test]
fn invalidation_normalizes_reference_labels() {
    let opts = Options {
        reference_definitions: ReferenceDefinitionsMode::Invalidate,
        ..Default::default()
    };

    let mut s = MdStream::new(opts);

    // Use a reference label that differs by case/whitespace from the later definition.
    s.append("See [Foo][Ref \t Name].\n\n");

    // Commit the definition block by including a blank line and a new non-empty line.
    let u = s.append("[ref name]: https://example.com\n\nNext\n");
    assert!(
        u.invalidated.contains(&mdstream::BlockId(1)),
        "definition commit should invalidate prior usage"
    );
}

#[test]
fn footnote_definitions_do_not_trigger_reference_invalidations() {
    // Avoid SingleBlock footnote transitions affecting this test.
    let opts = Options {
        footnotes: FootnotesMode::Invalidate,
        reference_definitions: ReferenceDefinitionsMode::Invalidate,
        ..Default::default()
    };

    let mut s = MdStream::new(opts);

    s.append("See [ref].\n\n");

    // This looks like a reference definition, but it's a footnote definition and must be excluded.
    let u = s.append("[^ref]: not a reference definition\n\nNext\n");
    assert!(u.invalidated.is_empty());
}

#[test]
fn reference_definitions_inside_code_fences_do_not_trigger_invalidations() {
    let opts = Options {
        reference_definitions: ReferenceDefinitionsMode::Invalidate,
        ..Default::default()
    };

    let mut s = MdStream::new(opts);

    s.append("See [ref].\n\n");

    // The definition is inside a code fence, so it must not act as a real reference definition.
    let u = s.append("```text\n[ref]: https://example.com\n```\n\nNext\n");
    assert!(u.invalidated.is_empty());
}
