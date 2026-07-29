"""Shared safe-tar policy for release artifact inspection."""

from __future__ import annotations

import os
import re
import shutil
import tarfile
import tempfile
import unicodedata
import zlib
from collections.abc import Callable, Iterator
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import BinaryIO, Mapping


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
_GZIP_MAGIC = b"\x1f\x8b"
_TAR_END_OF_ARCHIVE_BYTES = 2 * tarfile.BLOCKSIZE
_WINDOWS_RESERVED_BASENAMES = frozenset(
    {"con", "prn", "aux", "nul"}
    | {f"com{index}" for index in range(1, 10)}
    | {f"lpt{index}" for index in range(1, 10)}
)
_WINDOWS_INVALID_CHARACTERS = re.compile(r'[<>:"|?*]|[\x00-\x1f]')


def _decompress_single_gzip(
    source: BinaryIO,
    destination: BinaryIO,
    max_bytes: int,
) -> int:
    decompressor = zlib.decompressobj(16 + zlib.MAX_WBITS)
    decompressed_bytes = 0

    while chunk := source.read(_ARCHIVE_CHUNK_BYTES):
        remaining = max_bytes - decompressed_bytes
        data = decompressor.decompress(chunk, remaining + 1)
        decompressed_bytes += len(data)
        if decompressed_bytes > max_bytes:
            raise ArchivePolicyError(
                "archive decompressed stream exceeds ceiling: "
                f"{decompressed_bytes} > {max_bytes}"
            )
        destination.write(data)
        if decompressor.eof:
            trailing = bytearray(decompressor.unused_data)
            while len(trailing) < len(_GZIP_MAGIC):
                extra = source.read(len(_GZIP_MAGIC) - len(trailing))
                if not extra:
                    break
                trailing.extend(extra)
            if trailing:
                if trailing.startswith(_GZIP_MAGIC):
                    raise ArchivePolicyError(
                        "archive contains multiple gzip members"
                    )
                raise ArchivePolicyError(
                    "archive contains trailing data after gzip member"
                )
            break

    if not decompressor.eof:
        raise ArchivePolicyError("archive contains an incomplete gzip member")
    return decompressed_bytes


def _validate_tar_end_of_archive(
    stream: BinaryIO,
    end_offset: int,
    stream_bytes: int,
) -> None:
    trailing_bytes = stream_bytes - end_offset
    if trailing_bytes < _TAR_END_OF_ARCHIVE_BYTES:
        raise ArchivePolicyError(
            "archive is missing the tar end-of-archive marker"
        )
    stream.seek(end_offset)
    while chunk := stream.read(_ARCHIVE_CHUNK_BYTES):
        if any(chunk):
            raise ArchivePolicyError(
                "archive contains non-zero data after tar end-of-archive marker"
            )


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
    portable_names: dict[tuple[str, ...], str] = {}
    portable_prefix_spellings: dict[tuple[str, ...], tuple[str, ...]] = {}
    portable_file_names: set[tuple[str, ...]] = set()
    portable_implicit_directories: set[tuple[str, ...]] = set()
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
        with path.open("rb") as source, tempfile.TemporaryFile() as decompressed:
            stream_bytes = _decompress_single_gzip(
                source,
                decompressed,
                stream_ceiling,
            )
            decompressed.seek(0)
            with tarfile.open(fileobj=decompressed, mode="r|") as archive:
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
                    portable = _portable_member_key(normalized)
                    spelling = PurePosixPath(normalized).parts
                    for depth in range(1, len(portable) + 1):
                        prefix = portable[:depth]
                        prefix_spelling = spelling[:depth]
                        previous_spelling = portable_prefix_spellings.get(prefix)
                        if (
                            previous_spelling is not None
                            and previous_spelling != prefix_spelling
                        ):
                            raise ArchivePolicyError(
                                "archive contains non-portable path collision: "
                                f"{'/'.join(previous_spelling)} and "
                                f"{'/'.join(prefix_spelling)}"
                            )
                        portable_prefix_spellings[prefix] = prefix_spelling
                    collided = portable_names.get(portable)
                    if collided is not None:
                        raise ArchivePolicyError(
                            "archive contains non-portable path collision: "
                            f"{collided} and {normalized}"
                        )
                    for depth in range(1, len(portable)):
                        ancestor = portable[:depth]
                        if ancestor in portable_file_names:
                            raise ArchivePolicyError(
                                "archive contains file/subtree path conflict: "
                                f"{portable_names[ancestor]} and {normalized}"
                            )
                    if member.isfile() and portable in portable_implicit_directories:
                        descendant = next(
                            name
                            for key, name in portable_names.items()
                            if len(key) > len(portable) and key[: len(portable)] == portable
                        )
                        raise ArchivePolicyError(
                            "archive contains file/subtree path conflict: "
                            f"{normalized} and {descendant}"
                        )
                    portable_names[portable] = normalized
                    portable_implicit_directories.update(
                        portable[:depth] for depth in range(1, len(portable))
                    )
                    if member.isfile():
                        portable_file_names.add(portable)
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
                tar_end_offset = archive.offset
            _validate_tar_end_of_archive(
                decompressed,
                tar_end_offset,
                stream_bytes,
            )
    except (OSError, EOFError, tarfile.TarError, zlib.error) as error:
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


def extract_archive(
    path: Path,
    destination: Path,
    limits: ArchiveLimits | Mapping[str, object] | None = None,
) -> None:
    """Validate and atomically extract an archive into a new directory."""

    if destination.exists() or destination.is_symlink():
        raise ArchivePolicyError(
            f"archive extraction destination already exists: {destination}"
        )
    destination.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(
        tempfile.mkdtemp(
            prefix=f".{destination.name}.extract-",
            dir=destination.parent,
        )
    )

    def extract(member: ArchiveMember, chunks: Iterator[bytes]) -> None:
        target = extraction_path(staging, member.name)
        if not member.is_file:
            target.mkdir(parents=True, exist_ok=True)
            return
        target.parent.mkdir(parents=True, exist_ok=True)
        try:
            with target.open("xb") as handle:
                for chunk in chunks:
                    handle.write(chunk)
        except FileExistsError as error:
            raise ArchivePolicyError(
                f"archive extraction target already exists: {member.name}"
            ) from error

    try:
        visit_archive(path, extract, limits)
        os.replace(staging, destination)
    except BaseException:
        shutil.rmtree(staging, ignore_errors=True)
        raise


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


def _portable_member_key(name: str) -> tuple[str, ...]:
    key: list[str] = []
    for segment in PurePosixPath(name).parts:
        reserved_basename = unicodedata.normalize(
            "NFKC", segment.split(".", 1)[0]
        ).casefold()
        if (
            segment.endswith((" ", "."))
            or _WINDOWS_INVALID_CHARACTERS.search(segment) is not None
            or any("\ud800" <= character <= "\udfff" for character in segment)
            or reserved_basename in _WINDOWS_RESERVED_BASENAMES
        ):
            raise ArchivePolicyError(
                f"archive contains non-portable path segment {segment!r} in {name}"
            )
        key.append(unicodedata.normalize("NFC", segment).casefold())
    return tuple(key)
