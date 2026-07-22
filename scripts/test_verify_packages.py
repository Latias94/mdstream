#!/usr/bin/env python3
"""Contract tests for multi-ecosystem package verification."""

from __future__ import annotations

import importlib.util
import base64
import gzip
import io
import json
import re
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]
MODULE_PATH = Path(__file__).with_name("verify-packages.py")
REGISTRY_CHECKER_PATH = Path(__file__).with_name("check-registry-version.py")
WORKFLOW_ROOT = ROOT / ".github" / "workflows"
RELEASE_TOOL_BUNDLE_FILES = (
    "scripts/archive_policy.py",
    "scripts/check-registry-version.py",
    "scripts/release_notes.py",
    "scripts/verify-packages.py",
    "bindings/flutter/tool/native_artifact.py",
)
SPEC = importlib.util.spec_from_file_location("verify_packages", MODULE_PATH)
assert SPEC is not None
verify_packages = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = verify_packages
SPEC.loader.exec_module(verify_packages)
import archive_policy  # noqa: E402
import package_smoke  # noqa: E402
from native_test_fixture import elf_image  # noqa: E402
from native_artifact import is_native_like_artifact  # noqa: E402

REGISTRY_SPEC = importlib.util.spec_from_file_location(
    "check_registry_version", REGISTRY_CHECKER_PATH
)
assert REGISTRY_SPEC is not None
check_registry_version = importlib.util.module_from_spec(REGISTRY_SPEC)
assert REGISTRY_SPEC.loader is not None
sys.modules[REGISTRY_SPEC.name] = check_registry_version
REGISTRY_SPEC.loader.exec_module(check_registry_version)


class PackageContractTests(unittest.TestCase):
    def test_version_mismatch_is_rejected(self) -> None:
        with self.assertRaisesRegex(
            verify_packages.ValidationError,
            "mdstream-tokio.*0.3.0.*0.4.0",
        ):
            verify_packages.validate_versions(
                {
                    "mdstream-protocol": "0.4.0",
                    "mdstream-tokio": "0.3.0",
                }
            )

    def test_package_changelog_requires_current_nonempty_first_release(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            changelog = Path(temporary) / "CHANGELOG.md"
            changelog.write_text(
                "# Changelog\n\n## 0.4.0\n\n- Added a contract.\n",
                encoding="utf-8",
            )
            verify_packages.validate_package_changelog(changelog, "0.4.0")

            for text, message in (
                (
                    "# Changelog\n\n## 0.3.0\n\n- Old.\n\n"
                    "## 0.4.0\n\n- New.\n",
                    "starts at 0.3.0",
                ),
                ("# Changelog\n\n## 0.4.0\n", "empty changelog section"),
            ):
                with self.subTest(message=message):
                    changelog.write_text(text, encoding="utf-8")
                    with self.assertRaisesRegex(
                        verify_packages.ValidationError,
                        message,
                    ):
                        verify_packages.validate_package_changelog(
                            changelog,
                            "0.4.0",
                        )

    def test_flutter_native_metadata_matches_future_release_version(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            flutter = root / "bindings" / "flutter"
            (flutter / "android").mkdir(parents=True)
            (flutter / "ios").mkdir()
            (flutter / "macos").mkdir()
            (flutter / "android" / "build.gradle").write_text(
                'version = "1.2.3"\n',
                encoding="utf-8",
            )
            for platform in ("ios", "macos"):
                (flutter / platform / "mdstream_flutter.podspec").write_text(
                    "s.version = '1.2.3'\n",
                    encoding="utf-8",
                )

            verify_packages.validate_flutter_version_metadata(root, "1.2.3")
            (flutter / "macos" / "mdstream_flutter.podspec").write_text(
                "s.version = '0.4.0'\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(
                verify_packages.ValidationError,
                "macOS podspec.*0.4.0.*1.2.3",
            ):
                verify_packages.validate_flutter_version_metadata(root, "1.2.3")

    def test_dependency_must_precede_its_dependent(self) -> None:
        packages = {
            "protocol": package(),
            "engine": package(dependency("protocol")),
        }
        with self.assertRaisesRegex(
            verify_packages.ValidationError,
            "engine.*before.*protocol",
        ):
            verify_packages.validate_rust_topology(
                ("engine", "protocol"),
                packages,
            )

    def test_non_dev_internal_path_dependency_requires_a_version(self) -> None:
        packages = {
            "protocol": package(),
            "engine": package(dependency("protocol", requirement="*")),
        }
        with self.assertRaisesRegex(
            verify_packages.ValidationError,
            "path-only.*engine.*protocol",
        ):
            verify_packages.validate_internal_dependency_versions(
                packages,
                {"protocol", "engine"},
                "0.4.0",
            )

    def test_dev_only_path_dependency_is_allowed(self) -> None:
        packages = {
            "protocol": package(),
            "engine": package(
                dependency("protocol", kind="dev", requirement="*"),
            ),
        }
        verify_packages.validate_internal_dependency_versions(
            packages,
            {"protocol", "engine"},
            "0.4.0",
        )

    def test_inventory_rejects_missing_and_forbidden_files(self) -> None:
        with self.assertRaisesRegex(
            verify_packages.ValidationError,
            "missing.*lib/index.js",
        ):
            verify_packages.validate_inventory(
                "npm",
                {"package.json"},
                required={"package.json", "lib/index.js"},
            )

        with self.assertRaisesRegex(
            verify_packages.ValidationError,
            "forbidden.*repo-ref/private.md",
        ):
            verify_packages.validate_inventory(
                "crate",
                {"Cargo.toml", "repo-ref/private.md"},
                required={"Cargo.toml"},
            )

    def test_promised_examples_are_required_in_package_inventories(self) -> None:
        cases = (
            (
                "Dart mdstream",
                verify_packages.DART_REQUIRED_FILES,
                "example/golden_stream.dart",
            ),
            (
                "Flutter mdstream_flutter",
                verify_packages.FLUTTER_REQUIRED_FILES,
                "example/lib/bootstrap.dart",
            ),
            (
                "crate mdstream",
                verify_packages.RUST_REQUIRED_FILES["mdstream"],
                "examples/minimal.rs",
            ),
            (
                "crate mdstream-merman",
                verify_packages.RUST_REQUIRED_FILES["mdstream-merman"],
                "examples/render_golden.rs",
            ),
            (
                "crate mdstream-tokio",
                verify_packages.RUST_REQUIRED_FILES["mdstream-tokio"],
                "examples/agent_tui.rs",
            ),
        )
        for label, required, promised in cases:
            with self.subTest(package=label):
                actual = set(required)
                actual.remove(promised)
                with self.assertRaisesRegex(
                    verify_packages.ValidationError,
                    f"missing.*{re.escape(promised)}",
                ):
                    verify_packages.validate_inventory(
                        label,
                        actual,
                        required=set(required),
                    )

    def test_flutter_archive_rejects_repository_only_example_state(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            write_binding_policy(root, npm_ceiling=64_000, dart_ceiling=64_000)
            for forbidden in (
                "example/pubspec_overrides.yaml",
                "example/pubspec.lock",
                "example/.dart_tool/package_config.json",
                "example/build/output.bin",
                "example/test/golden_stream_test.dart",
                "example/integration_test/golden_stream_smoke_test.dart",
            ):
                with self.subTest(path=forbidden):
                    files = valid_flutter_files()
                    files[forbidden] = b"repository only"
                    archive = root / "flutter.tar.gz"
                    write_files_tar(archive, files)
                    with self.assertRaisesRegex(
                        verify_packages.ValidationError,
                        "forbidden path",
                    ):
                        verify_packages.verify_existing_archive(
                            root,
                            "flutter",
                            archive,
                        )

    def test_pub_archive_rejects_path_dependencies(self) -> None:
        self.assertTrue(
            verify_packages.pubspec_has_path_dependency(
                "dependencies:\n  mdstream:\n    path: ../dart\n"
            )
        )
        self.assertFalse(
            verify_packages.pubspec_has_path_dependency(
                "dependencies:\n  mdstream: ^0.4.0\n"
            )
        )

    def test_archive_reader_rejects_unsafe_members(self) -> None:
        cases = {
            "absolute": [tar_member("/escape", b"payload")],
            "parent": [tar_member("../escape", b"payload")],
            "symlink": [tar_link("link", "target", tarfile.SYMTYPE)],
            "hardlink": [tar_link("link", "target", tarfile.LNKTYPE)],
            "duplicate": [
                tar_member("pubspec.yaml", b"first"),
                tar_member("pubspec.yaml", b"second"),
            ],
            "case collision": [
                tar_member("lib/A.dart", b"first"),
                tar_member("lib/a.dart", b"second"),
            ],
            "Unicode collision": [
                tar_member("lib/\u00e9.dart", b"first"),
                tar_member("lib/e\u0301.dart", b"second"),
            ],
            "case-colliding parent directories": [
                tar_member("Lib/a.dart", b"first"),
                tar_member("lib/b.dart", b"second"),
            ],
            "Unicode-colliding parent directories": [
                tar_member("lib/caf\u00e9/a.dart", b"first"),
                tar_member("lib/cafe\u0301/b.dart", b"second"),
            ],
            "explicit directory aliases an implicit directory": [
                tar_member("Lib/a.dart", b"first"),
                tar_directory("lib"),
            ],
            "Windows reserved name": [tar_member("lib/CON.dart", b"payload")],
            "Windows superscript COM device": [
                tar_member("lib/COM\u00b9.txt", b"payload")
            ],
            "Windows superscript LPT device": [
                tar_member("lib/LPT\u00b3.txt", b"payload")
            ],
            "Windows trailing dot": [tar_member("lib/name.", b"payload")],
            "Windows invalid character": [tar_member("lib/name:stream", b"payload")],
            "file before subtree": [
                tar_member("lib", b"file"),
                tar_member("lib/a.dart", b"child"),
            ],
            "subtree before file": [
                tar_member("lib/a.dart", b"child"),
                tar_member("lib", b"file"),
            ],
        }
        for name, members in cases.items():
            with self.subTest(member=name), tempfile.TemporaryDirectory() as temporary:
                archive = Path(temporary) / "package.tar.gz"
                write_tar(archive, members)
                with self.assertRaisesRegex(
                    verify_packages.ValidationError,
                    "unsafe|link|duplicate|non-portable|path conflict",
                ):
                    verify_packages._archive_files(archive)

        with tempfile.TemporaryDirectory() as temporary:
            archive = Path(temporary) / "streaming.tar.gz"
            write_tar(
                archive,
                [
                    tar_member("large.bin", b"x" * 200_000),
                    tar_member("after.txt", b"after"),
                ],
            )
            visited: list[str] = []
            first_chunk_sizes: list[int] = []

            def visit(member: object, chunks: object) -> None:
                visited.append(member.name)
                if member.is_file:
                    first_chunk_sizes.append(len(next(iter(chunks), b"")))

            verify_packages.visit_archive(archive, visit)
            self.assertEqual(visited, ["large.bin", "after.txt"])
            self.assertEqual(first_chunk_sizes, [64 * 1024, 5])

        with self.assertRaisesRegex(
            archive_policy.ArchivePolicyError,
            "non-portable",
        ):
            archive_policy._portable_member_key("lib/bad\udcff.txt")

    def test_archive_reader_rejects_concatenated_gzip_members(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            archive = Path(temporary) / "concatenated.tar.gz"
            inner = io.BytesIO()
            with tarfile.open(fileobj=inner, mode="w") as tar:
                member, payload = tar_member("payload.txt", b"ok")
                tar.addfile(member, io.BytesIO(payload))
            archive.write_bytes(
                gzip.compress(inner.getvalue()) + gzip.compress(b"second member")
            )

            with self.assertRaisesRegex(
                archive_policy.ArchivePolicyError,
                "multiple gzip members",
            ):
                verify_packages.visit_archive(
                    archive,
                    lambda _member, _chunks: None,
                )

    def test_archive_reader_rejects_nonzero_data_after_tar_eoa(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            archive = Path(temporary) / "trailing.tar.gz"
            inner = io.BytesIO()
            with tarfile.open(fileobj=inner, mode="w") as tar:
                member, payload = tar_member("payload.txt", b"ok")
                tar.addfile(member, io.BytesIO(payload))
            archive.write_bytes(gzip.compress(inner.getvalue() + b"trailing"))

            with self.assertRaisesRegex(
                archive_policy.ArchivePolicyError,
                "non-zero data after tar end-of-archive",
            ):
                verify_packages.visit_archive(
                    archive,
                    lambda _member, _chunks: None,
                )

    def test_archive_reader_accepts_canonical_directory_entries(self) -> None:
        for name in ("lib", "lib/"):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temporary:
                archive = Path(temporary) / "package.tar.gz"
                directory = tarfile.TarInfo(name)
                directory.type = tarfile.DIRTYPE
                write_tar(archive, [(directory, b"")])
                visited: list[str] = []

                verify_packages.visit_archive(
                    archive,
                    lambda member, _chunks: visited.append(member.name),
                )

                self.assertEqual(visited, ["lib"])

    def test_extract_only_cli_uses_the_shared_atomic_archive_policy(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "package.tar.gz"
            destination = root / "package"
            write_tar(archive, [tar_member("lib/value.txt", b"verified")])
            stdout = io.StringIO()
            with patch.object(
                sys,
                "argv",
                [
                    str(MODULE_PATH),
                    "--extract-only",
                    str(archive),
                    str(destination),
                ],
            ), redirect_stdout(stdout):
                result = verify_packages.main()

            self.assertEqual(result, 0)
            self.assertEqual(
                (destination / "lib" / "value.txt").read_bytes(),
                b"verified",
            )
            self.assertEqual(
                json.loads(stdout.getvalue())["schema"],
                "mdstream.archive-extraction/1",
            )

    def test_release_tool_bundle_runs_archive_modes_without_a_checkout(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            bundle = root / "release-tools"
            for relative in RELEASE_TOOL_BUNDLE_FILES:
                destination = bundle / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(ROOT / relative, destination)
            archive = root / "package.tar.gz"
            destination = root / "package"
            write_tar(archive, [tar_member("lib/value.txt", b"verified")])

            result = subprocess.run(
                [
                    sys.executable,
                    str(bundle / "scripts" / "verify-packages.py"),
                    "--extract-only",
                    str(archive),
                    str(destination),
                ],
                cwd=root,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                (destination / "lib" / "value.txt").read_bytes(),
                b"verified",
            )
            validate = indented_block(
                (WORKFLOW_ROOT / "release.yml").read_text(encoding="utf-8"),
                "validate:",
            )
            for relative in RELEASE_TOOL_BUNDLE_FILES:
                self.assertIn(relative, validate)
            self.assertIn(
                "target/release-tools/scripts/verify-packages.py --help",
                validate,
            )
            self.assertIn("path: target/release-tools/**", validate)

    def test_special_cli_modes_cannot_bypass_each_other(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "package.tar.gz"
            destination = root / "package"
            write_tar(archive, [tar_member("value.txt", b"verified")])
            stderr = io.StringIO()
            with patch.object(
                sys,
                "argv",
                [
                    str(MODULE_PATH),
                    "--print-rust-order",
                    "--extract-only",
                    str(archive),
                    str(destination),
                ],
            ), redirect_stderr(stderr):
                result = verify_packages.main()

            self.assertEqual(result, 1)
            self.assertIn("cannot be combined", stderr.getvalue())
            self.assertFalse(destination.exists())

        with tempfile.TemporaryDirectory() as temporary:
            archive = Path(temporary) / "package.tar.gz"
            directories = []
            for name in ("lib", "lib/"):
                directory = tarfile.TarInfo(name)
                directory.type = tarfile.DIRTYPE
                directories.append((directory, b""))
            write_tar(archive, directories)

            with self.assertRaisesRegex(
                verify_packages.ValidationError,
                "duplicate member lib",
            ):
                verify_packages._archive_files(archive)

        with tempfile.TemporaryDirectory() as temporary:
            archive = Path(temporary) / "directory-after-child.tar.gz"
            directory = tarfile.TarInfo("lib")
            directory.type = tarfile.DIRTYPE
            write_tar(
                archive,
                [tar_member("lib/a.dart", b"child"), (directory, b"")],
            )
            visited: list[str] = []
            verify_packages.visit_archive(
                archive,
                lambda member, _chunks: visited.append(member.name),
            )
            self.assertEqual(
                visited,
                ["lib/a.dart", "lib"],
            )

    def test_existing_ecosystem_archives_are_verified_in_place(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            write_binding_policy(root, npm_ceiling=64_000, dart_ceiling=64_000)

            npm_archive = root / "mdstream-core-0.4.0.tgz"
            write_files_tar(npm_archive, valid_npm_files(), prefix="package")

            dart_archive = root / "mdstream-0.4.0.tar.gz"
            write_files_tar(dart_archive, valid_dart_files())

            flutter_archive = root / "mdstream_flutter-0.4.0.tar.gz"
            write_files_tar(flutter_archive, valid_flutter_files())

            with patch.object(
                verify_packages,
                "read_archive",
                side_effect=AssertionError("verifier must stream archive payloads"),
                create=True,
            ):
                verify_packages.verify_existing_archive(root, "npm", npm_archive)
                verify_packages.verify_existing_archive(root, "dart", dart_archive)
                verify_packages.verify_existing_archive(root, "flutter", flutter_archive)

    def test_existing_archives_bind_manifest_identity_to_the_release(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            write_binding_policy(root, npm_ceiling=64_000, dart_ceiling=64_000)

            cases: list[tuple[str, dict[str, bytes], str, str]] = []

            npm_files = valid_npm_files()
            npm_manifest = json.loads(npm_files["package.json"])
            npm_manifest["version"] = "9.9.9"
            npm_files["package.json"] = json.dumps(npm_manifest).encode()
            cases.append(("npm", npm_files, "package", "npm package version"))

            for field, value, message in (
                ("name", "other", "Dart package name"),
                ("version", "9.9.9", "Dart package version"),
            ):
                dart_files = valid_dart_files()
                dart_files["pubspec.yaml"] = re.sub(
                    rf"(?m)^{field}:.*$",
                    f"{field}: {value}",
                    dart_files["pubspec.yaml"].decode(),
                ).encode()
                cases.append(("dart", dart_files, "", message))

            for field, value, message in (
                ("name", "other", "Flutter package name"),
                ("version", "9.9.9", "Flutter package version"),
                ("mdstream", "^9.9.9", "Flutter mdstream requirement"),
            ):
                flutter_files = valid_flutter_files()
                pattern = (
                    rf"(?m)^  mdstream:.*$"
                    if field == "mdstream"
                    else rf"(?m)^{field}:.*$"
                )
                replacement = (
                    f"  mdstream: {value}" if field == "mdstream" else f"{field}: {value}"
                )
                flutter_files["pubspec.yaml"] = re.sub(
                    pattern,
                    replacement,
                    flutter_files["pubspec.yaml"].decode(),
                ).encode()
                cases.append(("flutter", flutter_files, "", message))

            for index, (ecosystem, files, prefix, message) in enumerate(cases):
                with self.subTest(ecosystem=ecosystem, case=index):
                    archive = root / f"{ecosystem}-{index}.tar.gz"
                    write_files_tar(archive, files, prefix=prefix or None)
                    with self.assertRaisesRegex(
                        verify_packages.ValidationError,
                        message,
                    ):
                        verify_packages.verify_existing_archive(
                            root,
                            ecosystem,
                            archive,
                        )

    def test_pub_lock_rejects_non_pub_dev_hosted_sources(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            lock = root / "bindings" / "pubspec.lock"
            lock.parent.mkdir(parents=True)
            lock.write_text(
                'packages:\n  ffi:\n    description:\n      url: "https://pub.dev"\n',
                encoding="utf-8",
            )
            verify_packages.validate_pub_lock_sources(root)

            lock.write_text(
                'packages:\n  ffi:\n    description:\n      url: "https://pub.flutter-io.cn"\n',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(
                verify_packages.ValidationError,
                "non-pub.dev hosted source",
            ):
                verify_packages.validate_pub_lock_sources(root)

    def test_existing_archive_enforces_budget_and_dependency_policy(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "package.tgz"

            write_binding_policy(root, npm_ceiling=1, dart_ceiling=64_000)
            write_files_tar(archive, valid_npm_files(), prefix="package")
            with self.assertRaisesRegex(verify_packages.ValidationError, "ceiling"):
                verify_packages.verify_existing_archive(root, "npm", archive)

            write_binding_policy(root, npm_ceiling=64_000, dart_ceiling=64_000)
            for requirement, message in [
                ("file:../local", "local"),
                ("^19.0.0", "forbidden"),
            ]:
                files = valid_npm_files(
                    dependencies={
                        "react" if message == "forbidden" else "local": requirement
                    }
                )
                write_files_tar(archive, files, prefix="package")
                with self.assertRaisesRegex(
                    verify_packages.ValidationError,
                    message,
                ):
                    verify_packages.verify_existing_archive(root, "npm", archive)

            files = valid_npm_files(
                dependencies={"shadowed": "file:../local"}
            )
            manifest = json.loads(files["package.json"])
            manifest["peerDependencies"] = {"shadowed": "^1.0.0"}
            files["package.json"] = json.dumps(manifest).encode()
            write_files_tar(archive, files, prefix="package")
            with self.assertRaisesRegex(
                verify_packages.ValidationError,
                "local",
            ):
                verify_packages.verify_existing_archive(root, "npm", archive)

            dart_archive = root / "package.tar.gz"
            write_files_tar(
                dart_archive,
                valid_dart_files(extra_dependency="crypto: ^3.0.7"),
            )
            with self.assertRaisesRegex(
                verify_packages.ValidationError,
                "only ffi",
            ):
                verify_packages.verify_existing_archive(root, "dart", dart_archive)

    def test_existing_archive_rejects_native_binary_magic(self) -> None:
        magics = {
            "elf": b"\x7fELFpayload",
            "macho": b"\xfe\xed\xfa\xcfpayload",
            "pe": b"MZpayload",
            "archive": b"!<arch>\npayload",
        }
        for name, magic in magics.items():
            with self.subTest(format=name), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                write_binding_policy(root, npm_ceiling=64_000, dart_ceiling=64_000)
                files = valid_dart_files()
                files[f"lib/native-{name}.bin"] = magic
                archive = root / "package.tar.gz"
                write_files_tar(archive, files)
                with self.assertRaisesRegex(
                    verify_packages.ValidationError,
                    "native binary magic",
                ):
                    verify_packages.verify_existing_archive(root, "dart", archive)

    def test_flutter_archive_rejects_native_files_outside_inventory(self) -> None:
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
                root = Path(temporary)
                write_binding_policy(root, npm_ceiling=64_000, dart_ceiling=64_000)
                files = valid_flutter_files()
                files[name] = data
                archive = root / "flutter.tar.gz"
                write_files_tar(archive, files)

                with self.assertRaisesRegex(
                    verify_packages.ValidationError,
                    "outside canonical native inventory",
                ):
                    verify_packages.verify_existing_archive(root, "flutter", archive)

    def test_flutter_native_inventory_rejections_match_the_deep_package_smoke(self) -> None:
        native = elf_image("x86_64")
        coff_object = struct.pack("<HHIIIHH", 0x8664, 1, 0, 0, 0, 0, 0) + bytes(40)
        bigobj_class_id = bytes.fromhex("c7a1bad1eebaa94baf20faf66aa4dcb8")
        coff_bigobj_v2 = (
            b"\x00\x00\xff\xff\x02\x00\x64\x86"
            + bytes(4)
            + bigobj_class_id
            + bytes(12)
        )
        coff_bigobj_v3 = (
            b"\x00\x00\xff\xff\x03\x00\x64\x86"
            + bytes(4)
            + bigobj_class_id
            + bytes(12)
        )
        wrong_bigobj_uuid = (
            b"\x00\x00\xff\xff\x03\x00\x64\x86"
            + bytes(4)
            + bytes(16)
            + bytes(12)
        )
        self.assertTrue(is_native_like_artifact("assets/v2.bin", coff_bigobj_v2))
        self.assertTrue(is_native_like_artifact("assets/v3.bin", coff_bigobj_v3))
        self.assertFalse(is_native_like_artifact("assets/wrong.bin", wrong_bigobj_uuid))
        cases = {
            "COFF object outside inventory": ("assets/hidden.obj", coff_object),
            "COFF BigObj v2 under a neutral suffix": (
                "assets/hidden-bigobj-v2.bin",
                coff_bigobj_v2,
            ),
            "COFF BigObj v3 under a neutral suffix": (
                "assets/hidden-bigobj-v3.bin",
                coff_bigobj_v3,
            ),
            "PDB magic under a neutral suffix": (
                "assets/hidden.bin",
                b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0",
            ),
            "extra file inside reserved native directory": (
                "android/src/main/jniLibs/x86_64/README.txt",
                b"not part of the native inventory",
            ),
        }

        for label, (name, data) in cases.items():
            with self.subTest(case=label), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                write_binding_policy(root, npm_ceiling=64_000, dart_ceiling=64_000)
                files = valid_flutter_files()
                files["android/src/main/jniLibs/x86_64/libmdstream_ffi.so"] = native
                files[name] = data
                archive = root / "flutter.tar.gz"
                write_files_tar(archive, files)

                with self.assertRaisesRegex(
                    verify_packages.ValidationError,
                    "outside canonical native inventory",
                ):
                    verify_packages.verify_existing_archive(root, "flutter", archive)
                with self.assertRaisesRegex(
                    package_smoke.PackageSmokeError,
                    "outside canonical native inventory",
                ):
                    package_smoke.inspect_package_archive(
                        archive,
                        forbidden_terms=set(),
                        native_ceiling_bytes=len(native),
                        increment_ceiling_bytes=sum(map(len, files.values())),
                        require_all_platforms=False,
                    )

    def test_repacked_archive_comparison_uses_file_content_not_tar_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            expected = root / "expected.tar.gz"
            candidate = root / "candidate.tar.gz"
            files = {"pubspec.yaml": b"name: mdstream\n", "lib/a.dart": b"a"}
            write_files_tar(expected, files)
            write_files_tar(candidate, dict(reversed(tuple(files.items()))))

            verify_packages.compare_archive_file_contents(expected, candidate)

            changed = dict(files)
            changed["lib/a.dart"] = b"changed"
            write_files_tar(candidate, changed)
            with self.assertRaisesRegex(
                verify_packages.ValidationError,
                "changed file content lib/a.dart",
            ):
                verify_packages.compare_archive_file_contents(expected, candidate)

            write_files_tar(candidate, {"pubspec.yaml": files["pubspec.yaml"]})
            with self.assertRaisesRegex(
                verify_packages.ValidationError,
                "missing file lib/a.dart",
            ):
                verify_packages.compare_archive_file_contents(expected, candidate)

            extra = dict(files)
            extra["extra.txt"] = b"extra"
            write_files_tar(candidate, extra)
            with self.assertRaisesRegex(
                verify_packages.ValidationError,
                "extra file extra.txt",
            ):
                verify_packages.compare_archive_file_contents(expected, candidate)

    def test_archive_fingerprints_reject_nonzero_data_after_tar_eoa(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            expected = root / "expected.tar.gz"
            candidate = root / "candidate.tar.gz"
            files = {"pubspec.yaml": b"name: mdstream\n", "lib/a.dart": b"a"}
            write_files_tar(expected, files)
            write_files_tar(candidate, files)
            raw_tar = gzip.decompress(candidate.read_bytes())
            candidate.write_bytes(gzip.compress(raw_tar + b"not fingerprinted"))

            with self.assertRaisesRegex(
                verify_packages.ValidationError,
                "non-zero data after tar end-of-archive",
            ):
                verify_packages.archive_file_fingerprints(candidate)
            with self.assertRaisesRegex(
                verify_packages.ValidationError,
                "non-zero data after tar end-of-archive",
            ):
                verify_packages.compare_archive_file_contents(expected, candidate)

    def test_registry_archive_descriptors_bind_identity_and_checksum(self) -> None:
        checksum = b"x" * 64
        npm = verify_packages.registry_archive_descriptor(
            "npm",
            "@mdstream/core",
            "0.4.0",
            {
                "name": "@mdstream/core",
                "version": "0.4.0",
                "dist": {
                    "tarball": (
                        "https://registry.npmjs.org/@mdstream/core/-/core-0.4.0.tgz"
                    ),
                    "integrity": f"sha512-{base64.b64encode(checksum).decode()}",
                },
            },
        )
        self.assertEqual(npm.checksum_algorithm, "sha512")
        self.assertEqual(npm.checksum, checksum)

        pub = verify_packages.registry_archive_descriptor(
            "pub.dev",
            "mdstream",
            "0.4.0",
            {
                "version": "0.4.0",
                "archive_url": "https://pub.dev/api/archives/mdstream-0.4.0.tar.gz",
                "archive_sha256": "ab" * 32,
            },
        )
        self.assertEqual(pub.checksum_algorithm, "sha256")
        self.assertEqual(pub.checksum, bytes.fromhex("ab" * 32))

        crates = verify_packages.registry_archive_descriptor(
            "crates.io",
            "mdstream",
            "0.4.0",
            {
                "version": {
                    "crate": "mdstream",
                    "num": "0.4.0",
                    "yanked": False,
                    "checksum": "cd" * 32,
                }
            },
        )
        self.assertEqual(crates.checksum_algorithm, "sha256")
        self.assertEqual(crates.checksum, bytes.fromhex("cd" * 32))

        for registry, metadata in (
            (
                "pub.dev",
                {
                    "version": "0.4.0",
                    "archive_url": "https://pub.dev/archive.tar.gz",
                    "retracted": True,
                },
            ),
            (
                "crates.io",
                {
                    "version": {
                        "crate": "mdstream",
                        "num": "0.4.0",
                        "yanked": True,
                        "checksum": "cd" * 32,
                    }
                },
            ),
        ):
            with self.subTest(registry=registry), self.assertRaisesRegex(
                verify_packages.ValidationError,
                "retracted|yanked",
            ):
                verify_packages.registry_archive_descriptor(
                    registry,
                    "mdstream",
                    "0.4.0",
                    metadata,
                )

        with self.assertRaisesRegex(
            verify_packages.ValidationError,
            "outside registry.npmjs.org",
        ):
            verify_packages.registry_archive_descriptor(
                "npm",
                "@mdstream/core",
                "0.4.0",
                {
                    "name": "@mdstream/core",
                    "version": "0.4.0",
                    "dist": {
                        "tarball": "https://example.com/substituted.tgz",
                        "shasum": "ab" * 20,
                    },
                },
            )

    def test_existing_registry_version_must_match_producer_archive(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            expected = root / "expected.tar.gz"
            registry = root / "registry.tar.gz"
            write_files_tar(expected, {"pubspec.yaml": b"expected"})
            write_files_tar(registry, {"pubspec.yaml": b"different"})

            metadata = {
                "version": "0.4.0",
                "archive_url": "https://pub.dev/archive.tar.gz",
            }

            def download(
                _url: str,
                destination: Path,
                **_kwargs: object,
            ) -> None:
                shutil.copyfile(registry, destination)

            with patch.object(
                verify_packages,
                "_registry_metadata",
                return_value=metadata,
            ), patch.object(verify_packages, "_curl_to_path", side_effect=download):
                with self.assertRaisesRegex(
                    verify_packages.ValidationError,
                    "pub.dev archive changed file content pubspec.yaml",
                ):
                    verify_packages.verify_registry_archive(
                        "pub.dev",
                        "mdstream",
                        "0.4.0",
                        expected,
                    )

    def test_pack_step_must_produce_exactly_one_archive(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            with self.assertRaisesRegex(verify_packages.ValidationError, "exactly one"):
                verify_packages._single_archive(directory, "*.tgz", "npm pack")
            (directory / "first.tgz").touch()
            self.assertEqual(
                verify_packages._single_archive(directory, "*.tgz", "npm pack"),
                directory / "first.tgz",
            )
            (directory / "second.tgz").touch()
            with self.assertRaisesRegex(verify_packages.ValidationError, "exactly one"):
                verify_packages._single_archive(directory, "*.tgz", "npm pack")

    def test_documentation_contract_rejects_a_broken_local_link(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            docs = root / "docs"
            docs.mkdir()
            links = "\n".join(
                f"- [{name}](docs/{name})" for name in verify_packages.REQUIRED_DOCUMENTS
            )
            (root / "README.md").write_text(
                links + "\n[missing](docs/MISSING.md)\n",
                encoding="utf-8",
            )
            for name in verify_packages.REQUIRED_DOCUMENTS:
                (docs / name).write_text(f"# {name}\n", encoding="utf-8")

            with self.assertRaisesRegex(
                verify_packages.ValidationError,
                "broken Markdown link.*MISSING.md",
            ):
                verify_packages.validate_documentation_contract(root)

    def test_example_catalog_requires_complete_machine_checked_entries(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            docs = root / "docs"
            docs.mkdir()
            source_paths = {
                contract.source_path
                for contract in verify_packages.EXAMPLE_CONTRACTS
            }
            for relative in source_paths:
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("example\n", encoding="utf-8")
            sections = []
            for contract in verify_packages.EXAMPLE_CONTRACTS:
                sections.append(
                    "\n".join(
                        (
                            f"<!-- example:{contract.identifier} -->",
                            f"- Role: {contract.role}",
                            f"- Source: [{contract.source_path}]"
                            f"(../{contract.source_path})",
                            f"- Prerequisites: {contract.prerequisite_marker}",
                            f"- Run: `{contract.command}`",
                            f"- Expect: `{contract.expected_marker}`",
                            f"- Next: [Continue]({contract.next_link})",
                            "<!-- /example -->",
                        )
                    )
                )
            catalog = "# Examples\n\n" + "\n\n".join(sections) + "\n"
            (docs / "EXAMPLES.md").write_text(catalog, encoding="utf-8")
            (root / "README.md").write_text(
                "\n".join(
                    f"- [{contract.identifier}]"
                    f"(docs/EXAMPLES.md#{contract.identifier})"
                    for contract in verify_packages.EXAMPLE_CONTRACTS
                )
                + "\n",
                encoding="utf-8",
            )

            verify_packages.validate_example_catalog(root)

            first = verify_packages.EXAMPLE_CONTRACTS[0]
            broken = catalog.replace(
                f"- Next: [Continue]({first.next_link})",
                "- Next: nowhere",
                1,
            )
            (docs / "EXAMPLES.md").write_text(broken, encoding="utf-8")
            with self.assertRaisesRegex(
                verify_packages.ValidationError,
                f"{first.identifier}.*Next",
            ):
                verify_packages.validate_example_catalog(root)

    def test_flutter_catalog_distinguishes_source_and_published_runs(self) -> None:
        flutter = next(
            contract
            for contract in verify_packages.EXAMPLE_CONTRACTS
            if contract.identifier == "flutter-host"
        )
        self.assertEqual(
            flutter.command,
            "python3 bindings/flutter/tool/build_native.py macos && "
            "cd bindings/flutter/example && flutter create --empty "
            "--platforms macos --project-name mdstream_flutter_example "
            "--org io.mdstream.example --no-pub . && "
            "dart run configure_host.dart macos && flutter run -d macos",
        )

        catalog = (ROOT / "docs" / "EXAMPLES.md").read_text(encoding="utf-8")
        section = catalog.split("<!-- example:flutter-host -->", 1)[1].split(
            "<!-- /example -->", 1
        )[0]
        self.assertIn(
            "From an extracted published package, start at `cd example && "
            "flutter create --empty --platforms macos --project-name "
            "mdstream_flutter_example --org io.mdstream.example --no-pub . "
            "&& dart run configure_host.dart macos && flutter run -d macos`; "
            "its native artifacts are already staged.",
            section,
        )

    def test_repository_static_contract(self) -> None:
        contract = verify_packages.validate_static_contract(ROOT)
        self.assertEqual(contract.version, "0.4.0")
        self.assertEqual(
            contract.rust_publish_order,
            verify_packages.RUST_PUBLISH_ORDER,
        )

    def test_reusable_workflows_isolate_their_concurrency_groups(self) -> None:
        expected_groups = {
            "ci.yml": "group: ci-${{ github.workflow }}-${{ github.ref }}",
            "flutter-platforms.yml": (
                "group: flutter-platforms-${{ github.workflow }}-${{ github.ref }}"
            ),
            "release.yml": "group: release-${{ github.workflow }}-${{ github.ref }}",
        }
        for filename, expected in expected_groups.items():
            with self.subTest(workflow=filename):
                workflow = (WORKFLOW_ROOT / filename).read_text(encoding="utf-8")
                self.assertIn(expected, workflow)

    def test_workflow_contract_rejects_silent_gate_bypasses(self) -> None:
        def replace_job_needs(text: str, job_name: str, replacement: str) -> str:
            start = text.index(f"  {job_name}:\n")
            line_start = text.index("    needs:", start)
            line_end = text.index("\n", line_start)
            return text[:line_start] + f"    needs: {replacement}" + text[line_end:]

        def replace_job_fragment(
            text: str,
            job_name: str,
            original: str,
            replacement: str,
        ) -> str:
            start = text.index(f"  {job_name}:\n")
            next_job = re.search(r"(?m)^  [A-Za-z0-9_-]+:\s*$", text[start + 1 :])
            end = start + 1 + next_job.start() if next_job is not None else len(text)
            block = text[start:end]
            if original not in block:
                raise AssertionError(f"{original!r} not found in {job_name}")
            return text[:start] + block.replace(original, replacement, 1) + text[end:]

        def comment_out_web_test(text: str) -> str:
            return text.replace("run: pnpm -r test", "# run: pnpm -r test", 1)

        def disable_web_job(text: str) -> str:
            return text.replace(
                "  web:\n    name: WASM and TypeScript / Node 24",
                "  web:\n    if: false\n    name: WASM and TypeScript / Node 24",
                1,
            )

        def disable_web_test_step(text: str) -> str:
            return text.replace(
                "      - name: Test TypeScript packages\n"
                "        run: pnpm -r test",
                "      - name: Test TypeScript packages\n"
                "        if: false\n"
                "        run: pnpm -r test",
                1,
            )

        def move_web_test_to_quality(text: str) -> str:
            text = text.replace("run: pnpm -r test", "run: pnpm -r typecheck", 1)
            return text.replace(
                "run: cargo fmt --all -- --check",
                "run: cargo fmt --all -- --check\n\n"
                "      - name: Misplaced web test\n"
                "        run: pnpm -r test",
                1,
            )

        def move_wasm_tools_after_build(text: str) -> str:
            install = (
                "      - name: Install wasm-tools 1.253.0\n"
                "        uses: taiki-e/install-action@v2\n"
                "        with:\n"
                "          tool: wasm-tools@1.253.0\n\n"
            )
            text = text.replace(install, "", 1)
            build = (
                "      - name: Build TypeScript packages\n"
                "        run: pnpm -r build"
            )
            return text.replace(build, f"{build}\n\n{install.rstrip()}", 1)

        def move_flutter_sync_after_integration(text: str) -> str:
            sync = (
                "      - name: Verify Golden fixture synchronization\n"
                "        run: python3 scripts/sync-example-fixtures.py --check\n\n"
            )
            text = text.replace(sync, "", 1)
            integration = (
                "        run: xvfb-run -a flutter test "
                "integration_test/golden_stream_smoke_test.dart -d linux"
            )
            return text.replace(integration, f"{integration}\n\n{sync.rstrip()}", 1)

        cases = (
            ("commented command", "ci.yml", comment_out_web_test),
            ("statically disabled job", "ci.yml", disable_web_job),
            ("statically disabled step", "ci.yml", disable_web_test_step),
            ("wrong job", "ci.yml", move_web_test_to_quality),
            ("late wasm-tools install", "ci.yml", move_wasm_tools_after_build),
            (
                "late Flutter fixture check",
                "flutter-platforms.yml",
                move_flutter_sync_after_integration,
            ),
            (
                "missing Golden sync",
                "ci.yml",
                lambda text: text.replace(
                    "run: python3 scripts/sync-example-fixtures.py --check",
                    "# removed Golden sync",
                    1,
                ),
            ),
            (
                "missing Rust assertion entry",
                "ci.yml",
                lambda text: text.replace(
                    "run: cargo run -p mdstream --example minimal -- --assert",
                    "# removed Rust assertion entry",
                    1,
                ),
            ),
            (
                "missing Tokio smoke entry",
                "ci.yml",
                lambda text: text.replace(
                    "run: cargo run -p mdstream-tokio --example agent_tui -- --smoke",
                    "# removed Tokio smoke entry",
                    1,
                ),
            ),
            (
                "missing Web browser entry",
                "ci.yml",
                lambda text: text.replace(
                    "run: pnpm --filter @mdstream/example-web test:e2e",
                    "# removed Web browser entry",
                    1,
                ),
            ),
            (
                "missing TypeScript transition probe",
                "ci.yml",
                lambda text: text.replace(
                    "run: node bindings/typescript/examples/transition-host.mjs --assert",
                    "# removed TypeScript transition probe",
                    1,
                ),
            ),
            (
                "missing Dart Golden entry",
                "ci.yml",
                lambda text: text.replace(
                    'dart run example/golden_stream.dart --library "$LIBRARY" --assert',
                    "# removed Dart Golden entry",
                    1,
                ),
            ),
            (
                "missing Merman assertion entry",
                "ci.yml",
                lambda text: text.replace(
                    "run: cargo +1.95.0 run --manifest-path mdstream-merman/Cargo.toml --example render_golden -- --assert",
                    "# removed Merman assertion entry",
                    1,
                ),
            ),
            (
                "missing Flutter bundled bootstrap entry",
                "flutter-platforms.yml",
                lambda text: text.replace(
                    "run: xvfb-run -a flutter test integration_test/golden_stream_smoke_test.dart -d linux",
                    "# removed Flutter bundled bootstrap entry",
                    1,
                ),
            ),
            (
                "missing workflow call trigger",
                "ci.yml",
                lambda text: text.replace("  workflow_call:\n", "", 1),
            ),
            (
                "wrong reusable call edge",
                "release.yml",
                lambda text: text.replace(
                    "uses: ./.github/workflows/ci.yml",
                    "uses: ./.github/workflows/flutter-platforms.yml",
                    1,
                ),
            ),
            (
                "missing pull request trigger",
                "flutter-platforms.yml",
                lambda text: text.replace(
                    "  pull_request:\n    branches: [main, master]\n",
                    "",
                    1,
                ),
            ),
            (
                "npm build waits on quality",
                "release.yml",
                lambda text: replace_job_needs(
                    text,
                    "build-npm",
                    "[validate, quality]",
                ),
            ),
            (
                "dart build waits on quality",
                "release.yml",
                lambda text: replace_job_needs(
                    text,
                    "build-dart",
                    "[validate, quality]",
                ),
            ),
            (
                "npm publish bypasses quality",
                "release.yml",
                lambda text: replace_job_needs(
                    text,
                    "publish-npm",
                    "[validate, build-npm]",
                ),
            ),
            (
                "dart publish bypasses quality",
                "release.yml",
                lambda text: replace_job_needs(
                    text,
                    "publish-dart",
                    "[validate, build-dart]",
                ),
            ),
            (
                "flutter publish bypasses quality",
                "release.yml",
                lambda text: replace_job_needs(
                    text,
                    "publish-flutter",
                    "[validate, publish-dart, flutter-platforms]",
                ),
            ),
            (
                "Rust publish bypasses Flutter native platforms",
                "release.yml",
                lambda text: replace_job_needs(
                    text,
                    "publish-rust",
                    "[validate, quality]",
                ),
            ),
            (
                "Rust publish bypasses producer preflight",
                "release.yml",
                lambda text: replace_job_needs(
                    text,
                    "publish-rust",
                    "[validate, quality, flutter-platforms]",
                ),
            ),
            (
                "npm publish bypasses Flutter native platforms",
                "release.yml",
                lambda text: replace_job_needs(
                    text,
                    "publish-npm",
                    "[validate, quality, build-npm]",
                ),
            ),
            (
                "Dart publish bypasses Flutter native platforms",
                "release.yml",
                lambda text: replace_job_needs(
                    text,
                    "publish-dart",
                    "[validate, quality, build-dart]",
                ),
            ),
            (
                "Flutter publish bypasses Flutter native platforms",
                "release.yml",
                lambda text: replace_job_needs(
                    text,
                    "publish-flutter",
                    "[validate, quality, publish-dart]",
                ),
            ),
            (
                "missing producer preflight",
                "release.yml",
                lambda text: text.replace(
                    "  release-preflight:\n",
                    "  release-preflight-disabled:\n",
                    1,
                ),
            ),
            *(
                (
                    f"{job_name} publishes after a registry probe error",
                    "release.yml",
                    lambda text, job_name=job_name: replace_job_fragment(
                        text,
                        job_name,
                        'if [[ "$registry_status" -ne 1 ]]; then',
                        'if [[ "$registry_status" -eq 1 ]]; then',
                    ),
                )
                for job_name in (
                    "publish-rust",
                    "publish-npm",
                    "publish-dart",
                    "publish-flutter",
                )
            ),
        )
        for label, filename, mutate in cases:
            with self.subTest(bypass=label), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                workflows = root / ".github" / "workflows"
                shutil.copytree(WORKFLOW_ROOT, workflows)
                path = workflows / filename
                path.write_text(
                    mutate(path.read_text(encoding="utf-8")),
                    encoding="utf-8",
                )
                with self.assertRaisesRegex(
                    verify_packages.ValidationError,
                    "workflow|gate|job|call",
                ):
                    verify_packages.validate_workflow_contract(root)

    def test_workflow_contract_rejects_conditional_or_ignored_release_gates(self) -> None:
        release = (WORKFLOW_ROOT / "release.yml").read_text(encoding="utf-8")
        mutations = {
            "dynamic job condition": release.replace(
                "  publish-npm:\n    name: Publish @mdstream/core",
                "  publish-npm:\n    if: ${{ false && always() }}\n"
                "    name: Publish @mdstream/core",
                1,
            ),
            "dynamic step condition": release.replace(
                "      - name: Publish npm package when missing\n",
                "      - name: Publish npm package when missing\n"
                "        if: ${{ false && always() }}\n",
                1,
            ),
            "ignored step failure": release.replace(
                "      - name: Publish npm package when missing\n",
                "      - name: Publish npm package when missing\n"
                "        continue-on-error: true\n",
                1,
            ),
        }
        for label, mutated in mutations.items():
            with self.subTest(bypass=label), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                workflows = root / ".github" / "workflows"
                shutil.copytree(WORKFLOW_ROOT, workflows)
                (workflows / "release.yml").write_text(mutated, encoding="utf-8")
                with self.assertRaisesRegex(
                    verify_packages.ValidationError,
                    "conditional|continue-on-error",
                ):
                    verify_packages.validate_workflow_contract(root)

    def test_crates_io_wait_uses_the_cargo_registry_view(self) -> None:
        release = (WORKFLOW_ROOT / "release.yml").read_text(encoding="utf-8")
        publish = indented_block(release, "publish-rust:")

        self.assertIn(
            'timeout 30s cargo info --registry crates-io "$crate@$VERSION"',
            publish,
        )
        self.assertIn(
            "cargo +1.95.0 publish --manifest-path "
            "mdstream-merman/Cargo.toml --locked --token",
            publish,
        )
        self.assertIn('cargo publish -p "$crate" --locked --token', publish)
        self.assertIn("--locked", verify_packages._cargo_package_command("mdstream-merman"))

    def test_trusted_publish_jobs_only_publish_verified_artifacts(self) -> None:
        release = (WORKFLOW_ROOT / "release.yml").read_text(encoding="utf-8")
        expected_needs = {
            "build-npm:": "needs: validate",
            "build-dart:": "needs: validate",
            "release-preflight:": (
                "needs: [validate, quality, flutter-platforms, build-npm, build-dart]"
            ),
            "publish-rust:": (
                "needs: [validate, quality, flutter-platforms, release-preflight]"
            ),
            "publish-npm:": (
                "needs: [validate, quality, build-npm, flutter-platforms, release-preflight]"
            ),
            "publish-dart:": (
                "needs: [validate, quality, build-dart, flutter-platforms, release-preflight]"
            ),
            "publish-flutter:": (
                "needs: [validate, quality, publish-dart, flutter-platforms, release-preflight]"
            ),
        }
        for marker, needs in expected_needs.items():
            with self.subTest(dependencies=marker):
                self.assertIn(needs, indented_block(release, marker))

        expectations = {
            "publish-npm:": "npm publish",
            "publish-dart:": "dart pub publish --force --skip-validation",
            "publish-flutter:": "dart pub publish --force --skip-validation",
        }
        forbidden = (
            "actions/checkout",
            "cargo build",
            "flutter pub get",
            "pnpm install",
            "wasm-pack build",
        )
        for marker, publish_command in expectations.items():
            with self.subTest(job=marker):
                job = indented_block(release, marker)
                self.assertIn("id-token: write", job)
                self.assertIn("actions/download-artifact", job)
                self.assertIn(publish_command, job)
                for disallowed in forbidden:
                    self.assertNotIn(disallowed, job)

        for marker in ("publish-dart:", "publish-flutter:"):
            with self.subTest(safe_extraction=marker):
                job = indented_block(release, marker)
                self.assertIn("--extract-only", job)
                self.assertNotRegex(job, r"\btar\s+-[A-Za-z]*x")

    def test_dart_ci_requires_native_for_the_complete_suite(self) -> None:
        workflow = (WORKFLOW_ROOT / "ci.yml").read_text(encoding="utf-8")
        job = indented_block(workflow, "dart:")
        native_suite = job.index("dart run tool/test_native.dart")
        example_build = job.index('LIBRARY="$(dart run tool/build_native.dart)"')
        example = job.index(
            'dart run example/golden_stream.dart --library "$LIBRARY" --assert'
        )
        self.assertLess(native_suite, example_build)
        self.assertLess(example_build, example)
        self.assertEqual(job.count("dart run tool/test_native.dart"), 1)
        self.assertNotIn("run: dart test", job)

    def test_typescript_build_jobs_install_pinned_wasm_tools(self) -> None:
        cases = (
            ("ci.yml", "web:", "pnpm -r build"),
            ("release.yml", "build-npm:", "pnpm --filter @mdstream/core pack"),
        )
        for filename, job_marker, build_marker in cases:
            with self.subTest(workflow=filename):
                workflow = (WORKFLOW_ROOT / filename).read_text(encoding="utf-8")
                job = indented_block(workflow, job_marker)
                install = "tool: wasm-tools@1.253.0"
                self.assertEqual(job.count(install), 1)
                self.assertLess(job.index(install), job.index(build_marker))

    def test_release_producers_verify_the_exact_uploaded_archive(self) -> None:
        release = (WORKFLOW_ROOT / "release.yml").read_text(encoding="utf-8")
        cases = {
            "build-npm:": ("NPM_ARCHIVE", "pnpm --filter @mdstream/core pack"),
            "build-dart:": ("DART_ARCHIVE", "dart pub publish --to-archive"),
        }
        for marker, (variable, pack_command) in cases.items():
            with self.subTest(job=marker):
                job = indented_block(release, marker)
                verify_command = (
                    f'--ecosystem {"npm" if variable == "NPM_ARCHIVE" else "dart"} '
                    f'--archive "${variable}"'
                )
                upload_path = f"path: ${{{{ env.{variable} }}}}"
                self.assertIn(f"{variable}:", job)
                self.assertIn(verify_command, job)
                self.assertIn(upload_path, job)
                self.assertLess(job.index(pack_command), job.index(verify_command))
                self.assertLess(job.index(verify_command), job.index(upload_path))

    def test_flutter_producer_verifies_and_uploads_dynamic_archive(self) -> None:
        workflow = (WORKFLOW_ROOT / "flutter-platforms.yml").read_text(
            encoding="utf-8"
        )
        package_job = indented_block(workflow, "package:")
        resolve = (
            "FLUTTER_ARCHIVE=$(python3 "
            "bindings/flutter/tool/package_smoke.py --print-archive-path)"
        )
        specialized = "bindings/flutter/tool/package_smoke.py --release"
        exact = '--ecosystem flutter --archive "$FLUTTER_ARCHIVE"'
        upload = "path: ${{ env.FLUTTER_ARCHIVE }}"

        self.assertIn(resolve, package_job)
        self.assertIn(specialized, package_job)
        self.assertIn(exact, package_job)
        self.assertIn(upload, package_job)
        self.assertNotIn("mdstream_flutter-0.4.0.tar.gz", package_job)
        self.assertLess(package_job.index(resolve), package_job.index(specialized))
        self.assertLess(package_job.index(specialized), package_job.index(exact))
        self.assertLess(package_job.index(exact), package_job.index(upload))

        release = (WORKFLOW_ROOT / "release.yml").read_text(encoding="utf-8")
        publish = indented_block(release, "publish-flutter:")
        self.assertIn(
            "mdstream_flutter-${{ needs.validate.outputs.version }}.tar.gz",
            publish,
        )

    def test_flutter_exact_archive_runs_linux_runtime_smoke(self) -> None:
        workflow = (WORKFLOW_ROOT / "flutter-platforms.yml").read_text(
            encoding="utf-8"
        )
        smoke = indented_block(workflow, "package-linux-smoke:")
        archive_runtime = (
            'package_smoke.py --archive "$FLUTTER_ARCHIVE" '
            "--platform linux --device linux --skip-native-build"
        )

        self.assertIn("needs: package", smoke)
        self.assertIn("name: mdstream-flutter-package", smoke)
        self.assertIn(archive_runtime, smoke)
        self.assertNotIn("--skip-runtime", smoke)

    def test_flutter_exact_archive_runs_ios_runtime_and_swiftpm_smokes(self) -> None:
        workflow = (WORKFLOW_ROOT / "flutter-platforms.yml").read_text(
            encoding="utf-8"
        )
        smoke = indented_block(workflow, "package-ios-smoke:")
        pods = (
            'package_smoke.py --archive "$FLUTTER_ARCHIVE" '
            '--platform ios --device "$DEVICE_ID" --skip-native-build'
        )
        swiftpm = (
            'package_smoke.py --swiftpm --archive "$FLUTTER_ARCHIVE" '
            '--platform ios --device "$DEVICE_ID" --skip-native-build'
        )

        self.assertIn("needs: package", smoke)
        self.assertIn("name: mdstream-flutter-package", smoke)
        self.assertIn('xcrun simctl bootstatus "$DEVICE_ID" -b', smoke)
        self.assertIn(pods, smoke)
        self.assertIn(swiftpm, smoke)
        self.assertLess(smoke.index(pods), smoke.index(swiftpm))

    def test_android_16k_emulator_uses_an_action_that_supports_ps16k(self) -> None:
        workflow = (WORKFLOW_ROOT / "flutter-platforms.yml").read_text(
            encoding="utf-8"
        )
        android = indented_block(workflow, "android:")
        self.assertIn(
            "reactivecircus/android-emulator-runner@v2.38.0",
            android,
        )
        self.assertIn("target: google_apis_ps16k", android)

    def test_flutter_exact_archive_runs_swiftpm_smoke(self) -> None:
        workflow = (WORKFLOW_ROOT / "flutter-platforms.yml").read_text(
            encoding="utf-8"
        )
        smoke = indented_block(workflow, "package-apple-swiftpm-smoke:")
        archive_runtime = (
            'package_smoke.py --swiftpm --archive "$FLUTTER_ARCHIVE" '
            "--platform macos --skip-native-build"
        )

        self.assertIn("needs: package", smoke)
        self.assertIn("name: mdstream-flutter-package", smoke)
        self.assertIn(archive_runtime, smoke)

    def test_flutter_archive_contract_includes_swiftpm_metadata(self) -> None:
        required = verify_packages.FLUTTER_REQUIRED_FILES
        for platform_name in ("ios", "macos"):
            with self.subTest(platform=platform_name):
                self.assertIn(
                    f"{platform_name}/mdstream_flutter/Package.swift", required
                )
                self.assertIn(
                    f"{platform_name}/mdstream_flutter/Sources/"
                    "mdstream_flutter/MdstreamFlutterPackage.swift",
                    required,
                )

    def test_ios_runtime_smoke_waits_for_simulator_boot(self) -> None:
        workflow = (WORKFLOW_ROOT / "flutter-platforms.yml").read_text(
            encoding="utf-8"
        )
        apple = indented_block(workflow, "apple:")
        boot = 'xcrun simctl boot "$DEVICE_ID"'
        ready = 'xcrun simctl bootstatus "$DEVICE_ID" -b'
        smoke = "package_smoke.py --platform ios"

        self.assertIn(boot, apple)
        self.assertIn(ready, apple)
        self.assertIn(smoke, apple)
        self.assertLess(apple.index(boot), apple.index(ready))
        self.assertLess(apple.index(ready), apple.index(smoke))
        boot_step = indented_block(
            apple,
            "- name: Boot iOS simulator and load bundled library",
        )
        self.assertIn("timeout-minutes: 30", boot_step)

    def test_release_notes_are_verified_before_any_registry_publish(self) -> None:
        release = (WORKFLOW_ROOT / "release.yml").read_text(encoding="utf-8")
        validate = indented_block(release, "validate:")
        github_release = indented_block(release, "github-release:")

        self.assertIn("scripts/release_notes.py", validate)
        self.assertIn("--output target/release-notes.md", validate)
        self.assertIn("name: mdstream-release-notes", validate)
        self.assertIn("path: target/release-notes.md", validate)
        self.assertNotIn("actions/checkout", github_release)
        self.assertIn("name: mdstream-release-notes", github_release)
        self.assertIn(
            "body_path: target/release-notes/release-notes.md",
            github_release,
        )

    def test_registry_checks_preserve_request_deadlines(self) -> None:
        endpoints = check_registry_version.REGISTRIES
        self.assertEqual(endpoints["crates.io"].max_time, 30)
        self.assertEqual(endpoints["pub.dev"].connect_timeout, 5)
        self.assertEqual(endpoints["pub.dev"].max_time, 20)

        with tempfile.TemporaryDirectory() as temporary:
            destination = Path(temporary) / "download"
            commands: list[tuple[str, ...]] = []

            def run(command: tuple[str, ...], **_kwargs: object) -> None:
                commands.append(command)
                destination.write_bytes(b"payload")

            with patch.object(verify_packages, "_run", side_effect=run):
                verify_packages._curl_to_path(
                    "https://pub.dev/archive.tar.gz",
                    destination,
                    connect_timeout=5,
                    max_time=20,
                    max_bytes=1024,
                )

            self.assertEqual(len(commands), 1)
            command = commands[0]
            self.assertIn("--connect-timeout", command)
            self.assertIn("5", command)
            self.assertIn("--max-time", command)
            self.assertIn("20", command)
            self.assertIn("--max-filesize", command)
            self.assertIn("--proto-redir", command)
            self.assertIn("--user-agent", command)
            self.assertIn(verify_packages.REGISTRY_USER_AGENT, command)

    def test_dead_dart_package_verifier_and_direct_crypto_dependency_are_removed(self) -> None:
        self.assertFalse((ROOT / "bindings/dart/tool/verify_package.dart").exists())
        pubspec = (ROOT / "bindings/dart/pubspec.yaml").read_text(encoding="utf-8")
        self.assertNotRegex(pubspec, r"(?m)^  crypto:")


class RegistryVersionCheckTests(unittest.TestCase):
    def test_http_results_have_explicit_tri_state_classification(self) -> None:
        cases = (
            (0, "200", check_registry_version.RegistryStatus.EXISTS),
            (0, "204", check_registry_version.RegistryStatus.EXISTS),
            (0, "299", check_registry_version.RegistryStatus.EXISTS),
            (0, "404", check_registry_version.RegistryStatus.MISSING),
            (0, "401", check_registry_version.RegistryStatus.ERROR),
            (0, "403", check_registry_version.RegistryStatus.ERROR),
            (0, "500", check_registry_version.RegistryStatus.ERROR),
            (7, "000", check_registry_version.RegistryStatus.ERROR),
            (0, "", check_registry_version.RegistryStatus.ERROR),
        )
        for returncode, http_status, expected in cases:
            with self.subTest(returncode=returncode, http_status=http_status):
                self.assertIs(
                    check_registry_version.classify_response(returncode, http_status),
                    expected,
                )

    def test_probe_uses_registry_specific_deadlines_and_urls(self) -> None:
        calls: list[tuple[tuple[object, ...], dict[str, object]]] = []

        def runner(*args: object, **kwargs: object) -> subprocess.CompletedProcess[str]:
            calls.append((args, kwargs))
            command = args[0]
            assert isinstance(command, tuple)
            return subprocess.CompletedProcess(command, 0, "404", "")

        status = check_registry_version.check_registry_version(
            "pub.dev",
            "mdstream_flutter",
            "0.4.0",
            runner=runner,
        )

        self.assertIs(status, check_registry_version.RegistryStatus.MISSING)
        self.assertEqual(len(calls), 1)
        command = calls[0][0][0]
        self.assertIsInstance(command, tuple)
        assert isinstance(command, tuple)
        self.assertIn("--connect-timeout", command)
        self.assertIn("5", command)
        self.assertIn("--max-time", command)
        self.assertIn("20", command)
        self.assertIn(
            "https://pub.dev/api/packages/mdstream_flutter/versions/0.4.0",
            command,
        )

    def test_probe_refuses_to_publish_on_transport_or_authorization_errors(self) -> None:
        def transport_error(
            *_args: object,
            **_kwargs: object,
        ) -> subprocess.CompletedProcess[str]:
            return subprocess.CompletedProcess(("curl",), 28, "000", "timed out")

        def forbidden(
            *_args: object,
            **_kwargs: object,
        ) -> subprocess.CompletedProcess[str]:
            return subprocess.CompletedProcess(("curl",), 0, "403", "")

        with patch("sys.stderr", io.StringIO()):
            transport_status = check_registry_version.check_registry_version(
                "crates.io",
                "mdstream",
                "0.4.0",
                runner=transport_error,
            )
            forbidden_status = check_registry_version.check_registry_version(
                "npm",
                "@mdstream/core",
                "0.4.0",
                runner=forbidden,
            )
        self.assertIs(transport_status, check_registry_version.RegistryStatus.ERROR)
        self.assertIs(forbidden_status, check_registry_version.RegistryStatus.ERROR)


def package(
    *dependencies: verify_packages.RustDependency,
) -> verify_packages.RustPackage:
    return verify_packages.RustPackage(
        version="0.4.0",
        dependencies=dependencies,
    )


def dependency(
    name: str,
    *,
    kind: str | None = None,
    requirement: str = "0.4.0",
) -> verify_packages.RustDependency:
    return verify_packages.RustDependency(
        name=name,
        kind=kind,
        requirement=requirement,
        source=None,
    )


def tar_member(name: str, payload: bytes) -> tuple[tarfile.TarInfo, bytes]:
    member = tarfile.TarInfo(name)
    member.size = len(payload)
    return member, payload


def tar_directory(name: str) -> tuple[tarfile.TarInfo, bytes]:
    member = tarfile.TarInfo(name)
    member.type = tarfile.DIRTYPE
    return member, b""


def tar_link(name: str, target: str, kind: bytes) -> tuple[tarfile.TarInfo, bytes]:
    member = tarfile.TarInfo(name)
    member.type = kind
    member.linkname = target
    return member, b""


def write_tar(
    path: Path,
    members: list[tuple[tarfile.TarInfo, bytes]],
) -> None:
    with tarfile.open(path, "w:gz") as archive:
        for member, payload in members:
            archive.addfile(member, io.BytesIO(payload) if member.isfile() else None)


def write_files_tar(
    path: Path,
    files: dict[str, bytes],
    *,
    prefix: str | None = None,
) -> None:
    members = []
    for name, payload in files.items():
        archive_name = f"{prefix}/{name}" if prefix is not None else name
        members.append(tar_member(archive_name, payload))
    write_tar(path, members)


def write_binding_policy(
    root: Path,
    *,
    npm_ceiling: int,
    dart_ceiling: int,
) -> None:
    bindings = root / "bindings"
    bindings.mkdir(parents=True, exist_ok=True)
    (bindings / "budgets.json").write_text(
        json.dumps(
            {
                "policy": {
                    "forbidden_default_dependencies": [
                        "merman",
                        "react",
                        "streamdown",
                        "incremark",
                    ]
                },
                "artifacts": [
                    {"artifact": "npm_packed", "ceiling_bytes": npm_ceiling},
                    {"artifact": "dart_packed", "ceiling_bytes": dart_ceiling},
                ],
            }
        ),
        encoding="utf-8",
    )
    protocol = root / "mdstream-protocol"
    protocol.mkdir(exist_ok=True)
    (protocol / "Cargo.toml").write_text(
        '[package]\nname = "mdstream-protocol"\nversion = "0.4.0"\n',
        encoding="utf-8",
    )


def valid_npm_files(
    *,
    dependencies: dict[str, str] | None = None,
) -> dict[str, bytes]:
    files = {path: b"content" for path in verify_packages.NPM_REQUIRED_FILES}
    files["package.json"] = json.dumps(
        {
            "name": "@mdstream/core",
            "version": "0.4.0",
            "dependencies": dependencies or {},
        }
    ).encode()
    files["wasm/mdstream_wasm_bg.wasm"] = b"\x00asm\x01\x00\x00\x00"
    return files


def valid_dart_files(*, extra_dependency: str | None = None) -> dict[str, bytes]:
    files = {path: b"content" for path in verify_packages.DART_REQUIRED_FILES}
    files["CHANGELOG.md"] = b"# Changelog\n\n## 0.4.0\n\n- Changed.\n"
    dependencies = "dependencies:\n  ffi: ^2.1.4\n"
    if extra_dependency is not None:
        dependencies += f"  {extra_dependency}\n"
    files["pubspec.yaml"] = (
        f"name: mdstream\nversion: 0.4.0\n{dependencies}"
    ).encode()
    return files


def valid_flutter_files() -> dict[str, bytes]:
    files = {path: b"content" for path in verify_packages.FLUTTER_REQUIRED_FILES}
    files["CHANGELOG.md"] = b"# Changelog\n\n## 0.4.0\n\n- Changed.\n"
    files["pubspec.yaml"] = (
        "name: mdstream_flutter\n"
        "version: 0.4.0\n"
        "dependencies:\n"
        "  flutter:\n"
        "    sdk: flutter\n"
        "  mdstream: ^0.4.0\n"
    ).encode()
    files["android/src/main/jniLibs/x86_64/libmdstream_ffi.so"] = (
        b"\x7fELFpayload"
    )
    return files


def indented_block(text: str, marker: str) -> str:
    lines = text.splitlines()
    for index, line in enumerate(lines):
        if line.strip() != marker:
            continue
        marker_indent = len(line) - len(line.lstrip(" "))
        block: list[str] = []
        for child in lines[index + 1 :]:
            if not child.strip():
                block.append(child)
                continue
            child_indent = len(child) - len(child.lstrip(" "))
            if child_indent <= marker_indent:
                break
            block.append(child)
        return "\n".join(block)
    raise AssertionError(f"could not find {marker!r}")


if __name__ == "__main__":
    unittest.main()
