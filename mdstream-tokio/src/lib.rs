//! Tokio glue for `mdstream`.
//!
//! `mdstream` remains runtime-agnostic and synchronous. This crate provides
//! bounded feeding and actor helpers for async producers:
//!
//! - Losslessly coalesce tiny chunks with bounded backpressure.
//! - Run an actor that owns [`mdstream::StreamEngine`] and emits atomic,
//!   replayable [`mdstream_protocol::ChangeSet`] batches.
//! - Close actor input and join without losing output that was not read yet.
//!
//! For a full TUI example, see `cargo run -p mdstream-tokio --example agent_tui`.

mod actor;
mod options;
mod receiver;
mod sender;

pub use actor::{ActorCommand, ActorResult, StreamEngineActor, spawn_stream_engine_actor};
pub use options::{CoalesceOptions, CoalescePreset};
pub use receiver::{CoalesceStats, CoalescedChunk, CoalescingReceiver, FlushReason};
pub use sender::{BackpressurePolicy, DeltaSender, SendError, SendOutcome};

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc;

    use super::*;

    #[tokio::test]
    async fn coalesces_until_newline_by_default() {
        let (tx, rx) = mpsc::channel::<String>(8);
        let mut cr = CoalescingReceiver::new(rx, CoalesceOptions::default());

        tx.send("he".to_string()).await.unwrap();
        tx.send("llo".to_string()).await.unwrap();
        tx.send("\n".to_string()).await.unwrap();

        let got = cr.recv_with_meta().await.unwrap();
        assert_eq!(got.text, "hello\n");
        assert_eq!(got.reason, FlushReason::Newline);
        assert_eq!(got.merged_messages, 3);

        let stats = cr.stats();
        assert_eq!(stats.total_in_messages, 3);
        assert_eq!(stats.total_out_chunks, 1);
        assert_eq!(stats.last_reason, Some(FlushReason::Newline));
    }
}
