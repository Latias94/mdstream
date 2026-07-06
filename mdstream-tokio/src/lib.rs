//! Tokio glue for `mdstream`.
//!
//! `mdstream` is runtime-agnostic and is intended to be owned by a UI thread (single-owner).
//! This crate provides small helpers for async producers:
//!
//! - Coalesce tiny deltas into larger chunks (newline-gated and/or time-window flush).
//! - Optionally run an actor task that owns `MdStream` and emits owned `Update`s.
//!
//! For a full TUI example, see `cargo run -p mdstream-tokio --example agent_tui`.

mod actor;
mod options;
mod receiver;
mod sender;

pub use actor::spawn_mdstream_actor;
pub use options::{CoalesceOptions, CoalescePreset};
pub use receiver::{CoalesceStats, CoalescedChunk, CoalescingReceiver, FlushReason};
pub use sender::{BackpressurePolicy, DeltaSender, SendError, SendOutcome};

#[cfg(test)]
mod tests {
    use std::time::Duration;

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

    #[tokio::test]
    async fn delta_sender_drop_new_drops_when_full() {
        let (tx, mut rx) = mpsc::channel::<String>(1);
        let mut s = DeltaSender::new(tx, BackpressurePolicy::DropNew);

        assert_eq!(s.send("a").await.unwrap(), SendOutcome::Sent);
        // Channel is full (receiver not drained yet).
        assert_eq!(s.send("b").await.unwrap(), SendOutcome::Dropped);

        assert_eq!(rx.recv().await.as_deref(), Some("a"));
        drop(s);
        let got = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("receiver should complete once all senders are dropped");
        assert_eq!(got, None);
    }

    #[tokio::test]
    async fn delta_sender_coalesce_local_flushes_eventually() {
        let (tx, mut rx) = mpsc::channel::<String>(1);
        let mut s = DeltaSender::new(tx, BackpressurePolicy::CoalesceLocal);
        s.set_local_max_bytes(4);

        // Fill channel so try_send will be full.
        s.tx.try_send("x".to_string()).unwrap();

        assert_eq!(s.send("ab").await.unwrap(), SendOutcome::Buffered);
        assert_eq!(s.send("cd").await.unwrap(), SendOutcome::Buffered); // reaches max_bytes, tries, still full

        // Drain one message, then force flush.
        assert_eq!(rx.recv().await.as_deref(), Some("x"));
        assert_eq!(s.flush().await.unwrap(), SendOutcome::Sent);
        assert_eq!(rx.recv().await.as_deref(), Some("abcd"));
    }
}
