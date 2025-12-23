//! Minimal `mdstream` usage (no adapters, no analyzers).
//!
//! Run:
//!   cargo run --example minimal

use mdstream::{DocumentState, MdStream, Options};

fn main() {
    let mut stream = MdStream::new(Options::default());
    let mut state = DocumentState::new();

    let chunks = [
        "# Title\n\n",
        "Hello **wor",
        "ld**.\n\n",
        "A list:\n",
        "- item 1\n",
        "- item 2\n",
    ];

    for (i, chunk) in chunks.iter().enumerate() {
        println!("\n== tick {i} ==");
        let applied = state.apply(stream.append(chunk));

        if applied.reset {
            println!("reset: true");
        }
        if !applied.invalidated.is_empty() {
            println!("invalidated: {:?}", applied.invalidated);
        }

        println!("committed={}", state.committed().len());
        if let Some(p) = state.pending() {
            println!(
                "pending id={} kind={:?} text={:?}",
                p.id.0,
                p.kind,
                p.display_or_raw()
            );
        } else {
            println!("pending: <none>");
        }
    }

    println!("\n== finalize ==");
    let applied = state.apply(stream.finalize());
    println!(
        "final reset={} invalidated={:?}",
        applied.reset, applied.invalidated
    );
    println!("final committed={}", state.committed().len());
}
