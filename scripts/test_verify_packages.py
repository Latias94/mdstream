#!/usr/bin/env python3
"""Contract tests for multi-ecosystem package verification."""

from __future__ import annotations

import importlib.util
import io
import json
import shutil
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]
MODULE_PATH = Path(__file__).with_name("verify-packages.py")
WORKFLOW_ROOT = ROOT / ".github" / "workflows"
SPEC = importlib.util.spec_from_file_location("verify_packages", MODULE_PATH)
assert SPEC is not None
verify_packages = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = verify_packages
SPEC.loader.exec_module(verify_packages)


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
        }
        for name, members in cases.items():
            with self.subTest(member=name), tempfile.TemporaryDirectory() as temporary:
                archive = Path(temporary) / "package.tar.gz"
                write_tar(archive, members)
                with self.assertRaisesRegex(
                    verify_packages.ValidationError,
                    "unsafe|link|duplicate",
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

        def comment_out_web_test(text: str) -> str:
            return text.replace("run: pnpm -r test", "# run: pnpm -r test", 1)

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

        cases = (
            ("commented command", "ci.yml", comment_out_web_test),
            ("wrong job", "ci.yml", move_web_test_to_quality),
            ("late wasm-tools install", "ci.yml", move_wasm_tools_after_build),
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

    def test_crates_io_wait_uses_the_cargo_registry_view(self) -> None:
        release = (WORKFLOW_ROOT / "release.yml").read_text(encoding="utf-8")
        publish = indented_block(release, "publish-rust:")

        self.assertIn(
            'timeout 30s cargo info --registry crates-io "$crate@$VERSION"',
            publish,
        )

    def test_trusted_publish_jobs_only_publish_verified_artifacts(self) -> None:
        release = (WORKFLOW_ROOT / "release.yml").read_text(encoding="utf-8")
        expected_needs = {
            "build-npm:": "needs: validate",
            "build-dart:": "needs: validate",
            "publish-npm:": "needs: [validate, quality, build-npm]",
            "publish-dart:": "needs: [validate, quality, build-dart]",
            "publish-flutter:": (
                "needs: [validate, quality, publish-dart, flutter-platforms]"
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

    def test_dart_ci_requires_native_for_the_complete_suite(self) -> None:
        workflow = (WORKFLOW_ROOT / "ci.yml").read_text(encoding="utf-8")
        job = indented_block(workflow, "dart:")
        native_build = job.index("dart run tool/build_native.dart")
        required = job.index('MDSTREAM_REQUIRE_NATIVE: "1"')
        full_suite = job.index("run: dart test")
        self.assertLess(native_build, required)
        self.assertLess(required, full_suite)
        self.assertEqual(job.count("run: dart test"), 1)

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

    def test_every_pub_dev_request_has_connection_and_total_timeouts(self) -> None:
        release = (WORKFLOW_ROOT / "release.yml").read_text(encoding="utf-8")
        requests = [
            line.strip()
            for line in release.splitlines()
            if "curl " in line and "https://pub.dev/" in line
        ]
        self.assertEqual(len(requests), 3)
        for request in requests:
            with self.subTest(request=request):
                self.assertIn("--connect-timeout", request)
                self.assertIn("--max-time", request)

    def test_dead_dart_package_verifier_and_direct_crypto_dependency_are_removed(self) -> None:
        self.assertFalse((ROOT / "bindings/dart/tool/verify_package.dart").exists())
        pubspec = (ROOT / "bindings/dart/pubspec.yaml").read_text(encoding="utf-8")
        self.assertNotRegex(pubspec, r"(?m)^  crypto:")


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
    dependencies = "dependencies:\n  ffi: ^2.1.4\n"
    if extra_dependency is not None:
        dependencies += f"  {extra_dependency}\n"
    files["pubspec.yaml"] = (
        f"name: mdstream\nversion: 0.4.0\n{dependencies}"
    ).encode()
    return files


def valid_flutter_files() -> dict[str, bytes]:
    files = {path: b"content" for path in verify_packages.FLUTTER_REQUIRED_FILES}
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
