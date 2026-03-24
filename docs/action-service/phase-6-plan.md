# Action Service: Phase 6 Detailed Plan — Enforce the Boundary

## Goal

Remove all provider crate dependencies from the app crate's `Cargo.toml` so the app physically cannot bypass the action service. After this phase, the compilation boundary is enforced — adding a direct provider call in the app crate causes a build failure.

## Current State

The app crate has 5 provider dependencies:

```toml
ratatoskr-provider-utils = { path = "../provider-utils" }
ratatoskr-gmail = { path = "../gmail" }
ratatoskr-graph = { path = "../graph" }
ratatoskr-jmap = { path = "../jmap" }
ratatoskr-imap = { path = "../imap" }
```

These are used in 6 places across 2 files:

| Usage | File | What it does | Blocker? |
|---|---|---|---|
| `load_encryption_key` | `main.rs:469` | Boot-time key loading | Re-export from core |
| `create_provider` wrapper | `provider.rs:22-29` | Wraps `core::actions::create_provider` | Remove — only used by sync |
| `ProviderOps` trait | `provider.rs:11` | Sync dispatch | Move sync dispatch to core |
| `ProviderCtx` type | `provider.rs:12` | Sync dispatch | Move sync dispatch to core |
| `JmapClient::from_account` | `provider.rs:146` | JMAP push setup | Move push setup to core |
| `jmap::push::*` | `provider.rs:153-154` | JMAP push channel + start | Move push setup to core |

## Design

### Move sync dispatch to core

`dispatch_sync_delta` currently lives in the app crate because it constructs `ProviderCtx` and calls `provider.sync_delta()`. This is the **read path** — not an action service concern, but it uses provider types directly.

Move the async sync logic to `core::sync_dispatch`:

```rust
// crates/core/src/sync_dispatch.rs
pub async fn sync_delta_for_account(
    db: &DbState,
    account_id: &str,
    encryption_key: [u8; 32],
    body_store: &BodyStoreState,
    inline_images: &InlineImageStoreState,
    search: &SearchState,
    progress: &dyn ProgressReporter,
) -> Result<(), String>
```

The app handler becomes a thin wrapper that clones stores and calls `Task::perform(sync_delta_for_account(...))`.

### Move JMAP push setup to core

`start_jmap_push` constructs `JmapClient`, creates a push channel, and starts the push manager. Move to `core::jmap_push`:

```rust
// crates/core/src/jmap_push.rs (or similar)
pub async fn start_jmap_push_for_account(
    db: &DbState,
    account_id: &str,
    encryption_key: [u8; 32],
) -> Result<String, String>  // returns account_id on first state change
```

The app handler calls this via `Task::perform` and maps the result to `Message::SyncComplete`.

### Re-export `load_encryption_key` from core

`provider-utils` is already a dependency of core. Add a re-export:

```rust
// crates/core/src/lib.rs
pub use ratatoskr_provider_utils::crypto::load_encryption_key;
```

The app changes `ratatoskr_provider_utils::crypto::load_encryption_key` to `ratatoskr_core::load_encryption_key`.

### Remove provider dependencies from app

After all usages are migrated, remove all 5 provider crate lines from `crates/app/Cargo.toml`. The `ratatoskr-provider-utils` re-export of `load_encryption_key` through core means the app doesn't need provider-utils directly either.

## Implementation Steps

### Step 1: Re-export `load_encryption_key` from core

Add to `crates/core/src/lib.rs`:
```rust
pub use ratatoskr_provider_utils::crypto::load_encryption_key;
```

Update `main.rs:469` to use `ratatoskr_core::load_encryption_key`.

### Step 2: Move sync dispatch to core

Create `crates/core/src/sync_dispatch.rs` with `sync_delta_for_account`. It:
1. Calls `actions::create_provider` to get `Box<dyn ProviderOps>`
2. Constructs `ProviderCtx` from the provided stores
3. Calls `provider.sync_delta(&ctx, None)`

The app's `dispatch_sync_delta` becomes:
```rust
fn dispatch_sync_delta(&self, account_id: String) -> Task<Message> {
    // ... extract stores ...
    Task::perform(
        async move {
            ratatoskr_core::sync_dispatch::sync_delta_for_account(
                &core_db, &account_id, encryption_key,
                &body_store, &inline_images, &*search_state,
                reporter.as_ref(),
            ).await
        },
        move |result| Message::SyncComplete(aid, result),
    )
}
```

### Step 3: Move JMAP push setup to core

Create `crates/core/src/jmap_push.rs` with `start_jmap_push_for_account`. It:
1. Constructs `JmapClient::from_account`
2. Creates push channel
3. Starts push manager
4. Waits for first state change
5. Returns `account_id`

The app's `start_jmap_push` calls this via `Task::perform`.

### Step 4: Remove the `create_provider` wrapper from app

Delete the `create_provider` function in `handlers/provider.rs:22-29`. It was a temporary wrapper — all remaining call sites now go through core.

### Step 5: Remove provider imports from app

Remove all `use ratatoskr_provider_utils::*`, `use ratatoskr_jmap::*`, etc. from `handlers/provider.rs`. The module becomes a thin layer of `Task::perform` calls to core functions.

### Step 6: Remove provider crate dependencies from `Cargo.toml`

Remove these 5 lines from `crates/app/Cargo.toml`:
```toml
ratatoskr-provider-utils = { path = "../provider-utils" }
ratatoskr-gmail = { path = "../gmail" }
ratatoskr-graph = { path = "../graph" }
ratatoskr-jmap = { path = "../jmap" }
ratatoskr-imap = { path = "../imap" }
```

### Step 7: Verify

- `cargo check --workspace` — the app compiles without provider crates
- `cargo clippy -p app`
- Try adding `use ratatoskr_gmail::*;` to any app file — should fail to compile

## Exit Criteria

1. `crates/app/Cargo.toml` has zero provider crate dependencies.
2. No `ratatoskr_provider_utils`, `ratatoskr_gmail`, `ratatoskr_graph`, `ratatoskr_jmap`, or `ratatoskr_imap` imports anywhere in `crates/app/src/`.
3. Sync dispatch works through `core::sync_dispatch`.
4. JMAP push works through core.
5. `load_encryption_key` is re-exported from core.
6. The compilation boundary is enforced — adding a provider import to the app crate is a build error.

## What Phase 6 Does NOT Do

- **Move the sync orchestrator into core.** Only the `sync_delta` dispatch moves. The sync scheduling (5-minute timer, SyncTick, account iteration) stays in the app.
- **Redesign the sync pipeline.** The read path is out of scope for the action service effort. The sync pipeline's internal architecture is unchanged.
- **Remove `ratatoskr-calendar` from app.** Calendar actions live in the calendar crate (not core) due to the circular dependency. The app legitimately depends on `ratatoskr-calendar` for calendar action access.
