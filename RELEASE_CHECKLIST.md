# Release Checklist

This checklist is optimized for `mdstream` releases where the `docs/` folder may be pruned (except internal ADP/ADR notes).

## Before tagging

- [ ] Update `Cargo.toml` `package.version`
- [ ] Update `CHANGELOG.md` for the release version
- [ ] Ensure `README.md` contains all user-facing guidance (installation, quick start, examples)
- [ ] Run formatting and lint:
  - [ ] `cargo fmt --all`
  - [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] Run tests:
  - [ ] `cargo test --tests`
  - [ ] `cargo test --all-features --tests`
- [ ] Verify optional feature builds:
  - [ ] `cargo check --examples`
  - [ ] `cargo check --features pulldown --examples`
- [ ] Verify packaging does not include large/internal folders:
  - [ ] `cargo package` (check the generated `.crate` contents)

## Prune `docs/` (if desired)

- [ ] Move any internal decisions you want to keep into an `adp/` (or similar) folder
- [ ] Delete `docs/` before publishing (if that is the release policy)
- [ ] Re-run `cargo package` to verify the crate is still self-explanatory

## Tag and publish

- [ ] Create tag `vX.Y.Z`
- [ ] Push tag
- [ ] Publish: `cargo publish`
- [ ] Create a GitHub release for the tag (attach notes / changelog)
