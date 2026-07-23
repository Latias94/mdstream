# mdstream-tokio

`mdstream-tokio` adds bounded asynchronous feeding and actor ownership around the synchronous, runtime-independent `mdstream::StreamEngine`. It requires Rust 1.88 or newer and emits the same ordered `mdstream_protocol::ChangeSet` batches as direct engine use.

## Actor TUI example

This is an advanced runtime-host entry in the repository's [example learning path](https://github.com/Latias94/mdstream/blob/main/docs/EXAMPLES.md#tokio-actor), not the universal quickstart.

Run the deterministic actor path without terminal control:

```sh
cargo +1.88.0 run -p mdstream-tokio --example agent_tui -- --smoke
```

The command ends with `SMOKE_OK`, a finalized lifecycle, zero errors, and bounded-channel command, batch, and change counters. Omit `--smoke` to open the scrollable Ratatui host:

```sh
cargo +1.88.0 run -p mdstream-tokio --example agent_tui
```

Both modes use `spawn_stream_engine_actor`. The example adds actor transport, lossless coalescing, bounded backpressure, follow-tail, scrolling, and terminal lifecycle. Ratatui composition is example-owned host policy, not a renderer exported by mdstream. Continue with the [replica recovery recipe](https://github.com/Latias94/mdstream/blob/main/docs/EXAMPLES.md#replica-recovery) before transporting changes across a fallible boundary.

## Actor contract

`spawn_stream_engine_actor` owns one `StreamEngine` and accepts ordered `ActorCommand::Append`, `Reset`, and `Finish` values. `recv` yields success-only `ActorBatch` values. A batch retains its ordered constituent `EngineOutput` transitions and is published with one channel send, so a receiver never observes only part of one coalescer flush.

Adjacent append commands share a bounded scheduling window, but canonical appends execute over their original non-empty boundaries. Reset and finish are ordering barriers: pending input must commit before a barrier executes. Closing the input finalizes an open document exactly once.

Await `join` after closing input. It returns `ActorJoinOutcome { unread, exit }`, including the owned engine and deterministic work counters. Both `join` and `cancel` borrow the actor, retain already-drained output internally, and can be cancelled and retried. `begin_cancel` synchronously closes output when a caller needs to wrap only the wait in a timeout. An engine rejection terminates intake as `ActorExit::Failed` and returns completed prefix transitions, unresolved chunks, unexecuted commands, and a closed `ActorCommandDrain`. Permits reserved before receiver closure may still enqueue, so `drain_ready` distinguishes `PendingPermits` from `Complete`. Intentional cancellation returns the same ownership planes plus the single committed batch, if any, that could not be published.

## Backpressure

`DeltaSender` supports only lossless canonical-input policies:

- `BackpressurePolicy::Block` waits for channel capacity.
- `BackpressurePolicy::CoalesceLocal` retains bursts in a bounded local buffer and flushes them in order.

`DeltaSender::new` requires explicit local byte and constituent limits. `SendOutcome::Buffered` means accepted but not yet admitted to the channel. Call `flush().await` or recover original constituents with `take_pending()` before dropping that sender. `set_policy` is asynchronous and fallible because it flushes prior buffered content before changing policy. Until `send` returns successfully, its borrowed delta remains caller-owned; a closed-channel error does not accept it. There is no `DropNew` policy for canonical Markdown; replaceable progress or status signals belong on a separate lossy channel.

Use `CoalesceOptions::new(max_delay, max_bytes, max_pending_chunks)` for receiver and actor policies. Empty chunks count as input attempts but consume no boundary metadata. `CoalesceStats` and `ActorStats` expose scan, copy, pending, append, commit, replay, and publication work without timing-based measurements.

## Migrating from 0.3

Replace `spawn_mdstream_actor` and owned `Update` values with `spawn_stream_engine_actor`, `ActorCommand`, success-only `ActorBatch` output, and terminal `ActorExit` ownership. Replace `close_output` with `begin_cancel` or borrowed `cancel`, and handle `Failed` or `Cancelled` pending input explicitly. Replace public `CoalesceOptions` literals and `CoalescePreset` with the constructor and modifiers. Pass explicit local byte and constituent limits to `DeltaSender::new`. Replace `BackpressurePolicy::DropNew` with `Block` or `CoalesceLocal`, handle `SendError`, await `set_policy`, and flush or take buffered content before sender disposal.
