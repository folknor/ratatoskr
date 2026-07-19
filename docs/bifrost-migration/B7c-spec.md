# B7c implementation spec — stale-calendar reap-vs-hide lifecycle

## Standing references (READ these before implementing)

This spec is written against, and must be judged against, the following. An
implementer or reviewer who has not read them is not equipped to work this spec:

- `reference/technical-implementation-spec.md` — the contract this document is
  written against (every-brick, obstacles-resolved-inline, no-deferral,
  no-shoehorning; the per-brick verification, keep/revert, concrete-artifact,
  ground-survey, and stopping-rule requirements).
- `reference/architecture.md` — the cross-cutting architecture contract
  (crate boundaries, generation counters, scope wiring, the calendar runtime
  layering). ALWAYS required regardless of target.
- `docs/bifrost-migration.md` — the TODO source. This spec builds item **B7c**
  (search "B7c. Stale-calendar reap-vs-hide lifecycle"); it also depends on
  B7a's landed retain-and-skip policy (`resolve_sync_targets`) and the B7a
  schema note that `history_backfilled_at` was B7a's one adjudicated schema
  addition and B7c is "the natural home for the second calendar-schema change."
- `reference/glossary/harness.md` — required for the sync-harness gate below
  (`brokkr service-*`, sync-harness scripts, `TestQueryDbState`, virtual clock).
- `reference/glossary/folders-labels.md` — NOT required: B7c touches only the
  `calendars` family of tables, none of the folders/labels/`label_kind`
  surface.
- No `UI.md` obligation for new UI: B7c adds no new overlay/widget. It changes
  which calendars the existing sidebar/agenda surfaces already read (via a new
  read-predicate), which is a query change, not a UI-convention change. If the
  implementer discovers the sidebar renders a calendar list from a path this
  spec's § 4.3 does not enumerate, that path is added to the same predicate and
  `UI.md` is consulted then.

## 1. What B7c is (the goal, stated once)

B7a chose **retain-and-skip** for a calendar the current backend no longer
lists: the `calendars` row is kept, its cached events keep rendering
indefinitely (`is_visible` unchanged), and it is merely excluded from the
fetch-target set (`crates/calendar/src/sync.rs::resolve_sync_targets`). That is
safe against a transient list omission but leaves a permanently-orphaned
calendar visible forever — e.g. a leftover Google `calendarId` on an account
the O10 precedence fix rerouted to CalDAV, or a calendar the user removed
server-side.

B7c makes unlisting **observable** and **eventually reaped**, without ever
losing a user's events to a transient blip:

1. On a **successful, non-empty** `calendars_list()` that omits a
   previously-known calendar, stamp `calendars.unlisted_since = now`.
2. Clear `unlisted_since` back to `NULL` the moment the calendar re-appears in
   a later successful list.
3. While stamped, **hide** the calendar from the sidebar/agenda read surfaces
   (a new read predicate; the row and its events survive, so a transient
   omission loses nothing).
4. Once `now - unlisted_since >= REAP_THRESHOLD` (7 days), **reap** it:
   delete its `calendar_events`, `calendar_attendees`, `calendar_reminders`,
   `caldav_event_map`, and the `calendars` row itself, in one transaction.

Only successful, non-empty list runs stamp or advance toward reap; a failed
`calendars_list()` (early-returns before any of this) and a
successful-but-empty list (§ 4.6 transient-safety guard) do neither.

## 2. Design decision: wall-clock threshold, not run-count

The TODO offers two threshold shapes: "~168 consecutive unlisted runs" or a
wall-clock `now - unlisted_since >= 7d`. **This spec chooses wall-clock**, for
three reasons:

- Robustness against missed kicks (the TODO's own stated reason): the calendar
  runner is a best-effort hourly kick, not a guaranteed cadence; a laptop
  asleep for a week must still reap on the next run, and a machine that kicks
  every 5 minutes must not reap in ~14 hours.
- Single-column cost: wall-clock needs only `unlisted_since`; a run-count would
  need a second counter column and a "was this the consecutive-next run?"
  interlock. B7a explicitly reserved B7c as the home for exactly **one** more
  calendar-schema change.
- Determinism under test: the reap boundary is a pure function of
  `(unlisted_since, now_ms)`, and `now_ms` is already an injected parameter of
  `calendar_sync_account_impl` (B7a: "on an injected `now_ms` clock"). The gate
  crosses 7 days by advancing the injected clock, not by looping 168 syncs
  (§ 5.5 builds the one missing seam — driving that injected clock from the
  harness).

## 3. Survey of the ground (what exists today, exactly)

### 3.1 Schema — `crates/db/src/db/schema/05_calendar.sql`

- `calendars` (PK `id TEXT`, `UNIQUE(account_id, remote_id)`) carries
  `is_visible INTEGER DEFAULT 1` (the **user's** show/hide toggle) and, since
  B7a, `history_backfilled_at INTEGER` (nullable). No `unlisted_since`.
- `calendar_events.calendar_id TEXT REFERENCES calendars(id) ON DELETE
  CASCADE` — cascades on a `calendars` row delete **iff `PRAGMA foreign_keys`
  is ON**.
- `caldav_event_map.calendar_id TEXT REFERENCES calendars(id) ON DELETE
  CASCADE` — same.
- **`calendar_attendees` and `calendar_reminders` do NOT reference
  `calendars`.** They key on `account_id` + `event_id` (attendees PK
  `(account_id, event_id, email)`; reminders reference `account_id`). Their
  only FK is to `accounts(id) ON DELETE CASCADE`. **A `calendars`-row delete
  therefore does NOT remove their rows.** This is the single most important
  correctness fact in this spec and the reason § 4.4 deletes all four tables
  explicitly rather than trusting cascade. The existing
  `db_delete_events_for_calendar` (`calendars/crud.rs:351`) already encodes the
  correct order: attendees + reminders by subselect on `calendar_id`, then
  events.

### 3.2 Schema policy — `crates/db/src/db/migrations.rs`

Pre-release policy (lines 65-75): schema changes go **directly into the
`schema/*.sql` file, extending v100 in place**. Do NOT add a v101 migration.
Dev DBs are wiped and re-seeded every launch. So B7c's column is one added line
in `05_calendar.sql`.

### 3.3 Sync entry — `crates/calendar/src/sync.rs`

`calendar_sync_account_impl(account_id, write_db, read_db, factory, now_ms,
cancellation_token) -> CalendarSyncOutcome { mutated, result }` delegates to
`sync_bifrost_calendar_account(...)`. That function:

- returns early (Ok, no-op) when `capabilities().pim_methods.calendars_list` or
  `events_in_range` is false — so an account whose backend is not queried never
  stamps (correct: we cannot infer unlisting from a backend we did not ask).
- calls `account.calendars_list().await.map_err(...)?` — **a list failure
  `?`-returns here, before any discovery/stamp/reap write.** This is the
  natural "successful runs only" guard.
- upserts every discovered calendar inside one `write_db.with_write` tx via
  `upsert_discovered_calendar` (`calendar_contacts_writes.rs:60`), accumulating
  a `changed` bool (O20: only genuine metadata changes flip it) into `mutated`.
- resolves targets: `let visible = load_visible_calendars(read_db,
  account_id)`; `let targets = resolve_sync_targets(&calendars, visible)`.
  `resolve_sync_targets` (line 208) already drops any visible row whose
  `remote_id` is not in the listed set (keyed by `idmap::calendar_remote_id`) —
  the retain-and-skip B7c now supersedes with stamp/hide/reap.

The **canonical remote-id space** is `idmap::calendar_remote_id(&Calendar)` (a
`&str`), which is exactly what `upsert_discovered_calendar` stores in
`calendars.remote_id` and what `resolve_sync_targets` matches on. Stamping's
"is this calendar in the listed set?" MUST use the same key.

### 3.4 Discovery upsert — `calendar_contacts_writes.rs:60`

`upsert_discovered_calendar(&WriteTxn, &DiscoveredCalendar) -> (String id,
bool changed)`. On conflict it updates `display_name/color/is_primary/can_edit`
only, guarded by a `WHERE ... IS NOT ...` clause that suppresses the write (and
the `updated_at` bump) when nothing tracked differs. It does **not** touch
`unlisted_since` today.

### 3.5 Read surfaces that must honor "hidden" — the survey

Every path that lists calendars or their events for the **user-facing** view
(so a stamped-but-not-reaped calendar disappears from view yet its data
survives):

- **CRITICAL crate correction (R2 finding 1, verified).** The sidebar/agenda
  SQL the app actually reads lives in **`db-read`**, not `db`. `rtsk` re-exports
  the calendar reads from `db-read` (`core/src/db/queries_extra.rs:5`:
  `pub use db_read::db::queries_extra::calendars::*;`) and the app imports them
  via `rtsk::db::queries_extra::calendars::{...}` (`app/src/db/calendar.rs:1-5`).
  The `crates/db` copies (line numbers below marked `db:`) are stale duplicates
  with **no app/core/calendar/service caller** (grep-verified). Editing only the
  `db` copies leaves both live UI surfaces unfiltered while any `-p db` test
  passes false-green. Brick R (§ 4.3) MUST edit the `db-read` definitions; the
  `db` duplicates should be updated in lockstep OR deleted as dead code (a
  lateral duplicate-implementation smell worth surfacing — confirm during
  implementation whether anything compiles the `db` copies).
  - `load_calendars_for_sidebar_sync` — **live copy `db-read/src/db/queries_extra/calendars/crud.rs:99`** (stale `db:635`) — the sidebar calendar list.
  - `load_view_event_rows_sync` — **live copy `db-read/src/db/queries_extra/calendars/view/mod.rs:65`** (stale `db:65`) — the agenda/event join, gated `WHERE (c.is_visible = 1 OR e.calendar_id IS NULL)`.
  - `db_get_visible_calendars` (`db:87`, `WHERE ... is_visible = 1`) — **no `db-read` copy and no app/core/calendar/service caller was found**; treat as possibly dead. Filter it only if implementation confirms a live caller; otherwise note it and leave it.
  - `set_calendar_visibility_sync` / `db_set_calendar_visibility` — user toggle,
    unaffected.
- `crates/calendar/src/sync.rs::load_visible_calendars` (line 572) — feeds
  `resolve_sync_targets`. An unlisted calendar is already dropped there by the
  listed-set intersection, so a `unlisted_since` filter here is belt-and-braces,
  not load-bearing; add it anyway for one consistent definition of "active".

Paths that must NOT filter on `unlisted_since` (they need the full row set):
`db_get_calendars_for_account` (line 68, the account-settings calendar
manager — the user may still want to see "this calendar went away"; out of
scope, left as-is), and every write/lookup-by-id path.

### 3.6 Runtime clock — `crates/service/src/calendar.rs:454`

The actual call chain (verified) is: the handler `handle_start_account_sync`
(`handlers/calendar.rs:36`) calls `runtime.start_account(params.account_id)`
(line 48); `start_account` (`service/calendar.rs:168`) spawns
`run_calendar_supervised` (line 373), which drives `run_calendar` (line 418);
`run_calendar` sources `now_ms` as `chrono::Utc::now().timestamp_millis()`,
**hardcoded at line 454**, and passes it to `calendar_sync_account_impl`.
(`run_calendar` never calls `start_account` — do not thread the clock the wrong
direction.) There is a **second internal `start_account` caller** at
`service/calendar.rs:122` — the staleness auto-kick — that must also compile
against the new signature (it passes `None`). `CalendarStartAccountSyncParams`
(`service-api/src/calendar.rs:58`) carries only `account_id`. **There is no way
for the harness to drive `now_ms` today** — the seam § 4.5 / § 5.5 must build,
and it must cross the `run_calendar_supervised` hop the old prose omitted.

### 3.7 Harness read surface — `crates/service/src/handlers/test_helpers.rs`

`TestQueryDbState` returns `calendar_count`, `calendar_event_count`, and
bounded `calendars` / `calendar_events` row snapshots (`read_harness_calendars`
line 2644, `test_db_calendar_from_row` line 2963). The calendar-row SELECT does
**not** currently include `unlisted_since` (the column will not exist until
§ 5.1). The gate needs to assert the stamp, so § 5.6 adds `unlisted_since` to
that row.

### 3.8 Existing calendar sync-harness scripts

`crates/app/tests/sync-harness/caldav-calendar-*.lua`. The CalDAV steady-state
and remote-delta scripts seed a `caldav` account against the `saehrimnir` mock
(fixture `graph-calendar-small.toml`, two calendars: `cal-work`,
`cal-personal`), drive `client:start_calendar_sync({account_id=...}, timeout)`,
and assert via `TestQueryDbState`. Remote-delta mutates the mock with
`harness.http` PUT/DELETE on individual `.ics` resources. **No existing script
removes a whole calendar collection from the listing, and no script drives the
clock** — both are new instrument needs (§ 5.5, § 5.7).

## 4. The target artifacts (concrete, buildable)

### 4.1 Brick S — schema column (`schema/05_calendar.sql`)

Extend the `calendars` CREATE TABLE with, after `history_backfilled_at`:

```sql
    -- Set to `now` (ms) on a successful, non-empty calendars_list() that
    -- omits this previously-known calendar; cleared to NULL the moment the
    -- calendar re-appears. While non-NULL the calendar is HIDDEN from the
    -- sidebar/agenda (its row and events survive). Once
    -- now - unlisted_since >= 7d the row is reaped. Only successful non-empty
    -- list runs stamp/advance it, so a provider outage cannot reap a live
    -- calendar. See docs/bifrost-migration.md B7c.
    unlisted_since INTEGER,
```

No index: the stamp/reap sweeps are per-account, bounded by the account's
calendar count (single digits), inside the discovery transaction that already
scans them.

### 4.2 Brick D — DB write helpers (`calendar_contacts_writes.rs`)

Three helpers alongside `upsert_discovered_calendar`, all taking `&WriteTxn`
so the sync layer runs them in the existing discovery transaction:

```rust
/// Clear `unlisted_since` for a calendar that re-appeared in the list.
/// The SHIPPED clear is folded into the discovery upsert path (see below); this
/// standalone form is NOT shipped as `pub` production surface — see the § 4.2
/// decision (drop it, or `#[cfg(test)]` it if a separate helper proves needed).
/// Signature kept here only to document the folded SQL it replaces.
fn clear_unlisted_since(conn: &WriteTxn<'_>, account_id: &str, remote_id: &str)
    -> Result<usize, String>;
// UPDATE calendars SET unlisted_since = NULL, updated_at = unixepoch()
//   WHERE account_id = ?1 AND remote_id = ?2 AND unlisted_since IS NOT NULL

/// Stamp `unlisted_since = now_ms` on every account calendar whose remote_id
/// is NOT in `listed_remote_ids` and is not already stamped. Returns count
/// stamped.
///
/// SELF-GUARDING (R2 finding 3): an empty `listed_remote_ids` is an internal
/// no-op — return `Ok(0)` before touching SQL. SQLite evaluates
/// `remote_id NOT IN ()` as TRUE for every row, so an unguarded empty slice
/// would stamp the WHOLE account's calendars (then reap them all after 7d).
/// The § 4.4 caller ALSO guards with `if !listed.is_empty()`, but per the
/// architecture rule "correctness must survive new call sites" the DB helper
/// must not depend on that discipline. Unit-test the empty-slice no-op directly.
pub fn stamp_unlisted_calendars(
    conn: &WriteTxn<'_>, account_id: &str, listed_remote_ids: &[String], now_ms: i64,
) -> Result<usize, String>;
// if listed_remote_ids.is_empty() { return Ok(0); }
// Build a placeholder list; UPDATE calendars SET unlisted_since = ?, updated_at = unixepoch()
//   WHERE account_id = ? AND unlisted_since IS NULL AND remote_id NOT IN (<placeholders>)

/// Reap every account calendar stamped >= threshold_ms ago. For each reaped
/// calendar deletes, in order: calendar_attendees + calendar_reminders (by
/// subselect on the calendar's events — they have NO FK to calendars),
/// caldav_event_map, calendar_events, then the calendars row. Returns count
/// of calendars reaped. Runs in the caller's transaction.
///
/// Deliberately does NOT reuse `db_delete_events_for_calendar` (crud.rs:351,
/// the model for the delete ORDER): that fn owns its own `with_write`/async
/// envelope, whereas reap must run inside the caller's discovery `WriteTxn`.
/// The extra `account_id = ?` predicate the reap adds over that fn is harmless
/// belt-and-braces — `calendar_events.id` is a global PK — not a correctness
/// difference.
pub fn reap_expired_unlisted_calendars(
    conn: &WriteTxn<'_>, account_id: &str, now_ms: i64, threshold_ms: i64,
) -> Result<usize, String>;
```

`reap_expired_unlisted_calendars` body (explicit deletes, cascade-independent —
see § 3.1):

```sql
-- reap_ids = SELECT id FROM calendars
--   WHERE account_id = ?1 AND unlisted_since IS NOT NULL
--     AND (?2 - unlisted_since) >= ?3        -- now_ms - unlisted_since >= threshold
-- for each cid in reap_ids:
DELETE FROM calendar_attendees WHERE account_id = ?acct AND event_id IN
  (SELECT id FROM calendar_events WHERE calendar_id = ?cid);
DELETE FROM calendar_reminders WHERE account_id = ?acct AND event_id IN
  (SELECT id FROM calendar_events WHERE calendar_id = ?cid);
DELETE FROM caldav_event_map WHERE calendar_id = ?cid;
DELETE FROM calendar_events WHERE calendar_id = ?cid;
DELETE FROM calendars WHERE id = ?cid;
```

The `unlisted_since = now_ms` DO-UPDATE clear is **folded into
`upsert_discovered_calendar`**: add `unlisted_since = NULL` to the `DO UPDATE
SET` list AND add `OR calendars.unlisted_since IS NOT NULL` to the guarded
`WHERE`, so a re-appearing calendar clears even when its metadata is otherwise
unchanged (otherwise the O20 guard suppresses the write and the stale stamp
survives — a real bug). The `changed` bool it returns then correctly flips
`mutated` on a clear (the UI must unhide).

**Decision on `clear_unlisted_since` (R1 finding 5 / R2 minor):** the folded
form is the shipped path, so the standalone `pub fn clear_unlisted_since` would
land as production-unused surface. **Do not ship it as a `pub` helper.** Either
(a) drop it entirely and prove the clear via the folded-upsert unit test (the
preferred outcome — the focused test in § 5.2 already exercises the folded
path), or (b) if a genuinely-separate clear helper turns out to be needed, gate
it `#[cfg(test)]`. Carry no dead `pub` surface into the merge.

### 4.3 Brick R — read-predicate (hide)

Add `AND unlisted_since IS NULL` to exactly the § 3.5 user-facing read paths.
**Edit the `db-read` copies** (per the § 3.5 crate correction) — that is what
the app reads:

- `load_calendars_for_sidebar_sync` (**`db-read` crud.rs:99**) — `... FROM
  calendars WHERE unlisted_since IS NULL ORDER BY ...`.
- `load_view_event_rows_sync` (**`db-read` view/mod.rs:65**) — the join
  predicate becomes `WHERE ((c.is_visible = 1 AND c.unlisted_since IS NULL) OR
  e.calendar_id IS NULL)`. (An event whose `calendar_id` is NULL — a
  local/imported event with no calendar — is still shown, unchanged.)
- `db_get_visible_calendars` (`db:87`) — `... WHERE account_id = ?1 AND
  is_visible = 1 AND unlisted_since IS NULL ...` **only if a live caller is
  confirmed** (§ 3.5 found none); else leave and note.
- Update the stale `db` duplicates of the first two in lockstep, or delete them
  as dead code.
- `load_visible_calendars` (sync.rs) — `... AND is_visible = 1 AND
  unlisted_since IS NULL ...` (belt-and-braces per § 3.5).

`db_get_calendars_for_account` (settings manager) is deliberately NOT filtered.

### 4.4 Brick W — wire stamp/reap into the sync loop (`sync.rs`)

Inside `sync_bifrost_calendar_account`, extend the **existing** discovery
`write_db.with_write` closure (currently lines ~94-109). After the
`for calendar in &discovered { upsert... }` loop, still inside the same
transaction:

```rust
let listed: Vec<String> = discovered
    .iter()
    .map(|c| idmap::calendar_remote_id(c).to_string())
    .collect();
// § 4.6: never stamp/reap on an empty successful list (transient-safety).
if !listed.is_empty() {
    let stamped = stamp_unlisted_calendars(&tx, &account_owned, &listed, now_ms)?;
    let reaped =
        reap_expired_unlisted_calendars(&tx, &account_owned, now_ms, REAP_THRESHOLD_MS)?;
    changed |= stamped > 0 || reaped > 0;
}
tx.commit()...
```

`changed` already feeds `*mutated |= calendars_changed;` at the call site, so a
stamp/hide (calendar vanishes from the sidebar) and a reap (calendar + events
vanish) both correctly emit `CalendarChanged` and reload the UI. Add the
constant near the other window constants (line 19-21):

```rust
const REAP_THRESHOLD_MS: i64 = 7 * 24 * 60 * 60 * 1000; // 7 days
```

Ordering note: stamp-then-reap in the same run is intentional and safe — a
calendar stamped *this* run has `now_ms - now_ms = 0 < threshold`, so it can
never stamp and reap in the same run; reap only ever fires on a row stamped in
a **prior** run.

### 4.5 Brick C — clock seam (`service` + `service-api`), so `now_ms` is drivable

Production keeps `Utc::now()`; the harness drives a virtual clock. The seam
touches **five** call sites (R1 findings 2-3 and R2 finding 2 — the original
"no other caller changes" claim was false and the chain is not buildable as
first written):

- `CalendarStartAccountSyncParams` (`service-api/src/calendar.rs:58`) gains
  `#[serde(default)] pub now_ms: Option<i64>` (backward-compatible: absent →
  `None`; every existing production caller omits it).
- `CalendarRuntime::start_account` (`service/calendar.rs:168`),
  **`run_calendar_supervised` (line 373)**, and `run_calendar` (line 418) all
  thread `now_ms: Option<i64>` through — the supervisor hop is not optional.
  `run_calendar` computes `let now_ms = now_ms.unwrap_or_else(|| chrono::Utc::now().timestamp_millis());`
  and passes it to `calendar_sync_account_impl` (which already takes `now_ms`).
- The **handler** `handle_start_account_sync` (`handlers/calendar.rs:48`) becomes
  `runtime.start_account(params.account_id, params.now_ms)`.
- The **second internal caller** — the staleness auto-kick at
  `service/calendar.rs:122` — becomes `start_account(account_id.clone(), None)`.
- The existing test `start_account_returns_err_after_shutdown`
  (`service/calendar.rs:570`) calls `start_account` and must be updated to the
  new arity.
- **App-side wiring (was missing).** `ServiceClient::start_calendar_sync`
  (`app/src/service_client.rs:1312`) today takes only `account_id` and
  constructs `CalendarStartAccountSyncParams { account_id }` — that literal
  struct build breaks the moment the field is added, so it must accept and
  forward `now_ms` (e.g. `start_calendar_sync(account_id, now_ms: Option<i64>)`,
  or a harness-only sibling entry point). The **Lua harness binding**
  `lua_client_start_calendar_sync` (`app/src/harness/mod.rs:1700`) currently
  extracts only `account_id` (line 1706) and discards other table fields; it
  must read an optional `now_ms` from the params table and pass it through.
  Without both, `client:start_calendar_sync({account_id=..., now_ms=T}, secs)`
  silently drops the clock and the § 5.7 reap gate cannot cross 7 days.

This is a legitimate parameter (the impl fn is already clock-injected), not
env-var scaffolding, so it satisfies the "no temporary switches" cleanliness
bar. **Scope caution (R2 finding 2):** `now_ms` drives the *entire* calendar
sync window and history-backfill behavior, not only the reap boundary, so the
earlier "future reap-now maintenance action" framing overstated its standalone
production value — a harness-driven clock is the intended and only current
consumer. Prefer the narrowest surface that lets the harness set it (an explicit
`Option<i64>` defaulting to `None` for every production caller); do not encourage
production callers to pass a synthetic clock.

### 4.6 Stopping rule / stamping guards (pinned decisions)

- **Empty successful list does not stamp.** If `calendars_list()` returns
  `Ok(vec![])` for an account that previously had calendars, treat it as a
  suspected transient (mirrors B7a's empty-pull guard) and skip stamp AND reap
  (§ 4.4 `if !listed.is_empty()`). Consequence (R1 finding 1 — corrected): an
  account whose calendars ALL genuinely vanished server-side is
  **retained-and-visible forever**, never stamped and never reaped. It is NOT
  hidden: hiding is driven entirely by a non-NULL `unlisted_since` stamp, and an
  empty list stamps nothing, so the § 4.3 `unlisted_since IS NULL` predicate
  keeps those rows rendering. Hiding only ever applies to a calendar omitted
  *while others were listed* in a prior non-empty run. This is the deliberate
  safe choice — reap fires only on the strong signal "the provider affirmatively
  listed OTHER calendars while omitting this one."
- **Failed list never stamps** — guaranteed structurally by the `?`-return in
  `sync_bifrost_calendar_account` before the discovery transaction.
- **Capability-off account never stamps** — the early no-op return precedes the
  list call.
- **B7c subsumes the O10/O17 reroute leftover.** A stale Google `calendarId`
  left on a now-CalDAV account is exactly "known calendar absent from a
  non-empty successful list"; it now stamps and reaps after 7 days. This is the
  intended cleanup, not a regression of retain-and-skip.
- Out of scope: any change to the user's `is_visible` toggle semantics; the
  settings calendar-manager view (`db_get_calendars_for_account`); a UI
  "calendar was removed" affordance; reaping on account deletion (already
  handled by `accounts ON DELETE CASCADE`); a per-provider variant of the reap
  (the logic is provider-agnostic — one call site — by construction).

## 5. Bricks in landing order (each keeps `brokkr check` green)

Order is chosen so the tree compiles and passes at every boundary. Schema and
DB helpers land before the code that calls them; the read-predicate and the
clock seam are independent and can land in either relative order; the harness
gate lands last because it exercises the whole chain.

### 5.1 Brick S — schema column

Add `unlisted_since` per § 4.1. Dev-seed re-creates the DB, so no migration.
Gate:

```
brokkr test -p db migrations_run_on_fresh_db
```

(Confirms v100 still builds with the new column.) Plus the universal gate at
the end of every brick:

```
brokkr check
```

### 5.2 Brick D — DB write helpers + folded clear

Implement § 4.2 (`stamp_unlisted_calendars` — with its empty-slice self-guard,
`reap_expired_unlisted_calendars`, the folded `unlisted_since = NULL` + guard in
`upsert_discovered_calendar`, and NO standalone `pub clear_unlisted_since` per
the § 4.2 decision). Add a focused
`#[cfg(test)]` module in `calendar_contacts_writes.rs` (or the existing db test
module) with an in-memory DB (`PRAGMA foreign_keys = ON`, `run_all`) covering:

- stamp: a known calendar absent from a non-empty listed set gets
  `unlisted_since = now`; a listed one is left NULL; an already-stamped one is
  not re-stamped (idempotent).
- clear-on-reappear: `upsert_discovered_calendar` on a currently-stamped
  calendar clears `unlisted_since` to NULL AND returns `changed = true` even
  when metadata is identical (the O20-guard-bypass fix).
- reap boundary: a row stamped at `now - (7d - 1ms)` is NOT reaped; at
  `now - 7d` it IS; the reap removes the calendar's `calendar_events`,
  `calendar_attendees`, `calendar_reminders`, `caldav_event_map` rows AND the
  `calendars` row, and leaves a *listed* sibling calendar and its rows fully
  intact. Seed at least one attendee and one reminder to prove the
  no-FK-to-calendars explicit-delete path (§ 3.1) — a cascade-only
  implementation would leak these and this assertion catches it.

Gate (name the module test appropriately):

```
brokkr test -p db calendar_unlisted
brokkr check
```

### 5.3 Brick R — read-predicate (hide)

Apply § 4.3. Add a unit test **in `db-read`** (the crate that owns the live
loaders — a `-p db` test would exercise the stale duplicates and pass
false-green) over the view/sidebar loaders: a stamped calendar is absent from
`load_calendars_for_sidebar_sync`, its events absent from
`load_view_event_rows_sync`, while a NULL-`unlisted_since` calendar and a
`calendar_id IS NULL` local event still appear.

```
brokkr test -p db-read calendar_hidden_when_unlisted
brokkr check
```

### 5.4 Brick W — wire stamp/reap into the sync loop

Apply § 4.4 (constant + the `if !listed.is_empty()` block in the discovery
transaction). This is the brick that makes `mutated` reflect stamp/reap. The
existing `-p cal` `stale_unlisted_calendar_is_not_fetched` test still passes
(resolve_sync_targets is unchanged — hidden calendars are additionally excluded
from targets via the § 4.3 `load_visible_calendars` predicate, so the stale row
never even reaches `resolve_sync_targets`; keep the test as a lower-level
guard). No new pure-unit surface here beyond §§ 5.2-5.3; the behavior is proven
end-to-end by the harness gate (§ 5.7).

```
brokkr test -p cal stale_unlisted_calendar_is_not_fetched
brokkr check
```

### 5.5 Brick C — clock seam

Apply § 4.5 across `service-api` (`CalendarStartAccountSyncParams.now_ms`),
`service` (`start_account` / `run_calendar_supervised` / `run_calendar`
threading + the line-122 caller + the line-570 test), and the app side
(`ServiceClient::start_calendar_sync` + the Lua binding).

**Gate correction (R1 finding 4 / R2 finding 5):** the previously-named
`calendar_start_params_roundtrip` matches NOTHING — brokkr's substring filter
treats zero matches as a skip (false-green). A round-trip test already exists:
`calendar_start_account_sync_params_round_trips_through_serde`
(`service-api/src/calendar.rs:199`). **Extend that existing test** to cover the
new field (serialize with and without `now_ms`, deserialize, assert
`now_ms == None` when the field is absent — proving wire back-compat). Do not
add a parallel test.

```
brokkr test -p service-api calendar_start_account_sync_params_round_trips_through_serde
brokkr check
```

### 5.6 Brick T — expose `unlisted_since` to the harness

Add `unlisted_since` to `read_harness_calendars`' SELECT and
`test_db_calendar_from_row` / `TestDbCalendarRow`
(`handlers/test_helpers.rs`) so the § 5.7 script can assert the stamp before
reap. Serde-optional so existing callers are unaffected.

```
brokkr check
```

### 5.7 Brick G — the reap sync-harness gate

New script `crates/app/tests/sync-harness/caldav-calendar-unlisted-reap.lua`
(CalDAV chosen: the `graph-calendar-small.toml` fixture already has two
calendars and the CalDAV scripts already mutate the mock). Shape:

1. Seed a `caldav` account; initial `start_calendar_sync` at `now_ms = T0`.
   Assert `calendar_count == 2`, both events present.
2. Drop `cal-personal` from the mock's listing (§ 5.8 instrument), keep
   `cal-work`. `start_calendar_sync` at `now_ms = T0` again. Assert via
   `TestQueryDbState`: `calendar_count` still `2` (row RETAINED, not deleted),
   the dropped calendar's row now carries `unlisted_since ~= T0` (stamp), and
   it is absent from the sidebar/agenda surface. **Prove hide through the real
   read surface, not the `unlisted_since` non-NULL proxy (R2 finding 5a):** the
   proxy proves stamping only, not that the production sidebar/agenda predicate
   filters. Assert against a `TestQueryDbState` field that is fed by the actual
   `load_calendars_for_sidebar_sync` / `load_view_event_rows_sync` (db-read)
   loaders — if § 3.7's harness read surface exposes no such sidebar-visible
   set, adding one (routed through those loaders) is a prerequisite brick of
   this gate, laid with § 5.6. `cal-work` untouched.
3. **Re-appear branch:** restore `cal-personal` to the listing;
   `start_calendar_sync` at `now_ms = T0 + 1h`. Assert `unlisted_since` back to
   NULL and the calendar visible again — proving a transient omission loses
   nothing. Then drop it again and re-stamp at `now_ms = T0 + 2h`.
4. **Reap branch:** `start_calendar_sync` at `now_ms = T0 + 8d`
   (> 7-day threshold measured from the `T0 + 2h` stamp). Assert
   `calendar_count == 1`, the dropped calendar's events/attendees/reminders
   gone (`calendar_event_count` dropped by its event count), `cal-work` and its
   events fully intact.
5. **Failure-does-not-reap — MANDATORY (R2 finding 5b), not optional.** § 4.6
   makes "a failed or empty list past the threshold must not reap" an explicit
   source requirement, so the lifecycle gate must exercise it end-to-end:
   between the stamp and the reap, one run whose list FAILS (or returns empty)
   at a `now_ms` past the threshold, then assert the row and its events SURVIVE.
   The CalDAV `on("caldav", ...)`/latency/override surface (or, for the
   empty-list arm, a fixture with the dropped calendar as the *only* remaining
   collection so the list is non-failing-but-effectively-empty for the reap
   decision) drives this within the same script. A `-p db` unit assertion that
   an empty `listed` no-ops is a **necessary complement** (it pins the
   self-guard of § 4.2 / R2 finding 3) but is NOT a substitute for the in-script
   end-to-end assertion — keep both.

Gate:

```
brokkr service-test crates/app/tests/sync-harness/caldav-calendar-unlisted-reap.lua
brokkr check
```

### 5.8 Instrument: mock "unlist a calendar" (saehrimnir side-quest)

The gate needs the mock to omit a previously-listed CalDAV calendar collection
from its PROPFIND listing between syncs (and restore it for the re-appear
branch). `saehrimnir` source lives in this checkout at `research/saehrimnir/`
(installed binary at `~/.cargo/bin/saehrimnir` per `brokkr.toml:110`).

**Verified gap (R2 finding 4 / R1 finding 7) — the "discover whether DELETE
works" framing was an unresolved obstacle; it is now resolved: DELETE on a
calendar collection does NOT work today.** `handle_delete`
(`research/saehrimnir/src/caldav/mod.rs:1256`) matches only
`ResourcePath::Event` and returns `not_found` for a `ResourcePath::Calendar`
(collection) path (`mod.rs:1262-1268`). saehrimnir's own docs confirm CalDAV v0
"explicitly does not implement" collection removal. There is a `MKCALENDAR`
create path (`mod.rs:227` region) but no unlist/remove-collection path. So the
plain `harness.http DELETE` on the collection URL 404s and cannot drive this
gate.

**Pinned instrument (build this in `research/saehrimnir/`, land before § 5.7):**
add a harness-reachable mutation to remove a calendar from
`Fixture::calendars_for(user)` (so PROPFIND on the calendar-home stops listing
it) and restore it for the re-appear branch. Two acceptable shapes — pick one
and pin it in the script:
- extend CalDAV `handle_delete` to accept `ResourcePath::Calendar` (delete the
  collection + its events from the fixture, recording a `calendar_destroyed`
  transition), so step 2 is `harness.http({ method = "DELETE", url =
  caldav_url(endpoint, "calendars/account-1/cal-personal/") })` and step 3
  re-creates via the existing `MKCALENDAR`; OR
- add a test-admin endpoint under saehrimnir's control plane (e.g. `POST
  /test/caldav/calendars/unlist` and a `.../relist`, keyed by account + calendar
  id) mutating the fixture directly.
The DELETE-collection extension is preferred (it exercises a real WebDAV verb
and reuses `MKCALENDAR` for restore). Cover it with a saehrimnir integration
test in `tests/caldav.rs` (delete-collection then PROPFIND shows it gone; then
MKCALENDAR re-adds). This instrument is a hard prerequisite of § 5.7 and its
feasibility is now confirmed against real source, not left to discovery.

## 6. Verification summary (copy-paste, per the contract)

| Brick | What it can break | Exact gate |
| --- | --- | --- |
| S schema | v100 build | `brokkr test -p db migrations_run_on_fresh_db` |
| D helpers | stamp/clear/reap SQL, cascade leak | `brokkr test -p db calendar_unlisted` |
| R read-predicate | sidebar/agenda hide | `brokkr test -p db-read calendar_hidden_when_unlisted` |
| W sync wiring | target resolution regression | `brokkr test -p cal stale_unlisted_calendar_is_not_fetched` |
| C clock seam | wire back-compat | `brokkr test -p service-api calendar_start_account_sync_params_round_trips_through_serde` |
| G end-to-end | hide-then-reap-then-reappear lifecycle | `brokkr service-test crates/app/tests/sync-harness/caldav-calendar-unlisted-reap.lua` |
| P sync-bench | calendar steady-state budget drift | `brokkr sync-bench caldav_calendar_steady_state` (record green vs `brokkr.toml:335`) |
| every brick | green tree | `brokkr check` |

**Sync-bench gate IS owed (R2 finding 6 — corrected).** The prior text declined
one and claimed `brokkr check` would surface baseline drift. Both are wrong:
`technical-implementation-spec.md:84-90` says a spec touching a sync/provider/
storage/Service hot path owes the relevant recorded `brokkr sync-bench` gate,
and B7c touches the calendar sync path; and `harness.md:58` states bare `brokkr
check` is **blind to orchestration blocks** ("no `[[check]]` cross-reference"),
so `brokkr check` cannot surface sync-bench drift at all. A suitable baseline
already exists: `caldav_calendar_steady_state` (`brokkr.toml:335`, with a
`meta.calendar_count` / `meta.calendar_event_count` / `meta.provider_requests`
baseline). B7c's expectation is genuinely a no-op on the steady-state budgets
(the stamp/reap sweeps live inside the discovery transaction that already scans
these single-digit rows, adding no page fetch or provider request), so the gate
should **record green against the existing baseline** — that recorded run is the
owed artifact, not a reason to skip. If it drifts, the stamp/reap sweep was
placed outside the discovery transaction (a mistake) — fix the placement, do not
re-baseline.

```
brokkr sync-bench caldav_calendar_steady_state
```

## 7. Lateral findings to watch for while implementing

Flag (do not silently absorb) if encountered:

- Any *other* read path that lists calendars for the user view beyond § 3.5 —
  add it to the § 4.3 predicate and note it.
- Whether `PRAGMA foreign_keys` is reliably ON on the writer connection — the
  reap deletes explicitly and does not depend on it, but a discovery that it is
  OFF would mean `account`-cascade delete-on-account-removal is also not
  cascading, which would be a pre-existing bug worth surfacing.
- Whether `caldav_event_map` rows for a reaped calendar are keyed only by
  `calendar_id` (they are — PK `(calendar_id, uri)`), so the explicit
  `DELETE ... WHERE calendar_id = ?` is complete; a schema drift here would
  orphan the map.
- Whether any consumer treats a shrinking `calendar_count` as an error rather
  than the expected reap outcome (the harness `assert_eq` on `calendar_count`
  is the canary).
- **Duplicate query implementations across `db` and `db-read`** (surfaced while
  validating R2 finding 1): `load_calendars_for_sidebar_sync` and
  `load_view_event_rows_sync` exist in BOTH crates; only the `db-read` copies are
  reachable from the app. The stale `db` copies are latent false-green traps for
  any future calendar-read change, not just this one. Worth an issue.

## 8. Review consolidation (R1 + R2 folded)

Both review reports (`B7c-R1.md`, Opus; `B7c-R2.md`, Codex xhigh) were validated
against the live tree and folded above. Dispositions:

| # | Finding (source) | Verdict | Where folded |
| --- | --- | --- | --- |
| 1 | Brick R edits the wrong crate; live sidebar/agenda SQL is in `db-read`, not `db` (R2-1) | **VALID (highest impact)** — grep-confirmed the app imports the `db-read` re-exports; `db` copies have no caller | § 3.5, § 4.3, § 5.3, § 6 table |
| 2 | Clock seam not buildable as written: `run_calendar_supervised` hop, handler, second caller (line 122), line-570 test, `ServiceClient::start_calendar_sync`, Lua binding all need changes; "no other caller changes" false (R1-2, R1-3, R2-2) | **VALID** — all call sites verified | § 3.6, § 4.5, § 5.5 |
| 3 | § 4.5 "reap-now production use" overstates value; `now_ms` drives the whole sync window (R2-2) | **VALID** | § 4.5 scope caution |
| 4 | Empty `listed` slice would stamp every calendar (`NOT IN ()` is TRUE); guard belongs in the DB helper, not caller discipline (R2-3) | **VALID** | § 4.2 self-guard, § 5.2 |
| 5 | saehrimnir cannot unlist a CalDAV collection today; `handle_delete` matches only `ResourcePath::Event` (R1-7, R2-4) | **VALID** — confirmed at `research/saehrimnir/src/caldav/mod.rs:1256-1268` | § 5.8 pinned instrument |
| 6 | § 5.7 hide-proxy proves stamping not hiding (R2-5a) | **VALID** | § 5.7 step 2 |
| 7 | Failure-does-not-reap made optional despite being a source requirement (R2-5b) | **VALID** | § 5.7 step 5 |
| 8 | Clock gate names a test that matches nothing; real test is `calendar_start_account_sync_params_round_trips_through_serde` (R1-4, R2-5c) | **VALID** — confirmed at `service-api/src/calendar.rs:199` | § 5.5, § 6 table |
| 9 | Sync-bench gate is owed; "brokkr check surfaces drift" is false per `harness.md:58` (R2-6) | **VALID** — both contract refs verified | § 6 sync-bench para + table row P |
| 10 | § 4.6 "retained-and-hidden forever" is wrong for the empty-list case (should be visible) (R1-1) | **VALID** | § 4.6 first bullet |
| 11 | `clear_unlisted_since` ships as production-unused `pub` surface (R1-5, R2-minor) | **VALID** | § 4.2 decision, § 5.2 |
| 12 | Reap re-implements `db_delete_events_for_calendar`; state the non-reuse reason (R1-6) | **VALID (clarification)** | § 4.2 reap doc |

### Rejected / partially rejected

- **R1 finding 8 — "§ 3.6's section header cites `sync.rs`."** *Rejected as
  stated.* The § 3.6 header already reads `crates/service/src/calendar.rs:454`,
  not `sync.rs`, so the specific claim is factually wrong. Its valid kernel —
  that `:454` is the hardcoded-clock line *inside* `run_calendar` (which starts
  at 418), not the function definition — is real but marginal and is already
  absorbed by the § 3.6 prose rewrite (folded finding 2), which now spells out
  the whole chain with the correct line for each hop. No separate change owed.

No finding was rejected on substance; only R1-8's mis-citation was rejected,
with its underlying nuance retained.
