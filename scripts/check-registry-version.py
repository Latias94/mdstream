#!/usr/bin/env python3
"""Classify whether an exact package version exists in a release registry."""

from __future__ import annotations

import argparse
import importlib.util
import json
import subprocess
import sys
from dataclasses import dataclass
from enum import IntEnum
from functools import cache
from pathlib import Path
from typing import Callable, Sequence
from urllib.parse import quote


class RegistryStatus(IntEnum):
    EXISTS = 0
    MISSING = 1
    ERROR = 2


@dataclass(frozen=True)
class RegistryEndpoint:
    base_url: str
    package_safe: str
    connect_timeout: int
    max_time: int


@dataclass(frozen=True)
class RegistryTarget:
    registry: str
    package: str
    version: str


@dataclass(frozen=True)
class TagEvidence:
    tag: str
    local: RegistryStatus
    remote: RegistryStatus


@dataclass(frozen=True)
class WorkspaceAuditReport:
    version: str
    workspace_verified: bool
    packages: tuple[tuple[RegistryTarget, RegistryStatus], ...]
    tags: tuple[TagEvidence, ...]

    @property
    def indeterminate(self) -> bool:
        return not self.workspace_verified or any(
            status is RegistryStatus.ERROR
            for _target, status in self.packages
        ) or any(
            evidence.remote is RegistryStatus.ERROR
            for evidence in self.tags
        )

    def as_json(self) -> dict[str, object]:
        return {
            "schema": "mdstream.release-version-audit/1",
            "version": self.version,
            "workspace": "verified" if self.workspace_verified else "indeterminate",
            "packages": [
                {
                    "registry": target.registry,
                    "package": target.package,
                    "version": target.version,
                    "status": _audit_status_label(status),
                }
                for target, status in self.packages
            ],
            "tags": [
                {
                    "tag": evidence.tag,
                    "local": _audit_status_label(evidence.local),
                    "remote": _audit_status_label(evidence.remote),
                }
                for evidence in self.tags
            ],
        }


REGISTRIES = {
    "crates.io": RegistryEndpoint(
        base_url="https://crates.io/api/v1/crates",
        package_safe="",
        connect_timeout=30,
        max_time=30,
    ),
    "npm": RegistryEndpoint(
        base_url="https://registry.npmjs.org",
        package_safe="@",
        connect_timeout=30,
        max_time=30,
    ),
    "pub.dev": RegistryEndpoint(
        base_url="https://pub.dev/api/packages",
        package_safe="",
        connect_timeout=5,
        max_time=20,
    ),
}


def registry_version_url(registry: str, package: str, version: str) -> str:
    endpoint = REGISTRIES[registry]
    encoded_package = quote(package, safe=endpoint.package_safe)
    encoded_version = quote(version, safe="")
    if registry == "pub.dev":
        return f"{endpoint.base_url}/{encoded_package}/versions/{encoded_version}"
    return f"{endpoint.base_url}/{encoded_package}/{encoded_version}"


def classify_response(returncode: int, http_status: str) -> RegistryStatus:
    if returncode != 0:
        return RegistryStatus.ERROR
    try:
        status = int(http_status)
    except ValueError:
        return RegistryStatus.ERROR
    if 200 <= status < 300:
        return RegistryStatus.EXISTS
    if status == 404:
        return RegistryStatus.MISSING
    return RegistryStatus.ERROR


def check_registry_version(
    registry: str,
    package: str,
    version: str,
    *,
    runner: Callable[..., subprocess.CompletedProcess[str]] = subprocess.run,
) -> RegistryStatus:
    endpoint = REGISTRIES[registry]
    url = registry_version_url(registry, package, version)
    command = (
        "curl",
        "--silent",
        "--show-error",
        "--output",
        "/dev/null",
        "--write-out",
        "%{http_code}",
        "--user-agent",
        "mdstream-release-workflow/1 (+https://github.com/Latias94/mdstream)",
        "--connect-timeout",
        str(endpoint.connect_timeout),
        "--max-time",
        str(endpoint.max_time),
        url,
    )
    try:
        result = runner(command, capture_output=True, text=True, check=False)
    except OSError as error:
        print(f"registry probe failed to start: {error}", file=sys.stderr)
        return RegistryStatus.ERROR

    status = classify_response(result.returncode, result.stdout.strip())
    if status is RegistryStatus.ERROR:
        detail = result.stderr.strip() or f"HTTP status {result.stdout.strip()!r}"
        print(f"registry probe failed for {url}: {detail}", file=sys.stderr)
    return status


@cache
def _release_contract_module() -> object:
    path = Path(__file__).with_name("verify-packages.py")
    spec = importlib.util.spec_from_file_location(
        "mdstream_release_package_contract",
        path,
    )
    if spec is None or spec.loader is None:
        raise RuntimeError(f"could not load package contract from {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def workspace_registry_targets(root: Path, version: str) -> tuple[RegistryTarget, ...]:
    """Inventory exact release targets from the repository's package contract."""

    contract = _release_contract_module()
    try:
        release = contract.validate_static_contract(root)
        if release.version != version:
            raise RuntimeError(
                f"workspace version is {release.version}, expected {version}"
            )
        rust_packages = contract.load_rust_packages(root)
        targets = [
            RegistryTarget("crates.io", name, rust_packages[name].version)
            for name in contract.RUST_PUBLISH_ORDER
        ]
        typescript = json.loads(
            (root / "bindings" / "typescript" / "package.json").read_text(
                encoding="utf-8"
            )
        )
        targets.extend((
            RegistryTarget("npm", "@mdstream/core", str(typescript["version"])),
            RegistryTarget(
                "pub.dev",
                contract.top_level_yaml_scalar(
                    root / "bindings" / "dart" / "pubspec.yaml",
                    "name",
                ),
                contract.top_level_yaml_scalar(
                    root / "bindings" / "dart" / "pubspec.yaml",
                    "version",
                ),
            ),
            RegistryTarget(
                "pub.dev",
                contract.top_level_yaml_scalar(
                    root / "bindings" / "flutter" / "pubspec.yaml",
                    "name",
                ),
                contract.top_level_yaml_scalar(
                    root / "bindings" / "flutter" / "pubspec.yaml",
                    "version",
                ),
            ),
        ))
    except Exception as error:
        raise RuntimeError(f"workspace package contract is indeterminate: {error}") from error
    return tuple(targets)


def check_local_tag(
    root: Path,
    tag: str,
    *,
    runner: Callable[..., subprocess.CompletedProcess[str]] = subprocess.run,
) -> RegistryStatus:
    command = ("git", "show-ref", "--verify", "--quiet", f"refs/tags/{tag}")
    try:
        result = runner(
            command,
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
            timeout=30,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        print(f"local tag probe failed for {tag}: {error}", file=sys.stderr)
        return RegistryStatus.ERROR
    if result.returncode == 0:
        return RegistryStatus.EXISTS
    if result.returncode == 1:
        return RegistryStatus.MISSING
    print(f"local tag probe failed for {tag}", file=sys.stderr)
    return RegistryStatus.ERROR


def check_remote_tags(
    remote: str,
    tags: Sequence[str],
    *,
    root: Path,
    runner: Callable[..., subprocess.CompletedProcess[str]] = subprocess.run,
) -> dict[str, RegistryStatus]:
    refs = tuple(f"refs/tags/{tag}" for tag in tags)
    command = ("git", "ls-remote", "--tags", "--refs", remote, *refs)
    try:
        result = runner(
            command,
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
            timeout=30,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        print(f"remote tag probe failed for {remote}: {error}", file=sys.stderr)
        return {tag: RegistryStatus.ERROR for tag in tags}
    if result.returncode != 0:
        print(f"remote tag probe failed for {remote}", file=sys.stderr)
        return {tag: RegistryStatus.ERROR for tag in tags}
    present = {
        line.split("\t", 1)[1].removeprefix("refs/tags/")
        for line in result.stdout.splitlines()
        if "\trefs/tags/" in line
    }
    return {
        tag: RegistryStatus.EXISTS if tag in present else RegistryStatus.MISSING
        for tag in tags
    }


def audit_workspace(
    version: str,
    *,
    root: Path,
    remote: str,
    registry_runner: Callable[..., subprocess.CompletedProcess[str]] = subprocess.run,
    git_runner: Callable[..., subprocess.CompletedProcess[str]] = subprocess.run,
) -> WorkspaceAuditReport:
    """Collect every release target and tag result without short-circuiting probes."""

    try:
        targets = workspace_registry_targets(root, version)
        workspace_verified = True
    except RuntimeError as error:
        print(error, file=sys.stderr)
        targets = ()
        workspace_verified = False

    packages = tuple(
        (
            target,
            check_registry_version(
                target.registry,
                target.package,
                target.version,
                runner=registry_runner,
            ),
        )
        for target in targets
    )
    tags = (version, f"v{version}")
    remote_statuses = check_remote_tags(
        remote,
        tags,
        root=root,
        runner=git_runner,
    )
    return WorkspaceAuditReport(
        version=version,
        workspace_verified=workspace_verified,
        packages=packages,
        tags=tuple(
            TagEvidence(
                tag=tag,
                local=check_local_tag(root, tag, runner=git_runner),
                remote=remote_statuses[tag],
            )
            for tag in tags
        ),
    )


def _audit_status_label(status: RegistryStatus) -> str:
    if status is RegistryStatus.EXISTS:
        return "present"
    if status is RegistryStatus.MISSING:
        return "missing"
    return "indeterminate"


def parse_audit_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Collect exact package and tag evidence before a release freeze"
    )
    parser.add_argument("version")
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--remote", required=True)
    return parser.parse_args(argv)


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Check whether an exact package version exists in a registry"
    )
    parser.add_argument("registry", choices=tuple(REGISTRIES))
    parser.add_argument("package")
    parser.add_argument("version")
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    arguments = tuple(sys.argv[1:] if argv is None else argv)
    if arguments and arguments[0] == "audit-workspace":
        args = parse_audit_args(arguments[1:])
        report = audit_workspace(
            args.version,
            root=args.root.resolve(),
            remote=args.remote,
        )
        print(json.dumps(report.as_json(), indent=2, sort_keys=True))
        return int(RegistryStatus.ERROR if report.indeterminate else RegistryStatus.EXISTS)

    args = parse_args(arguments)
    status = check_registry_version(args.registry, args.package, args.version)
    if status is RegistryStatus.EXISTS:
        print(f"{args.package} {args.version} exists on {args.registry}")
    elif status is RegistryStatus.MISSING:
        print(f"{args.package} {args.version} is missing from {args.registry}")
    return int(status)


if __name__ == "__main__":
    raise SystemExit(main())
