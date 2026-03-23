# Action Service: Phase 1 Detailed Plan

## Goal

Prove the action service pattern with archive. After this phase: archive flows through the service (local DB + provider dispatch), the app handler delegates to the service, and the types are ready for Phase 2 to replicate the pattern.

## Key Findings from Audit

1. **`ProviderOps` already has `fn archive()`** — but the app never calls it. The current handler only does `remove_label(conn, account_id, thread_id, "INBOX")`. Archive has been local-only since it was wired. Phase 1 fixes this.

2. **`provider_label_write_back` re-initializes stores on every call.** It calls `BodyStoreState::init()`, `SearchState::init()`, and `InlineImageStoreState::init()` — all filesystem I/O — even though these stores already exist on `App` and are `Clone`. The action service uses pre-initialized stores.

3. **`create_provider` lives in the app crate** (`handlers/provider.rs`) and imports all four provider crates directly. This function needs to move to core, which already depends on all provider crates.

4. **`ProviderCtx` requires 6 fields:** `account_id`, `db` (DbState), `body_store`, `inline_images`, `search`, `progress`. All are references with a shared lifetime. The action context holds owned/cloned versions so it can construct `ProviderCtx` internally.

5. **All stores are `Clone`** (wrap `Arc<Mutex<Connection>>` or similar). The action context owns clones cheaply.

6. **`create_provider` takes `&Arc<Db>`, not `&DbState`.** The app's `Db` wrapper and core's `DbState` are different types. Moving `create_provider` to core requires adapting it to use `DbState` (which exposes `conn()` returning `Arc<Mutex<Connection>>`). The account lookup query and provider construction use the same underlying connection — the adaptation is mechanical but must be done explicitly, not assumed.

## Design Decisions

### Where the service lives

In `ratatoskr-core`, module `core/src/actions/`. Core already depends on all provider crates, the DB crate, stores, and provider-utils. No new crate needed.

### Action context

```rust
#[derive(Clone)]
pub struct ActionContext {
    pub db: DbState,
    pub body_store: BodyStoreState,
    pub inline_images: InlineImageStoreState,
    pub search: SearchState,
    pub encryption_key: [u8; 32],
}
```

All fields are owned and cheaply cloneable. No lifetimes. Constructed once at app startup from the same stores `App` already initializes. The service constructs `ProviderCtx` internally from these fields per-call — the caller never sees `ProviderCtx`.

No `progress` field in Phase 1. Individual actions don't emit progress events yet. When they do (Phase 5 bulk operations), the field gets added. Carrying unused fields invites confusion.

`ActionContext` is app-global and account-independent. Actions take `account_id` as a parameter because a single context serves all accounts.

### Action contract

Actions return `ActionOutcome` directly, not `Result<ActionOutcome, _>`. The outcome type already has a `Failed` variant for errors. A wrapping `Result` would create ambiguity about what constitutes an "outer" error vs an "inner" failure. One type, one place to check.

```rust
#[derive(Debug, Clone)]
pub enum ActionOutcome {
    /// Local and remote both succeeded.
    Success,
    /// Local succeeded, remote dispatch failed.
    /// The action took effect locally but may revert on next sync.
    LocalOnly { remote_error: String },
    /// The action failed entirely (local not applied).
    Failed { error: String },
}
```

The `String` error fields are temporary. Phase 3 replaces them with a structured error enum. This is noted here so Phase 2 doesn't proliferate stringly-typed errors across 10 actions — if the pattern starts feeling wrong during Phase 2, introduce the error enum early.

### How the UI handles outcomes

In Phase 1, the app handler maps outcomes to user-visible feedback:

- `Success` — show the existing confirmation toast ("Archived").
- `LocalOnly` — show a warning toast ("Archived locally — sync may revert this"). This is new behavior: archive failures were previously silent.
- `Failed` — show an error in the status bar. Do not auto-advance.

This is provisional. Phase 3 defines the mature outcome-to-UI mapping. But Phase 1 must surface outcomes visibly, otherwise the new service provides no observable improvement over the old path.

### Provider resolution

Move `create_provider` from `crates/app/src/handlers/provider.rs` to `crates/core/src/actions/provider.rs`. The function signature changes from `&Arc<Db>` to `&DbState`:

```rust
pub async fn create_provider(
    db: &DbState,
    account_id: &str,
    encryption_key: [u8; 32],
) -> Result<Box<dyn ProviderOps>, String>
```

The adaptation: the current function uses `db.with_conn()` (an async method on the app's `Db` wrapper). The core version uses `DbState::conn()` to get the `Arc<Mutex<Connection>>`, then `spawn_blocking` for the account lookup query. The provider client construction (`GmailClient::from_account`, etc.) already takes `&DbState` — no change needed there.

The app crate's `handlers/provider.rs` becomes a thin wrapper or is removed. Other app code that calls `create_provider` (e.g., sync orchestrator) is updated to call the core version.

### Async model

Action functions are `async` and return `ActionOutcome`. DB operations use `tokio::task::spawn_blocking` (same as the current codebase — DB connections are sync). Provider calls are `await`ed directly.

The service does not depend on iced or `Task<Message>`. It is a pure async library. The app wraps calls in `Task::perform`.

### No fallback to the old path

If `ActionContext` cannot be constructed at boot (a store fails to initialize), archive does not silently fall back to the old local-only path. That would preserve exactly the class of silent divergence Phase 1 is supposed to eliminate.

Instead: `ActionContext` construction failure is logged as an error at boot. The `action_ctx` field on `App` is `Option<ActionContext>`. If `None`, the archive handler shows an error in the status bar and returns `Task::none()` — this is a pre-dispatch UI error, not an `ActionOutcome`. The service is never called. The old inline DB code for archive is removed — there is one path, not two.

This means: if stores fail to initialize, archive doesn't work. That's correct. The stores are already required for the app to function (body store is needed for reading emails, search for search). If they're broken, the app is already degraded. Archive failing explicitly is better than archive silently not syncing.

### Existing undo tokens

The current code creates `UndoToken::Archive` in the UI layer regardless of what the action service does. Phase 1 leaves this in place — the token is still created by the UI, still based on what was requested, still not backed by execution history. This is explicitly not fixed until Phase 4. The token is inert (undo was already a no-op for archive since it never reached the provider), so leaving it doesn't regress behavior.

**Warning for implementers:** archive now reaches the provider for the first time. But undo still does not. A user who archives and then undoes will reverse the local state but not the remote state. This was always true (undo never reached the provider), but it is more consequential now that archive does. Do not interpret a working archive as meaning undo is trustworthy. Add a code comment at the undo token creation site noting this.

## Implementation Steps

### Step 1: Create module structure

Create `crates/core/src/actions/mod.rs` with:
- `mod context;` — `ActionContext`
- `mod outcome;` — `ActionOutcome`
- `mod provider;` — `create_provider` (moved from app)
- `mod archive;` — the archive action

Re-export:
```rust
pub use context::ActionContext;
pub use outcome::ActionOutcome;
pub use archive::archive;
```

Add `pub mod actions;` to `crates/core/src/lib.rs`.

### Step 2: Move `create_provider` to core

Move the body of `handlers/provider.rs::create_provider()` into `core/src/actions/provider.rs`.

Interface change: replace `db.with_conn(|conn| ...)` with `spawn_blocking` + `db.conn().lock()`. The account lookup query (`SELECT provider FROM accounts WHERE id = ?1`) and provider client construction are unchanged — they already work with `&Connection` and `&DbState` respectively.

Leave a thin wrapper in `crates/app/src/handlers/provider.rs` that constructs a `DbState` from `self.db` and delegates to the core function:

```rust
pub(crate) async fn create_provider(
    db: &Arc<Db>,
    account_id: &str,
    encryption_key: [u8; 32],
) -> Result<Box<dyn ProviderOps>, String> {
    let core_db = ratatoskr_core::db::DbState::from_arc(db.write_conn_arc());
    ratatoskr_core::actions::provider::create_provider(&core_db, account_id, encryption_key).await
}
```

This wrapper is temporary — Phase 2 removes it as other callers migrate. To be precise: Phase 1 moves the implementation and ownership of `create_provider` to core. Some app-crate call sites (sync orchestrator, label write-back) still go through the wrapper until they're migrated in Phase 2.

### Step 3: Implement `ActionContext`

As specified in the design section. No `progress` field. All owned, all `Clone`.

### Step 4: Implement `ActionOutcome`

As specified in the design section. Three variants, `String` errors, helper methods.

### Step 5: Implement archive action

```rust
pub async fn archive(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
) -> ActionOutcome {
    // 1. Local DB mutation (on blocking thread)
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        crate::email_actions::remove_inbox_label(&conn, &aid, &tid)
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))
    .and_then(|r| r);

    if let Err(e) = local_result {
        return ActionOutcome::Failed { error: e };
    }

    // 2. Provider dispatch
    let provider = match super::provider::create_provider(
        &ctx.db, account_id, ctx.encryption_key,
    ).await {
        Ok(p) => p,
        Err(e) => {
            log::warn!("Archive local-only (provider create failed): {e}");
            return ActionOutcome::LocalOnly { remote_error: e };
        }
    };

    let provider_ctx = ratatoskr_provider_utils::types::ProviderCtx {
        account_id,
        db: &ctx.db,
        body_store: &ctx.body_store,
        inline_images: &ctx.inline_images,
        search: &ctx.search,
        progress: &crate::progress::NoopProgressReporter,
    };

    match provider.archive(&provider_ctx, thread_id).await {
        Ok(()) => ActionOutcome::Success,
        Err(e) => {
            let msg = e.to_string();
            log::warn!("Archive remote failed for {account_id}/{thread_id}: {msg}");
            ActionOutcome::LocalOnly { remote_error: msg }
        }
    }
}
```

DB mutation runs on `spawn_blocking`. Provider dispatch is awaited. `ProviderCtx.progress` uses `NoopProgressReporter` — the context doesn't carry progress because no action emits progress events yet. When one does, the field is added to `ActionContext` and threaded through here.

Note: the loop over multiple threads is the caller's concern, not the action function's. Each action operates on one thread. Sequential dispatch for now — Phase 5 addresses concurrency and batching.

### Step 6: Construct `ActionContext` in app boot

```rust
let action_ctx = match (&body_store, &inline_image_store, &search_state, encryption_key) {
    (Some(bs), Some(iis), Some(ss), Some(key)) => {
        Some(ratatoskr_core::actions::ActionContext {
            db: ratatoskr_core::db::DbState::from_arc(db.write_conn_arc()),
            body_store: bs.clone(),
            inline_images: iis.clone(),
            search: (**ss).clone(),
            encryption_key: key,
        })
    }
    _ => {
        log::error!("Action service unavailable: one or more stores failed to initialize");
        None
    }
};
```

Add `action_ctx: Option<ratatoskr_core::actions::ActionContext>` to `App` struct.

### Step 7: Migrate app archive handler

Remove the `EmailAction::Archive` arm from `dispatch_email_db_action`. Replace with a dedicated method:

```rust
fn dispatch_archive(&self, threads: Vec<(String, String)>) -> Task<Message> {
    let Some(ref action_ctx) = self.action_ctx else {
        self.status_bar.show_error("Archive unavailable — service not initialized");
        return Task::none();
    };

    let ctx = action_ctx.clone();
    Task::perform(
        async move {
            let mut outcomes = Vec::with_capacity(threads.len());
            for (account_id, thread_id) in &threads {
                outcomes.push(
                    ratatoskr_core::actions::archive(&ctx, account_id, thread_id).await
                );
            }
            outcomes
        },
        |outcomes| {
            // Surface the worst outcome to the user
            Message::ArchiveCompleted(outcomes)
        },
    )
}
```

Add a `Message::ArchiveCompleted(Vec<ActionOutcome>)` variant. The handler maps outcomes to UI feedback:

- All `Success` → confirmation toast ("Archived")
- Any `LocalOnly` → warning toast ("Archived locally — sync may revert")
- Any `Failed` → error in status bar

The old inline `remove_label(conn, "INBOX")` code for archive is deleted. No fallback path.

### Step 8: Verify

- `cargo check --workspace` and `cargo clippy -p app -p ratatoskr-core`.
- Verify the app crate's archive handler does not call `remove_inbox_label` or any label DB function directly.
- Verify `create_provider` in core compiles and constructs all four provider types.
- Manual smoke test: archive a thread, verify it reaches the provider (check server-side state or provider call logs). This is the minimum verification that the `ProviderOps::archive()` call actually works — it has never been called before.
- Verify that `LocalOnly` and `Failed` outcomes produce visible UI feedback (warning/error toast).

## What This Produces

- `crates/core/src/actions/mod.rs` — module root, re-exports
- `crates/core/src/actions/context.rs` — `ActionContext`
- `crates/core/src/actions/outcome.rs` — `ActionOutcome`
- `crates/core/src/actions/provider.rs` — `create_provider` (moved from app)
- `crates/core/src/actions/archive.rs` — the archive action
- Modified `crates/app/src/main.rs` — constructs and stores `ActionContext`
- Modified `crates/app/src/handlers/commands.rs` — archive delegates to service
- New `Message::ArchiveCompleted` variant with outcome-based UI feedback

## Exit Criteria

1. `actions::archive()` performs local DB mutation + provider dispatch end-to-end via `spawn_blocking` (DB) and `await` (provider).
2. The app handler for archive delegates to the service. No direct `remove_inbox_label` or `remove_label` calls in the app crate's archive path.
3. When `ActionContext` is unavailable, archive fails visibly — no silent fallback to local-only.
4. `ActionOutcome::LocalOnly` and `ActionOutcome::Failed` produce visible user feedback (warning/error toast), distinct from `Success`.
5. `create_provider` lives in core. The app crate retains a thin wrapper (removed in Phase 2).
6. `ActionContext` and `ActionOutcome` types are usable for Phase 2 actions without modification (except error type refinement).
7. Manual verification that `ProviderOps::archive()` is actually called and reaches the provider — this method has never been exercised.
8. The workspace compiles and passes clippy.

## What Phase 2 Will Do With This

Phase 2 replicates the archive pattern for each remaining action:
- Add `trash.rs`, `star.rs`, `read.rs`, `label.rs`, `move_to_folder.rs`, `snooze.rs`, `delete.rs`, `pin.rs`, `mute.rs` to `core/src/actions/`.
- Each follows the same structure: local DB mutation (spawn_blocking) → provider dispatch (await) → return `ActionOutcome`.
- Migrate each app handler to call the service.
- Remove `provider_label_write_back` and the remaining inline provider code from the app crate.
- Remove the thin `create_provider` wrapper from the app crate.
- Define the local-only-by-design marker for pin/mute.
- If `String` error fields start causing friction, introduce a structured error enum early.
