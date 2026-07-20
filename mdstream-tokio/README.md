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

`spawn_stream_engine_actor` owns one `StreamEngine` and accepts ordered `ActorCommand::Append`, `Reset`, and `Finish` values. Each successful `ActorResult` contains every change produced by one engine transition, so a receiver never observes half of an atomic batch.

Adjacent append commands may be coalesced without losing bytes. Reset and finish are ordering barriers: buffered content is applied before the barrier. Closing the input finalizes an open document exactly once. Consume results with `StreamEngineActor::recv`; after input closes, `join` drains and returns unread output before the task exits. `close_output` is explicit cancellation and may discard pending input and output.

## Backpressure

`DeltaSender` supports only lossless canonical-input policies:

- `BackpressurePolicy::Block` waits for channel capacity.
- `BackpressurePolicy::CoalesceLocal` retains bursts in a bounded local buffer and flushes them in order.

`SendOutcome::Buffered` means accepted but not yet admitted to the channel. Call `flush().await` before dropping that sender. `set_policy` is asynchronous and fallible because it flushes prior buffered content before changing policy. There is no `DropNew` policy for canonical Markdown; replaceable progress or status signals belong on a separate lossy channel.

## Migrating from 0.3

Replace `spawn_mdstream_actor` and owned `Update` values with `spawn_stream_engine_actor`, `ActorCommand`, and fallible `ActorResult` change batches. Replace `BackpressurePolicy::DropNew` with `Block` or `CoalesceLocal`, handle `SendError`, await `set_policy`, and flush buffered content before sender disposal.
