# mdstream fuzzing

The fuzz package is intentionally outside the default workspace. Normal CI runs
deterministic tests, examples, benchmark compilation, and packaging; fuzzing is a
local hardening workflow because `cargo-fuzz` normally uses nightly Rust and
long-running sanitizer builds.

## Setup

```powershell
cargo install cargo-fuzz
rustup toolchain install nightly
```

## Targets

Build all targets:

```powershell
cargo +nightly fuzz build
```

Run the stream chunking target:

```powershell
cargo +nightly fuzz run stream_chunking
```

Run the pending terminator target:

```powershell
cargo +nightly fuzz run terminator
```

`stream_chunking` compares final `(BlockKind, raw)` output for whole input,
generated chunk boundaries, and the borrowed `append_ref` path. `terminator`
stresses `terminate_markdown` option combinations and a conservative output-size
invariant.
