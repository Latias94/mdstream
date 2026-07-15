use std::time::Duration;

use mdstream_tokio::{
    BackpressurePolicy, CoalesceOptions, CoalescingReceiver, DeltaSender, SendOutcome,
};
use tokio::sync::mpsc;

#[tokio::test]
async fn lossless_coalescing_applies_backpressure_at_local_limit() {
    let (tx, mut rx) = mpsc::channel::<String>(1);
    tx.send("occupied".to_string()).await.unwrap();

    let mut sender = DeltaSender::new(tx, BackpressurePolicy::CoalesceLocal);
    sender.set_local_max_bytes(4);
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

    let mut sender = DeltaSender::new(tx, BackpressurePolicy::CoalesceLocal);
    sender.set_local_max_bytes(4);
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
    let mut sender = DeltaSender::new(tx, BackpressurePolicy::CoalesceLocal);
    sender.set_local_max_bytes(8);

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
        let mut sender = DeltaSender::new(tx, policy);
        sender.set_local_max_bytes(local_max_bytes);
        for chunk in chunks {
            sender.send(&chunk).await.expect("receiver remains open");
        }
        sender
            .flush()
            .await
            .expect("final local buffer should flush");
    });

    let mut receiver = CoalescingReceiver::new(
        rx,
        CoalesceOptions {
            flush_on_newline: true,
            max_delay: Duration::from_millis(5),
            max_bytes: 5,
        },
    );
    let mut received = String::new();
    while let Some(chunk) = receiver.recv().await {
        received.push_str(&chunk);
    }
    producer.await.expect("producer task should not panic");
    received
}
