#!/usr/bin/env python3
"""Extract one non-empty release section from a Markdown changelog."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


class ReleaseNotesError(RuntimeError):
    """Raised when a changelog cannot provide the requested release notes."""


STABLE_VERSION = re.compile(r"\d+\.\d+\.\d+")


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
    parser.add_argument("--changelog", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        text = args.changelog.read_text(encoding="utf-8")
        notes = extract_release_notes(text, args.version)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(f"{notes}\n", encoding="utf-8")
    except (OSError, ReleaseNotesError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
