#!/usr/bin/env python3

import io
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
    inspect_package_archive,
    validate_dependency_graph,
)


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
            self.assertIn(f'ndk;{selected.name}', workflow)

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


def _write_archive(path: Path, entries: dict[str, bytes]) -> None:
    with tarfile.open(path, "w:gz") as archive:
        for name, contents in entries.items():
            info = tarfile.TarInfo(name)
            info.size = len(contents)
            info.mode = 0o644
            archive.addfile(info, io.BytesIO(contents))


if __name__ == "__main__":
    unittest.main()
