#!/usr/bin/env python3
"""Extract one version's curated release notes from CHANGELOG.md."""

from __future__ import annotations

import re
import sys
from pathlib import Path


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: extract-release-notes.py <version-or-tag> <output>", file=sys.stderr)
        return 2

    version = sys.argv[1].removeprefix("v")
    output = Path(sys.argv[2])
    changelog = Path("CHANGELOG.md").read_text(encoding="utf-8")
    pattern = re.compile(
        rf"^## \[{re.escape(version)}\][^\n]*\n(?P<body>.*?)(?=^## \[|\Z)",
        re.MULTILINE | re.DOTALL,
    )
    match = pattern.search(changelog)
    if match is None:
        print(f"release notes for {version} not found in CHANGELOG.md", file=sys.stderr)
        return 1

    body = match.group("body").strip()
    if not body:
        print(f"release notes for {version} are empty", file=sys.stderr)
        return 1

    output.write_text(body + "\n", encoding="utf-8")
    print(f"wrote {output} ({len(body)} characters)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
