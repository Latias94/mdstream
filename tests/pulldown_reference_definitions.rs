#![cfg(feature = "pulldown")]

use mdstream::adapters::pulldown::{PulldownAdapter, PulldownAdapterOptions};
use mdstream::{MdStream, Options, ReferenceDefinitionsMode};
use pulldown_cmark::{Event, Tag};

fn contains_link(events: &[Event<'static>]) -> bool {
    events
        .iter()
        .any(|e| matches!(e, Event::Start(Tag::Link { .. })))
}

#[test]
fn pulldown_adapter_reparses_invalidated_blocks_with_reference_definitions() {
    let opts = Options {
        reference_definitions: ReferenceDefinitionsMode::Invalidate,
        ..Default::default()
    };

    let mut s = MdStream::new(opts);
    let mut a = PulldownAdapter::new(PulldownAdapterOptions::default());

    // Start paragraph with a shortcut reference.
    s.append("See [ref].\n\n");

    // Starting the definition line commits the previous paragraph block.
    let u1 = s.append("[ref]: https://example.com\n");
    a.apply_update(&u1);
    let block1_id = u1.committed[0].id;

    let e1 = a.committed_events(block1_id).expect("events");
    assert!(!contains_link(e1), "definition not committed yet");

    // Commit the definition block, which should invalidate block 1.
    s.append("\n");
    let u2 = s.append("Next\n");
    a.apply_update(&u2);

    assert!(u2.invalidated.contains(&block1_id));
    let e2 = a.committed_events(block1_id).expect("events");
    assert!(
        contains_link(e2),
        "block should be re-parsed with definitions"
    );
}
