#!/usr/bin/env python3
"""Reflow markdown by joining hard-wrapped paragraphs into single lines.

Preserves: fenced code blocks, headings, blank lines, horizontal rules, table
rows, and list-item structure. Joins continuation lines within paragraphs and
within list items (so `- foo\n  bar` -> `- foo bar`).

Usage: scripts/reflow_markdown.py FILE [FILE ...]  (rewrites in place)
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

LIST_RE = re.compile(r"^(\s*)([-*+]|\d+\.)\s+")
HEADING_RE = re.compile(r"^#{1,6}\s")
HR_RE = re.compile(r"^---+\s*$")
TABLE_RE = re.compile(r"^\s*\|")
FENCE_RE = re.compile(r"^\s*```")


def reflow(text: str) -> str:
    lines = text.splitlines()
    out: list[str] = []
    buf: list[str] = []  # current paragraph being accumulated
    in_code = False

    def flush() -> None:
        nonlocal buf
        if buf:
            out.append(" ".join(buf))
            buf = []

    for raw in lines:
        if FENCE_RE.match(raw):
            flush()
            out.append(raw)
            in_code = not in_code
            continue

        if in_code:
            out.append(raw)
            continue

        # Blank line ends any paragraph / list item.
        if not raw.strip():
            flush()
            out.append("")
            continue

        # Headings, HR, and table rows are emitted untouched and end the buffer.
        if HEADING_RE.match(raw) or HR_RE.match(raw) or TABLE_RE.match(raw):
            flush()
            out.append(raw)
            continue

        if LIST_RE.match(raw):
            # New list item: flush the prior paragraph/item, start a new buffer.
            flush()
            buf.append(raw.rstrip())
            continue

        # Continuation of either a list item or a plain paragraph.
        stripped = raw.strip()
        if buf:
            buf.append(stripped)
        else:
            buf.append(raw.rstrip())

    flush()
    return "\n".join(out) + ("\n" if text.endswith("\n") else "")


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    for path in argv[1:]:
        p = Path(path)
        original = p.read_text()
        rewritten = reflow(original)
        if rewritten != original:
            p.write_text(rewritten)
            print(f"reflowed: {path}")
        else:
            print(f"unchanged: {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
