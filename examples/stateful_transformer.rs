//! Demonstrate a stateful pending transformer.
//!
//! This example shows how to use `Arc<AtomicUsize>` for thread-safe state
//! in closures that implement `PendingTransformer`.
//!
//! Run:
//!   cargo run --example stateful_transformer

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use mdstream::{FnPendingTransformer, MdStream, Options};

fn main() {
    let mut s = MdStream::new(Options::default());

    // PendingTransformer requires Send + Sync, so mutable state needs Arc<Atomic*>
    let seen = Arc::new(AtomicUsize::new(0));
    let seen_clone = Arc::clone(&seen);
    s.push_pending_transformer(FnPendingTransformer(
        move |input: mdstream::PendingTransformInput<'_>| {
            let count = seen_clone.fetch_add(1, Ordering::Relaxed) + 1;
            Some(format!("[seen={count}] {}", input.display))
        },
    ));

    for chunk in ["Hello", " ", "**wor", "ld"] {
        let u = s.append(chunk);
        if let Some(p) = u.pending {
            println!("pending: {}", p.display_or_raw());
        }
    }
}
