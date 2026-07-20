#!/usr/bin/env python3

import io
import gzip
import json
import os
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


TOOL_ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(TOOL_ROOT))

import package_smoke  # noqa: E402

from build_native import (  # noqa: E402
    ANDROID_TARGETS,
    PackagingError,
    _android_ndk_home,
    atomic_stage,
    detect_native_format,
    validate_native_artifact,
)
from package_smoke import (  # noqa: E402
    PackageSmokeError,
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
                f'"ndk;{selected.name}"',
                workflow,
            )

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
        self.assertLess(
            linux.index(generate_linux_host),
            linux.index(integration_smoke),
        )

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

    def test_native_validation_checks_magic_and_absolute_ceiling(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            library = Path(temporary) / "libmdstream_ffi.so"
            library.write_bytes(b"\x7fELFpayload")
            self.assertEqual(detect_native_format(library), "elf")
            validate_native_artifact(
                library,
                expected_format="elf",
                ceiling_bytes=64,
                check_exports=False,
            )

            with self.assertRaisesRegex(PackagingError, "ceiling"):
                validate_native_artifact(
                    library,
                    expected_format="elf",
                    ceiling_bytes=4,
                    check_exports=False,
                )

            library.write_bytes(b"not-an-elf")
            with self.assertRaisesRegex(PackagingError, "format"):
                validate_native_artifact(
                    library,
                    expected_format="elf",
                    ceiling_bytes=64,
                    check_exports=False,
                )

    def test_symbol_text_without_an_export_table_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            library = Path(temporary) / "libmdstream_ffi.so"
            library.write_bytes(
                b"\x7fELFmdstream_abi_version mdstream_package_version "
                b"mdstream_engine_new mdstream_reducer_new"
            )
            with self.assertRaisesRegex(PackagingError, "command failed"):
                validate_native_artifact(
                    library,
                    expected_format="elf",
                    ceiling_bytes=256,
                    check_exports=True,
                )


class PackageSmokeContractTest(unittest.TestCase):
    def test_flutter_tool_uses_the_windows_batch_entrypoint(self) -> None:
        with patch.object(package_smoke.sys, "platform", "win32"):
            self.assertEqual(package_smoke._flutter_tool(), "flutter.bat")
        with patch.object(package_smoke.sys, "platform", "linux"):
            self.assertEqual(package_smoke._flutter_tool(), "flutter")

    def test_apple_smoke_host_matches_plugin_deployment_targets(self) -> None:
        cases = {
            "ios": (
                "IPHONEOS_DEPLOYMENT_TARGET",
                "# platform :ios, '12.0'\n",
                "platform :ios, '13.0'",
                "13.0",
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
            native = (
                b"\x7fELFmdstream_abi_version|mdstream_package_version|"
                b"mdstream_engine_new|mdstream_reducer_new"
            )
            entries = {
                "pubspec.yaml": b"name: mdstream_flutter\n",
                "android/build.gradle": b"android {}\n",
                "android/src/main/jniLibs/x86_64/libmdstream_ffi.so": native,
            }
            _write_archive(archive, entries)

            report = inspect_package_archive(
                archive,
                forbidden_terms={"merman", "react"},
                native_ceiling_bytes=128,
                increment_ceiling_bytes=128,
                require_all_platforms=False,
            )
            self.assertEqual(report.max_native_bytes, 90)
            self.assertEqual(report.max_platform_increment_bytes, 101)

            entries["lib/leak.dart"] = b"import 'package:react/react.dart';\n"
            _write_archive(archive, entries)
            with self.assertRaisesRegex(PackageSmokeError, "react"):
                inspect_package_archive(
                    archive,
                    forbidden_terms={"merman", "react"},
                    native_ceiling_bytes=128,
                    increment_ceiling_bytes=128,
                    require_all_platforms=False,
                )

    def test_archive_rejects_noncanonical_and_unsupported_members(self) -> None:
        native = (
            b"\x7fELFmdstream_abi_version|mdstream_package_version|"
            b"mdstream_engine_new|mdstream_reducer_new"
        )
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
                        native_ceiling_bytes=128,
                        increment_ceiling_bytes=128,
                        require_all_platforms=False,
                    )

    def test_archive_rejects_resource_limits_before_reading_payload(self) -> None:
        native = (
            b"\x7fELFmdstream_abi_version|mdstream_package_version|"
            b"mdstream_engine_new|mdstream_reducer_new"
        )
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
                        native_ceiling_bytes=128,
                        increment_ceiling_bytes=128,
                        require_all_platforms=False,
                        archive_limits=limits,
                    )

            declared = root / "declared-too-large.tar.gz"
            _write_declared_size_archive(declared, "lib/huge.bin", 1_000_000)
            with self.assertRaisesRegex(PackageSmokeError, "member.*ceiling"):
                inspect_package_archive(
                    declared,
                    forbidden_terms=set(),
                    native_ceiling_bytes=128,
                    increment_ceiling_bytes=128,
                    require_all_platforms=False,
                    archive_limits=default_limits,
                )

            pax = root / "oversized-pax.tar.gz"
            _write_oversized_pax_archive(pax, declared_size=1_000_000)
            with self.assertRaisesRegex(PackageSmokeError, "decompressed stream"):
                inspect_package_archive(
                    pax,
                    forbidden_terms=set(),
                    native_ceiling_bytes=128,
                    increment_ceiling_bytes=128,
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

            with self.assertRaisesRegex(PackageSmokeError, "extraction path"):
                _extract_archive(archive, destination)

            self.assertFalse((outside / "escape.txt").exists())


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
