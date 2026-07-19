#!/usr/bin/env python3
"""Validate mdstream release versions, dependency order, and package contents."""

from __future__ import annotations

import argparse
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


SCRIPT_ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_ROOT))

from archive_policy import ArchiveMember, ArchivePolicyError, visit_archive  # noqa: E402


ROOT = Path(__file__).resolve().parents[1]
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
RUST_REQUIRED_FILES["mdstream-ffi"] |= {"README.md", "include/mdstream.h"}
RUST_REQUIRED_FILES["mdstream-merman"] |= {"README.md"}

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
    "README.md",
    "LICENSE",
    "lib/mdstream.dart",
    "lib/src/ffi.dart",
    "lib/src/reducer_handle.dart",
}
FLUTTER_REQUIRED_FILES = {
    "pubspec.yaml",
    "README.md",
    "LICENSE",
    "lib/mdstream_flutter.dart",
    "android/build.gradle",
    "ios/mdstream_flutter.podspec",
    "linux/CMakeLists.txt",
    "macos/mdstream_flutter.podspec",
    "windows/CMakeLists.txt",
}
FORBIDDEN_PACKAGE_PREFIXES = (
    ".git/",
    ".dart_tool/",
    "docs/plans/",
    "node_modules/",
    "repo-ref/",
    "target/",
)
NATIVE_FILE_SUFFIXES = (".a", ".dylib", ".dll", ".lib", ".node", ".so")
NATIVE_MAGICS = (
    b"\x7fELF",
    b"\xfe\xed\xfa\xce",
    b"\xfe\xed\xfa\xcf",
    b"\xce\xfa\xed\xfe",
    b"\xcf\xfa\xed\xfe",
    b"\xca\xfe\xba\xbe",
    b"\xbe\xba\xfe\xca",
    b"MZ",
    b"!<arch>\n",
)
REQUIRED_DOCUMENTS = (
    "ARCHITECTURE.md",
    "STATE.md",
    "EXTENSIONS.md",
    "ADAPTERS.md",
    "COMPATIBILITY.md",
    "PERFORMANCE.md",
    "USAGE.md",
    "ROADMAP.md",
)

WASM_TOOLS_MARKER = "tool: wasm-tools@1.253.0"


@dataclass(frozen=True)
class WorkflowJobContract:
    run_markers: tuple[str, ...] = ()
    job_markers: tuple[str, ...] = ()
    step_marker_groups: tuple[tuple[str, ...], ...] = ()
    marker_order: tuple[str, str] | None = None
    required_needs: frozenset[str] | None = None
    reusable_call: str | None = None


WORKFLOW_JOB_CONTRACTS: Mapping[
    tuple[str, str], WorkflowJobContract
] = MappingProxyType(
    {
        ("ci.yml", "core-msrv"): WorkflowJobContract(
            job_markers=("dtolnay/rust-toolchain@1.85.0",),
        ),
        ("ci.yml", "workspace-msrv"): WorkflowJobContract(
            job_markers=("dtolnay/rust-toolchain@1.88.0",),
        ),
        ("ci.yml", "release-contract"): WorkflowJobContract(
            run_markers=(
                "scripts/verify-packages.py --phase static",
                "python3 -m unittest scripts/test_verify_packages.py",
            ),
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
        ),
        ("ci.yml", "merman"): WorkflowJobContract(
            run_markers=(
                "cargo +1.95.0 nextest run --manifest-path "
                "mdstream-merman/Cargo.toml --all-features",
            ),
            job_markers=("dtolnay/rust-toolchain@1.95.0",),
        ),
        ("ci.yml", "web"): WorkflowJobContract(
            run_markers=(
                "cargo check -p mdstream-wasm --target "
                "wasm32-unknown-unknown --all-features",
                "wasm-pack test --node mdstream-wasm",
                "pnpm install --frozen-lockfile",
                "pnpm -r test",
                "pnpm -r build",
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
        ),
        ("ci.yml", "dart"): WorkflowJobContract(
            run_markers=("dart analyze", "dart test"),
            job_markers=("sdk: 3.8.1", "flutter-version: 3.32.1"),
        ),
        ("flutter-platforms.yml", "linux"): WorkflowJobContract(
            run_markers=("flutter analyze", "flutter test"),
            job_markers=("flutter-version: 3.32.1",),
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
        ("release.yml", "publish-rust"): WorkflowJobContract(
            run_markers=(
                "scripts/verify-packages.py --print-rust-order",
                "scripts/verify-packages.py --phase registry --package",
                "timeout 30s cargo info --registry crates-io",
            ),
            required_needs=frozenset(("validate", "quality")),
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
            run_markers=("npm publish",),
            required_needs=frozenset(("validate", "quality", "build-npm")),
        ),
        ("release.yml", "build-dart"): WorkflowJobContract(
            required_needs=frozenset(("validate",)),
        ),
        ("release.yml", "publish-dart"): WorkflowJobContract(
            run_markers=("dart pub publish",),
            required_needs=frozenset(("validate", "quality", "build-dart")),
        ),
        ("release.yml", "publish-flutter"): WorkflowJobContract(
            run_markers=(
                "dart pub publish",
                "https://pub.dev/api/packages/mdstream_flutter",
            ),
            required_needs=frozenset(
                ("validate", "quality", "publish-dart", "flutter-platforms")
            ),
        ),
        ("release.yml", "quality"): WorkflowJobContract(
            reusable_call="./.github/workflows/ci.yml",
        ),
        ("release.yml", "flutter-platforms"): WorkflowJobContract(
            reusable_call="./.github/workflows/flutter-platforms.yml",
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
    match = re.search(
        rf"^(?!\s){re.escape(field)}:\s*['\"]?([^'\"\s#]+)",
        text,
        flags=re.MULTILINE,
    )
    if match is None:
        raise ValidationError(f"{path} has no top-level {field}")
    return match.group(1)


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


def _workflow_run_commands(block: str) -> str:
    lines = block.splitlines()
    commands: list[str] = []
    index = 0
    while index < len(lines):
        line = lines[index]
        if line.lstrip().startswith("#"):
            index += 1
            continue
        match = re.match(r"^(\s*)run:\s*(.*)$", line)
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
    return "\n".join(commands)


def _workflow_steps(block: str) -> tuple[str, ...]:
    lines = block.splitlines()
    starts = [
        index
        for index, line in enumerate(lines)
        if re.match(r"^      - (?:name|uses|run):", line) is not None
    ]
    steps: list[str] = []
    for position, start in enumerate(starts):
        end = starts[position + 1] if position + 1 < len(starts) else len(lines)
        steps.append(_active_workflow_text("\n".join(lines[start:end])))
    return tuple(steps)


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
        active = _active_workflow_text(job)
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
        steps = _workflow_steps(job)
        for markers in marker_groups:
            if not any(all(marker in step for marker in markers) for step in steps):
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
        active = _active_workflow_text(job)
        first, second = markers
        missing = [marker for marker in markers if marker not in active]
        if missing:
            raise ValidationError(
                f"workflow {filename} job {job_name} is missing ordered marker(s): {missing}"
            )
        if active.index(first) >= active.index(second):
            raise ValidationError(
                f"workflow {filename} job {job_name} must place {first} before {second}"
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
        if f"uses: {target}" not in _active_workflow_text(job):
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
    validate_flutter_version_metadata(root, version)
    validate_workspace_inventory(root)
    validate_rust_topology(RUST_PUBLISH_ORDER, rust_packages)
    validate_internal_dependency_versions(
        rust_packages,
        set(RUST_PUBLISH_ORDER),
        version,
    )
    validate_lock_versions(root, versions)

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
        if relative in {"package.json", "pubspec.yaml"}:
            data = b"".join(chunks)
            leading = data[:8]
        else:
            leading_bytes = bytearray()
            for chunk in chunks:
                leading_bytes.extend(chunk[: 8 - len(leading_bytes)])
                if len(leading_bytes) == 8:
                    break
            leading = bytes(leading_bytes)
            data = None
        if reject_native and (
            _is_native_path(relative) or _has_native_magic(leading)
        ):
            validation_error = ValidationError(
                f"archive contains native binary magic or extension: {relative}"
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


def _is_native_path(path: str) -> bool:
    lower = path.lower()
    if any(part.endswith(".framework") for part in PurePosixPath(lower).parts):
        return True
    return lower.endswith(NATIVE_FILE_SUFFIXES) or ".so." in lower


def _has_native_magic(leading: bytes) -> bool:
    return leading.startswith(NATIVE_MAGICS)


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
        forbidden = _verify_archive_budget(root, archive, "dart_packed")
        paths, pubspec = _archive_files(archive, reject_native=True)
        validate_inventory("Dart mdstream", paths, required=DART_REQUIRED_FILES)
        if not pubspec:
            raise ValidationError("Dart mdstream archive has no pubspec.yaml")
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
        paths, pubspec = _archive_files(archive)
        validate_inventory(
            "Flutter mdstream_flutter",
            paths,
            required=FLUTTER_REQUIRED_FILES,
        )
        if not pubspec:
            raise ValidationError("Flutter mdstream_flutter archive has no pubspec.yaml")
        if pubspec_has_path_dependency(pubspec):
            raise ValidationError(
                "Flutter mdstream_flutter archive contains a path dependency"
            )
        dependencies = top_level_yaml_mapping_keys(pubspec, "dependencies")
        if dependencies != {"flutter", "mdstream"}:
            raise ValidationError(
                "Flutter production dependencies must contain only flutter and mdstream"
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
) -> None:
    if archive is not None:
        if len(ecosystems) != 1:
            raise ValidationError(
                "--archive requires exactly one --ecosystem"
            )
        ecosystem = next(iter(ecosystems))
        verify_existing_archive(root, ecosystem, archive)
        return
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
    parser.add_argument("--print-rust-order", action="store_true")
    args = parser.parse_args()

    if args.print_rust_order:
        print("\n".join(RUST_PUBLISH_ORDER))
        return 0
    try:
        contract = validate_static_contract(ROOT)
        if args.phase == "local":
            verify_local_packages(
                ROOT,
                parse_ecosystems(args.ecosystem),
                args.archive,
            )
        elif args.phase == "registry":
            if args.archive is not None:
                raise ValidationError("--archive is supported only in local phase")
            if args.package is None:
                raise ValidationError("registry phase requires --package")
            verify_registry_package(ROOT, args.package)
        elif args.archive is not None:
            raise ValidationError("--archive is supported only in local phase")
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
