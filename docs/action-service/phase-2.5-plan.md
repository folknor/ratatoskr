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

All provider create/update functions return `Result<CalendarEventDto, String>` (defined in `crates/calendar/src/types.rs:23-52`). `CalendarEventDto` carries `remote_event_id`, `etag`, `summary`, timestamps, and all other event metadata the provider returns.

**Key differences from email actions:**
- Calendar writes are **not on `ProviderOps`**. Each provider exposes standalone async functions with provider-specific client types (`&GmailClient`, `&GraphClient`, `&JmapClient`, or `(db, key, account_id)` for CalDAV).
- The action service's `create_provider()` returns `Box<dyn ProviderOps>` — unusable for calendar operations. A second provider-resolution path (`create_calendar_provider`) is needed. This lives in `actions/calendar.rs` alongside the dispatch helpers, not in `actions/provider.rs`, because it's calendar-domain-specific.
- Provider APIs take different parameter shapes: Google/Graph take `serde_json::Value`, JMAP takes individual fields, CalDAV takes `serde_json::Value` + `etag`.
- **Google needs `calendar_remote_id` for all three operations** (create, update, delete). Graph and JMAP need it only for create. CalDAV needs it for create. All three action functions must look up the event's `calendar_id` → `calendars.remote_id` to have this available.

### What doesn't exist

- **No provider dispatch from app.** Events are local-only.
- **No `remote_event_id` mapping for locally-created events.** Locally-created events have `remote_event_id = NULL`. After provider dispatch, the returned ID must be stored.
- **No `calendar_remote_id` resolution.** Provider APIs need the calendar's remote ID (not the local `calendar_id`). The app handler has `event.calendar_id` (local) but the provider needs the `remote_id` from the `calendars` table.

## Design Decisions

### Calendar actions live in a separate module

The implementation-phases doc noted: *"Calendar and contact writes are different domains from email actions. They may warrant their own service modules."*

**Decision:** Calendar actions live in `core::actions::calendar`. They share `ActionContext` and `ActionOutcome` but have their own provider resolution (typed clients, not `ProviderOps`). This creates a second provider-resolution path alongside `create_provider()` — intentional duplication because the calendar domain uses fundamentally different client types.

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

**Create** is local-first: insert the event into the local DB immediately (instant UI feedback), then dispatch to the provider. If the provider succeeds, update the local row with `remote_event_id` from the returned `CalendarEventDto`. If the provider fails, return `LocalOnly` — the event exists locally but not on the server. The caller can surface this ("Event saved locally — not synced to server"). No automatic retry mechanism exists in Phase 2.5 — this is an honest `LocalOnly`, same semantics as email actions where local succeeded but remote didn't.

**Update** and **delete** are provider-first for events that have a `remote_event_id` (synced from server). The provider is the source of truth. For locally-created events without a `remote_event_id`, update/delete are local-only and return `Success`.

### Event metadata lookup: `calendar_remote_id` and `etag`

All three action functions need metadata beyond what the caller provides:

- **`calendar_remote_id`**: looked up from `calendars.remote_id` via the event's `calendar_id`. Needed by Google for all operations, by Graph/JMAP/CalDAV for create.
- **`remote_event_id`**: looked up from `calendar_events.remote_event_id`. Needed for update/delete provider dispatch. `NULL` means locally-created (no provider dispatch).
- **`etag`**: looked up from `calendar_events.etag`. Needed by CalDAV for optimistic concurrency on update/delete.

For **create**, the caller passes `calendar_id` → the action looks up `calendar_remote_id`.
For **update/delete**, the action looks up `remote_event_id`, `etag`, `calendar_id`, and then `calendar_remote_id` from the existing event row + calendars table.

### Event data serialization for provider APIs

Define a `CalendarEventInput` struct as the provider-agnostic interface:

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

Each provider dispatch branch serializes this to the format the provider expects. This conversion is internal to the action module.

### Completion message pattern

Calendar operates on the calendar view, not the thread list. Like email send, it doesn't fit `ActionCompleted` (which carries per-thread outcomes and toggle rollback).

**Decision:** Reuse the existing `CalendarMessage::EventSaved(Result<(), String>)` and `CalendarMessage::EventDeleted(Result<(), String>)` callbacks. The action service result is mapped to these: `Success`/`LocalOnly` → `Ok(())`, `Failed` → `Err(error)`. The existing completion handlers (close overlay, reload events) work unchanged.

This avoids adding more `Message` variants. The tradeoff: `LocalOnly` is reported as success to the existing handler (overlay closes, events reload). To surface the "saved locally, not synced" distinction, the action function can log a warning and the status bar can show a degraded confirmation. This is acceptable for Phase 2.5 — Phase 3 will introduce richer outcome reporting.

### `ActionOutcome` semantics

- **Create**: `Success` = local insert + provider dispatch both succeeded. `LocalOnly` = local insert succeeded, provider failed (event is local-only with `remote_event_id = NULL`). `Failed` = local insert failed.
- **Update (synced event)**: `Success` = provider + local both succeeded. `Failed` = provider failed.
- **Update (local-only event)**: `Success` = local update succeeded. No provider dispatch.
- **Delete (synced event)**: `Success` = provider + local both succeeded. `Failed` = provider failed.
- **Delete (local-only event)**: `Success` = local delete succeeded. No provider dispatch.

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

In `crates/core/src/actions/calendar.rs`.

### Step 2: Implement `create_calendar_provider`

Reads `provider` and `calendar_provider` from the `accounts` table. Same resolution logic as `calendar_sync_account_impl`: `gmail_api` or `calendar_provider = 'google_api'` → Google, `graph` → Graph, `caldav` or `calendar_provider = 'caldav'` → CalDAV, `jmap` → JMAP. Constructs the typed client using `GmailClient::from_account`, `GraphClient::from_account`, `JmapClient::from_account` (same as `create_provider` in `actions/provider.rs`).

### Step 3: Implement provider dispatch helpers

Private functions that take `CalendarProvider`, `&ActionContext` (for `db`, `encryption_key`), and the operation-specific parameters:

```rust
async fn dispatch_create(
    provider: &CalendarProvider,
    ctx: &ActionContext,
    calendar_remote_id: &str,
    input: &CalendarEventInput,
) -> Result<CalendarEventDto, String>

async fn dispatch_update(
    provider: &CalendarProvider,
    ctx: &ActionContext,
    calendar_remote_id: &str,
    remote_event_id: &str,
    input: &CalendarEventInput,
    etag: Option<&str>,
) -> Result<CalendarEventDto, String>

async fn dispatch_delete(
    provider: &CalendarProvider,
    ctx: &ActionContext,
    calendar_remote_id: &str,
    remote_event_id: &str,
    etag: Option<&str>,
) -> Result<(), String>
```

Each builds the provider-specific payload and calls the existing provider function. `calendar_remote_id` is available for all operations — Google needs it for update/delete, others may ignore it.

### Step 4: Implement `create_calendar_event`

1. Look up `calendar_remote_id` from `calendars` table via `calendar_id`.
2. Local DB insert via `create_calendar_event_sync()` — instant feedback. Captures the generated `event_id`.
3. Create calendar provider client. If provider creation fails, return `LocalOnly` (local insert already succeeded).
4. Call `dispatch_create`. On success, update local row: `UPDATE calendar_events SET remote_event_id = ?1, etag = ?2 WHERE id = ?3` with values from `CalendarEventDto`. Return `Success`.
5. On provider failure, return `LocalOnly { remote_error }`. The event exists locally with `remote_event_id = NULL`.

### Step 5: Implement `update_calendar_event`

1. Look up event's `remote_event_id`, `etag`, and `calendar_id` from `calendar_events` table.
2. If `remote_event_id` is `NULL` (locally-created, never synced): local-only update via `update_calendar_event_sync()`, return `Success`.
3. Look up `calendar_remote_id` from `calendars` table via `calendar_id`.
4. Create calendar provider client.
5. Call `dispatch_update` with `calendar_remote_id`, `remote_event_id`, `input`, `etag`.
6. On success, update local DB with returned `CalendarEventDto` metadata. Return `Success`.
7. On failure, return `Failed` (provider is source of truth for synced events — local not modified).

### Step 6: Implement `delete_calendar_event`

1. Look up event's `remote_event_id`, `etag`, and `calendar_id` from `calendar_events` table.
2. If `remote_event_id` is `NULL`: local-only delete via `delete_calendar_event_sync()`, return `Success`.
3. Look up `calendar_remote_id` from `calendars` table via `calendar_id`.
4. Create calendar provider client.
5. Call `dispatch_delete` with `calendar_remote_id`, `remote_event_id`, `etag`.
6. On success, delete locally via `delete_calendar_event_sync()`. Return `Success`.
7. On failure, return `Failed`.

### Step 7: Register in `crates/core/src/actions/mod.rs`

```rust
pub mod calendar;
```

### Step 8: Migrate app handler

**Field mapping** from `CalendarEventData` (app-side) to `CalendarEventInput` (action service):

```rust
let input = CalendarEventInput {
    title: event.title.clone(),                    // CalendarEventData.title → title
    description: event.description.clone(),        // .description → description
    location: event.location.clone(),              // .location → location
    start_time: start_ts,                          // computed from start_date + start_hour/minute
    end_time: end_ts,                              // computed from start_date + end_hour/minute
    is_all_day: event.all_day,                     // .all_day → is_all_day
    timezone: event.timezone.clone(),              // .timezone → timezone
    recurrence_rule: event.recurrence_rule.clone(),// .recurrence_rule → recurrence_rule
    availability: event.availability.clone(),      // .availability → availability
    visibility: event.visibility.clone(),          // .visibility → visibility
};
```

The `start_ts` / `end_ts` computation (via `calendar_data_to_timestamp`) stays in the app handler — it uses app-side types (`NaiveDate`, hour/minute accessors).

**`handle_save_event` becomes:**

```rust
fn handle_save_event(&mut self) -> Task<Message> {
    // ... extract event, compute timestamps, resolve account_id (same as now) ...
    let input = CalendarEventInput { /* field mapping above */ };

    let Some(ref action_ctx) = self.action_ctx else {
        return Task::none();
    };
    let ctx = action_ctx.clone();
    let aid = account_id.clone();

    if let Some(id) = event.id.clone() {
        // Update existing
        Task::perform(
            async move {
                let outcome = ratatoskr_core::actions::calendar::update_calendar_event(
                    &ctx, &aid, &id, input,
                ).await;
                outcome_to_result(outcome)
            },
            |r| Message::Calendar(Box::new(CalendarMessage::EventSaved(r))),
        )
    } else {
        // Create new
        let cal_id = event.calendar_id.clone().unwrap_or_default();
        Task::perform(
            async move {
                let outcome = ratatoskr_core::actions::calendar::create_calendar_event(
                    &ctx, &aid, &cal_id, input,
                ).await;
                outcome_to_result(outcome)
            },
            |r| Message::Calendar(Box::new(CalendarMessage::EventSaved(r))),
        )
    }
}

/// Map ActionOutcome to the Result<(), String> that CalendarMessage::EventSaved expects.
fn outcome_to_result(outcome: ActionOutcome) -> Result<(), String> {
    match outcome {
        ActionOutcome::Success | ActionOutcome::LocalOnly { .. } => Ok(()),
        ActionOutcome::Failed { error } => Err(error),
    }
}
```

`LocalOnly` maps to `Ok(())` — the overlay closes and events reload. The event is visible locally. A warning is logged by the action function. Phase 3 can add richer outcome reporting.

**Delete handler becomes similar** — calls `actions::calendar::delete_calendar_event()`, maps outcome to `CalendarMessage::EventDeleted(Result<(), String>)`.

### Step 9: Verify

- `cargo check --workspace`
- `cargo clippy -p ratatoskr-core -p app`
- Verify `handle_save_event` no longer calls `db.create_calendar_event()` or `db.update_calendar_event()` directly.
- Manual smoke test: create an event, verify it appears on the provider's calendar.

## What This Produces

- `crates/core/src/actions/calendar.rs` — `create_calendar_event()`, `update_calendar_event()`, `delete_calendar_event()`, `CalendarEventInput`, `CalendarProvider`, `create_calendar_provider()`, provider dispatch helpers
- Modified `crates/core/src/actions/mod.rs` — registers calendar module
- Modified `crates/app/src/handlers/calendar.rs` — delegates to action service via existing `CalendarMessage` completion callbacks

## Exit Criteria

1. `create_calendar_event()` inserts locally (instant feedback) then dispatches to the provider. Provider-returned `remote_event_id` and `etag` stored on the local row. Returns `LocalOnly` (not `Success`) if provider fails.
2. `update_calendar_event()` looks up `remote_event_id`, `etag`, and `calendar_remote_id`. Dispatches to provider (if synced) then updates locally. Local-only events updated locally without provider dispatch.
3. `delete_calendar_event()` looks up `remote_event_id`, `etag`, and `calendar_remote_id`. Dispatches to provider (if synced) then deletes locally. Local-only events deleted locally without provider dispatch.
4. `calendar_remote_id` is resolved for all operations — Google update/delete needs it.
5. All four providers are wired: Google (`GmailClient`), Graph (`GraphClient`), JMAP (`JmapClient`), CalDAV (`db + encryption_key + account_id`).
6. The app handler delegates to the action service. Existing `CalendarMessage::EventSaved`/`EventDeleted` completion callbacks are reused.
7. Workspace compiles and passes clippy.

## What Phase 2.5 Does NOT Do

- **Attendee/reminder write-back.** `CalendarEventInput` carries the core event fields. Attendees and reminders are managed by separate DB tables and are not yet propagated to providers.
- **Recurrence expansion.** `recurrence_rule` is stored and passed through to providers. Instance generation and series editing are separate features.
- **Etag conflict resolution.** CalDAV and Graph support optimistic concurrency via etag. The action function passes the etag through but does not handle 412 Precondition Failed with retry/merge. That's Phase 3 territory.
- **IMAP calendar.** IMAP has no calendar API. IMAP accounts that use CalDAV for calendar are handled via the `calendar_provider` column.
- **Calendar RSVP.** Responding to event invitations is a separate write operation.
- **Automatic retry for failed creates.** `LocalOnly` events (local-only due to provider failure) have no retry mechanism. A future sync-up worker or Phase 3 retry infrastructure could handle this.
