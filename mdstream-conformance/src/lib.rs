//! Engine-neutral conformance fixtures for streaming rich content.
//!
//! This crate deliberately depends on the canonical protocol, not on a
//! Markdown parser or UI framework. Producers can use the same chunk schedules,
//! fixture envelope, and replay laws while their internal implementation
//! changes.

#![forbid(unsafe_code)]

mod assertions;
mod budget;
mod chunks;
mod fixture;
mod trace;

pub use assertions::*;
pub use budget::*;
pub use chunks::*;
pub use fixture::*;
pub use trace::*;
