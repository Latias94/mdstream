use std::time::Duration;

use mdstream::{MdStream, Options};
use mdstream_tokio::{
    BackpressurePolicy, CoalesceOptions, CoalescingReceiver, DeltaSender, FlushReason, SendError,
    SendOutcome, spawn_mdstream_actor,
};
use tokio::sync::mpsc;

fn test_options() -> CoalesceOptions {
    CoalesceOptions {
        flush_on_newline: false,
        max_delay: Duration::from_millis(20),
        max_bytes: 4,
    }
}

#[tokio::test]
async fn receiver_flushes_on_max_bytes() {
    let (tx, rx) = mpsc::channel::<String>(8);
    let mut receiver = CoalescingReceiver::new(rx, test_options());

    tx.send("ab".to_string()).await.unwrap();
    tx.send("cd".to_string()).await.unwrap();

    let chunk = receiver.recv_with_meta().await.expect("chunk");
    assert_eq!(chunk.text, "abcd");
    assert_eq!(chunk.reason, FlushReason::MaxBytes);
    assert_eq!(chunk.merged_messages, 2);
}

#[tokio::test]
async fn receiver_flushes_on_max_delay() {
    let (tx, rx) = mpsc::channel::<String>(8);
    let mut receiver = CoalescingReceiver::new(
        rx,
        CoalesceOptions {
            max_bytes: 1024,
            ..test_options()
        },
    );

    tx.send("pending".to_string()).await.unwrap();

    let chunk = receiver.recv_with_meta().await.expect("chunk");
    assert_eq!(chunk.text, "pending");
    assert_eq!(chunk.reason, FlushReason::MaxDelay);
    assert_eq!(chunk.merged_messages, 1);
}

#[tokio::test]
async fn receiver_flushes_buffer_when_channel_closes() {
    let (tx, rx) = mpsc::channel::<String>(8);
    let mut receiver = CoalescingReceiver::new(
        rx,
        CoalesceOptions {
            max_bytes: 1024,
            max_delay: Duration::from_secs(60),
            ..test_options()
        },
    );

    tx.send("tail".to_string()).await.unwrap();
    drop(tx);

    let chunk = receiver.recv_with_meta().await.expect("final chunk");
    assert_eq!(chunk.text, "tail");
    assert_eq!(chunk.reason, FlushReason::ChannelClosed);
}

#[tokio::test]
async fn sender_policies_report_expected_outcomes_and_closed_errors() {
    let (tx, mut rx) = mpsc::channel::<String>(1);
    let mut block = DeltaSender::new(tx, BackpressurePolicy::Block);
    assert_eq!(block.send("a").await.unwrap(), SendOutcome::Sent);
    assert_eq!(rx.recv().await.as_deref(), Some("a"));

    let (tx, _rx) = mpsc::channel::<String>(1);
    let mut drop_new = DeltaSender::new(tx, BackpressurePolicy::DropNew);
    assert_eq!(drop_new.send("b").await.unwrap(), SendOutcome::Sent);
    assert_eq!(drop_new.send("c").await.unwrap(), SendOutcome::Dropped);

    let (tx, _rx) = mpsc::channel::<String>(1);
    let mut coalesce = DeltaSender::new(tx, BackpressurePolicy::CoalesceLocal);
    assert_eq!(coalesce.send("d").await.unwrap(), SendOutcome::Buffered);

    let (tx, rx) = mpsc::channel::<String>(1);
    drop(rx);
    let mut closed = DeltaSender::new(tx, BackpressurePolicy::DropNew);
    assert_eq!(closed.send("x").await, Err(SendError::Closed));

    let (tx, rx) = mpsc::channel::<String>(1);
    drop(rx);
    let mut closed_coalesce = DeltaSender::new(tx, BackpressurePolicy::CoalesceLocal);
    closed_coalesce.set_local_max_bytes(1);
    assert_eq!(closed_coalesce.send("x").await, Err(SendError::Closed));
}

#[tokio::test]
async fn actor_emits_final_update_when_input_closes() {
    let (tx, rx) = mpsc::channel::<String>(8);
    let mut updates = spawn_mdstream_actor(MdStream::new(Options::default()), rx, test_options());

    tx.send("Hello".to_string()).await.unwrap();
    drop(tx);

    let first = updates.recv().await.expect("append update");
    assert!(first.pending.is_some());

    let final_update = updates.recv().await.expect("final update");
    assert_eq!(final_update.committed.len(), 1);
    assert_eq!(final_update.committed[0].raw, "Hello");
    assert!(final_update.pending.is_none());
}

#[tokio::test]
async fn actor_exits_when_output_receiver_closes() {
    let (tx, rx) = mpsc::channel::<String>(1);
    let updates = spawn_mdstream_actor(MdStream::new(Options::default()), rx, test_options());
    drop(updates);

    tx.send("Hello".to_string()).await.unwrap();

    let mut closed = false;
    for _ in 0..20 {
        match tx.send("after".to_string()).await {
            Ok(()) => tokio::time::sleep(Duration::from_millis(10)).await,
            Err(_) => {
                closed = true;
                break;
            }
        }
    }
    assert!(closed, "actor should drop input receiver after output closes");
}
