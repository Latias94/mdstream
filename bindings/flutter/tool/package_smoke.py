#!/usr/bin/env python3
"""Validate the Flutter release archive and run a bundled-library smoke app."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path, PurePosixPath

from build_native import PLUGIN_ROOT, REPOSITORY_ROOT, REQUIRED_EXPORTS
from package_metadata import PackageMetadataError, package_archive_path

sys.path.insert(0, str(REPOSITORY_ROOT / "scripts"))

from archive_policy import (  # noqa: E402
    ArchiveLimits,
    ArchivePolicyError,
    extraction_path,
    read_archive,
)


BUDGET_PATH = REPOSITORY_ROOT / "bindings" / "budgets.json"
INTEGRATION_TEST = PLUGIN_ROOT / "integration_test" / "native_load_test.dart"
TEXT_IMPORT_PATTERN = re.compile(
    rb"(?:import|export)\s+['\"]package:([a-zA-Z0-9_]+)(?:/[^'\"]*)?['\"]"
)


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


def _native_format(data: bytes) -> str:
    prefix = data[:4]
    if prefix == b"\x7fELF":
        return "elf"
    if prefix[:2] == b"MZ":
        return "pe"
    if prefix in {
        b"\xfe\xed\xfa\xce",
        b"\xce\xfa\xed\xfe",
        b"\xfe\xed\xfa\xcf",
        b"\xcf\xfa\xed\xfe",
        b"\xca\xfe\xba\xbe",
        b"\xbe\xba\xfe\xca",
        b"\xca\xfe\xba\xbf",
        b"\xbf\xba\xfe\xca",
    }:
        return "macho"
    return "unknown"


def _native_binary(name: str) -> tuple[str, str, str] | None:
    parts = PurePosixPath(name).parts
    if len(parts) == 6 and parts[:4] == (
        "android",
        "src",
        "main",
        "jniLibs",
    ) and parts[5] == "libmdstream_ffi.so":
        return "android", f"android/{parts[4]}", "elf"
    if (
        len(parts) == 4
        and parts[0:2] == ("linux", "lib")
        and parts[3] == "libmdstream_ffi.so"
    ):
        return "linux", f"linux/{parts[2]}", "elf"
    if (
        len(parts) == 4
        and parts[0:2] == ("windows", "lib")
        and parts[3] == "mdstream_ffi.dll"
    ):
        return "windows", f"windows/{parts[2]}", "pe"
    if len(parts) >= 5 and parts[0] in {"ios", "macos"}:
        if parts[1] == "MdstreamFFI.xcframework" and parts[-1] == "MdstreamFFI":
            if parts[-2] == "MdstreamFFI.framework":
                return parts[0], f"{parts[0]}/{parts[2]}", "macho"
    return None


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

    native_groups: dict[str, int] = {}
    platform_static = {name: 0 for name in ("android", "ios", "macos", "linux", "windows")}
    native_sizes: list[int] = []
    platform_binaries: dict[str, set[str]] = {
        name: set() for name in platform_static
    }
    for name, data in entries.items():
        parts = PurePosixPath(name).parts
        platform_name = parts[0] if parts and parts[0] in platform_static else None
        grouped = _group_for_entry(name)
        if grouped is not None:
            _, group = grouped
            native_groups[group] = native_groups.get(group, 0) + len(data)
        elif platform_name is not None:
            platform_static[platform_name] += len(data)

        native = _native_binary(name)
        if native is None:
            continue
        native_platform, group, expected_format = native
        actual_format = _native_format(data)
        if actual_format != expected_format:
            raise PackageSmokeError(
                f"native magic mismatch for {name}: expected {expected_format}, got {actual_format}"
            )
        if len(data) > native_ceiling_bytes:
            raise PackageSmokeError(
                f"native library exceeds {native_ceiling_bytes}-byte ceiling: "
                f"{name} ({len(data)} bytes)"
            )
        missing_symbols = [
            symbol for symbol in REQUIRED_EXPORTS if symbol.encode("ascii") not in data
        ]
        if missing_symbols:
            raise PackageSmokeError(
                f"native library lacks required ABI symbol names "
                f"{', '.join(missing_symbols)}: {name}"
            )
        native_sizes.append(len(data))
        platform_binaries[native_platform].add(group)

    if not native_sizes:
        raise PackageSmokeError("publish archive contains no mdstream native library")

    expected_groups = {
        "android": {"android/arm64-v8a", "android/armeabi-v7a", "android/x86_64"},
        "linux": {"linux/x86_64"},
        "windows": {"windows/x64"},
    }
    if require_all_platforms:
        for platform_name in ("android", "ios", "macos", "linux", "windows"):
            if not platform_binaries[platform_name]:
                raise PackageSmokeError(
                    f"publish archive has no staged {platform_name} native library"
                )
        for platform_name, required in expected_groups.items():
            missing = sorted(required - platform_binaries[platform_name])
            if missing:
                raise PackageSmokeError(
                    f"publish archive is missing {platform_name} slice(s): "
                    f"{', '.join(missing)}"
                )
        if len(platform_binaries["ios"]) < 2:
            raise PackageSmokeError("iOS XCFramework must contain device and simulator slices")

    increments = {
        group: size + platform_static[group.split("/", 1)[0]]
        for group, size in native_groups.items()
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
        )
    except FileNotFoundError as error:
        raise PackageSmokeError(f"required tool not found: {command[0]}") from error
    except subprocess.CalledProcessError as error:
        detail = (error.stderr or error.stdout or "").strip()
        suffix = f": {detail}" if detail else ""
        raise PackageSmokeError(
            f"command failed with exit code {error.returncode}: {' '.join(command)}{suffix}"
        ) from error


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
    entries = _safe_archive_entries(archive)
    for name, data in entries.items():
        try:
            path = extraction_path(destination, name)
        except ArchivePolicyError as error:
            raise PackageSmokeError(str(error)) from error
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)


def run_runtime_smoke(
    *,
    platform_name: str,
    device: str,
    plugin_source: Path,
    keep_temporary: bool,
) -> None:
    if not INTEGRATION_TEST.is_file():
        raise PackageSmokeError(f"integration test does not exist: {INTEGRATION_TEST}")
    temporary = Path(
        tempfile.mkdtemp(prefix=f"mdstream-flutter-{platform_name}-")
    )
    try:
        _run(
            [
                "flutter",
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
        _run(
            [
                "flutter",
                "pub",
                "add",
                f"mdstream_flutter:{{path: {plugin_source.as_posix()}}}",
                f"override:mdstream:{{path: {(REPOSITORY_ROOT / 'bindings' / 'dart').as_posix()}}}",
                "dev:integration_test:{sdk: flutter}",
            ],
            cwd=temporary,
        )
        target = temporary / "integration_test" / INTEGRATION_TEST.name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(INTEGRATION_TEST, target)
        env = os.environ.copy()
        env.pop("MDSTREAM_NATIVE_LIBRARY", None)
        env.pop("MDSTREAM_FFI_LIBRARY", None)
        _run(
            [
                "flutter",
                "test",
                str(target.relative_to(temporary)),
                "-d",
                device,
            ],
            cwd=temporary,
            env=env,
        )
    finally:
        if keep_temporary:
            print(f"kept temporary smoke app: {temporary}")
        else:
            shutil.rmtree(temporary, ignore_errors=True)


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--host", action="store_true")
    mode.add_argument("--release", action="store_true")
    mode.add_argument("--archive", type=Path)
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
        _run(["flutter", "pub", "get"], cwd=REPOSITORY_ROOT / "bindings")
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
            if archive is not None:
                with tempfile.TemporaryDirectory(
                    prefix="mdstream-flutter-archive-"
                ) as temporary:
                    extracted = Path(temporary) / "plugin"
                    _extract_archive(archive, extracted)
                    run_runtime_smoke(
                        platform_name=platform_name,
                        device=device,
                        plugin_source=extracted,
                        keep_temporary=args.keep_temporary,
                    )
            else:
                run_runtime_smoke(
                    platform_name=platform_name,
                    device=device,
                    plugin_source=PLUGIN_ROOT,
                    keep_temporary=args.keep_temporary,
                )
    except PackageSmokeError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
