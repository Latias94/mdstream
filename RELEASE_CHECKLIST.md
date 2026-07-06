# Release Checklist

This checklist is optimized for `mdstream` releases where the `docs/` folder may be pruned (except internal ADP/ADR notes).

## Before tagging

- [ ] Update `Cargo.toml` `package.version`
- [ ] Update `CHANGELOG.md` for the release version
- [ ] Ensure `README.md` contains all user-facing guidance (installation, quick start, examples)
- [ ] Run formatting and lint:
  - [ ] `cargo fmt --all`
  - [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] Run tests:
  - [ ] `cargo nextest run --workspace --all-features`
  - [ ] `cargo test --workspace --all-features --doc`
- [ ] Verify MSRV:
  - [ ] `cargo +1.85.0 test -p mdstream --tests --all-features`
  - [ ] `cargo +1.88.0 nextest run --workspace --all-features`
- [ ] Verify examples and benchmarks:
  - [ ] `cargo check -p mdstream --examples`
  - [ ] `cargo check -p mdstream --features pulldown --examples`
  - [ ] `cargo check -p mdstream-tokio --examples`
  - [ ] `cargo check -p mdstream --benches`
- [ ] Verify packaging does not include large/internal folders:
  - [ ] `cargo package -p mdstream` (check the generated `.crate` contents)

## Prune `docs/` (if desired)

- [ ] Move any internal decisions you want to keep into an `adp/` (or similar) folder
- [ ] Delete `docs/` before publishing (if that is the release policy)
- [ ] Re-run `cargo package` to verify the crate is still self-explanatory

## Tag and publish

- [ ] Create tag `vX.Y.Z`
- [ ] Push tag
- [ ] Publish: `cargo publish`
- [ ] Create a GitHub release for the tag (attach notes / changelog)
