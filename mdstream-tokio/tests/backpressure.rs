use std::time::Duration;

use mdstream_tokio::{
    BackpressurePolicy, CoalesceOptions, CoalescingReceiver, DeltaSender, SendOutcome,
};
use tokio::sync::mpsc;

#[tokio::test]
async fn lossless_coalescing_applies_backpressure_at_local_limit() {
    let (tx, mut rx) = mpsc::channel::<String>(1);
    tx.send("occupied".to_string()).await.unwrap();

    let mut sender = DeltaSender::new(tx, BackpressurePolicy::CoalesceLocal, 4, 1024);
    let send = tokio::spawn(async move {
        let outcome = sender.send("abcd").await;
        (sender, outcome)
    });

    tokio::task::yield_now().await;
    assert!(
        !send.is_finished(),
        "reaching the local limit must wait for bounded channel capacity"
    );

    assert_eq!(rx.recv().await.as_deref(), Some("occupied"));
    let (sender, outcome) = tokio::time::timeout(Duration::from_secs(1), send)
        .await
        .expect("send should resume after downstream capacity is available")
        .expect("sender task should not panic");
    assert_eq!(outcome, Ok(SendOutcome::Sent));
    assert_eq!(rx.recv().await.as_deref(), Some("abcd"));
    drop(sender);
    assert_eq!(rx.recv().await, None);
}

#[tokio::test]
async fn cancelling_flush_keeps_buffered_content_for_retry() {
    let (tx, mut rx) = mpsc::channel::<String>(1);
    tx.send("occupied".to_string()).await.unwrap();

    let mut sender = DeltaSender::new(tx, BackpressurePolicy::CoalesceLocal, 4, 1024);
    assert_eq!(sender.send("ab").await, Ok(SendOutcome::Buffered));

    assert!(
        tokio::time::timeout(Duration::from_millis(10), sender.flush())
            .await
            .is_err(),
        "flush should wait while the bounded channel is full"
    );

    assert_eq!(rx.recv().await.as_deref(), Some("occupied"));
    assert_eq!(sender.flush().await, Ok(SendOutcome::Sent));
    let retried = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("retry must deliver the retained local buffer");
    assert_eq!(retried.as_deref(), Some("ab"));
}

#[tokio::test]
async fn blocking_sender_and_receiver_preserve_one_byte_order() {
    let source = "alpha\r\nbeta\ngamma";
    let chunks = source
        .bytes()
        .map(|byte| char::from(byte).to_string())
        .collect();

    assert_eq!(
        round_trip(BackpressurePolicy::Block, 1, chunks).await,
        source
    );
}

#[tokio::test]
async fn lossless_local_coalescing_preserves_bursty_unicode_order() {
    let chunks = ["A", "界", "-", "burst", "\n", "尾", "🙂"]
        .into_iter()
        .map(str::to_string)
        .collect();

    assert_eq!(
        round_trip(BackpressurePolicy::CoalesceLocal, 6, chunks).await,
        "A界-burst\n尾🙂"
    );
}

#[tokio::test]
async fn changing_policy_cannot_overtake_locally_buffered_content() {
    let (tx, mut rx) = mpsc::channel(4);
    let mut sender = DeltaSender::new(tx, BackpressurePolicy::CoalesceLocal, 8, 1024);

    assert_eq!(sender.send("a").await, Ok(SendOutcome::Buffered));
    sender
        .set_policy(BackpressurePolicy::Block)
        .await
        .expect("policy switch should flush retained content first");
    assert_eq!(sender.send("b").await, Ok(SendOutcome::Sent));
    assert_eq!(sender.flush().await, Ok(SendOutcome::Sent));
    drop(sender);

    let mut received = String::new();
    while let Some(chunk) = rx.recv().await {
        received.push_str(&chunk);
    }
    assert_eq!(received, "ab");
}

async fn round_trip(
    policy: BackpressurePolicy,
    local_max_bytes: usize,
    chunks: Vec<String>,
) -> String {
    let (tx, rx) = mpsc::channel(1);
    let producer = tokio::spawn(async move {
        let mut sender = DeltaSender::new(tx, policy, local_max_bytes, 1024);
        for chunk in chunks {
            sender.send(&chunk).await.expect("receiver remains open");
        }
        sender
            .flush()
            .await
            .expect("final local buffer should flush");
    });

    let mut receiver =
        CoalescingReceiver::new(rx, CoalesceOptions::new(Duration::from_millis(5), 5, 16));
    let mut received = String::new();
    while let Some(chunk) = receiver.recv().await {
        received.push_str(&chunk);
    }
    producer.await.expect("producer task should not panic");
    received
}

#[tokio::test]
async fn cancelling_threshold_send_does_not_accept_the_new_delta() {
    let (tx, mut rx) = mpsc::channel::<String>(1);
    tx.send("occupied".to_string()).await.unwrap();

    let mut sender = DeltaSender::new(tx, BackpressurePolicy::CoalesceLocal, 4, 1024);
    assert_eq!(sender.send("ab").await, Ok(SendOutcome::Buffered));

    let threshold_delta = String::from("cd");
    assert!(
        tokio::time::timeout(Duration::from_millis(10), sender.send(&threshold_delta))
            .await
            .is_err(),
        "the threshold-crossing delta must wait before it is accepted"
    );
    assert_eq!(sender.stats().pending_bytes, 2);
    assert_eq!(sender.stats().pending_constituents, 1);

    assert_eq!(rx.recv().await.as_deref(), Some("occupied"));
    assert_eq!(
        sender.send(&threshold_delta).await,
        Ok(SendOutcome::Buffered)
    );
    assert_eq!(rx.recv().await.as_deref(), Some("ab"));
    assert_eq!(sender.flush().await, Ok(SendOutcome::Sent));
    drop(sender);

    let mut received = String::from("ab");
    while let Some(chunk) = rx.recv().await {
        received.push_str(&chunk);
    }
    assert_eq!(received, "abcd");
}

#[tokio::test]
async fn cancelling_constituent_preflush_does_not_accept_the_new_delta() {
    let (tx, mut rx) = mpsc::channel::<String>(1);
    tx.send("occupied".to_string()).await.unwrap();
    let mut sender = DeltaSender::new(tx, BackpressurePolicy::CoalesceLocal, 1024, 1);
    assert_eq!(sender.send("a").await, Ok(SendOutcome::Buffered));

    let next = String::from("b");
    assert!(
        tokio::time::timeout(Duration::from_millis(10), sender.send(&next))
            .await
            .is_err()
    );
    assert_eq!(next, "b");
    assert_eq!(sender.stats().pending_constituents, 1);
    assert_eq!(sender.stats().pending_bytes, 1);

    assert_eq!(rx.recv().await.as_deref(), Some("occupied"));
    assert_eq!(sender.send(&next).await, Ok(SendOutcome::Buffered));
    assert_eq!(rx.recv().await.as_deref(), Some("a"));
    sender.flush().await.unwrap();
    assert_eq!(rx.recv().await.as_deref(), Some("b"));
}

#[tokio::test]
async fn empty_sender_flood_counts_attempts_without_retaining_boundaries() {
    let (tx, _rx) = mpsc::channel::<String>(1);
    let mut sender = DeltaSender::new(tx, BackpressurePolicy::CoalesceLocal, 16 * 1024, 1024);
    for _ in 0..4096 {
        assert_eq!(sender.send("").await, Ok(SendOutcome::Buffered));
    }

    let stats = sender.stats();
    assert_eq!(stats.input_attempts, 4096);
    assert_eq!(stats.scan_bytes, 0);
    assert_eq!(stats.pending_bytes, 0);
    assert_eq!(stats.pending_constituents, 0);
    assert_eq!(stats.boundary_metadata_bytes, 0);
    assert_eq!(sender.flush().await, Ok(SendOutcome::Sent));
}

#[tokio::test]
async fn cancelling_oversized_standalone_send_leaves_it_with_the_caller() {
    let (tx, mut rx) = mpsc::channel::<String>(1);
    tx.send("occupied".to_string()).await.unwrap();
    let mut sender = DeltaSender::new(tx, BackpressurePolicy::CoalesceLocal, 4, 1024);

    assert!(
        tokio::time::timeout(Duration::from_millis(10), sender.send("abcd"))
            .await
            .is_err()
    );
    assert_eq!(sender.stats().pending_bytes, 0);
    assert_eq!(sender.stats().pending_constituents, 0);

    assert_eq!(rx.recv().await.as_deref(), Some("occupied"));
    assert_eq!(sender.send("abcd").await, Ok(SendOutcome::Sent));
    assert_eq!(rx.recv().await.as_deref(), Some("abcd"));
}

#[tokio::test]
async fn closed_newline_send_rejects_the_new_delta_and_returns_prior_pending_input() {
    let (tx, rx) = mpsc::channel::<String>(1);
    let mut sender = DeltaSender::new(tx, BackpressurePolicy::CoalesceLocal, 8, 8);
    assert_eq!(sender.send("a").await, Ok(SendOutcome::Buffered));
    drop(rx);

    let rejected = String::from("b\n");
    assert_eq!(
        sender.send(&rejected).await,
        Err(mdstream_tokio::SendError::Closed)
    );
    assert_eq!(rejected, "b\n");
    assert_eq!(sender.stats().pending_bytes, 1);
    assert_eq!(
        sender
            .take_pending()
            .into_chunks()
            .into_iter()
            .collect::<Vec<_>>(),
        vec!["a"]
    );
    assert_eq!(sender.stats().pending_bytes, 0);
    assert_eq!(sender.stats().boundary_metadata_bytes, 0);
}
