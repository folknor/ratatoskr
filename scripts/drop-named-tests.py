#!/usr/bin/env python3
"""Drop named tests (plus their preceding doc-comment and attributes) from
a Rust test file. Used to migrate libtest cohorts to the Lua harness without
hand-editing 300+ line chunks.

Usage: drop-named-tests.py <file> <test_fn_name> [<test_fn_name> ...]

A "test" is detected as `(async )?fn NAME(...)`. The script also removes:
- Preceding `#[...]` attribute lines (`#[tokio::test]`, `#[ignore]`, etc.)
- Preceding `///` doc-comment lines (until a non-doc, non-attribute line).
- The matching `}` that closes the fn body (brace-balanced).
- One trailing blank line if present (to avoid double-blank-line padding).
"""

import re
import sys
from pathlib import Path


def find_test_span(lines, name):
    fn_re = re.compile(rf"^(async\s+)?fn\s+{re.escape(name)}\s*\(")
    fn_start = None
    for i, line in enumerate(lines):
        if fn_re.match(line.lstrip()):
            fn_start = i
            break
    if fn_start is None:
        raise SystemExit(f"test {name!r} not found")

    # Walk upward through attributes and doc comments to find the real start.
    block_start = fn_start
    while block_start > 0:
        prev = lines[block_start - 1].rstrip()
        if prev.startswith("///") or prev.startswith("#["):
            block_start -= 1
        else:
            break

    # Find the matching `}` for the fn body by brace-counting from the
    # opening `{` on the fn line (or the next line containing one).
    open_idx = fn_start
    while "{" not in lines[open_idx]:
        open_idx += 1
        if open_idx >= len(lines):
            raise SystemExit(f"could not find {{ for {name}")
    depth = 0
    block_end = None
    for i in range(open_idx, len(lines)):
        for ch in lines[i]:
            if ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
                if depth == 0:
                    block_end = i
                    break
        if block_end is not None:
            break
    if block_end is None:
        raise SystemExit(f"unterminated fn body for {name}")

    # Eat one trailing blank line so we don't accumulate double blanks.
    if block_end + 1 < len(lines) and lines[block_end + 1].strip() == "":
        block_end += 1

    return block_start, block_end


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2

    path = Path(sys.argv[1])
    names = sys.argv[2:]
    lines = path.read_text().splitlines(keepends=True)

    spans = []
    for name in names:
        start, end = find_test_span(lines, name)
        spans.append((start, end, name))

    # Apply deletions in descending order so earlier line numbers stay valid.
    spans.sort(reverse=True)
    for start, end, name in spans:
        print(f"  drop {name}: lines {start + 1}-{end + 1}")
        del lines[start : end + 1]

    path.write_text("".join(lines))
    return 0


if __name__ == "__main__":
    sys.exit(main())
