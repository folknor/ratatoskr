#!/usr/bin/env python3
"""Phase 2 task 7: switch service-side action consumers from
`ProviderCtx` to `ActionProviderCtx`.

Uniform pattern across the per-action files in
`crates/service/src/actions/`: a 7-line `ProviderCtx { ... }` block
that drops 3 fields (`body_store`, `inline_images`, `search`) on the
narrower `ActionProviderCtx` and keeps `account_id`, `db`, `progress`.
Plus an import switch.

`folder.rs` and `send.rs` are NOT touched - they call
`create_folder` / `send_email` / draft methods which stay on the
wider `ProviderCtx`.
"""

from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
ACTIONS = REPO / "crates" / "service" / "src" / "actions"

# Files whose ProviderCtx construction is for ACTION methods (8 files,
# label.rs has two sites). folder.rs and send.rs stay on ProviderCtx.
TARGETS = [
    "archive.rs",
    "trash.rs",
    "mark_read.rs",
    "star.rs",
    "spam.rs",
    "permanent_delete.rs",
    "move_to_folder.rs",
    "label.rs",
]

OLD_BLOCK = """    let provider_ctx = ProviderCtx {
        account_id,
        db: &ctx.db,
        body_store: &ctx.body_store,
        inline_images: &ctx.inline_images,
        search: &ctx.search,
        progress: &NoopProgressReporter,
    };"""

NEW_BLOCK = """    let provider_ctx = ActionProviderCtx {
        account_id,
        db: &ctx.db,
        progress: &NoopProgressReporter,
    };"""

OLD_IMPORT = "use common::types::ProviderCtx;"
NEW_IMPORT = "use common::types::ActionProviderCtx;"


def main() -> int:
    total_blocks = 0
    total_imports = 0
    for name in TARGETS:
        path = ACTIONS / name
        text = path.read_text()
        new_text = text
        block_count = new_text.count(OLD_BLOCK)
        new_text = new_text.replace(OLD_BLOCK, NEW_BLOCK)
        import_count = new_text.count(OLD_IMPORT)
        new_text = new_text.replace(OLD_IMPORT, NEW_IMPORT)
        if new_text != text:
            path.write_text(new_text)
            total_blocks += block_count
            total_imports += import_count
            print(f"  {name}: {block_count} block(s), {import_count} import(s)")
    print(f"\n{total_blocks} ctx blocks, {total_imports} imports updated")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
