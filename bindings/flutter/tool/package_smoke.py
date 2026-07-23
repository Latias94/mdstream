#!/usr/bin/env python3
"""Validate the Flutter release archive and run a bundled-library smoke app."""

from __future__ import annotations

import argparse
import json
import os
import plistlib
import re
import shutil
import subprocess
import sys
import tempfile
import time
from collections.abc import Callable
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import Iterator

from build_native import (
    HEADER_PATH,
    IOS_DEPLOYMENT_TARGET,
    MACOS_DEPLOYMENT_TARGET,
    PLUGIN_ROOT,
    REPOSITORY_ROOT,
    REQUIRED_EXPORTS,
)
from native_artifact import (
    FRAMEWORK_MODULE_MAP,
    NATIVE_MAGIC_PREFIX_BYTES,
    NATIVE_CONTRACTS,
    NativeArtifactError,
    canonical_flutter_native_binary,
    expected_native_groups,
    inspect_framework_info,
    inspect_xcframework,
    is_canonical_flutter_native_path,
    is_native_like_artifact,
    is_reserved_flutter_native_path,
    validate_native_image,
)
from package_metadata import PackageMetadataError, package_archive_path, package_version

sys.path.insert(0, str(REPOSITORY_ROOT / "scripts"))

from archive_policy import (  # noqa: E402
    ArchiveLimits,
    ArchivePolicyError,
    extract_archive,
    read_archive,
)


BUDGET_PATH = REPOSITORY_ROOT / "bindings" / "budgets.json"
INTEGRATION_TEST = PLUGIN_ROOT / "integration_test" / "native_load_test.dart"
IOS_RUNTIME_SMOKE_SOURCE = PLUGIN_ROOT / "tool" / "ios_runtime_smoke.dart"
RUNTIME_SMOKE_PROBE_SOURCE = PLUGIN_ROOT / "tool" / "runtime_smoke_probe.dart"
IOS_RUNTIME_SMOKE_RESULT = "mdstream-flutter-runtime-smoke.json"
IOS_RUNTIME_SMOKE_SCHEMA = "mdstream.flutter-runtime-smoke/1"
IOS_RUNTIME_SMOKE_TIMEOUT_SECONDS = 60.0
IOS_RUNTIME_SMOKE_SIMCTL_TIMEOUT_SECONDS = 30.0
IOS_RUNTIME_SMOKE_DIAGNOSTIC_TIMEOUT_SECONDS = 10.0
IOS_RUNTIME_SMOKE_DIAGNOSTIC_CHARS = 8_000
IOS_RUNTIME_SMOKE_EXPECTED = {
    "abi_version": 1,
    "package_version": package_version(),
    "binding_schema": "mdstream.bindings/0.4",
    "is_finalized": True,
    "has_root_node": True,
    "native_allocations_zero": True,
}
TEXT_IMPORT_PATTERN = re.compile(
    rb"(?:import|export)\s+['\"]package:([a-zA-Z0-9_]+)(?:/[^'\"]*)?['\"]"
)
APPLE_HOST_TARGETS = {
    "ios": ("IPHONEOS_DEPLOYMENT_TARGET", "ios", IOS_DEPLOYMENT_TARGET),
    "macos": ("MACOSX_DEPLOYMENT_TARGET", "osx", MACOS_DEPLOYMENT_TARGET),
}
SWIFTPM_PLATFORMS = {
    "ios": ("iOS", IOS_DEPLOYMENT_TARGET),
    "macos": ("macOS", MACOS_DEPLOYMENT_TARGET),
}
SWIFTPM_CONSUMER_NAME = "MdstreamSwiftPMConsumer"


class PackageSmokeError(RuntimeError):
    """Raised when a publish archive violates the Flutter package contract."""


@dataclass(frozen=True)
class ArchiveReport:
    archive_bytes: int
    max_native_bytes: int
    max_platform_increment_bytes: int
    native_groups: dict[str, int]
    platforms: tuple[str, ...]


def _load_budget() -> dict[str, object]:
    try:
        value = json.loads(BUDGET_PATH.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise PackageSmokeError(f"failed to load binding budgets: {error}") from error
    if not isinstance(value, dict):
        raise PackageSmokeError("binding budget must be a JSON object")
    return value


def _budget_ceiling(budget: dict[str, object], artifact: str) -> int:
    entries = budget.get("artifacts")
    if not isinstance(entries, list):
        raise PackageSmokeError("binding budget artifacts must be an array")
    for entry in entries:
        if isinstance(entry, dict) and entry.get("artifact") == artifact:
            ceiling = entry.get("ceiling_bytes")
            if isinstance(ceiling, int) and ceiling > 0:
                return ceiling
    raise PackageSmokeError(f"binding budget does not define {artifact}")


def _forbidden_dependencies(budget: dict[str, object]) -> set[str]:
    policy = budget.get("policy")
    if not isinstance(policy, dict):
        raise PackageSmokeError("binding budget policy must be an object")
    values = policy.get("forbidden_default_dependencies")
    if not isinstance(values, list) or not all(isinstance(value, str) for value in values):
        raise PackageSmokeError("forbidden_default_dependencies must be strings")
    return {value.lower() for value in values}


def validate_dependency_graph(
    graph: dict[str, object], forbidden: set[str]
) -> None:
    packages = graph.get("packages")
    if not isinstance(packages, list):
        raise PackageSmokeError("pub dependency graph does not contain packages")
    names = {
        package.get("name", "").lower()
        for package in packages
        if isinstance(package, dict) and isinstance(package.get("name"), str)
    }
    matches = sorted(names & {name.lower() for name in forbidden})
    if matches:
        raise PackageSmokeError(
            f"Flutter dependency graph contains forbidden package(s): {', '.join(matches)}"
        )


def _group_for_entry(name: str) -> tuple[str, str] | None:
    parts = PurePosixPath(name).parts
    if len(parts) >= 5 and parts[:4] == (
        "android",
        "src",
        "main",
        "jniLibs",
    ):
        return "android", f"android/{parts[4]}"
    if len(parts) >= 3 and parts[:2] == ("linux", "lib"):
        return "linux", f"linux/{parts[2]}"
    if len(parts) >= 3 and parts[:2] == ("windows", "lib"):
        return "windows", f"windows/{parts[2]}"
    if (
        len(parts) >= 3
        and parts[0] in {"ios", "macos"}
        and parts[1] == "MdstreamFFI.xcframework"
        and parts[2] != "Info.plist"
    ):
        return parts[0], f"{parts[0]}/{parts[2]}"
    return None


def _safe_archive_entries(
    path: Path,
    archive_limits: ArchiveLimits | dict[str, object] | None = None,
) -> dict[str, bytes]:
    try:
        return {
            entry.name: entry.data
            for entry in read_archive(path, archive_limits)
            if entry.data is not None
        }
    except ArchivePolicyError as error:
        raise PackageSmokeError(str(error)) from error


def inspect_package_archive(
    archive: Path,
    *,
    forbidden_terms: set[str],
    native_ceiling_bytes: int,
    increment_ceiling_bytes: int,
    require_all_platforms: bool,
    archive_limits: ArchiveLimits | dict[str, object] | None = None,
) -> ArchiveReport:
    if not archive.is_file():
        raise PackageSmokeError(f"publish archive does not exist: {archive}")
    entries = _safe_archive_entries(archive, archive_limits)
    pubspec = entries.get("pubspec.yaml")
    if pubspec is None:
        raise PackageSmokeError("publish archive does not contain pubspec.yaml")
    if re.search(rb"(?m)^\s+path\s*:", pubspec):
        raise PackageSmokeError("publish archive contains a path dependency")

    forbidden = {term.lower() for term in forbidden_terms}
    for name, data in entries.items():
        path_tokens = set(re.split(r"[^a-z0-9]+", name.lower()))
        path_matches = sorted(path_tokens & forbidden)
        if path_matches:
            raise PackageSmokeError(
                f"publish archive path contains forbidden term "
                f"{path_matches[0]}: {name}"
            )
        if name.endswith(".dart"):
            imports = {
                match.group(1).decode("ascii").lower()
                for match in TEXT_IMPORT_PATTERN.finditer(data)
            }
            import_matches = sorted(imports & forbidden)
            if import_matches:
                raise PackageSmokeError(
                    f"Dart import contains forbidden package "
                    f"{import_matches[0]}: {name}"
                )

    try:
        canonical_header = HEADER_PATH.read_bytes()
    except OSError as error:
        raise PackageSmokeError(
            f"failed to read canonical mdstream header: {error}"
        ) from error

    apple_binaries: dict[str, str] = {}
    for platform_name in ("ios", "macos"):
        framework_root = f"{platform_name}/MdstreamFFI.xcframework/"
        info_name = f"{framework_root}Info.plist"
        info = entries.get(info_name)
        framework_entries = any(name.startswith(framework_root) for name in entries)
        if info is None:
            if framework_entries:
                raise PackageSmokeError(
                    f"{platform_name} XCFramework does not contain Info.plist"
                )
            continue
        try:
            slices = inspect_xcframework(info, platform_name)
        except NativeArtifactError as error:
            raise PackageSmokeError(
                f"invalid {platform_name} XCFramework metadata: {error}"
            ) from error
        expected_files = {info_name}
        for slice_ in slices:
            bundle_root = (
                f"{framework_root}{slice_.identifier}/MdstreamFFI.framework/"
            )
            binary_name = f"{framework_root}{slice_.binary_path}"
            header_name = f"{bundle_root}Headers/mdstream.h"
            module_name = f"{bundle_root}Modules/module.modulemap"
            bundle_info_name = f"{bundle_root}Info.plist"
            expected_files.update(
                (binary_name, header_name, module_name, bundle_info_name)
            )
            missing = [
                name
                for name in (header_name, module_name, bundle_info_name)
                if name not in entries
            ]
            if missing:
                raise PackageSmokeError(
                    "Apple framework slice is missing " + ", ".join(missing)
                )
            if entries[header_name] != canonical_header:
                raise PackageSmokeError(
                    f"Apple framework header differs from mdstream.h: {header_name}"
                )
            if entries[module_name] != FRAMEWORK_MODULE_MAP.encode("utf-8"):
                raise PackageSmokeError(
                    f"Apple framework has an unexpected module.modulemap: {module_name}"
                )
            try:
                inspect_framework_info(
                    entries[bundle_info_name],
                    NATIVE_CONTRACTS[slice_.group],
                )
            except NativeArtifactError as error:
                raise PackageSmokeError(
                    f"invalid Apple framework Info.plist {bundle_info_name}: {error}"
                ) from error
            apple_binaries[binary_name] = slice_.group
        actual_files = {
            name for name in entries if name.startswith(framework_root)
        }
        if actual_files != expected_files:
            missing = sorted(expected_files - actual_files)
            unexpected = sorted(actual_files - expected_files)
            detail = []
            if missing:
                detail.append(f"missing {missing[0]}")
            if unexpected:
                detail.append(f"unexpected {unexpected[0]}")
            raise PackageSmokeError(
                f"{platform_name} XCFramework file inventory mismatch: "
                + "; ".join(detail)
            )

    native_groups: dict[str, int] = {}
    platform_static = {name: 0 for name in ("android", "ios", "macos", "linux", "windows")}
    native_sizes: list[int] = []
    platform_binaries: dict[str, set[str]] = {
        name: set() for name in platform_static
    }
    seen_apple_binaries: set[str] = set()
    for name, data in entries.items():
        if (
            not is_canonical_flutter_native_path(name)
            and (
                is_reserved_flutter_native_path(name)
                or is_native_like_artifact(name, data[:NATIVE_MAGIC_PREFIX_BYTES])
            )
        ):
            raise PackageSmokeError(
                "publish archive contains a native-like file outside canonical "
                f"native inventory: {name}"
            )
        parts = PurePosixPath(name).parts
        platform_name = parts[0] if parts and parts[0] in platform_static else None
        grouped = _group_for_entry(name)
        if grouped is not None:
            _, group = grouped
            native_groups[group] = native_groups.get(group, 0) + len(data)
        elif platform_name is not None:
            platform_static[platform_name] += len(data)

        native = canonical_flutter_native_binary(name)
        if native is None:
            continue
        native_platform, group = native
        contract = NATIVE_CONTRACTS.get(group)
        if contract is None:
            raise PackageSmokeError(
                f"publish archive contains an unsupported native slice: {name}"
            )
        if native_platform in {"ios", "macos"}:
            if apple_binaries.get(name) != group:
                raise PackageSmokeError(
                    f"Apple native binary is not declared by its XCFramework: {name}"
                )
            seen_apple_binaries.add(name)
        try:
            image = validate_native_image(data, contract)
        except NativeArtifactError as error:
            raise PackageSmokeError(
                f"native artifact contract mismatch for {name}: {error}"
            ) from error
        if len(data) > native_ceiling_bytes:
            raise PackageSmokeError(
                f"native library exceeds {native_ceiling_bytes}-byte ceiling: "
                f"{name} ({len(data)} bytes)"
            )
        missing_symbols = [
            symbol for symbol in REQUIRED_EXPORTS if symbol not in image.exported_symbols
        ]
        if missing_symbols:
            raise PackageSmokeError(
                f"native library lacks required ABI symbol names "
                f"{', '.join(missing_symbols)}: {name}"
            )
        native_sizes.append(len(data))
        platform_binaries[native_platform].add(group)

    if seen_apple_binaries != set(apple_binaries):
        missing = sorted(set(apple_binaries) - seen_apple_binaries)
        raise PackageSmokeError(
            "publish archive is missing XCFramework binary path(s): "
            f"{', '.join(missing)}"
        )

    if not native_sizes:
        raise PackageSmokeError("publish archive contains no mdstream native library")

    if require_all_platforms:
        for platform_name in ("android", "ios", "macos", "linux", "windows"):
            if not platform_binaries[platform_name]:
                raise PackageSmokeError(
                    f"publish archive has no staged {platform_name} native library"
                )
        for platform_name in platform_binaries:
            required = expected_native_groups(platform_name)
            missing = sorted(required - platform_binaries[platform_name])
            if missing:
                raise PackageSmokeError(
                    f"publish archive is missing {platform_name} slice(s): "
                    f"{', '.join(missing)}"
                )
    platform_native = {name: 0 for name in platform_static}
    for group, size in native_groups.items():
        platform_native[group.split("/", 1)[0]] += size
    increments = {
        platform_name: platform_native[platform_name] + static_bytes
        for platform_name, static_bytes in platform_static.items()
    }
    max_increment = max(increments.values(), default=0)
    if max_increment > increment_ceiling_bytes:
        raise PackageSmokeError(
            f"platform package increment exceeds {increment_ceiling_bytes}-byte ceiling: "
            f"{max_increment} bytes"
        )
    return ArchiveReport(
        archive_bytes=archive.stat().st_size,
        max_native_bytes=max(native_sizes),
        max_platform_increment_bytes=max_increment,
        native_groups=dict(sorted(native_groups.items())),
        platforms=tuple(
            platform_name
            for platform_name, groups in platform_binaries.items()
            if groups
        ),
    )


def _run(
    command: list[str],
    *,
    cwd: Path,
    env: dict[str, str] | None = None,
    capture: bool = False,
    timeout: float | None = None,
) -> subprocess.CompletedProcess[str]:
    print("+", " ".join(command), flush=True)
    try:
        return subprocess.run(
            command,
            cwd=cwd,
            env=env,
            check=True,
            text=True,
            capture_output=capture,
            timeout=timeout,
        )
    except FileNotFoundError as error:
        raise PackageSmokeError(f"required tool not found: {command[0]}") from error
    except subprocess.TimeoutExpired as error:
        duration = timeout if timeout is not None else error.timeout
        raise PackageSmokeError(
            f"command timed out after {duration:g} seconds: {' '.join(command)}"
        ) from error
    except subprocess.CalledProcessError as error:
        detail = (error.stderr or error.stdout or "").strip()
        suffix = f": {detail}" if detail else ""
        raise PackageSmokeError(
            f"command failed with exit code {error.returncode}: {' '.join(command)}{suffix}"
        ) from error


def _flutter_tool() -> str:
    return "flutter.bat" if sys.platform == "win32" else "flutter"


def _create_archive(output: Path) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.exists():
        output.unlink()
    _run(
        ["dart", "pub", "publish", f"--to-archive={output}"],
        cwd=PLUGIN_ROOT,
    )


def _dependency_graph() -> dict[str, object]:
    result = _run(
        ["dart", "pub", "deps", "--json"], cwd=PLUGIN_ROOT, capture=True
    )
    try:
        value = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise PackageSmokeError(f"invalid pub dependency graph: {error}") from error
    if not isinstance(value, dict):
        raise PackageSmokeError("pub dependency graph must be an object")
    return value


def _extract_archive(archive: Path, destination: Path) -> None:
    try:
        extract_archive(archive, destination)
    except ArchivePolicyError as error:
        raise PackageSmokeError(str(error)) from error


@contextmanager
def plugin_source_context(archive: Path | None) -> Iterator[Path]:
    """Yield repository source or a safely extracted, short-lived archive tree."""

    if archive is None:
        yield PLUGIN_ROOT
        return
    with tempfile.TemporaryDirectory(
        prefix="mdstream-flutter-archive-source-"
    ) as temporary:
        extracted = Path(temporary) / "plugin"
        _extract_archive(archive, extracted)
        yield extracted


def validate_package_archive(
    archive: Path, *, require_all_platforms: bool = False
) -> ArchiveReport:
    budget = _load_budget()
    return inspect_package_archive(
        archive,
        forbidden_terms=_forbidden_dependencies(budget),
        native_ceiling_bytes=_budget_ceiling(budget, "flutter_native_library"),
        increment_ceiling_bytes=_budget_ceiling(
            budget, "platform_package_increment"
        ),
        require_all_platforms=require_all_platforms,
    )


def _restore_macos_frameworks(plugin_source: Path) -> None:
    """Undo the CocoaPods build-time framework layout in a package tree."""
    xcframework = plugin_source / "macos" / "MdstreamFFI.xcframework"
    if not xcframework.is_dir():
        return
    for framework in xcframework.glob("*/*.framework"):
        current = framework / "Versions" / "Current"
        version = framework / "Versions" / "A"
        if not current.is_symlink() or not version.is_dir():
            continue
        for name in ("MdstreamFFI", "Headers", "Modules", "Resources"):
            root_entry = framework / name
            if root_entry.is_symlink() or root_entry.exists():
                root_entry.unlink()
        for name in ("MdstreamFFI", "Headers", "Modules"):
            shutil.move(str(version / name), str(framework / name))
        resources = version / "Resources"
        shutil.move(str(resources / "Info.plist"), str(framework / "Info.plist"))
        shutil.rmtree(framework / "Versions")


def _swiftpm_manifest_root(platform_name: str, plugin_source: Path = PLUGIN_ROOT) -> Path:
    if platform_name not in SWIFTPM_PLATFORMS:
        raise PackageSmokeError(
            f"SwiftPM smoke only supports Apple platforms, got {platform_name!r}"
        )
    root = plugin_source / platform_name / "mdstream_flutter"
    manifest = root / "Package.swift"
    framework = root.parent / "MdstreamFFI.xcframework"
    if not manifest.is_file():
        raise PackageSmokeError(f"SwiftPM manifest does not exist: {manifest}")
    if not framework.is_dir():
        raise PackageSmokeError(
            f"SwiftPM binary target does not exist: {framework}"
        )
    return root


def _validate_swiftpm_manifest(platform_name: str, manifest: object) -> None:
    if not isinstance(manifest, dict):
        raise PackageSmokeError("swift package dump-package did not return an object")
    if manifest.get("name") != "mdstream_flutter":
        raise PackageSmokeError("SwiftPM manifest has an unexpected package name")

    expected_platform, expected_version = SWIFTPM_PLATFORMS[platform_name]
    platforms = manifest.get("platforms")
    if not isinstance(platforms, list) or not any(
        isinstance(value, dict)
        and value.get("platformName") == platform_name
        and value.get("version") == expected_version
        for value in platforms
    ):
        raise PackageSmokeError(
            f"SwiftPM manifest does not declare {expected_platform} {expected_version}"
        )

    products = manifest.get("products")
    if not isinstance(products, list) or not any(
        isinstance(value, dict)
        and value.get("name") == "mdstream-flutter"
        and value.get("targets") == ["mdstream_flutter"]
        for value in products
    ):
        raise PackageSmokeError("SwiftPM manifest does not expose mdstream-flutter")

    targets = manifest.get("targets")
    if not isinstance(targets, list):
        raise PackageSmokeError("SwiftPM manifest does not contain targets")
    binary = next(
        (
            value
            for value in targets
            if isinstance(value, dict) and value.get("name") == "MdstreamFFI"
        ),
        None,
    )
    if not isinstance(binary, dict) or binary.get("type") != "binary":
        raise PackageSmokeError("SwiftPM manifest does not declare MdstreamFFI binary target")
    if binary.get("path") != "../MdstreamFFI.xcframework":
        raise PackageSmokeError("SwiftPM MdstreamFFI target points at an unexpected path")

    wrapper = next(
        (
            value
            for value in targets
            if isinstance(value, dict) and value.get("name") == "mdstream_flutter"
        ),
        None,
    )
    dependencies = wrapper.get("dependencies") if isinstance(wrapper, dict) else None
    if not isinstance(dependencies, list) or not any(
        isinstance(value, dict)
        and isinstance(value.get("byName"), list)
        and value.get("byName")
        and value["byName"][0] == "MdstreamFFI"
        for value in dependencies
    ):
        raise PackageSmokeError(
            "SwiftPM mdstream_flutter target does not depend on MdstreamFFI"
        )


def _write_swiftpm_consumer(root: Path, platform_name: str, package_root: Path) -> None:
    root.mkdir(parents=True, exist_ok=True)
    _, minimum_version = SWIFTPM_PLATFORMS[platform_name]
    swift_version = minimum_version.split(".", 1)[0]
    platform_clause = (
        f".iOS(.v{swift_version})"
        if platform_name == "ios"
        else f".macOS(.v{swift_version})"
    )
    package_path = json.dumps(str(package_root), ensure_ascii=True)
    package_manifest = f'''// swift-tools-version: 5.9

import PackageDescription

let package = Package(
    name: "{SWIFTPM_CONSUMER_NAME}",
    platforms: [{platform_clause}],
    dependencies: [
        .package(path: {package_path}),
    ],
    targets: [
        .target(
            name: "{SWIFTPM_CONSUMER_NAME}",
            dependencies: [
                .product(name: "mdstream-flutter", package: "mdstream_flutter"),
            ],
            path: "Sources/{SWIFTPM_CONSUMER_NAME}"
        ),
        .testTarget(
            name: "{SWIFTPM_CONSUMER_NAME}Tests",
            dependencies: ["{SWIFTPM_CONSUMER_NAME}"],
            path: "Tests/{SWIFTPM_CONSUMER_NAME}"
        ),
    ]
)
'''
    source = '''import mdstream_flutter

public enum MdstreamSwiftPMSmoke {
    public static func bundledABIVersion() -> UInt32 {
        mdstream_abi_version()
    }

    public static func bundledPackageVersion() -> String {
        guard let value = mdstream_package_version() else {
            return ""
        }
        return String(cString: value)
    }
}
'''
    tests = f'''import XCTest
@testable import {SWIFTPM_CONSUMER_NAME}

final class {SWIFTPM_CONSUMER_NAME}Tests: XCTestCase {{
    func testBundledLibraryLoads() {{
        XCTAssertEqual(MdstreamSwiftPMSmoke.bundledABIVersion(), 1)
        XCTAssertFalse(MdstreamSwiftPMSmoke.bundledPackageVersion().isEmpty)
    }}
}}
'''
    (root / "Package.swift").write_text(package_manifest, encoding="utf-8")
    source_root = root / "Sources" / SWIFTPM_CONSUMER_NAME
    source_root.mkdir(parents=True, exist_ok=True)
    (source_root / "MdstreamSwiftPMSmoke.swift").write_text(
        source, encoding="utf-8"
    )
    test_root = root / "Tests" / SWIFTPM_CONSUMER_NAME
    test_root.mkdir(parents=True, exist_ok=True)
    (test_root / "MdstreamSwiftPMSmokeTests.swift").write_text(
        tests, encoding="utf-8"
    )


def run_swiftpm_smoke(
    *,
    platform_name: str,
    device: str | None,
    keep_temporary: bool,
    plugin_source: Path = PLUGIN_ROOT,
) -> None:
    if sys.platform != "darwin":
        raise PackageSmokeError(
            "SwiftPM Apple smoke requires a macOS runner; Linux emulation is unsupported"
        )
    package_root = _swiftpm_manifest_root(platform_name, plugin_source)
    if platform_name == "ios" and not device:
        raise PackageSmokeError("iOS SwiftPM smoke requires a simulator device id")

    manifest_result = _run(
        [
            "swift",
            "package",
            "--package-path",
            str(package_root),
            "dump-package",
        ],
        cwd=REPOSITORY_ROOT,
        capture=True,
    )
    try:
        manifest = json.loads(manifest_result.stdout)
    except json.JSONDecodeError as error:
        raise PackageSmokeError(f"invalid SwiftPM manifest JSON: {error}") from error
    _validate_swiftpm_manifest(platform_name, manifest)

    temporary = Path(tempfile.mkdtemp(prefix=f"mdstream-swiftpm-{platform_name}-"))
    try:
        _write_swiftpm_consumer(temporary, platform_name, package_root)
        env = os.environ.copy()
        env.pop("MDSTREAM_NATIVE_LIBRARY", None)
        env.pop("MDSTREAM_FFI_LIBRARY", None)
        if platform_name == "ios":
            assert device is not None
            _run(
                [
                    "xcodebuild",
                    "test",
                    "-scheme",
                    f"{SWIFTPM_CONSUMER_NAME}-Package",
                    "-destination",
                    f"platform=iOS Simulator,id={device}",
                    "-derivedDataPath",
                    str(temporary / "DerivedData"),
                    "CODE_SIGNING_ALLOWED=NO",
                    "CODE_SIGNING_REQUIRED=NO",
                ],
                cwd=temporary,
                env=env,
            )
        else:
            _run(
                [
                    "swift",
                    "test",
                    "--package-path",
                    str(temporary),
                    "--scratch-path",
                    str(temporary / ".build"),
                ],
                cwd=REPOSITORY_ROOT,
                env=env,
            )
    finally:
        _restore_macos_frameworks(plugin_source)
        if keep_temporary:
            print(f"kept temporary SwiftPM consumer: {temporary}")
        else:
            shutil.rmtree(temporary, ignore_errors=True)


def configure_apple_host_target(project_root: Path, platform_name: str) -> None:
    target = APPLE_HOST_TARGETS.get(platform_name)
    if target is None:
        return
    setting, pod_platform, minimum = target
    platform_root = project_root / platform_name
    project_path = platform_root / "Runner.xcodeproj" / "project.pbxproj"
    podfile_path = platform_root / "Podfile"
    try:
        project = project_path.read_text(encoding="utf-8")
        podfile = podfile_path.read_text(encoding="utf-8")
    except OSError as error:
        raise PackageSmokeError(
            f"failed to read generated {platform_name} host metadata: {error}"
        ) from error

    project_pattern = re.compile(
        rf"({re.escape(setting)}\s*=\s*)[0-9]+(?:\.[0-9]+)*(;)"
    )
    project, project_count = project_pattern.subn(
        lambda match: f"{match.group(1)}{minimum}{match.group(2)}",
        project,
    )
    pod_pattern = re.compile(
        rf"(?m)^\s*#?\s*platform\s+:{pod_platform},\s*['\"][^'\"]+['\"]\s*$"
    )
    podfile, pod_count = pod_pattern.subn(
        f"platform :{pod_platform}, '{minimum}'",
        podfile,
    )
    if project_count == 0 or pod_count == 0:
        raise PackageSmokeError(
            f"generated {platform_name} host omitted its deployment target metadata"
        )
    try:
        project_path.write_text(project, encoding="utf-8")
        podfile_path.write_text(podfile, encoding="utf-8")
    except OSError as error:
        raise PackageSmokeError(
            f"failed to update generated {platform_name} host metadata: {error}"
        ) from error


def _ios_bundle_identifier(app: Path) -> str:
    info_path = app / "Info.plist"
    try:
        with info_path.open("rb") as handle:
            info = plistlib.load(handle)
    except (OSError, plistlib.InvalidFileException) as error:
        raise PackageSmokeError(
            f"failed to read built iOS application metadata: {error}"
        ) from error
    if not isinstance(info, dict):
        raise PackageSmokeError(
            f"built iOS application metadata is not a dictionary: {info_path}"
        )
    bundle_identifier = info.get("CFBundleIdentifier")
    if not isinstance(bundle_identifier, str) or not bundle_identifier:
        raise PackageSmokeError(
            f"built iOS application has no bundle identifier: {info_path}"
        )
    return bundle_identifier


def _validate_ios_runtime_smoke_payload(payload: object) -> None:
    if not isinstance(payload, dict):
        raise PackageSmokeError("iOS runtime smoke result must be a JSON object")
    schema = payload.get("schema")
    if type(schema) is not str or schema != IOS_RUNTIME_SMOKE_SCHEMA:
        raise PackageSmokeError("iOS runtime smoke returned an unexpected schema")
    ok = payload.get("ok")
    if type(ok) is not bool:
        raise PackageSmokeError("iOS runtime smoke result field ok must be a boolean")
    if not ok:
        detail = payload.get("error")
        stack_trace = payload.get("stack_trace")
        if type(detail) is not str or not detail:
            raise PackageSmokeError(
                "iOS runtime smoke failure must include a non-empty error"
            )
        if stack_trace is not None and type(stack_trace) is not str:
            raise PackageSmokeError(
                "iOS runtime smoke failure stack_trace must be a string"
            )
        suffix = f": {detail}"
        if stack_trace:
            suffix += "\n" + stack_trace
        suffix = suffix[:IOS_RUNTIME_SMOKE_DIAGNOSTIC_CHARS]
        raise PackageSmokeError(f"iOS runtime smoke failed{suffix}")
    mismatches = [
        f"{name}={payload.get(name)!r} (expected {expected!r})"
        for name, expected in IOS_RUNTIME_SMOKE_EXPECTED.items()
        if type(payload.get(name)) is not type(expected)
        or payload.get(name) != expected
    ]
    if mismatches:
        raise PackageSmokeError(
            "iOS runtime smoke returned invalid values: " + ", ".join(mismatches)
        )


def _wait_for_ios_runtime_smoke_result(
    result_path: Path,
    *,
    diagnostics: Callable[[], str] | None = None,
) -> None:
    deadline = time.monotonic() + IOS_RUNTIME_SMOKE_TIMEOUT_SECONDS
    while not result_path.is_file():
        if time.monotonic() >= deadline:
            detail = diagnostics() if diagnostics is not None else ""
            suffix = f"\n{detail}" if detail else ""
            raise PackageSmokeError(
                "iOS runtime smoke did not publish a result within "
                f"{IOS_RUNTIME_SMOKE_TIMEOUT_SECONDS:g} seconds: {result_path}"
                f"{suffix}"
            )
        time.sleep(0.25)
    try:
        payload = json.loads(result_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise PackageSmokeError(
            f"failed to read iOS runtime smoke result: {error}"
        ) from error
    _validate_ios_runtime_smoke_payload(payload)


def _diagnostic_tail(text: str) -> str:
    if len(text) <= IOS_RUNTIME_SMOKE_DIAGNOSTIC_CHARS:
        return text
    return "[truncated]\n" + text[-IOS_RUNTIME_SMOKE_DIAGNOSTIC_CHARS:]


def _collect_ios_runtime_diagnostics(
    *,
    project_root: Path,
    device: str,
    bundle_identifier: str,
    stdout_path: Path,
    stderr_path: Path,
) -> str:
    sections: list[str] = []
    for label, path in (
        ("Runner stdout", stdout_path),
        ("Runner stderr", stderr_path),
    ):
        try:
            value = path.read_text(encoding="utf-8", errors="replace")
        except OSError as error:
            sections.append(f"{label}: unavailable ({error})")
        else:
            sections.append(f"{label}:\n{_diagnostic_tail(value) if value else '[empty]'}")

    try:
        state = _run(
            ["xcrun", "simctl", "spawn", device, "launchctl", "print", "system"],
            cwd=project_root,
            capture=True,
            timeout=IOS_RUNTIME_SMOKE_DIAGNOSTIC_TIMEOUT_SECONDS,
        ).stdout
        matches = [
            line
            for line in state.splitlines()
            if bundle_identifier.lower() in line.lower() or "Runner" in line
        ]
        sections.append(
            "Runner process state:\n"
            + (_diagnostic_tail("\n".join(matches)) if matches else "[not running]")
        )
    except PackageSmokeError as error:
        sections.append(f"Runner process state: unavailable ({error})")

    predicate = (
        f'process == "Runner" OR eventMessage CONTAINS[c] "{bundle_identifier}"'
    )
    try:
        logs = _run(
            [
                "xcrun",
                "simctl",
                "spawn",
                device,
                "log",
                "show",
                "--last",
                "2m",
                "--style",
                "compact",
                "--predicate",
                predicate,
            ],
            cwd=project_root,
            capture=True,
            timeout=IOS_RUNTIME_SMOKE_DIAGNOSTIC_TIMEOUT_SECONDS,
        ).stdout
        sections.append(
            "Runner unified log:\n" + (_diagnostic_tail(logs) if logs else "[empty]")
        )
    except PackageSmokeError as error:
        sections.append(f"Runner unified log: unavailable ({error})")
    return _diagnostic_tail("\n".join(sections))


def _terminate_ios_runtime_smoke(
    *, project_root: Path, device: str, bundle_identifier: str
) -> None:
    try:
        _run(
            ["xcrun", "simctl", "terminate", device, bundle_identifier],
            cwd=project_root,
            capture=True,
            timeout=IOS_RUNTIME_SMOKE_DIAGNOSTIC_TIMEOUT_SECONDS,
        )
    except PackageSmokeError as error:
        print(f"warning: failed to terminate iOS runtime smoke: {error}", file=sys.stderr)


def _run_ios_runtime_smoke(
    *,
    project_root: Path,
    device: str,
    env: dict[str, str],
) -> None:
    if not IOS_RUNTIME_SMOKE_SOURCE.is_file():
        raise PackageSmokeError(
            f"iOS runtime smoke source does not exist: {IOS_RUNTIME_SMOKE_SOURCE}"
        )
    shutil.copy2(IOS_RUNTIME_SMOKE_SOURCE, project_root / "lib" / "main.dart")
    shutil.copy2(
        RUNTIME_SMOKE_PROBE_SOURCE,
        project_root / "lib" / RUNTIME_SMOKE_PROBE_SOURCE.name,
    )
    _run(
        [_flutter_tool(), "build", "ios", "--simulator", "--debug"],
        cwd=project_root,
        env=env,
    )

    app = project_root / "build" / "ios" / "iphonesimulator" / "Runner.app"
    if not app.is_dir():
        raise PackageSmokeError(f"Flutter did not produce an iOS application: {app}")
    bundle_identifier = _ios_bundle_identifier(app)
    _run(
        ["xcrun", "simctl", "install", device, str(app)],
        cwd=project_root,
        timeout=IOS_RUNTIME_SMOKE_SIMCTL_TIMEOUT_SECONDS,
    )
    container_result = _run(
        [
            "xcrun",
            "simctl",
            "get_app_container",
            device,
            bundle_identifier,
            "data",
        ],
        cwd=project_root,
        capture=True,
        timeout=IOS_RUNTIME_SMOKE_SIMCTL_TIMEOUT_SECONDS,
    )
    container_text = container_result.stdout.strip()
    container = Path(container_text) if container_text else None
    if container is None or not container.is_absolute() or not container.is_dir():
        raise PackageSmokeError(
            "simulator returned an invalid application container: "
            f"{container_text!r}"
        )
    result_path = container / "tmp" / IOS_RUNTIME_SMOKE_RESULT
    stdout_path = container / "tmp" / "mdstream-flutter-runtime-smoke.stdout"
    stderr_path = container / "tmp" / "mdstream-flutter-runtime-smoke.stderr"
    for stale_path in (result_path, stdout_path, stderr_path):
        stale_path.unlink(missing_ok=True)
    try:
        _run(
            [
                "xcrun",
                "simctl",
                "launch",
                "--terminate-running-process",
                f"--stdout={stdout_path}",
                f"--stderr={stderr_path}",
                device,
                bundle_identifier,
            ],
            cwd=project_root,
            env=env,
            timeout=IOS_RUNTIME_SMOKE_SIMCTL_TIMEOUT_SECONDS,
        )
        _wait_for_ios_runtime_smoke_result(
            result_path,
            diagnostics=lambda: _collect_ios_runtime_diagnostics(
                project_root=project_root,
                device=device,
                bundle_identifier=bundle_identifier,
                stdout_path=stdout_path,
                stderr_path=stderr_path,
            ),
        )
    finally:
        _terminate_ios_runtime_smoke(
            project_root=project_root,
            device=device,
            bundle_identifier=bundle_identifier,
        )


def run_runtime_smoke(
    *,
    platform_name: str,
    device: str,
    plugin_source: Path,
    keep_temporary: bool,
) -> None:
    if not RUNTIME_SMOKE_PROBE_SOURCE.is_file():
        raise PackageSmokeError(
            f"runtime smoke probe does not exist: {RUNTIME_SMOKE_PROBE_SOURCE}"
        )
    if platform_name != "ios" and not INTEGRATION_TEST.is_file():
        raise PackageSmokeError(f"integration test does not exist: {INTEGRATION_TEST}")
    temporary = Path(
        tempfile.mkdtemp(prefix=f"mdstream-flutter-{platform_name}-")
    )
    try:
        _run(
            [
                _flutter_tool(),
                "create",
                "--platforms",
                platform_name,
                "--project-name",
                "mdstream_flutter_smoke",
                "--org",
                "io.mdstream.smoke",
                str(temporary),
            ],
            cwd=PLUGIN_ROOT,
        )
        dependencies = [
            _flutter_tool(),
            "pub",
            "add",
            f"mdstream_flutter:{{path: {plugin_source.as_posix()}}}",
            f"override:mdstream:{{path: {(REPOSITORY_ROOT / 'bindings' / 'dart').as_posix()}}}",
        ]
        if platform_name != "ios":
            dependencies.append("dev:integration_test:{sdk: flutter}")
        _run(dependencies, cwd=temporary)
        configure_apple_host_target(temporary, platform_name)
        env = os.environ.copy()
        for name in (
            "MDSTREAM_NATIVE_LIBRARY",
            "MDSTREAM_FFI_LIBRARY",
            "SIMCTL_CHILD_MDSTREAM_NATIVE_LIBRARY",
            "SIMCTL_CHILD_MDSTREAM_FFI_LIBRARY",
        ):
            env.pop(name, None)
        if platform_name == "ios":
            _run_ios_runtime_smoke(
                project_root=temporary,
                device=device,
                env=env,
            )
        else:
            target = temporary / "integration_test" / INTEGRATION_TEST.name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(INTEGRATION_TEST, target)
            probe_target = temporary / "tool" / RUNTIME_SMOKE_PROBE_SOURCE.name
            probe_target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(RUNTIME_SMOKE_PROBE_SOURCE, probe_target)
            _run(
                [
                    _flutter_tool(),
                    "test",
                    str(target.relative_to(temporary)),
                    "-d",
                    device,
                ],
                cwd=temporary,
                env=env,
            )
    finally:
        _restore_macos_frameworks(plugin_source)
        if keep_temporary:
            print(f"kept temporary smoke app: {temporary}")
        else:
            shutil.rmtree(temporary, ignore_errors=True)


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--host", action="store_true")
    mode.add_argument("--release", action="store_true")
    mode.add_argument(
        "--swiftpm",
        action="store_true",
        help="Run a downstream SwiftPM consumer against an Apple package manifest",
    )
    parser.add_argument(
        "--archive",
        type=Path,
        help="Use this exact publish archive as the plugin source",
    )
    parser.add_argument(
        "--platform", choices=("ios", "macos", "linux", "windows")
    )
    parser.add_argument("--device")
    parser.add_argument("--skip-native-build", action="store_true")
    parser.add_argument("--skip-runtime", action="store_true")
    parser.add_argument("--skip-archive", action="store_true")
    parser.add_argument("--keep-temporary", action="store_true")
    parser.add_argument("--print-archive-path", action="store_true")
    return parser.parse_args()


def _host_platform() -> str:
    mapping = {"darwin": "macos", "linux": "linux", "win32": "windows"}
    try:
        return mapping[sys.platform]
    except KeyError as error:
        raise PackageSmokeError(f"unsupported host platform: {sys.platform}") from error


def main() -> int:
    args = _parse_args()
    if args.archive is not None and not args.skip_native_build:
        print(
            "error: --archive requires --skip-native-build to preserve exact producer bytes",
            file=sys.stderr,
        )
        return 1
    if args.swiftpm:
        try:
            if args.platform not in SWIFTPM_PLATFORMS:
                raise PackageSmokeError(
                    "--swiftpm requires --platform ios or --platform macos"
                )
            if args.archive is not None:
                budget = _load_budget()
                forbidden = _forbidden_dependencies(budget)
                report = inspect_package_archive(
                    args.archive,
                    forbidden_terms=forbidden,
                    native_ceiling_bytes=_budget_ceiling(
                        budget, "flutter_native_library"
                    ),
                    increment_ceiling_bytes=_budget_ceiling(
                        budget, "platform_package_increment"
                    ),
                    require_all_platforms=False,
                )
                print(
                    json.dumps(
                        {
                            "schema": "mdstream.flutter-swiftpm-archive/1",
                            "archive": str(args.archive),
                            "archive_bytes": report.archive_bytes,
                            "platforms": report.platforms,
                        },
                        indent=2,
                        sort_keys=True,
                    )
                )
                with plugin_source_context(args.archive) as plugin_source:
                    run_swiftpm_smoke(
                        platform_name=args.platform,
                        device=args.device,
                        keep_temporary=args.keep_temporary,
                        plugin_source=plugin_source,
                    )
                return 0
            if not args.skip_native_build:
                _run(
                    [
                        sys.executable,
                        str(Path(__file__).with_name("build_native.py")),
                        args.platform,
                    ],
                    cwd=REPOSITORY_ROOT,
                )
            run_swiftpm_smoke(
                platform_name=args.platform,
                device=args.device,
                keep_temporary=args.keep_temporary,
                plugin_source=PLUGIN_ROOT,
            )
        except PackageSmokeError as error:
            print(f"error: {error}", file=sys.stderr)
            return 1
        return 0
    try:
        default_archive = package_archive_path()
    except PackageMetadataError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    if args.print_archive_path:
        print(default_archive.relative_to(REPOSITORY_ROOT).as_posix())
        return 0
    budget = _load_budget()
    forbidden = _forbidden_dependencies(budget)
    native_ceiling = _budget_ceiling(budget, "flutter_native_library")
    increment_ceiling = _budget_ceiling(budget, "platform_package_increment")
    platform_name = args.platform or (_host_platform() if args.host else None)
    archive = args.archive
    try:
        if platform_name is not None and not args.skip_native_build:
            build_platform = "host" if args.host else platform_name
            _run(
                [sys.executable, str(Path(__file__).with_name("build_native.py")), build_platform],
                cwd=REPOSITORY_ROOT,
            )
        if archive is None:
            _run([_flutter_tool(), "pub", "get"], cwd=REPOSITORY_ROOT / "bindings")
            validate_dependency_graph(_dependency_graph(), forbidden)

        if archive is None and not args.skip_archive:
            archive = default_archive
            _create_archive(archive)
        report: ArchiveReport | None = None
        if archive is not None:
            report = inspect_package_archive(
                archive,
                forbidden_terms=forbidden,
                native_ceiling_bytes=native_ceiling,
                increment_ceiling_bytes=increment_ceiling,
                require_all_platforms=args.release,
            )
            print(
                json.dumps(
                    {
                        "schema": "mdstream.flutter-package-measurement/1",
                        "archive": str(archive),
                        "archive_bytes": report.archive_bytes,
                        "flutter_native_library": {
                            "ceiling_bytes": native_ceiling,
                            "measured_bytes": report.max_native_bytes,
                        },
                        "platform_package_increment": {
                            "ceiling_bytes": increment_ceiling,
                            "measured_bytes": report.max_platform_increment_bytes,
                            "groups": report.native_groups,
                        },
                        "platforms": report.platforms,
                    },
                    indent=2,
                    sort_keys=True,
                )
            )

        if platform_name is not None and not args.skip_runtime:
            device = args.device or platform_name
            with plugin_source_context(archive) as plugin_source:
                run_runtime_smoke(
                    platform_name=platform_name,
                    device=device,
                    plugin_source=plugin_source,
                    keep_temporary=args.keep_temporary,
                )
    except PackageSmokeError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
