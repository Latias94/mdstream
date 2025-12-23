//! Incremental `pulldown-cmark` parsing with `mdstream`.
//!
//! Run:
//!   cargo run --features pulldown --example pulldown_incremental

#[cfg(not(feature = "pulldown"))]
fn main() {
    eprintln!("This example requires the `pulldown` feature.");
    eprintln!("Run: cargo run --features pulldown --example pulldown_incremental");
}

#[cfg(feature = "pulldown")]
fn main() {
    use mdstream::adapters::pulldown::{PulldownAdapter, PulldownAdapterOptions};
    use mdstream::{MdStream, Options, ReferenceDefinitionsMode};
    use pulldown_cmark::{Event, Options as PulldownOptions, Tag};

    let mut opts = Options::default();
    // Optional: demonstrate invalidation when reference definitions arrive late.
    opts.reference_definitions = ReferenceDefinitionsMode::Invalidate;
    let mut s = MdStream::new(opts)
        .with_pending_transformer(mdstream::IncompleteLinkPlaceholderTransformer::default());
    s.push_pending_transformer(mdstream::IncompleteImageDropTransformer::default());

    let mut adapter = PulldownAdapter::new(PulldownAdapterOptions {
        pulldown: PulldownOptions::ENABLE_TABLES | PulldownOptions::ENABLE_STRIKETHROUGH,
        prefer_display_for_pending: true,
    });

    let chunks = [
        "See a [ref].\n\n",
        "Now define it:\n\n",
        "[ref]: https://example.com\n\n",
        "And some `code`.\n",
    ];

    let mut tracked: Option<mdstream::BlockId> = None;

    for (i, chunk) in chunks.iter().enumerate() {
        println!("\n== tick {i} ==");
        let update = s.append(chunk);

        if !update.invalidated.is_empty() {
            println!("invalidated: {:?}", update.invalidated);
        }

        adapter.apply_update(&update);

        for b in &update.committed {
            println!("committed block id={} kind={:?}", b.id.0, b.kind);
            let events = adapter.committed_events(b.id).unwrap_or(&[]);
            println!("  events.len={}", events.len());
            // Track the first committed block so we can show invalidation effects later.
            if tracked.is_none() {
                tracked = Some(b.id);
            }
        }

        if let Some(id) = tracked {
            let events = adapter.committed_events(id).unwrap_or(&[]);
            let has_link = events
                .iter()
                .any(|e| matches!(e, Event::Start(Tag::Link { .. })));
            println!("tracked committed id={} has_link={}", id.0, has_link);
        }

        if let Some(p) = &update.pending {
            let events = adapter.parse_pending(p);
            let has_link = events
                .iter()
                .any(|e| matches!(e, Event::Start(Tag::Link { .. })));
            println!(
                "pending block id={} kind={:?} events.len={} has_link={}",
                p.id.0,
                p.kind,
                events.len(),
                has_link
            );
        } else {
            println!("pending: <none>");
        }
    }
}
