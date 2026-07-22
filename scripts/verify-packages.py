#!/usr/bin/env python3
"""Validate mdstream release versions, dependency order, and package contents."""

from __future__ import annotations

import argparse
import base64
import binascii
import hashlib
import json
import re
import subprocess
import sys
import tempfile
import tomllib
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from types import MappingProxyType
from typing import Iterable, Iterator, Mapping, NamedTuple, Sequence
from urllib.parse import quote, urlparse


SCRIPT_ROOT = Path(__file__).resolve().parent
ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPT_ROOT))

from archive_policy import (  # noqa: E402
    DEFAULT_ARCHIVE_LIMITS,
    ArchiveMember,
    ArchivePolicyError,
    extract_archive,
    visit_archive,
)
from release_notes import (  # noqa: E402
    ReleaseNotesError,
    extract_release_notes,
    first_release_version,
)

FLUTTER_TOOL_ROOT = ROOT / "bindings" / "flutter" / "tool"
sys.path.insert(0, str(FLUTTER_TOOL_ROOT))

from native_artifact import (  # noqa: E402
    NATIVE_MAGIC_PREFIX_BYTES,
    is_canonical_flutter_native_path,
    is_native_like_artifact,
    is_reserved_flutter_native_path,
)

RUST_PUBLISH_ORDER = (
    "mdstream-protocol",
    "mdstream-processors",
    "mdstream",
    "mdstream-bindings-core",
    "mdstream-tokio",
    "mdstream-ffi",
    "mdstream-wasm",
    "mdstream-merman",
)
RUST_MANIFESTS = {
    name: Path(name) / "Cargo.toml" for name in RUST_PUBLISH_ORDER
}
RUST_REQUIRED_FILES = {
    name: {"Cargo.toml", "src/lib.rs"} for name in RUST_PUBLISH_ORDER
}
RUST_REQUIRED_FILES["mdstream"] |= {
    "examples/README.md",
    "examples/custom_blocks.rs",
    "examples/fixtures/golden-ai-stream.json",
    "examples/headless_state.rs",
    "examples/minimal.rs",
    "examples/processor_lifecycle.rs",
    "examples/replica_recovery.rs",
    "examples/transition_trace.rs",
}
RUST_REQUIRED_FILES["mdstream-ffi"] |= {"README.md", "include/mdstream.h"}
RUST_REQUIRED_FILES["mdstream-tokio"] |= {
    "README.md",
    "examples/agent_tui.rs",
}
RUST_REQUIRED_FILES["mdstream-merman"] |= {
    "README.md",
    "examples/fixtures/golden-ai-stream.json",
    "examples/render_golden.rs",
}

NPM_REQUIRED_FILES = {
    "package.json",
    "README.md",
    "dist/index.js",
    "dist/index.d.ts",
    "wasm/mdstream_wasm.js",
    "wasm/mdstream_wasm_bg.wasm",
}
DART_REQUIRED_FILES = {
    "pubspec.yaml",
    "CHANGELOG.md",
    "README.md",
    "LICENSE",
    "lib/mdstream.dart",
    "lib/src/ffi.dart",
    "lib/src/reducer_handle.dart",
    "example/fixtures/golden_ai_stream.json",
    "example/golden_stream.dart",
}
FLUTTER_REQUIRED_FILES = {
    "pubspec.yaml",
    "CHANGELOG.md",
    "README.md",
    "LICENSE",
    "lib/mdstream_flutter.dart",
    "android/build.gradle",
    "ios/mdstream_flutter.podspec",
    "ios/mdstream_flutter/Package.swift",
    "ios/mdstream_flutter/Sources/mdstream_flutter/MdstreamFlutterPackage.swift",
    "linux/CMakeLists.txt",
    "macos/prepare_macos_framework.sh",
    "macos/mdstream_flutter.podspec",
    "macos/mdstream_flutter/Package.swift",
    "macos/mdstream_flutter/Sources/mdstream_flutter/MdstreamFlutterPackage.swift",
    "windows/CMakeLists.txt",
    "example/assets/golden_ai_stream.json",
    "example/configure_host.dart",
    "example/lib/configure_host.dart",
    "example/lib/bootstrap.dart",
    "example/lib/content_ir_view.dart",
    "example/lib/golden_stream_host.dart",
    "example/lib/main.dart",
    "example/pubspec.yaml",
}
FORBIDDEN_PACKAGE_PREFIXES = (
    ".git/",
    ".dart_tool/",
    "docs/plans/",
    "node_modules/",
    "repo-ref/",
    "target/",
)
PUB_REPOSITORY_ONLY_PREFIXES = (
    *FORBIDDEN_PACKAGE_PREFIXES,
    "integration_test/",
    "test/",
    "tool/",
    "example/.dart_tool/",
    "example/build/",
    "example/integration_test/",
    "example/test/",
    "example/pubspec.lock",
    "example/pubspec_overrides.yaml",
)
REQUIRED_DOCUMENTS = (
    "ARCHITECTURE.md",
    "STATE.md",
    "EXTENSIONS.md",
    "ADAPTERS.md",
    "COMPATIBILITY.md",
    "PERFORMANCE.md",
    "EXAMPLES.md",
    "USAGE.md",
    "ROADMAP.md",
)


@dataclass(frozen=True)
class ExampleContract:
    identifier: str
    role: str
    source_path: str
    prerequisite_marker: str
    command: str
    expected_marker: str
    next_link: str


EXAMPLE_CONTRACTS = (
    ExampleContract(
        identifier="rust-minimal",
        role="First-success tutorial",
        source_path="mdstream/examples/minimal.rs",
        prerequisite_marker="Rust 1.85",
        command="cargo run -p mdstream --example minimal -- --assert",
        expected_marker="ASSERTIONS_OK scenario=golden-ai-stream",
        next_link="../examples/web/README.md",
    ),
    ExampleContract(
        identifier="web-flagship",
        role="Interactive visual showcase",
        source_path="examples/web/src/main.ts",
        prerequisite_marker="Node 24",
        command="pnpm web:prepare && pnpm --filter @mdstream/example-web dev",
        expected_marker="Stream settled with finalized canonical content.",
        next_link="#flutter-host",
    ),
    ExampleContract(
        identifier="dart-headless",
        role="Headless binding tutorial",
        source_path="bindings/dart/example/golden_stream.dart",
        prerequisite_marker="Dart 3.8",
        command="cd bindings/dart && LIBRARY=$(dart run tool/build_native.dart) && dart run example/golden_stream.dart --library \"$LIBRARY\" --assert",
        expected_marker="assertions=passed",
        next_link="#flutter-host",
    ),
    ExampleContract(
        identifier="flutter-host",
        role="Interactive native host",
        source_path="bindings/flutter/example/lib/main.dart",
        prerequisite_marker="Flutter 3.32",
        command=(
            "python3 bindings/flutter/tool/build_native.py macos && "
            "cd bindings/flutter/example && flutter create --empty "
            "--platforms macos --project-name mdstream_flutter_example "
            "--org io.mdstream.example --no-pub . && "
            "dart run configure_host.dart macos && flutter run -d macos"
        ),
        expected_marker="Settled",
        next_link="#merman-artifact",
    ),
    ExampleContract(
        identifier="tokio-actor",
        role="Machine smoke probe",
        source_path="mdstream-tokio/examples/agent_tui.rs",
        prerequisite_marker="Rust 1.88",
        command="cargo +1.88.0 run -p mdstream-tokio --example agent_tui -- --smoke",
        expected_marker="SMOKE_OK",
        next_link="#web-flagship",
    ),
    ExampleContract(
        identifier="merman-artifact",
        role="Processor recipe",
        source_path="mdstream-merman/examples/render_golden.rs",
        prerequisite_marker="Rust 1.95",
        command="cargo +1.95.0 run --manifest-path mdstream-merman/Cargo.toml --example render_golden -- --assert",
        expected_marker="mdstream-merman golden stream: ok",
        next_link="EXTENSIONS.md",
    ),
)

WASM_TOOLS_MARKER = "tool: wasm-tools@1.253.0"
REGISTRY_USER_AGENT = (
    "mdstream-release-workflow/1 (+https://github.com/Latias94/mdstream)"
)
REGISTRY_CHECKER_ARTIFACT = "name: release-registry-version-checker"
REGISTRY_STATUS_GUARD_MARKERS = (
    "registry_status=$?",
    'if [[ "$registry_status" -ne 1 ]]; then',
    'exit "$registry_status"',
)


@dataclass(frozen=True)
class WorkflowJobContract:
    run_markers: tuple[str, ...] = ()
    job_markers: tuple[str, ...] = ()
    step_marker_groups: tuple[tuple[str, ...], ...] = ()
    marker_order: tuple[str, ...] | None = None
    required_needs: frozenset[str] | None = None
    reusable_call: str | None = None
    allowed_step_conditions: tuple[tuple[str, str], ...] = ()


@dataclass(frozen=True)
class WorkflowStep:
    text: str
    condition: str | None
    continue_on_error: str | None


WORKFLOW_JOB_CONTRACTS: Mapping[
    tuple[str, str], WorkflowJobContract
] = MappingProxyType(
    {
        ("ci.yml", "release-contract"): WorkflowJobContract(
            run_markers=(
                "scripts/verify-packages.py --phase static",
                "scripts/sync-example-fixtures.py --check",
                "python3 -m unittest scripts/test_sync_example_fixtures.py scripts/test_verify_packages.py",
            ),
        ),
        ("ci.yml", "core-msrv"): WorkflowJobContract(
            run_markers=(
                "cargo nextest run -p mdstream-conformance -p mdstream-protocol -p mdstream-processors -p mdstream --all-features",
                "cargo run -p mdstream --example minimal -- --assert",
                "cargo run -p mdstream --example headless_state",
                "cargo run -p mdstream --example transition_trace",
                "cargo run -p mdstream --example custom_blocks",
                "cargo run -p mdstream --example processor_lifecycle",
                "cargo run -p mdstream --example replica_recovery",
            ),
            job_markers=("dtolnay/rust-toolchain@1.85.0",),
            required_needs=frozenset(("release-contract",)),
        ),
        ("ci.yml", "workspace-msrv"): WorkflowJobContract(
            run_markers=(
                "cargo nextest run --workspace --all-features",
                "cargo run -p mdstream-tokio --example agent_tui -- --smoke",
            ),
            job_markers=("dtolnay/rust-toolchain@1.88.0",),
            required_needs=frozenset(("release-contract",)),
        ),
        ("ci.yml", "quality"): WorkflowJobContract(
            run_markers=(
                "cargo fmt --all -- --check",
                "cargo clippy --workspace --all-targets --all-features -- -D warnings",
                "cargo nextest run --workspace --all-features",
                "cargo test --workspace --all-features --doc",
                "cargo check -p mdstream --examples --all-features",
                "cargo check -p mdstream --benches --all-features",
                "cargo check --manifest-path fuzz/Cargo.toml --bins",
            ),
            required_needs=frozenset(("release-contract",)),
        ),
        ("ci.yml", "merman"): WorkflowJobContract(
            run_markers=(
                "cargo +1.95.0 nextest run --manifest-path "
                "mdstream-merman/Cargo.toml --all-features",
                "cargo +1.95.0 run --manifest-path "
                "mdstream-merman/Cargo.toml --example render_golden -- --assert",
            ),
            job_markers=("dtolnay/rust-toolchain@1.95.0",),
            required_needs=frozenset(("release-contract",)),
        ),
        ("ci.yml", "web"): WorkflowJobContract(
            run_markers=(
                "cargo check -p mdstream-wasm --target "
                "wasm32-unknown-unknown --all-features",
                "wasm-pack test --node mdstream-wasm",
                "pnpm install --frozen-lockfile",
                "pnpm web:prepare",
                "pnpm -r test",
                "pnpm -r build",
                "node bindings/typescript/examples/transition-host.mjs --assert",
                "pnpm --filter @mdstream/example-web exec playwright install --with-deps chromium",
                "pnpm --filter @mdstream/example-web test:e2e",
            ),
            job_markers=(
                "tool: wasm-pack@0.15.0",
                WASM_TOOLS_MARKER,
                "node-version: 24",
                "version: 11.9.0",
            ),
            step_marker_groups=(
                ("uses: taiki-e/install-action@v2", WASM_TOOLS_MARKER),
            ),
            marker_order=(WASM_TOOLS_MARKER, "pnpm -r build"),
            required_needs=frozenset(("release-contract",)),
        ),
        ("ci.yml", "dart"): WorkflowJobContract(
            run_markers=(
                "dart analyze",
                "dart run tool/test_native.dart",
                'dart run example/golden_stream.dart --library "$LIBRARY" --assert',
            ),
            job_markers=("sdk: 3.8.1", "flutter-version: 3.32.1"),
            required_needs=frozenset(("release-contract",)),
        ),
        ("ci.yml", "rust-packages"): WorkflowJobContract(
            required_needs=frozenset(("release-contract",)),
        ),
        ("flutter-platforms.yml", "linux"): WorkflowJobContract(
            run_markers=(
                "python3 scripts/sync-example-fixtures.py --check",
                "flutter analyze",
                "flutter test",
                "flutter test test/golden_stream_test.dart",
                "flutter test integration_test/golden_stream_smoke_test.dart -d linux",
                "python3 bindings/flutter/tool/build_native.py linux",
            ),
            job_markers=(
                "flutter-version: 3.32.1",
                "uses: mlugg/setup-zig@v2",
                "version: 0.15.2",
                "tool: cargo-zigbuild@0.23.0",
            ),
            marker_order=(
                "python3 scripts/sync-example-fixtures.py --check",
                "flutter test integration_test/golden_stream_smoke_test.dart -d linux",
            ),
        ),
        ("flutter-platforms.yml", "android"): WorkflowJobContract(
            run_markers=(
                "python3 bindings/flutter/tool/build_native.py android",
            ),
            job_markers=(
                '"build-tools;35.0.0"',
                "reactivecircus/android-emulator-runner@v2.38.0",
                "target: google_apis_ps16k",
                "api-level: 35",
                "python3 bindings/flutter/tool/android_smoke.py --skip-native-build",
            ),
        ),
        ("flutter-platforms.yml", "apple"): WorkflowJobContract(
            run_markers=(
                'xcrun simctl bootstatus "$DEVICE_ID" -b',
                "package_smoke.py --platform ios",
                "package_smoke.py --swiftpm --platform ios",
            ),
            step_marker_groups=(
                (
                    "Boot iOS simulator and load bundled library",
                    "timeout-minutes: 30",
                    'xcrun simctl bootstatus "$DEVICE_ID" -b',
                ),
            ),
            allowed_step_conditions=(
                (
                    "package_smoke.py --platform macos --device macos",
                    "matrix.platform == 'macos'",
                ),
                (
                    "package_smoke.py --swiftpm --platform macos",
                    "matrix.platform == 'macos'",
                ),
                (
                    "Boot iOS simulator and load bundled library",
                    "matrix.platform == 'ios'",
                ),
            ),
        ),
        ("flutter-platforms.yml", "package"): WorkflowJobContract(
            run_markers=(
                "bindings/flutter/tool/package_smoke.py --print-archive-path",
                "bindings/flutter/tool/package_smoke.py --release",
                '--ecosystem flutter --archive "$FLUTTER_ARCHIVE"',
            ),
        ),
        ("flutter-platforms.yml", "package-linux-smoke"): WorkflowJobContract(
            run_markers=(
                'package_smoke.py --archive "$FLUTTER_ARCHIVE" '
                "--platform linux --device linux --skip-native-build",
            ),
            job_markers=(
                "name: mdstream-flutter-package",
                "flutter-version: 3.32.1",
            ),
            required_needs=frozenset(("package",)),
        ),
        ("flutter-platforms.yml", "package-linux-legacy-smoke"): WorkflowJobContract(
            run_markers=(
                "scripts/verify-packages.py --extract-only",
                "target/flutter-extracted",
                "zig cc -target x86_64-linux-gnu.2.17",
                "debian:10.13-slim /mdstream/c-consumer",
            ),
            job_markers=(
                "name: mdstream-flutter-package",
                "uses: mlugg/setup-zig@v2",
                "version: 0.15.2",
            ),
            required_needs=frozenset(("package",)),
        ),
        ("flutter-platforms.yml", "package-ios-smoke"): WorkflowJobContract(
            run_markers=(
                'xcrun simctl bootstatus "$DEVICE_ID" -b',
                'package_smoke.py --archive "$FLUTTER_ARCHIVE" '
                '--platform ios --device "$DEVICE_ID" --skip-native-build',
                'package_smoke.py --swiftpm --archive "$FLUTTER_ARCHIVE" '
                '--platform ios --device "$DEVICE_ID" --skip-native-build',
            ),
            job_markers=(
                "name: mdstream-flutter-package",
                "flutter-version: 3.32.1",
                "timeout-minutes: 30",
            ),
            marker_order=(
                'xcrun simctl bootstatus "$DEVICE_ID" -b',
                'package_smoke.py --archive "$FLUTTER_ARCHIVE" '
                '--platform ios --device "$DEVICE_ID" --skip-native-build',
                'package_smoke.py --swiftpm --archive "$FLUTTER_ARCHIVE" '
                '--platform ios --device "$DEVICE_ID" --skip-native-build',
            ),
            required_needs=frozenset(("package",)),
        ),
        ("release.yml", "publish-rust"): WorkflowJobContract(
            run_markers=(
                "scripts/verify-packages.py --print-rust-order",
                "scripts/verify-packages.py --phase registry --package",
                "scripts/check-registry-version.py crates.io",
                *REGISTRY_STATUS_GUARD_MARKERS,
                "timeout 30s cargo info --registry crates-io",
                "--locked --token",
                "--compare-registry crates.io",
            ),
            step_marker_groups=(
                (
                    "scripts/check-registry-version.py crates.io",
                    *REGISTRY_STATUS_GUARD_MARKERS,
                    "cargo publish",
                    "--compare-registry crates.io",
                ),
            ),
            marker_order=(
                "scripts/check-registry-version.py crates.io",
                "cargo publish",
                "--compare-registry crates.io",
            ),
            required_needs=frozenset(
                ("validate", "quality", "flutter-platforms", "release-preflight")
            ),
        ),
        ("release.yml", "build-npm"): WorkflowJobContract(
            run_markers=(
                "pnpm --filter @mdstream/core pack",
                '--ecosystem npm --archive "$NPM_ARCHIVE"',
            ),
            job_markers=(
                "tool: wasm-pack@0.15.0",
                WASM_TOOLS_MARKER,
                "node-version: 24",
                "version: 11.9.0",
            ),
            step_marker_groups=(
                ("uses: taiki-e/install-action@v2", WASM_TOOLS_MARKER),
            ),
            marker_order=(
                WASM_TOOLS_MARKER,
                "pnpm --filter @mdstream/core pack",
            ),
            required_needs=frozenset(("validate",)),
        ),
        ("release.yml", "publish-npm"): WorkflowJobContract(
            run_markers=(
                "check-registry-version.py npm @mdstream/core",
                *REGISTRY_STATUS_GUARD_MARKERS,
                "npm publish",
                "--compare-registry npm",
            ),
            job_markers=(REGISTRY_CHECKER_ARTIFACT,),
            step_marker_groups=(
                (
                    "check-registry-version.py npm @mdstream/core",
                    *REGISTRY_STATUS_GUARD_MARKERS,
                    "npm publish",
                    "--compare-registry npm",
                ),
                (
                    "uses: actions/download-artifact@v4",
                    REGISTRY_CHECKER_ARTIFACT,
                    "path: target/release-tools",
                ),
            ),
            marker_order=(
                "check-registry-version.py npm @mdstream/core",
                "npm publish",
                "--compare-registry npm",
            ),
            required_needs=frozenset(
                (
                    "validate",
                    "quality",
                    "build-npm",
                    "flutter-platforms",
                    "release-preflight",
                )
            ),
        ),
        ("release.yml", "build-dart"): WorkflowJobContract(
            required_needs=frozenset(("validate",)),
        ),
        ("release.yml", "publish-dart"): WorkflowJobContract(
            run_markers=(
                "--extract-only",
                "dart pub publish --skip-validation --to-archive",
                "--compare-only",
                "check-registry-version.py\" pub.dev mdstream",
                *REGISTRY_STATUS_GUARD_MARKERS,
                "dart pub publish",
                "--compare-registry pub.dev",
            ),
            job_markers=(REGISTRY_CHECKER_ARTIFACT,),
            step_marker_groups=(
                (
                    "check-registry-version.py\" pub.dev mdstream",
                    *REGISTRY_STATUS_GUARD_MARKERS,
                    "dart pub publish",
                    "--compare-registry pub.dev",
                ),
                (
                    "uses: actions/download-artifact@v4",
                    REGISTRY_CHECKER_ARTIFACT,
                    "path: target/release-tools",
                ),
                (
                    "dart pub publish --skip-validation --to-archive",
                    "--compare-only",
                ),
            ),
            marker_order=(
                "--extract-only",
                "dart pub publish --skip-validation --to-archive",
                "--compare-only",
                "check-registry-version.py\" pub.dev mdstream",
                "dart pub publish --force",
                "--compare-registry pub.dev",
            ),
            required_needs=frozenset(
                (
                    "validate",
                    "quality",
                    "build-dart",
                    "flutter-platforms",
                    "release-preflight",
                )
            ),
        ),
        ("release.yml", "publish-flutter"): WorkflowJobContract(
            run_markers=(
                "--extract-only",
                "dart pub publish --skip-validation --to-archive",
                "--compare-only",
                "check-registry-version.py\" pub.dev mdstream_flutter",
                *REGISTRY_STATUS_GUARD_MARKERS,
                "dart pub publish",
                "--compare-registry pub.dev",
            ),
            job_markers=(REGISTRY_CHECKER_ARTIFACT,),
            step_marker_groups=(
                (
                    "check-registry-version.py\" pub.dev mdstream_flutter",
                    *REGISTRY_STATUS_GUARD_MARKERS,
                    "dart pub publish",
                    "--compare-registry pub.dev",
                ),
                (
                    "uses: actions/download-artifact@v4",
                    REGISTRY_CHECKER_ARTIFACT,
                    "path: target/release-tools",
                ),
                (
                    "dart pub publish --skip-validation --to-archive",
                    "--compare-only",
                ),
            ),
            marker_order=(
                "--extract-only",
                "dart pub publish --skip-validation --to-archive",
                "--compare-only",
                "check-registry-version.py\" pub.dev mdstream_flutter",
                "dart pub publish --force",
                "--compare-registry pub.dev",
            ),
            required_needs=frozenset(
                (
                    "validate",
                    "quality",
                    "publish-dart",
                    "flutter-platforms",
                    "release-preflight",
                )
            ),
        ),
        ("release.yml", "validate"): WorkflowJobContract(
            run_markers=(
                "scripts/release_notes.py",
                "--output target/release-notes.md",
                "target/release-tools/scripts/verify-packages.py --help",
            ),
            job_markers=(
                REGISTRY_CHECKER_ARTIFACT,
                "name: mdstream-release-notes",
                "path: target/release-notes.md",
            ),
            step_marker_groups=(
                (
                    "mkdir -p target/release-tools/scripts",
                    "scripts/archive_policy.py",
                    "scripts/check-registry-version.py",
                    "scripts/release_notes.py",
                    "scripts/verify-packages.py",
                    "bindings/flutter/tool/native_artifact.py",
                    "target/release-tools/scripts/verify-packages.py --help",
                ),
                (
                    "uses: actions/upload-artifact@v4",
                    REGISTRY_CHECKER_ARTIFACT,
                    "path: target/release-tools/**",
                ),
                (
                    "uses: actions/upload-artifact@v4",
                    "name: mdstream-release-notes",
                    "path: target/release-notes.md",
                ),
            ),
        ),
        ("release.yml", "quality"): WorkflowJobContract(
            reusable_call="./.github/workflows/ci.yml",
        ),
        ("release.yml", "flutter-platforms"): WorkflowJobContract(
            reusable_call="./.github/workflows/flutter-platforms.yml",
        ),
        ("release.yml", "release-preflight"): WorkflowJobContract(
            run_markers=("Rust, npm, Dart, and Flutter producer gates passed",),
            required_needs=frozenset(
                ("validate", "quality", "flutter-platforms", "build-npm", "build-dart")
            ),
        ),
        ("release.yml", "github-release"): WorkflowJobContract(
            job_markers=(
                "name: mdstream-release-notes",
                "path: target/release-notes",
                "body_path: target/release-notes/release-notes.md",
            ),
            step_marker_groups=(
                (
                    "uses: actions/download-artifact@v4",
                    "name: mdstream-release-notes",
                    "path: target/release-notes",
                ),
                (
                    "uses: softprops/action-gh-release@v2",
                    "body_path: target/release-notes/release-notes.md",
                ),
            ),
            required_needs=frozenset(
                ("validate", "publish-rust", "publish-npm", "publish-flutter")
            ),
        ),
    }
)

REQUIRED_WORKFLOW_EVENTS = {
    "ci.yml": {"workflow_call", "push", "pull_request"},
    "flutter-platforms.yml": {"workflow_call", "push", "pull_request"},
    "release.yml": {"push"},
}


class ValidationError(RuntimeError):
    """Raised when a release or package contract is invalid."""


class ReleaseContract(NamedTuple):
    version: str
    rust_publish_order: tuple[str, ...]


class ArchiveContents(NamedTuple):
    paths: set[str]
    manifests: dict[str, bytes]


class ArchiveFileFingerprint(NamedTuple):
    size: int
    sha256: str


@dataclass(frozen=True)
class RegistryArchiveDescriptor:
    url: str
    checksum_algorithm: str | None = None
    checksum: bytes | None = None


@dataclass(frozen=True)
class RustDependency:
    name: str
    kind: str | None
    requirement: str
    source: str | None


@dataclass(frozen=True)
class RustPackage:
    version: str
    dependencies: tuple[RustDependency, ...]


def load_toml(path: Path) -> dict[str, object]:
    try:
        with path.open("rb") as handle:
            value = tomllib.load(handle)
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise ValidationError(f"failed to parse {path}: {error}") from error
    return value


def top_level_yaml_scalar(path: Path, field: str) -> str:
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as error:
        raise ValidationError(f"failed to read {path}: {error}") from error
    return top_level_yaml_scalar_text(text, field, str(path))


_YAML_SCALAR = r"(?:'([^']+)'|\"([^\"]+)\"|([^#\s]+))"


def _yaml_scalar_match_value(match: re.Match[str]) -> str:
    return next(value for value in match.groups() if value is not None)


def top_level_yaml_scalar_text(text: str, field: str, label: str) -> str:
    match = re.search(
        rf"^(?!\s){re.escape(field)}:\s*{_YAML_SCALAR}\s*(?:#.*)?$",
        text,
        flags=re.MULTILINE,
    )
    if match is None:
        raise ValidationError(f"{label} has no top-level {field}")
    return _yaml_scalar_match_value(match)


def top_level_yaml_mapping_scalar(
    text: str,
    field: str,
    key: str,
    label: str,
) -> str:
    lines = text.splitlines()
    for index, line in enumerate(lines):
        if line != f"{field}:":
            continue
        pattern = re.compile(
            rf"^  {re.escape(key)}:\s*{_YAML_SCALAR}\s*(?:#.*)?$"
        )
        for child in lines[index + 1 :]:
            if child and not child[0].isspace() and not child.lstrip().startswith("#"):
                break
            match = pattern.match(child)
            if match is not None:
                return _yaml_scalar_match_value(match)
        break
    raise ValidationError(f"{label} has no scalar {field}.{key}")


def expected_release_version(root: Path) -> str:
    path = root / RUST_MANIFESTS["mdstream-protocol"]
    manifest = load_toml(path)
    package = manifest.get("package")
    if not isinstance(package, dict) or package.get("name") != "mdstream-protocol":
        raise ValidationError(f"{path} must define package mdstream-protocol")
    version = str(package.get("version", ""))
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValidationError(f"release version is not stable semver: {version}")
    return version


def validate_package_changelog(path: Path, version: str) -> None:
    try:
        text = path.read_text(encoding="utf-8")
        first_version = first_release_version(text)
        extract_release_notes(text, version)
    except (OSError, ReleaseNotesError) as error:
        raise ValidationError(f"invalid package changelog {path}: {error}") from error
    if first_version != version:
        raise ValidationError(
            f"package changelog {path} starts at {first_version}, expected {version}"
        )


def validate_pub_lock_sources(root: Path) -> None:
    path = root / "bindings" / "pubspec.lock"
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as error:
        raise ValidationError(f"failed to read {path}: {error}") from error
    urls = set(re.findall(r'^\s+url:\s*["\']?([^"\'\s#]+)', text, re.MULTILINE))
    unexpected = sorted(url for url in urls if url != "https://pub.dev")
    if unexpected:
        raise ValidationError(
            f"{path} contains non-pub.dev hosted source {unexpected[0]}"
        )


def validate_versions(version_by_package: Mapping[str, str]) -> str:
    expected = version_by_package.get("mdstream-protocol")
    if expected is None:
        raise ValidationError("mdstream-protocol is the required release version source")
    mismatches = [
        (name, version)
        for name, version in version_by_package.items()
        if version != expected
    ]
    if mismatches:
        detail = ", ".join(
            f"{name} is {version}, expected {expected}"
            for name, version in mismatches
        )
        raise ValidationError(f"release version mismatch: {detail}")
    if not re.fullmatch(r"\d+\.\d+\.\d+", expected):
        raise ValidationError(f"release version is not stable semver: {expected}")
    return expected


def validate_flutter_version_metadata(root: Path, version: str) -> None:
    metadata = (
        (
            "Android Gradle metadata",
            root / "bindings" / "flutter" / "android" / "build.gradle",
            r'(?m)^version\s*=\s*[\'\"]([^\'\"]+)[\'\"]\s*$',
        ),
        (
            "iOS podspec",
            root
            / "bindings"
            / "flutter"
            / "ios"
            / "mdstream_flutter.podspec",
            r'(?m)^\s*s\.version\s*=\s*[\'\"]([^\'\"]+)[\'\"]\s*$',
        ),
        (
            "macOS podspec",
            root
            / "bindings"
            / "flutter"
            / "macos"
            / "mdstream_flutter.podspec",
            r'(?m)^\s*s\.version\s*=\s*[\'\"]([^\'\"]+)[\'\"]\s*$',
        ),
    )
    for label, path, pattern in metadata:
        try:
            text = path.read_text(encoding="utf-8")
        except OSError as error:
            raise ValidationError(f"failed to read {label}: {error}") from error
        match = re.search(pattern, text)
        if match is None:
            raise ValidationError(f"{label} has no package version")
        actual = match.group(1)
        if actual != version:
            raise ValidationError(
                f"{label} version is {actual}, expected {version}"
            )


def _dependency_records(manifest: Mapping[str, object]) -> tuple[RustDependency, ...]:
    records: list[RustDependency] = []
    sections = (
        ("dependencies", None),
        ("build-dependencies", "build"),
        ("dev-dependencies", "dev"),
    )
    for section, kind in sections:
        dependencies = manifest.get(section, {})
        if not isinstance(dependencies, dict):
            raise ValidationError(f"Cargo {section} must be a table")
        for alias, raw in dependencies.items():
            if isinstance(raw, str):
                name = alias
                requirement = raw
                source = "registry"
            elif isinstance(raw, dict):
                name = raw.get("package", alias)
                requirement = raw.get("version", "*")
                source = None if "path" in raw else "registry"
            else:
                raise ValidationError(f"invalid Cargo dependency {alias}")
            records.append(
                RustDependency(
                    name=str(name),
                    kind=kind,
                    requirement=str(requirement),
                    source=source,
                )
            )
    return tuple(records)


def load_rust_packages(root: Path) -> dict[str, RustPackage]:
    packages: dict[str, RustPackage] = {}
    for expected_name, relative_path in RUST_MANIFESTS.items():
        manifest = load_toml(root / relative_path)
        package = manifest.get("package")
        if not isinstance(package, dict):
            raise ValidationError(f"{relative_path} has no [package] table")
        name = package.get("name")
        if name != expected_name:
            raise ValidationError(
                f"{relative_path} package name is {name!r}, expected {expected_name}"
            )
        if package.get("publish") is False:
            raise ValidationError(f"release crate {name} is marked publish = false")
        packages[expected_name] = RustPackage(
            version=str(package.get("version", "")),
            dependencies=_dependency_records(manifest),
        )
    return packages


def validate_rust_topology(
    order: Sequence[str],
    packages: Mapping[str, RustPackage],
) -> None:
    if len(order) != len(set(order)):
        raise ValidationError("Rust publish order contains duplicates")
    expected = set(packages)
    actual = set(order)
    if actual != expected:
        missing = sorted(expected - actual)
        extra = sorted(actual - expected)
        raise ValidationError(
            f"Rust publish order mismatch; missing={missing}, extra={extra}"
        )
    positions = {name: index for index, name in enumerate(order)}
    for dependent, package in packages.items():
        for dependency in package.dependencies:
            if dependency.kind == "dev":
                continue
            if (
                dependency.name in positions
                and positions[dependency.name] > positions[dependent]
            ):
                raise ValidationError(
                    "release order publishes "
                    f"{dependent} before dependency {dependency.name}"
                )


def _requirement_matches(requirement: str, version: str) -> bool:
    requirement = requirement.strip()
    if requirement == "*" or not requirement:
        return False
    match = re.fullmatch(r"(?:=|\^|~)?(\d+\.\d+\.\d+)", requirement)
    return match is not None and match.group(1) == version


def validate_internal_dependency_versions(
    packages: Mapping[str, RustPackage],
    publishable: set[str],
    version: str,
) -> None:
    for dependent, package in packages.items():
        for dependency in package.dependencies:
            name = dependency.name
            if dependency.kind == "dev" or name not in publishable:
                continue
            requirement = dependency.requirement
            if dependency.source is None and requirement == "*":
                raise ValidationError(
                    f"path-only dependency from {dependent} to {name} cannot be published"
                )
            if not _requirement_matches(requirement, version):
                raise ValidationError(
                    f"{dependent} requires {name} {requirement}, expected {version}"
                )


def validate_workspace_inventory(root: Path) -> None:
    workspace = load_toml(root / "Cargo.toml").get("workspace")
    if not isinstance(workspace, dict):
        raise ValidationError("root Cargo.toml has no [workspace]")
    members = workspace.get("members")
    if not isinstance(members, list):
        raise ValidationError("workspace members must be an array")
    publishable: set[str] = {"mdstream-merman"}
    for member in members:
        manifest = load_toml(root / str(member) / "Cargo.toml")
        package = manifest.get("package")
        if not isinstance(package, dict):
            continue
        if package.get("publish") is not False:
            publishable.add(str(package.get("name")))
    if publishable != set(RUST_PUBLISH_ORDER):
        raise ValidationError(
            "publishable Rust package inventory differs from RUST_PUBLISH_ORDER: "
            f"{sorted(publishable)}"
        )


def validate_lock_versions(root: Path, versions: Mapping[str, str]) -> None:
    lock = load_toml(root / "Cargo.lock")
    packages = lock.get("package")
    if not isinstance(packages, list):
        raise ValidationError("Cargo.lock has no package array")
    locked = {
        str(package.get("name")): str(package.get("version"))
        for package in packages
        if isinstance(package, dict) and package.get("source") is None
    }
    for name in RUST_PUBLISH_ORDER:
        if name == "mdstream-merman":
            continue
        expected = versions[name]
        if locked.get(name) != expected:
            raise ValidationError(
                f"Cargo.lock has {name} {locked.get(name)!r}, expected {expected}"
            )


def _workflow_jobs(text: str, filename: str) -> dict[str, str]:
    lines = text.splitlines()
    try:
        jobs_index = lines.index("jobs:")
    except ValueError as error:
        raise ValidationError(f"workflow {filename} has no jobs mapping") from error
    starts = [
        (index, match.group(1))
        for index, line in enumerate(lines[jobs_index + 1 :], jobs_index + 1)
        if (match := re.match(r"^  ([A-Za-z0-9_-]+):\s*$", line)) is not None
    ]
    jobs: dict[str, str] = {}
    for position, (start, name) in enumerate(starts):
        end = starts[position + 1][0] if position + 1 < len(starts) else len(lines)
        jobs[name] = "\n".join(lines[start:end])
    return jobs


def _active_workflow_text(block: str) -> str:
    active: list[str] = []
    for line in block.splitlines():
        if line.lstrip().startswith("#"):
            continue
        active.append(line.split(" #", 1)[0])
    return "\n".join(active)


def _workflow_scalar(
    block: str,
    key: str,
    *,
    mapping_indent: int,
    sequence_item: bool = False,
) -> str | None:
    prefixes = [" " * mapping_indent + f"{key}:"]
    if sequence_item:
        prefixes.append(" " * (mapping_indent - 2) + f"- {key}:")
    for line in _active_workflow_text(block).splitlines():
        for prefix in prefixes:
            if line.startswith(prefix):
                return line[len(prefix) :].strip()
    return None


def _workflow_condition(
    block: str,
    *,
    mapping_indent: int,
    sequence_item: bool = False,
) -> str | None:
    return _workflow_scalar(
        block,
        "if",
        mapping_indent=mapping_indent,
        sequence_item=sequence_item,
    )


def _condition_is_statically_false(condition: str | None) -> bool:
    if condition is None:
        return False
    value = condition.strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in ("'", '"'):
        value = value[1:-1].strip()
    value = " ".join(value.casefold().split())
    return value in {"false", "${{ false }}"}


def _workflow_run_commands_from_step(block: str) -> tuple[str, ...]:
    lines = block.splitlines()
    commands: list[str] = []
    index = 0
    while index < len(lines):
        line = lines[index]
        if line.lstrip().startswith("#"):
            index += 1
            continue
        match = re.match(r"^(\s*)(?:- )?run:\s*(.*)$", line)
        if match is None:
            index += 1
            continue
        indent = len(match.group(1))
        value = match.group(2)
        if value not in ("|", "|-", ">", ">-"):
            commands.append(value.split(" #", 1)[0])
            index += 1
            continue
        index += 1
        while index < len(lines):
            child = lines[index]
            if child.strip() and len(child) - len(child.lstrip(" ")) <= indent:
                break
            if child.strip() and not child.lstrip().startswith("#"):
                commands.append(child.strip())
            index += 1
    return tuple(commands)


def _workflow_steps(block: str) -> tuple[WorkflowStep, ...]:
    active = _active_workflow_text(block)
    lines = active.splitlines()
    try:
        steps_index = lines.index("    steps:")
    except ValueError:
        return ()
    steps_end = next(
        (
            index
            for index in range(steps_index + 1, len(lines))
            if lines[index].strip()
            and len(lines[index]) - len(lines[index].lstrip(" ")) <= 4
        ),
        len(lines),
    )
    starts = [
        index
        for index in range(steps_index + 1, steps_end)
        if re.match(r"^      - \S", lines[index]) is not None
    ]
    steps: list[WorkflowStep] = []
    for position, start in enumerate(starts):
        end = starts[position + 1] if position + 1 < len(starts) else steps_end
        text = "\n".join(lines[start:end])
        steps.append(
            WorkflowStep(
                text=text,
                condition=_workflow_condition(
                    text,
                    mapping_indent=8,
                    sequence_item=True,
                ),
                continue_on_error=_workflow_scalar(
                    text,
                    "continue-on-error",
                    mapping_indent=8,
                    sequence_item=True,
                ),
            )
        )
    return tuple(steps)


def _enabled_workflow_steps(block: str) -> tuple[WorkflowStep, ...]:
    return tuple(
        step
        for step in _workflow_steps(block)
        if not _condition_is_statically_false(step.condition)
    )


def _enabled_workflow_job_text(block: str) -> str:
    active = _active_workflow_text(block)
    for step in _workflow_steps(block):
        if _condition_is_statically_false(step.condition):
            active = active.replace(step.text, "", 1)
    return active


def _workflow_run_commands(block: str) -> str:
    commands = (
        command
        for step in _enabled_workflow_steps(block)
        for command in _workflow_run_commands_from_step(step.text)
    )
    return "\n".join(commands)


def _workflow_job_needs(block: str, filename: str, job_name: str) -> frozenset[str]:
    for line in _active_workflow_text(block).splitlines():
        match = re.fullmatch(r"    needs:\s*(.+?)\s*", line)
        if match is None:
            continue
        value = match.group(1)
        if value.startswith("[") and value.endswith("]"):
            needs = tuple(item.strip() for item in value[1:-1].split(","))
        else:
            needs = (value,)
        if not needs or any(
            re.fullmatch(r"[A-Za-z0-9_-]+", dependency) is None
            for dependency in needs
        ):
            raise ValidationError(
                f"workflow {filename} job {job_name} has unsupported needs {value!r}"
            )
        return frozenset(needs)
    return frozenset()


def validate_workflow_contract(root: Path) -> None:
    workflow_root = root / ".github" / "workflows"
    filenames = {
        filename for filename, _ in WORKFLOW_JOB_CONTRACTS
    } | set(REQUIRED_WORKFLOW_EVENTS)
    texts: dict[str, str] = {}
    jobs_by_workflow: dict[str, dict[str, str]] = {}
    for filename in sorted(filenames):
        path = workflow_root / filename
        try:
            text = path.read_text(encoding="utf-8")
        except OSError as error:
            raise ValidationError(f"failed to read workflow {filename}: {error}") from error
        texts[filename] = text
        jobs_by_workflow[filename] = _workflow_jobs(text, filename)

    for filename, required_events in REQUIRED_WORKFLOW_EVENTS.items():
        events = top_level_yaml_mapping_keys(texts[filename], "on")
        missing = sorted(required_events - events)
        if missing:
            raise ValidationError(
                f"workflow {filename} is missing trigger event(s): {missing}"
            )

    for filename, job_name in WORKFLOW_JOB_CONTRACTS:
        job = jobs_by_workflow[filename].get(job_name)
        if job is None:
            raise ValidationError(f"workflow {filename} is missing job {job_name}")
        condition = _workflow_condition(job, mapping_indent=4)
        if condition is not None:
            raise ValidationError(
                f"workflow {filename} job {job_name} is conditional"
            )
        continue_on_error = _workflow_scalar(
            job,
            "continue-on-error",
            mapping_indent=4,
        )
        if continue_on_error is not None:
            raise ValidationError(
                f"workflow {filename} job {job_name} uses continue-on-error"
            )

        contract = WORKFLOW_JOB_CONTRACTS[(filename, job_name)]
        allowed_conditions = dict(contract.allowed_step_conditions)
        for step in _workflow_steps(job):
            if step.continue_on_error is not None:
                raise ValidationError(
                    f"workflow {filename} job {job_name} step uses continue-on-error"
                )
            if step.condition is None:
                continue
            matching = [
                expected
                for marker, expected in allowed_conditions.items()
                if marker in step.text
            ]
            if len(matching) != 1 or step.condition != matching[0]:
                raise ValidationError(
                    f"workflow {filename} job {job_name} has an unsupported "
                    f"conditional step: {step.condition}"
                )

    for (filename, job_name), contract in WORKFLOW_JOB_CONTRACTS.items():
        markers = contract.run_markers
        if not markers:
            continue
        job = jobs_by_workflow[filename].get(job_name)
        if job is None:
            raise ValidationError(f"workflow {filename} is missing job {job_name}")
        commands = _workflow_run_commands(job)
        missing = [marker for marker in markers if marker not in commands]
        if missing:
            raise ValidationError(
                f"workflow {filename} job {job_name} is missing executable gate(s): {missing}"
            )

    for (filename, job_name), contract in WORKFLOW_JOB_CONTRACTS.items():
        markers = contract.job_markers
        if not markers:
            continue
        job = jobs_by_workflow[filename].get(job_name)
        if job is None:
            raise ValidationError(f"workflow {filename} is missing job {job_name}")
        active = _enabled_workflow_job_text(job)
        missing = [marker for marker in markers if marker not in active]
        if missing:
            raise ValidationError(
                f"workflow {filename} job {job_name} is missing configuration gate(s): {missing}"
            )

    for (filename, job_name), contract in WORKFLOW_JOB_CONTRACTS.items():
        marker_groups = contract.step_marker_groups
        if not marker_groups:
            continue
        job = jobs_by_workflow[filename].get(job_name)
        if job is None:
            raise ValidationError(f"workflow {filename} is missing job {job_name}")
        steps = _enabled_workflow_steps(job)
        for markers in marker_groups:
            if not any(
                all(marker in step.text for marker in markers) for step in steps
            ):
                raise ValidationError(
                    f"workflow {filename} job {job_name} has no step containing {markers}"
                )

    for (filename, job_name), contract in WORKFLOW_JOB_CONTRACTS.items():
        markers = contract.marker_order
        if markers is None:
            continue
        job = jobs_by_workflow[filename].get(job_name)
        if job is None:
            raise ValidationError(f"workflow {filename} is missing job {job_name}")
        active = _enabled_workflow_job_text(job)
        missing = [marker for marker in markers if marker not in active]
        if missing:
            raise ValidationError(
                f"workflow {filename} job {job_name} is missing ordered marker(s): {missing}"
            )
        for first, second in zip(markers, markers[1:]):
            if active.index(first) >= active.index(second):
                raise ValidationError(
                    f"workflow {filename} job {job_name} must place "
                    f"{first} before {second}"
                )

    for (filename, job_name), contract in WORKFLOW_JOB_CONTRACTS.items():
        required_needs = contract.required_needs
        if required_needs is None:
            continue
        job = jobs_by_workflow[filename].get(job_name)
        if job is None:
            raise ValidationError(f"workflow {filename} is missing job {job_name}")
        actual_needs = _workflow_job_needs(job, filename, job_name)
        if actual_needs != required_needs:
            raise ValidationError(
                f"workflow {filename} job {job_name} needs {sorted(actual_needs)}, "
                f"expected {sorted(required_needs)}"
            )

    for (filename, job_name), contract in WORKFLOW_JOB_CONTRACTS.items():
        target = contract.reusable_call
        if target is None:
            continue
        job = jobs_by_workflow[filename].get(job_name)
        if job is None:
            raise ValidationError(f"workflow {filename} is missing reusable job {job_name}")
        if f"uses: {target}" not in _enabled_workflow_job_text(job):
            raise ValidationError(
                f"workflow {filename} job {job_name} is missing reusable call {target}"
            )
        callee = Path(target).name
        if "workflow_call" not in top_level_yaml_mapping_keys(texts[callee], "on"):
            raise ValidationError(
                f"workflow {callee} is missing workflow_call for release job {job_name}"
            )


def validate_release_checklist(root: Path) -> None:
    checklist = (root / "RELEASE_CHECKLIST.md").read_text(encoding="utf-8")
    order = " -> ".join(f"`{name}`" for name in RUST_PUBLISH_ORDER)
    if order not in checklist:
        raise ValidationError("RELEASE_CHECKLIST.md does not contain canonical Rust order")
    for marker in ("local prepublish", "registry-dependent", "mdstream_flutter"):
        if marker not in checklist:
            raise ValidationError(
                f"RELEASE_CHECKLIST.md is missing release marker {marker!r}"
            )


def validate_documentation_contract(root: Path) -> None:
    readme = (root / "README.md").read_text(encoding="utf-8")
    paths = [root / "README.md"]
    for filename in REQUIRED_DOCUMENTS:
        path = root / "docs" / filename
        if not path.is_file() or not path.read_text(encoding="utf-8").strip():
            raise ValidationError(f"required documentation is missing: docs/{filename}")
        if f"docs/{filename}" not in readme:
            raise ValidationError(f"README.md does not link docs/{filename}")
        paths.append(path)

    for path in paths:
        text = path.read_text(encoding="utf-8")
        for target in re.findall(r"\[[^\]]*\]\(([^)]+)\)", text):
            if target.startswith(("http://", "https://", "mailto:", "#")):
                continue
            relative = target.split("#", 1)[0]
            if not relative:
                continue
            destination = (path.parent / relative).resolve()
            if not destination.exists():
                raise ValidationError(
                    f"broken Markdown link in {path.relative_to(root)}: {target}"
                )


def validate_example_catalog(root: Path) -> None:
    catalog_path = root / "docs" / "EXAMPLES.md"
    readme_path = root / "README.md"
    try:
        catalog = catalog_path.read_text(encoding="utf-8")
        readme = readme_path.read_text(encoding="utf-8")
    except OSError as error:
        raise ValidationError(f"failed to read example catalog: {error}") from error

    for contract in EXAMPLE_CONTRACTS:
        opening = f"<!-- example:{contract.identifier} -->"
        closing = "<!-- /example -->"
        if catalog.count(opening) != 1:
            raise ValidationError(
                f"example {contract.identifier} must have exactly one catalog entry"
            )
        start = catalog.index(opening) + len(opening)
        end = catalog.find(closing, start)
        if end < 0:
            raise ValidationError(
                f"example {contract.identifier} is missing its closing marker"
            )
        section = catalog[start:end]
        required_markers = {
            "Role": ("- Role:", contract.role),
            "Source": ("- Source:", f"](../{contract.source_path})"),
            "Prerequisites": (
                "- Prerequisites:",
                contract.prerequisite_marker,
            ),
            "Run": ("- Run:", f"`{contract.command}`"),
            "Expect": ("- Expect:", contract.expected_marker),
            "Next": ("- Next:", f"]({contract.next_link})"),
        }
        for field, markers in required_markers.items():
            if any(marker not in section for marker in markers):
                raise ValidationError(
                    f"example {contract.identifier} is missing or invalid {field}"
                )
        source = root / contract.source_path
        if not source.is_file():
            raise ValidationError(
                f"example {contract.identifier} source does not exist: "
                f"{contract.source_path}"
            )
        root_link = f"docs/EXAMPLES.md#{contract.identifier}"
        if root_link not in readme:
            raise ValidationError(
                f"README.md does not map example {contract.identifier} to {root_link}"
            )


def validate_static_contract(root: Path = ROOT) -> ReleaseContract:
    rust_packages = load_rust_packages(root)
    versions = {name: package.version for name, package in rust_packages.items()}
    typescript = json.loads(
        (root / "bindings" / "typescript" / "package.json").read_text(
            encoding="utf-8"
        )
    )
    versions["npm:@mdstream/core"] = str(typescript.get("version", ""))
    versions["dart:mdstream"] = top_level_yaml_scalar(
        root / "bindings" / "dart" / "pubspec.yaml", "version"
    )
    versions["flutter:mdstream_flutter"] = top_level_yaml_scalar(
        root / "bindings" / "flutter" / "pubspec.yaml", "version"
    )
    version = validate_versions(versions)
    validate_package_changelog(root / "bindings" / "dart" / "CHANGELOG.md", version)
    validate_package_changelog(
        root / "bindings" / "flutter" / "CHANGELOG.md", version
    )
    validate_flutter_version_metadata(root, version)
    validate_workspace_inventory(root)
    validate_rust_topology(RUST_PUBLISH_ORDER, rust_packages)
    validate_internal_dependency_versions(
        rust_packages,
        set(RUST_PUBLISH_ORDER),
        version,
    )
    validate_lock_versions(root, versions)
    validate_pub_lock_sources(root)

    flutter_manifest = (
        root / "bindings" / "flutter" / "pubspec.yaml"
    ).read_text(encoding="utf-8")
    if not re.search(
        rf"^  mdstream:\s+\^{re.escape(version)}\s*$",
        flutter_manifest,
        flags=re.MULTILINE,
    ):
        raise ValidationError(
            f"mdstream_flutter must depend on mdstream ^{version}"
        )
    validate_workflow_contract(root)
    validate_release_checklist(root)
    validate_documentation_contract(root)
    validate_example_catalog(root)
    return ReleaseContract(version, RUST_PUBLISH_ORDER)


def validate_inventory(
    label: str,
    actual: set[str],
    *,
    required: set[str],
    forbidden_prefixes: Sequence[str] = FORBIDDEN_PACKAGE_PREFIXES,
) -> None:
    missing = sorted(required - actual)
    if missing:
        raise ValidationError(f"{label} package is missing {', '.join(missing)}")
    forbidden = sorted(
        path
        for path in actual
        if any(path == prefix.rstrip("/") or path.startswith(prefix) for prefix in forbidden_prefixes)
        or "/node_modules/" in f"/{path}"
        or "/__pycache__/" in f"/{path}"
    )
    if forbidden:
        raise ValidationError(
            f"{label} package contains forbidden path {forbidden[0]}"
        )


def pubspec_has_path_dependency(text: str) -> bool:
    return re.search(r"(?m)^\s+path\s*:", text) is not None


def top_level_yaml_mapping_keys(text: str, field: str) -> set[str]:
    lines = text.splitlines()
    for index, line in enumerate(lines):
        if line == f"{field}:":
            keys: set[str] = set()
            for child in lines[index + 1 :]:
                if child and not child[0].isspace() and not child.lstrip().startswith("#"):
                    break
                match = re.match(r"^  ([A-Za-z0-9_-]+):", child)
                if match is not None:
                    keys.add(match.group(1))
            return keys
    return set()


def _run(
    command: Sequence[str],
    *,
    cwd: Path,
    capture: bool = True,
) -> subprocess.CompletedProcess[str]:
    print("+", " ".join(command), flush=True)
    try:
        return subprocess.run(
            list(command),
            cwd=cwd,
            check=True,
            text=True,
            capture_output=capture,
        )
    except FileNotFoundError as error:
        raise ValidationError(f"required tool not found: {command[0]}") from error
    except subprocess.CalledProcessError as error:
        detail = (error.stderr or error.stdout or "").strip()
        raise ValidationError(
            f"command failed ({error.returncode}): {' '.join(command)}"
            + (f"\n{detail}" if detail else "")
        ) from error


def _cargo_package_command(name: str) -> tuple[str, ...]:
    if name == "mdstream-merman":
        return (
            "cargo",
            "+1.95.0",
            "package",
            "--manifest-path",
            str(RUST_MANIFESTS[name]),
            "--locked",
        )
    return ("cargo", "package", "-p", name, "--locked")


def verify_rust_inventories(root: Path, names: Iterable[str]) -> None:
    for name in names:
        command = (*_cargo_package_command(name), "--list", "--allow-dirty")
        result = _run(command, cwd=root)
        paths = {line.strip() for line in result.stdout.splitlines() if line.strip()}
        validate_inventory(
            f"crate {name}",
            paths,
            required=RUST_REQUIRED_FILES[name],
        )


def _single_archive(directory: Path, pattern: str, label: str) -> Path:
    archives = sorted(directory.glob(pattern))
    if len(archives) != 1:
        raise ValidationError(
            f"{label} must produce exactly one archive, found {len(archives)}"
        )
    return archives[0]


def verify_npm_inventory(root: Path) -> None:
    with tempfile.TemporaryDirectory(prefix="mdstream-npm-package-") as temporary:
        _run(
            (
                "pnpm",
                "--filter",
                "@mdstream/core",
                "pack",
                "--pack-destination",
                temporary,
            ),
            cwd=root,
        )
        archive = _single_archive(Path(temporary), "*.tgz", "npm pack")
        verify_existing_archive(root, "npm", archive)


def _archive_contents(
    path: Path,
    *,
    prefix: str | None = None,
    reject_native: bool = False,
    reject_unlisted_flutter_native: bool = False,
) -> ArchiveContents:
    paths: set[str] = set()
    manifests: dict[str, bytes] = {}
    validation_error: ValidationError | None = None

    def summarize(member: ArchiveMember, chunks: Iterator[bytes]) -> None:
        nonlocal validation_error
        if validation_error is not None:
            return
        pure = PurePosixPath(member.name)
        relative = member.name
        if prefix is not None:
            if not pure.parts or pure.parts[0] != prefix:
                validation_error = ValidationError(
                    f"archive member is outside required {prefix}/ root: {member.name}"
                )
                return
            if len(pure.parts) == 1:
                return
            relative = str(PurePosixPath(*pure.parts[1:]))
        if not member.is_file:
            return

        # Full names are unique, and removing one fixed prefix is injective.
        paths.add(relative)
        if relative in {"CHANGELOG.md", "package.json", "pubspec.yaml"}:
            data = b"".join(chunks)
            leading = data[:NATIVE_MAGIC_PREFIX_BYTES]
        else:
            leading_bytes = bytearray()
            for chunk in chunks:
                leading_bytes.extend(
                    chunk[: NATIVE_MAGIC_PREFIX_BYTES - len(leading_bytes)]
                )
                if len(leading_bytes) == NATIVE_MAGIC_PREFIX_BYTES:
                    break
            leading = bytes(leading_bytes)
            data = None
        native_like = is_native_like_artifact(relative, leading)
        if reject_native and native_like:
            validation_error = ValidationError(
                f"archive contains native binary magic or extension: {relative}"
            )
            return
        if (
            reject_unlisted_flutter_native
            and not is_canonical_flutter_native_path(relative)
            and (is_reserved_flutter_native_path(relative) or native_like)
        ):
            validation_error = ValidationError(
                "archive contains a native-like file outside canonical native "
                f"inventory: {relative}"
            )
            return
        if data is not None:
            manifests[relative] = data

    try:
        visit_archive(path, summarize)
    except ArchivePolicyError as error:
        raise ValidationError(str(error)) from error
    except UnicodeDecodeError as error:
        raise ValidationError(f"failed to inspect archive {path}: {error}") from error
    if validation_error is not None:
        raise validation_error
    return ArchiveContents(paths, manifests)


def _archive_files(
    path: Path,
    *,
    reject_native: bool = False,
) -> tuple[set[str], str]:
    contents = _archive_contents(path, reject_native=reject_native)
    try:
        pubspec = contents.manifests.get("pubspec.yaml", b"").decode("utf-8")
    except UnicodeDecodeError as error:
        raise ValidationError(f"failed to decode packed pubspec.yaml: {error}") from error
    return contents.paths, pubspec


def _archive_text(contents: ArchiveContents, path: str, label: str) -> str:
    try:
        return contents.manifests[path].decode("utf-8")
    except KeyError as error:
        raise ValidationError(f"{label} archive has no {path}") from error
    except UnicodeDecodeError as error:
        raise ValidationError(f"failed to decode packed {path}: {error}") from error


def _validate_packed_changelog(
    contents: ArchiveContents,
    version: str,
    label: str,
) -> None:
    changelog = _archive_text(contents, "CHANGELOG.md", label)
    try:
        first_version = first_release_version(changelog)
        extract_release_notes(changelog, version)
    except ReleaseNotesError as error:
        raise ValidationError(f"invalid packed {label} CHANGELOG.md: {error}") from error
    if first_version != version:
        raise ValidationError(
            f"packed {label} CHANGELOG.md starts at {first_version}, expected {version}"
        )


def archive_file_fingerprints(path: Path) -> dict[str, ArchiveFileFingerprint]:
    fingerprints: dict[str, ArchiveFileFingerprint] = {}

    def fingerprint(member: ArchiveMember, chunks: Iterator[bytes]) -> None:
        if not member.is_file:
            return
        digest = hashlib.sha256()
        size = 0
        for chunk in chunks:
            digest.update(chunk)
            size += len(chunk)
        fingerprints[member.name] = ArchiveFileFingerprint(size, digest.hexdigest())

    try:
        visit_archive(path, fingerprint)
    except ArchivePolicyError as error:
        raise ValidationError(str(error)) from error
    return fingerprints


def compare_archive_file_contents(
    expected: Path,
    candidate: Path,
    *,
    candidate_label: str = "repacked archive",
) -> None:
    expected_files = archive_file_fingerprints(expected)
    candidate_files = archive_file_fingerprints(candidate)
    missing = sorted(expected_files.keys() - candidate_files.keys())
    if missing:
        raise ValidationError(f"{candidate_label} is missing file {missing[0]}")
    extra = sorted(candidate_files.keys() - expected_files.keys())
    if extra:
        raise ValidationError(f"{candidate_label} contains extra file {extra[0]}")
    changed = sorted(
        path
        for path, fingerprint in expected_files.items()
        if candidate_files[path] != fingerprint
    )
    if changed:
        raise ValidationError(f"{candidate_label} changed file content {changed[0]}")


def _registry_metadata_url(registry: str, package: str, version: str) -> str | None:
    encoded_package = quote(package, safe="@")
    encoded_version = quote(version, safe="")
    if registry == "crates.io":
        return (
            f"https://crates.io/api/v1/crates/{encoded_package}/{encoded_version}"
        )
    if registry == "npm":
        return f"https://registry.npmjs.org/{encoded_package}/{encoded_version}"
    if registry == "pub.dev":
        return (
            f"https://pub.dev/api/packages/{encoded_package}/versions/{encoded_version}"
        )
    raise ValidationError(f"unsupported release registry {registry!r}")


def _decode_integrity(value: str) -> tuple[str, bytes]:
    try:
        algorithm, encoded = value.split("-", 1)
        digest = base64.b64decode(encoded, validate=True)
    except (ValueError, binascii.Error) as error:
        raise ValidationError(f"invalid npm dist.integrity {value!r}") from error
    if algorithm not in hashlib.algorithms_available or not digest:
        raise ValidationError(f"unsupported npm integrity algorithm {algorithm!r}")
    return algorithm, digest


def registry_archive_descriptor(
    registry: str,
    package: str,
    version: str,
    metadata: object | None,
) -> RegistryArchiveDescriptor:
    if registry == "crates.io":
        if not isinstance(metadata, dict) or not isinstance(
            metadata.get("version"), dict
        ):
            raise ValidationError("crates.io returned invalid version metadata")
        release = metadata["version"]
        if release.get("crate") != package or release.get("num") != version:
            raise ValidationError(
                f"crates.io metadata identity does not match {package} {version}"
            )
        if release.get("yanked") is not False:
            raise ValidationError(f"crates.io version {package} {version} is yanked")
        checksum = release.get("checksum")
        if not isinstance(checksum, str) or re.fullmatch(
            r"[0-9a-fA-F]{64}", checksum
        ) is None:
            raise ValidationError("crates.io metadata has no valid checksum")
        encoded_package = quote(package, safe="")
        encoded_version = quote(version, safe="")
        return RegistryArchiveDescriptor(
            f"https://crates.io/api/v1/crates/{encoded_package}/"
            f"{encoded_version}/download",
            "sha256",
            bytes.fromhex(checksum),
        )
    if not isinstance(metadata, dict):
        raise ValidationError(f"{registry} returned non-object package metadata")
    if metadata.get("name", package) != package or metadata.get("version") != version:
        raise ValidationError(
            f"{registry} metadata identity does not match {package} {version}"
        )
    if registry == "npm":
        dist = metadata.get("dist")
        if not isinstance(dist, dict) or not isinstance(dist.get("tarball"), str):
            raise ValidationError("npm metadata has no dist.tarball URL")
        integrity = dist.get("integrity")
        if isinstance(integrity, str):
            algorithm, checksum = _decode_integrity(integrity)
        else:
            shasum = dist.get("shasum")
            if not isinstance(shasum, str) or re.fullmatch(r"[0-9a-fA-F]{40}", shasum) is None:
                raise ValidationError("npm metadata has no valid integrity or shasum")
            algorithm, checksum = "sha1", bytes.fromhex(shasum)
        descriptor = RegistryArchiveDescriptor(
            dist["tarball"],
            algorithm,
            checksum,
        )
    elif registry == "pub.dev":
        retracted = metadata.get("retracted", False)
        if not isinstance(retracted, bool):
            raise ValidationError("pub.dev metadata has invalid retracted state")
        if retracted:
            raise ValidationError(f"pub.dev version {package} {version} is retracted")
        archive_url = metadata.get("archive_url")
        if not isinstance(archive_url, str):
            raise ValidationError("pub.dev metadata has no archive_url")
        archive_sha256 = metadata.get("archive_sha256")
        if archive_sha256 is None:
            descriptor = RegistryArchiveDescriptor(archive_url)
        elif isinstance(archive_sha256, str) and re.fullmatch(
            r"[0-9a-fA-F]{64}", archive_sha256
        ):
            descriptor = RegistryArchiveDescriptor(
                archive_url,
                "sha256",
                bytes.fromhex(archive_sha256),
            )
        else:
            raise ValidationError("pub.dev metadata has an invalid archive_sha256")
    else:
        raise ValidationError(f"unsupported release registry {registry!r}")
    parsed = urlparse(descriptor.url)
    if (
        parsed.scheme != "https"
        or not parsed.hostname
        or parsed.username is not None
        or parsed.password is not None
        or parsed.port not in (None, 443)
    ):
        raise ValidationError(f"{registry} returned an unsafe archive URL")
    if registry == "npm" and parsed.hostname != "registry.npmjs.org":
        raise ValidationError("npm archive URL is outside registry.npmjs.org")
    return descriptor


def _curl_to_path(
    url: str,
    destination: Path,
    *,
    connect_timeout: int,
    max_time: int,
    max_bytes: int,
) -> None:
    command = (
        "curl",
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--user-agent",
        REGISTRY_USER_AGENT,
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--connect-timeout",
        str(connect_timeout),
        "--max-time",
        str(max_time),
        "--max-filesize",
        str(max_bytes),
        "--output",
        str(destination),
        url,
    )
    _run(command, cwd=Path.cwd(), capture=False)
    try:
        size = destination.stat().st_size
    except OSError as error:
        raise ValidationError(f"registry download did not create {destination}: {error}") from error
    if size > max_bytes:
        raise ValidationError(
            f"registry download exceeds {max_bytes}-byte ceiling: {size}"
        )


def _registry_metadata(registry: str, package: str, version: str, path: Path) -> object:
    url = _registry_metadata_url(registry, package, version)
    if url is None:
        return None
    max_time = 20 if registry == "pub.dev" else 30
    _curl_to_path(
        url,
        path,
        connect_timeout=5 if registry == "pub.dev" else 30,
        max_time=max_time,
        max_bytes=8 * 1024 * 1024,
    )
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValidationError(f"invalid {registry} package metadata: {error}") from error


def verify_registry_archive(
    registry: str,
    package: str,
    version: str,
    expected: Path,
) -> None:
    if re.fullmatch(r"\d+\.\d+\.\d+", version) is None:
        raise ValidationError(f"invalid registry artifact version {version!r}")
    if not expected.is_file():
        raise ValidationError(f"expected producer archive does not exist: {expected}")
    with tempfile.TemporaryDirectory(prefix="mdstream-registry-artifact-") as temporary:
        root = Path(temporary)
        metadata = _registry_metadata(
            registry,
            package,
            version,
            root / "metadata.json",
        )
        descriptor = registry_archive_descriptor(registry, package, version, metadata)
        downloaded = root / "registry-archive.tar.gz"
        _curl_to_path(
            descriptor.url,
            downloaded,
            connect_timeout=30,
            max_time=120,
            max_bytes=DEFAULT_ARCHIVE_LIMITS.max_compressed_bytes,
        )
        if descriptor.checksum_algorithm is not None:
            digest = hashlib.new(descriptor.checksum_algorithm)
            with downloaded.open("rb") as handle:
                for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                    digest.update(chunk)
            if digest.digest() != descriptor.checksum:
                raise ValidationError(
                    f"{registry} archive checksum does not match registry metadata"
                )
        compare_archive_file_contents(
            expected,
            downloaded,
            candidate_label=f"{registry} archive",
        )


def _binding_budget(root: Path, artifact: str) -> tuple[int, set[str]]:
    path = root / "bindings" / "budgets.json"
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
        artifacts = document["artifacts"]
        entry = next(item for item in artifacts if item.get("artifact") == artifact)
        ceiling = entry["ceiling_bytes"]
        forbidden = document["policy"]["forbidden_default_dependencies"]
    except (OSError, json.JSONDecodeError, KeyError, StopIteration, TypeError) as error:
        raise ValidationError(f"invalid binding budget contract {path}: {error}") from error
    if not isinstance(ceiling, int) or ceiling <= 0:
        raise ValidationError(f"{artifact} ceiling_bytes must be a positive integer")
    if not isinstance(forbidden, list) or not all(
        isinstance(value, str) for value in forbidden
    ):
        raise ValidationError("forbidden_default_dependencies must be strings")
    return ceiling, {value.lower() for value in forbidden}


def _verify_archive_budget(root: Path, archive: Path, artifact: str) -> set[str]:
    ceiling, forbidden = _binding_budget(root, artifact)
    try:
        size = archive.stat().st_size
    except OSError as error:
        raise ValidationError(f"failed to stat package archive {archive}: {error}") from error
    if not archive.is_file():
        raise ValidationError(f"package archive does not exist: {archive}")
    if size > ceiling:
        raise ValidationError(
            f"{artifact} archive is {size} bytes; ceiling is {ceiling} bytes"
        )
    return forbidden


def verify_existing_archive(root: Path, ecosystem: str, archive: Path) -> None:
    archive = Path(archive)
    if ecosystem == "npm":
        expected_version = expected_release_version(root)
        forbidden = _verify_archive_budget(root, archive, "npm_packed")
        contents = _archive_contents(
            archive,
            prefix="package",
            reject_native=True,
        )
        validate_inventory(
            "npm @mdstream/core",
            contents.paths,
            required=NPM_REQUIRED_FILES,
        )
        try:
            manifest = json.loads(contents.manifests["package.json"])
        except (KeyError, UnicodeDecodeError, json.JSONDecodeError) as error:
            raise ValidationError(f"invalid packed npm package.json: {error}") from error
        if not isinstance(manifest, dict) or manifest.get("name") != "@mdstream/core":
            raise ValidationError("packed npm package name must be @mdstream/core")
        if manifest.get("version") != expected_version:
            raise ValidationError(
                "packed npm package version must be "
                f"{expected_version}, got {manifest.get('version')!r}"
            )
        dependencies: list[tuple[str, str]] = []
        for field in ("dependencies", "optionalDependencies", "peerDependencies"):
            values = manifest.get(field, {})
            if not isinstance(values, dict) or not all(
                isinstance(name, str) and isinstance(requirement, str)
                for name, requirement in values.items()
            ):
                raise ValidationError(f"packed npm {field} must be a string map")
            dependencies.extend(values.items())
        local = sorted(
            name
            for name, requirement in dependencies
            if requirement.startswith(("file:", "link:", "workspace:"))
        )
        if local:
            raise ValidationError(
                f"npm package contains local dependency {local[0]}"
            )
        forbidden_present = sorted(
            name for name, _ in dependencies if name.lower() in forbidden
        )
        if forbidden_present:
            raise ValidationError(
                f"npm package contains forbidden dependency {forbidden_present[0]}"
            )
        return
    if ecosystem == "dart":
        expected_version = expected_release_version(root)
        forbidden = _verify_archive_budget(root, archive, "dart_packed")
        contents = _archive_contents(archive, reject_native=True)
        paths = contents.paths
        pubspec = _archive_text(contents, "pubspec.yaml", "Dart mdstream")
        validate_inventory(
            "Dart mdstream",
            paths,
            required=DART_REQUIRED_FILES,
            forbidden_prefixes=PUB_REPOSITORY_ONLY_PREFIXES,
        )
        if not pubspec:
            raise ValidationError("Dart mdstream archive has no pubspec.yaml")
        name = top_level_yaml_scalar_text(pubspec, "name", "packed Dart pubspec.yaml")
        version = top_level_yaml_scalar_text(
            pubspec, "version", "packed Dart pubspec.yaml"
        )
        if name != "mdstream":
            raise ValidationError(
                f"packed Dart package name must be mdstream, got {name!r}"
            )
        if version != expected_version:
            raise ValidationError(
                f"packed Dart package version must be {expected_version}, got {version!r}"
            )
        _validate_packed_changelog(contents, expected_version, "Dart mdstream")
        if pubspec_has_path_dependency(pubspec):
            raise ValidationError("Dart mdstream archive contains a path dependency")
        dependencies = top_level_yaml_mapping_keys(pubspec, "dependencies")
        if dependencies != {"ffi"}:
            raise ValidationError(
                "standalone Dart production dependencies must contain only ffi"
            )
        forbidden_present = sorted(
            name for name in dependencies if name.lower() in forbidden
        )
        if forbidden_present:
            raise ValidationError(
                f"Dart package contains forbidden dependency {forbidden_present[0]}"
            )
        return
    if ecosystem == "flutter":
        expected_version = expected_release_version(root)
        contents = _archive_contents(
            archive,
            reject_unlisted_flutter_native=True,
        )
        paths = contents.paths
        pubspec = _archive_text(
            contents,
            "pubspec.yaml",
            "Flutter mdstream_flutter",
        )
        validate_inventory(
            "Flutter mdstream_flutter",
            paths,
            required=FLUTTER_REQUIRED_FILES,
            forbidden_prefixes=PUB_REPOSITORY_ONLY_PREFIXES,
        )
        if not pubspec:
            raise ValidationError("Flutter mdstream_flutter archive has no pubspec.yaml")
        name = top_level_yaml_scalar_text(
            pubspec, "name", "packed Flutter pubspec.yaml"
        )
        version = top_level_yaml_scalar_text(
            pubspec, "version", "packed Flutter pubspec.yaml"
        )
        if name != "mdstream_flutter":
            raise ValidationError(
                "packed Flutter package name must be mdstream_flutter, "
                f"got {name!r}"
            )
        if version != expected_version:
            raise ValidationError(
                "packed Flutter package version must be "
                f"{expected_version}, got {version!r}"
            )
        _validate_packed_changelog(
            contents,
            expected_version,
            "Flutter mdstream_flutter",
        )
        if pubspec_has_path_dependency(pubspec):
            raise ValidationError(
                "Flutter mdstream_flutter archive contains a path dependency"
            )
        dependencies = top_level_yaml_mapping_keys(pubspec, "dependencies")
        if dependencies != {"flutter", "mdstream"}:
            raise ValidationError(
                "Flutter production dependencies must contain only flutter and mdstream"
            )
        mdstream_requirement = top_level_yaml_mapping_scalar(
            pubspec,
            "dependencies",
            "mdstream",
            "packed Flutter pubspec.yaml",
        )
        if mdstream_requirement != f"^{expected_version}":
            raise ValidationError(
                "packed Flutter mdstream requirement must be "
                f"^{expected_version}, got {mdstream_requirement!r}"
            )
        return
    raise ValidationError(
        "existing archive verification supports only npm, dart, or flutter"
    )


def verify_pub_inventory(root: Path, package: str) -> None:
    if package == "dart":
        package_root = root / "bindings" / "dart"
        version = top_level_yaml_scalar(package_root / "pubspec.yaml", "version")
        filename = f"mdstream-{version}.tar.gz"
    elif package == "flutter":
        package_root = root / "bindings" / "flutter"
        version = top_level_yaml_scalar(package_root / "pubspec.yaml", "version")
        filename = f"mdstream_flutter-{version}.tar.gz"
    else:
        raise ValidationError(f"unknown Pub package {package}")
    with tempfile.TemporaryDirectory(prefix=f"mdstream-{package}-package-") as temporary:
        archive = Path(temporary) / filename
        _run(
            ("dart", "pub", "publish", f"--to-archive={archive}"),
            cwd=package_root,
            capture=False,
        )
        verify_existing_archive(root, package, archive)


def verify_local_packages(
    root: Path,
    ecosystems: set[str],
    archive: Path | None = None,
    compare_archive: Path | None = None,
) -> None:
    if archive is not None:
        if len(ecosystems) != 1:
            raise ValidationError(
                "--archive requires exactly one --ecosystem"
            )
        ecosystem = next(iter(ecosystems))
        verify_existing_archive(root, ecosystem, archive)
        if compare_archive is not None:
            if ecosystem not in {"dart", "flutter"}:
                raise ValidationError(
                    "--compare-archive supports only Dart or Flutter Pub archives"
                )
            verify_existing_archive(root, ecosystem, compare_archive)
            compare_archive_file_contents(archive, compare_archive)
        return
    if compare_archive is not None:
        raise ValidationError("--compare-archive requires --archive")
    if "rust" in ecosystems:
        verify_rust_inventories(root, RUST_PUBLISH_ORDER)
    if "npm" in ecosystems:
        verify_npm_inventory(root)
    if "dart" in ecosystems:
        verify_pub_inventory(root, "dart")
    if "flutter" in ecosystems:
        verify_pub_inventory(root, "flutter")


def verify_registry_package(root: Path, package: str) -> None:
    if package not in RUST_PUBLISH_ORDER:
        raise ValidationError(
            "registry phase currently accepts one Rust package from RUST_PUBLISH_ORDER"
        )
    _run(_cargo_package_command(package), cwd=root, capture=False)


def parse_ecosystems(values: Sequence[str] | None) -> set[str]:
    supported = {"rust", "npm", "dart", "flutter"}
    if not values or "all" in values:
        return supported
    selected = set(values)
    unknown = selected - supported
    if unknown:
        raise ValidationError(f"unknown ecosystem(s): {sorted(unknown)}")
    return selected


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--phase",
        choices=("static", "local", "registry"),
        default="local",
    )
    parser.add_argument(
        "--ecosystem",
        action="append",
        choices=("all", "rust", "npm", "dart", "flutter"),
    )
    parser.add_argument("--package")
    parser.add_argument("--archive", type=Path)
    parser.add_argument("--compare-archive", type=Path)
    parser.add_argument(
        "--compare-only",
        nargs=2,
        type=Path,
        metavar=("EXPECTED", "CANDIDATE"),
    )
    parser.add_argument(
        "--extract-only",
        nargs=2,
        type=Path,
        metavar=("ARCHIVE", "DESTINATION"),
    )
    parser.add_argument(
        "--compare-registry",
        choices=("crates.io", "npm", "pub.dev"),
    )
    parser.add_argument("--version")
    parser.add_argument("--print-rust-order", action="store_true")
    args = parser.parse_args()

    try:
        if args.print_rust_order:
            if any(
                (
                    args.extract_only is not None,
                    args.compare_only is not None,
                    args.compare_registry is not None,
                    args.version is not None,
                    args.package is not None,
                    args.archive is not None,
                    args.compare_archive is not None,
                    args.ecosystem is not None,
                )
            ):
                raise ValidationError(
                    "--print-rust-order cannot be combined with package verification options"
                )
            print("\n".join(RUST_PUBLISH_ORDER))
            return 0
        if args.extract_only is not None:
            if any(
                (
                    args.compare_only is not None,
                    args.compare_registry is not None,
                    args.version is not None,
                    args.package is not None,
                    args.archive is not None,
                    args.compare_archive is not None,
                    args.ecosystem is not None,
                    args.print_rust_order,
                )
            ):
                raise ValidationError(
                    "--extract-only cannot be combined with package verification options"
                )
            try:
                extract_archive(*args.extract_only)
            except ArchivePolicyError as error:
                raise ValidationError(str(error)) from error
            print(
                json.dumps(
                    {
                        "schema": "mdstream.archive-extraction/1",
                        "result": "extracted",
                    },
                    indent=2,
                )
            )
            return 0
        if args.compare_only is not None:
            if any(
                (
                    args.compare_registry is not None,
                    args.version is not None,
                    args.package is not None,
                    args.archive is not None,
                    args.compare_archive is not None,
                    args.ecosystem is not None,
                )
            ):
                raise ValidationError(
                    "--compare-only cannot be combined with package verification options"
                )
            compare_archive_file_contents(*args.compare_only)
            print(
                json.dumps(
                    {
                        "schema": "mdstream.archive-content-comparison/1",
                        "result": "equal",
                    },
                    indent=2,
                )
            )
            return 0
        if args.compare_registry is not None:
            if args.package is None or args.version is None or args.archive is None:
                raise ValidationError(
                    "--compare-registry requires --package, --version, and --archive"
                )
            if args.compare_archive is not None or args.ecosystem is not None:
                raise ValidationError(
                    "--compare-registry cannot be combined with local package options"
                )
            verify_registry_archive(
                args.compare_registry,
                args.package,
                args.version,
                args.archive,
            )
            print(
                json.dumps(
                    {
                        "schema": "mdstream.registry-artifact-comparison/1",
                        "registry": args.compare_registry,
                        "package": args.package,
                        "version": args.version,
                        "result": "equal",
                    },
                    indent=2,
                )
            )
            return 0
        contract = validate_static_contract(ROOT)
        if args.phase == "local":
            verify_local_packages(
                ROOT,
                parse_ecosystems(args.ecosystem),
                args.archive,
                args.compare_archive,
            )
        elif args.phase == "registry":
            if args.archive is not None or args.compare_archive is not None:
                raise ValidationError(
                    "--archive and --compare-archive are supported only in local phase"
                )
            if args.package is None:
                raise ValidationError("registry phase requires --package")
            verify_registry_package(ROOT, args.package)
        elif args.archive is not None or args.compare_archive is not None:
            raise ValidationError(
                "--archive and --compare-archive are supported only in local phase"
            )
        print(
            json.dumps(
                {
                    "schema": "mdstream.package-verification/1",
                    "phase": args.phase,
                    "version": contract.version,
                    "rust_publish_order": contract.rust_publish_order,
                },
                indent=2,
            )
        )
    except ValidationError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
