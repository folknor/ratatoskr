# Action Service: Phase 2.4 Detailed Plan

## Goal

Add folder CRUD to the action service so that when folder management UI is built, it goes through the service from day one. All four providers have real `create_folder`, `rename_folder`, and `delete_folder` implementations on `ProviderOps`. No UI or app-crate call sites exist yet — this phase defines the service API and local DB operations.

## Current State

### Provider interface

```rust
async fn create_folder(ctx, name, parent_id, text_color, bg_color)
    -> Result<ProviderFolderMutation, ProviderError>;
async fn rename_folder(ctx, folder_id, new_name, text_color, bg_color)
    -> Result<ProviderFolderMutation, ProviderError>;
async fn delete_folder(ctx, folder_id) -> Result<(), ProviderError>;
```

`ProviderFolderMutation` returns the folder's identity after creation/rename:

```rust
pub struct ProviderFolderMutation {
    pub id: String,
    pub name: String,
    pub path: String,
    pub folder_type: String,
    pub special_use: Option<String>,
    pub delimiter: Option<String>,
    pub color_bg: Option<String>,
    pub color_fg: Option<String>,
}
```

### Local DB

The `labels` table stores folders as rows with `label_kind = 'container'`:

```
id TEXT NOT NULL,
account_id TEXT NOT NULL,
name TEXT NOT NULL,
type TEXT NOT NULL,
color_bg TEXT, color_fg TEXT,
visible INTEGER DEFAULT 1,
sort_order INTEGER DEFAULT 0,
imap_folder_path TEXT,
imap_special_use TEXT,
label_kind TEXT NOT NULL DEFAULT 'container',
PRIMARY KEY (account_id, id)
```

Existing helpers:
- `db_upsert_label_coalesce(db, id, account_id, name, type, color_bg, color_fg, imap_folder_path, imap_special_use)` — inserts or updates a label row.
- `DELETE FROM labels WHERE account_id = ?1 AND id = ?2` — exists in `queries.rs:602`.

No `label_kind` parameter on `db_upsert_label_coalesce` — it writes `type` but not `label_kind`. For folder creation, `label_kind` defaults to `'container'` (the column default), which is correct. But the upsert helper doesn't set `label_kind` explicitly, which could be a problem if the row somehow already exists as a tag. In practice this won't happen (folder IDs and tag IDs use different prefixes), but the action should set `label_kind = 'container'` explicitly for correctness.

### No existing call sites

No app-crate code calls `create_folder`, `rename_folder`, or `delete_folder` on `ProviderOps`. No UI for folder management exists. This phase defines the action functions and leaves them ready for wiring.

## Design Decisions

### Folder operations are provider-first, local-second

Unlike thread actions (where local DB is mutated first for instant UI feedback), folder operations are **provider-first**: the provider creates/renames/deletes the folder, then the local DB is updated to match. This is because:

- `create_folder` returns a `ProviderFolderMutation` with the provider-assigned ID, path, and metadata. The local DB needs this data — you can't create a local row without knowing the ID.
- `rename_folder` returns updated metadata (the path may change on some providers). The local row should reflect what the provider returned.
- `delete_folder` should only remove the local row if the provider succeeded — deleting locally before the provider creates orphaned `thread_labels` rows that reference a folder the user believes still exists.

This is a different pattern from archive/star/label (local-first, provider-second). The `ActionOutcome` semantics for provider-first actions: `Success` means the provider succeeded. The local DB update is best-effort — if it fails, the action still returns `Success` because the provider state is canonical and the next sync will reconcile. `Failed` means the provider failed (local not modified). This is intentionally different from thread actions where local success is the gate.

### No `LocalOnly` for folder operations

If the provider fails, the local DB is not modified. There is no meaningful "local-only" state for folder creation (what would the ID be?) or deletion (the folder still exists on the server). `Failed` is returned.

For rename, a case could be made for local-only (rename the display name locally, let sync fix it). But that creates confusion — the user sees the new name, the server has the old name, and sync may revert it. Better to fail cleanly.

### Callers must refresh the sidebar after folder operations

If the provider succeeds but the local DB write fails (unusual but possible), the function returns `Success` with a warning log. The provider state is canonical — the next sync will reconcile. But the sidebar won't reflect the change until then. When folder management UI is built, the caller should trigger a navigation/sidebar reload after any successful folder operation, regardless of whether the local DB write succeeded. The `ProviderFolderMutation` return value gives the caller enough data to update the sidebar optimistically if desired.

### ActionOutcome carries the mutation result for create/rename

`create_folder` and `rename_folder` return provider-assigned metadata that the caller may need (e.g., to navigate to the new folder, update the sidebar). The current `ActionOutcome::Success` has no payload.

**Decision:** Return the `ProviderFolderMutation` alongside the outcome. Since `ActionOutcome` is shared across all actions (most of which don't return data), the folder action functions return a tuple `(ActionOutcome, Option<ProviderFolderMutation>)` rather than modifying the enum. On success, both are populated. On failure, the outcome is `Failed` and the mutation is `None`.

### `label_kind` is set explicitly on create

The INSERT sets `label_kind = 'container'` explicitly, not relying on the column default, for defense against future schema changes or rows that might pre-exist from sync.

## Action Function Signatures

```rust
// crates/core/src/actions/folder.rs

pub async fn create_folder(
    ctx: &ActionContext,
    account_id: &str,
    name: &str,
    parent_id: Option<&str>,
    text_color: Option<&str>,
    bg_color: Option<&str>,
) -> (ActionOutcome, Option<ProviderFolderMutation>)

pub async fn rename_folder(
    ctx: &ActionContext,
    account_id: &str,
    folder_id: &str,
    new_name: &str,
    text_color: Option<&str>,
    bg_color: Option<&str>,
) -> (ActionOutcome, Option<ProviderFolderMutation>)

pub async fn delete_folder(
    ctx: &ActionContext,
    account_id: &str,
    folder_id: &str,
) -> ActionOutcome
```

## Implementation Steps

### Step 1: Create `crates/core/src/actions/folder.rs`

**`create_folder`:**

```rust
pub async fn create_folder(
    ctx: &ActionContext,
    account_id: &str,
    name: &str,
    parent_id: Option<&str>,
    text_color: Option<&str>,
    bg_color: Option<&str>,
) -> (ActionOutcome, Option<ProviderFolderMutation>) {
    // 1. Provider dispatch first — we need the provider-assigned ID
    let provider = match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(p) => p,
        Err(e) => return (ActionOutcome::Failed { error: e }, None),
    };

    let provider_ctx = build_provider_ctx(ctx, account_id);

    let mutation = match provider
        .create_folder(&provider_ctx, name, parent_id, text_color, bg_color)
        .await
    {
        Ok(m) => m,
        Err(e) => {
            let msg = e.to_string();
            log::warn!("create_folder failed for {account_id}: {msg}");
            return (ActionOutcome::Failed { error: msg }, None);
        }
    };

    // 2. Local DB — insert the new folder into labels
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let m = mutation.clone();
    let parent_id_for_db = parent_id.map(str::to_string);
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        conn.execute(
            "INSERT INTO labels (id, account_id, name, type, color_bg, color_fg, \
             imap_folder_path, imap_special_use, parent_label_id, label_kind) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'container') \
             ON CONFLICT(account_id, id) DO UPDATE SET \
               name = ?3, type = ?4, color_bg = ?5, color_fg = ?6, \
               imap_folder_path = ?7, imap_special_use = ?8, \
               parent_label_id = ?9, label_kind = 'container'",
            rusqlite::params![
                m.id, aid, m.name, m.folder_type,
                m.color_bg, m.color_fg, m.path, m.special_use, parent_id_for_db,
            ],
        )
        .map_err(|e| format!("local insert: {e}"))?;
        Ok(())
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))
    .and_then(|r| r);

    if let Err(e) = local_result {
        // Provider succeeded but local DB failed — unusual but possible.
        // The folder exists on the server; next sync will pick it up.
        log::warn!("create_folder local insert failed (provider succeeded): {e}");
    }

    (ActionOutcome::Success, Some(mutation))
}
```

**`rename_folder`:**

Same pattern: provider first, then update local row with the returned metadata.

```rust
pub async fn rename_folder(
    ctx: &ActionContext,
    account_id: &str,
    folder_id: &str,
    new_name: &str,
    text_color: Option<&str>,
    bg_color: Option<&str>,
) -> (ActionOutcome, Option<ProviderFolderMutation>) {
    let provider = match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(p) => p,
        Err(e) => return (ActionOutcome::Failed { error: e }, None),
    };

    let provider_ctx = build_provider_ctx(ctx, account_id);

    let mutation = match provider
        .rename_folder(&provider_ctx, folder_id, new_name, text_color, bg_color)
        .await
    {
        Ok(m) => m,
        Err(e) => {
            let msg = e.to_string();
            log::warn!("rename_folder failed for {account_id}/{folder_id}: {msg}");
            return (ActionOutcome::Failed { error: msg }, None);
        }
    };

    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let fid = folder_id.to_string();
    let m = mutation.clone();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        conn.execute(
            "UPDATE labels SET name = ?1, type = ?2, color_bg = ?3, color_fg = ?4, \
             imap_folder_path = ?5, imap_special_use = ?6 \
             WHERE account_id = ?7 AND id = ?8",
            rusqlite::params![
                m.name, m.folder_type, m.color_bg, m.color_fg,
                m.path, m.special_use, aid, fid,
            ],
        )
        .map_err(|e| format!("local update: {e}"))?;
        Ok(())
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))
    .and_then(|r| r);

    if let Err(e) = local_result {
        log::warn!("rename_folder local update failed (provider succeeded): {e}");
    }

    (ActionOutcome::Success, Some(mutation))
}
```

**`delete_folder`:**

```rust
pub async fn delete_folder(
    ctx: &ActionContext,
    account_id: &str,
    folder_id: &str,
) -> ActionOutcome {
    let provider = match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(p) => p,
        Err(e) => return ActionOutcome::Failed { error: e },
    };

    let provider_ctx = build_provider_ctx(ctx, account_id);

    if let Err(e) = provider.delete_folder(&provider_ctx, folder_id).await {
        let msg = e.to_string();
        log::warn!("delete_folder failed for {account_id}/{folder_id}: {msg}");
        return ActionOutcome::Failed { error: msg };
    }

    // Provider succeeded — remove local row
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let fid = folder_id.to_string();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        // Delete thread_labels first — there is no FK cascade from
        // labels to thread_labels (the FK is threads → thread_labels only).
        conn.execute(
            "DELETE FROM thread_labels WHERE account_id = ?1 AND label_id = ?2",
            rusqlite::params![aid, fid],
        )
        .map_err(|e| format!("thread_labels cleanup: {e}"))?;
        conn.execute(
            "DELETE FROM labels WHERE account_id = ?1 AND id = ?2",
            rusqlite::params![aid, fid],
        )
        .map_err(|e| format!("local delete: {e}"))?;
        Ok(())
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))
    .and_then(|r| r);

    if let Err(e) = local_result {
        log::warn!("delete_folder local delete failed (provider succeeded): {e}");
    }

    ActionOutcome::Success
}
```

**Helper for `ProviderCtx` construction:**

All three functions build the same `ProviderCtx`. Extract a helper to reduce duplication:

```rust
fn build_provider_ctx<'a>(ctx: &'a ActionContext, account_id: &'a str) -> ProviderCtx<'a> {
    ProviderCtx {
        account_id,
        db: &ctx.db,
        body_store: &ctx.body_store,
        inline_images: &ctx.inline_images,
        search: &ctx.search,
        progress: &NoopProgressReporter,
    }
}
```

Note: this helper is also useful for all existing action functions (archive, star, label, etc.) which each construct `ProviderCtx` inline. Extracting it here is the right time since `folder.rs` has three functions that all need it. The existing actions can be refactored to use it later — not in this phase.

### Step 2: Register in `crates/core/src/actions/mod.rs`

```rust
mod folder;
pub use folder::{create_folder, rename_folder, delete_folder};
```

Also re-export `ProviderFolderMutation` from actions so callers don't need to import `provider-utils` directly:

```rust
pub use ratatoskr_provider_utils::types::ProviderFolderMutation;
```

### Step 3: Verify

- `cargo check --workspace`
- `cargo clippy -p ratatoskr-core`
- No app-crate changes needed (no UI exists).

## What This Produces

- `crates/core/src/actions/folder.rs` — `create_folder()`, `rename_folder()`, `delete_folder()`
- Modified `crates/core/src/actions/mod.rs` — registers folder module, re-exports types

## Exit Criteria

1. `actions::create_folder()` calls `ProviderOps::create_folder()`, then inserts the returned folder into the `labels` table with `label_kind = 'container'` and `parent_label_id` set.
2. `actions::rename_folder()` calls `ProviderOps::rename_folder()`, then updates the local `labels` row with all provider-returned metadata (`name`, `type`, `color_bg`, `color_fg`, `imap_folder_path`, `imap_special_use`).
3. `actions::delete_folder()` calls `ProviderOps::delete_folder()`, then deletes `thread_labels` rows for that folder, then deletes the `labels` row. (No FK cascade from labels to thread_labels — explicit cleanup required.)
4. All three are provider-first. `Success` means the provider succeeded (local DB update is best-effort — sync reconciles on failure). `Failed` means the provider failed (local not modified). No `LocalOnly`.
5. `create_folder` and `rename_folder` return `(ActionOutcome, Option<ProviderFolderMutation>)` so the caller has the provider-assigned metadata.
6. `ProviderCtx` construction extracted into a shared helper.
7. Core crate compiles and passes clippy.

## What Phase 2.4 Does NOT Do

- **Wire to UI.** No folder management UI exists. These functions are ready for when it's built.
- **Folder hierarchy changes on rename.** `rename_folder` doesn't take a `parent_id` parameter — it renames in place without moving. Reparenting a folder (moving it under a different parent) is a separate operation not covered by `ProviderOps::rename_folder`. If needed, it would be a new action.
- **System folder protection.** Preventing deletion of Inbox, Sent, Trash etc. The provider will reject it (these are immutable on most providers). If a provider doesn't reject it, that's a provider bug. The action service doesn't add its own guard.
- **Refactor existing actions to use the `ProviderCtx` helper.** The helper exists in `folder.rs`. Existing actions can be migrated to use it in a cleanup pass.
