# Action Service: Phase 2.5 Detailed Plan

## Goal

Wire calendar event mutations through the action service so that creating, updating, and deleting events reaches the provider. Today, all three operations are local-only — they write to the `calendar_events` table and never notify the provider. Events created in Ratatoskr don't appear on the server; edits don't propagate; deletions don't sync.

## Current State

### What exists

**App-side handlers** (`crates/app/src/handlers/calendar.rs:360-414`):
- `handle_save_event()` — branches on `event.id`: if `Some` → `db.update_calendar_event()`, else → `db.create_calendar_event()`.
- `CalendarMessage::DeleteEvent(event_id)` → `db.delete_calendar_event(event_id)`.
- Both are local DB only. No provider dispatch.

**Local DB functions** (`crates/core/src/db/queries_extra/calendars.rs:560-646`):
- `create_calendar_event_sync(conn, params)` — inserts with generated UUID, `google_event_id = NULL`, `remote_event_id = NULL`.
- `update_calendar_event_sync(conn, event_id, params)` — updates mutable fields.
- `delete_calendar_event_sync(conn, event_id)` — cascades to `calendar_attendees`, `calendar_reminders`, then deletes event.

**Provider write APIs** — all exist and are implemented but never called from app code:

| Provider | Create | Update | Delete |
|----------|--------|--------|--------|
| **Google** | `google_calendar_create_event_impl(client, db, calendar_remote_id, event_json)` | `google_calendar_update_event_impl(client, db, calendar_remote_id, remote_event_id, event_json)` | `google_calendar_delete_event_impl(client, db, calendar_remote_id, remote_event_id)` |
| **Graph** | `graph_calendar_create_event_impl(client, db, calendar_remote_id, event_json)` | `graph_calendar_update_event_impl(client, db, remote_event_id, event_json)` | `graph_calendar_delete_event_impl(client, db, remote_event_id)` |
| **JMAP** | `create_event_remote(client, calendar_remote_id, title, desc, location, start, end, is_all_day)` | `update_event_remote(client, event_remote_id, title, desc, location, start, end, is_all_day)` | `delete_event_remote(client, event_remote_id)` |
| **CalDAV** | `caldav_create_event_impl(db, key, account_id, calendar_remote_id, event_json)` | `caldav_update_event_impl(db, key, account_id, remote_event_id, event_json, etag)` | `caldav_delete_event_impl(db, key, account_id, remote_event_id, etag)` |

**Key differences from email actions:**
- Calendar writes are **not on `ProviderOps`**. Each provider exposes standalone async functions with provider-specific client types (`&GmailClient`, `&GraphClient`, `&JmapClient`, or `(db, key, account_id)` for CalDAV).
- The action service's `create_provider()` returns `Box<dyn ProviderOps>` — unusable for calendar operations.
- Provider APIs take different parameter shapes: Google/Graph take `serde_json::Value`, JMAP takes individual fields, CalDAV takes `serde_json::Value` + `etag`.

### What doesn't exist

- **No provider dispatch from app.** Events are local-only.
- **No `remote_event_id` mapping for locally-created events.** Locally-created events have `remote_event_id = NULL`. After provider dispatch, the returned ID must be stored.
- **No `calendar_remote_id` resolution.** Provider APIs need the calendar's remote ID (not the local `calendar_id`). The app handler has `event.calendar_id` (local) but the provider needs the `remote_id` from the `calendars` table.

## Design Decisions

### Calendar actions live in a separate module, not in `core::actions`

The implementation-phases doc noted: *"Calendar and contact writes are different domains from email actions. They may warrant their own service modules (`core::calendar_actions`, `core::contact_actions`) rather than expanding `core::actions` into a grab-bag."*

**Decision:** Calendar actions live in `core::actions::calendar`. They share `ActionContext` and `ActionOutcome` but have their own provider resolution (typed clients, not `ProviderOps`).

### Calendar-specific provider resolution

A new function `create_calendar_provider()` resolves the account's calendar provider and returns an enum that carries the typed client:

```rust
enum CalendarProvider {
    Google(GmailClient),
    Graph(GraphClient),
    Jmap(JmapClient),
    CalDav { account_id: String },
}
```

CalDAV doesn't need a pre-constructed client — it takes `(db, encryption_key, account_id)` and constructs its own HTTP client internally. The enum carries just the `account_id` for CalDAV.

This function reads `provider` and `calendar_provider` from the `accounts` table (same logic as `calendar_sync_account_impl` in `crates/calendar/src/sync.rs`), then constructs the appropriate client.

### Local-first for create, provider-first for update/delete

**Create** is local-first: insert the event into the local DB immediately (instant UI feedback), then dispatch to the provider. If the provider succeeds, update the local row with `remote_event_id`. If it fails, the event exists locally but not on the server — the user sees it, and a future sync-up could retry.

**Update** and **delete** are provider-first for events that have a `remote_event_id` (synced from server). The provider is the source of truth. For locally-created events without a `remote_event_id`, update/delete are local-only.

This hybrid approach matches user expectations:
- Creating an event should appear immediately in the calendar view.
- Editing a server-synced event should propagate to the server.
- Deleting a server-synced event should remove it from the server.

### `calendar_remote_id` resolution

Provider APIs need the calendar's `remote_id`, not the local `calendar_id`. The action function looks this up from the `calendars` table: `SELECT remote_id FROM calendars WHERE id = ?1 AND account_id = ?2`.

### Event data serialization for provider APIs

Google and Graph take `serde_json::Value`. JMAP takes individual fields. CalDAV takes `serde_json::Value`. Rather than the action function building three different JSON shapes, define a `CalendarEventInput` struct with all fields, and let each provider dispatch branch serialize it appropriately:

```rust
pub struct CalendarEventInput {
    pub title: String,
    pub description: String,
    pub location: String,
    pub start_time: i64,
    pub end_time: i64,
    pub is_all_day: bool,
    pub timezone: Option<String>,
    pub recurrence_rule: Option<String>,
    pub availability: Option<String>,
    pub visibility: Option<String>,
}
```

The Google/Graph branches convert this to `serde_json::Value` using provider-specific field names. The JMAP branch passes individual fields. CalDAV converts to its own JSON shape. This conversion logic lives in the action module, not in the provider crates (which already have their own input formats).

### `ActionOutcome` semantics

Same as folder operations (provider-first for update/delete):
- `Success` = provider succeeded (for events with `remote_event_id`) or local-only write succeeded (for events without).
- `Failed` = provider or local DB failed.
- For create: `Success` means local insert succeeded. Provider dispatch is best-effort — if it fails, the event is local-only until sync retries. This matches the immediate-feedback expectation.

## Action Function Signatures

```rust
// crates/core/src/actions/calendar.rs

pub async fn create_calendar_event(
    ctx: &ActionContext,
    account_id: &str,
    calendar_id: &str,
    input: CalendarEventInput,
) -> ActionOutcome

pub async fn update_calendar_event(
    ctx: &ActionContext,
    account_id: &str,
    event_id: &str,
    input: CalendarEventInput,
) -> ActionOutcome

pub async fn delete_calendar_event(
    ctx: &ActionContext,
    account_id: &str,
    event_id: &str,
) -> ActionOutcome
```

## Implementation Steps

### Step 1: Define `CalendarEventInput` and `CalendarProvider`

In `crates/core/src/actions/calendar.rs`:

```rust
/// Provider-agnostic input for calendar event create/update.
#[derive(Debug, Clone)]
pub struct CalendarEventInput {
    pub title: String,
    pub description: String,
    pub location: String,
    pub start_time: i64,
    pub end_time: i64,
    pub is_all_day: bool,
    pub timezone: Option<String>,
    pub recurrence_rule: Option<String>,
    pub availability: Option<String>,
    pub visibility: Option<String>,
}
```

### Step 2: Implement `create_calendar_provider`

Reads the account's calendar provider from the DB and constructs the typed client. Same provider-resolution logic as `calendar_sync_account_impl`.

### Step 3: Implement provider dispatch helpers

Private functions that take `CalendarProvider` + `CalendarEventInput` and call the right provider API:

- `dispatch_create(provider, db, calendar_remote_id, input)` — returns `Result<CalendarEventDto, String>`
- `dispatch_update(provider, db, remote_event_id, input, etag)` — returns `Result<CalendarEventDto, String>`
- `dispatch_delete(provider, db, remote_event_id, etag)` — returns `Result<(), String>`

Each builds the provider-specific payload (JSON for Google/Graph/CalDAV, individual fields for JMAP) and calls the existing provider function.

### Step 4: Implement `create_calendar_event`

1. Look up `calendar_remote_id` from `calendars` table.
2. Local DB insert via `create_calendar_event_sync()` — instant feedback.
3. Create calendar provider client.
4. Dispatch to provider. On success, update local row with `remote_event_id` from the `CalendarEventDto`.
5. Return `Success` regardless of provider result (local insert is the gate for create).

### Step 5: Implement `update_calendar_event`

1. Look up event's `remote_event_id` and `etag` from `calendar_events` table.
2. If `remote_event_id` is `NULL` (locally-created, never synced): local-only update, return `Success`.
3. If `remote_event_id` exists: dispatch to provider first, then update local DB with returned metadata.
4. Return `Success` if provider succeeded (or if local-only), `Failed` if provider failed.

### Step 6: Implement `delete_calendar_event`

1. Look up event's `remote_event_id` and `etag` from `calendar_events` table.
2. If `remote_event_id` is `NULL`: local-only delete, return `Success`.
3. If `remote_event_id` exists: dispatch to provider first, then delete locally.
4. Return `Success` if provider succeeded (or if local-only), `Failed` if provider failed.

### Step 7: Register in `crates/core/src/actions/mod.rs`

```rust
pub mod calendar;
```

Re-export `CalendarEventInput` from actions.

### Step 8: Migrate app handler

Replace `handle_save_event()`:
- Build `CalendarEventInput` from `CalendarEventData` (same field mapping that currently builds `LocalCalendarEventParams`).
- Call `actions::calendar::create_calendar_event()` or `actions::calendar::update_calendar_event()` depending on `event.id`.
- On completion, reload calendar events (same as current `EventSaved` handler).

Replace `DeleteEvent` handler:
- Call `actions::calendar::delete_calendar_event()`.
- On completion, reload calendar events.

### Step 9: Verify

- `cargo check --workspace`
- `cargo clippy -p ratatoskr-core -p app`
- Verify `handle_save_event` no longer calls `db.create_calendar_event()` or `db.update_calendar_event()` directly.
- Manual smoke test: create an event, verify it appears on the provider's calendar.

## What This Produces

- `crates/core/src/actions/calendar.rs` — `create_calendar_event()`, `update_calendar_event()`, `delete_calendar_event()`, `CalendarEventInput`, `CalendarProvider`, provider dispatch helpers
- Modified `crates/core/src/actions/mod.rs` — registers calendar module
- Modified `crates/app/src/handlers/calendar.rs` — delegates to action service
- Modified `crates/app/src/db/calendar.rs` — async wrappers may become unused (check)

## Exit Criteria

1. `create_calendar_event()` inserts locally then dispatches to the provider. Provider-returned `remote_event_id` is stored on the local row.
2. `update_calendar_event()` dispatches to the provider (if `remote_event_id` exists) then updates locally. Local-only events are updated locally without provider dispatch.
3. `delete_calendar_event()` dispatches to the provider (if `remote_event_id` exists) then deletes locally. Local-only events are deleted locally without provider dispatch.
4. All three providers are wired: Google (via `GmailClient`), Graph (via `GraphClient`), JMAP (via `JmapClient`), CalDAV (via `db + encryption_key + account_id`).
5. The app handler delegates to the action service — no direct `calendar_events` DB writes for create/update/delete.
6. `CalendarEventInput` provides a provider-agnostic interface — provider-specific serialization is internal to the action module.
7. Workspace compiles and passes clippy.

## What Phase 2.5 Does NOT Do

- **Attendee/reminder write-back.** `CalendarEventInput` carries the core event fields. Attendees and reminders are managed by separate DB tables (`calendar_attendees`, `calendar_reminders`) and are not yet propagated to providers.
- **Recurrence expansion.** `recurrence_rule` is stored and passed through to providers. Instance generation and series editing (this-event vs all-events) are separate features.
- **Etag conflict resolution.** CalDAV and Graph support optimistic concurrency via etag. The action function passes the etag through but does not handle 412 Precondition Failed with retry/merge. That's Phase 3 territory.
- **IMAP calendar.** IMAP has no calendar API. IMAP accounts that use CalDAV for calendar are handled via the `calendar_provider` column on the account.
- **Calendar RSVP.** Responding to event invitations is a separate write operation with different provider APIs.
