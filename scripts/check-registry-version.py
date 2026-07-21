#!/usr/bin/env python3
"""Classify whether an exact package version exists in a release registry."""

from __future__ import annotations

import argparse
import subprocess
import sys
from dataclasses import dataclass
from enum import IntEnum
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


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Check whether an exact package version exists in a registry"
    )
    parser.add_argument("registry", choices=tuple(REGISTRIES))
    parser.add_argument("package")
    parser.add_argument("version")
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    status = check_registry_version(args.registry, args.package, args.version)
    if status is RegistryStatus.EXISTS:
        print(f"{args.package} {args.version} exists on {args.registry}")
    elif status is RegistryStatus.MISSING:
        print(f"{args.package} {args.version} is missing from {args.registry}")
    return int(status)


if __name__ == "__main__":
    raise SystemExit(main())
