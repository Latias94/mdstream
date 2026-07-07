# Release Checklist

This checklist is optimized for `mdstream` releases where the `docs/` folder may be pruned (except internal ADP/ADR notes).

## Before tagging

- [ ] Update crate versions:
  - [ ] `mdstream/Cargo.toml` `package.version`
  - [ ] `mdstream-tokio/Cargo.toml` `package.version` if releasing Tokio glue
  - [ ] `mdstream-tokio/Cargo.toml` `mdstream = "X.Y.Z"` dependency when the core crate version changes
- [ ] Move `CHANGELOG.md` `Unreleased` entries into a dated `X.Y.Z` section
- [ ] Ensure `README.md` contains all user-facing guidance (installation, quick start, examples)
- [ ] Run formatting and lint:
  - [ ] `cargo fmt --all -- --check`
  - [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] Run tests:
  - [ ] `cargo nextest run --workspace --all-features`
  - [ ] `cargo test --workspace --all-features --doc`
- [ ] Verify MSRV:
  - [ ] `cargo +1.85.0 test -p mdstream --tests --all-features`
  - [ ] `cargo +1.85.0 check -p mdstream --examples`
  - [ ] `cargo +1.85.0 check -p mdstream --features pulldown --examples`
  - [ ] `cargo +1.88.0 nextest run --workspace --all-features`
  - [ ] `cargo +1.88.0 test --workspace --all-features --doc`
  - [ ] `cargo +1.88.0 check -p mdstream-tokio --examples`
- [ ] Verify examples and benchmarks:
  - [ ] `cargo check -p mdstream --examples`
  - [ ] `cargo check -p mdstream --features pulldown --examples`
  - [ ] `cargo check -p mdstream-tokio --examples`
  - [ ] `cargo check -p mdstream --benches`
- [ ] Verify standalone fuzz package:
  - [ ] `cargo check --manifest-path fuzz/Cargo.toml --bins`
  - [ ] Optional deep run: `cargo +nightly fuzz build` and targeted `cargo +nightly fuzz run <target>`
- [ ] Verify packaging does not include large/internal folders:
  - [ ] `cargo package -p mdstream` (check the generated `.crate` contents)
  - [ ] `cargo package -p mdstream-tokio --list` (file list inspection before the new core crate is on crates.io)

## Prune `docs/` (if desired)

- [ ] Move any internal decisions you want to keep into an `adp/` (or similar) folder
- [ ] Delete `docs/` before publishing (if that is the release policy)
- [ ] Re-run `cargo package` to verify the crate is still self-explanatory

## Tag and publish

- [ ] Create tag `vX.Y.Z`
- [ ] Push tag
- [ ] Publish core crate first: `cargo publish -p mdstream`
- [ ] Wait until `mdstream` `X.Y.Z` is visible on crates.io
- [ ] Verify Tokio glue packaging after the new core crate is visible: `cargo package -p mdstream-tokio`
- [ ] Publish Tokio glue crate after its `mdstream` dependency version is visible: `cargo publish -p mdstream-tokio`
- [ ] Create a GitHub release for the tag (attach notes / changelog)
