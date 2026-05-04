#!/usr/bin/env python3
"""Phase 3 task 3: rename `InlineImageStoreState` ->
`InlineImageStoreReadState` across the workspace. Idempotent.

The renamed type keeps all its methods for the moment; Phase 3 task 5 / 7
plumbs `InlineImageStoreWriteState` through the sync provider contexts
and removes the write methods from the read state.
"""

from __future__ import annotations

from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
TARGETS = [
    "crates/imap/src/imap_delta.rs",
    "crates/imap/src/imap_initial.rs",
    "crates/imap/src/sync_pipeline.rs",
    "crates/graph/src/shared_mailbox_sync.rs",
    "crates/graph/src/sync/mod.rs",
    "crates/graph/src/sync/stores.rs",
    "crates/sync/src/persistence.rs",
    "crates/gmail/src/sync/storage.rs",
    "crates/gmail/src/sync/mod.rs",
    "crates/common/src/signature_images.rs",
    "crates/common/src/types.rs",
    "crates/app/src/app.rs",
    "crates/app/src/db/threads.rs",
    "crates/core/src/sync_dispatch.rs",
    "crates/core/src/chat.rs",
    "crates/core/src/provider/account_resync.rs",
    "crates/dev-seed/src/lib.rs",
    "crates/stores/src/inline_image_store.rs",
    "crates/stores/src/attachment_cache.rs",
    "crates/service/src/actions/tests.rs",
    "crates/service/src/actions/context.rs",
    "crates/service/src/actions/worker.rs",
    "crates/jmap/src/shared_mailbox_sync.rs",
    "crates/jmap/src/sync/mod.rs",
]

OLD = "InlineImageStoreState"
NEW = "InlineImageStoreReadState"


def rewrite(path: Path) -> bool:
    text = path.read_text()
    new_text = text.replace(OLD, NEW)
    if new_text != text:
        path.write_text(new_text)
        return True
    return False


def main() -> int:
    changed: list[str] = []
    for rel in TARGETS:
        p = REPO_ROOT / rel
        if not p.exists():
            print(f"missing: {rel}")
            continue
        if rewrite(p):
            changed.append(rel)
    print(f"Rewrote {len(changed)} files")
    for r in changed:
        print(f"  {r}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
