"""Canonical Flutter package metadata derived from pubspec.yaml."""

from __future__ import annotations

import re
from pathlib import Path


PLUGIN_ROOT = Path(__file__).resolve().parents[1]
REPOSITORY_ROOT = Path(__file__).resolve().parents[3]
PUBSPEC_PATH = PLUGIN_ROOT / "pubspec.yaml"


class PackageMetadataError(RuntimeError):
    """Raised when Flutter package metadata is missing or invalid."""


def package_version(pubspec_path: Path = PUBSPEC_PATH) -> str:
    try:
        manifest = pubspec_path.read_text(encoding="utf-8")
    except OSError as error:
        raise PackageMetadataError(f"failed to read {pubspec_path}: {error}") from error
    match = re.search(
        r"(?m)^version:\s*['\"]?(\d+\.\d+\.\d+)['\"]?\s*(?:#.*)?$",
        manifest,
    )
    if match is None:
        raise PackageMetadataError(
            f"{pubspec_path} must declare a stable semantic version"
        )
    return match.group(1)


def package_archive_path(
    repository_root: Path = REPOSITORY_ROOT,
    pubspec_path: Path = PUBSPEC_PATH,
) -> Path:
    version = package_version(pubspec_path)
    return (
        repository_root
        / "target"
        / "flutter-package"
        / f"mdstream_flutter-{version}.tar.gz"
    )
