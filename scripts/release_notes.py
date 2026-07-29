#!/usr/bin/env python3
"""Render one portable GitHub release section from a Markdown changelog."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path, PurePosixPath
from urllib.parse import quote, unquote, urlsplit


class ReleaseNotesError(RuntimeError):
    """Raised when a changelog cannot provide the requested release notes."""


STABLE_VERSION = re.compile(r"\d+\.\d+\.\d+")
GITHUB_REPOSITORY = re.compile(
    r"[A-Za-z0-9](?:[A-Za-z0-9_.-]*[A-Za-z0-9])?/"
    r"[A-Za-z0-9](?:[A-Za-z0-9_.-]*[A-Za-z0-9])?"
)
MARKDOWN_INLINE_LINK = re.compile(
    r"(?P<prefix>!?\[[^\]\n]*\]\(\s*)"
    r"(?P<destination><[^>\n]+>|[^()\s]+)"
    r"(?P<suffix>(?:\s+(?:\"[^\"\n]*\"|'[^'\n]*'|\([^\)\n]*\)))?\s*\))"
)


def release_heading_pattern(version: str) -> re.Pattern[str]:
    if STABLE_VERSION.fullmatch(version) is None:
        raise ReleaseNotesError(f"invalid stable release version: {version}")
    return re.compile(
        rf"^##\s+(?:{re.escape(version)}|\[{re.escape(version)}\])"
        rf"(?:\s+-\s+\d{{4}}-\d{{2}}-\d{{2}})?\s*$",
        flags=re.MULTILINE,
    )


def extract_release_notes(text: str, version: str) -> str:
    heading = release_heading_pattern(version)
    matches = tuple(heading.finditer(text))
    if len(matches) != 1:
        raise ReleaseNotesError(
            f"expected exactly one changelog section for {version}, found {len(matches)}"
        )
    match = matches[0]
    next_heading = re.search(r"^##\s+", text[match.end() :], flags=re.MULTILINE)
    end = match.end() + next_heading.start() if next_heading else len(text)
    notes = text[match.end() : end].strip()
    if not notes:
        raise ReleaseNotesError(f"empty changelog section for {version}")
    return notes


def _repository_relative_destination(destination: str) -> tuple[str, str, str] | None:
    parsed = urlsplit(destination)
    if (
        parsed.scheme
        or parsed.netloc
        or destination.startswith(("#", "/", "?"))
        or not parsed.path
    ):
        return None

    decoded = unquote(parsed.path).replace("\\", "/")
    path = PurePosixPath(decoded)
    if ".." in path.parts:
        raise ReleaseNotesError(
            f"repository-relative release link escapes the repository: {destination}"
        )
    normalized = path.as_posix()
    while normalized.startswith("./"):
        normalized = normalized[2:]
    if normalized in ("", "."):
        return None
    return normalized, parsed.query, parsed.fragment


def rewrite_repository_links(notes: str, version: str, repository: str) -> str:
    """Bind repository-relative Markdown links to an immutable release tag."""

    release_heading_pattern(version)
    if GITHUB_REPOSITORY.fullmatch(repository) is None:
        raise ReleaseNotesError(
            f"invalid GitHub repository; expected owner/name: {repository}"
        )
    ref = f"v{version}"

    def replace(match: re.Match[str]) -> str:
        raw_destination = match.group("destination")
        bracketed = raw_destination.startswith("<") and raw_destination.endswith(">")
        destination = raw_destination[1:-1] if bracketed else raw_destination
        relative = _repository_relative_destination(destination)
        if relative is None:
            return match.group(0)
        path, query, fragment = relative
        encoded_path = quote(path, safe="/@:+,;=-._~!$&'*")
        if match.group("prefix").startswith("!"):
            target = (
                f"https://raw.githubusercontent.com/{repository}/{ref}/{encoded_path}"
            )
        else:
            target = f"https://github.com/{repository}/blob/{ref}/{encoded_path}"
        if query:
            target = f"{target}?{query}"
        if fragment:
            target = f"{target}#{fragment}"
        if bracketed:
            target = f"<{target}>"
        return f"{match.group('prefix')}{target}{match.group('suffix')}"

    return MARKDOWN_INLINE_LINK.sub(replace, notes)


def render_release_notes(text: str, version: str, repository: str) -> str:
    notes = extract_release_notes(text, version)
    return rewrite_repository_links(notes, version, repository)


def first_release_version(text: str) -> str:
    match = re.search(
        r"^##\s+(?:\[(\d+\.\d+\.\d+)\]|(\d+\.\d+\.\d+))"
        r"(?:\s+-\s+\d{4}-\d{2}-\d{2})?\s*$",
        text,
        flags=re.MULTILINE,
    )
    if match is None:
        raise ReleaseNotesError("changelog has no stable release heading")
    return next(value for value in match.groups() if value is not None)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument(
        "--repository",
        required=True,
        help="GitHub repository in owner/name form",
    )
    parser.add_argument("--changelog", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        text = args.changelog.read_text(encoding="utf-8")
        notes = render_release_notes(text, args.version, args.repository)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(f"{notes}\n", encoding="utf-8")
    except (OSError, ReleaseNotesError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
