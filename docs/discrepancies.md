# Discrepancies: db writer/reader boundary slice review

Findings from the review of the unstaged work that landed as commit
"db boundary: route calendar sync + pending retry through writer pool"
on 2026-05-17, on top of `729eabe4`. The commit message references a
"gap-closing commit" follow-up; this doc is the authoritative checklist
for that follow-up plus the longer-tail items.

Items are grouped by severity. Each entry names the file and line range
so a follow-up agent can act on it directly.

## Must-fix (correctness)

### Silent data loss on CalDAV events without DTSTART

`crates/calendar/src/sync.rs:823-828`

When `upsert_caldav_parsed_event` encounters a VEVENT with no DTSTART
it returns `Ok(google_event_id)` without pushing the key into
`seen_keys`. On the next sync iteration `reap_orphan_overrides` deletes
the previously-stored row, even if the only mutation was a transient
parser hiccup. Either:

- Push the key into `seen_keys` before returning (preserves the row).
- Explicitly delete the row at skip time (intentional purge).

Currently the row gets purged silently, which is the worst of the two.

### JMAP calendar persist path still owns shared-table SQL

`crates/jmap/src/calendar_sync/mod.rs:67-101, 152-173, 226-304, 374-390`
`crates/jmap/src/calendar_sync/persist.rs:56-136, 295-305, 312-363`

`sync_calendar_list`, `sync_all_events`, `sync_events_delta`,
`sync_calendars`, `persist_jmap_event[_record]`,
`delete_event_by_jmap_id`, `upsert_calendar` are still `pub` and
execute INSERT/UPDATE/DELETE on shared tables through
`ReadDbState::with_conn`. No remaining callers after this slice (the
new path in `crates/calendar/src/jmap.rs` replaces them) but they are
exactly the trap the boundary work exists to prevent. Delete them.

### Raw shared-table SQL inside the service crate

`crates/service/src/actions/star.rs:36-44`

Runs `UPDATE messages SET is_starred = ?1 ...` directly via
`tx.execute(...)`. Shared-table SQL belongs in `db` behind a typed
helper. Move next to `set_thread_starred`.

## Atomicity regressions

Writes that used to share a single `conn.lock()` are now multiple
separate writer-pool round-trips. A crash between round-trips leaves
the DB partially updated.

### `persist_jmap_calendar_event`

`crates/calendar/src/jmap.rs:106-160`

Commits the `upsert_calendar_event_row` transaction at line 136, then
runs `replace_event_attendees` and `replace_event_reminders` against
the bare conn afterwards. A crash between commit and the attendee/
reminder writes leaves the event row with stale attendees/reminders.
The pre-migration `persist_jmap_event_record` did all three inside one
`with_conn` closure. Restore by either:

- Wrapping all three writes in a single `unchecked_transaction()` and
  committing once at the end, or
- Adding a `WriteTxn`-taking variant of the attendee/reminder helpers
  and passing one `WriteTxn` through.

### `update_calendar_event` optimistic update + etag write

`crates/calendar/src/actions.rs:583-622`

The optimistic update path does the local UPDATE in `with_conn_mapped`,
then a separate `with_write_mapped` for the etag. Two writer-pool
round-trips, not atomic. Fold them into one closure.

### CalDAV per-resource events + attendees + reminders + map

`crates/calendar/src/sync.rs:744-767, 778-782, 906-916`

`sync_caldav_calendar_events` performs roughly three separate
`with_write` calls per resource (event upsert, attendees, reminders)
plus an `upsert_caldav_event_map` and `reap_orphan_overrides`, none
atomic with each other. Pass `&WriteTxn` through and commit once per
resource.

## Boundary violations still present

### Writer helpers still typed against `&rusqlite::Connection`

`crates/db/src/db/queries_extra/calendar_contacts_writes.rs:619-847`

These helpers were ported with the rest of the slice but kept the raw
`&rusqlite::Connection` signature instead of `&WriteConn` / `&WriteTxn`:

- `upsert_contact_group`
- `delete_contact_group_members`
- `insert_contact_group_member_email`
- `delete_contact_group_by_id`
- `delete_contact_groups_for_account_by_source`
- `list_contact_groups_for_account_by_source` (also: this is a read, not
  a write, and does not belong in a "writes" module)
- `upsert_message_reaction`
- `upsert_message_reaction_update_type`
- `delete_message_reaction`
- `upsert_seen_address_google_other`
- `delete_seen_address_google_other`

Tightening these to `&WriteConn` (and moving the read to a read module)
catches the leak below at compile time.

### Reader handle writing through a typed-by-name escape hatch

`crates/gmail/src/contacts/other_contacts.rs:23`

`sync_google_other_contacts` takes `&ReadDbState` and proceeds to call
`upsert_seen_address_google_other` and `delete_seen_address_google_other`
against the borrowed connection. Compiles today because those helpers
still take raw `&rusqlite::Connection`; will fail to compile once
they take `&WriteConn`. Migrate this call site as part of the
tightening above.

### JMAP protocol-state SQL inlined instead of using the shared helper

`crates/calendar/src/jmap.rs:164-208`

`save_jmap_calendar_sync_state` / `load_jmap_calendar_sync_state`
re-implement `sync_state::save_jmap_sync_state` /
`load_jmap_sync_state` (`crates/sync/src/state.rs:36-100`) with inline
raw SQL against `jmap_sync_state`. The shared helper is what every
other JMAP path uses (`provider-sync/src/jmap/sync/mod.rs`,
`jmap/src/contacts_sync.rs`, the legacy
`calendar_sync/persist.rs:312-328`). Two divergent copies of the same
JMAP-state SQL is a drift hazard. Consolidate.

## Dead code that will cause regressions

### Pending-ops async wrappers typed against `&ReadDbState`

`crates/db/src/db/pending_ops.rs:68-90, 168-176, 192-195, 207-222, 241-244, 396-399`

`db_pending_ops_enqueue`, `db_pending_ops_update_status`,
`db_pending_ops_delete`, `db_pending_ops_cancel_for_resource`,
`db_pending_ops_increment_retry`, `db_pending_ops_recover_executing`
still take `&ReadDbState` and execute writes through it. **Zero
remaining callers** after this slice. This is exactly the bug this
slice fixed: the next caller that picks them up reopens the
respawn-persistence failure. Delete or retype to `&WriteDbState`.

`db_pending_ops_compact`, `db_pending_ops_clear_failed`,
`db_pending_ops_retry_failed`, `db_pending_ops_recover_executing` are
in the same bucket.

### Duplicate calendar `_sync` helpers in two locations

`crates/db/src/db/queries_extra/calendars/crud.rs`

Still exposes `upsert_calendar_sync`, `upsert_calendar_event_sync`,
`replace_event_attendees_sync`, `replace_event_reminders_sync`,
`save_calendar_sync_token_sync`, `load_calendar_sync_token_sync`,
`delete_event_by_account_remote_id_sync`. These now duplicate the
typed helpers in `calendar_contacts_writes.rs`. The only remaining
callers are the to-be-deleted JMAP persistence path
(`crates/jmap/src/calendar_sync/persist.rs:9-10, 77, 122, 132, 303, 351`).
Delete after that path is gone, or a future contributor will add a
third copy because both surfaces exist.

### Dead exemption in the lockdown grep

`crates/db-read-lockdown/tests/lockdown.rs:139`

`db_read_raw_rusqlite_access_is_quarantined` skips `raw.rs`, but
`crates/db-read/src/` no longer contains a `raw.rs` file. Drop the
special case so the next person reading the test does not assume the
file exists.

## Lockdown coverage gaps

`crates/db-read-lockdown/tests/lockdown.rs`

The trybuild cases exercise `ReadConn::execute`, `ReadConn::transaction`,
`ReadConn::unchecked_transaction`, and `ReadStatement::execute`. Three
useful additions:

1. `compile_fail` proof that `use db_read::WriteConn` (and
   `WriterPool`, `WriteTxn`) does not resolve. The existing
   `db_read_public_surface_does_not_reexport_rusqlite` grep at line 92
   covers the indirect rusqlite leak, but a direct `compile_fail` is
   more durable.
2. `compile_fail` of an `UPDATE ... RETURNING` issued through
   `ReadConn::query_row`. The routing through `prepare` is in
   `lib.rs:176` but no test verifies the gate fires.
3. A `compile_fail` that a writer borrowed via `with_write` cannot
   escape its closure.

## Smaller findings

### Upsert helpers do redundant round-trips

`crates/db/src/db/queries_extra/calendar_contacts_writes.rs:53-87, 155-213`

`upsert_discovered_calendar` and `upsert_calendar_event_row` follow a
"SELECT id, generate UUID, INSERT ON CONFLICT, SELECT id again"
pattern. Replace with `INSERT ... ON CONFLICT ... DO UPDATE ...
RETURNING id` to collapse to one round-trip and remove the wasted UUID
generation.

### Empty-string ETag persistence

`crates/calendar/src/sync.rs:716`

`etag_map.get(uri.as_str()).unwrap_or(&"").to_string()` silently coerces
a missing ETag into the empty string, which then mismatches the
server's real ETag next sync and forces a re-fetch. Normalize missing
to `None`.

### `representative_uid` derived from first parsed event

`crates/calendar/src/sync.rs:744`

For resources whose iCalendar blob orders an override before the
master, `caldav_event_map.event_uid` holds the override's UID.
Functionally equivalent per RFC 5545 (UID is the same per resource)
but worth a comment explaining the assumption.

### Three different defaults for missing `end_time`

`crates/calendar/src/caldav/mod.rs:418-423` (post-PUT optimistic),
matching read path at the same line, and parsed path at line 829
(`start_time`, zero-length). Pick one.

### `schedule.len() - 1` can underflow

`crates/db/src/db/pending_ops.rs:275`

`db_pending_ops_increment_retry_sync` panics if a future
`BACKOFF_*` constant is empty. `schedule.last().copied().unwrap_or(default)`
is safer. Latent today because every schedule is non-empty.

### `ActionContext` vs `CalendarActionContext` field naming asymmetry

`crates/action-types/src/context.rs`

`ActionContext.db` is `ReadDbState`, but `CalendarActionContext.db` is
`WriteDbState` (with `read_db` for reading). Same field name, opposite
meaning depending on context type. A reader skimming `ctx.db.with_conn(...)`
has to know which context type they are holding to know whether they
are writing or reading. Rename `CalendarActionContext.db` to `write_db`
to mirror `ActionContext`.

### `send_identity.rs` transitional shim

`crates/db/src/db/queries_extra/send_identity.rs`

Keeps the old `&Connection` versions (`get_send_identities`,
`get_all_send_identity_emails`) and adds parallel `_read` variants
taking `&ReadConn`. Fine for one slice, but no follow-up task or lint
pins the deletion of the legacy ones. Track as a follow-up or convert
in the gap-closing commit.

### `WriteDbState` escape hatches

`crates/service-state/src/lib.rs`

`WriteDbState::from_arc` and the untyped `with_conn*` shims remain
"while the writer helper migration is in flight". They are the open
door for callers to bypass `WriteConn`. Either `#[deprecated]` them or
include a "remaining callers: N" count in the next commit message so
the work is bounded.

### `WriteConn::as_read` has no callers

`crates/db/src/db/mod.rs:516`

Defined but unreferenced under `crates/`. Likely scaffolding for an
upcoming move; remove or note its intended caller.

## What this slice did NOT change (in case future-you wonders)

- `crates/core/src/auto_responses.rs` is intentionally kept; only the
  unused `db_get_auto_response` / `db_upsert_auto_response` /
  `ExternalAudience::as_str` / `parse` were trimmed. The Graph / Gmail
  / JMAP fetch+push helpers and `any_auto_response_active` remain.
- `crates/core/src/bimi.rs` and `cloud_attachments.rs` lost large
  amounts of dead code but the surviving surface is still used by the
  app.
- `crates/db-read/src/lib.rs` re-exports were narrowed, not deleted.
  The named items inside `queries` and `queries_extra` are still
  reachable; only the wholesale `pub use writer_db::db::queries{,_extra}`
  globs were removed.
