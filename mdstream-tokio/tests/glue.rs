use std::time::Duration;

use mdstream::{EngineError, EngineLimits, SplitSafety, StreamEngine};
use mdstream_protocol::{
    ChangeSet, DocumentLifecycle, ProjectionOp, ProtocolLimits, Reducer, Sequence,
};
use mdstream_tokio::{
    ActorCommand, ActorDrainState, ActorExit, BackpressurePolicy, CoalesceOptions,
    CoalescingReceiver, DeltaSender, FlushReason, SendError, SendOutcome,
    spawn_stream_engine_actor,
};
use tokio::sync::mpsc;

fn test_options() -> CoalesceOptions {
    CoalesceOptions::new(Duration::from_millis(20), 4, 8).with_newline_flush(false)
}

#[tokio::test]
async fn actor_failure_returns_engine_pending_input_and_unexecuted_commands() {
    let engine = StreamEngine::builder()
        .protocol_limits(ProtocolLimits {
            max_source_bytes: 1,
            ..ProtocolLimits::default()
        })
        .build()
        .unwrap();
    let (tx, rx) = mpsc::channel(8);
    let mut actor = spawn_stream_engine_actor(engine, rx, test_options().with_max_bytes(1024));
    tx.send(ActorCommand::Append("too-large".to_string()))
        .await
        .unwrap();
    tx.send(ActorCommand::Finish).await.unwrap();
    tx.send(ActorCommand::Reset).await.unwrap();
    drop(tx);

    let outcome = actor.join().await.expect("actor task");
    assert!(outcome.unread.is_empty());
    let ActorExit::Failed(mut failure) = outcome.exit else {
        panic!("append rejection must terminate the actor");
    };
    assert_eq!(failure.engine.snapshot(), None);
    assert_eq!(
        failure.pending.chunks().collect::<Vec<_>>(),
        vec!["too-large"]
    );
    assert!(failure.completed.is_empty());
    let drained = failure.commands.drain_ready(8);
    assert_eq!(
        drained.commands,
        vec![ActorCommand::Finish, ActorCommand::Reset]
    );
    assert_eq!(drained.state, ActorDrainState::Complete);
}

#[tokio::test]
async fn actor_partial_failure_returns_committed_prefix_and_unresolved_suffix() {
    let engine = StreamEngine::builder()
        .protocol_limits(ProtocolLimits {
            max_source_bytes: 1,
            ..ProtocolLimits::default()
        })
        .build()
        .unwrap();
    let (tx, rx) = mpsc::channel(8);
    let mut actor = spawn_stream_engine_actor(engine, rx, test_options().with_max_bytes(1024));
    tx.send(ActorCommand::Append("a".to_string()))
        .await
        .unwrap();
    tx.send(ActorCommand::Append("b".to_string()))
        .await
        .unwrap();
    tx.send(ActorCommand::Finish).await.unwrap();
    drop(tx);

    let outcome = actor.join().await.expect("actor task");
    assert!(outcome.unread.is_empty());
    let ActorExit::Failed(mut failure) = outcome.exit else {
        panic!("the second cumulative append must terminate the actor");
    };
    assert_eq!(failure.engine.snapshot().unwrap().source(), "a");
    assert_eq!(failure.completed.len(), 1);
    assert_eq!(failure.completed[0].changes()[0].source().suffix, "a");
    assert_eq!(failure.pending.chunks().collect::<Vec<_>>(), vec!["b"]);
    assert_eq!(failure.stats.input_attempts, 2);
    assert_eq!(failure.stats.append_attempts, 2);
    assert_eq!(failure.stats.successful_appends, 1);
    assert_eq!(failure.stats.committed_bytes, 1);
    assert_eq!(failure.stats.pending_constituents, 1);
    assert_eq!(failure.stats.join_copy_bytes, 0);
    assert_eq!(failure.stats.replay_count, 0);
    let drained = failure.commands.drain_ready(8);
    assert_eq!(drained.commands, vec![ActorCommand::Finish]);
    assert_eq!(drained.state, ActorDrainState::Complete);
}

#[tokio::test]
async fn actor_never_reads_past_a_full_constituent_budget() {
    let engine = StreamEngine::builder()
        .protocol_limits(ProtocolLimits {
            max_source_bytes: 1,
            ..ProtocolLimits::default()
        })
        .build()
        .unwrap();
    let (tx, rx) = mpsc::channel(8);
    let mut actor = spawn_stream_engine_actor(
        engine,
        rx,
        CoalesceOptions::new(Duration::from_secs(60), 1024, 2).with_newline_flush(false),
    );
    for chunk in ["xx", "yy", "zz"] {
        tx.send(ActorCommand::Append(chunk.to_string()))
            .await
            .unwrap();
    }
    drop(tx);

    let outcome = actor.join().await.expect("actor task");
    let ActorExit::Failed(mut failure) = outcome.exit else {
        panic!("the first constituent must exceed the source limit");
    };
    assert_eq!(
        failure.pending.chunks().collect::<Vec<_>>(),
        vec!["xx", "yy"]
    );
    assert_eq!(failure.stats.pending_constituents, 2);
    assert!(failure.stats.boundary_metadata_bytes > 0);
    let drained = failure.commands.drain_ready(8);
    assert_eq!(
        drained.commands,
        vec![ActorCommand::Append("zz".to_string())]
    );
    assert_eq!(drained.state, ActorDrainState::Complete);
}

#[tokio::test]
async fn actor_constituent_first_avoids_a_split_safe_join_failure() {
    let chunks = ["a", "b", "c"];
    let mut constituent_probe = StreamEngine::new();
    let mut constituent_limit = 0;
    for chunk in chunks {
        constituent_probe.append(chunk).unwrap();
        constituent_limit =
            constituent_limit.max(constituent_probe.metrics().work.last_change_bytes);
    }
    let mut joined_probe = StreamEngine::new();
    joined_probe.append("abc").unwrap();
    assert!(joined_probe.metrics().work.last_change_bytes > constituent_limit);

    let limits = EngineLimits {
        max_change_bytes: constituent_limit,
        ..EngineLimits::default()
    };
    let mut joined = StreamEngine::builder()
        .engine_limits(limits)
        .build()
        .unwrap();
    let joined_error = joined.append("abc").unwrap_err();
    assert_eq!(
        joined_error.split_safety(),
        SplitSafety::RetryAtOriginalBoundaries
    );

    let engine = StreamEngine::builder()
        .engine_limits(limits)
        .build()
        .unwrap();
    let (tx, rx) = mpsc::channel(8);
    let mut actor = spawn_stream_engine_actor(engine, rx, test_options().with_max_bytes(1024));
    for chunk in chunks {
        tx.send(ActorCommand::Append(chunk.to_string()))
            .await
            .unwrap();
    }
    drop(tx);

    let outcome = actor.join().await.unwrap();
    let ActorExit::Completed(completion) = outcome.exit else {
        panic!("constituent-first input must avoid the joined rejection");
    };
    assert_eq!(completion.engine.snapshot().unwrap().source(), "abc");
    assert_eq!(completion.stats.append_attempts, 3);
    assert_eq!(completion.stats.successful_appends, 3);
    assert_eq!(completion.stats.replay_count, 0);
    assert_eq!(completion.stats.join_copy_bytes, 0);
}

#[tokio::test]
async fn single_split_safe_failure_is_not_retried_or_subdivided() {
    let mut probe = StreamEngine::new();
    probe.append("a").unwrap();
    let limits = EngineLimits {
        max_change_bytes: probe.metrics().work.last_change_bytes - 1,
        ..EngineLimits::default()
    };
    let engine = StreamEngine::builder()
        .engine_limits(limits)
        .build()
        .unwrap();
    let (tx, rx) = mpsc::channel(1);
    let mut actor = spawn_stream_engine_actor(engine, rx, test_options());
    tx.send(ActorCommand::Append("a".to_string()))
        .await
        .unwrap();
    drop(tx);

    let outcome = actor.join().await.unwrap();
    let ActorExit::Failed(failure) = outcome.exit else {
        panic!("the configured single transition must fail");
    };
    assert_eq!(
        failure.error.split_safety(),
        SplitSafety::RetryAtOriginalBoundaries
    );
    assert_eq!(failure.pending.chunks().collect::<Vec<_>>(), vec!["a"]);
    assert!(failure.completed.is_empty());
    assert_eq!(failure.stats.append_attempts, 1);
    assert_eq!(failure.stats.successful_appends, 0);
    assert_eq!(failure.stats.replay_count, 0);
}

#[tokio::test]
async fn actor_failure_drain_observes_reserved_permits_exactly_once() {
    let engine = StreamEngine::builder()
        .protocol_limits(ProtocolLimits {
            max_source_bytes: 1,
            ..ProtocolLimits::default()
        })
        .build()
        .unwrap();
    let (tx, rx) = mpsc::channel(4);
    let mut actor = spawn_stream_engine_actor(engine, rx, test_options().with_max_bytes(1024));

    let failing = tx.reserve().await.unwrap();
    let borrowed = tx.reserve().await.unwrap();
    let owned = tx.clone().reserve_owned().await.unwrap();
    failing.send(ActorCommand::Append("too-large".to_string()));

    let outcome = actor.join().await.expect("actor task");
    let ActorExit::Failed(mut failure) = outcome.exit else {
        panic!("the append must terminate the actor");
    };
    let before = failure.commands.drain_ready(8);
    assert!(before.commands.is_empty());
    assert_eq!(before.state, ActorDrainState::PendingPermits);

    borrowed.send(ActorCommand::Finish);
    drop(owned.send(ActorCommand::Reset));
    drop(tx);

    let after = failure.commands.drain_ready(2);
    assert_eq!(
        after.commands,
        vec![ActorCommand::Finish, ActorCommand::Reset]
    );
    assert_eq!(after.state, ActorDrainState::Complete);
    assert!(failure.commands.recv().await.is_none());
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
    let mut receiver = CoalescingReceiver::new(rx, test_options().with_max_bytes(1024));

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
        CoalesceOptions::new(Duration::from_secs(60), 1024, 8).with_newline_flush(false),
    );

    tx.send("tail".to_string()).await.unwrap();
    drop(tx);

    let chunk = receiver.recv_with_meta().await.expect("final chunk");
    assert_eq!(chunk.text, "tail");
    assert_eq!(chunk.reason, FlushReason::ChannelClosed);
}

#[tokio::test]
async fn cancelled_receiver_wait_preserves_bytes_metadata_and_scan_work() {
    let (tx, rx) = mpsc::channel::<String>(8);
    let mut receiver =
        CoalescingReceiver::new(rx, CoalesceOptions::new(Duration::from_secs(60), 1024, 8));

    tx.send("a".to_string()).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(10), receiver.recv_with_meta())
            .await
            .is_err(),
        "the first receive should be cancelled while the coalescer is waiting"
    );
    tx.send("b\n".to_string()).await.unwrap();

    let chunk = receiver.recv_with_meta().await.expect("retained chunk");
    assert_eq!(chunk.text, "ab\n");
    assert_eq!(chunk.merged_messages, 2);
    assert_eq!(receiver.stats().input_attempts, 2);
    assert_eq!(receiver.stats().scan_bytes, 3);
}

#[tokio::test(start_paused = true)]
async fn receiver_option_changes_reuse_cached_facts_and_the_original_deadline() {
    let (tx, rx) = mpsc::channel::<String>(8);
    let mut receiver = CoalescingReceiver::new(
        rx,
        CoalesceOptions::new(Duration::from_millis(100), 1024, 8).with_newline_flush(false),
    );
    tx.send("pending".to_string()).await.unwrap();

    assert!(
        tokio::time::timeout(Duration::from_millis(10), receiver.recv_with_meta())
            .await
            .is_err()
    );
    let scan_bytes = receiver.stats().scan_bytes;
    receiver.set_options(receiver.options().with_max_delay(Duration::from_millis(50)));

    let chunk = tokio::time::timeout(Duration::from_millis(41), receiver.recv_with_meta())
        .await
        .expect("the shortened delay is measured from the original first input")
        .expect("retained chunk");
    assert_eq!(chunk.reason, FlushReason::MaxDelay);
    assert_eq!(chunk.text, "pending");
    assert_eq!(receiver.stats().scan_bytes, scan_bytes);
}

#[tokio::test(start_paused = true)]
async fn enabling_newline_flush_uses_the_cached_newline_fact() {
    let (tx, rx) = mpsc::channel::<String>(8);
    let mut receiver = CoalescingReceiver::new(
        rx,
        CoalesceOptions::new(Duration::from_secs(60), 1024, 8).with_newline_flush(false),
    );
    tx.send("ready\n".to_string()).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(1), receiver.recv_with_meta())
            .await
            .is_err()
    );
    assert_eq!(receiver.stats().scan_bytes, 6);

    receiver.set_options(receiver.options().with_newline_flush(true));
    let chunk = receiver
        .recv_with_meta()
        .await
        .expect("cached newline flush");
    assert_eq!(chunk.reason, FlushReason::Newline);
    assert_eq!(chunk.text, "ready\n");
    assert_eq!(receiver.stats().scan_bytes, 6);
}

#[tokio::test(start_paused = true)]
async fn lowering_constituent_budget_flushes_cached_pending_input() {
    let (tx, rx) = mpsc::channel::<String>(8);
    let mut receiver = CoalescingReceiver::new(
        rx,
        CoalesceOptions::new(Duration::from_secs(60), 1024, 4).with_newline_flush(false),
    );
    for chunk in ["a", "b", "c"] {
        tx.send(chunk.to_string()).await.unwrap();
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(1), receiver.recv_with_meta())
            .await
            .is_err()
    );
    assert_eq!(receiver.stats().pending_constituents, 3);
    let scan_bytes = receiver.stats().scan_bytes;

    receiver.set_options(receiver.options().with_max_pending_chunks(2));
    let chunk = receiver.recv_with_meta().await.unwrap();
    assert_eq!(chunk.text, "abc");
    assert_eq!(chunk.reason, FlushReason::MaxPendingChunks);
    assert_eq!(receiver.stats().scan_bytes, scan_bytes);
}

#[tokio::test]
async fn unrepresentable_deadline_waits_for_input_close_without_panicking() {
    let (tx, rx) = mpsc::channel::<String>(8);
    let mut receiver = CoalescingReceiver::new(
        rx,
        CoalesceOptions::new(Duration::MAX, 1024, 8).with_newline_flush(false),
    );
    tx.send("tail".to_string()).await.unwrap();
    drop(tx);

    let chunk = receiver.recv_with_meta().await.unwrap();
    assert_eq!(chunk.text, "tail");
    assert_eq!(chunk.reason, FlushReason::ChannelClosed);
}

#[tokio::test]
async fn receiver_defers_an_oversized_later_chunk_without_mixing_batches() {
    let (tx, rx) = mpsc::channel::<String>(8);
    let mut receiver = CoalescingReceiver::new(
        rx,
        CoalesceOptions::new(Duration::from_secs(60), 4, 8).with_newline_flush(false),
    );
    let first_input = "ab".to_string();
    let first_pointer = first_input.as_ptr();
    let second_input = "cdefg".to_string();
    let second_pointer = second_input.as_ptr();
    tx.send(first_input).await.unwrap();
    tx.send(second_input).await.unwrap();
    drop(tx);

    let first = receiver.recv_with_meta().await.unwrap();
    assert_eq!(first.text, "ab");
    assert_eq!(first.text.as_ptr(), first_pointer);
    assert_eq!(first.reason, FlushReason::MaxBytes);
    assert_eq!(receiver.stats().pending_bytes, 5);
    assert_eq!(receiver.stats().pending_constituents, 1);

    let second = receiver.recv_with_meta().await.unwrap();
    assert_eq!(second.text, "cdefg");
    assert_eq!(second.text.as_ptr(), second_pointer);
    assert_eq!(second.reason, FlushReason::MaxBytes);
    assert!(receiver.recv().await.is_none());
}

#[tokio::test]
async fn receiver_bounds_constituents_and_does_not_retain_empty_boundaries() {
    let (tx, rx) = mpsc::channel::<String>(8);
    let mut receiver = CoalescingReceiver::new(
        rx,
        CoalesceOptions::new(Duration::from_secs(60), 1024, 2).with_newline_flush(false),
    );

    tx.send(String::new()).await.unwrap();
    tx.send(String::new()).await.unwrap();
    tx.send("a".to_string()).await.unwrap();
    tx.send("b".to_string()).await.unwrap();
    tx.send("c".to_string()).await.unwrap();

    let first = receiver.recv_with_meta().await.expect("bounded batch");
    assert_eq!(first.text, "ab");
    assert_eq!(first.reason, FlushReason::MaxPendingChunks);
    assert_eq!(first.merged_messages, 4);
    let stats = receiver.stats();
    assert_eq!(stats.input_attempts, 4);
    assert_eq!(stats.pending_constituents, 0);
    assert_eq!(stats.pending_bytes, 0);
    assert_eq!(stats.boundary_metadata_bytes, 0);

    drop(tx);
    let second = receiver.recv_with_meta().await.expect("remaining chunk");
    assert_eq!(second.text, "c");
    assert_eq!(second.reason, FlushReason::ChannelClosed);
    assert_eq!(receiver.stats().input_attempts, 5);
}

#[tokio::test]
async fn receiver_one_byte_growth_respects_the_boundary_budget_at_every_scale() {
    for size in [64, 128, 256, 512, 1024] {
        let (tx, rx) = mpsc::channel::<String>(32);
        let producer = tokio::spawn(async move {
            for _ in 0..size {
                tx.send("x".to_string()).await.unwrap();
            }
        });
        let mut receiver = CoalescingReceiver::new(
            rx,
            CoalesceOptions::new(Duration::from_secs(60), usize::MAX, 16).with_newline_flush(false),
        );
        let mut output_bytes = 0;
        while let Some(chunk) = receiver.recv_with_meta().await {
            assert!(chunk.merged_messages <= 16);
            output_bytes += chunk.text.len();
        }
        producer.await.unwrap();

        let stats = receiver.stats();
        assert_eq!(output_bytes, size);
        assert_eq!(stats.input_attempts, u64::try_from(size).unwrap());
        assert_eq!(stats.scan_bytes, u64::try_from(size).unwrap());
        assert_eq!(stats.pending_constituents, 0);
        assert_eq!(stats.boundary_metadata_bytes, 0);
    }
}

#[tokio::test(start_paused = true)]
async fn overflow_lookahead_keeps_its_original_deadline() {
    let (tx, rx) = mpsc::channel::<String>(8);
    let mut receiver = CoalescingReceiver::new(
        rx,
        CoalesceOptions::new(Duration::from_millis(100), 3, 8).with_newline_flush(false),
    );

    tx.send("ab".to_string()).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(1), receiver.recv_with_meta())
            .await
            .is_err()
    );
    tokio::time::advance(Duration::from_millis(39)).await;
    tx.send("bc".to_string()).await.unwrap();

    let first = receiver.recv_with_meta().await.expect("overflow flush");
    assert_eq!(first.text, "ab");
    assert_eq!(first.reason, FlushReason::MaxBytes);

    tokio::time::advance(Duration::from_millis(70)).await;
    let second = tokio::time::timeout(Duration::from_millis(31), receiver.recv_with_meta())
        .await
        .expect("lookahead deadline is measured from its channel arrival")
        .expect("lookahead chunk");
    assert_eq!(second.text, "bc");
    assert_eq!(second.reason, FlushReason::MaxDelay);
}

#[tokio::test]
async fn sender_policies_report_expected_outcomes_and_closed_errors() {
    let (tx, mut rx) = mpsc::channel::<String>(1);
    let mut block = DeltaSender::new(tx, BackpressurePolicy::Block, 16 * 1024, 1024);
    assert_eq!(block.send("a").await.unwrap(), SendOutcome::Sent);
    assert_eq!(rx.recv().await.as_deref(), Some("a"));

    let (tx, _rx) = mpsc::channel::<String>(1);
    let mut coalesce = DeltaSender::new(tx, BackpressurePolicy::CoalesceLocal, 16 * 1024, 1024);
    assert_eq!(coalesce.send("d").await.unwrap(), SendOutcome::Buffered);

    let (tx, rx) = mpsc::channel::<String>(1);
    drop(rx);
    let mut closed = DeltaSender::new(tx, BackpressurePolicy::Block, 16 * 1024, 1024);
    assert_eq!(closed.send("x").await, Err(SendError::Closed));

    let (tx, rx) = mpsc::channel::<String>(1);
    drop(rx);
    let mut closed_coalesce = DeltaSender::new(tx, BackpressurePolicy::CoalesceLocal, 1, 1024);
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

    let mut changes: Vec<ChangeSet> = Vec::new();
    while let Some(batch) = output.recv().await {
        changes.extend(batch.changes().cloned());
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
    let outcome = output.join().await.expect("actor task should exit cleanly");
    assert!(outcome.unread.is_empty());
    assert!(matches!(outcome.exit, ActorExit::Completed(_)));
}

#[tokio::test]
async fn stream_engine_actor_stops_at_terminal_error_and_returns_later_commands() {
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
    while let Some(batch) = output.recv().await {
        changes.extend(batch.changes().cloned());
    }

    let mut reducer = Reducer::new();
    for change in changes.clone() {
        reducer
            .apply(change)
            .expect("ordered actor trace should replay");
    }
    let document = reducer.document().expect("original epoch should exist");
    assert_eq!(document.source(), "old");
    assert_eq!(document.lifecycle(), DocumentLifecycle::Finalized);
    assert!(changes.iter().all(|change| {
        change
            .epoch_start()
            .is_none_or(|start| start.predecessor.is_none())
    }));

    let outcome = output.join().await.expect("actor task should exit cleanly");
    assert!(outcome.unread.is_empty());
    let ActorExit::Failed(mut failure) = outcome.exit else {
        panic!("late append must terminate the actor");
    };
    assert_eq!(failure.error, EngineError::Finished);
    assert_eq!(failure.pending.chunks().collect::<Vec<_>>(), vec!["late"]);
    let drained = failure.commands.drain_ready(8);
    assert_eq!(
        drained.commands,
        vec![ActorCommand::Reset, ActorCommand::Append("new".to_string())]
    );
    assert_eq!(drained.state, ActorDrainState::Complete);
}

#[tokio::test]
async fn closing_actor_output_cancels_and_releases_input_without_panic() {
    let (tx, rx) = mpsc::channel(1);
    let mut actor = spawn_stream_engine_actor(StreamEngine::new(), rx, test_options());

    let outcome = tokio::time::timeout(Duration::from_secs(1), actor.cancel())
        .await
        .expect("actor task must not leak after output cancellation")
        .expect("actor task must not panic");
    assert!(outcome.unread.is_empty());
    assert!(matches!(outcome.exit, ActorExit::Cancelled(_)));

    assert!(tx.is_closed());
    assert!(
        tx.send(ActorCommand::Append("ignored".to_string()))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn explicit_cancellation_wins_when_input_close_is_already_ready() {
    let (tx, rx) = mpsc::channel(1);
    let mut actor = spawn_stream_engine_actor(StreamEngine::new(), rx, test_options());
    drop(tx);
    actor.begin_cancel();

    let outcome = actor.join().await.expect("cancelled actor task");
    assert!(outcome.unread.is_empty());
    let ActorExit::Cancelled(cancellation) = outcome.exit else {
        panic!("explicit output closure must win the ready-state race");
    };
    assert_eq!(cancellation.engine.lifecycle(), DocumentLifecycle::Open);
    assert!(cancellation.unpublished.is_none());
    assert!(cancellation.pending.is_empty());
}

#[tokio::test]
async fn output_cancellation_returns_the_committed_batch_that_could_not_publish() {
    const COMMAND_COUNT: usize = 65;

    let (tx, rx) = mpsc::channel(COMMAND_COUNT);
    let mut actor = spawn_stream_engine_actor(StreamEngine::new(), rx, test_options());
    for _ in 0..COMMAND_COUNT {
        tx.send(ActorCommand::Reset).await.unwrap();
    }

    tokio::time::timeout(Duration::from_secs(1), async {
        while tx.capacity() != COMMAND_COUNT {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("actor must receive every queued reset");
    drop(tx);

    let outcome = actor.cancel().await.expect("actor cancellation");
    assert_eq!(outcome.unread.len(), COMMAND_COUNT - 1);
    let ActorExit::Cancelled(cancellation) = outcome.exit else {
        panic!("closing output must produce a cancellation exit");
    };
    assert_eq!(
        cancellation
            .unpublished
            .as_ref()
            .expect("one committed batch could not publish")
            .transitions()
            .len(),
        1
    );
    assert_eq!(
        cancellation.stats.published_results,
        (COMMAND_COUNT - 1) as u64
    );
}

#[tokio::test(start_paused = true)]
async fn cancelled_join_wait_can_be_retried_without_losing_drained_output() {
    let (tx, rx) = mpsc::channel(1);
    let mut actor = spawn_stream_engine_actor(StreamEngine::new(), rx, test_options());
    tx.send(ActorCommand::Reset).await.unwrap();

    assert!(
        tokio::time::timeout(Duration::from_millis(1), actor.join())
            .await
            .is_err(),
        "an open input keeps join pending"
    );
    let retained = actor
        .recv()
        .await
        .expect("output drained by the cancelled join remains readable");
    assert_eq!(retained.transitions().len(), 1);
    drop(tx);

    let outcome = actor.join().await.expect("retry must recover ownership");
    assert_eq!(outcome.unread.len(), 1);
    assert!(matches!(outcome.exit, ActorExit::Completed(_)));
}

#[tokio::test]
async fn actor_join_drains_more_than_output_capacity_without_deadlock() {
    // One more command than the actor's 64-slot output channel.
    const COMMAND_COUNT: usize = 65;

    let (tx, rx) = mpsc::channel(COMMAND_COUNT);
    let mut actor = spawn_stream_engine_actor(StreamEngine::new(), rx, test_options());
    for _ in 0..COMMAND_COUNT {
        tx.send(ActorCommand::Reset).await.unwrap();
    }
    drop(tx);

    let outcome = tokio::time::timeout(Duration::from_secs(1), actor.join())
        .await
        .expect("join must consume unread actor output while waiting")
        .expect("actor task should exit cleanly");

    assert_eq!(outcome.unread.len(), COMMAND_COUNT + 1);
    assert!(matches!(outcome.exit, ActorExit::Completed(_)));
    let changes: Vec<_> = outcome
        .unread
        .into_iter()
        .flat_map(|batch| batch.into_transitions())
        .flat_map(|result| result.into_changes())
        .collect();
    assert_eq!(changes.len(), COMMAND_COUNT + 1);
    assert_eq!(
        changes
            .iter()
            .filter(|change| change.epoch_start().is_some())
            .count(),
        COMMAND_COUNT
    );
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
        reducer
            .apply(change)
            .expect("joined actor trace should replay");
    }
    let document = reducer.document().expect("actor should start an epoch");
    assert_eq!(document.lifecycle(), DocumentLifecycle::Finalized);

    let repeated_join = actor.join().await.expect_err("outcome is one-shot");
    assert!(repeated_join.unread().is_empty());
    assert!(repeated_join.join_error().is_none());
    let repeated_cancel = actor.cancel().await.expect_err("outcome remains one-shot");
    assert!(repeated_cancel.unread().is_empty());
    assert!(repeated_cancel.join_error().is_none());
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

#[tokio::test]
async fn actor_one_byte_work_is_linear_and_empty_floods_retain_no_boundaries() {
    let source = "x".repeat(512);
    let (tx, rx) = mpsc::channel(32);
    let mut actor = spawn_stream_engine_actor(
        StreamEngine::new(),
        rx,
        CoalesceOptions::new(Duration::from_secs(60), 1024, 16).with_newline_flush(false),
    );
    let producer = tokio::spawn({
        let source = source.clone();
        async move {
            for byte in source.bytes() {
                tx.send(ActorCommand::Append(char::from(byte).to_string()))
                    .await
                    .unwrap();
            }
        }
    });
    producer.await.unwrap();

    let outcome = actor.join().await.unwrap();
    let ActorExit::Completed(completion) = outcome.exit else {
        panic!("one-byte actor must complete");
    };
    assert_eq!(completion.engine.snapshot().unwrap().source(), source);
    assert_eq!(completion.stats.input_attempts, 512);
    assert_eq!(completion.stats.scan_bytes, 512);
    assert_eq!(completion.stats.append_attempts, 512);
    assert_eq!(completion.stats.successful_appends, 512);
    assert_eq!(completion.stats.join_copy_bytes, 0);
    assert_eq!(completion.stats.replay_count, 0);
    assert_eq!(completion.stats.pending_constituents, 0);

    let (tx, rx) = mpsc::channel(32);
    let mut actor = spawn_stream_engine_actor(StreamEngine::new(), rx, test_options());
    for _ in 0..128 {
        tx.send(ActorCommand::Append(String::new())).await.unwrap();
    }
    drop(tx);
    let outcome = actor.join().await.unwrap();
    let ActorExit::Completed(completion) = outcome.exit else {
        panic!("empty input actor must complete");
    };
    assert_eq!(completion.stats.input_attempts, 128);
    assert_eq!(completion.stats.scan_bytes, 0);
    assert_eq!(completion.stats.append_attempts, 0);
    assert_eq!(completion.stats.pending_bytes, 0);
    assert_eq!(completion.stats.pending_constituents, 0);
    assert_eq!(completion.stats.boundary_metadata_bytes, 0);
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
    while let Some(batch) = actor.recv().await {
        for change in batch.changes().cloned() {
            reducer.apply(change).expect("actor trace should replay");
        }
    }
    producer.await.expect("producer task should not panic");
    let outcome = actor.join().await.expect("actor task should exit cleanly");
    assert!(outcome.unread.is_empty());
    assert!(matches!(outcome.exit, ActorExit::Completed(_)));
    let document = reducer.document().expect("actor should start an epoch");
    assert_eq!(document.lifecycle(), DocumentLifecycle::Finalized);
    document.source().to_string()
}
