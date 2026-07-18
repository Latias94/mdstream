#!/usr/bin/env python3
"""Build and run a temporary Android app against bundled mdstream slices."""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import tempfile
import time
import zipfile
from pathlib import Path

from package_metadata import package_version

from build_native import PLUGIN_ROOT, REPOSITORY_ROOT
from package_smoke import PackageSmokeError


EXPECTED_APK_LIBRARIES = {
    "lib/arm64-v8a/libmdstream_ffi.so",
    "lib/armeabi-v7a/libmdstream_ffi.so",
    "lib/x86_64/libmdstream_ffi.so",
}
APPLICATION_ID = "io.mdstream.smoke.mdstream_flutter_android_smoke"
SMOKE_OK = "MDSTREAM_FLUTTER_SMOKE_OK"
SMOKE_ERROR = "MDSTREAM_FLUTTER_SMOKE_ERROR"


def _run(
    command: list[str], *, cwd: Path, capture: bool = False
) -> subprocess.CompletedProcess[str]:
    print("+", " ".join(command), flush=True)
    try:
        return subprocess.run(
            command,
            cwd=cwd,
            check=True,
            env=_clean_environment(),
            text=True,
            capture_output=capture,
        )
    except FileNotFoundError as error:
        raise PackageSmokeError(f"required tool not found: {command[0]}") from error
    except subprocess.CalledProcessError as error:
        raise PackageSmokeError(
            f"command failed with exit code {error.returncode}: {' '.join(command)}"
        ) from error


def _clean_environment() -> dict[str, str]:
    environment = os.environ.copy()
    environment.pop("MDSTREAM_NATIVE_LIBRARY", None)
    environment.pop("MDSTREAM_FFI_LIBRARY", None)
    return environment


def _write_smoke_main(path: Path) -> None:
    source = """import 'package:flutter/material.dart';
import 'package:mdstream_flutter/mdstream_flutter.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  try {
    final runtime = MdstreamFlutterRuntime.open();
    if (runtime.abiVersion != 1 ||
        runtime.packageVersion != '@PACKAGE_VERSION@' ||
        runtime.bindingSchema != 'mdstream.bindings/0.4') {
      throw StateError('unexpected mdstream runtime metadata');
    }
    final controller = MdstreamController.fromRuntime(runtime);
    try {
      controller.append('# Bundled runtime\\n\\nstreamed content');
      controller.finish();
      if (!controller.value.isFinalized ||
          controller.value.document?.roots?.children.isEmpty != false) {
        throw StateError('shared mdstream smoke trace did not finalize');
      }
    } finally {
      controller.dispose();
    }
    if (!runtime.nativeAllocations.isZero) {
      throw StateError('native allocations remain after controller disposal');
    }
    debugPrint('MDSTREAM_FLUTTER_SMOKE_OK abi=${runtime.abiVersion} '
        'version=${runtime.packageVersion}');
    runApp(const MaterialApp(home: Text('mdstream smoke passed')));
  } catch (error, stackTrace) {
    debugPrint('MDSTREAM_FLUTTER_SMOKE_ERROR $error');
    debugPrintStack(stackTrace: stackTrace);
    rethrow;
  }
}
"""
    path.write_text(
        source.replace("@PACKAGE_VERSION@", package_version()),
        encoding="utf-8",
    )


def _run_on_device(apk: Path, device: str) -> None:
    adb = ["adb", "-s", device]
    _run([*adb, "wait-for-device"], cwd=apk.parent)
    _run([*adb, "install", "-r", str(apk)], cwd=apk.parent)
    try:
        _run([*adb, "logcat", "-c"], cwd=apk.parent)
        _run(
            [
                *adb,
                "shell",
                "am",
                "start",
                "-W",
                "-n",
                f"{APPLICATION_ID}/.MainActivity",
            ],
            cwd=apk.parent,
        )
        deadline = time.monotonic() + 60.0
        latest = ""
        while time.monotonic() < deadline:
            latest = _run(
                [*adb, "logcat", "-d", "-v", "brief"],
                cwd=apk.parent,
                capture=True,
            ).stdout
            if SMOKE_OK in latest:
                print(next(line for line in latest.splitlines() if SMOKE_OK in line))
                return
            if SMOKE_ERROR in latest:
                break
            time.sleep(0.5)
        tail = "\n".join(latest.splitlines()[-80:])
        raise PackageSmokeError(
            f"Android runtime smoke did not report success on {device}:\n{tail}"
        )
    finally:
        _run([*adb, "uninstall", APPLICATION_ID], cwd=apk.parent)


def _build_and_inspect_apk(keep_temporary: bool, device: str | None) -> None:
    temporary = Path(tempfile.mkdtemp(prefix="mdstream-flutter-android-apk-"))
    try:
        _run(
            [
                "flutter",
                "create",
                "--platforms",
                "android",
                "--project-name",
                "mdstream_flutter_android_smoke",
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
                f"mdstream_flutter:{{path: {PLUGIN_ROOT.as_posix()}}}",
                f"override:mdstream:{{path: {(REPOSITORY_ROOT / 'bindings' / 'dart').as_posix()}}}",
            ],
            cwd=temporary,
        )
        _write_smoke_main(temporary / "lib" / "main.dart")
        _run(
            [
                "flutter",
                "build",
                "apk",
                "--debug",
                "--target-platform",
                "android-arm,android-arm64,android-x64",
            ],
            cwd=temporary,
        )
        apk = temporary / "build" / "app" / "outputs" / "flutter-apk" / "app-debug.apk"
        if not apk.is_file():
            raise PackageSmokeError(f"Flutter did not produce the expected APK: {apk}")
        with zipfile.ZipFile(apk) as archive:
            names = set(archive.namelist())
        missing = sorted(EXPECTED_APK_LIBRARIES - names)
        if missing:
            raise PackageSmokeError(
                f"Android APK is missing bundled slice(s): {', '.join(missing)}"
            )
        if device is not None:
            _run_on_device(apk, device)
    finally:
        if keep_temporary:
            print(f"kept temporary Android APK app: {temporary}")
        else:
            shutil.rmtree(temporary, ignore_errors=True)


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--device", default="emulator-5554")
    parser.add_argument("--skip-native-build", action="store_true")
    parser.add_argument("--build-only", action="store_true")
    parser.add_argument("--keep-temporary", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = _parse_args()
    try:
        if not args.skip_native_build:
            _run(
                [
                    sys.executable,
                    str(Path(__file__).with_name("build_native.py")),
                    "android",
                ],
                cwd=REPOSITORY_ROOT,
            )
        _build_and_inspect_apk(
            args.keep_temporary,
            None if args.build_only else args.device,
        )
    except PackageSmokeError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
