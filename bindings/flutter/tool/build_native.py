#!/usr/bin/env python3
"""Build, validate, and atomically stage mdstream Flutter native libraries."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform as host_platform_module
import plistlib
import re
import shutil
import subprocess
import sys
import tempfile
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

from package_metadata import (
    PLUGIN_ROOT,
    REPOSITORY_ROOT,
    PackageMetadataError,
    package_version,
)
from native_artifact import (
    FRAMEWORK_MODULE_MAP,
    LINUX_GLIBC_BASELINE,
    NATIVE_CONTRACTS,
    NativeArtifactError,
    NativeContract,
    inspect_xcframework,
    validate_native_image,
)


BUDGET_PATH = REPOSITORY_ROOT / "bindings" / "budgets.json"
HEADER_PATH = REPOSITORY_ROOT / "mdstream-ffi" / "include" / "mdstream.h"
FRAMEWORK_NAME = "MdstreamFFI"
ANDROID_NDK_VERSION = "26.3.11579264"
IOS_DEPLOYMENT_TARGET = "14.0"
MACOS_DEPLOYMENT_TARGET = "11.0"
REQUIRED_EXPORTS = (
    "mdstream_abi_version",
    "mdstream_package_version",
    "mdstream_engine_new",
    "mdstream_reducer_new",
)

ANDROID_TARGETS = {
    "aarch64-linux-android": "arm64-v8a",
    "armv7-linux-androideabi": "armeabi-v7a",
    "x86_64-linux-android": "x86_64",
}
IOS_TARGETS = (
    "aarch64-apple-ios",
    "aarch64-apple-ios-sim",
    "x86_64-apple-ios",
)
MACOS_TARGETS = (
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
)
LINUX_TARGETS = {
    "x86_64-unknown-linux-gnu": "x86_64",
}
LINUX_SONAME = "libmdstream_ffi.so"
WINDOWS_TARGETS = {
    "x86_64-pc-windows-msvc": "x64",
}


class PackagingError(RuntimeError):
    """Raised when a native artifact cannot satisfy the package contract."""


@dataclass(frozen=True)
class BuildOptions:
    profile: str
    toolchain: str
    install_targets: bool
    skip_strip: bool
    ndk_home: Path | None


@dataclass(frozen=True)
class StagedArtifact:
    platform: str
    target: str
    path: Path
    size: int
    sha256: str


def load_budget_ceiling(artifact: str) -> int:
    try:
        budget = json.loads(BUDGET_PATH.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise PackagingError(f"failed to load binding budgets: {error}") from error
    for entry in budget.get("artifacts", []):
        if isinstance(entry, dict) and entry.get("artifact") == artifact:
            ceiling = entry.get("ceiling_bytes")
            if isinstance(ceiling, int) and ceiling > 0:
                return ceiling
    raise PackagingError(f"binding budget does not define {artifact}")


def validate_native_artifact(
    path: Path,
    *,
    contract: NativeContract,
    ceiling_bytes: int,
    check_exports: bool = True,
    symbol_tool: Path | None = None,
) -> int:
    if not path.is_file():
        raise PackagingError(f"native artifact does not exist: {path}")
    try:
        validate_native_image(path.read_bytes(), contract)
    except (OSError, NativeArtifactError) as error:
        raise PackagingError(f"native artifact contract mismatch for {path}: {error}") from error
    size = path.stat().st_size
    if size > ceiling_bytes:
        raise PackagingError(
            f"native artifact exceeds {ceiling_bytes}-byte ceiling: {path} ({size} bytes)"
        )
    if check_exports:
        exported = _exported_symbols(
            path, native_format=contract.format, symbol_tool=symbol_tool
        )
        missing = [symbol for symbol in REQUIRED_EXPORTS if symbol not in exported]
        if missing:
            raise PackagingError(
                f"native export table is missing {', '.join(missing)}: {path}"
            )
    return size


def _exported_symbols(
    path: Path, *, native_format: str, symbol_tool: Path | None
) -> set[str]:
    if symbol_tool is not None:
        tool = str(symbol_tool)
    elif native_format == "pe":
        tool = (
            shutil.which("dumpbin")
            or shutil.which("llvm-readobj")
            or shutil.which("llvm-nm")
            or ""
        )
    else:
        tool = shutil.which("llvm-nm") or shutil.which("nm") or ""
    if not tool:
        raise PackagingError(f"no export-table inspection tool is available for {path}")

    tool_name = Path(tool).name.lower()
    if "dumpbin" in tool_name:
        command = [tool, "/exports", str(path)]
    elif "readobj" in tool_name:
        command = [tool, "--coff-exports", str(path)]
    elif native_format == "macho":
        command = [tool, "-gU", str(path)]
    elif native_format == "elf":
        command = [tool, "-D", "--defined-only", str(path)]
    else:
        command = [tool, "--defined-only", "--extern-only", str(path)]

    result = _run(command, capture=True)
    tokens = re.findall(
        r"(?<![A-Za-z0-9_])_?mdstream_[A-Za-z0-9_]+(?![A-Za-z0-9_])",
        result.stdout,
    )
    return {
        token[1:] if token.startswith("_mdstream_") else token
        for token in tokens
    }


def atomic_stage(source: Path, destination: Path) -> None:
    """Replace a staged directory without exposing a partially copied tree."""
    if not source.is_dir():
        raise PackagingError(f"staging source is not a directory: {source}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    incoming = destination.parent / f".{destination.name}.incoming-{uuid.uuid4().hex}"
    backup = destination.parent / f".{destination.name}.backup-{uuid.uuid4().hex}"
    shutil.copytree(source, incoming, symlinks=True)
    moved_existing = False
    try:
        if destination.exists():
            os.replace(destination, backup)
            moved_existing = True
        os.replace(incoming, destination)
    except Exception:
        if destination.exists():
            shutil.rmtree(destination)
        if moved_existing and backup.exists():
            os.replace(backup, destination)
        raise
    finally:
        if incoming.exists():
            shutil.rmtree(incoming)
        if backup.exists():
            shutil.rmtree(backup)


def _run(
    command: list[str],
    *,
    cwd: Path = REPOSITORY_ROOT,
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
        raise PackagingError(f"required tool not found: {command[0]}") from error
    except subprocess.CalledProcessError as error:
        detail = (error.stderr or error.stdout or "").strip()
        suffix = f": {detail}" if detail else ""
        raise PackagingError(
            f"command failed with exit code {error.returncode}: {' '.join(command)}{suffix}"
        ) from error


def _installed_targets(toolchain: str) -> set[str]:
    result = _run(
        ["rustup", "target", "list", "--installed", "--toolchain", toolchain],
        capture=True,
    )
    return {line.strip() for line in result.stdout.splitlines() if line.strip()}


def _ensure_targets(targets: Iterable[str], options: BuildOptions) -> None:
    installed = _installed_targets(options.toolchain)
    missing = [target for target in targets if target not in installed]
    if not missing:
        return
    if not options.install_targets:
        joined = " ".join(missing)
        raise PackagingError(
            f"missing Rust target(s): {joined}; install explicitly with "
            f"rustup target add --toolchain {options.toolchain} {joined}, or pass "
            "--install-targets"
        )
    _run(
        [
            "rustup",
            "target",
            "add",
            "--toolchain",
            options.toolchain,
            *missing,
        ]
    )


def _cargo_artifact(
    target: str,
    options: BuildOptions,
    *,
    env: dict[str, str] | None = None,
    cargo_subcommand: str = "build",
    cargo_target: str | None = None,
) -> Path:
    command = [
        "cargo",
        f"+{options.toolchain}",
        cargo_subcommand,
        "--locked",
        "--manifest-path",
        str(REPOSITORY_ROOT / "Cargo.toml"),
        "-p",
        "mdstream-ffi",
        "--target",
        cargo_target or target,
        "--message-format=json-render-diagnostics",
    ]
    if options.profile == "release":
        command.append("--release")
    result = _run(command, env=env, capture=True)
    artifact: Path | None = None
    for line in result.stdout.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        target_info = message.get("target", {})
        if (
            message.get("reason") != "compiler-artifact"
            or target_info.get("name") != "mdstream_ffi"
            or "cdylib" not in target_info.get("kind", [])
        ):
            continue
        for filename in message.get("filenames", []):
            candidate = Path(filename)
            if candidate.suffix.lower() in {".so", ".dylib", ".dll"}:
                artifact = candidate
    if artifact is None or not artifact.is_file():
        raise PackagingError(f"Cargo did not emit the mdstream-ffi cdylib for {target}")
    return artifact


def _copy_and_strip(
    source: Path,
    destination: Path,
    *,
    platform: str,
    options: BuildOptions,
    ndk_strip: Path | None = None,
) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)
    if options.skip_strip or options.profile != "release":
        return
    if platform in {"ios", "macos"}:
        _run(["xcrun", "strip", "-x", str(destination)])
        return
    if platform == "android":
        if ndk_strip is None or not ndk_strip.is_file():
            raise PackagingError("Android NDK llvm-strip is unavailable")
        _run([str(ndk_strip), "--strip-unneeded", str(destination)])
        return
    strip = shutil.which("llvm-strip") or shutil.which("strip")
    if strip is not None:
        _run([strip, "--strip-unneeded", str(destination)])
    elif platform != "windows":
        raise PackagingError(f"no strip tool is available for {platform}")


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _report(platform: str, target: str, path: Path) -> StagedArtifact:
    return StagedArtifact(
        platform=platform,
        target=target,
        path=path,
        size=path.stat().st_size,
        sha256=_sha256(path),
    )


def _host_system() -> str:
    system = host_platform_module.system()
    mapping = {"Darwin": "macos", "Linux": "linux", "Windows": "windows"}
    try:
        return mapping[system]
    except KeyError as error:
        raise PackagingError(f"unsupported host operating system: {system}") from error


def _host_architecture() -> str:
    machine = host_platform_module.machine().lower()
    if machine in {"x86_64", "amd64"}:
        return "x86_64"
    if machine in {"arm64", "aarch64"}:
        return "aarch64"
    raise PackagingError(f"unsupported host architecture: {machine}")


def _android_ndk_home(explicit: Path | None) -> Path:
    if explicit is not None:
        candidate = explicit.expanduser().resolve()
    else:
        from_environment = os.environ.get("ANDROID_NDK_HOME") or os.environ.get(
            "ANDROID_NDK_ROOT"
        )
        if from_environment:
            candidate = Path(from_environment).expanduser().resolve()
        else:
            sdk = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
            if not sdk:
                default_sdk = Path.home() / "Library" / "Android" / "sdk"
                if not default_sdk.is_dir():
                    raise PackagingError(
                        "Android NDK not found; set ANDROID_NDK_HOME or ANDROID_HOME"
                    )
                sdk = str(default_sdk)
            ndk_root = Path(sdk).expanduser().resolve() / "ndk"
            candidate = ndk_root / ANDROID_NDK_VERSION
            if not candidate.is_dir():
                raise PackagingError(
                    f"Android NDK {ANDROID_NDK_VERSION} is not installed under "
                    f"{ndk_root}; install it or pass --ndk-home explicitly"
                )
    if not candidate.is_dir():
        raise PackagingError(f"Android NDK directory does not exist: {candidate}")
    return candidate


def _ndk_host_tag() -> str:
    system = host_platform_module.system()
    if system == "Darwin":
        return "darwin-x86_64"
    if system == "Linux":
        return "linux-x86_64"
    if system == "Windows":
        return "windows-x86_64"
    raise PackagingError(f"unsupported Android build host: {system}")


def _android_clang_name(target: str) -> str:
    names = {
        "aarch64-linux-android": "aarch64-linux-android21-clang",
        "armv7-linux-androideabi": "armv7a-linux-androideabi21-clang",
        "x86_64-linux-android": "x86_64-linux-android21-clang",
    }
    name = names[target]
    return f"{name}.cmd" if host_platform_module.system() == "Windows" else name


def _android_build_environment(target: str, linker: Path) -> dict[str, str]:
    key = target.upper().replace("-", "_")
    env = os.environ.copy()
    env[f"CARGO_TARGET_{key}_LINKER"] = str(linker)
    env[f"CARGO_TARGET_{key}_RUSTFLAGS"] = (
        "-C link-arg=-Wl,-z,max-page-size=16384 "
        "-C link-arg=-Wl,-z,common-page-size=16384"
    )
    return env


def _linux_build_environment(target: str) -> dict[str, str]:
    """Give Linux consumers a stable shared-library identity."""
    key = target.upper().replace("-", "_")
    variable = f"CARGO_TARGET_{key}_RUSTFLAGS"
    env = os.environ.copy()
    soname_flag = f"-C link-arg=-Wl,-soname,{LINUX_SONAME}"
    existing = env.get(variable, "").strip()
    env[variable] = f"{existing} {soname_flag}".strip()
    return env


def maximum_glibc_requirement(path: Path) -> tuple[int, int] | None:
    tool = shutil.which("readelf") or shutil.which("llvm-readelf")
    if tool is None:
        raise PackagingError("readelf is required to validate the Linux glibc baseline")
    result = _run([tool, "--version-info", str(path)], capture=True)
    versions = {
        tuple(int(part) for part in match.groups())
        for match in re.finditer(r"GLIBC_(\d+)\.(\d+)", result.stdout)
    }
    return max(versions, default=None)


def build_android(
    targets: list[str], options: BuildOptions
) -> list[StagedArtifact]:
    unknown = sorted(set(targets) - set(ANDROID_TARGETS))
    if unknown:
        raise PackagingError(f"unsupported Android Rust target(s): {' '.join(unknown)}")
    _ensure_targets(targets, options)
    ndk = _android_ndk_home(options.ndk_home)
    toolchain = ndk / "toolchains" / "llvm" / "prebuilt" / _ndk_host_tag() / "bin"
    strip_name = "llvm-strip.exe" if host_platform_module.system() == "Windows" else "llvm-strip"
    ceiling = load_budget_ceiling("flutter_native_library")
    reports: list[StagedArtifact] = []
    with tempfile.TemporaryDirectory(prefix="mdstream-android-") as temporary:
        staged = Path(temporary) / "jniLibs"
        for target in targets:
            linker = toolchain / _android_clang_name(target)
            if not linker.is_file():
                raise PackagingError(f"Android NDK linker does not exist: {linker}")
            env = _android_build_environment(target, linker)
            artifact = _cargo_artifact(target, options, env=env)
            destination = staged / ANDROID_TARGETS[target] / "libmdstream_ffi.so"
            _copy_and_strip(
                artifact,
                destination,
                platform="android",
                options=options,
                ndk_strip=toolchain / strip_name,
            )
            validate_native_artifact(
                destination,
                contract=NATIVE_CONTRACTS[f"android/{ANDROID_TARGETS[target]}"],
                ceiling_bytes=ceiling,
                symbol_tool=toolchain / (
                    "llvm-nm.exe"
                    if host_platform_module.system() == "Windows"
                    else "llvm-nm"
                ),
            )
        atomic_stage(staged, PLUGIN_ROOT / "android" / "src" / "main" / "jniLibs")
    for target in targets:
        path = (
            PLUGIN_ROOT
            / "android"
            / "src"
            / "main"
            / "jniLibs"
            / ANDROID_TARGETS[target]
            / "libmdstream_ffi.so"
        )
        reports.append(_report("android", target, path))
    return reports


def _write_framework_metadata(
    framework: Path, *, platform_name: str, minimum_version: str
) -> None:
    headers = framework / "Headers"
    modules = framework / "Modules"
    resources = framework
    headers.mkdir(parents=True, exist_ok=True)
    modules.mkdir(parents=True, exist_ok=True)
    shutil.copy2(HEADER_PATH, headers / "mdstream.h")
    (modules / "module.modulemap").write_text(
        FRAMEWORK_MODULE_MAP,
        encoding="utf-8",
    )
    try:
        version = package_version()
    except PackageMetadataError as error:
        raise PackagingError(str(error)) from error
    plist = {
        "CFBundleDevelopmentRegion": "en",
        "CFBundleExecutable": FRAMEWORK_NAME,
        "CFBundleIdentifier": "io.mdstream.flutter.MdstreamFFI",
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleName": FRAMEWORK_NAME,
        "CFBundlePackageType": "FMWK",
        "CFBundleShortVersionString": version,
        "CFBundleVersion": version,
        "MinimumOSVersion": minimum_version,
        "CFBundleSupportedPlatforms": [platform_name],
    }
    with (resources / "Info.plist").open("wb") as handle:
        plistlib.dump(plist, handle, sort_keys=True)
def _make_framework(
    source: Path,
    destination: Path,
    *,
    platform_name: str,
    minimum_version: str,
    options: BuildOptions,
) -> Path:
    destination.mkdir(parents=True, exist_ok=True)
    binary = destination / FRAMEWORK_NAME
    _copy_and_strip(
        source,
        binary,
        platform="ios" if platform_name.startswith("iPhone") else "macos",
        options=options,
    )
    _run(
        [
            "install_name_tool",
            "-id",
            f"@rpath/{FRAMEWORK_NAME}.framework/{FRAMEWORK_NAME}",
            str(binary),
        ]
    )
    _write_framework_metadata(
        destination, platform_name=platform_name, minimum_version=minimum_version
    )
    return binary


def _create_xcframework(frameworks: list[Path], output: Path) -> None:
    command = ["xcodebuild", "-create-xcframework"]
    for framework in frameworks:
        command.extend(["-framework", str(framework)])
    command.extend(["-output", str(output)])
    _run(command)


def _validate_xcframework(
    path: Path, ceiling: int, platform_name: str
) -> list[Path]:
    info_path = path / "Info.plist"
    if not info_path.is_file():
        raise PackagingError(f"XCFramework Info.plist is missing: {path}")
    try:
        slices = inspect_xcframework(info_path.read_bytes(), platform_name)
    except (OSError, NativeArtifactError) as error:
        raise PackagingError(f"invalid XCFramework metadata for {path}: {error}") from error
    binaries: list[Path] = []
    for slice_ in slices:
        binary = path / slice_.binary_path
        validate_native_artifact(
            binary,
            contract=NATIVE_CONTRACTS[slice_.group],
            ceiling_bytes=ceiling,
        )
        binaries.append(binary)
    return binaries


def _xcframework_identifier(binary: Path) -> str:
    framework_marker = f"{FRAMEWORK_NAME}.framework"
    for parent in binary.parents:
        if parent.name == framework_marker:
            return parent.parent.name
    raise PackagingError(f"binary is not inside a {framework_marker}: {binary}")


def build_ios(options: BuildOptions) -> list[StagedArtifact]:
    if _host_system() != "macos":
        raise PackagingError("iOS artifacts must be built on macOS")
    _ensure_targets(IOS_TARGETS, options)
    ceiling = load_budget_ceiling("flutter_native_library")
    env = os.environ.copy()
    env["IPHONEOS_DEPLOYMENT_TARGET"] = IOS_DEPLOYMENT_TARGET
    artifacts = {
        target: _cargo_artifact(target, options, env=env) for target in IOS_TARGETS
    }
    with tempfile.TemporaryDirectory(prefix="mdstream-ios-") as temporary:
        root = Path(temporary)
        device_framework = root / "ios-arm64" / f"{FRAMEWORK_NAME}.framework"
        device_binary = _make_framework(
            artifacts["aarch64-apple-ios"],
            device_framework,
            platform_name="iPhoneOS",
            minimum_version=IOS_DEPLOYMENT_TARGET,
            options=options,
        )
        simulator_binary = root / "ios-simulator" / FRAMEWORK_NAME
        simulator_binary.parent.mkdir(parents=True, exist_ok=True)
        _run(
            [
                "lipo",
                "-create",
                str(artifacts["aarch64-apple-ios-sim"]),
                str(artifacts["x86_64-apple-ios"]),
                "-output",
                str(simulator_binary),
            ]
        )
        simulator_framework = (
            root / "ios-arm64_x86_64-simulator" / f"{FRAMEWORK_NAME}.framework"
        )
        simulator_framework_binary = _make_framework(
            simulator_binary,
            simulator_framework,
            platform_name="iPhoneSimulator",
            minimum_version=IOS_DEPLOYMENT_TARGET,
            options=options,
        )
        validate_native_artifact(
            device_binary,
            contract=NATIVE_CONTRACTS["ios/ios-arm64"],
            ceiling_bytes=ceiling,
        )
        validate_native_artifact(
            simulator_framework_binary,
            contract=NATIVE_CONTRACTS["ios/ios-arm64_x86_64-simulator"],
            ceiling_bytes=ceiling,
        )
        xcframework = root / f"{FRAMEWORK_NAME}.xcframework"
        _create_xcframework([device_framework, simulator_framework], xcframework)
        _validate_xcframework(xcframework, ceiling, "ios")
        atomic_stage(xcframework, PLUGIN_ROOT / "ios" / xcframework.name)
    output = PLUGIN_ROOT / "ios" / f"{FRAMEWORK_NAME}.xcframework"
    binaries = _validate_xcframework(output, ceiling, "ios")
    return [_report("ios", _xcframework_identifier(binary), binary) for binary in binaries]


def build_macos(
    targets: list[str], options: BuildOptions
) -> list[StagedArtifact]:
    if _host_system() != "macos":
        raise PackagingError("macOS artifacts must be built on macOS")
    unknown = sorted(set(targets) - set(MACOS_TARGETS))
    if unknown:
        raise PackagingError(f"unsupported macOS Rust target(s): {' '.join(unknown)}")
    if set(targets) != set(MACOS_TARGETS):
        raise PackagingError(
            "macOS staging requires both arm64 and x86_64 slices"
        )
    _ensure_targets(targets, options)
    ceiling = load_budget_ceiling("flutter_native_library")
    env = os.environ.copy()
    env["MACOSX_DEPLOYMENT_TARGET"] = MACOS_DEPLOYMENT_TARGET
    artifacts = {target: _cargo_artifact(target, options, env=env) for target in targets}
    with tempfile.TemporaryDirectory(prefix="mdstream-macos-") as temporary:
        root = Path(temporary)
        combined = root / FRAMEWORK_NAME
        if len(targets) == 1:
            shutil.copy2(artifacts[targets[0]], combined)
        else:
            _run(
                [
                    "lipo",
                    "-create",
                    *(str(artifacts[target]) for target in targets),
                    "-output",
                    str(combined),
                ]
            )
        framework = root / "macos-universal" / f"{FRAMEWORK_NAME}.framework"
        framework_binary = _make_framework(
            combined,
            framework,
            platform_name="MacOSX",
            minimum_version=MACOS_DEPLOYMENT_TARGET,
            options=options,
        )
        validate_native_artifact(
            framework_binary,
            contract=NATIVE_CONTRACTS["macos/macos-arm64_x86_64"],
            ceiling_bytes=ceiling,
        )
        xcframework = root / f"{FRAMEWORK_NAME}.xcframework"
        _create_xcframework([framework], xcframework)
        _validate_xcframework(xcframework, ceiling, "macos")
        atomic_stage(xcframework, PLUGIN_ROOT / "macos" / xcframework.name)
    output = PLUGIN_ROOT / "macos" / f"{FRAMEWORK_NAME}.xcframework"
    binaries = _validate_xcframework(output, ceiling, "macos")
    return [_report("macos", _xcframework_identifier(binary), binary) for binary in binaries]


def build_linux(
    targets: list[str], options: BuildOptions
) -> list[StagedArtifact]:
    if _host_system() != "linux":
        raise PackagingError("Linux artifacts must be built on Linux")
    unknown = sorted(set(targets) - set(LINUX_TARGETS))
    if unknown:
        raise PackagingError(f"unsupported Linux Rust target(s): {' '.join(unknown)}")
    _ensure_targets(targets, options)
    ceiling = load_budget_ceiling("flutter_native_library")
    with tempfile.TemporaryDirectory(prefix="mdstream-linux-") as temporary:
        staged = Path(temporary) / "lib"
        for target in targets:
            env = _linux_build_environment(target)
            artifact = _cargo_artifact(
                target,
                options,
                env=env,
                cargo_subcommand="zigbuild",
                cargo_target=(
                    f"{target}.{LINUX_GLIBC_BASELINE[0]}."
                    f"{LINUX_GLIBC_BASELINE[1]}"
                ),
            )
            destination = staged / LINUX_TARGETS[target] / "libmdstream_ffi.so"
            _copy_and_strip(
                artifact, destination, platform="linux", options=options
            )
            validate_native_artifact(
                destination,
                contract=NATIVE_CONTRACTS[f"linux/{LINUX_TARGETS[target]}"],
                ceiling_bytes=ceiling,
            )
            required_glibc = maximum_glibc_requirement(destination)
            if required_glibc is not None and required_glibc > LINUX_GLIBC_BASELINE:
                raise PackagingError(
                    "Linux artifact exceeds the declared glibc baseline "
                    f"{LINUX_GLIBC_BASELINE[0]}.{LINUX_GLIBC_BASELINE[1]}: "
                    f"requires {required_glibc[0]}.{required_glibc[1]}"
                )
        atomic_stage(staged, PLUGIN_ROOT / "linux" / "lib")
    return [
        _report(
            "linux",
            target,
            PLUGIN_ROOT
            / "linux"
            / "lib"
            / LINUX_TARGETS[target]
            / "libmdstream_ffi.so",
        )
        for target in targets
    ]


def build_windows(
    targets: list[str], options: BuildOptions
) -> list[StagedArtifact]:
    if _host_system() != "windows":
        raise PackagingError("Windows artifacts must be built on Windows")
    unknown = sorted(set(targets) - set(WINDOWS_TARGETS))
    if unknown:
        raise PackagingError(f"unsupported Windows Rust target(s): {' '.join(unknown)}")
    _ensure_targets(targets, options)
    ceiling = load_budget_ceiling("flutter_native_library")
    with tempfile.TemporaryDirectory(prefix="mdstream-windows-") as temporary:
        staged = Path(temporary) / "lib"
        for target in targets:
            artifact = _cargo_artifact(target, options)
            destination = staged / WINDOWS_TARGETS[target] / "mdstream_ffi.dll"
            _copy_and_strip(
                artifact, destination, platform="windows", options=options
            )
            validate_native_artifact(
                destination,
                contract=NATIVE_CONTRACTS[f"windows/{WINDOWS_TARGETS[target]}"],
                ceiling_bytes=ceiling,
            )
        atomic_stage(staged, PLUGIN_ROOT / "windows" / "lib")
    return [
        _report(
            "windows",
            target,
            PLUGIN_ROOT
            / "windows"
            / "lib"
            / WINDOWS_TARGETS[target]
            / "mdstream_ffi.dll",
        )
        for target in targets
    ]


def _default_desktop_target(platform_name: str) -> str:
    architecture = _host_architecture()
    if platform_name == "macos":
        return f"{architecture}-apple-darwin"
    if platform_name == "linux":
        if architecture != "x86_64":
            raise PackagingError("mdstream Flutter 0.4 supports Linux x86_64 only")
        return "x86_64-unknown-linux-gnu"
    if platform_name == "windows":
        if architecture != "x86_64":
            raise PackagingError("mdstream Flutter 0.4 supports Windows x86_64 only")
        return "x86_64-pc-windows-msvc"
    raise PackagingError(f"{platform_name} is not a desktop platform")


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "platform",
        choices=("host", "android", "ios", "macos", "linux", "windows"),
    )
    parser.add_argument("--targets", nargs="+", help="Override Rust target triples")
    parser.add_argument(
        "--profile", choices=("debug", "release"), default="release"
    )
    parser.add_argument("--toolchain", default="1.85.0")
    parser.add_argument(
        "--install-targets",
        action="store_true",
        help="Explicitly allow rustup to install missing Rust targets",
    )
    parser.add_argument("--skip-strip", action="store_true")
    parser.add_argument("--ndk-home", type=Path)
    return parser.parse_args()


def main() -> int:
    args = _parse_args()
    options = BuildOptions(
        profile=args.profile,
        toolchain=args.toolchain,
        install_targets=args.install_targets,
        skip_strip=args.skip_strip,
        ndk_home=args.ndk_home,
    )
    selected = _host_system() if args.platform == "host" else args.platform
    try:
        if selected == "android":
            reports = build_android(args.targets or list(ANDROID_TARGETS), options)
        elif selected == "ios":
            if args.targets:
                raise PackagingError("iOS target slices are fixed by the XCFramework contract")
            reports = build_ios(options)
        elif selected == "macos":
            reports = build_macos(args.targets or list(MACOS_TARGETS), options)
        elif selected == "linux":
            reports = build_linux(
                args.targets or [_default_desktop_target("linux")], options
            )
        elif selected == "windows":
            reports = build_windows(
                args.targets or [_default_desktop_target("windows")], options
            )
        else:
            raise PackagingError(f"unsupported platform: {selected}")
    except PackagingError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    payload = {
        "schema": "mdstream.flutter-native-artifacts/1",
        "artifacts": [
            {
                "platform": report.platform,
                "target": report.target,
                "path": str(report.path.relative_to(PLUGIN_ROOT)),
                "bytes": report.size,
                "sha256": report.sha256,
            }
            for report in reports
        ],
    }
    print(json.dumps(payload, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
