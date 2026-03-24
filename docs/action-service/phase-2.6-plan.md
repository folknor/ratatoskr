# Action Service: Phase 2.6 Detailed Plan

## Goal

Move contact write-back into the action service so that provider dispatch for contact edits has an authoritative path. Today, `dispatch_provider_write_back` exists in the app crate as scaffolding — it logs "queued" or "not yet wired" for all providers except user-local contacts. JMAP is fully implemented but not wired. Google and Graph have body builders and server-info lookups but no HTTP dispatch. CardDAV has no write support.

Phase 2.6 moves the write-back logic into the action service, wires JMAP (which is ready), and provides the dispatch structure for Google/Graph/CardDAV to be completed independently.

## Current State

### What exists

**App handler** (`crates/app/src/handlers/contacts.rs:38-80`):
- `handle_save_contact(entry)` — saves locally via `db.save_contact()`, then calls `dispatch_provider_write_back()` (best-effort, errors logged). Refreshes contact list.
- `handle_delete_contact(id)` — deletes locally via `db.delete_contact()`. No provider dispatch. Refreshes contact list.

**Provider write-back** (`crates/app/src/handlers/contacts.rs:317-368`):
- `dispatch_provider_write_back(db, source, email, phone, company, notes)` — matches on source (`google`/`graph`/`jmap`/`carddav`/`user`). All branches log and return `Ok(())`. No provider call is made.

**Provider scaffolding** (all in core, except JMAP):

| Provider | Server info lookup | Body builder | HTTP dispatch | Status |
|----------|-------------------|--------------|---------------|--------|
| **Google** | `get_google_contact_server_info(db, email)` → `GoogleServerInfo { resource_name, account_id }` | `build_google_contact_update_body(phone, company, etag)` → JSON | Not wired | Scaffolding only |
| **Graph** | `get_graph_contact_server_info(db, email)` → `GraphServerInfo { graph_contact_id, account_id }` | `build_graph_contact_update_body(phone, company)` → JSON | Not wired | Scaffolding only |
| **JMAP** | `get_jmap_contact_server_info(db, email)` → `JmapContactServerInfo { server_id, account_id }` | N/A (built inline) | `jmap_contacts_push_update(client, server_id, phone, company, notes)` | **Fully implemented** |
| **CardDAV** | `carddav_contact_map` table exists | No vCard builder | No PUT method on `CardDavClient` | Not started |

**Key design from `contacts/save.rs`:**
- Display name changes are **local-only** — never pushed to providers. The `display_name_overridden` flag protects local edits from being overwritten by sync.
- Phone, company, and notes are the only fields pushed to providers. Google's body builder doesn't include notes (People API limitation).

**`save_contact_inner`** (`crates/app/src/db/contacts.rs:576-611`):
- Lives in the **app crate**, not core. The action function in core cannot call it. Phase 2.6 must either move it to core or inline equivalent SQL.

### What doesn't exist

- **No contact delete dispatches to providers.** Delete is local-only. Server contacts are orphaned.
- **No typed client access in the write-back path.** `dispatch_provider_write_back` takes `&Arc<Db>`, not typed provider clients. JMAP, Google, and Graph all need their respective clients for HTTP calls.
- **No CardDAV PUT.** `CardDavClient` has PROPFIND and REPORT but no PUT method. Needs vCard generation.

### Critical bug: Settings UI strips synced contact source on save

`crates/app/src/ui/settings/update.rs:850` hardcodes `source: Some("user".to_string())` when building the `ContactEntry` for save, even though the editor state loaded the real source at line 804. This means every synced-contact save arrives at the handler looking like a local contact. Provider write-back never fires.

**Fix required in Phase 2.6:** The save path must preserve the editor's `source` field. Change line 850 from `source: Some("user".to_string())` to `source: editor.source.clone().or(Some("user".to_string()))` — use the editor's source if present (synced contact), fall back to "user" (new contact).

## Design Decisions

### Contact actions live in `core::actions::contacts`

Unlike calendar (which lives in a separate crate due to circular dependency), the contact write-back infrastructure lives in `core` already: `core::contacts::sync_google`, `core::contacts::sync_graph`, `core::carddav`. The JMAP piece is in the `jmap` crate, which core depends on. So `core::actions::contacts` has access to everything it needs. No new crate dependency required.

### Unimplemented providers return `LocalOnly`, not fake `Success`

Google, Graph, and CardDAV write-back branches return `ActionOutcome::LocalOnly` with a descriptive reason ("Google contact write-back not yet wired to HTTP") instead of `Ok(())` mapped to `Success`. This is honest: the local save succeeded but the provider was not notified. The caller (Settings UI) maps `LocalOnly` to `Ok(())` for the reload callback (same as calendar), but the outcome is truthful in logs and for Phase 3 structured reporting.

### Delete is provider-first for synced contacts

Delete for synced contacts (those with a `server_id` and non-user `source`) dispatches to the provider first, then deletes locally on success. Rationale: if the provider delete fails, you don't want to delete locally and orphan the server record — the contact would reappear on next sync. Same logic as calendar and folder delete.

Delete for local contacts (`source = 'user'` or no `server_id`) is local-only.

For Phase 2.6: only JMAP delete is wired. Google, Graph, and CardDAV delete return `LocalOnly` — the contact is deleted locally but remains on the server until the HTTP calls are wired.

### Contact identity for provider dispatch uses `(source, server_id, account_id)`, not email

The existing server-info lookups (`get_google_contact_server_info`, `get_graph_contact_server_info`, `get_jmap_contact_server_info`) resolve by email with `LIMIT 1`. This is fragile for cross-account contacts where the same email exists on multiple providers.

For the save path: the action uses the contact's own `source` and `account_id` (from `ContactSaveInput`) to determine which provider to dispatch to. The server-info lookup by email is still used to find the provider-specific ID (`resource_name`, `graph_contact_id`, `server_id`) — but the provider selection is not ambiguous because `source` is authoritative.

For the delete path: the action looks up `source`, `server_id`, and `account_id` from the `contacts` table by the contact's local `id`. No email-based lookup needed.

### `save_contact_inner` must be extracted to core

The SQL for upserting a contact (`INSERT ... ON CONFLICT(id) DO UPDATE SET ...`) currently lives in `crates/app/src/db/contacts.rs:576-611`. The action function in core needs this. Options:
1. Move the function to `core::db::queries_extra::contacts`
2. Inline the SQL in the action function

**Decision:** Add a `db_upsert_contact_full` function to `core::db::queries_extra::contacts.rs` that takes all fields (email, display_name, email2, phone, company, notes, account_id, source). The app's `save_contact_inner` becomes a thin wrapper or is removed. This is a small refactor — the SQL is ~15 lines.

### No new completion message type needed

Contact save/delete operate from the Settings panel. The existing `SettingsMessage::ContactsLoaded` callback works — the action result is mapped to a contact list reload, same as today.

## Action Function Signatures

```rust
// crates/core/src/actions/contacts.rs

/// Save a contact locally, then dispatch write-back to the provider.
/// Display name is local-only — only phone, company, notes are pushed.
pub async fn save_contact(
    ctx: &ActionContext,
    input: ContactSaveInput,
) -> ActionOutcome

/// Delete a contact. For synced contacts, dispatches provider delete first
/// (provider-first), then deletes locally. For local contacts, deletes
/// locally only.
pub async fn delete_contact(
    ctx: &ActionContext,
    contact_id: &str,
) -> ActionOutcome
```

## `ContactSaveInput`

```rust
pub struct ContactSaveInput {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub email2: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub notes: Option<String>,
    /// Used for local save. NOT used for provider routing — provider is
    /// determined by `source`. This field is informational for the local
    /// DB row; provider dispatch resolves the account via server-info lookup.
    pub account_id: Option<String>,
    /// Provider source: "user", "google", "graph", "jmap", "carddav".
    /// Determines whether and where write-back is dispatched.
    pub source: Option<String>,
}
```

## Implementation Steps

### Step 1: Fix Settings UI source preservation

In `crates/app/src/ui/settings/update.rs:850`, change:
```rust
source: Some("user".to_string()),
```
to:
```rust
source: editor.source.clone().or_else(|| Some("user".to_string())),
```
This preserves the synced contact's source through the save path. New contacts (where `editor.source` is `None`) get `"user"` as before.

### Step 2: Add `db_upsert_contact_full` to core

In `crates/core/src/db/queries_extra/contacts.rs`, add a sync function that takes all contact fields and does the `INSERT ... ON CONFLICT(id) DO UPDATE` — same SQL as the current app-side `save_contact_inner`. This makes the upsert callable from the action module.

### Step 3: Define `ContactSaveInput` and implement `save_contact`

In `crates/core/src/actions/contacts.rs`:

1. Local DB save via `spawn_blocking` using `db_upsert_contact_full`.
2. If `source` is synced (not "user", not None): dispatch write-back.
3. Return `Success` if local + provider both succeeded, `LocalOnly` if local succeeded but provider failed/stubbed, `Failed` if local failed.

### Step 4: Implement provider write-back dispatch

```rust
async fn dispatch_write_back(
    ctx: &ActionContext,
    source: &str,
    email: &str,
    phone: Option<&str>,
    company: Option<&str>,
    notes: Option<&str>,
) -> Result<(), String> {
    match source {
        "jmap" => {
            let info = get_jmap_contact_server_info(&ctx.db, email.to_string()).await?;
            let Some(info) = info else { return Ok(()) };
            let client = JmapClient::from_account(
                &ctx.db, &info.account_id, &ctx.encryption_key,
            ).await?;
            jmap_contacts_push_update(&client, &info.server_id, phone, company, notes).await
        }
        "google" => {
            // Scaffolding ready (build_google_contact_update_body,
            // get_google_contact_server_info). HTTP PATCH not wired.
            Err("Google contact write-back not yet wired to HTTP".to_string())
        }
        "graph" => {
            Err("Graph contact write-back not yet wired to HTTP".to_string())
        }
        "carddav" => {
            Err("CardDAV contact write-back not implemented (PUT + vCard needed)".to_string())
        }
        _ => Ok(()),
    }
}
```

Unimplemented providers return `Err` — the action maps this to `LocalOnly { remote_error }`. This is honest: the local save succeeded, the provider was not notified.

### Step 5: Implement `delete_contact`

1. Look up contact's `source`, `server_id`, and `account_id` from the `contacts` table by local `id` (not email). This is the canonical identity for delete.
2. If synced and has `server_id`: dispatch provider delete (JMAP wired, others return `LocalOnly`).
3. Delete locally via `spawn_blocking`.
4. Return outcome.

For JMAP delete: construct `JmapClient` from the looked-up `account_id`, call `ContactCard/set` with `destroy`. (Extend or parallel `jmap_contacts_push_update` pattern.)

### Step 6: Register in `crates/core/src/actions/mod.rs`

```rust
pub mod contacts;
```

### Step 7: Migrate app handler

Replace `handle_save_contact`:
- Build `ContactSaveInput` from `ContactEntry` (direct field mapping).
- Call `actions::contacts::save_contact(ctx, input)`.
- Map outcome to contact list reload.

Replace `handle_delete_contact`:
- Call `actions::contacts::delete_contact(ctx, contact_id)`.
- Map outcome to contact list reload.

Delete `dispatch_provider_write_back` from the app crate.

### Step 8: Verify

- `cargo check --workspace`
- `cargo clippy -p ratatoskr-core -p app`
- Verify `dispatch_provider_write_back` no longer exists in the app crate.
- Verify JMAP write-back is actually called (log output shows JMAP push, not "not yet wired").
- Verify synced contact saves preserve source through the UI → handler → action path.
- Verify Google/Graph/CardDAV saves return `LocalOnly` (not `Success`).

## What This Produces

- `crates/core/src/actions/contacts.rs` — `save_contact()`, `delete_contact()`, `ContactSaveInput`, `dispatch_write_back()`
- `crates/core/src/db/queries_extra/contacts.rs` — `db_upsert_contact_full()` (extracted from app)
- Modified `crates/core/src/actions/mod.rs` — registers contacts module
- Modified `crates/app/src/handlers/contacts.rs` — delegates to action service, `dispatch_provider_write_back` deleted
- Modified `crates/app/src/ui/settings/update.rs` — preserves synced contact source on save

## Exit Criteria

1. `save_contact()` saves locally then dispatches write-back. JMAP write-back is functional — `jmap_contacts_push_update()` is called with a real `JmapClient`.
2. `delete_contact()` looks up contact identity by local `id` (not email). For synced JMAP contacts, dispatches `ContactCard/set destroy`. For local contacts, deletes locally only.
3. Google, Graph, and CardDAV write-back return `LocalOnly` with descriptive error — not fake `Success`.
4. Google/Graph/CardDAV delete return `LocalOnly` — contact is deleted locally but remains on server.
5. `dispatch_provider_write_back` no longer exists in the app crate.
6. Settings UI preserves synced contact `source` through the save path (the `source: "user"` bug is fixed).
7. Contact groups save/delete are unchanged (local-only, stay in app handler).
8. `db_upsert_contact_full` exists in core — the app crate no longer owns the only contact upsert SQL.
9. Workspace compiles and passes clippy.

## What Phase 2.6 Does NOT Do

- **Wire Google HTTP dispatch.** `build_google_contact_update_body` and `get_google_contact_server_info` exist. Wiring needs `GmailClient.patch_absolute()` or equivalent. Separate task. Google's body builder doesn't include notes (People API limitation — only phone and company are pushed).
- **Wire Graph HTTP dispatch.** Same — `build_graph_contact_update_body` and `get_graph_contact_server_info` exist. Needs `GraphClient.patch()`. Separate task. Graph's body builder also doesn't include notes.
- **Implement CardDAV PUT.** `CardDavClient` needs a `put_vcard()` method and vCard serialization. Separate task.
- **Contact create on provider.** Creating new contacts on the server (not just pushing edits to synced contacts) is not in scope. The current system only writes back edits to contacts that were synced from the server.
- **Contact group write-back.** Groups are local-only. No provider has a native equivalent that we sync.
