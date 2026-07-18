"""Shared safe-tar policy for release artifact inspection."""

from __future__ import annotations

import gzip
import tarfile
from collections.abc import Callable, Iterator
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import Mapping


class ArchivePolicyError(RuntimeError):
    """Raised when an archive member violates the release extraction policy."""


@dataclass(frozen=True)
class ArchiveEntry:
    name: str
    data: bytes | None

    @property
    def is_file(self) -> bool:
        return self.data is not None


@dataclass(frozen=True)
class ArchiveMember:
    name: str
    size: int | None

    @property
    def is_file(self) -> bool:
        return self.size is not None


@dataclass(frozen=True)
class ArchiveLimits:
    max_compressed_bytes: int
    max_members: int
    max_member_bytes: int
    max_uncompressed_bytes: int

    @classmethod
    def from_value(
        cls,
        value: ArchiveLimits | Mapping[str, object] | None,
    ) -> ArchiveLimits:
        if value is None:
            return DEFAULT_ARCHIVE_LIMITS
        if isinstance(value, cls):
            return value
        try:
            limits = cls(
                max_compressed_bytes=int(value["max_compressed_bytes"]),
                max_members=int(value["max_members"]),
                max_member_bytes=int(value["max_member_bytes"]),
                max_uncompressed_bytes=int(value["max_uncompressed_bytes"]),
            )
        except (KeyError, TypeError, ValueError) as error:
            raise ArchivePolicyError(f"invalid archive limits: {error}") from error
        if any(limit <= 0 for limit in limits.as_tuple()):
            raise ArchivePolicyError("archive limits must be positive integers")
        return limits

    def as_tuple(self) -> tuple[int, int, int, int]:
        return (
            self.max_compressed_bytes,
            self.max_members,
            self.max_member_bytes,
            self.max_uncompressed_bytes,
        )


DEFAULT_ARCHIVE_LIMITS = ArchiveLimits(
    max_compressed_bytes=64 * 1024 * 1024,
    max_members=4096,
    max_member_bytes=8 * 1024 * 1024,
    max_uncompressed_bytes=96 * 1024 * 1024,
)

_ARCHIVE_CHUNK_BYTES = 64 * 1024


class _BoundedReader:
    def __init__(self, source: gzip.GzipFile, max_bytes: int) -> None:
        self._source = source
        self._max_bytes = max_bytes
        self._read_bytes = 0

    def read(self, size: int = -1) -> bytes:
        remaining = self._max_bytes - self._read_bytes
        request = remaining + 1 if size < 0 or size > remaining + 1 else size
        data = self._source.read(request)
        self._read_bytes += len(data)
        if self._read_bytes > self._max_bytes:
            raise ArchivePolicyError(
                "archive decompressed stream exceeds ceiling: "
                f"{self._read_bytes} > {self._max_bytes}"
            )
        return data


def read_archive(
    path: Path,
    limits: ArchiveLimits | Mapping[str, object] | None = None,
) -> tuple[ArchiveEntry, ...]:
    """Read regular files and directories after validating every tar member."""

    entries: list[ArchiveEntry] = []

    def collect(member: ArchiveMember, chunks: Iterator[bytes]) -> None:
        data = b"".join(chunks) if member.is_file else None
        entries.append(ArchiveEntry(member.name, data))

    visit_archive(path, collect, limits)
    return tuple(entries)


def visit_archive(
    path: Path,
    visitor: Callable[[ArchiveMember, Iterator[bytes]], None],
    limits: ArchiveLimits | Mapping[str, object] | None = None,
) -> None:
    """Validate an archive while streaming each member through ``visitor``."""

    limits = ArchiveLimits.from_value(limits)
    names: set[str] = set()
    member_count = 0
    declared_bytes = 0
    read_bytes = 0
    try:
        compressed_bytes = path.stat().st_size
        if compressed_bytes > limits.max_compressed_bytes:
            raise ArchivePolicyError(
                "archive compressed size exceeds ceiling: "
                f"{compressed_bytes} > {limits.max_compressed_bytes}"
            )
        stream_ceiling = (
            limits.max_uncompressed_bytes
            + limits.max_members * 2048
            + 10 * 1024
        )
        with path.open("rb") as source, gzip.GzipFile(fileobj=source) as decompressed:
            bounded = _BoundedReader(decompressed, stream_ceiling)
            with tarfile.open(fileobj=bounded, mode="r|") as archive:
                for member in archive:
                    if member_count >= limits.max_members:
                        raise ArchivePolicyError(
                            f"archive member count exceeds ceiling {limits.max_members}"
                        )
                    normalized = _normalized_member_name(member)
                    if normalized in names:
                        raise ArchivePolicyError(
                            f"archive contains duplicate member {normalized}"
                        )
                    names.add(normalized)
                    if member.issym() or member.islnk():
                        raise ArchivePolicyError(
                            f"archive contains unsupported link {normalized}"
                        )
                    if not member.isfile() and not member.isdir():
                        raise ArchivePolicyError(
                            f"archive contains unsupported member type {normalized}"
                        )
                    if member.isdir():
                        visitor(ArchiveMember(normalized, None), iter(()))
                        member_count += 1
                        continue
                    if member.size < 0 or member.size > limits.max_member_bytes:
                        raise ArchivePolicyError(
                            "archive member size exceeds ceiling: "
                            f"{normalized} ({member.size} > {limits.max_member_bytes})"
                        )
                    declared_bytes += member.size
                    if declared_bytes > limits.max_uncompressed_bytes:
                        raise ArchivePolicyError(
                            "archive declared uncompressed size exceeds ceiling: "
                            f"{declared_bytes} > {limits.max_uncompressed_bytes}"
                        )
                    handle = archive.extractfile(member)
                    if handle is None:
                        raise ArchivePolicyError(
                            f"failed to read archive member {normalized}"
                        )

                    def chunks() -> Iterator[bytes]:
                        nonlocal read_bytes
                        member_read = 0
                        while member_read < member.size:
                            chunk = handle.read(
                                min(_ARCHIVE_CHUNK_BYTES, member.size - member_read)
                            )
                            if not chunk:
                                break
                            member_read += len(chunk)
                            yield chunk
                        extra = handle.read(1)
                        member_read += len(extra)
                        if member_read != member.size:
                            raise ArchivePolicyError(
                                f"archive member size mismatch for {normalized}: "
                                f"declared {member.size}, read {member_read}"
                            )
                        read_bytes += member_read
                        if read_bytes > limits.max_uncompressed_bytes:
                            raise ArchivePolicyError(
                                "archive read uncompressed size exceeds ceiling: "
                                f"{read_bytes} > {limits.max_uncompressed_bytes}"
                            )

                    member_chunks = chunks()
                    visitor(
                        ArchiveMember(normalized, member.size),
                        member_chunks,
                    )
                    for _ in member_chunks:
                        pass
                    member_count += 1
    except (OSError, EOFError, gzip.BadGzipFile, tarfile.TarError) as error:
        raise ArchivePolicyError(f"failed to inspect archive {path}: {error}") from error


def extraction_path(destination: Path, member_name: str) -> Path:
    """Resolve a validated member path without following it outside the root."""

    root = destination.resolve()
    target = destination.joinpath(*PurePosixPath(member_name).parts)
    resolved = target.resolve()
    try:
        resolved.relative_to(root)
    except ValueError as error:
        raise ArchivePolicyError(
            f"archive extraction path escapes destination: {member_name}"
        ) from error
    return target


def _normalized_member_name(member: tarfile.TarInfo) -> str:
    raw = member.name
    pure = PurePosixPath(raw)
    normalized = str(pure)
    if (
        not raw
        or "\\" in raw
        or pure.is_absolute()
        or ".." in pure.parts
        or normalized in ("", ".")
        or (pure.parts and len(pure.parts[0]) == 2 and pure.parts[0][1] == ":")
    ):
        raise ArchivePolicyError(f"archive contains unsafe path {raw}")
    canonical_names = {normalized, f"{normalized}/"} if member.isdir() else {normalized}
    if raw not in canonical_names:
        raise ArchivePolicyError(f"archive contains non-canonical path {raw}")
    return normalized
