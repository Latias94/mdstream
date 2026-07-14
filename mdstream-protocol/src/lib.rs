//! Parser- and renderer-neutral state protocol for streaming rich content.
//!
//! This crate owns mdstream's canonical source, Content IR, ordered change
//! sets, snapshots, and reducer. Producers emit [`ChangeSet`] values;
//! consumers apply them through [`Reducer`] and render the resulting
//! [`Document`] without reparsing Markdown.
//!
//! # Stability
//!
//! `mdstream.content/0.4-candidate.1` is the current binding candidate. Its Rust
//! and JSON surfaces may still change through breaking candidate revisions
//! until adoption conformance promotes the contract to final 0.4. Renderer
//! artifacts and parser-specific types intentionally do not belong here.

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
