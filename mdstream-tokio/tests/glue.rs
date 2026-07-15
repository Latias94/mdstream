use std::time::Duration;

use mdstream::{EngineError, StreamEngine};
use mdstream_protocol::{ChangeSet, DocumentLifecycle, ProjectionOp, Reducer, Sequence};
use mdstream_tokio::{
    ActorCommand, BackpressurePolicy, CoalesceOptions, CoalescingReceiver, DeltaSender,
    FlushReason, SendError, SendOutcome, spawn_stream_engine_actor,
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
    let mut coalesce = DeltaSender::new(tx, BackpressurePolicy::CoalesceLocal);
    assert_eq!(coalesce.send("d").await.unwrap(), SendOutcome::Buffered);

    let (tx, rx) = mpsc::channel::<String>(1);
    drop(rx);
    let mut closed = DeltaSender::new(tx, BackpressurePolicy::Block);
    assert_eq!(closed.send("x").await, Err(SendError::Closed));

    let (tx, rx) = mpsc::channel::<String>(1);
    drop(rx);
    let mut closed_coalesce = DeltaSender::new(tx, BackpressurePolicy::CoalesceLocal);
    closed_coalesce.set_local_max_bytes(1);
    assert_eq!(closed_coalesce.send("x").await, Err(SendError::Closed));
}

#[tokio::test]
async fn stream_engine_actor_closes_with_one_replayable_finalization() {
    let (tx, rx) = mpsc::channel(8);
    let mut output = spawn_stream_engine_actor(StreamEngine::new(), rx, test_options());

    for byte in ["H", "e", "l", "l", "o"] {
        tx.send(ActorCommand::Append(byte.to_string()))
            .await
            .unwrap();
    }
    drop(tx);

    let mut changes = Vec::new();
    while let Some(result) = output.recv().await {
        changes.extend(result.expect("actor command should succeed"));
    }

    assert!(!changes.is_empty());
    for (index, change) in changes.iter().enumerate() {
        assert_eq!(change.sequence(), Sequence::new(index as u64));
        if let Some(previous) = index.checked_sub(1).and_then(|i| changes.get(i)) {
            assert_ne!(change.change_id(), previous.change_id());
        }
    }
    assert_eq!(
        changes
            .iter()
            .flat_map(|change| change.operations())
            .filter(|operation| matches!(operation, ProjectionOp::FinishDocument))
            .count(),
        1
    );

    let mut reducer = Reducer::new();
    for change in changes {
        reducer.apply(change).expect("actor trace should replay");
    }
    let document = reducer.document().expect("actor should start an epoch");
    assert_eq!(document.source(), "Hello");
    assert_eq!(document.lifecycle(), DocumentLifecycle::Finalized);
    output.join().await.expect("actor task should exit cleanly");
}

#[tokio::test]
async fn stream_engine_actor_reports_terminal_errors_and_reset_changes_in_order() {
    let (tx, rx) = mpsc::channel(8);
    let mut output = spawn_stream_engine_actor(StreamEngine::new(), rx, test_options());

    for command in [
        ActorCommand::Append("old".to_string()),
        ActorCommand::Finish,
        ActorCommand::Append("late".to_string()),
        ActorCommand::Reset,
        ActorCommand::Append("new".to_string()),
    ] {
        tx.send(command).await.unwrap();
    }
    drop(tx);

    let mut changes: Vec<ChangeSet> = Vec::new();
    let mut errors = Vec::new();
    while let Some(result) = output.recv().await {
        match result {
            Ok(batch) => changes.extend(batch),
            Err(error) => errors.push(error),
        }
    }

    assert_eq!(errors, vec![EngineError::Finished]);
    let reset = changes
        .iter()
        .find(|change| {
            change
                .epoch_start()
                .is_some_and(|start| start.predecessor.is_some())
        })
        .expect("reset must cross the actor as an epoch-start change");
    assert_eq!(reset.sequence(), Sequence::new(0));

    let mut reducer = Reducer::new();
    for change in changes {
        reducer
            .apply(change)
            .expect("ordered actor trace should replay");
    }
    let document = reducer.document().expect("reset epoch should exist");
    assert_eq!(document.source(), "new");
    assert_eq!(document.lifecycle(), DocumentLifecycle::Finalized);
    output.join().await.expect("actor task should exit cleanly");
}

#[tokio::test]
async fn closing_actor_output_cancels_and_releases_input_without_panic() {
    let (tx, rx) = mpsc::channel(1);
    let mut actor = spawn_stream_engine_actor(StreamEngine::new(), rx, test_options());

    actor.close_output();
    tokio::time::timeout(Duration::from_secs(1), actor.join())
        .await
        .expect("actor task must not leak after output cancellation")
        .expect("actor task must not panic");

    assert!(tx.is_closed());
    assert!(
        tx.send(ActorCommand::Append("ignored".to_string()))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn actor_preserves_normalized_order_for_one_byte_and_bursty_schedules() {
    let source = "a\r\nb\rc\n";
    let one_byte = source
        .bytes()
        .map(|byte| char::from(byte).to_string())
        .collect();
    let bursty = ["a\r", "\nb", "\rc\n"]
        .into_iter()
        .map(str::to_string)
        .collect();

    assert_eq!(actor_source(one_byte).await, "a\nb\nc\n");
    assert_eq!(actor_source(bursty).await, "a\nb\nc\n");
}

async fn actor_source(chunks: Vec<String>) -> String {
    let (tx, rx) = mpsc::channel(1);
    let mut actor = spawn_stream_engine_actor(StreamEngine::new(), rx, test_options());
    let producer = tokio::spawn(async move {
        for chunk in chunks {
            tx.send(ActorCommand::Append(chunk)).await.unwrap();
        }
    });

    let mut reducer = Reducer::new();
    while let Some(result) = actor.recv().await {
        for change in result.expect("actor command should succeed") {
            reducer.apply(change).expect("actor trace should replay");
        }
    }
    producer.await.expect("producer task should not panic");
    actor.join().await.expect("actor task should exit cleanly");
    let document = reducer.document().expect("actor should start an epoch");
    assert_eq!(document.lifecycle(), DocumentLifecycle::Finalized);
    document.source().to_string()
}
