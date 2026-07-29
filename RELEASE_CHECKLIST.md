# Release Checklist

This checklist covers the complete mdstream 0.4 release graph. Release from a
clean tag only; do not publish individual packages from an unverified working
tree.

## Version and contract

- [ ] Move `CHANGELOG.md` entries from `Unreleased` into a dated `X.Y.Z`
  section.
- [ ] Set the same `X.Y.Z` version in every publishable Rust, npm, Dart, and
  Flutter manifest.
- [ ] Run the static release contract:

  ```sh
  python3 scripts/verify-packages.py --phase static
  python3 -m unittest scripts/test_verify_packages.py
  ```

- [ ] Confirm the tag is exactly `vX.Y.Z` and matches the contract version.
- [ ] Confirm protocol metadata still declares final `mdstream.content/0.4`
  and canonical conformance fixture IDs remain `mdstream.protocol/0.4`, rather
  than draft or candidate status.

## Rust publish graph

Canonical crates.io order:

`mdstream-protocol` -> `mdstream-processors` -> `mdstream` -> `mdstream-bindings-core` -> `mdstream-tokio` -> `mdstream-ffi` -> `mdstream-wasm` -> `mdstream-merman`

- [ ] Print the executable order and compare it with the line above:

  ```sh
  python3 scripts/verify-packages.py --print-rust-order
  ```

- [ ] Run local prepublish inventory validation. This checks packaged file
  lists, explicit internal dependency versions, and path-only dependency
  rules without pretending unpublished dependencies already exist on
  crates.io:

  ```sh
  python3 scripts/verify-packages.py --phase local --ecosystem rust
  ```

- [ ] During publishing, run the registry-dependent verification for one crate
  immediately before its upload, then wait until that exact version is visible
  before advancing:

  ```sh
  python3 scripts/verify-packages.py --phase registry --package mdstream-protocol
  ```

- [ ] Keep `mdstream-conformance` private. Publishable crates may use it only as
  a path-only dev dependency that Cargo removes from their published manifest.
- [ ] Verify `mdstream-merman` with Rust 1.95 while all core crates remain on
  their documented lower toolchain lanes.

## Verification gates

- [ ] Rust formatting, lint, tests, docs, examples, benchmarks, fuzz compile,
  deterministic budgets, and package inventories pass.
- [ ] Core MSRV passes on Rust 1.85; Tokio/workspace passes on Rust 1.88;
  standalone Merman passes on Rust 1.95.
- [ ] `wasm-pack 0.15.0` target/runtime tests pass with Rust 1.85 and
  `wasm32-unknown-unknown`.
- [ ] Node 24 and pnpm 11.9.0 pass frozen install, TypeScript typecheck/tests,
  build, package smoke, and absolute WASM/npm budgets.
- [ ] Dart 3.8.1 passes analyze/tests, host-supplied C FFI smoke, package
  inventory, and the standalone archive ceiling.
- [ ] Flutter 3.32.1 passes analyze/tests and the Android, iOS, macOS, Linux,
  and Windows bundled-library load matrix.
- [ ] Android ELF LOAD segments and the downstream APK ZIP entries are 16 KiB
  aligned, and the APK loads on the Android 15 16 KiB system image.
- [ ] Linux native libraries require no glibc newer than 2.17 and load on the
  pinned legacy-runtime smoke image.
- [ ] The assembled `mdstream_flutter` archive contains every declared native
  slice and passes absolute native-library and per-platform package ceilings.
- [ ] Android 16 KiB, Windows x64, macOS CocoaPods, Apple SwiftPM, iOS, and
  Linux consumers all download and load the producer's exact archive; none of
  those consumer jobs installs Rust, rebuilds native code, or falls back to the
  repository-local Flutter plugin.
- [ ] Refresh the checked-in `platform_package_increment` measurement from the
  exact clean-CI five-platform archive; do not retain a partial local archive
  fingerprint in a release commit.
- [ ] Default Rust, WASM, npm, Dart, and Flutter dependency scans contain no
  Merman, React, Streamdown, or Incremark production dependency.
- [ ] `git diff --check` passes and no generated binary, cache, or unrelated
  user change is staged.

## Binding release chain

- [ ] Pack and publish `@mdstream/core` only after WASM runtime and npm package
  smoke pass. The producer job must validate inventory, dependency policy,
  native-binary exclusion, and the absolute budget on the exact `.tgz` path
  uploaded for trusted publishing. No first-party React package is part of the
  release.
- [ ] Publish the standalone Dart package `mdstream` first; it intentionally
  contains no native binary and documents host-supplied library loading. Run
  the archive-aware verifier on the producer `.tar.gz`, then prove Pub's
  required repack preserves the same normalized file set and content.
- [ ] Wait until Dart `mdstream X.Y.Z` is visible on pub.dev.
- [ ] Publish `mdstream_flutter` from the assembled five-platform artifact; do
  not rebuild native slices in the trusted publish job. Before publishing,
  compare Pub's required repack with the verified archive by file path and
  content digest.
- [ ] Confirm npm and pub.dev trusted publishers are configured for
  `.github/workflows/release.yml` and the `npm` / `pub.dev` environments.

## Tag and release

- [ ] Create and push `vX.Y.Z` from the verified commit.
- [ ] Watch the release workflow until all crates.io, npm, Dart, Flutter, and
  GitHub release jobs complete.
- [ ] Confirm every registry exposes exactly `X.Y.Z`, package contents match
  the uploaded artifacts, and release notes match the changelog section.
