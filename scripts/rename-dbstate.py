#!/usr/bin/env python3
"""Phase 2 task 3: rename `DbState` -> `ReadDbState` workspace-wide.

Word-boundary regex so `WriteDbState` (in service-state) and any future
`*DbState` types are not touched. Excludes the `service-state` crate
which already uses `WriteDbState` correctly. Excludes the script itself.

Run from repo root:
    python3 scripts/rename-dbstate.py

Idempotent: re-running on already-renamed files is a no-op (the regex
matches `DbState` standalone; `ReadDbState` does not match).

After running, commit if `brokkr check` is clean. The script can be
left in `scripts/` or deleted - one-shot mechanical rename, no reuse
expected.
"""

import re
import sys
from pathlib import Path

PATTERN = re.compile(r'\bDbState\b')
REPO_ROOT = Path(__file__).resolve().parent.parent


def should_process(path: Path) -> bool:
    if path.suffix != '.rs':
        return False
    rel = path.relative_to(REPO_ROOT)
    parts = rel.parts
    # Skip the service-state crate (already uses WriteDbState).
    if len(parts) >= 2 and parts[0] == 'crates' and parts[1] == 'service-state':
        return False
    # Skip target/ (build artefacts).
    if 'target' in parts:
        return False
    return True


def rename_file(path: Path) -> int:
    text = path.read_text()
    new_text, count = PATTERN.subn('ReadDbState', text)
    if count > 0:
        path.write_text(new_text)
    return count


def main() -> int:
    crates = REPO_ROOT / 'crates'
    if not crates.is_dir():
        print(f'crates/ not found under {REPO_ROOT}', file=sys.stderr)
        return 1

    total_files = 0
    total_replacements = 0
    for path in crates.rglob('*.rs'):
        if not should_process(path):
            continue
        replacements = rename_file(path)
        if replacements > 0:
            total_files += 1
            total_replacements += replacements
            print(f'  {path.relative_to(REPO_ROOT)}: {replacements}')

    print(f'\n{total_replacements} replacements across {total_files} files')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
