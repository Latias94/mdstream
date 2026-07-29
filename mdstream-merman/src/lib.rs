#![forbid(unsafe_code)]

//! Optional headless Mermaid processing for mdstream.
//!
//! This crate is deliberately excluded from mdstream's Rust 1.85 workspace.
//! It pins Merman on its Rust 1.95 lane and implements
//! [`mdstream_processors::ContentProcessor`] without adding renderer output to
//! canonical Content IR.
//!
//! Source limits are checked by this adapter before invoking Merman. For
//! flowchart and class diagrams, Merman's model and label limits run after the
//! semantic model has been materialized and before layout. Other diagram
//! families do not currently have equivalent model-stage hard caps and remain
//! trusted cooperative work. The SVG byte limit is different again: Merman
//! builds the complete `String` first, so this adapter applies it only before
//! creating a retained processor artifact. It is not a renderer
//! peak-allocation guarantee. Isolate Merman in a separate process when
//! processing adversarial input.

mod options;
mod processor;

pub use options::*;
pub use processor::*;
