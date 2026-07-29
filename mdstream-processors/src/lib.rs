#![forbid(unsafe_code)]

//! Renderer-neutral processor requests and derived artifact lifecycle.
//!
//! [`ArtifactHost`] is a synchronous state machine that issues owned requests,
//! validates result freshness, and accounts for retained artifacts. It does not
//! own an async runtime or execute processor code while mutating canonical
//! document state. Callers schedule [`ProcessorRequest`] values on their own
//! thread, worker, or process and submit the resulting [`ProcessorResult`].
//!
//! In-process [`ContentProcessor`] implementations are trusted, cooperative
//! code. [`run_catching`] contains unwind panics when the build uses
//! `panic = "unwind"`; applications must isolate untrusted processors in a
//! separate process or worker. Artifacts and processor failures are derived
//! state and never enter `mdstream-protocol` snapshots or reducer operations.

mod citation;
mod error;
mod host;
mod key;
mod limits;
mod request;
mod result;
mod store;

pub use citation::*;
pub use error::*;
pub use host::*;
pub use key::*;
pub use limits::*;
pub use request::*;
pub use result::*;
