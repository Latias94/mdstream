//! Parser- and renderer-neutral state protocol for streaming rich content.
//!
//! This crate owns mdstream's canonical source, Content IR, ordered change
//! sets, snapshots, and reducer. Producers emit [`ChangeSet`] values;
//! consumers apply them through [`Reducer`] and render the resulting
//! [`Document`] without reparsing Markdown.
//!
//! # Stability
//!
//! The `0.4-draft` schema is an internal draft. Its Rust and JSON surfaces may
//! change until compiler, processor, and adoption conformance promotes it to a
//! candidate. Renderer artifacts and parser-specific types intentionally do
//! not belong in this crate.

mod delta;
mod document;
mod error;
mod ids;
mod ir;
mod lifecycle;
mod wire;

pub use delta::*;
pub use document::*;
pub use error::*;
pub use ids::*;
pub use ir::*;
pub use lifecycle::*;
pub use wire::*;
