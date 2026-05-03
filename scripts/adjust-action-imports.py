#!/usr/bin/env python3
"""Phase 2 task 6: adjust imports in moved action service files.

Reads each .rs file under crates/service/src/actions/ and crates/service/src/send.rs,
rewrites `crate::*` paths that pointed at core's modules to direct
references against the workspace crates that core was re-exporting.

Idempotent regex-based; safe to re-run. Run from repo root:
    python3 scripts/adjust-action-imports.py

After running, brokkr check should pass. Script can be left in
scripts/ or deleted.
"""

import re
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# Order matters: more-specific paths first so they don't get clobbered
# by a more-general regex. Each rule is (pattern, replacement).
RULES = [
    # core::email_actions only re-exports from db::db::queries_extra
    (r"\bcrate::email_actions::", "db::db::queries_extra::"),
    # core::body_store re-exports store::body_store
    (r"\bcrate::body_store::", "store::body_store::"),
    # core::db is a facade over db::db
    (r"\bcrate::db::", "db::db::"),
    # core::progress is a re-export of db::progress
    (r"\bcrate::progress::", "db::progress::"),
    # core::search is a re-export of the search crate
    (r"\bcrate::search::", "search::"),
    # core::provider::encoding is a re-export of common::encoding
    (r"\bcrate::provider::encoding::", "common::encoding::"),
    # core::provider re-exports common::typed_ids and common::ops
    (r"\bcrate::provider::typed_ids::", "common::typed_ids::"),
    (r"\bcrate::provider::ops::", "common::ops::"),
    # crate::send stays - service crate now has its own send module at
    # crates/service/src/send.rs, so `crate::send::*` resolves correctly.
]


def adjust(text: str) -> tuple[str, int]:
    total = 0
    for pattern, replacement in RULES:
        text, count = re.subn(pattern, replacement, text)
        total += count
    return text, total


def main() -> int:
    targets = [
        REPO_ROOT / "crates" / "service" / "src" / "send.rs",
    ]
    actions_dir = REPO_ROOT / "crates" / "service" / "src" / "actions"
    targets.extend(sorted(actions_dir.rglob("*.rs")))

    total_files = 0
    total_replacements = 0
    for path in targets:
        if not path.is_file():
            continue
        text = path.read_text()
        new_text, count = adjust(text)
        if count > 0:
            path.write_text(new_text)
            total_files += 1
            total_replacements += count
            print(f"  {path.relative_to(REPO_ROOT)}: {count}")

    print(f"\n{total_replacements} replacements across {total_files} files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
