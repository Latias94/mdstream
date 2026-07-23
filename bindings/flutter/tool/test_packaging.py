#!/usr/bin/env python3

import io
import gzip
import json
import os
import plistlib
import struct
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


TOOL_ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(TOOL_ROOT))

import package_smoke  # noqa: E402
import android_smoke  # noqa: E402

from build_native import (  # noqa: E402
    ANDROID_TARGETS,
    BuildOptions,
    PackagingError,
    _android_ndk_home,
    _exported_symbols,
    _linux_build_environment,
    _make_framework,
    atomic_stage,
    validate_native_artifact,
)
from native_artifact import (  # noqa: E402
    FRAMEWORK_MODULE_MAP,
    NATIVE_CONTRACTS,
    NativeArtifactError,
    inspect_xcframework,
    validate_native_image,
)
from native_test_fixture import (  # noqa: E402
    REQUIRED_SYMBOL_TEXT as _REQUIRED_SYMBOL_TEXT,
    elf_image as _elf_image,
    required_symbols as _required_symbols,
)
from package_smoke import (  # noqa: E402
    PackageSmokeError,
    SWIFTPM_CONSUMER_NAME,
    _validate_swiftpm_manifest,
    _swiftpm_manifest_root,
    _write_swiftpm_consumer,
    _extract_archive,
    inspect_package_archive,
    validate_dependency_graph,
)
from package_metadata import package_archive_path, package_version  # noqa: E402


class BuildNativeContractTest(unittest.TestCase):
    def test_android_targets_cover_flutter_release_abis(self) -> None:
        self.assertEqual(
            ANDROID_TARGETS,
            {
                "aarch64-linux-android": "arm64-v8a",
                "armv7-linux-androideabi": "armeabi-v7a",
                "x86_64-linux-android": "x86_64",
            },
        )

    def test_linux_build_environment_sets_a_stable_soname(self) -> None:
        key = "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS"
        with patch.dict(
            os.environ,
            {key: "-C opt-level=2"},
            clear=True,
        ):
            environment = _linux_build_environment("x86_64-unknown-linux-gnu")

        self.assertEqual(
            environment[key],
            "-C opt-level=2 -C link-arg=-Wl,-soname,libmdstream_ffi.so",
        )

    def test_default_android_ndk_matches_the_gradle_pin(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            sdk = Path(temporary)
            for version in ("26.3.11579264", "29.0.13113456"):
                (sdk / "ndk" / version).mkdir(parents=True)
            with patch.dict(os.environ, {"ANDROID_HOME": str(sdk)}, clear=True):
                selected = _android_ndk_home(None)

            self.assertEqual(selected.name, "26.3.11579264")
            gradle = (
                TOOL_ROOT.parent / "android" / "build.gradle"
            ).read_text(encoding="utf-8")
            self.assertIn(f'ndkVersion = "{selected.name}"', gradle)
            workflow = (
                TOOL_ROOT.parents[2]
                / ".github"
                / "workflows"
                / "flutter-platforms.yml"
            ).read_text(encoding="utf-8")
            self.assertIn(
                f'"${{ANDROID_HOME}}/cmdline-tools/latest/bin/sdkmanager" '
                f'"ndk;{selected.name}" "build-tools;35.0.0"',
                workflow,
            )
            self.assertIn("target: google_apis_ps16k", workflow)
            android_smoke = (TOOL_ROOT / "android_smoke.py").read_text(
                encoding="utf-8"
            )
            self.assertIn('"-c", "-P", "16", "-v", "4"', android_smoke)
            self.assertIn('"getconf", "PAGE_SIZE"', android_smoke)

    def test_ci_resolves_the_independent_example_before_package_analysis(self) -> None:
        workflow = (
            TOOL_ROOT.parents[2]
            / ".github"
            / "workflows"
            / "flutter-platforms.yml"
        ).read_text(encoding="utf-8")
        jobs = {
            "linux": workflow[
                workflow.index("  linux:\n") : workflow.index("  android:\n")
            ],
            "windows": workflow[
                workflow.index("  windows:\n") : workflow.index("  package:\n")
            ],
        }
        resolve = (
            "      - name: Resolve Golden stream example\n"
            "        working-directory: bindings/flutter/example\n"
            "        run: flutter pub get"
        )
        analyze = (
            "      - name: Analyze Flutter package\n"
            "        working-directory: bindings/flutter\n"
            "        run: flutter analyze"
        )
        generate_linux_host = (
            "      - name: Generate Linux Golden stream host\n"
            "        working-directory: bindings/flutter/example\n"
            "        run: flutter create --empty --platforms linux "
            "--project-name mdstream_flutter_example "
            "--org io.mdstream.example --no-pub ."
        )
        integration_smoke = (
            "      - name: Test bundled Golden stream bootstrap\n"
            "        working-directory: bindings/flutter/example\n"
            "        run: xvfb-run -a flutter test "
            "integration_test/golden_stream_smoke_test.dart -d linux"
        )

        for name, job in jobs.items():
            with self.subTest(job=name):
                self.assertIn(resolve, job)
                self.assertIn(analyze, job)
                self.assertLess(job.index(resolve), job.index(analyze))

        self.assertIn("    runs-on: windows-2022", jobs["windows"])

        linux = jobs["linux"]
        self.assertIn(generate_linux_host, linux)
        self.assertIn(integration_smoke, linux)
        self.assertIn("uses: mlugg/setup-zig@v2", linux)
        self.assertIn("tool: cargo-zigbuild@0.23.0", linux)
        self.assertLess(
            linux.index(generate_linux_host),
            linux.index(integration_smoke),
        )
        self.assertIn("package-linux-legacy-smoke:", workflow)
        self.assertIn("debian:10.13-slim /mdstream/c-consumer", workflow)
        self.assertIn("target/flutter-extracted/linux/lib/x86_64", workflow)

    def test_atomic_stage_replaces_complete_directory(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            destination = root / "staged"
            destination.mkdir()
            (destination / "stale.so").write_bytes(b"stale")

            source = root / "source"
            source.mkdir()
            (source / "libmdstream_ffi.so").write_bytes(b"\x7fELFpayload")

            atomic_stage(source, destination)

            self.assertFalse((destination / "stale.so").exists())
            self.assertEqual(
                (destination / "libmdstream_ffi.so").read_bytes(),
                b"\x7fELFpayload",
            )

    def test_atomic_stage_preserves_relative_framework_symlinks(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source"
            version = source / "Versions" / "A"
            version.mkdir(parents=True)
            (version / "MdstreamFFI").write_bytes(b"binary")
            (source / "Versions" / "Current").symlink_to("A")
            (source / "MdstreamFFI").symlink_to(
                Path("Versions") / "Current" / "MdstreamFFI"
            )

            destination = root / "staged"
            atomic_stage(source, destination)

            self.assertTrue((destination / "Versions" / "Current").is_symlink())
            self.assertEqual(
                (destination / "Versions" / "Current").readlink(), Path("A")
            )
            self.assertTrue((destination / "MdstreamFFI").is_symlink())
            self.assertEqual(
                (destination / "MdstreamFFI").readlink(),
                Path("Versions") / "Current" / "MdstreamFFI",
            )

    def test_macos_framework_is_shallow_before_cocoapods_prepares_it(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "libmdstream_ffi.dylib"
            source.write_bytes(b"binary")
            framework = root / "MdstreamFFI.framework"
            options = BuildOptions(
                profile="debug",
                toolchain="1.85.0",
                install_targets=False,
                skip_strip=True,
                ndk_home=None,
            )

            with patch("build_native._run"), patch(
                "build_native._copy_and_strip",
                side_effect=lambda source, destination, **_: destination.write_bytes(
                    source.read_bytes()
                ),
            ):
                binary = _make_framework(
                    source,
                    framework,
                    platform_name="MacOSX",
                    minimum_version="11.0",
                    options=options,
                )

            self.assertEqual(binary, framework / "MdstreamFFI")
            self.assertTrue((framework / "Info.plist").is_file())
            self.assertTrue((framework / "Headers" / "mdstream.h").is_file())
            self.assertTrue((framework / "Modules" / "module.modulemap").is_file())
            self.assertFalse((framework / "Versions").exists())

    def test_macos_cocoapods_framework_prepare_script_is_shell_valid(self) -> None:
        script = TOOL_ROOT.parent / "macos" / "prepare_macos_framework.sh"
        result = subprocess.run(
            ["sh", "-n", str(script)],
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        podspec = (TOOL_ROOT.parent / "macos" / "mdstream_flutter.podspec").read_text(
            encoding="utf-8"
        )
        self.assertIn("prepare_macos_framework.sh", podspec)
        self.assertIn(":always_out_of_date => '1'", podspec)

    def test_macos_cocoapods_prepare_script_versions_a_shallow_framework(self) -> None:
        script = TOOL_ROOT.parent / "macos" / "prepare_macos_framework.sh"
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source_root = root / "plugin" / "macos"
            framework = (
                source_root
                / "MdstreamFFI.xcframework"
                / "macos-arm64_x86_64"
                / "MdstreamFFI.framework"
            )
            (framework / "Headers").mkdir(parents=True)
            (framework / "Modules").mkdir()
            (framework / "MdstreamFFI").write_bytes(b"binary")
            (framework / "Info.plist").write_text("plist", encoding="utf-8")
            (framework / "Headers" / "mdstream.h").write_text(
                "header", encoding="utf-8"
            )
            (framework / "Modules" / "module.modulemap").write_text(
                "module", encoding="utf-8"
            )

            env = os.environ.copy()
            env["PODS_XCFRAMEWORKS_BUILD_DIR"] = str(root / "intermediates")
            env["PODS_TARGET_SRCROOT"] = str(source_root)
            for _ in range(2):
                result = subprocess.run(
                    ["sh", str(script)],
                    env=env,
                    check=False,
                    capture_output=True,
                    text=True,
                )
                self.assertEqual(result.returncode, 0, result.stderr)

            self.assertEqual(
                (framework / "Versions" / "A" / "MdstreamFFI").read_bytes(),
                b"binary",
            )
            self.assertEqual(
                (framework / "Versions" / "A" / "Resources" / "Info.plist").read_text(
                    encoding="utf-8"
                ),
                "plist",
            )
            links = {
                "MdstreamFFI": "Versions/Current/MdstreamFFI",
                "Headers": "Versions/Current/Headers",
                "Modules": "Versions/Current/Modules",
                "Resources": "Versions/Current/Resources",
            }
            for name, target in links.items():
                with self.subTest(link=name):
                    self.assertTrue((framework / name).is_symlink())
                    self.assertEqual((framework / name).readlink().as_posix(), target)

            package_smoke._restore_macos_frameworks(root / "plugin")
            self.assertFalse((framework / "Versions").exists())
            self.assertEqual((framework / "MdstreamFFI").read_bytes(), b"binary")
            self.assertTrue((framework / "Info.plist").is_file())
            self.assertTrue((framework / "Headers" / "mdstream.h").is_file())
            self.assertTrue(
                (framework / "Modules" / "module.modulemap").is_file()
            )

            env.pop("PODS_XCFRAMEWORKS_BUILD_DIR")
            result = subprocess.run(
                ["sh", str(script)],
                env=env,
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue((framework / "Versions" / "Current").is_symlink())
            package_smoke._restore_macos_frameworks(root / "plugin")

    def test_native_validation_checks_magic_and_absolute_ceiling(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            library = Path(temporary) / "libmdstream_ffi.so"
            native = _elf_image("x86_64")
            library.write_bytes(native)
            validate_native_artifact(
                library,
                contract=NATIVE_CONTRACTS["linux/x86_64"],
                ceiling_bytes=len(native),
                check_exports=False,
            )

            with self.assertRaisesRegex(PackagingError, "ceiling"):
                validate_native_artifact(
                    library,
                    contract=NATIVE_CONTRACTS["linux/x86_64"],
                    ceiling_bytes=len(native) - 1,
                    check_exports=False,
                )

            library.write_bytes(b"not-an-elf")
            with self.assertRaisesRegex(PackagingError, "contract mismatch"):
                validate_native_artifact(
                    library,
                    contract=NATIVE_CONTRACTS["linux/x86_64"],
                    ceiling_bytes=len(native),
                    check_exports=False,
                )

    def test_native_validation_checks_architecture_alignment_and_apple_platform(
        self,
    ) -> None:
        validate_native_image(
            _elf_image("arm64", alignment=16 * 1024),
            NATIVE_CONTRACTS["android/arm64-v8a"],
        )
        with self.assertRaisesRegex(NativeArtifactError, "alignment"):
            validate_native_image(
                _elf_image("arm64", alignment=4 * 1024),
                NATIVE_CONTRACTS["android/arm64-v8a"],
            )
        with self.assertRaisesRegex(NativeArtifactError, "architecture"):
            validate_native_image(
                _elf_image("x86_64"),
                NATIVE_CONTRACTS["android/arm64-v8a"],
            )
        with self.assertRaisesRegex(NativeArtifactError, "platform"):
            validate_native_image(
                _macho_image("arm64", platform=1),
                NATIVE_CONTRACTS["ios/ios-arm64"],
            )
        with self.assertRaisesRegex(NativeArtifactError, "minimum OS version"):
            validate_native_image(
                _macho_image("arm64", platform=2, minimum=(13, 0, 0)),
                NATIVE_CONTRACTS["ios/ios-arm64"],
            )
        validate_native_image(
            _pe_image("x86_64"),
            NATIVE_CONTRACTS["windows/x64"],
        )
        fat = bytearray(_fat_macho_image(("arm64", "x86_64"), platform=7))
        struct.pack_into(">I", fat, 16, len(fat) + 1)
        with self.assertRaisesRegex(NativeArtifactError, "slice range"):
            validate_native_image(
                bytes(fat),
                NATIVE_CONTRACTS["ios/ios-arm64_x86_64-simulator"],
            )

    def test_native_validation_rejects_header_only_elf_and_pe_images(self) -> None:
        with self.assertRaisesRegex(NativeArtifactError, "dynamic|shared object"):
            validate_native_image(
                _header_only_elf_image("x86_64"),
                NATIVE_CONTRACTS["linux/x86_64"],
            )
        with self.assertRaisesRegex(NativeArtifactError, "optional header|DLL|section"):
            validate_native_image(
                _header_only_pe_image("x86_64"),
                NATIVE_CONTRACTS["windows/x64"],
            )

    def test_native_validation_rejects_invalid_or_forwarded_exports(self) -> None:
        with self.assertRaisesRegex(NativeArtifactError, "no exports"):
            validate_native_image(
                _elf_image("x86_64", export_section_index=99),
                NATIVE_CONTRACTS["linux/x86_64"],
            )
        with self.assertRaisesRegex(NativeArtifactError, "forwarded exports"):
            validate_native_image(
                _pe_image("x86_64", forwarded=True),
                NATIVE_CONTRACTS["windows/x64"],
            )

    def test_xcframework_metadata_requires_exact_slice_inventory(self) -> None:
        ios = _xcframework_plist("ios")
        slices = inspect_xcframework(ios, "ios")
        self.assertEqual(
            {slice_.group for slice_ in slices},
            {
                "ios/ios-arm64",
                "ios/ios-arm64_x86_64-simulator",
            },
        )
        value = plistlib.loads(ios)
        value["AvailableLibraries"][0]["SupportedPlatform"] = "macos"
        with self.assertRaisesRegex(NativeArtifactError, "supported platform"):
            inspect_xcframework(plistlib.dumps(value), "ios")

    def test_symbol_text_without_an_export_table_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            library = Path(temporary) / "libmdstream_ffi.so"
            library.write_bytes(_elf_image("x86_64"))
            with patch(
                "build_native._exported_symbols",
                return_value={f"{name}_v2" for name in _required_symbols()},
            ), self.assertRaisesRegex(PackagingError, "export table is missing"):
                validate_native_artifact(
                    library,
                    contract=NATIVE_CONTRACTS["linux/x86_64"],
                    ceiling_bytes=len(library.read_bytes()),
                    check_exports=True,
                )

    def test_external_symbol_inspection_uses_exact_names(self) -> None:
        output = "\n".join(
            (
                "0000 T mdstream_abi_version_v2",
                "0000 T mdstream_package_version_v2",
                "0000 T mdstream_engine_new_v2",
                "0000 T mdstream_reducer_new_v2",
            )
        )
        completed = subprocess.CompletedProcess([], 0, stdout=output, stderr="")
        with patch("build_native._run", return_value=completed):
            exported = _exported_symbols(
                Path("library.so"),
                native_format="elf",
                symbol_tool=Path("llvm-nm"),
            )
        self.assertTrue(exported.isdisjoint(_required_symbols()))


class PackageSmokeContractTest(unittest.TestCase):
    def test_android_command_timeout_is_phase_named_and_bounded(self) -> None:
        diagnostic = "x" * (android_smoke.ANDROID_DIAGNOSTIC_CHARS * 2)
        with patch.object(
            android_smoke.subprocess,
            "run",
            side_effect=subprocess.TimeoutExpired(
                ["adb", "install"],
                android_smoke.ANDROID_PHASE_TIMEOUT_SECONDS["adb-install"],
                output=diagnostic,
            ),
        ), patch("builtins.print"):
            with self.assertRaises(PackageSmokeError) as raised:
                android_smoke._run(
                    ["adb", "install"],
                    cwd=TOOL_ROOT,
                    phase="adb-install",
                    capture=True,
                )

        message = str(raised.exception)
        self.assertIn("adb-install timed out", message)
        self.assertIn("[truncated]", message)
        self.assertLessEqual(
            len(message),
            android_smoke.ANDROID_DIAGNOSTIC_CHARS + 256,
        )

    def test_android_device_smoke_assigns_every_command_a_timeout(self) -> None:
        calls: list[tuple[list[str], str]] = []

        def fake_run(
            command: list[str],
            *,
            cwd: Path,
            phase: str,
            capture: bool = False,
        ) -> subprocess.CompletedProcess[str]:
            del cwd, capture
            self.assertGreater(android_smoke.ANDROID_PHASE_TIMEOUT_SECONDS[phase], 0)
            calls.append((command, phase))
            stdout = ""
            if phase == "adb-page-size":
                stdout = "16384\n"
            elif phase == "adb-logcat-read":
                stdout = f"{android_smoke.SMOKE_OK} abi=1\n"
            return subprocess.CompletedProcess(command, 0, stdout=stdout, stderr="")

        with patch.object(android_smoke, "_run", side_effect=fake_run):
            android_smoke._run_on_device(Path("smoke.apk"), "emulator-5554")

        self.assertEqual(
            [phase for _, phase in calls],
            [
                "adb-wait-for-device",
                "adb-page-size",
                "adb-install",
                "adb-logcat-clear",
                "adb-launch",
                "adb-logcat-read",
                "adb-uninstall",
            ],
        )

    def test_android_install_failure_does_not_attempt_cleanup(self) -> None:
        phases: list[str] = []

        def fake_run(
            command: list[str],
            *,
            cwd: Path,
            phase: str,
            capture: bool = False,
        ) -> subprocess.CompletedProcess[str]:
            del cwd, capture
            phases.append(phase)
            if phase == "adb-page-size":
                return subprocess.CompletedProcess(
                    command,
                    0,
                    stdout="16384\n",
                    stderr="",
                )
            if phase == "adb-install":
                raise PackageSmokeError("install failed")
            return subprocess.CompletedProcess(command, 0, stdout="", stderr="")

        with patch.object(android_smoke, "_run", side_effect=fake_run):
            with self.assertRaisesRegex(PackageSmokeError, "install failed"):
                android_smoke._run_on_device(Path("smoke.apk"), "emulator-5554")

        self.assertNotIn("adb-uninstall", phases)

    def test_android_cleanup_does_not_replace_the_primary_failure(self) -> None:
        primary = PackageSmokeError("launch failed")

        def fake_run(
            command: list[str],
            *,
            cwd: Path,
            phase: str,
            capture: bool = False,
        ) -> subprocess.CompletedProcess[str]:
            del cwd, capture
            if phase == "adb-page-size":
                return subprocess.CompletedProcess(
                    command,
                    0,
                    stdout="16384\n",
                    stderr="",
                )
            if phase == "adb-launch":
                raise primary
            if phase == "adb-uninstall":
                raise PackageSmokeError("uninstall failed")
            return subprocess.CompletedProcess(command, 0, stdout="", stderr="")

        with patch.object(android_smoke, "_run", side_effect=fake_run):
            with self.assertRaises(PackageSmokeError) as raised:
                android_smoke._run_on_device(Path("smoke.apk"), "emulator-5554")

        self.assertIs(raised.exception, primary)
        self.assertEqual(str(raised.exception), "launch failed")
        self.assertEqual(
            raised.exception.__notes__,
            ["Android cleanup also failed: uninstall failed"],
        )

    def test_android_cleanup_failure_alone_fails_the_smoke(self) -> None:
        def fake_run(
            command: list[str],
            *,
            cwd: Path,
            phase: str,
            capture: bool = False,
        ) -> subprocess.CompletedProcess[str]:
            del cwd, capture
            if phase == "adb-page-size":
                return subprocess.CompletedProcess(
                    command,
                    0,
                    stdout="16384\n",
                    stderr="",
                )
            if phase == "adb-logcat-read":
                return subprocess.CompletedProcess(
                    command,
                    0,
                    stdout=f"{android_smoke.SMOKE_OK} abi=1\n",
                    stderr="",
                )
            if phase == "adb-uninstall":
                raise PackageSmokeError("uninstall failed")
            return subprocess.CompletedProcess(command, 0, stdout="", stderr="")

        with patch.object(android_smoke, "_run", side_effect=fake_run):
            with self.assertRaisesRegex(PackageSmokeError, "uninstall failed"):
                android_smoke._run_on_device(Path("smoke.apk"), "emulator-5554")

    def test_android_workflow_has_an_outer_timeout(self) -> None:
        workflow = (
            TOOL_ROOT.parents[2]
            / ".github"
            / "workflows"
            / "flutter-platforms.yml"
        ).read_text(encoding="utf-8")
        android_job = workflow[
            workflow.index("  android:\n") : workflow.index("  apple:\n")
        ]
        self.assertIn("    timeout-minutes: 60", android_job)

    def test_flutter_tool_uses_the_windows_batch_entrypoint(self) -> None:
        with patch.object(package_smoke.sys, "platform", "win32"):
            self.assertEqual(package_smoke._flutter_tool(), "flutter.bat")
        with patch.object(package_smoke.sys, "platform", "linux"):
            self.assertEqual(package_smoke._flutter_tool(), "flutter")

    def test_swiftpm_consumer_imports_plugin_and_probes_bundled_abi(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            package_root = root / "ios" / "mdstream_flutter"
            _write_swiftpm_consumer(root / "consumer", "ios", package_root)

            package = (root / "consumer" / "Package.swift").read_text(
                encoding="utf-8"
            )
            source = (
                root
                / "consumer"
                / "Sources"
                / SWIFTPM_CONSUMER_NAME
                / "MdstreamSwiftPMSmoke.swift"
            ).read_text(encoding="utf-8")
            tests = (
                root
                / "consumer"
                / "Tests"
                / SWIFTPM_CONSUMER_NAME
                / "MdstreamSwiftPMSmokeTests.swift"
            ).read_text(encoding="utf-8")

            self.assertIn(
                '.product(name: "mdstream-flutter", package: "mdstream_flutter")',
                package,
            )
            self.assertIn("platforms: [.iOS(.v14)]", package)
            self.assertIn("import mdstream_flutter", source)
            self.assertIn("mdstream_abi_version()", source)
            self.assertIn("mdstream_package_version()", source)
            self.assertIn("testBundledLibraryLoads", tests)
            self.assertFalse((root / "consumer" / "Podfile").exists())

    def test_ios_runtime_smoke_does_not_depend_on_the_vm_debug_log_reader(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            project = root / "project"
            (project / "lib").mkdir(parents=True)
            app = project / "build" / "ios" / "iphonesimulator" / "Runner.app"
            app.mkdir(parents=True)
            (app / "Info.plist").write_bytes(
                plistlib.dumps(
                    {
                        "CFBundleIdentifier": (
                            "io.mdstream.smoke.mdstreamFlutterSmoke"
                        )
                    }
                )
            )
            container = root / "container"
            (container / "tmp").mkdir(parents=True)
            result_path = (
                container / "tmp" / package_smoke.IOS_RUNTIME_SMOKE_RESULT
            )
            result_path.write_text("stale", encoding="utf-8")
            commands: list[list[str]] = []

            def fake_run(
                command: list[str],
                *,
                cwd: Path,
                env: dict[str, str] | None = None,
                capture: bool = False,
                timeout: float | None = None,
            ) -> subprocess.CompletedProcess[str]:
                del cwd, env
                commands.append(command)
                if command[0] == "xcrun":
                    self.assertIsNotNone(timeout)
                if command[:3] == ["xcrun", "simctl", "get_app_container"]:
                    self.assertTrue(capture)
                    return subprocess.CompletedProcess(
                        command,
                        0,
                        stdout=f"{container}\n",
                        stderr="",
                    )
                if command[:3] == ["xcrun", "simctl", "launch"]:
                    self.assertFalse(result_path.exists())
                    result_path.write_text(
                        json.dumps(
                            {
                                "schema": "mdstream.flutter-runtime-smoke/1",
                                "ok": True,
                                "abi_version": 1,
                                "package_version": "0.4.0",
                                "binding_schema": "mdstream.bindings/0.4",
                                "is_finalized": True,
                                "has_root_node": True,
                                "native_allocations_zero": True,
                            }
                        ),
                        encoding="utf-8",
                    )
                return subprocess.CompletedProcess(
                    command,
                    0,
                    stdout="",
                    stderr="",
                )

            with patch.object(
                package_smoke.tempfile,
                "mkdtemp",
                return_value=str(project),
            ), patch.object(
                package_smoke,
                "configure_apple_host_target",
            ), patch.object(
                package_smoke,
                "_restore_macos_frameworks",
            ), patch.object(
                package_smoke,
                "_run",
                side_effect=fake_run,
            ):
                package_smoke.run_runtime_smoke(
                    platform_name="ios",
                    device="simulator-id",
                    plugin_source=TOOL_ROOT.parent,
                    keep_temporary=True,
                )

            self.assertIn(
                ["flutter", "build", "ios", "--simulator", "--debug"],
                commands,
            )
            self.assertTrue(
                any(
                    command[:3] == ["xcrun", "simctl", "install"]
                    for command in commands
                )
            )
            self.assertTrue(
                any(
                    command[:3] == ["xcrun", "simctl", "launch"]
                    for command in commands
                )
            )
            self.assertTrue(
                any(
                    command[:3] == ["xcrun", "simctl", "terminate"]
                    for command in commands
                )
            )
            self.assertFalse(
                any(command[:2] == ["flutter", "test"] for command in commands)
            )
            install = next(
                index
                for index, command in enumerate(commands)
                if command[:3] == ["xcrun", "simctl", "install"]
            )
            get_container = next(
                index
                for index, command in enumerate(commands)
                if command[:3] == ["xcrun", "simctl", "get_app_container"]
            )
            launch = next(
                index
                for index, command in enumerate(commands)
                if command[:3] == ["xcrun", "simctl", "launch"]
            )
            terminate = next(
                index
                for index, command in enumerate(commands)
                if command[:3] == ["xcrun", "simctl", "terminate"]
            )
            self.assertLess(install, get_container)
            self.assertLess(get_container, launch)
            self.assertLess(launch, terminate)
            self.assertEqual(
                (project / "lib" / "main.dart").read_bytes(),
                package_smoke.IOS_RUNTIME_SMOKE_SOURCE.read_bytes(),
            )
            self.assertEqual(
                (
                    project
                    / "lib"
                    / package_smoke.RUNTIME_SMOKE_PROBE_SOURCE.name
                ).read_bytes(),
                package_smoke.RUNTIME_SMOKE_PROBE_SOURCE.read_bytes(),
            )

    def test_non_ios_runtime_smoke_copies_the_shared_probe(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            project = Path(temporary) / "project"
            project.mkdir()
            commands: list[list[str]] = []

            def fake_run(
                command: list[str],
                *,
                cwd: Path,
                env: dict[str, str] | None = None,
                capture: bool = False,
            ) -> subprocess.CompletedProcess[str]:
                del cwd, env, capture
                commands.append(command)
                return subprocess.CompletedProcess(
                    command,
                    0,
                    stdout="",
                    stderr="",
                )

            with patch.object(
                package_smoke.tempfile,
                "mkdtemp",
                return_value=str(project),
            ), patch.object(
                package_smoke,
                "_restore_macos_frameworks",
            ), patch.object(
                package_smoke,
                "_run",
                side_effect=fake_run,
            ):
                package_smoke.run_runtime_smoke(
                    platform_name="linux",
                    device="linux",
                    plugin_source=TOOL_ROOT.parent,
                    keep_temporary=True,
                )

            self.assertEqual(
                (
                    project
                    / "integration_test"
                    / package_smoke.INTEGRATION_TEST.name
                ).read_bytes(),
                package_smoke.INTEGRATION_TEST.read_bytes(),
            )
            self.assertEqual(
                (
                    project
                    / "tool"
                    / package_smoke.RUNTIME_SMOKE_PROBE_SOURCE.name
                ).read_bytes(),
                package_smoke.RUNTIME_SMOKE_PROBE_SOURCE.read_bytes(),
            )
            self.assertTrue(
                any(command[:2] == ["flutter", "test"] for command in commands)
            )

    def test_ios_runtime_smoke_terminates_after_probe_failure(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            project = Path(temporary)
            (project / "lib").mkdir()
            app = project / "build" / "ios" / "iphonesimulator" / "Runner.app"
            app.mkdir(parents=True)
            (app / "Info.plist").write_bytes(
                plistlib.dumps({"CFBundleIdentifier": "io.mdstream.smoke.failure"})
            )
            container = project / "container"
            (container / "tmp").mkdir(parents=True)
            commands: list[list[str]] = []

            def fake_run(
                command: list[str],
                *,
                cwd: Path,
                env: dict[str, str] | None = None,
                capture: bool = False,
                timeout: float | None = None,
            ) -> subprocess.CompletedProcess[str]:
                del cwd, env
                commands.append(command)
                if command[0] == "xcrun":
                    self.assertIsNotNone(timeout)
                if command[:3] == ["xcrun", "simctl", "get_app_container"]:
                    self.assertTrue(capture)
                    return subprocess.CompletedProcess(
                        command,
                        0,
                        stdout=f"{container}\n",
                        stderr="",
                    )
                if command[:3] == ["xcrun", "simctl", "launch"]:
                    result = (
                        container
                        / "tmp"
                        / package_smoke.IOS_RUNTIME_SMOKE_RESULT
                    )
                    result.write_text(
                        json.dumps(
                            {
                                "schema": package_smoke.IOS_RUNTIME_SMOKE_SCHEMA,
                                "ok": False,
                                "error": "probe failed",
                                "stack_trace": "runtime_smoke_probe.dart:42",
                            }
                        ),
                        encoding="utf-8",
                    )
                if command[:3] == ["xcrun", "simctl", "terminate"]:
                    raise PackageSmokeError("simulator cleanup failed")
                return subprocess.CompletedProcess(
                    command,
                    0,
                    stdout="",
                    stderr="",
                )

            with patch.object(
                package_smoke,
                "_run",
                side_effect=fake_run,
            ), patch("builtins.print"):
                with self.assertRaisesRegex(
                    PackageSmokeError,
                    "probe failed\\nruntime_smoke_probe.dart:42",
                ):
                    package_smoke._run_ios_runtime_smoke(
                        project_root=project,
                        device="simulator-id",
                        env={},
                    )

            self.assertTrue(
                any(
                    command[:3] == ["xcrun", "simctl", "terminate"]
                    for command in commands
                )
            )

    def test_ios_runtime_smoke_collects_bounded_launch_diagnostics(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            stdout = root / "stdout"
            stderr = root / "stderr"
            stdout.write_text("application output", encoding="utf-8")
            stderr.write_text("dyld failure", encoding="utf-8")
            commands: list[list[str]] = []

            def fake_run(
                command: list[str],
                *,
                cwd: Path,
                env: dict[str, str] | None = None,
                capture: bool = False,
                timeout: float | None = None,
            ) -> subprocess.CompletedProcess[str]:
                del cwd, env
                self.assertTrue(capture)
                self.assertEqual(
                    timeout,
                    package_smoke.IOS_RUNTIME_SMOKE_DIAGNOSTIC_TIMEOUT_SECONDS,
                )
                commands.append(command)
                output = (
                    "service = io.mdstream.smoke.failure\n"
                    if "launchctl" in command
                    else "Runner crashed before publishing its result\n"
                )
                return subprocess.CompletedProcess(
                    command,
                    0,
                    stdout=output,
                    stderr="",
                )

            with patch.object(package_smoke, "_run", side_effect=fake_run):
                diagnostics = package_smoke._collect_ios_runtime_diagnostics(
                    project_root=root,
                    device="simulator-id",
                    bundle_identifier="io.mdstream.smoke.failure",
                    stdout_path=stdout,
                    stderr_path=stderr,
                )

            self.assertIn("Runner stdout:\napplication output", diagnostics)
            self.assertIn("Runner stderr:\ndyld failure", diagnostics)
            self.assertIn("service = io.mdstream.smoke.failure", diagnostics)
            self.assertIn("Runner crashed before publishing", diagnostics)
            self.assertLessEqual(
                len(diagnostics),
                package_smoke.IOS_RUNTIME_SMOKE_DIAGNOSTIC_CHARS
                + len("[truncated]\n"),
            )
            self.assertEqual(len(commands), 2)

    def test_command_timeout_becomes_a_package_smoke_error(self) -> None:
        with patch.object(
            package_smoke.subprocess,
            "run",
            side_effect=subprocess.TimeoutExpired(["xcrun", "simctl"], 10),
        ), patch("builtins.print"):
            with self.assertRaisesRegex(
                PackageSmokeError,
                "command timed out after 10 seconds: xcrun simctl",
            ):
                package_smoke._run(
                    ["xcrun", "simctl"],
                    cwd=TOOL_ROOT,
                    timeout=10,
                )

    def test_ios_runtime_smoke_rejects_an_application_failure(self) -> None:
        with self.assertRaisesRegex(
            PackageSmokeError,
            "iOS runtime smoke failed: native library could not be opened",
        ):
            package_smoke._validate_ios_runtime_smoke_payload(
                {
                    "schema": package_smoke.IOS_RUNTIME_SMOKE_SCHEMA,
                    "ok": False,
                    "error": "native library could not be opened",
                    "stack_trace": "runtime_smoke_probe.dart:42",
                }
            )

    def test_ios_runtime_smoke_rejects_incomplete_success_payload(self) -> None:
        with self.assertRaisesRegex(
            PackageSmokeError,
            "native_allocations_zero=None",
        ):
            package_smoke._validate_ios_runtime_smoke_payload(
                {
                    "schema": package_smoke.IOS_RUNTIME_SMOKE_SCHEMA,
                    "ok": True,
                    "abi_version": 1,
                    "package_version": "0.4.0",
                    "binding_schema": "mdstream.bindings/0.4",
                    "is_finalized": True,
                    "has_root_node": True,
                }
            )

    def test_ios_runtime_smoke_rejects_json_type_coercion(self) -> None:
        payload = {
            "schema": package_smoke.IOS_RUNTIME_SMOKE_SCHEMA,
            "ok": True,
            **package_smoke.IOS_RUNTIME_SMOKE_EXPECTED,
        }
        payload["abi_version"] = True

        with self.assertRaisesRegex(
            PackageSmokeError,
            r"abi_version=True \(expected 1\)",
        ):
            package_smoke._validate_ios_runtime_smoke_payload(payload)

    def test_ios_runtime_smoke_timeout_includes_launch_diagnostics(self) -> None:
        with tempfile.TemporaryDirectory() as temporary, patch.object(
            package_smoke.time,
            "monotonic",
            side_effect=(0.0, 61.0),
        ):
            result = Path(temporary) / "missing.json"
            with self.assertRaisesRegex(
                PackageSmokeError,
                "Runner stderr:\\ndyld: Library not loaded",
            ):
                package_smoke._wait_for_ios_runtime_smoke_result(
                    result,
                    diagnostics=lambda: (
                        "Runner stderr:\ndyld: Library not loaded"
                    ),
                )

    def test_runtime_smoke_entries_share_one_dart_probe_contract(self) -> None:
        probe_path = TOOL_ROOT / "runtime_smoke_probe.dart"
        probe = probe_path.read_text(encoding="utf-8")
        ios = (TOOL_ROOT / "ios_runtime_smoke.dart").read_text(encoding="utf-8")
        integration = (
            TOOL_ROOT.parent / "integration_test" / "native_load_test.dart"
        ).read_text(encoding="utf-8")

        self.assertIn(
            f"const runtimeSmokeResultName = '{package_smoke.IOS_RUNTIME_SMOKE_RESULT}';",
            probe,
        )
        self.assertIn(
            f"const runtimeSmokeSchema = '{package_smoke.IOS_RUNTIME_SMOKE_SCHEMA}';",
            probe,
        )
        self.assertIn(
            f"const runtimeSmokePackageVersion = '{package_version()}';",
            probe,
        )
        for adapter in (ios, integration):
            self.assertIn("runtime_smoke_probe.dart", adapter)
            self.assertIn("runBundledRuntimeSmoke()", adapter)
            self.assertNotIn("controller.append", adapter)

        with tempfile.TemporaryDirectory() as temporary:
            main = Path(temporary) / "lib" / "main.dart"
            main.parent.mkdir()
            android_smoke._write_smoke_main(main)
            self.assertIn("runBundledRuntimeSmoke()", main.read_text(encoding="utf-8"))
            self.assertEqual(
                (main.parent / probe_path.name).read_bytes(),
                probe_path.read_bytes(),
            )

    def test_swiftpm_manifest_contract_covers_binary_target_and_wrapper(self) -> None:
        manifest = {
            "name": "mdstream_flutter",
            "platforms": [{"platformName": "macos", "version": "11.0"}],
            "products": [
                {"name": "mdstream-flutter", "targets": ["mdstream_flutter"]}
            ],
            "targets": [
                {
                    "name": "MdstreamFFI",
                    "type": "binary",
                    "path": "../MdstreamFFI.xcframework",
                },
                {
                    "name": "mdstream_flutter",
                    "dependencies": [{"byName": ["MdstreamFFI", None]}],
                },
            ],
        }
        _validate_swiftpm_manifest("macos", manifest)

        broken = dict(manifest)
        broken["targets"] = [
            {
                "name": "MdstreamFFI",
                "type": "binary",
                "path": "../wrong.xcframework",
            },
            manifest["targets"][1],
        ]
        with self.assertRaisesRegex(PackageSmokeError, "unexpected path"):
            _validate_swiftpm_manifest("macos", broken)

    def test_swiftpm_manifest_root_can_be_loaded_from_an_extracted_archive(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            plugin = Path(temporary) / "plugin"
            root = plugin / "macos" / "mdstream_flutter"
            root.mkdir(parents=True)
            (root / "Package.swift").write_text("// fixture", encoding="utf-8")
            (plugin / "macos" / "MdstreamFFI.xcframework").mkdir()

            self.assertEqual(_swiftpm_manifest_root("macos", plugin), root)

    def test_swiftpm_smoke_rejects_non_macos_hosts(self) -> None:
        with patch.object(package_smoke.sys, "platform", "linux"):
            with self.assertRaisesRegex(PackageSmokeError, "requires a macOS runner"):
                package_smoke.run_swiftpm_smoke(
                    platform_name="ios",
                    device="simulator",
                    keep_temporary=False,
                )

    def test_apple_workflow_calls_both_swiftpm_variants_and_keeps_pods(self) -> None:
        workflow = (
            TOOL_ROOT.parents[2]
            / ".github"
            / "workflows"
            / "flutter-platforms.yml"
        ).read_text(encoding="utf-8")
        apple = workflow[
            workflow.index("  apple:\n") : workflow.index("  windows:\n")
        ]
        macos_pods = (
            "package_smoke.py --platform macos --device macos "
            "--skip-native-build --skip-archive"
        )
        macos_swiftpm = (
            "package_smoke.py --swiftpm --platform macos --skip-native-build"
        )
        ios_pods = (
            "package_smoke.py --platform ios --device \"$DEVICE_ID\" "
            "--skip-native-build --skip-archive"
        )
        ios_swiftpm = (
            "package_smoke.py --swiftpm --platform ios --device \"$DEVICE_ID\" "
            "--skip-native-build"
        )

        self.assertEqual(apple.count(macos_swiftpm), 1)
        self.assertEqual(apple.count(ios_swiftpm), 1)
        self.assertIn(macos_pods, apple)
        self.assertIn(ios_pods, apple)
        self.assertLess(apple.index(macos_pods), apple.index(macos_swiftpm))
        self.assertLess(apple.index(ios_pods), apple.index(ios_swiftpm))

        exact_ios_job = workflow[
            workflow.index("  package-ios-smoke:\n") : workflow.index(
                "  package-apple-swiftpm-smoke:\n"
            )
        ]
        self.assertIn("needs: package", exact_ios_job)
        self.assertIn("name: mdstream-flutter-package", exact_ios_job)
        self.assertIn('xcrun simctl bootstatus "$DEVICE_ID" -b', exact_ios_job)
        exact_ios_pods = (
            'package_smoke.py --archive "$FLUTTER_ARCHIVE" '
            '--platform ios --device "$DEVICE_ID" --skip-native-build'
        )
        exact_ios_swiftpm = (
            'package_smoke.py --swiftpm --archive "$FLUTTER_ARCHIVE" '
            '--platform ios --device "$DEVICE_ID" --skip-native-build'
        )
        self.assertIn(exact_ios_pods, exact_ios_job)
        self.assertIn(exact_ios_swiftpm, exact_ios_job)
        self.assertLess(
            exact_ios_job.index(exact_ios_pods),
            exact_ios_job.index(exact_ios_swiftpm),
        )

        exact_macos_job = workflow[
            workflow.index("  package-apple-swiftpm-smoke:\n") :
        ]
        self.assertIn("needs: package", exact_macos_job)
        self.assertIn("name: mdstream-flutter-package", exact_macos_job)
        self.assertIn(
            'package_smoke.py --swiftpm --archive "$FLUTTER_ARCHIVE" '
            "--platform macos --skip-native-build",
            exact_macos_job,
        )

    def test_apple_smoke_host_matches_plugin_deployment_targets(self) -> None:
        cases = {
            "ios": (
                "IPHONEOS_DEPLOYMENT_TARGET",
                "# platform :ios, '12.0'\n",
                "platform :ios, '14.0'",
                "14.0",
            ),
            "macos": (
                "MACOSX_DEPLOYMENT_TARGET",
                "platform :osx, '10.14'\n",
                "platform :osx, '11.0'",
                "11.0",
            ),
        }
        for platform_name, values in cases.items():
            setting, podfile, expected_pod, expected_target = values
            with self.subTest(platform=platform_name):
                with tempfile.TemporaryDirectory() as temporary:
                    root = Path(temporary)
                    project = (
                        root
                        / platform_name
                        / "Runner.xcodeproj"
                        / "project.pbxproj"
                    )
                    project.parent.mkdir(parents=True)
                    project.write_text(
                        f"{setting} = 10.0;\n{setting} = 12.0;\n",
                        encoding="utf-8",
                    )
                    podfile_path = root / platform_name / "Podfile"
                    podfile_path.write_text(podfile, encoding="utf-8")

                    package_smoke.configure_apple_host_target(root, platform_name)

                    self.assertEqual(
                        project.read_text(encoding="utf-8").count(
                            f"{setting} = {expected_target};"
                        ),
                        2,
                    )
                    self.assertIn(
                        expected_pod,
                        podfile_path.read_text(encoding="utf-8"),
                    )
                    podspec = (
                        TOOL_ROOT.parent
                        / platform_name
                        / "mdstream_flutter.podspec"
                    ).read_text(encoding="utf-8")
                    self.assertIn(f"'{expected_target}'", podspec)

    def test_dependency_graph_rejects_forbidden_packages(self) -> None:
        graph = {
            "packages": [
                {"name": "mdstream_flutter"},
                {"name": "flutter"},
                {"name": "merman"},
            ]
        }
        with self.assertRaisesRegex(PackageSmokeError, "merman"):
            validate_dependency_graph(graph, {"merman", "react"})

    def test_future_manifest_version_drives_archive_path(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            pubspec = root / "pubspec.yaml"
            pubspec.write_text(
                "name: mdstream_flutter\nversion: 1.2.3\n",
                encoding="utf-8",
            )

            self.assertEqual(package_version(pubspec), "1.2.3")
            self.assertEqual(
                package_archive_path(root, pubspec),
                root / "target" / "flutter-package" / "mdstream_flutter-1.2.3.tar.gz",
            )

    def test_archive_reports_single_slice_increment_and_rejects_forbidden_text(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            archive = Path(temporary) / "mdstream_flutter.tar.gz"
            native = _elf_image("x86_64")
            entries = {
                "pubspec.yaml": b"name: mdstream_flutter\n",
                "android/build.gradle": b"android {}\n",
                "android/src/main/jniLibs/x86_64/libmdstream_ffi.so": native,
            }
            _write_archive(archive, entries)

            report = inspect_package_archive(
                archive,
                forbidden_terms={"merman", "react"},
                native_ceiling_bytes=len(native),
                increment_ceiling_bytes=len(native) + 11,
                require_all_platforms=False,
            )
            self.assertEqual(report.max_native_bytes, len(native))
            self.assertEqual(
                report.max_platform_increment_bytes,
                len(native) + len(entries["android/build.gradle"]),
            )

            entries["lib/leak.dart"] = b"import 'package:react/react.dart';\n"
            _write_archive(archive, entries)
            with self.assertRaisesRegex(PackageSmokeError, "react"):
                inspect_package_archive(
                    archive,
                    forbidden_terms={"merman", "react"},
                    native_ceiling_bytes=len(native),
                    increment_ceiling_bytes=len(native) + 11,
                    require_all_platforms=False,
                )

    def test_archive_rejects_native_files_outside_inventory(self) -> None:
        cases = {
            "native-like extension": ("lib/hidden.so", b"not an ELF"),
            "native-like container": (
                "lib/Hidden.xcframework/Info.plist",
                b"not a framework",
            ),
            "native binary magic": ("lib/hidden.bin", b"\x7fELFpayload"),
            "fat Mach-O magic": ("lib/hidden-fat.bin", b"\xca\xfe\xba\xbfpayload"),
        }
        for label, (name, data) in cases.items():
            with self.subTest(case=label), tempfile.TemporaryDirectory() as temporary:
                archive = Path(temporary) / "mdstream_flutter.tar.gz"
                native = _elf_image("x86_64")
                _write_archive(
                    archive,
                    {
                        "pubspec.yaml": b"name: mdstream_flutter\n",
                        "android/src/main/jniLibs/x86_64/libmdstream_ffi.so": native,
                        name: data,
                    },
                )

                with self.assertRaisesRegex(
                    PackageSmokeError,
                    "outside canonical native inventory",
                ):
                    inspect_package_archive(
                        archive,
                        forbidden_terms=set(),
                        native_ceiling_bytes=len(native),
                        increment_ceiling_bytes=len(native),
                        require_all_platforms=False,
                    )

    def test_archive_budget_sums_every_native_slice_for_a_platform(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            archive = Path(temporary) / "mdstream_flutter.tar.gz"
            entries = {
                "pubspec.yaml": b"name: mdstream_flutter\n",
                "android/src/main/jniLibs/arm64-v8a/libmdstream_ffi.so": (
                    _elf_image("arm64")
                ),
                "android/src/main/jniLibs/armeabi-v7a/libmdstream_ffi.so": (
                    _elf_image("armv7")
                ),
                "android/src/main/jniLibs/x86_64/libmdstream_ffi.so": (
                    _elf_image("x86_64")
                ),
            }
            _write_archive(archive, entries)
            total = sum(len(data) for name, data in entries.items() if name.endswith(".so"))

            with self.assertRaisesRegex(PackageSmokeError, "package increment"):
                inspect_package_archive(
                    archive,
                    forbidden_terms=set(),
                    native_ceiling_bytes=max(map(len, entries.values())),
                    increment_ceiling_bytes=total - 1,
                    require_all_platforms=False,
                )

    def test_archive_cross_checks_apple_metadata_and_binary_platforms(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            archive = Path(temporary) / "mdstream_flutter.tar.gz"
            entries = {
                "pubspec.yaml": b"name: mdstream_flutter\n",
                **_apple_framework_entries("ios"),
            }
            _write_archive(archive, entries)
            ceiling = max(map(len, entries.values()))
            inspect_package_archive(
                archive,
                forbidden_terms=set(),
                native_ceiling_bytes=ceiling,
                increment_ceiling_bytes=sum(map(len, entries.values())),
                require_all_platforms=False,
            )

            device = next(name for name in entries if name.endswith("ios-arm64/MdstreamFFI.framework/MdstreamFFI"))
            entries[device] = _macho_image("arm64", platform=1)
            _write_archive(archive, entries)
            with self.assertRaisesRegex(PackageSmokeError, "platform"):
                inspect_package_archive(
                    archive,
                    forbidden_terms=set(),
                    native_ceiling_bytes=ceiling,
                    increment_ceiling_bytes=sum(map(len, entries.values())),
                    require_all_platforms=False,
                )

    def test_archive_requires_every_apple_framework_interface_file(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            archive = Path(temporary) / "mdstream_flutter.tar.gz"
            entries = {
                "pubspec.yaml": b"name: mdstream_flutter\n",
                "ios/MdstreamFFI.xcframework/Info.plist": _xcframework_plist("ios"),
                (
                    "ios/MdstreamFFI.xcframework/ios-arm64/"
                    "MdstreamFFI.framework/MdstreamFFI"
                ): _macho_image("arm64", platform=2),
                (
                    "ios/MdstreamFFI.xcframework/ios-arm64_x86_64-simulator/"
                    "MdstreamFFI.framework/MdstreamFFI"
                ): _fat_macho_image(("arm64", "x86_64"), platform=7),
            }
            _write_archive(archive, entries)

            with self.assertRaisesRegex(
                PackageSmokeError,
                "Headers/mdstream.h|module.modulemap|framework Info.plist",
            ):
                inspect_package_archive(
                    archive,
                    forbidden_terms=set(),
                    native_ceiling_bytes=max(map(len, entries.values())),
                    increment_ceiling_bytes=sum(map(len, entries.values())),
                    require_all_platforms=False,
                )

    def test_archive_rejects_inconsistent_apple_framework_info(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            archive = Path(temporary) / "mdstream_flutter.tar.gz"
            entries = {
                "pubspec.yaml": b"name: mdstream_flutter\n",
                **_apple_framework_entries("ios"),
            }
            info_name = next(
                name
                for name in entries
                if "ios-arm64/MdstreamFFI.framework/Info.plist" in name
            )
            info = plistlib.loads(entries[info_name])
            info["MinimumOSVersion"] = "13.0"
            entries[info_name] = plistlib.dumps(info)
            _write_archive(archive, entries)

            with self.assertRaisesRegex(
                PackageSmokeError,
                "MinimumOSVersion",
            ):
                inspect_package_archive(
                    archive,
                    forbidden_terms=set(),
                    native_ceiling_bytes=max(map(len, entries.values())),
                    increment_ceiling_bytes=sum(map(len, entries.values())),
                    require_all_platforms=False,
                )

    def test_archive_rejects_noncanonical_and_unsupported_members(self) -> None:
        native = _elf_image("x86_64")
        base = [
            _tar_file("pubspec.yaml", b"name: mdstream_flutter\n"),
            _tar_file(
                "android/src/main/jniLibs/x86_64/libmdstream_ffi.so",
                native,
            ),
        ]
        fifo = tarfile.TarInfo("lib/pipe")
        fifo.type = tarfile.FIFOTYPE
        cases = {
            "backslash": [*base, _tar_file(r"lib\escape.dart", b"content")],
            "dot alias": [*base, _tar_file("lib/./escape.dart", b"content")],
            "normalized duplicate": [
                *base,
                _tar_file("lib/escape.dart", b"first"),
                _tar_file("lib/./escape.dart", b"second"),
            ],
            "special member": [*base, (fifo, b"")],
        }

        for name, members in cases.items():
            with self.subTest(member=name), tempfile.TemporaryDirectory() as temporary:
                archive = Path(temporary) / "mdstream_flutter.tar.gz"
                _write_archive_members(archive, members)
                with self.assertRaisesRegex(
                    PackageSmokeError,
                    "unsafe|non-canonical|duplicate|unsupported",
                ):
                    inspect_package_archive(
                        archive,
                        forbidden_terms=set(),
                        native_ceiling_bytes=len(native),
                        increment_ceiling_bytes=len(native),
                        require_all_platforms=False,
                    )

    def test_archive_rejects_extra_gzip_members_and_tar_eoa_data(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            archive = Path(temporary) / "mdstream_flutter.tar.gz"
            native = _elf_image("x86_64")
            entries = {
                "pubspec.yaml": b"name: mdstream_flutter\n",
                "android/src/main/jniLibs/x86_64/libmdstream_ffi.so": native,
            }
            _write_archive(archive, entries)
            raw_tar = gzip.decompress(archive.read_bytes())
            cases = {
                "multiple gzip members": (
                    gzip.compress(raw_tar) + gzip.compress(b"second member")
                ),
                "non-zero data after tar end-of-archive": gzip.compress(
                    raw_tar + b"trailing"
                ),
            }

            for message, data in cases.items():
                with self.subTest(case=message):
                    archive.write_bytes(data)
                    with self.assertRaisesRegex(PackageSmokeError, message):
                        inspect_package_archive(
                            archive,
                            forbidden_terms=set(),
                            native_ceiling_bytes=len(native),
                            increment_ceiling_bytes=len(native),
                            require_all_platforms=False,
                        )

    def test_archive_rejects_resource_limits_before_reading_payload(self) -> None:
        native = _elf_image("x86_64")
        entries = {
            "pubspec.yaml": b"name: mdstream_flutter\n",
            "android/src/main/jniLibs/x86_64/libmdstream_ffi.so": native,
            "lib/extra.dart": b"content",
        }
        default_limits = {
            "max_compressed_bytes": 64_000,
            "max_members": 32,
            "max_member_bytes": 1_024,
            "max_uncompressed_bytes": 4_096,
        }

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "mdstream_flutter.tar.gz"
            _write_archive(archive, entries)
            cases = {
                "compressed size": {**default_limits, "max_compressed_bytes": 1},
                "member count": {**default_limits, "max_members": 2},
                "total uncompressed": {
                    **default_limits,
                    "max_uncompressed_bytes": 100,
                },
            }
            for label, limits in cases.items():
                with self.subTest(limit=label), self.assertRaisesRegex(
                    PackageSmokeError,
                    "compressed|member count|uncompressed",
                ):
                    inspect_package_archive(
                        archive,
                        forbidden_terms=set(),
                        native_ceiling_bytes=len(native),
                        increment_ceiling_bytes=len(native),
                        require_all_platforms=False,
                        archive_limits=limits,
                    )

            declared = root / "declared-too-large.tar.gz"
            _write_declared_size_archive(declared, "lib/huge.bin", 1_000_000)
            with self.assertRaisesRegex(PackageSmokeError, "member.*ceiling"):
                inspect_package_archive(
                    declared,
                    forbidden_terms=set(),
                    native_ceiling_bytes=len(native),
                    increment_ceiling_bytes=len(native),
                    require_all_platforms=False,
                    archive_limits=default_limits,
                )

            pax = root / "oversized-pax.tar.gz"
            _write_oversized_pax_archive(pax, declared_size=1_000_000)
            with self.assertRaisesRegex(PackageSmokeError, "decompressed stream"):
                inspect_package_archive(
                    pax,
                    forbidden_terms=set(),
                    native_ceiling_bytes=len(native),
                    increment_ceiling_bytes=len(native),
                    require_all_platforms=False,
                    archive_limits={
                        **default_limits,
                        "max_members": 1,
                        "max_uncompressed_bytes": 4_096,
                    },
                )

    def test_archive_extraction_stays_beneath_destination(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "package.tar.gz"
            _write_archive(archive, {"linked/escape.txt": b"escaped"})
            destination = root / "destination"
            destination.mkdir()
            outside = root / "outside"
            outside.mkdir()
            (destination / "linked").symlink_to(outside, target_is_directory=True)

            with self.assertRaisesRegex(
                PackageSmokeError,
                "extraction destination|extraction path",
            ):
                _extract_archive(archive, destination)

            self.assertFalse((outside / "escape.txt").exists())


def _header_only_elf_image(
    architecture: str, *, alignment: int = 16 * 1024
) -> bytes:
    machine = {"armv7": 40, "x86_64": 62, "arm64": 183}[architecture]
    elf_class = 1 if architecture == "armv7" else 2
    ident = b"\x7fELF" + bytes((elf_class, 1, 1, 0)) + (b"\0" * 8)
    if elf_class == 1:
        header = struct.pack(
            "<HHIIIIIHHHHHH",
            3,
            machine,
            1,
            0,
            52,
            0,
            0,
            52,
            32,
            1,
            0,
            0,
            0,
        )
        program = struct.pack("<IIIIIIII", 1, 0, 0, 0, 0, 0, 5, alignment)
    else:
        header = struct.pack(
            "<HHIQQQIHHHHHH",
            3,
            machine,
            1,
            0,
            64,
            0,
            0,
            64,
            56,
            1,
            0,
            0,
            0,
        )
        program = struct.pack("<IIQQQQQQ", 1, 5, 0, 0, 0, 0, 0, alignment)
    return ident + header + program + _REQUIRED_SYMBOL_TEXT


def _macho_image(
    architecture: str,
    *,
    platform: int,
    minimum: tuple[int, int, int] | None = None,
) -> bytes:
    cpu_type = {"x86_64": 0x01000007, "arm64": 0x0100000C}[architecture]
    if minimum is None:
        minimum = (11, 0, 0) if platform == 1 else (14, 0, 0)
    packed_minimum = (minimum[0] << 16) | (minimum[1] << 8) | minimum[2]
    build_version = struct.pack(
        "<IIIIII", 0x32, 24, platform, packed_minimum, 0, 0
    )
    install_name = b"@rpath/MdstreamFFI.framework/MdstreamFFI\0"
    dylib_command_size = (24 + len(install_name) + 7) & ~7
    dylib_command = bytearray(b"\0" * dylib_command_size)
    struct.pack_into("<IIIIII", dylib_command, 0, 0xD, dylib_command_size, 24, 0, 0, 0)
    dylib_command[24 : 24 + len(install_name)] = install_name
    header_size = 32
    command_size = 24 + dylib_command_size + 24
    symbol_offset = header_size + command_size
    strings = b"\0" + b"\0".join(
        f"_{symbol}".encode("ascii") for symbol in _required_symbols()
    ) + b"\0"
    string_positions = {
        symbol: strings.index(f"_{symbol}".encode("ascii"))
        for symbol in _required_symbols()
    }
    symbol_table = b"".join(
        struct.pack("<IBBHQ", string_positions[symbol], 0x0F, 1, 0, 0)
        for symbol in _required_symbols()
    )
    string_offset = symbol_offset + len(symbol_table)
    symtab_command = struct.pack(
        "<IIIIII",
        0x2,
        24,
        symbol_offset,
        len(_required_symbols()),
        string_offset,
        len(strings),
    )
    header = b"\xcf\xfa\xed\xfe" + struct.pack(
        "<IIIIIII", cpu_type, 0, 6, 3, command_size, 0, 0
    )
    return (
        header
        + build_version
        + bytes(dylib_command)
        + symtab_command
        + symbol_table
        + strings
    )


def _fat_macho_image(architectures: tuple[str, ...], *, platform: int) -> bytes:
    slices = [(_macho_image(architecture, platform=platform), architecture) for architecture in architectures]
    header_size = 8 + len(slices) * 20
    offset = (header_size + 255) & ~255
    entries: list[bytes] = []
    payload = bytearray(b"\0" * offset)
    for data, architecture in slices:
        cpu_type = {"x86_64": 0x01000007, "arm64": 0x0100000C}[architecture]
        entries.append(struct.pack(">IIIII", cpu_type, 0, offset, len(data), 8))
        payload.extend(data)
        offset += len(data)
        padding = (-offset) % 256
        payload.extend(b"\0" * padding)
        offset += padding
    payload[:8] = struct.pack(">II", 0xCAFEBABE, len(slices))
    payload[8:header_size] = b"".join(entries)
    return bytes(payload)


def _header_only_pe_image(architecture: str) -> bytes:
    machine = {"x86_64": 0x8664, "arm64": 0xAA64}[architecture]
    data = bytearray(b"\0" * 0x80)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 0x3C, 0x40)
    data[0x40:0x44] = b"PE\0\0"
    struct.pack_into("<H", data, 0x44, machine)
    data.extend(_REQUIRED_SYMBOL_TEXT)
    return bytes(data)


def _pe_image(architecture: str, *, forwarded: bool = False) -> bytes:
    machine = {"x86_64": 0x8664, "arm64": 0xAA64}[architecture]
    pe_offset = 0x80
    optional_size = 0xF0
    headers_size = 0x200
    section_rva = 0x1000
    section_offset = 0x200
    section_size = 0x400
    image = bytearray(b"\0" * (section_offset + section_size))
    image[:2] = b"MZ"
    struct.pack_into("<I", image, 0x3C, pe_offset)
    image[pe_offset : pe_offset + 4] = b"PE\0\0"
    struct.pack_into(
        "<HHIIIHH", image, pe_offset + 4, machine, 1, 0, 0, 0, optional_size, 0x2022
    )
    optional = pe_offset + 24
    struct.pack_into("<H", image, optional, 0x20B)
    struct.pack_into("<II", image, optional + 32, 0x1000, 0x200)
    struct.pack_into("<II", image, optional + 56, 0x2000, headers_size)
    struct.pack_into("<I", image, optional + 108, 16)
    struct.pack_into("<II", image, optional + 112, section_rva, 0x180)
    section_header = optional + optional_size
    image[section_header : section_header + 8] = b".rdata\0\0"
    struct.pack_into(
        "<IIIIIIHHI",
        image,
        section_header + 8,
        section_size,
        section_rva,
        section_size,
        section_offset,
        0,
        0,
        0,
        0,
        0x40000040,
    )
    function_offset = section_offset + 0x40
    name_offset = section_offset + 0x60
    ordinal_offset = section_offset + 0x80
    strings_offset = section_offset + 0x100
    cursor = strings_offset
    dll_name = b"mdstream_ffi.dll\0"
    image[cursor : cursor + len(dll_name)] = dll_name
    dll_name_rva = section_rva + cursor - section_offset
    cursor += len(dll_name)
    name_rvas = []
    for symbol in _required_symbols():
        encoded = symbol.encode("ascii") + b"\0"
        name_rvas.append(section_rva + cursor - section_offset)
        image[cursor : cursor + len(encoded)] = encoded
        cursor += len(encoded)
    count = len(name_rvas)
    struct.pack_into(
        "<IIHHIIIIIII",
        image,
        section_offset,
        0,
        0,
        0,
        0,
        dll_name_rva,
        1,
        count,
        count,
        section_rva + 0x40,
        section_rva + 0x60,
        section_rva + 0x80,
    )
    for index, name_rva in enumerate(name_rvas):
        target_rva = section_rva + (0x100 if forwarded else 0x300 + index)
        struct.pack_into("<I", image, function_offset + index * 4, target_rva)
        struct.pack_into("<I", image, name_offset + index * 4, name_rva)
        struct.pack_into("<H", image, ordinal_offset + index * 2, index)
    return bytes(image)


def _xcframework_plist(platform: str) -> bytes:
    libraries = []
    for group, contract in NATIVE_CONTRACTS.items():
        if not group.startswith(f"{platform}/"):
            continue
        identifier = group.split("/", 1)[1]
        library = {
            "BinaryPath": "MdstreamFFI.framework/MdstreamFFI",
            "LibraryIdentifier": identifier,
            "LibraryPath": "MdstreamFFI.framework",
            "SupportedArchitectures": sorted(contract.architectures),
            "SupportedPlatform": contract.apple_platform,
        }
        if contract.apple_variant is not None:
            library["SupportedPlatformVariant"] = contract.apple_variant
        libraries.append(library)
    return plistlib.dumps(
        {
            "AvailableLibraries": libraries,
            "CFBundlePackageType": "XFWK",
            "XCFrameworkFormatVersion": "1.0",
        },
        sort_keys=True,
    )


def _framework_info_plist(group: str) -> bytes:
    contract = NATIVE_CONTRACTS[group]
    platform_name = {
        ("macos", None): "MacOSX",
        ("ios", None): "iPhoneOS",
        ("ios", "simulator"): "iPhoneSimulator",
    }[(contract.apple_platform, contract.apple_variant)]
    minimum = contract.apple_minimum_version
    assert minimum is not None
    return plistlib.dumps(
        {
            "CFBundleExecutable": "MdstreamFFI",
            "CFBundleIdentifier": "io.mdstream.flutter.MdstreamFFI",
            "CFBundleName": "MdstreamFFI",
            "CFBundlePackageType": "FMWK",
            "CFBundleSupportedPlatforms": [platform_name],
            "MinimumOSVersion": f"{minimum[0]}.{minimum[1]}",
        },
        sort_keys=True,
    )


def _apple_framework_entries(platform: str) -> dict[str, bytes]:
    entries = {
        f"{platform}/MdstreamFFI.xcframework/Info.plist": _xcframework_plist(
            platform
        )
    }
    canonical_header = (
        TOOL_ROOT.parents[2] / "mdstream-ffi" / "include" / "mdstream.h"
    ).read_bytes()
    for group, contract in NATIVE_CONTRACTS.items():
        if not group.startswith(f"{platform}/"):
            continue
        identifier = group.split("/", 1)[1]
        root = (
            f"{platform}/MdstreamFFI.xcframework/{identifier}/"
            "MdstreamFFI.framework/"
        )
        platform_number = {
            ("macos", None): 1,
            ("ios", None): 2,
            ("ios", "simulator"): 7,
        }[(contract.apple_platform, contract.apple_variant)]
        architectures = tuple(sorted(contract.architectures))
        entries[f"{root}MdstreamFFI"] = (
            _macho_image(architectures[0], platform=platform_number)
            if len(architectures) == 1
            else _fat_macho_image(architectures, platform=platform_number)
        )
        entries[f"{root}Headers/mdstream.h"] = canonical_header
        entries[f"{root}Modules/module.modulemap"] = FRAMEWORK_MODULE_MAP.encode(
            "utf-8"
        )
        entries[f"{root}Info.plist"] = _framework_info_plist(group)
    return entries


def _write_archive(path: Path, entries: dict[str, bytes]) -> None:
    _write_archive_members(
        path,
        [_tar_file(name, contents) for name, contents in entries.items()],
    )


def _tar_file(name: str, contents: bytes) -> tuple[tarfile.TarInfo, bytes]:
    info = tarfile.TarInfo(name)
    info.size = len(contents)
    info.mode = 0o644
    return info, contents


def _write_archive_members(
    path: Path,
    members: list[tuple[tarfile.TarInfo, bytes]],
) -> None:
    with tarfile.open(path, "w:gz") as archive:
        for info, contents in members:
            archive.addfile(info, io.BytesIO(contents) if info.isfile() else None)


def _write_declared_size_archive(path: Path, name: str, size: int) -> None:
    info = tarfile.TarInfo(name)
    info.size = size
    path.write_bytes(gzip.compress(info.tobuf() + (b"\0" * 1024)))


def _write_oversized_pax_archive(path: Path, declared_size: int) -> None:
    info = tarfile.TarInfo("././@PaxHeader")
    info.type = tarfile.XHDTYPE
    info.size = declared_size
    path.write_bytes(gzip.compress(info.tobuf() + (b"0" * 100_000)))


if __name__ == "__main__":
    unittest.main()
