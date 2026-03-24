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
- Phone, company, and notes are the only fields pushed to providers.

### What doesn't exist

- **No contact delete dispatches to providers.** Delete is local-only. Server contacts are orphaned.
- **No typed client access in the write-back path.** `dispatch_provider_write_back` takes `&Arc<Db>`, not typed provider clients. JMAP, Google, and Graph all need their respective clients for HTTP calls.
- **No CardDAV PUT.** `CardDavClient` has PROPFIND and REPORT but no PUT method. Needs vCard generation.

## Design Decisions

### Contact actions live in `core::actions::contacts`

Unlike calendar (which lives in a separate crate due to circular dependency), the contact write-back infrastructure lives in `core` already: `core::contacts::sync_google`, `core::contacts::sync_graph`, `core::carddav`. The JMAP piece is in the `jmap` crate, which core depends on. So `core::actions::contacts` has access to everything it needs. No new crate dependency required.

### Only JMAP is fully wired; others are dispatch stubs

JMAP's `jmap_contacts_push_update()` is complete and ready. Google and Graph have body builders and server-info lookups but no HTTP call — the action function calls the existing scaffolding and stubs the final HTTP step with a log + `Ok(())` (same as today, but now the stub is in core instead of the app crate). CardDAV returns a descriptive error.

This is the same strategy as Phase 2.4 (folder CRUD for IMAP) — define the service API now, wire what's ready, and leave stubs for what isn't. When HTTP dispatch is added for Google/Graph, it goes through the existing action function.

### Save is local-first, write-back is best-effort

Same pattern as current code: save locally first (instant UI feedback), then attempt provider write-back. Provider failure does not roll back the local save. The action returns `Success` if local save succeeded, `LocalOnly` if local succeeded but provider failed, `Failed` if local failed.

### Delete dispatches to provider for synced contacts

Currently delete is local-only. The action function should delete on the provider when the contact has a `server_id` and a known source. Provider APIs:
- **Google**: `DELETE /v1/{resourceName}:deleteContact`
- **Graph**: `DELETE /me/contacts/{id}`
- **JMAP**: `ContactCard/set` with `destroy`
- **CardDAV**: `DELETE {uri}` with `If-Match: {etag}`

For Phase 2.6: only JMAP delete is wired (extend `jmap_contacts_push_update` pattern). Others are stubs.

### Contact groups are local-only

Group save/delete don't involve providers — groups are a local organization concept. They stay in the app handler and don't go through the action service.

### No new completion message type needed

Contact save/delete operate from the Settings panel. The existing `SettingsMessage::ContactsLoaded(Result<Vec<ContactEntry>, String>)` callback works — the action result is mapped to a contact list reload, same as today.

## Action Function Signatures

```rust
// crates/core/src/actions/contacts.rs

/// Save a contact locally, then dispatch write-back to the provider.
/// Display name is local-only — only phone, company, notes are pushed.
pub async fn save_contact(
    ctx: &ActionContext,
    entry: ContactEntry,
) -> ActionOutcome

/// Delete a contact locally. For synced contacts with a server_id,
/// also dispatches delete to the provider.
pub async fn delete_contact(
    ctx: &ActionContext,
    contact_id: &str,
) -> ActionOutcome
```

`ContactEntry` is the existing app-side type. The action function needs it for the local save (which uses `save_contact_inner`). To avoid importing app-crate types into core, define a `ContactSaveInput` in core that carries the same fields.

## Implementation Steps

### Step 1: Define `ContactSaveInput`

```rust
pub struct ContactSaveInput {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub email2: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub notes: Option<String>,
    pub account_id: Option<String>,
    pub source: Option<String>,
}
```

The app handler maps `ContactEntry` → `ContactSaveInput` before calling the action.

### Step 2: Implement provider write-back dispatch

Move `dispatch_provider_write_back` from the app crate to `core::actions::contacts`. The function gains access to `ActionContext` (and thus `encryption_key`, `DbState`), which provides the missing piece for constructing typed clients.

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
            let client = JmapClient::from_account(&ctx.db, &info.account_id, &ctx.encryption_key).await?;
            jmap_contacts_push_update(&client, &info.server_id, phone, company, notes).await
        }
        "google" => {
            // Scaffolding ready, HTTP not wired.
            // When wired: get_google_contact_server_info → build body → GmailClient.patch()
            log::info!("Google contact write-back for {email} (HTTP not yet wired)");
            Ok(())
        }
        "graph" => {
            log::info!("Graph contact write-back for {email} (HTTP not yet wired)");
            Ok(())
        }
        "carddav" => {
            log::info!("CardDAV contact write-back for {email} (PUT not implemented)");
            Ok(())
        }
        "user" | _ => Ok(()),
    }
}
```

### Step 3: Implement `save_contact`

1. Local DB save via `spawn_blocking` (reuse existing `save_contact_inner` logic or equivalent core function).
2. If source is synced, dispatch write-back. Return `LocalOnly` on write-back failure.
3. Return `Success` if both succeeded, `Failed` if local save failed.

### Step 4: Implement `delete_contact`

1. Look up contact's `source`, `server_id`, and account mapping from the DB.
2. If synced and has `server_id`: dispatch provider delete (JMAP wired, others stubbed).
3. Delete locally via `spawn_blocking`.
4. Return outcome.

### Step 5: Register in `crates/core/src/actions/mod.rs`

```rust
pub mod contacts;
```

### Step 6: Migrate app handler

Replace `handle_save_contact`:
- Build `ContactSaveInput` from `ContactEntry`.
- Call `actions::contacts::save_contact(ctx, input)`.
- On completion, reload contact list (same as current `ContactsLoaded` callback).

Replace `handle_delete_contact`:
- Call `actions::contacts::delete_contact(ctx, contact_id)`.
- On completion, reload contact list.

Delete `dispatch_provider_write_back` from the app crate.

### Step 7: Verify

- `cargo check --workspace`
- `cargo clippy -p ratatoskr-core -p app`
- Verify `dispatch_provider_write_back` no longer exists in the app crate.
- Verify JMAP write-back is actually called (log output shows JMAP push, not "not yet wired").

## What This Produces

- `crates/core/src/actions/contacts.rs` — `save_contact()`, `delete_contact()`, `ContactSaveInput`, `dispatch_write_back()`
- Modified `crates/core/src/actions/mod.rs` — registers contacts module
- Modified `crates/app/src/handlers/contacts.rs` — delegates to action service, `dispatch_provider_write_back` deleted

## Exit Criteria

1. `save_contact()` saves locally then dispatches write-back. JMAP write-back is functional — `jmap_contacts_push_update()` is called with a real `JmapClient`.
2. `delete_contact()` deletes locally and dispatches provider delete for synced contacts. JMAP delete is functional.
3. Google, Graph, and CardDAV write-back branches exist as stubs with log messages — same behavior as today but the code lives in core, not the app crate.
4. `dispatch_provider_write_back` no longer exists in the app crate.
5. Contact groups save/delete are unchanged (local-only, stay in app handler).
6. Workspace compiles and passes clippy.

## What Phase 2.6 Does NOT Do

- **Wire Google HTTP dispatch.** `build_google_contact_update_body` and `get_google_contact_server_info` exist. Wiring needs `GmailClient.patch_absolute()` or equivalent. Separate task.
- **Wire Graph HTTP dispatch.** Same — `build_graph_contact_update_body` and `get_graph_contact_server_info` exist. Needs `GraphClient.patch()`. Separate task.
- **Implement CardDAV PUT.** `CardDavClient` needs a `put_vcard()` method and vCard serialization. Separate task.
- **Contact create on provider.** Creating new contacts on the server (not just pushing edits to synced contacts) is not in scope. The current system only writes back edits to contacts that were synced from the server.
- **Contact group write-back.** Groups are local-only. No provider has a native equivalent that we sync.
