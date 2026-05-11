#!/usr/bin/env python3
"""Strip cfg(feature = "test-helpers") attributes across the workspace.

Rules:
- #[cfg(feature = "test-helpers")]            -> delete the attribute line
                                                 (item below stays, always-compiled)
- #[cfg(any(test, feature = "test-helpers"))] -> delete the attribute line
                                                 (gated item was active in tests OR
                                                 harness; after cleanup, both arms
                                                 collapse to "always on")
- #[cfg(all(test, feature = "test-helpers"))] -> rewrite to #[cfg(test)]
                                                 (was "tests AND harness only";
                                                 after cleanup, the harness arm is
                                                 always-on, so the AND collapses to
                                                 just "tests only")
- #[cfg(not(feature = "test-helpers"))]       -> emit a warning, leave the line alone
                                                 (these need hand-deletion of both attr +
                                                 the gated item, since the always-on
                                                 sibling makes the not() arm dead).

Pass --dry-run to print the planned edits without writing. Default: write.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

DELETE_RE = re.compile(
    r'^\s*#\[cfg\(feature\s*=\s*"test-helpers"\)\]\s*$'
)
ANY_TEST_RE = re.compile(
    r'^\s*#\[cfg\(any\(test,\s*feature\s*=\s*"test-helpers"\)\)\]\s*$'
)
ALL_TEST_RE = re.compile(
    r'^(\s*)#\[cfg\(all\(test,\s*feature\s*=\s*"test-helpers"\)\)\]\s*$'
)
NOT_RE = re.compile(r'^\s*#\[cfg\(not\(feature\s*=\s*"test-helpers"\)\)\]\s*$')

# Anything else that mentions the feature - flag it, don't touch it.
MENTIONS_RE = re.compile(r'feature\s*=\s*"test-helpers"')


def process_file(path: Path, write: bool) -> tuple[int, int, list[str]]:
    """Return (deletions, rewrites, warnings)."""
    text = path.read_text()
    lines = text.splitlines(keepends=True)
    out: list[str] = []
    deletions = 0
    rewrites = 0
    warnings: list[str] = []
    for i, line in enumerate(lines, start=1):
        if DELETE_RE.match(line) or ANY_TEST_RE.match(line):
            deletions += 1
            continue  # drop the line entirely
        m = ALL_TEST_RE.match(line)
        if m:
            indent = m.group(1)
            out.append(f"{indent}#[cfg(test)]\n")
            rewrites += 1
            continue
        if NOT_RE.match(line):
            warnings.append(f"  {path}:{i}  cfg(not(feature)) - hand-fix needed")
            out.append(line)
            continue
        if MENTIONS_RE.search(line):
            warnings.append(f"  {path}:{i}  unrecognized form: {line.rstrip()}")
        out.append(line)
    new_text = "".join(out)
    if write and new_text != text:
        path.write_text(new_text)
    return deletions, rewrites, warnings


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    rs_files = sorted(p for p in (ROOT / "crates").rglob("*.rs"))
    total_d = total_r = 0
    all_warnings: list[str] = []
    touched_files: list[tuple[Path, int, int]] = []
    for path in rs_files:
        d, r, warnings = process_file(path, write=not args.dry_run)
        if d or r or warnings:
            touched_files.append((path, d, r))
            all_warnings.extend(warnings)
            total_d += d
            total_r += r

    for path, d, r in touched_files:
        rel = path.relative_to(ROOT)
        bits = []
        if d:
            bits.append(f"delete x{d}")
        if r:
            bits.append(f"rewrite x{r}")
        if bits:
            print(f"  {rel}: {', '.join(bits)}")

    if all_warnings:
        print("\nWarnings (hand-fix required):")
        for w in all_warnings:
            print(w)

    print(
        f"\n{'DRY-RUN' if args.dry_run else 'APPLIED'}: "
        f"{total_d} deletions, {total_r} rewrites, "
        f"{len(all_warnings)} warnings"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
