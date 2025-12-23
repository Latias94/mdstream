//! Demonstrate `Update.reset` when switching into single-block footnote mode.
//!
//! Run:
//!   cargo run --example footnotes_reset

use mdstream::{DocumentState, FootnotesMode, MdStream, Options};

fn main() {
    let opts = Options {
        footnotes: FootnotesMode::SingleBlock,
        ..Default::default()
    };

    let mut stream = MdStream::new(opts);
    let mut state = DocumentState::new();

    let chunks = [
        "# Before footnotes\n\n",
        "Normal paragraph.\n\n",
        // This chunk introduces a footnote reference and triggers a scope-driven reset.
        "Now a footnote appears[^1].\n\n",
        // Later, the definition arrives (still single-block mode).
        "[^1]: The footnote definition.\n",
    ];

    for (i, chunk) in chunks.iter().enumerate() {
        println!("\n== tick {i} ==");
        let applied = state.apply(stream.append(chunk));
        println!(
            "reset={} invalidated={:?}",
            applied.reset, applied.invalidated
        );
        println!(
            "state: committed={} pending={}",
            state.committed().len(),
            state.pending().is_some()
        );
        if let Some(p) = state.pending() {
            println!("pending kind={:?} bytes={}", p.kind, p.raw.len());
        }
    }

    println!("\n== finalize ==");
    let applied = state.apply(stream.finalize());
    println!(
        "reset={} invalidated={:?}",
        applied.reset, applied.invalidated
    );
    println!("final committed={}", state.committed().len());
    if let Some(last) = state.committed().last() {
        println!("final block kind={:?} bytes={}", last.kind, last.raw.len());
    }
}
