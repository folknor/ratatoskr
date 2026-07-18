# B7a - Calendar read sync onto the bifrost calendar pull surface

Technical implementation specification. Collapse ratatoskr's four
per-provider calendar read-sync implementations onto bifrost's unified
`Account` calendar pull surface (`calendars_list` + `events_in_range`),
establish the id-translation seam that B7b reuses, and delete the
per-provider sync impls. One coherent, fully intrusive landing.

## Required reading (clause 10)

Read these before implementing or reviewing. They are the ground this
work is built on and judged against; naming them is not enough.

- `reference/technical-implementation-spec.md` - the contract this spec
  is written against. Every clause reference below (clause 5, 6, 7, 8,
  9, 10) points here.
- `reference/architecture.md` - the cross-cutting architecture contract.
  Always required. Binds crate boundaries (bifrost must not become a dep
  of `core`/`rtsk`, per § 7 below), the Service-to-app wire contract, the
  `OperationResult`/`ActionOutcome` taxonomy, generation counters, and
  scope wiring. The calendar runtime is a Service subsystem; its drain
  ordering and notification contract live in this contract's orbit.
- `docs/bifrost-migration.md` - the TODO source. This spec is item B7a.
  Read § 1 (goal: maximal integration, no parallel hand-rolled surface
  survives), § 2 (first principle: ratatoskr is never contorted around a
  bifrost wart; the side-quest protocol), § 7 B7/B7a/B7b (the
  decomposition note, settled against frozen bifrost `be11bbb` - the
  frozen `research/bifrost` / `../bifrost` reference for this item,
  advanced there by the B7a calendar side-quests (earlier drafts pinned
  `0e71226` then `d3f9cca`)), § 9
  (calendar risk), and § 10 (behavioral gates are mandatory).
- `reference/glossary/harness.md` - the sync-harness and service-test
  surface. The per-provider calendar gates this spec pins
  (`*-calendar-initial.lua`, `*-calendar-recurrence-initial.lua`,
  `*-calendar-remote-delta.lua`) run through it. Read the `start_calendar_sync`
  client binding and the `test.query_db_state` calendar snapshot before
  touching a gate.
- `docs/calendar/` - the calendar feature design docs (the app-side view
  model, RRULE expansion, and the open questions the runtime comments
  cite - these live in `docs/calendar/discrepancies.md` and
  `review-findings.md`; there is no `phase-5-plan.md`, an earlier draft
  named a file that does not exist). The read-sync output contract (what
  the DB cache must contain for the view to render) is grounded here.
  NOTE `discrepancies.md` § High item 7 records that CalDAV reminders ARE
  surfaced in event-detail views today - load-bearing for O7 (§ 11).
- Bifrost reference, read in `./research/bifrost`: `reference/caldav.md`
  (the standalone CalDAV `Account`, the only DAV calendar path),
  `reference/google.md` / `reference/graph.md` / `reference/jmap.md`
  (each protocol crate's `Account` impl, including its calendar
  primitives), and `reference/error-model.md` (the `AccountError` ->
  `RecoveryClass` contract the runner maps down).

This spec does NOT cover folders/labels, so
`reference/glossary/folders-labels.md` is not required reading: calendar
collections are not the `labels` table and do not touch `label_kind` or
system folder ids. (Calendar collections live in their own `calendars`
table - see § 2.3.)

## 1. The goal (clause 7: the target as concrete artifacts)

Today calendar read-sync branches four ways on a provider string, in
`crates/calendar/src/sync.rs::calendar_sync_account_impl`, into four
hand-rolled protocol implementations (`google_calendar_*_impl`,
`graph_calendar_*_impl`, `jmap::sync_jmap_calendar_account`,
`sync_caldav_*`). Each speaks its own wire dialect, owns its own delta
mechanism (Google sync tokens, Graph delta tokens, JMAP changes, CalDAV
CTags/ETags), and writes the same `calendars` / `calendar_events` /
`calendar_attendees` / `calendar_reminders` / `caldav_event_map` tables.

The target: one provider-agnostic read-sync body that

1. opens one bifrost `Arc<dyn Account>` for the account's calendar
   backend (built by a new Service-side calendar account-factory router,
   § 4.1);
2. reads `account.capabilities().pim_methods.calendars_list`; if false,
   the account has no calendar backend - the run is a clean no-op
   (no error, no mutation);
3. calls `account.calendars_list()` -> `Vec<bifrost_types::Calendar>`,
   translates each to a `DiscoveredCalendar` row (§ 4.3), and upserts;
4. for each visible calendar, pages `account.events_in_range(EventRange
   { calendar_id, start, end, page_cursor, limit })` over the configured
   rolling cache window (§ 2.6), translating each
   `bifrost_types::CalendarEvent` to the existing `CalendarEventRow` +
   attendee/reminder rows (§ 4.4), and upserting; then reconciles
   deletions over the windowed set (§ 4.5);
5. maps `AccountError` -> the runtime's `CalendarSyncOutcome { mutated,
   result: Result<(), String> }` so the existing emission rules
   (`CalendarChanged` when `mutated`, `CalendarRunCompleted` always) are
   untouched.

There is one call site per operation after the rewire (the B7
decomposition note: the per-provider axis collapses). The four
per-provider sync impls and their supporting modules are deleted in the
same landing (§ 5).

The id-translation seam (`bifrost_types::CalendarId` / `EventId` /
`CalendarProvenance` <-> ratatoskr's `(account_id, remote_id)` calendar
key and `google_event_id` / `remote_event_id` / `etag` event keys, § 2.3
and § 4.6) is established here as a small standalone module and reused by
B7b for the write path.

Unchanged and out of scope: the `calendars` / `calendar_events` /
`calendar_attendees` / `calendar_reminders` / `caldav_event_map` schema;
the DB write helpers in
`crates/db/src/db/queries_extra/calendar_contacts_writes.rs`; the
`CalendarRuntime` lifecycle (supervisor, semaphore, kick gating, drain,
notifications) in `crates/service/src/calendar.rs`; the app-side view
model and RRULE expansion (`crates/app/src/db/calendar.rs`); the
`calendar.*` IPC and the `CalendarChanged` / `CalendarRunCompleted` wire
contract. The write path (create/update/delete/RSVP) is B7b.

## 2. Survey of the ground (clause 8)

### 2.1 What B7a rips out (the per-provider read-sync impls)

`crates/calendar/src/sync.rs` (1017 LOC) is the orchestration layer.
`calendar_sync_account_impl` reads `(provider, calendar_provider,
caldav_url)` from the `accounts` row and resolves a provider tag (lines
74-97). IMPORTANT - the actual resolution is NOT "calendar_provider
wins"; it is a fixed-order if/else chain where the mail `provider` can
short-circuit before `calendar_provider` is consulted (see the
behavior-change note in § 4.1 and O10): branch 1 fires on
`calendar_provider == "google_api" || provider == "gmail_api"`, branch 2
on `... == "graph" || provider == "graph"`, branch 3 on
`... == "caldav" || (provider == "caldav" && non-empty caldav_url)`,
branch 4 on `... == "jmap" || provider == "jmap"`. So a row with
`provider = "gmail_api"` and `calendar_provider = "caldav"` resolves to
**google today**, NOT caldav - the CalDAV intent is silently lost for a
gmail/graph mail account. It then dispatches into:

- `sync_google_calendar_account` -> `google_calendar_list_calendars_impl`
  + per-calendar `google_calendar_sync_events_impl` (sync-token delta),
  applied via `apply_calendar_sync_result_impl`. Backed by
  `crates/calendar/src/google.rs` (715 LOC).
- `sync_graph_calendar_account` -> `graph_calendar_list_calendars_impl` +
  `graph_calendar_sync_events_impl` (Graph delta token). Backed by
  `crates/calendar/src/graph.rs` (225 LOC).
- `super::jmap::sync_jmap_calendar_account` -
  `crates/calendar/src/jmap.rs` (178 LOC), JMAP calendar changes.
- `sync_caldav_calendar_account` -> the in-file CalDAV machinery
  (`run_caldav_sync_attempt`, `sync_caldav_calendars`,
  `sync_caldav_calendar_events`, `upsert_caldav_parsed_event_tx`, the
  CTag/ETag diff, the stale-URL rediscovery retry), backed by
  `crates/calendar/src/caldav/` (mod.rs 560, ical.rs 244) and
  `rtsk::caldav` (the `CalDavClient` + iCalendar parse in
  `crates/core/src/caldav/`).

The provider clients (`GmailState`/`GraphState`/`JmapState`,
`gmail::client::GmailClient::from_account`, etc.) are constructed
per-run inside these helpers, keyed by the runtime's per-provider client
registries (`crates/service/src/calendar.rs` lines 123-125).

Load-bearing behaviors the rip must preserve (clause 8: drop no
load-bearing work):

- **`CalendarSyncOutcome.mutated` semantics** (sync.rs lines 26-37):
  `mutated` is set true after each successful helper write, independent
  of `result`. A partial-commit failure (discovered-calendars upserted,
  then a per-calendar apply errors) MUST still emit `CalendarChanged`.
  The new body keeps the exact same `&mut bool` threading: flip `mutated`
  after the discovered-calendars upsert and after each per-calendar
  event apply.
- **Cancellation checkpoints between RPC boundaries** (sync.rs lines
  353-360): point-checks of `cancellation_token.is_cancelled()` before
  each calendar's event fetch and between page fetches, never mid-RPC.
  Calendar sync is idempotent against provider state, so a cancelled run
  needs no marker-file repair (the runtime comment in
  `crates/service/src/calendar.rs` lines 22-34 is the contract for this).
- **CalDAV "server returned 0 events but local cache is non-empty"
  guard** (sync.rs lines 677-704): a transient empty 207 must NOT
  trigger a mass delete. The windowed-reconcile design (§ 4.5) carries
  this guard forward in provider-agnostic form.
- **CalDAV per-resource 207 failure preservation** (sync.rs lines
  692-704): hrefs the server reports as failing in a multistatus are
  preserved locally, not deleted. The snapshot path already models
  `failed_hrefs` (`research/bifrost/crates/caldav/src/account.rs` line
  146) but frozen `events_in_range` swallows failures; the prerequisite
  side-quest SQ-4 surfaces the failed set and § 4.5 subtracts it from
  the deletion reconcile (O3 RESOLVED, § 12 ruling 4).

### 2.2 The bifrost calendar pull surface (concrete types)

Read against frozen bifrost (the item's pinned commit; verify
`./research/bifrost` HEAD before speccing changes). The `Account` trait
(`research/bifrost/crates/types/src/account.rs` lines 660-696) exposes:

```rust
fn calendars_list(&self) -> AccountFuture<Result<Vec<Calendar>, AccountError>>;
fn events_in_range(&self, range: EventRange)
    -> AccountFuture<Result<Page<CalendarEvent>, AccountError>>;
fn event_get(&self, event: EventId) -> AccountFuture<Result<CalendarEvent, AccountError>>;
// event_create / event_update / event_delete / event_rsvp / event_search -> B7b
```

The data types (`research/bifrost/crates/types/src/calendar.rs`):

- `Calendar { id: CalendarId, native_id, name, color: Option<String>,
  provenance: CalendarProvenance, is_default, can_create_events,
  can_update_events, can_delete_events }`. Note bifrost splits write
  rights three ways where ratatoskr's `calendars.can_edit` is a single
  bit; B7a maps `can_edit = can_update_events` (the closest single-bit
  analog the existing schema and UI read) and records the finding for a
  possible schema enrichment (§ 8, O4).
- `CalendarEvent { id: EventId, calendar_id: CalendarId, native_id, uid:
  Option<String>, etag: Option<String>, provenance, title, description,
  location, start: EventTime, end: EventTime, is_all_day, status:
  EventStatus, availability: EventAvailability, visibility:
  EventVisibility, self_response: RsvpStatus, organizer:
  Option<EventOrganizer>, attendees: Vec<EventAttendee>, recurrence:
  EventRecurrence, html_link, raw_ical }`.
- `EventTime { value: String, timezone: Option<String> }` - RFC 3339
  date-time, or `YYYY-MM-DD` when `is_all_day`. ratatoskr's schema stores
  `start_time`/`end_time` as Unix `i64` plus a `timezone` TEXT and an
  `is_all_day` flag, so the translation parses `EventTime.value` to an
  epoch (§ 4.4); the all-day and floating-vs-zoned cases are the careful
  part and get unit-test gates (§ 6).
- `EventRecurrence { rrule: Option<String>, rdate: Vec<String>, exdate:
  Vec<String>, recurrence_id: Option<String> }` - RFC 5545 text. Maps to
  `calendar_events.recurrence_rule` (rrule) and `recurrence_id`. The
  `recurrence_id` canonical-string contract (host-tz-independent
  `YYYYMMDD` / `...TZ` forms, schema comment lines 56-69) MUST be
  preserved by the translation, because the load-path phantom-dedup keys
  off `(account_id, uid)` + recurrence_id (the
  `make_google_event_id` keying in sync.rs lines 927-932 and its unit
  tests lines 986-1016).
- `EventRange { calendar_id, start: EventTime, end: EventTime,
  page_cursor: Option<Vec<u8>>, limit: Option<u32> }` and
  `Page<CalendarEvent> { items, next_cursor: Option<Vec<u8>> }` - the
  opaque-cursor paging contract.
- `CalendarProvenance { provider: ProtocolKind, native: String,
  calendar_native: Option<String> }` - carries the wire-native id and
  protocol family. This is the seam input (§ 4.6).

POST-SIDE-QUEST SHAPE (adjudicated, § 12): B7a is implemented against
the bifrost freeze AFTER the § 12 prerequisite side-quests land, which
change this surface in four ways: `CalendarEvent` gains a
`reminders: Vec<EventReminder>` field (SQ-3, populated by CalDAV VALARM
and JMAP JSCalendar alerts; empty for Google/Graph); CalDAV
`events_in_range` yields EVERY VEVENT of a multi-VEVENT resource, not
only the first (SQ-2); the all-day `DTEND` decrement is removed and the
`EventTime` contract is documented on the type (SQ-1: all-day end is
EXCLUSIVE uniformly; timed values are offset/`Z`-bearing, or bare
wall-clock interpreted in the separate `timezone` field, or floating);
and the CalDAV `events_in_range` result surfaces the per-resource
failed set instead of silently dropping it (SQ-4). Re-verify the exact
post-side-quest signatures against the advanced freeze before citing
line numbers; the § 2.6/§ 4.4/§ 4.5 instructions below are written
against this post-side-quest surface.

Capabilities (`research/bifrost/crates/types/src/capabilities.rs` lines
244-253): `pim_methods.{calendars_list, events_in_range, event_get,
event_create, event_update, event_delete, event_rsvp, event_search,
event_autocomplete}`. B7a reads `calendars_list` (and `events_in_range`)
to gate the run. The accounts that leave them false return
`Err(Unsupported)` from the primitive; the gate avoids even calling them.

Account construction: `AccountFactory::open(account_id) -> Arc<dyn
Account>` (`account.rs` lines 949-959). Native calendar backends are
already wired: `GoogleAccountFactory`, `GraphAccountFactory`,
`JmapAccountFactory` each open an `Account` whose calendar primitives
hit the provider's native calendar API
(`research/bifrost/crates/{google,graph,jmap}/src/account/calendar.rs`).
DAV is a STANDALONE account: `CalDavAccountFactory::new(CalDavConfig {
base_url, credentials: CalDavCredentials::{Basic | Bearer{token_source}}
})` opens a `CalDavAccount` (`research/bifrost/crates/caldav/src/lib.rs`
lines 83-127, `account.rs` lines 31-56). A7 (landed) made DAV
first-class; the CalDAV account discovers calendar-home / outbox / rsvp
email at open.

### 2.3 The DB cache contract + id-translation seam (what is preserved)

The read-sync output contract is the DB cache; the app view loads from
it (`crates/app/src/db/calendar.rs::load_calendar_events_for_view`,
windowed read + off-lock RRULE expansion). Schema
(`crates/db/src/db/schema/05_calendar.sql`):

- `calendars`: PK `id` (ratatoskr-minted), `UNIQUE(account_id,
  remote_id)`. `remote_id` is the provider-native calendar id (Google
  calendarId, Graph calendar id, JMAP calendar id, CalDAV collection
  href). `sync_token` / `ctag` are the per-provider delta cursors.
  `provider` TEXT, `can_edit`, `is_primary`, `is_visible`.
- `calendar_events`: PK `id`, `UNIQUE(account_id, google_event_id)`.
  `google_event_id` is the dedup key (CalDAV synthesizes
  `caldav:<uid>` / `caldav:<uid>::recurrence-id=<rid>` via
  `make_google_event_id`; native providers use the provider event id).
  `remote_event_id` is the wire id used for write-back; `etag` for
  concurrency; `calendar_id` FK; `recurrence_id` canonical string.
- `caldav_event_map`: `(calendar_id, uri)` -> `(event_uid, etag)`, the
  CalDAV incremental diff index.

The seam these tables imply, made concrete in a new module
`crates/calendar/src/idmap.rs` (owned by B7a, reused by B7b):

- `calendar_remote_id(&Calendar) -> String` = `calendar.native_id`
  (== `calendar.id.0`; the engine-facing `CalendarId` and `native_id`
  coincide for all four backends in frozen bifrost - confirm at
  implementation, § 8 O1, since a divergence would change which value
  keys `calendars.remote_id`).
- `event_dedup_key(&CalendarEvent) -> String`: CAUTION (O13) - this
  CANNOT be a naive `event.native_id` for native providers. Confirmed
  against the pinned commit, bifrost Google and Graph set
  `native_id = join_event_id(calendar_id, event_id)` - a COMPOSITE
  `calendar_id::event_id` (`research/bifrost/crates/google/src/account/calendar.rs`
  line 404, `research/bifrost/crates/graph/src/account/calendar.rs`
  line 443), whereas legacy ratatoskr stored the BARE provider event id
  as `google_event_id` (`crates/calendar/src/google.rs` line 590). Using
  the composite verbatim re-keys every native row and orphans the entire
  existing cache. So for native providers the dedup key must STRIP the
  calendar prefix back to the bare event id (split on the join separator)
  to preserve legacy keys, or B7a must own an explicit one-shot cache
  migration - O13 picks the strip. CalDAV
  (`provenance.provider == ProtocolKind::CalDav`) ->
  `make_google_event_id(uid, recurrence_id)` preserving today's
  `caldav:` keying so an in-place migration does not orphan cached rows,
  with `uid = ev.uid` falling back to `href_synthetic_uid(&ev.native_id)`
  when the VEVENT carries no UID (today's fallback). Both
  `make_google_event_id` and `href_synthetic_uid` MOVE from sync.rs into
  `idmap.rs` verbatim (not duplicated), and their unit tests (sync.rs
  lines 986-1016) move with them and must keep passing.
- `event_remote_id(&CalendarEvent) -> String`: the write-back wire id.
  For CalDAV this is the resource href (`native_id`), matching today's
  `remote_event_id = uri`. For native providers, write-back needs the
  BARE provider event id, so this likewise strips the composite
  `native_id` prefix (O13) rather than storing `calendar_id::event_id`.
- A back-translation `event_id_for_writeback(provider, remote_event_id,
  etag, calendar_remote_id) -> (EventId, Option<CalendarId>)` that B7b
  consumes to drive `event_update`/`event_delete`/`event_rsvp` from a DB
  row. The `provider` (a typed provenance input, not an inferred one) is
  MANDATORY: reconstructing a bifrost `EventId` requires re-composing the
  `calendar_id::event_id` join for Google/Graph while passing the bare
  href/id through for CalDAV/JMAP (the inverse of the O13 strip), and the
  helper cannot pick the right shape without knowing the backend. An
  earlier draft omitted `provider` (R2-4, § 11); that signature cannot
  support B7b and is corrected here.

This module is the ONLY place protocol identity is reasoned about
consumer-side; everywhere else is provider-agnostic.

### 2.4 Account-factory gap: calendar construction is NOT wired today

`crates/service/src/bifrost/factory.rs::build_account_factory` routes on
the MAIL provider column only (`MailProviderKind::{Gmail, Graph, Jmap,
Imap}`) and wires NO calendar/DAV composition. It produces the mail
`Arc<dyn AccountFactory>` used by the B3/B4 engine. For three providers
that is already enough: a Gmail/Graph/JMAP mail account's bifrost
`Account` exposes native calendar primitives. But the CalDAV case is
NOT reachable through `build_account_factory`: a CalDAV calendar is a
standalone `CalDavAccountFactory`, and ratatoskr models it on the
`accounts` row via `calendar_provider = 'caldav'` (or `provider =
'caldav'`) plus a `caldav_url` and credentials, independent of the mail
provider (an IMAP or even Gmail mail account can carry a separate CalDAV
calendar).

This is the central construction obstacle and it is resolved inline
(§ 4.1) by a NEW Service-side router `build_calendar_account_factory`
that reuses the credential-read machinery but routes on the calendar
precedence from sync.rs lines 74-97, returning the native mail factory
for gmail/graph/jmap and a `CalDavAccountFactory` for caldav. It is a
new function, not a contortion of the mail factory, because the routing
axis differs (calendar provider, not mail provider) and the CalDAV
config path (url + bearer/basic credentials, OAuth via the shared
`DbWriteBackTokenSource`) is calendar-specific.

This is consistent with § 2's first principle: bifrost already exposes a
uniform `Account` calendar surface and a first-class `CalDavAccountFactory`
(A7), so no bifrost wart is being worked around - the router is pure
ratatoskr account-identity resolution that bifrost cannot do for us
(it does not own the `accounts` table).

### 2.5 The two questions B7 left to this spec's survey (resolved)

The B7 decomposition note (`docs/bifrost-migration.md` lines 1164-1174)
parks two questions for the B7a author:

- **(i) CalDAV change-stream asymmetry.** bifrost-caldav DOES emit
  `CursorScope::Type(CalendarEvent)` through a `changes_stream`, while
  the HTTP providers expose calendar only via the pull surface.
  Resolution (per B7 § 2): B7a drives ALL calendar uniformly through
  `calendars_list` + `events_in_range`. The CalDAV change-stream is NOT
  consumed by the calendar runtime; it is at most a future optimization
  or a bifrost side-quest, never a consumer-side per-provider special
  case. The calendar runtime is a direct PULL runtime (the `SyncEngine`
  multiplexer is mail-only), so there is nothing to wire the
  change-stream into without building a calendar consumer - explicitly
  out of scope (§ 5).
- **(ii) iMIP scope.** The email-embedded ICS path
  (`crates/common/src/email_parsing.rs`: `has_meeting_invite`, REQUEST /
  REPLY / CANCEL method detection, RSVP-from-reading-pane) is parsed
  app-side and routes RSVP through the write path. Resolution: iMIP is a
  WRITE concern (it produces an RSVP action), so it belongs to B7b
  (which rewires RSVP onto `event_rsvp`), NOT B7a. B7a touches no
  email-parsing path. This spec records the boundary so B7b's author
  picks it up; it is named-and-excluded, not deferred (clause 3).

### 2.6 The range-window obstacle (delta-whole-calendar vs range-pull)

PREMISE CORRECTION (adjudicated, § 12): earlier drafts claimed "today
every provider helper syncs the WHOLE calendar" - that is only half
true, and the half matters for the history ruling. Verified against the
code: legacy Google and Graph BOTH seed their initial sync at
`[now - 90d, now + 365d]` (`crates/calendar/src/google.rs:157-158`,
`crates/graph/src/calendar_sync.rs:336-337`) and then ride sync-token /
delta-token deltas anchored to that never-re-anchored window - so
today's client has NEVER cached unbounded Google/Graph calendar
history. Only JMAP (`fetch_all_events` + `CalendarEvent/changes`) and
CalDAV (whole-collection CTag/ETag diff) are genuinely whole-calendar.
Bifrost's pull surface is `events_in_range(EventRange { start, end,
... })` - a bounded, paged time-window query, not a whole-calendar
delta. There is no `events_changed_since` primitive in frozen bifrost.

This is a genuine model change and is resolved inline, not deferred:

- B7a syncs a **rolling cache window**: `[now - BACK, now + FORWARD]`,
  pinned as `CalendarSyncWindow { back: Duration, forward: Duration }`
  with concrete defaults `back = 365 days`, `forward = 730 days`. The
  defaults are chosen so the cache always covers the app's view window
  (`load_calendar_events_for_view` is itself windowed and is the only
  reader) with margin for "jump forward a year" navigation, while
  bounding per-run cost. The window is a single constant in
  `crates/calendar/src/sync.rs`, not an env var or runtime knob
  (clause: cleanliness - no scaffolding). The window is anchored at an
  INJECTED `now_ms` parameter (see the § 4.2 signature), not a
  `SystemTime::now()` read inside the body, so harness runs against
  fixed-date fixtures are deterministic.
- **History backfill (adjudicated, § 12 ruling 1).** The rolling window
  alone would cap fresh-setup history at 1 year, regressing JMAP/CalDAV
  (unbounded today) against the product's 5-year history mandate. B7a
  therefore adds a ONE-TIME per-calendar history backfill: after a
  calendar's active-window pull + reconcile succeed, and while
  `calendars.history_backfilled_at` (a new nullable INTEGER column,
  § 4.5a) is NULL, pull `[now - 1825d, now - 365d)` as four consecutive
  365-day `events_in_range` slices (oldest first; fixed 365-day slices
  dodge any provider range-span cap without a per-provider branch),
  translating + upserting each slice UPSERT-ONLY - the deletion
  reconcile NEVER runs over backfill ranges. When all four slices
  succeed, stamp `history_backfilled_at = now_ms`; any slice error
  leaves the stamp NULL (idempotent retry next kick) and propagates as
  the run's `result: Err` AFTER the active-window work has committed.
  `full_resync` clears the stamp along with its force-clear. Backfilled
  rows sit outside the active window, so the "outside the window are
  left untouched" rule below is what PRESERVES them thereafter. Net
  coverage: Google/Graph gain history versus today's legacy `-90d`
  anchor; JMAP/CalDAV keep 5 years on a fresh setup (beyond-5y archive
  events on a fresh setup are the one recorded residue - § 12 ruling 1
  signs that boundary off; rows already cached by a legacy install are
  never deleted regardless of age).
- Each `events_in_range` page is translated and upserted as it arrives
  (streaming upsert, bounded memory - the enterprise volume requirement
  in AGENTS.md); paging follows `Page::next_cursor` until exhausted,
  with a cancellation point between pages. CAVEAT (O11): the streaming /
  bounded-memory property only holds for backends that actually page.
  Frozen bifrost CalDAV does NOT: `events_in_range` ignores
  `page_cursor`, runs one whole-collection query, applies `limit` by
  `Vec::truncate`, and returns `Page::single` (no `next_cursor`)
  (`research/bifrost/crates/caldav/src/account.rs` lines 717-720). So for
  CalDAV the loop is a single page, must pass `limit: None` (a `limit`
  would silently DROP in-window events rather than page them), and the
  whole windowed collection lands in memory at once. Acceptable for the
  rolling window, but the per-backend asymmetry must be stated, not
  assumed uniform.
- Deletion reconciliation is **windowed**, not whole-calendar (§ 4.5):
  a cached event that falls inside `[now - BACK, now + FORWARD]` and is
  absent from the freshly-pulled windowed set is deleted; cached events
  OUTSIDE the window are left untouched (they were synced by an earlier,
  differently-positioned window and the current pull says nothing about
  them). This is the provider-agnostic generalization of today's CalDAV
  "diff the listing" delete. It covers the PAGE-LEVEL failure guard by
  construction (a page fetch error aborts the run before any delete - no
  partial-window delete). It does NOT, however, subsume the legacy CalDAV
  EMPTY-RESULT guard - CORRECTION (R2-1, § 11): an earlier draft claimed
  "an empty windowed pull deletes only window-resident cached rows" was
  the provider-agnostic form of that guard. It is the INVERSE. Legacy
  CalDAV explicitly SKIPS the whole deletion step when the server returns
  zero events but the local cache is non-empty (sync.rs:677, treating a
  successful-but-empty 207 as a suspected transient failure), whereas the
  windowed reconcile as first written would DELETE every in-window cached
  row on a successful-but-empty pull - a mass-delete on exactly the case
  legacy protected. This must carry the guard forward (see O3/O16 in
  § 11), not "subsume" it. It also does NOT subsume the legacy CalDAV
  PER-RESOURCE failure guard by construction -
  see the corrected O3 below. Frozen bifrost CalDAV `events_in_range`
  silently drops a malformed/transiently-bad resource via `filter_map(...
  .ok())` and returns `Page::single` with no `failed_hrefs` and no error
  (`research/bifrost/crates/caldav/src/account.rs` lines 704-720), so such
  a resource appears as plain ABSENCE - indistinguishable from a real
  remote delete - and the windowed reconcile WOULD delete the cached row.
  That is a genuine regression versus today's per-resource-preservation
  guard (sync.rs lines 692-704), now confirmed against the pinned commit,
  and its mitigation is tracked as O3 (no longer "confirm whether"; it is
  "confirmed broken, here is the fix").

Recurring masters are the one wrinkle: a master with `DTSTART` before the
window but instances inside it must still be cached so the load-path
RRULE expansion can render the in-window instances. This holds per
backend (O2 RESOLVED, § 8): Google/Graph expand instances server-side,
JMAP matches occurrences server-side and returns the master, and the
CalDAV local-filter defect that dropped an out-of-window master is
fixed by prerequisite SQ-2 (§ 12). The recurrence gates
(`*-calendar-recurrence-initial.lua`, already present) pin it, with an
out-of-window-master fixture for CalDAV.

With the backfill in place, the observable window boundary moves to
[-5y, +2y]. A correctness caveat on the window itself (O12): calendar
navigation does NOT trigger a provider re-sync. `PrevMonth` /
`NextMonth` / `Today` only mutate local view state and call
`reload_calendar_events`, which reads the DB window ONLY
(`crates/app/src/handlers/calendar.rs` lines 27-37, 717-727); provider
fetch happens only on the hourly kick and explicit `start_calendar_sync`.
Navigating beyond -5y / +2y shows an empty (or stale, for
backfilled-then-never-reconciled history) view that navigation cannot
fill. This is the adjudicated stopping point (§ 12 ruling 1): within
the product's own "5+ years" number, and strictly wider than legacy
Google/Graph coverage. If real-world use later wants unbounded caching
or navigation-widened sync, the named remedy is the bifrost
`events_delta` side-quest recorded in § 12 (a capability-flagged
changed-since primitive), not a B7a contortion. One Graph caveat is
folded into O0's pre-cut survey: the active window is a single
1095-day `calendarView` span, wider than the 455-day span legacy Graph
ever issued - the survey must confirm Graph (and saehrimnir's mock)
accept it, and if a span cap surfaces, the pre-stated mechanical
fallback is to run the ACTIVE window pull as three 365-day slices for
all backends (one seen-set + one reconcile per calendar per run either
way; the backfill already runs sliced).

## 3. The split and ordering (clause 6: keep/revert, green at every boundary)

B7a is ONE landing (the B7 note: no per-provider cutover - the axis
collapses, so there is nothing to stage provider-by-provider). The
ordering inside the single commit that keeps `brokkr check` green:

1. Add the seam module `crates/calendar/src/idmap.rs` and the new
   translation helpers (additive; nothing calls them yet).
2. Add `build_calendar_account_factory` in
   `crates/service/src/bifrost/factory.rs` (additive).
3. Rewrite `calendar_sync_account_impl` body onto the bifrost surface,
   keeping its signature's OUTPUT (`CalendarSyncOutcome`) and the
   `&mut bool mutated` contract, but changing its INPUTS from
   `(gmail, graph, jmap: &*State)` to the bifrost factory router (a
   pre-1.0 internal signature change is legal, clause 4). Update the one
   caller `run_calendar` in `crates/service/src/calendar.rs` and the
   `CalendarRuntime` construction to drop the per-provider `*State`
   registries and carry what the factory router needs (read DB, writer
   pool, encryption key - already on `CalendarRuntimeInner`).
4. Delete the four per-provider sync impls and now-orphaned modules
   (§ 5), in the same commit, so no parallel surface survives (§ 1
   maximal integration).

Because the delete and the rewire are the same commit, there is no
intermediate state with two calendar read paths. The keep/revert unit is
the whole commit, gated by § 6.

## 4. The bricks (concrete artifacts)

### 4.1 `build_calendar_account_factory` (Service-side router)

New function in `crates/service/src/bifrost/factory.rs`:

```rust
pub async fn build_calendar_account_factory(
    db: &ReadDbState,
    writer: WriterPool,
    account_id: &str,
    encryption_key: [u8; 32],
) -> Result<Option<Arc<dyn AccountFactory>>, BifrostBuildError>;
```

Returns `Ok(None)` when the account has no calendar backend (no
`calendar_provider`, no caldav_url, and a mail provider with no native
calendar) - the runner treats `None` as a clean no-op, mirroring today's
`_ => Err("No calendar provider configured")` but as a non-error skip
(the current code returns an Err there, but the runner's emission rules
make that observably a failed run; a no-calendar account is not a
failure - confirm against today's UX, § 8 O5).

Routing - and this is a DELIBERATE behavior change from sync.rs lines
74-97, not a preservation (O10): today's fixed-order chain lets the mail
`provider` short-circuit (a `gmail_api`/`graph` mail account with
`calendar_provider = "caldav"` resolves to google/graph today, losing the
CalDAV intent - § 2.1). B7a makes `calendar_provider` GENUINELY win over
the mail provider, so the calendar axis is independent of the mail axis
(the § 6 router gate seeds exactly this gmail-mail + caldav-calendar row
and asserts a `CalDavAccountFactory`). This is a routing-semantics
change, not merely the pre-1.0 signature change clause 4 covers; it is
flagged as an intentional fix with its own gate (O10) rather than
smuggled in under "preserve precedence." The router therefore checks the
caldav arm on `calendar_provider == "caldav"` BEFORE falling through to
the mail-provider arms:

- `calendar_provider == "caldav"` OR (`provider == "caldav"` and
  non-empty `caldav_url`): build `CalDavAccountFactory::new(CalDavConfig
  { base_url: <caldav_url>, credentials })`. Credentials - CORRECTION
  (R2-2, § 11): CalDAV auth is NOT derived from the mail account's auth.
  The `accounts` row carries DEDICATED `caldav_username` / `caldav_password`
  columns (schema `01_core.sql:36-37`) and there is no CalDAV auth-method
  column; today's CalDAV path always reads those and uses Basic auth
  (`crates/calendar/src/caldav/mod.rs:251`). So the common Gmail-mail +
  CalDAV-calendar account has OAuth MAIL creds but BASIC CalDAV creds, and
  the earlier draft's "OAuth accounts use `bearer_source` from the shared
  mail `DbWriteBackTokenSource`" would send the wrong credential type. The
  router MUST build `CalDavCredentials::Basic { username, password }` from
  the decrypted `caldav_username`/`caldav_password` columns. (A bearer/
  `token_source` path is only correct if a CalDAV row genuinely stores an
  OAuth-bearer CalDAV credential - not inferred from the mail provider;
  absent a caldav-auth-method column, Basic is the only wired path.) This
  also requires EXTENDING the shared credential-read helper: the existing
  factory credential row does not load the CalDAV columns
  (`factory.rs:266`), so the refactor in the paragraph below must add them.
  Honor a `RATATOSKR_TEST_CALDAV_ENDPOINT` override
  symmetric with the mail factory's test-endpoint handling (the
  caldav harness fixtures need it - confirm the existing
  `caldav-calendar-*.lua` endpoint env var name and reuse it, § 8 O6).
- `calendar_provider == "google_api"` OR `provider == "gmail_api"`:
  reuse the existing Gmail arm of `build_account_factory`
  (`GoogleAccountFactory`).
- `calendar_provider == "graph"` OR `provider == "graph"`:
  reuse the Graph arm.
- `calendar_provider == "jmap"` OR `provider == "jmap"`:
  reuse the JMAP arm.

Refactor the credential-read + decrypt + native-factory construction in
`build_account_factory` into shared helpers so both routers call them
(no duplicated credential logic - clause: no parallel hand-rolled
surface). The router returns `Arc<dyn AccountFactory>`; the runner calls
`factory.open(AccountId::from(account_id))` to get the `Arc<dyn Account>`.

### 4.2 The rewritten `calendar_sync_account_impl`

Signature output unchanged; inputs swapped to the factory router. New
body (provider-agnostic):

```rust
pub async fn calendar_sync_account_impl(
    account_id: &str,
    write_db: &WriteDbState,
    read_db: &ReadDbState,
    factory: Arc<dyn AccountFactory>,   // from build_calendar_account_factory
    now_ms: i64,                        // injected clock; anchors the window
    cancellation_token: &CancellationToken,
) -> CalendarSyncOutcome
```

(the runner builds the factory and short-circuits to a no-op
`CalendarSyncOutcome { mutated: false, result: Ok(()) }` when the router
returns `None`, so this fn always receives a real factory. `now_ms` is
injected rather than read via `SystemTime::now()` inside the body so the
rolling window is deterministic under the harness clock: the
CalendarRuntime passes real wall-clock ms; tests pass a fixed instant.)

Flow:

1. `cancelled?` -> early `Err("calendar sync cancelled")`, `mutated:
   false` (unchanged).
2. `let account = factory.open(account_id.into()).await` mapping
   `AccountError` -> `String` via the existing
   `error_map` (`crates/service/src/bifrost/error_map.rs`); on Err,
   `CalendarSyncOutcome { mutated: false, result: Err(_) }`.
3. `if !account.capabilities().pim_methods.calendars_list { return
   Ok no-op }`.
4. `let calendars = account.calendars_list().await?` ->
   `upsert_discovered_calendars` (§ 4.3); set `mutated = true`.
5. `let visible = load_visible_calendars(read_db, account_id).await?`
   (the existing query, sync.rs lines 285-309; visibility is a ratatoskr
   user toggle, not a provider concept). CORRECTION (R2-3, § 11): this set
   is per-`account_id` and provider-BLIND (`WHERE account_id = ?1 AND
   is_visible = 1`, sync.rs:297). It can contain STALE rows from a
   different backend - e.g. after the O10 precedence fix reroutes a
   Gmail-mail account's calendar from Google to CalDAV, the previously
   discovered Google calendar rows (Google calendarIds) are still in this
   set, and discovery upsert only refreshes metadata, it does not remove
   or re-provider them (`calendar_contacts_writes.rs:63`). Feeding a
   Google calendarId to `CalDavAccount.events_in_range` will error/abort
   the run. So step 6 MUST iterate only the INTERSECTION of `visible` with
   the calendars just returned by `calendars_list()` (keyed on the
   translated `remote_id`), and B7a MUST define a stale-row policy for
   calendars in the cache that the current backend no longer lists (reap
   vs mark-hidden). See O17 (§ 11).
6. For each surviving (listed-and-visible) calendar: cancellation
   checkpoint; page
   `events_in_range` over the window (§ 2.6); translate + upsert each
   page (§ 4.4); set `mutated = true` after the calendar's applies;
   windowed-delete reconcile (§ 4.5).
7. For each surviving calendar whose `history_backfilled_at` is NULL:
   the one-time history backfill (§ 2.6) - four 365-day upsert-only
   slices over `[now - 1825d, now - 365d)` with a cancellation
   checkpoint between slices, `mutated` per O20, NO reconcile, stamp
   `history_backfilled_at = now_ms` only when all four slices
   succeeded. Runs after step 6 so freshness always precedes
   archaeology; a backfill error surfaces in `result` but cannot undo
   the committed active-window work.
8. `CalendarSyncOutcome { mutated, result }`.

The `&mut bool mutated` is replaced by a local `mutated` accumulator
returned in the outcome - same observable contract, cleaner ownership
(the old `&mut bool` threaded through 4 helpers; one body needs no
out-param).

### 4.3 Calendar translation (`Calendar` -> `DiscoveredCalendar`)

In `crates/calendar/src/idmap.rs`:

```rust
fn to_discovered_calendar<'a>(account_id: &'a str, provider: &'a str,
    cal: &'a bifrost_types::Calendar) -> DiscoveredCalendar<'a>
```

mapping `remote_id = cal.native_id`, `display_name = Some(&cal.name)`,
`color = cal.color.as_deref()`, `is_primary = cal.is_default`,
`can_edit = cal.can_update_events`. `provider` is the canonical
ratatoskr provider string for the calendars.provider column - derive it
from `cal.provenance.provider` (`ProtocolKind` -> "google" / "graph" /
"jmap" / "caldav") so it matches today's stored values (sync.rs passes
"google" / "graph" / "caldav" literals). Upsert via the existing
`upsert_discovered_calendar` in a single write txn (the existing
`upsert_discovered_calendars_impl` shape, sync.rs lines 168-196).

### 4.4 Event translation (`CalendarEvent` -> `CalendarEventRow` + rows)

In `crates/calendar/src/idmap.rs`, the load-bearing translation:

```rust
fn to_event_row(account_id: &str, calendar_id: &str,
    ev: &bifrost_types::CalendarEvent) -> CalendarEventRow
fn to_attendees(account_id: &str, dedup_key: &str,
    ev: &CalendarEvent) -> Vec<CalDavAttendee>      // reuse existing row types
fn to_reminders(...) -> Vec<CalDavReminder>  // from ev.reminders (SQ-3): minutes_before + method,
                                             // legacy parity for CalDAV VALARM and JMAP alerts (O7 RESOLVED)
```

Field mapping (the careful parts get unit gates, § 6):

- `google_event_id = event_dedup_key(ev)` (§ 2.3); `remote_event_id =
  event_remote_id(ev)` (§ 2.3) - NOT the raw `ev.native_id`. CORRECTION
  (R2-4, § 11): an earlier draft wrote `remote_event_id = ev.native_id`,
  which contradicts § 2.3's strip rule and would persist the composite
  `calendar_id::event_id` for Google/Graph, breaking legacy write-back
  and the scripts' `remote_event_id` DB-value assertions. BOTH the dedup
  key and the write-back id go through the `idmap` helpers so the strip
  is applied in exactly one place; no field-mapping call site reads
  `ev.native_id` directly. `etag = ev.etag`; `uid = ev.uid`.
- `start_time` / `end_time`: `event_time_to_epoch(&EventTime,
  is_all_day) -> Option<i64>` - `None` on an unparseable value, and the
  caller then REFUSES to persist the row (mirroring sync.rs lines
  853-858) rather than writing an epoch-0 event; a unit gate (§ 6) pins
  the `None` path. The interpretation rule is ADJUDICATED (O19
  RESOLVED, § 12 ruling 5) and is ONE uniform rule for all four
  backends, written against the post-SQ-1 `EventTime` contract:
  - TIMED value carrying an offset or `Z`: parse RFC 3339 -> epoch.
  - TIMED bare wall-clock value with `timezone: Some(zone)` (the CalDAV
    TZID shape, `caldav/src/ical.rs:416`): resolve via `chrono_tz`
    (already in the tree - lift the resolution approach from the legacy
    `crates/core/src/caldav/parse/ical/mod.rs` TZID ladder). DST gap ->
    the next valid instant; DST ambiguity -> the earlier offset. Both
    cases get unit gates (§ 6).
  - TIMED floating (bare value, `timezone: None`): interpret as UTC.
  - ALL-DAY: `start_time` = midnight of the start date in
    `ev.start.timezone` (UTC when None); `end_time` = `start_time +
    days * 86400` where `days` = (exclusive end date - start date) -
    the legacy CalDAV convention
    (`crates/core/src/caldav/parse/ical/mod.rs:221-227`), which is
    host-tz-independent and DST-stable by construction. Post-SQ-1 every
    backend delivers the all-day end as the EXCLUSIVE date (the caldav
    decrement is removed), so a one-day event yields `days = 1`, never
    zero-duration, and no double-adjust exists to make. NOTE this
    deliberately REPLACES the legacy Google path's `chrono::Local`
    midnight / 23:59:59-of-exclusive-date form
    (`crates/calendar/src/google.rs:541-566`) - that form is the
    host-tz-dependent hazard the schema comment (lines 56-69) warns
    about and was inconsistent with the CalDAV-path convention; the
    normalization onto the CalDAV form is intentional, and any harness
    DB-value assertion that pinned the old Google-path all-day epochs
    is updated to the tz-independent values (called out in the landing
    notes; the caldav fixtures already pin the kept convention).
  Carry `ev.start.timezone` into `timezone`. The translation must NOT
  round-trip through `chrono::Local` anywhere.
- `recurrence_rule = ev.recurrence.rrule`; `recurrence_id =
  canonicalize(ev.recurrence.recurrence_id)` into the schema's canonical
  wall-clock string forms (reuse / lift today's canonicalizer rather
  than re-deriving). CAUTION (O14): `ev.recurrence.recurrence_id` is NOT
  uniformly a RECURRENCE-ID occurrence discriminator across backends.
  Confirmed against the pinned commit, bifrost Graph maps Graph's
  `seriesMasterId` into `recurrence_id`
  (`research/bifrost/crates/graph/src/account/calendar.rs` line 497),
  which is the parent SERIES id, not the wall-clock instance that
  RECURRENCE-ID encodes. Blindly canonicalizing it as the schema's
  occurrence discriminator would corrupt the phantom-dedup key. The
  translation must only canonicalize a genuine RECURRENCE-ID value, and
  treat the Graph `seriesMasterId` case distinctly (do not persist it as
  an override key); the recurrence unit gate (§ 6) must cover the Graph
  shape, not only the iCal/CalDAV shape. `rdate` / `exdate`: today's
  schema has no columns -
  confirm whether the load-path expansion needs them (it currently
  reads only `recurrence_rule`), record as O8 if a column add is
  warranted; B7a does not add columns (stopping rule § 5).
- `status` (`EventStatus` -> "confirmed" / "tentative" / "cancelled"),
  `availability` (`EventAvailability` -> existing string),
  `visibility`. These bifrost enums are `#[non_exhaustive]` and carry
  variants the three-way enumeration above omits: `EventStatus::Unknown`,
  `EventAvailability::{Tentative, OutOfOffice, Unknown}`,
  `RsvpStatus::{Delegated, Unknown}`, `EventVisibility::Confidential`. The
  translation must be a TOTAL mapping with an explicit fallback for each
  (and, because the enums are `#[non_exhaustive]`, a wildcard arm), so an
  `Unknown` / `Delegated` / `Confidential` / `OutOfOffice` value maps to a
  defined string instead of silently taking whatever a catch-all picks.
  Add a unit gate (§ 6) pinning the fallback mapping.
  `organizer_email`/`organizer_name`,
  `attendees_json` (serialize `ev.attendees` to today's
  `[{email, displayName, responseStatus}]` JSON shape, matching sync.rs
  lines 835-851), `html_link`, `ical_data = ev.raw_ical`,
  `rsvp_status = ev.self_response` mapped to today's string,
  `summary`/`title = ev.title`.

Upsert via the existing `upsert_calendar_event_row` +
`sync_caldav_attendees` + `sync_caldav_reminders` in one txn per page.
The DB helpers are unchanged. Multi-VEVENT CalDAV resources (O15
RESOLVED, § 12 ruling 3): post-SQ-2, bifrost CalDAV projects EVERY
VEVENT of a multi-VEVENT `.ics` resource (master + override instances +
CANCELLED exceptions, each with its own `recurrence_id` from
RECURRENCE-ID and `status`), restoring legacy parity
(`crates/calendar/src/sync.rs` line 746 wrote every VEVENT). The
translation therefore persists one row per projected `CalendarEvent`,
keyed per override via `make_google_event_id(uid, recurrence_id)` -
the existing phantom-dedup keying - and a CANCELLED override persists
as a `status = "cancelled"` row exactly as legacy did. The § 6
recurrence gates run against a multi-VEVENT CalDAV fixture to pin
this.

### 4.5 Windowed deletion reconcile (provider-agnostic)

Per calendar, accumulate the dedup keys seen across all pages of the
window pull into a `HashSet<String>`. After the pull completes
successfully (NOT if any page errored - a fetch error aborts the run
before reconcile, preserving today's "don't delete on transient
failure" guarantee):

- Query cached `calendar_events` for this `calendar_id` that INTERSECT
  `[now - BACK, now + FORWARD]`. CORRECTION (R2-7, § 11): "whose
  `start_time` falls inside the window" is too narrow - it misses a
  non-recurring event that STARTS before the window but OVERLAPS it, and
  leaves remotely-deleted recurring series cached forever. The candidate
  predicate must be an OVERLAP test (`start_time < window_end AND end_time
  > window_start`), and recurring masters must be handled specially -
  loaded regardless of their `DTSTART` position and reconciled against
  whether the pull still returned the master (mirror the view query, which
  already does exactly this:
  `crates/db/src/db/queries_extra/calendars/view/mod.rs:82-86` selects
  `recurrence_rule IS NOT NULL OR (start_time < window_end AND end_time >
  window_start)`). The upstream range-semantics prerequisite is CLOSED
  (O2 RESOLVED, § 12): Google/Graph expand instances server-side, JMAP
  matches occurrences server-side, and the CalDAV local-filter defect
  (an out-of-window master with in-window instances dropped by
  `account.rs::event_in_range`) is fixed by prerequisite SQ-2, so a
  still-live recurring master always comes back in the pull and a
  recurring row absent from the seen set is safely a real delete.
- Delete the cached in-window rows whose `google_event_id` is NOT in the
  seen set, via `delete_calendar_event_by_remote_id` / a new
  windowed-delete query keyed on dedup key (add to
  `calendar_contacts_writes.rs` if no existing query fits - that is a
  DB-crate brick, gated by a `brokkr test -p db` query test).
- Empty windowed pull -> a naive reconcile deletes ALL in-window cached
  rows. This is NOT the CalDAV empty-207 guard - it is its inverse (R2-1,
  § 11). Legacy skips deletion entirely when a successful pull returns
  zero events against a non-empty cache (sync.rs:677). ADJUDICATED
  (O16 RESOLVED, § 12 ruling 4): B7a reproduces that skip,
  provider-agnostically: if a calendar's windowed pull yields zero
  events while the cache holds in-window rows for it, treat the pull as
  a suspected transient failure and skip the delete step for that
  calendar (the `full_resync` path stays the intentional force-clear).
  No protocol offers a genuine server-attested completeness signal, so
  the legacy heuristic - today's shipped behavior - is the correct
  consumer-side complement to the per-resource signal below, kept as
  defense-in-depth for all four backends.
- CalDAV per-resource failure (O3 RESOLVED, § 12 ruling 4): post-SQ-4,
  bifrost CalDAV `events_in_range` no longer `filter_map(... .ok())`-
  swallows a resource that could not be fetched or projected; the
  failed set surfaces on the result (the same `failed_hrefs` machinery
  the snapshot path already has, `caldav/src/account.rs:146`). The
  reconcile MUST subtract the failed set from the absence computation:
  a cached row whose dedup key maps to a failed resource is PRESERVED
  (legacy per-resource-preservation, sync.rs 692-704), while a row that
  is absent AND not failed is a true remote delete and propagates -
  exactly the transient-vs-real distinction R1-6 demanded, now driven
  by an authoritative signal instead of a heuristic. Backends that
  report no failed set (the HTTP providers) contribute an empty set and
  flow through the same code path - no per-provider branch. The
  `caldav-calendar-remote-delta.lua` gate carries both cases: a
  transiently-failing resource (preserved) and a real remote delete
  (propagated).
- The backfill ranges (§ 2.6) are exempt: reconcile runs over the
  active window ONLY. Out-of-window rows (including everything the
  backfill wrote) are never deletion candidates (O22).

CTag/sync-token short-circuit (today's optimization, sync.rs lines
603-612): frozen bifrost's `events_in_range` does not expose a
collection-level "unchanged since CTag" skip. B7a drops the
client-side ctag-skip optimization (the windowed pull is the source of
truth); the `calendars.ctag`/`sync_token` columns become dormant for
read-sync (they stay for B7b / future use, not removed - § 5). The
structural cost consequence is ADJUDICATED (O9 re-ruled, § 12 ruling
1): see O9 in § 8 - the full-window hourly pull is accepted, the
sync-bench baseline is recorded on the NEW path as an absolute
regression guard, and the named future remedy is the bifrost
`events_delta` side-quest, not a consumer-side ctag cache.

### 4.5a The backfill marker column (schema brick)

`crates/db/src/db/schema/05_calendar.sql` (and the single v100
migration in `crates/db/src/db/migrations.rs`): add
`history_backfilled_at INTEGER` (nullable, default NULL) to
`calendars`. NULL means the one-time history backfill (§ 2.6) has not
completed for that calendar; the sync body stamps it with `now_ms` on
backfill completion, and the `full_resync` force-clear resets it to
NULL. This is the ONE deliberate schema addition in B7a (an adjudicated
amendment to the § 5 "no schema enrichment" stopping rule - a marker,
not an enrichment; rdate/exdate/three-way-rights stay out). Gated by a
`brokkr test -p db` query test alongside the windowed-delete query.

### 4.6 Runner + runtime rewire

`crates/service/src/calendar.rs`:

- `CalendarRuntime::new` drops `gmail: GmailState, graph: GraphState,
  jmap: JmapState` and the per-provider client registries (lines
  123-125, 152-154). The runtime already holds `db: WriteDbState` and
  `read_db: ReadDbState`, but it does NOT hold the encryption key today:
  the doc comment at lines 116-122 is explicit that the `SecretKey` is
  consumed at construction and the key bytes survive only inside the
  `gmail`/`graph`/`jmap` `ProviderState` registries this rewire deletes.
  So the rewire must ADD a `key_bytes: [u8; 32]` (or `SecretKey`) field
  to `CalendarRuntimeInner` and keep the `SecretKey` -> `[u8; 32]`
  extraction in `new` feeding it - the field does not exist yet and
  cannot be relied on as "already held." (`WriteDbState::writer_pool()`,
  the other input the § 4.1 signature needs, IS already reachable - it is
  used at sync.rs line 341 - so no new plumbing is needed for the writer
  pool.)
- `run_calendar` (lines 436-499): build the factory via
  `build_calendar_account_factory`; on `Ok(None)` synthesize the no-op
  outcome; else call the rewritten `calendar_sync_account_impl`. The
  emission block (lines 469-498), the `mutated`/`CalendarChanged`
  contract, `last_completed` stamping, and `CalendarRunCompleted` are
  UNCHANGED.
- The module doc-comment's "RSVP send is the candidate" drain-ordering
  note (lines 36-48, 72-75) stays accurate (RSVP is B7b); no drain
  reshuffle.

The `calendar` crate's `Cargo.toml` does NOT drop the `gmail` / `graph` /
`jmap` / `rtsk::caldav` deps in B7a (correcting an earlier draft of this
section): the B7b write helpers stay in this landing, and
`crates/calendar/src/actions.rs` lines 25-37 still `use
gmail::client::GmailClient`, `graph::client::GraphClient`,
`jmap::client::JmapClient` and the per-provider `*_create/update/delete`
impls. Those deps can only be dropped when B7b cuts the write path.
B7a only ADDS the bifrost calendar type/account deps it needs (confined
to writer-side crates per § 7 - `calendar` is already a Service-side
crate, not pulled by the app through `rtsk`). The dep removal is recorded
as a B7b cleanup, not done here. Confirm `calendar` is not in
`rtsk`'s dependency tree (it is not today - actions.rs lines 1-9 note
adding `calendar` to `core` would be circular); if any bifrost type
leaks toward `core`, that is a § 7 violation to stop on.

## 5. Stopping rule (clause 9)

In scope: calendar READ sync (discovery + windowed event pull +
windowed delete reconcile) for all four backends, the factory router,
the id-translation seam, the runner/runtime rewire, and the deletion of
the per-provider read-sync code.

Deleted in this landing (the rip, § 1 maximal integration):

- `crates/calendar/src/google.rs`, `graph.rs`, `jmap.rs` (the calendar
  read-sync impls). Note `jmap.rs` / the per-provider files also host
  write helpers (`jmap::calendar_sync::create_event_remote` etc.) called
  by `actions.rs` (B7b) - delete ONLY the read-sync functions in this
  landing; the write helpers stay until B7b cuts them. If a file is
  purely read-sync it is removed; if mixed, only its read-sync items go.
  (Survey each file at implementation; this is the one place B7a and B7b
  share a file, so the cut is surgical.)
- The CalDAV read-sync machinery in `sync.rs`
  (`sync_caldav_*`, `run_caldav_sync_attempt`,
  `upsert_caldav_parsed_event_tx`, `load_stored_etags`,
  `load_calendar_ctag`, the CTag/ETag diff) and its dependence on
  `rtsk::caldav::client::CalDavClient` + `rtsk::caldav::parse` for SYNC.
  The `crates/core/src/caldav/` parse/client and `crates/calendar/src/caldav/`
  retire fully ONLY if nothing else (B7b write path, iMIP) still needs
  them - survey before deleting; if B7b's CalDAV write still uses
  `rtsk::caldav`, that deletion is B7b/B15, recorded here so it is not
  orphaned.

Deletion audit (clause 8 precision) - before the delete lands, run and
record a symbol sweep so the cut is exact and no live caller remains:
`git grep -n` each deleted read-sync symbol
(`google_calendar_list_calendars_impl`,
`google_calendar_sync_events_impl`, `graph_calendar_list_calendars_impl`,
`graph_calendar_sync_events_impl`, `sync_jmap_calendar_account`,
`sync_caldav_calendar`, `run_caldav_sync_attempt`,
`apply_calendar_sync_result_impl`, `upsert_caldav_parsed_event_tx`,
`load_calendar_ctag`, `load_stored_etags`) and confirm the only
remaining references are inside the deleted bodies; confirm the sole
caller of `calendar_sync_account_impl` is `run_calendar`; confirm the
`new_gmail_state` / `new_graph_state` / `new_jmap_state` constructions
are gone from `crates/service/src/calendar.rs`; and confirm
`make_google_event_id` / `href_synthetic_uid` have MOVED (not been
duplicated) into `idmap.rs`.

Explicitly OUT of scope (named, not deferred - clause 3):

- The calendar WRITE path (create/update/delete/RSVP) and the
  `CalendarProvider` enum dispatch in `crates/calendar/src/actions.rs` +
  `crates/service/src/cal_actions/` -> B7b.
- iMIP / email-embedded ICS RSVP -> B7b (§ 2.5).
- Consuming bifrost-caldav's `changes_stream` / building a calendar
  change-stream consumer -> not planned (§ 2.5 (i)); a future
  optimization at most.
- Unbounded calendar caching beyond [-5y, +2y] (§ 2.6 backfill) -> the
  named future remedy is the `events_delta` bifrost side-quest (§ 12),
  not done here.
- Schema enrichment (three-way write rights, rdate/exdate columns,
  reminders from NATIVE Google/Graph reminder APIs - legacy never
  synced those either) -> recorded as open items (§ 8), not done here.
  The `history_backfilled_at` marker column (§ 4.5a) is the one
  adjudicated exception; CalDAV VALARM + JMAP alert reminders are IN
  scope via SQ-3 (that is legacy parity, not enrichment).
- `calendars.ctag` / `sync_token` column removal -> not removed (dormant
  for read-sync; B7b / B15 decide final disposition).

## 6. Verification per brick (clause 5: exact, copy-pasteable gates)

A compile-only replacement of provider sync is under-gated and must be
rejected (B7 § 10). The behavioral gates are the per-provider
calendar sync-harness scripts, which drive REAL provider sync against the
`saehrimnir` mock. The scripts exist (`crates/app/tests/sync-harness/`)
but the "stay green with the SAME assertions" claim of an earlier draft
is FALSE and is the largest under-acknowledged brick (O0, a pre-cut
survey): today these scripts exercise the legacy hand-rolled clients and
assert LEGACY delta wire shapes - e.g. `graph-calendar-remote-delta.lua`
lines 168-184 assert `GET .../calendarView/delta` request counts, and
`jmap-calendar-remote-delta.lua` lines 164-166 assert
`CalendarEvent/changes`. After B7a the bifrost `Account` pull surface
calls DIFFERENT endpoints (range-scoped `calendarView`, JMAP
`CalendarEvent/query` + `/get`, CalDAV `REPORT`/`calendar-query`), so
BOTH of two things are required and neither is free:
(1) the per-provider assertions that pin legacy delta endpoints must be
REWRITTEN for the range-pull request shapes; (2) `saehrimnir` must
actually SERVE the bifrost calendar pull surface for all four backends
- B3/B4 wired MAIL through bifrost+saehrimnir, but nothing here confirms
the CALENDAR endpoints on saehrimnir are bifrost-shaped. O0 is to survey
saehrimnir's calendar fixtures against the bifrost pull surface BEFORE
the cut; if a backend is unserved, extending those fixtures is a
load-bearing brick of B7a, not an assumption. The gate LIST below is the
target post-rewrite; the assertions inside several scripts change.
Per-provider round-trip gates (the B7a TODO requirement):

```
brokkr service-test crates/app/tests/sync-harness/gcal-calendar-initial.lua
brokkr service-test crates/app/tests/sync-harness/gcal-calendar-recurrence-initial.lua
brokkr service-test crates/app/tests/sync-harness/gcal-calendar-remote-delta.lua
brokkr service-test crates/app/tests/sync-harness/graph-calendar-initial.lua
brokkr service-test crates/app/tests/sync-harness/graph-calendar-recurrence-initial.lua
brokkr service-test crates/app/tests/sync-harness/graph-calendar-remote-delta.lua
brokkr service-test crates/app/tests/sync-harness/jmap-calendar-initial.lua
brokkr service-test crates/app/tests/sync-harness/jmap-calendar-recurrence-initial.lua
brokkr service-test crates/app/tests/sync-harness/jmap-calendar-remote-delta.lua
brokkr service-test crates/app/tests/sync-harness/caldav-calendar-initial.lua
brokkr service-test crates/app/tests/sync-harness/caldav-calendar-recurrence-initial.lua
brokkr service-test crates/app/tests/sync-harness/caldav-calendar-remote-delta.lua
brokkr service-test crates/app/tests/sync-harness/caldav-multi-account-principal-scoping.lua
brokkr service-test crates/app/tests/sync-harness/graph-calendar-caldav-mutation-delta.lua
brokkr service-test crates/app/tests/sync-harness/google-oauth-multi-account-calendar-people.lua
brokkr service-test crates/app/tests/sync-harness/jmap-fixture-schema-calendar-oauth.lua
```

(Verify the exact runner verb and any cohort form against
`reference/glossary/harness.md` and `brokkr service-list` at
implementation - the harness doc shows `brokkr service-test <SCRIPT>`
for both service-harness and sync-harness scripts.)

One script in the list was MISCHARACTERIZED by an earlier draft
(R2-8, § 11): `graph-calendar-caldav-mutation-delta.lua` does NOT exercise
ratatoskr's calendar ACTION/write path. It mutates the MOCK server
directly over HTTP (`harness.http { method = "PUT", ... }`, line 103) and
then a delta SYNC reads the change back - so it is a server-side-mutation
READ-sync test, squarely on the path B7a rewrites, not the B7b-owned
`actions.rs` write path. It must pass with its DB-value assertions
unchanged (its needles may need the O0 pull-shape rewrite); a failure is a
B7a read-sync regression. Separately, because B7a re-keys native rows
(O13) and changes `remote_event_id`, the four provider action-CRUD
sync-harness scripts (the real write-path scripts, per backend) must be
run as REGRESSION gates even though B7a does not modify the write path -
they CONSUME the rows and ids B7a re-keys. State both outcomes explicitly
in the landing notes.

These are the load-bearing gates: `*-initial` proves discovery + window
pull + translation populate the cache; `*-recurrence-initial` proves the
master/override/recurrence-id keying survives the new translation (the
phantom-dedup hazard, § 2.3 / § 4.4); `*-remote-delta` proves a second
run picks up remote create/update/delete via the windowed reconcile
(including the no-spurious-delete guard, § 4.5).

Adjudication-added gate surface (§ 12; each rides an existing script or
is a named extension, fixture support per the saehrimnir side-quest):

- History backfill: extend `caldav-calendar-initial.lua` and
  `gcal-calendar-initial.lua` fixtures with events older than 365 days
  (inside [-5y, -1y)) and assert the rows are present after the first
  completed run and that `calendars.history_backfilled_at` is stamped;
  a second run must NOT re-issue the backfill ranges (request-count
  needle).
- Reminders (O7): `caldav-calendar-initial.lua` asserts
  `calendar_reminders` rows from a VALARM fixture;
  `jmap-calendar-initial.lua` asserts rows from a JSCalendar `alerts`
  fixture (legacy parity for both paths).
- Multi-VEVENT (O15): the `caldav-calendar-recurrence-initial.lua`
  fixture carries a single `.ics` resource with master + override +
  CANCELLED exception and asserts three rows with the correct
  `recurrence_id` keys and the cancelled `status`.
- Per-resource failure vs real delete (O3):
  `caldav-calendar-remote-delta.lua` carries both a
  transiently-failing resource (per-href 5xx inside the 207; asserted
  PRESERVED) and a true remote delete (asserted PROPAGATED).
- Empty-pull guard (O16): the existing
  empty-207-against-populated-cache extension of
  `caldav-calendar-remote-delta.lua` asserting NO deletion.
- Time translation (O19): unit gates below, plus a
  bare-wall-clock+TZID timed event and a one-day all-day event in the
  caldav initial fixture with exact epoch DB-value assertions
  (tz-independent by construction).

Deterministic unit gates (the translation correctness a harness cannot
isolate cheaply), all `brokkr test -p cal <NAME>` (the crate lives at
`crates/calendar/` but its Cargo package name is `cal`, Cargo.toml:2 - an
earlier draft wrote `-p calendar`, which does not resolve):

- `idmap_event_dedup_key_caldav_preserves_caldav_prefix` and
  `idmap_event_dedup_key_native_uses_native_id` - the dedup keying
  (§ 2.3); reuse / extend the existing `make_google_event_id` unit tests
  (sync.rs lines 986-1016) which must keep passing.
- `idmap_recurrence_id_canonical_form_is_host_tz_independent` - the
  schema lines 56-69 hazard; assert all-day / floating / zoned /
  UTC forms map to the canonical string regardless of host TZ.
- `idmap_event_time_all_day_parses_to_midnight` and
  `idmap_event_time_rfc3339_parses_to_epoch` - the `EventTime` parse.
- `idmap_event_time_all_day_one_day_has_86400_duration` (the exclusive
  day-count convention, § 4.4),
  `idmap_event_time_bare_wall_clock_with_tzid_resolves_zone`,
  `idmap_event_time_dst_gap_shifts_forward_and_ambiguity_picks_earlier`,
  and `idmap_event_time_floating_parses_as_utc` - the O19 pinned rule.
- `idmap_attendees_json_matches_legacy_shape` - the
  `[{email, displayName, responseStatus}]` JSON contract the view reads.
- `idmap_calendar_can_edit_maps_from_can_update_events`.

DB-crate gate if a windowed-delete query is added:
`brokkr test -p db calendar_windowed_delete_reconcile`.

Factory router gate (mirrors the existing
`bifrost_factory_builds_each_provider_kind` test in factory.rs):
`brokkr test -p service build_calendar_account_factory_routes_each_backend`
- seed one account per calendar backend (incl. a `caldav_url` +
calendar_provider='caldav' row, and a gmail-mail-account-with-caldav-calendar
row to prove the calendar axis is independent of the mail axis) and
assert each returns the right factory shape (the CalDAV vs native
distinction is observable; for the native arms reuse the implicit-dispatch
argument from factory.rs lines 665-677).

Performance gate (clause 5 + 10: ratatoskr measures provider-sync cost).
The windowed pull replaces delta sync, so per-run provider-request count
and elapsed are in scope. If a calendar `sync-bench` script exists,
gate it; if none exists, BUILDING the smallest calendar sync-bench that
records (elapsed, provider requests, peak RSS) against a `brokkr.toml`
baseline is itself a brick of this spec, laid before the cut:
`brokkr sync-bench crates/app/tests/sync-harness/gcal-calendar-steady-state.lua --gate gcal_calendar_steady_state --as-baseline`
(no `<provider>` placeholder - author one concrete steady-state script
per backend, `gcal-`/`graph-`/`jmap-`/`caldav-calendar-steady-state.lua`,
each with its own `--gate` name and a recorded `brokkr.toml` baseline;
none exist today, so building them is a brick of B7a, not a to-do.
Name/shape to match the existing mail steady-state benches; verify
against `reference/glossary/harness.md` and the mail
`gmail-steady-state-delta.lua` precedent). Per the O9 re-ruling (§ 8,
§ 12 ruling 1) these baselines are recorded on the NEW pull path at
landing (`--as-baseline`) as absolute regression guards for future
work - NOT held against the legacy delta path's request counts, which
a full-window pull structurally exceeds by design (that increase is
adjudicated accepted; the recorded numbers quantify it in the landing
notes).

Universal green-tree gate (every landing): `brokkr check`.

## 7. Stance (clause: structural over micro; cleanliness is a deliverable)

This is a full rewrite of the calendar read path, labeled as such: four
protocol implementations and their delta machinery collapse to one
provider-agnostic body plus a thin translation seam. No adapter around a
bifrost wart, no per-provider branch in the consumer, no env-var or
runtime switch between old and new paths (the rolling window is a
constant, not a knob). The `CalendarProvider`-style provider tag does
not survive read-sync (it survives in actions.rs only until B7b removes
it). Old abstractions earn no protection from age (clause: aggressive
internal rewrites assumed); the pre-1.0 signature change to
`calendar_sync_account_impl` is legal and intended.

Crate-boundary discipline (§ 3 of the governing plan): bifrost stays
confined to Service-side crates. `calendar` is Service-side (not in
`rtsk`'s tree); the bifrost calendar types it now imports do not cross
the core/UI firewall. If implementation finds any path pulling a bifrost
type toward `core`/`rtsk`/`app`, STOP - that is a § 7 violation, not a
thing to route around.

## 8. Open items reconciled into the spec (no deferral holes)

Each is a question the implementer resolves against the pinned bifrost
commit during the ground survey, with the resolution path stated so none
is a hole:

- **O1.** Confirm `Calendar.native_id == Calendar.id.0` (and
  `CalendarEvent.native_id == id.0`) for all four backends in frozen
  bifrost. CONFIRMED for events: `id.0 == native_id` for Google/Graph
  (both are `join_event_id(calendar_id, event_id)`). BUT that shared
  value is a COMPOSITE `calendar_id::event_id`, not the bare provider
  event id legacy stored - so the divergence that matters is between
  bifrost `native_id` and the legacy cache key / write-back id, not
  between `native_id` and `id`. The remediation lives in O13 (strip the
  prefix); pin both in `idmap.rs` with a comment.
- **O2 (RESOLVED per backend - § 12, folded into SQ-2).** Whether
  `events_in_range` returns recurrences whose instances touch the
  window when `DTSTART` precedes it, per backend: Google and Graph
  expand recurrences SERVER-SIDE (bifrost-google issues
  `singleEvents=true`, `google/src/account/calendar.rs:60`; bifrost
  graph uses ranged `calendarView`, which returns occurrences), so
  in-window instances arrive as instance events - matching legacy,
  which also used `singleEvents=true`. JMAP's time-range query matches
  occurrences server-side and returns the master. CalDAV is CONFIRMED
  BROKEN at the frozen commit: the server's time-range REPORT matches
  recurrences, but bifrost's post-query local filter
  (`account.rs::event_in_range`, lines 1273-1285) compares only the
  master's own DTSTART/DTEND interval and drops an out-of-window
  master with in-window instances. The fix is folded into SQ-2
  (§ 12): the local filter passes any event with a non-empty
  recurrence whose rule could intersect the range (conservatively, any
  recurring master with `DTSTART <= range_end`), leaving instance
  precision to the consumer's expansion - mirroring the ratatoskr view
  predicate. Pinned by the `*-calendar-recurrence-initial.lua` gates
  with an out-of-window-master fixture for CalDAV.
- **O3 (RESOLVED - § 12 ruling 4, via prerequisite SQ-4).** Frozen
  bifrost CalDAV `events_in_range` `filter_map(... .ok())`-drops a bad
  resource and returns `Page::single` with NO `failed_hrefs` and NO
  error (`research/bifrost/crates/caldav/src/account.rs` lines
  704-720) - absence was untrustworthy. The bifrost side-quest SQ-4
  makes the failed resource set surface on the `events_in_range`
  result; the reconcile subtracts it from the absence computation
  (§ 4.5), preserving transiently-failed rows while propagating true
  deletes - the authoritative form of the transient-vs-real distinction
  R1-6 demanded. No consumer-side heuristic candidates remain to pick
  between. Gated by the two-case `caldav-calendar-remote-delta.lua`
  extension (§ 6).
- **O4 (refined - R1-a, § 11).** Three-way write rights
  (`can_create/update/delete_events`) collapse to one `can_edit` bit.
  Resolution: map `can_update_events`; record a schema-enrichment
  candidate (not done in B7a). The R1 concern - "a delete button that
  reads `can_edit` would offer a delete on an update-only calendar" - is
  presently INERT: `can_edit` has zero readers in `crates/app/src` today
  (grep-confirmed), so the collapse mis-gates no live UI action. The
  finding stands only as a schema-fidelity note for the future
  enrichment, not a live-bug guard; if B7b (write path) starts gating
  actions on `can_edit`, revisit the three-way split then.
- **O5 (pinned).** Today's no-calendar-provider path returns `Err`; the
  runner's emission makes that a "failed" run. Adjudicated: a
  no-calendar account is a clean no-op (`Ok(None)` from the router ->
  no-op outcome). Implementation-time confirmation only: verify no UI
  surface depends on the old Err (none is known).
- **O6.** The CalDAV harness endpoint override env var name (symmetric
  with `RATATOSKR_TEST_GRAPH_ENDPOINT` etc.). Resolution: read the
  existing `caldav-calendar-*.lua` + `brokkr.toml [ratatoskr]` endpoint
  env names and reuse.
- **O7 (RESOLVED - § 12 ruling 2, via prerequisite SQ-3).** Reminders
  are synced, stored, AND displayed in event-detail views
  (`docs/calendar/discrepancies.md` § High 7, § Medium 16), and -
  a premise correction the earlier drafts missed - legacy populates
  them from TWO paths, not one: CalDAV VALARM AND JMAP JSCalendar
  alerts (`crates/calendar/src/jmap.rs:164-172` maps `record.reminders`
  through `replace_event_reminders`). The bifrost side-quest SQ-3 adds
  `reminders: Vec<EventReminder>` to `CalendarEvent`, projected from
  CalDAV VALARM and JMAP alerts; `to_reminders` (§ 4.4) translates them
  into today's `calendar_reminders` rows. Google/Graph stay empty
  (legacy never synced native reminders there - recorded as a future
  enrichment, not a loss). Gated by the § 6 reminder assertions on the
  caldav AND jmap initial scripts.
- **O8.** `rdate` / `exdate` have no schema columns and the load-path
  expansion reads only `recurrence_rule`. Resolution: do not add columns
  in B7a; record whether expansion correctness needs them (the recurrence
  gate is the check).
- **O9 (RE-RULED - § 12 ruling 1).** Dropping the client-side
  CTag/sync-token skip means B7a re-pulls the FULL active window on
  every hourly kick where delta sync previously transferred only
  changes - a structural cost increase. The earlier GO/NO-GO framing
  ("held against the delta baseline") was a predetermined failure: a
  full-window pull ALWAYS issues more requests than a no-op delta, so
  that gate could only ever force the side-quest or block the landing -
  a fork in disguise. Adjudicated: the increase is ACCEPTED at the
  bounded window size (hourly cadence, tens of requests per calendar -
  negligible absolute load next to mail sync). The § 6 sync-bench
  baselines are recorded on the NEW path at landing as absolute
  regression guards and quantify the accepted increase in the landing
  notes. The named future remedy, if real-world cost ever bites, is the
  `events_delta` bifrost side-quest (§ 12, deferred list) - not a B7a
  prerequisite and not a consumer-side ctag cache (which rebuilds the
  per-provider surface B7a deletes).

Open items added by the R1+R2 review consolidation (each confirmed
against code / the pinned bifrost commit, see § 9):

- **O0 (PRE-CUT survey, load-bearing).** The § 6 harness gates are NOT
  free. Today they assert legacy delta wire shapes
  (`graph-calendar-remote-delta.lua` 168-184 -> `calendarView/delta`;
  `jmap-calendar-remote-delta.lua` 164-166 -> `CalendarEvent/changes`),
  and it is unverified that `saehrimnir` serves the bifrost calendar PULL
  surface for all four backends. Partially narrowed by the second
  derivation's survey: saehrimnir DOES carry calendar mock modules for
  all four backends (`src/gcal`, `src/graph/calendar.rs`,
  `src/jmap_calendar.rs`, `src/caldav`), so the open question is not
  "does it serve calendar" but whether those mocks serve the PULL-shaped
  requests bifrost issues (non-delta
  `calendarView?startDateTime=...&endDateTime=...` for Graph,
  `CalendarEvent/query` + `/get` for JMAP, time-range
  `REPORT`/`calendar-query` for CalDAV, `timeMin`/`timeMax` events list
  for Google). Resolution: before the cut, survey saehrimnir's calendar
  handlers against those exact request shapes; treat fixture extension +
  assertion rewrites as bricks of B7a (§ 6), executed as the § 12
  saehrimnir side-quest (SQ-5) sized by this survey. The survey also
  covers (a) the Graph 1095-day `calendarView` span acceptance (the
  § 2.6 slicing-fallback trigger) and (b) mock affordances for the
  adjudication-added gates (§ 6): multi-VEVENT resource, VALARM +
  JSCalendar alerts, bare-wall-clock+TZID and all-day fixtures,
  per-href 5xx inside a 207, empty-207 mode, and events older than 365
  days for the backfill gates. Needle rewrites have
  precedent: B4a rewired the IMAP write-back needles off
  `UID COPY`/`STORE` onto server round-trips - a needle change is legal;
  a DB-VALUE assertion change is a red flag that the translation is
  wrong, and the DB-value assertions in every calendar script must pass
  unchanged - with ONE adjudicated carve-out: any assertion that pinned
  the legacy Google path's host-tz-dependent all-day epochs (such an
  assertion would have been host-flaky already) is updated to the
  tz-independent convention (§ 4.4, O19), each called out in the
  landing notes.
- **O10 (intentional behavior change).** B7a's calendar account-factory
  router makes `calendar_provider` GENUINELY win over the mail provider,
  fixing today's fixed-order short-circuit where a gmail/graph mail
  account silently loses a `caldav` calendar intent (§ 2.1, § 4.1).
  Resolution: implement caldav-arm-first routing, flag as a behavior fix,
  and gate it with the gmail-mail + caldav-calendar router test (§ 6).
- **O11 (per-backend paging asymmetry).** bifrost CalDAV `events_in_range`
  ignores `page_cursor`, applies `limit` via `truncate`, and returns
  `Page::single` (account.rs 717-720). Resolution: pass `limit: None` for
  CalDAV (a limit silently DROPS in-window events), accept the
  single-page whole-collection load for CalDAV, and keep cursor paging
  for the native backends (§ 2.6).
- **O12 (NARROWED - § 12 ruling 1).** Navigation does not trigger
  provider sync (`handlers/calendar.rs` 27-37, 717-727 read the DB
  window only); only the hourly kick + explicit `start_calendar_sync`
  fetch. With the § 2.6 history backfill the un-navigable boundary
  moves from -1y out to -5y (and +2y forward) - within the product's
  own stated history number, and wider than legacy Google/Graph ever
  covered. Beyond that boundary the view is empty (or stale for
  backfilled history); adjudicated as the stopping point. Widening
  further is the future `events_delta` side-quest (§ 12).
- **O13 (native dedup/remote-id key migration).** bifrost Google/Graph
  `native_id = calendar_id::event_id` (composite); legacy stored the bare
  event id. Resolution: `idmap` strips the calendar prefix for both
  `event_dedup_key` and `event_remote_id` on native providers (preserving
  the legacy cache key and the bare write-back id), or B7a owns an
  explicit cache migration - strip is the pick (§ 2.3). Gated by the
  dedup-key unit tests (§ 6).
- **O14 (Graph recurrence_id is seriesMasterId).** bifrost Graph maps
  `seriesMasterId` into `recurrence_id` (graph calendar.rs 497) - a series
  id, not a RECURRENCE-ID instance discriminator. Resolution: do not
  canonicalize/persist it as an override key; the recurrence unit gate
  must cover the Graph shape (§ 4.4, § 6).
- **O15 (RESOLVED - § 12 ruling 3, via prerequisite SQ-2).** bifrost
  CalDAV projected only the first VEVENT per `.ics` resource (ical.rs
  325-329, the `parse_vevent` early-`break`); legacy wrote every VEVENT
  (sync.rs 746), so override/CANCEL instances were lost. SQ-2 makes the
  CalDAV projection yield every VEVENT (master + overrides + CANCELLED
  exceptions, each carrying its RECURRENCE-ID and STATUS); § 4.4
  persists one row per projected event on the existing phantom-dedup
  keys. Gated by the § 6 multi-VEVENT fixture.

Open items added by the latest R1+R2 review pass (§ 11), each validated
against the current code / bifrost `be11bbb`:

- **O16 (RESOLVED - § 12 ruling 4).** A successful windowed pull that
  returns zero events against a non-empty in-window cache SKIPS the
  delete step (suspected transient failure), provider-agnostically -
  the legacy CalDAV empty-207 guard (sync.rs:677) reproduced as § 4.5
  mandates. Adjudicated as the correct mechanism (no protocol offers a
  server-attested completeness signal to prefer; this is today's
  shipped behavior), complementing the SQ-4 per-resource failed set.
  Gate: extend `caldav-calendar-remote-delta.lua` with an
  empty-207-against-populated-cache case that asserts NO deletion.
- **O17 (stale-calendar reconcile on backend switch - R2-3, MUST fix).**
  `load_visible_calendars` is provider-blind (sync.rs:297); after the O10
  reroute it can hand a Google calendarId to a CalDAV account and abort.
  Step 6 must iterate only calendars the CURRENT `calendars_list()`
  returned, and B7a must define a policy for cache rows the current
  backend no longer lists (discovery upsert does not re-provider or reap,
  `calendar_contacts_writes.rs:63`). Gate: extend the router/backend-switch
  test to seed a stale Google row and assert it is not fetched via CalDAV.
- **O18 (CalDAV auth is independent of mail auth - R2-2, MUST fix).** Build
  `CalDavCredentials::Basic` from the dedicated `caldav_username`/
  `caldav_password` columns (schema `01_core.sql:36-37`,
  `caldav/mod.rs:251`), NOT bearer-from-mail-OAuth. Extend the shared
  credential-read helper to load the CalDAV columns (`factory.rs:266` does
  not today). Gate: the § 6 router test's gmail-mail + caldav-calendar row
  must assert Basic creds are wired.
- **O19 (RESOLVED - § 12 ruling 5, via prerequisite SQ-1 + the § 4.4
  pinned rule).** The root fact: bifrost's `EventTime` was internally
  INCONSISTENT - bifrost-google passes Google's EXCLUSIVE all-day end
  date through verbatim (`google/src/account/calendar.rs:636`) while
  bifrost-caldav DECREMENTS `DTEND` to inclusive (`ical.rs:432-440`),
  and CalDAV timed values are bare wall-clock + separate `timezone`
  (`ical.rs:416`) with the contract nowhere documented. SQ-1 fixes
  bifrost: the decrement is removed (all-day end EXCLUSIVE uniformly)
  and the `EventTime` interpretation contract is documented on the
  type. Ratatoskr-side, `event_time_to_epoch` (§ 4.4) implements the
  single uniform rule - offset/Z absolute; bare+zone via `chrono_tz`
  with pinned DST gap (next valid instant) / ambiguity (earlier offset)
  policy; floating as UTC; all-day as start-anchored
  `+ days * 86400` with the exclusive day count (the legacy-CalDAV,
  tz-independent convention, `core/src/caldav/parse/ical/mod.rs:221-227`),
  deliberately replacing the legacy Google path's host-tz-dependent
  `chrono::Local` form. Gates: the § 6 all-day + zoned + DST unit
  tests and the fixture epoch assertions.
- **O20 (`CalendarChanged` fires every run - R1-4).** § 4.2 sets
  `mutated = true` unconditionally after every upsert; under a full
  windowed pull (no delta) the upserts run every hourly kick, so
  `CalendarChanged` fires every hour even when nothing changed, driving
  app reload/re-render churn the delta path avoided. Resolution: gate
  `mutated` on ROWS-ACTUALLY-CHANGED from the upsert helpers (not "an
  upsert ran"), so the emission contract's INPUT frequency matches its
  prior meaning. Confirm the upsert helpers can report rows-affected; if
  not, that plumbing is part of B7a.
- **O21 (windowed-delete candidate set - R2-7).** Use the overlap
  predicate + special recurring-master load the view query already uses
  (`calendars/view/mod.rs:82-86`), not `start_time`-in-window. The O2
  upstream prerequisite this depended on is CLOSED (§ 8 O2, SQ-2), so
  the recurring-row reconcile is safe as specified (§ 4.5 corrected).
- **O22 (RECONFIRMED under the backfill - § 12 ruling 1).** Events
  outside the active window are never reconciled (§ 2.6 "left
  untouched"), so remotely-deleted out-of-window rows persist - and
  with the § 2.6 backfill this is now partly a FEATURE (it is what
  keeps backfilled history cached) and partly the same recorded
  cache-coherence debt (a deletion or edit of a >1y-old event after its
  calendar's backfill completed never propagates; calendar history
  rarely mutates retroactively, and legacy Google/Graph had the
  equivalent staleness beyond their delta windows). `full_resync`
  remains the manual coherence fix; a future GC/tombstone pass (or
  `events_delta`) is the systematic one if it matters.
- **O23 (verification-section corrections - R2-8).** Unit gates run
  `-p cal` (not `-p calendar`); the perf gate names concrete per-backend
  steady-state scripts (no `<provider>` placeholder);
  `graph-calendar-caldav-mutation-delta.lua` is a READ-sync test B7a
  touches (not an untouched action-path script); and the four provider
  action-CRUD scripts are regression gates because B7a re-keys their rows
  (§ 6 corrected).

Post-adjudication status (§ 12): no open item carries a decision
anymore. O3, O7, O15 resolve via the prerequisite bifrost side-quests;
O16, O19 resolve via pinned in-spec mechanisms; O9 and O12 are re-ruled
(accepted, quantified, named future remedy); O5, O10, O13, O14, O17,
O18, O20, O21, O23 were already concrete instructions; O1, O2, O4, O6,
O8 remain implementation-time CONFIRMATIONS with stated resolution
paths and gates (verify-and-pin, not choose). The only pre-cut
scheduling dependency is O0's survey, executed as SQ-5 (§ 12). O13,
O17, O18 and the reconcile mechanics remain the items that, if
mis-implemented, cause silent data loss, so they keep the most explicit
guard wording.

## 9. Review consolidation (R1 + R2)

The two independent reviews (`B7a-R1.md`, Opus; `B7a-R2.md`, codex) were
validated finding-by-finding against the current repo and the pinned
bifrost checkout (`research/bifrost`, then at commit `0e71226`;
re-pinned to the current freeze `be11bbb` and every load-bearing
citation was re-verified to still hold there - see § 11). EVERY finding was
confirmed - none was rejected as a misread. The validation evidence and
the landing site of each are below; duplicates across the two reviews are
merged.

Confirmed and folded:

- **R1 BUG1 / precedence** (sync.rs 79-97 is a fixed-order if/else where
  `provider == "gmail_api"` short-circuits before `calendar_provider`).
  Folded: § 2.1 corrected, § 4.1 flags the deliberate fix, O10.
- **R1 GAP2 + R2 #7 / harness gates** (graph 168-184 assert
  `calendarView/delta`; jmap 164-166 assert `CalendarEvent/changes`;
  bifrost pull surface calls different endpoints). Folded: § 6 rewritten,
  O0 (pre-cut survey + assertion rewrites).
- **R1 GAP3 / runtime key** (calendar.rs 116-122 doc: `SecretKey` consumed
  at construction, not stored on `Inner`). Folded: § 4.6 - add a
  `key_bytes` field; also confirmed `writer_pool()` is reachable.
- **R1 SMELL4 / non_exhaustive enums** (bifrost enums are
  `#[non_exhaustive]` with `Unknown`/`Delegated`/`Confidential`/
  `OutOfOffice` variants). Folded: § 4.4 total-mapping requirement + unit
  gate.
- **R1 SMELL5 + R2 #1 / CalDAV failed resources** (account.rs 704-720:
  `filter_map(.ok())`, `Page::single`, no `failed_hrefs`). Folded:
  § 2.6 and § 4.5 prose corrected, O3 upgraded to "confirmed broken" with
  an in-scope guard.
- **R1 NIT6 / "only behavioral difference"** overstated. Folded: § 2.6
  re-worded (window is the largest, not only; O4/O7 also observable).
- **R1 NIT7 / O7 circular** (resolution stated before the gating
  question). Folded: O7 reordered to confirm-then-decide.
- **R2 #2 / CalDAV multi-VEVENT** (ical.rs 325-329: only first VEVENT
  projected; legacy sync.rs 746 wrote every VEVENT). Folded: § 4.4, O15.
- **R2 #3 / native_id composite** (google calendar.rs 404, graph 443:
  `join_event_id(calendar_id, event_id)`; legacy google.rs 590 stored bare
  id). Folded: § 2.3 idmap strip, O1 updated, O13.
- **R2 #4 / navigation does not sync** (handlers/calendar.rs 27-37,
  717-727 read DB only). Folded: § 2.6 reassurance corrected, O12.
- **R2 #5 / Graph seriesMasterId -> recurrence_id** (graph calendar.rs
  497). Folded: § 4.4 caution, O14.
- **R2 #6 / CalDAV ignores page_cursor, truncates on limit** (account.rs
  717-720). Folded: § 2.6 paging caveat, O11.
- **R2 #8 / `calendar` cannot drop provider deps in B7a** (actions.rs
  25-37 still imports `GmailClient`/`GraphClient`/`JmapClient` + write
  impls for B7b). Folded: § 4.6 corrected.

Rejected: none. Both reviews were accurate; the only adjustments were
merging the two overlapping pairs (R1 SMELL5 == R2 #1; R1 GAP2 == R2 #7)
and recording the previously-"open" O1/O3 as now-confirmed against the
pinned commit rather than left as questions.

## 10. Second-derivation reconciliation (Jul 18)

A second, independent B7a derivation was authored later (as
`B7a-spec.new.md`) without knowledge of this spec's R1+R2 consolidation.
The two were reconciled per clause 8 (sibling surveys reconcile before
implementation); this document remains canonical. Every disputed fact
was re-verified against the code during the reconciliation:

Folded IN from the second derivation (genuine improvements):

- The injected `now_ms` clock on `calendar_sync_account_impl` (§ 2.6,
  § 4.2) so the rolling window is deterministic under the harness clock.
- The pre-delete `git grep` symbol audit (§ 5).
- The `href_synthetic_uid` no-UID fallback and the MOVE-not-duplicate
  requirement for both dedup helpers and their tests (§ 2.3).
- The `event_time_to_epoch -> Option<i64>` refuse-to-persist contract
  mirroring sync.rs 853-858 (§ 4.4).
- The O0 narrowing: saehrimnir already carries calendar mock modules for
  all four backends; the remaining survey is pull-shape coverage, with
  the B4a needle-rewrite precedent recorded (§ 8 O0). SIZING CAVEAT (R1-5,
  § 11): the "one coherent landing" shape (§ 1) assumes this survey comes
  back small. It is not yet sized. If any backend's saehrimnir calendar
  mock does not serve the pull-shaped requests, extending those fixtures
  is unestimated mock-server work that gates the ENTIRE behavioral-
  verification story and could break the single-commit shape. O0 must be
  RESOLVED (survey done, extension work sized) before the cut is
  scheduled - it is the largest schedule risk in the spec, not a
  parallel to-do.
- The explicit whole-script expectation for
  `graph-calendar-caldav-mutation-delta.lua` (§ 6).

REJECTED from the second derivation (each refuted against the code
during this reconciliation - a critique pass should re-check these
citations, not re-litigate from memory):

- "Google/Graph/JMAP event `native_id` is the bare provider event id."
  FALSE: google calendar.rs 404 and graph calendar.rs 443 both set
  `native_id = join_event_id(calendar_id, event_id)` (composite). Using
  `native_id` as the dedup key re-keys every native cached row and fails
  the scripts' own `google_event_id` DB-value assertions. O13 (strip)
  stands.
- Uniform `limit: Some(WINDOW_PAGE_LIMIT)` on `events_in_range`. For
  CalDAV that silently DROPS in-window events (caldav account.rs
  717-720: `truncate` + `Page::single`, no cursor). O11 (limit: None
  for CalDAV) stands.
- Mapping `recurrence.recurrence_id` straight into the schema's
  `recurrence_id`. For Graph that persists `seriesMasterId` (graph
  calendar.rs 497) as an occurrence discriminator, corrupting the
  phantom-dedup key. O14 stands.
- Preserving the legacy router precedence bit-for-bit (mail provider
  short-circuits `calendar_provider`). This spec instead makes
  `calendar_provider` genuinely win, as a FLAGGED, GATED behavior fix
  (O10) - the legacy order silently loses a configured CalDAV calendar
  on a gmail/graph mail account. The second derivation's concern
  (existing accounts re-route) is exactly the intended fix: the row only
  carries `calendar_provider = "caldav"` when a CalDAV calendar was
  configured.
- "The request-count assertions survive bifrost's wire pattern in almost
  all cases." The delta scripts' floors target legacy delta endpoints
  (`calendarView/delta`, `CalendarEvent/changes`) that the pull surface
  never calls; those assertions must be rewritten (O0).
- Omission of the per-resource CalDAV failure guard (it carried only the
  page-level zero-events guard). O3 (confirmed broken upstream; add a
  consumer-side guard) stands.
- Omission of the CalDAV multi-VEVENT projection loss. O15 stands.
- Skipping the calendar sync-bench baseline. Clause 5/10: a spec
  touching a provider-sync path owes the perf gate, and O9 (dropped
  ctag/sync-token skip may raise per-run cost) is exactly what it
  measures. The § 6 sync-bench brick stands.

Window size: this spec's `back = 365d, forward = 730d` stands (the
second derivation proposed 730/730 with no stronger rationale; either
covers the harness fixtures once `now_ms` is injected).

## 11. Third review pass (R1 + R2 on the consolidated spec)

Two further independent reviews of THIS document -
`docs/bifrost-migration/B7a-spec.R1.md` (Opus) and `B7a-spec.R2.md`
(codex xhigh) - were validated finding-by-finding against the current
repo and bifrost HEAD `d3f9cca` (re-verified to hold at the current
freeze `be11bbb`). Unlike the § 9/§ 10 rounds (which
reviewed earlier drafts), these reviewed the consolidated spec, so most
findings land as CORRECTIONS to prose the earlier rounds had already
touched. Validation evidence and landing site per finding:

Confirmed and folded:

- **R1-D + R2 doc-drift / pin.** `research/bifrost` HEAD is `d3f9cca`, not
  the cited `0e71226` (verified). Claims still hold at `d3f9cca` but line
  numbers slid. Folded: required-reading + § 9 re-pinned; § 11 re-anchors.
  Since advanced: the B7a calendar side-quests carried the freeze to
  `be11bbb`, to which this spec's load-bearing citations are now re-pinned.
- **R2-1 / empty-pull deletion inverted.** Legacy SKIPS deletion on a
  successful-but-empty pull against a non-empty cache (sync.rs:677); the
  first draft called the mass-delete "the provider-agnostic form" of that
  guard - the INVERSE. Folded: § 2.6 + § 4.5 corrected, O16.
- **R2-2 / CalDAV auth from mail auth.** Dedicated
  `caldav_username`/`caldav_password` columns exist (schema 01_core.sql:36-37)
  and the live path uses Basic (caldav/mod.rs:251); the router's
  bearer-from-mail-OAuth would send the wrong credential type. Folded:
  § 4.1 corrected, O18.
- **R2-3 / stale calendars through wrong backend.** `load_visible_calendars`
  is provider-blind (sync.rs:297); post-O10 reroute feeds Google ids to
  the CalDAV account. Folded: § 4.2 step 5-6 corrected, O17.
- **R2-4 / id-seam self-contradiction.** § 4.4 wrote `remote_event_id =
  ev.native_id`, contradicting § 2.3's strip rule; `event_id_for_writeback`
  lacked a provider input. Folded: § 4.4 + § 2.3 corrected.
- **R2-5 / confirmed regressions vs side-quest rule.** Reminders ARE
  displayed today (discrepancies.md § High 7), so O7's confirm-question is
  answered YES; O7 and O15 are visible losses that the feature-preservation
  + side-quest protocol (bifrost-migration.md § 2) governs. Folded: O7 and
  O15 rewritten from "documented acceptance" to "side-quest or explicit
  sign-off."
- **R2-6 / time translation vs real `EventTime`.** CalDAV emits bare
  wall-clock + separate `timezone` (ical.rs:416) and pre-decrements all-day
  `DTEND` (ical.rs:432-440); midnight-to-midnight makes a one-day event
  zero-duration. Folded: § 4.4 caution, O19.
- **R2-7 / windowed delete for overlap + recurring.** `start_time`-in-window
  misses overlapping and recurring rows; the view query already uses the
  right predicate (calendars/view/mod.rs:82-86) and bifrost CalDAV filters
  locally without RRULE expansion (account.rs:715). Folded: § 4.5 corrected,
  O21.
- **R2-8 / verification gates.** Package is `cal` not `calendar`
  (Cargo.toml:2); perf gate had a `<provider>` placeholder;
  `graph-calendar-caldav-mutation-delta.lua` mutates the mock over HTTP
  (line 103), a READ-sync test, not the action path; provider action-CRUD
  scripts omitted. Folded: § 6 corrected, O23.
- **R1-1 / rolling window vs "5+ years searchable" (product).** The window
  discards calendar history this product's stated value promises, and O12
  confirms navigation cannot widen it. Recorded below as an elevated
  product-sign-off item (not merely an engineering stopping rule).
- **R1-2 / delta -> full-pull cost, remedy out of landing.** Folded:
  O9 sharpened to a GO/NO-GO baseline (side-quest becomes a prerequisite
  if over budget).
- **R1-3 / CalDAV aggregate fidelity loss.** Recorded below as an elevated
  decision item (O3+O7+O11+O15+O16+O18+O19 all land hardest on CalDAV).
- **R1-4 / `CalendarChanged` every run.** `mutated` is unconditional under
  a full pull. Folded: O20 (gate `mutated` on rows-actually-changed).
- **R1-5 / O0 unsized.** Folded: O0 sizing caveat (survey must be resolved
  and extension work sized before scheduling the cut).
- **R1-6 / O3 candidate contradicts its own gate.** "Skip CalDAV deletes
  entirely" fails `caldav-calendar-remote-delta.lua`. Folded: O3 rules that
  candidate out.
- **R1-a / O4 `can_edit`.** Refined: `can_edit` has zero readers in
  `crates/app/src` today, so the mis-gate is inert; kept as a schema note.
  Folded: O4.
- **R1-b / monotonic cache growth.** Out-of-window rows never reconciled.
  Folded: O22.

Elevated decisions - both now ADJUDICATED, see § 12 (kept here for the
audit trail):

- **Rolling-window product sign-off (R1-1).** ADJUDICATED as § 12
  ruling 1: the premise ("today caches unbounded history") was half
  false - legacy Google/Graph ship a stale-anchored `[-90d, +365d]`
  initial window - and the ruling closes the real JMAP/CalDAV gap with
  the § 2.6 one-time 5-year history backfill instead of either a
  silent regression or an unbounded-pull cost explosion. The recorded
  residue (beyond-5y archive events absent on FRESH setups only) is
  signed off in § 12 as within the product's own stated history number.
- **CalDAV aggregate fidelity (R1-3).** ADJUDICATED as § 12 ruling 6:
  CalDAV fidelity is PRESERVED, not signed away. O3/O7/O15/O19 are
  fixed in bifrost first (the § 12 prerequisite side-quests - per the
  governing § 2 first principle), O16/O18 are fixed consumer-side in
  this spec, and the two survivors are non-regressions: O11
  (single-page whole-window load - legacy loaded the whole collection
  in memory too) and the O22 out-of-window staleness (legacy
  Google/Graph had the equivalent beyond their delta windows). Nothing
  CalDAV-visible that ships today is lost.
- **Document length (R1-c, nit).** At ~1300 lines the spec reads partly as
  a record of having-been-reviewed (§ 9/§ 10/§ 11 process archaeology;
  O10 restated several times). Retained deliberately - the author values
  the audit trail and asked for this consolidation - but noted: a future
  editorial pass could move the review-consolidation sections to a PR log
  and leave §§ 1-8 as the buildable spec.

Rejected:

- **R2 "obsolete `crates/app/src/db/calendar.rs` path."** FALSE. The file
  exists and `load_calendar_events_for_view` is at line 46 (verified); the
  spec's citation of that path is correct. R2 also cited a view query at
  `view/mod.rs:84`; the real path is
  `crates/db/src/db/queries_extra/calendars/view/mod.rs:82-86` (the
  overlap + recurring-master predicate is there as R2 described) - the
  finding is valid, only its path was shorthand, and the corrected path is
  used above. No other finding was rejected; both reviews were otherwise
  accurate.

## 12. Adjudication (post-consolidation decision ruling, Jul 18)

The step-3 consolidation deliberately left a cluster of decisions open
(the § 11 elevated items plus O7, O15, O16, O19, O21-adjacent
mechanics). This section records the adjudication that closed them, the
premise corrections it rests on, and the prerequisite side-quests it
mandates. Every open fork in §§ 1-8 has been rewritten in place to the
adjudicated form; this section is the rationale-of-record. Governing
law: `docs/bifrost-migration.md` § 1 (feature-preserving) and § 2
(bifrost is fixed FIRST; ratatoskr is never contorted around a bifrost
wart).

Premise corrections (verified in code, load-bearing for the rulings):

- Legacy Google and Graph calendar sync are WINDOWED today: initial
  sync covers `[now - 90d, now + 365d]`
  (`crates/calendar/src/google.rs:157-158`,
  `crates/graph/src/calendar_sync.rs:336-337`), and the subsequent
  sync-token / delta-token stream stays anchored to that
  never-re-anchored window. Only JMAP and CalDAV are whole-calendar
  unbounded. "Today every provider caches the whole calendar" was
  false for half the backends.
- Legacy populates `calendar_reminders` from TWO paths - CalDAV VALARM
  AND JMAP JSCalendar alerts (`crates/calendar/src/jmap.rs:164-172`) -
  and the rows are displayed read-only in event detail
  (`docs/calendar/discrepancies.md` § High 7, § Medium 16).
- bifrost is internally inconsistent on all-day ends: bifrost-google
  passes Google's EXCLUSIVE end date through verbatim
  (`google/src/account/calendar.rs:636`) while bifrost-caldav
  decrements to inclusive (`caldav/src/ical.rs:432-440`). That makes
  O19 a bifrost-side contract defect, not a consumer parsing puzzle.
- Legacy's own all-day epoch conventions disagreed per provider: the
  CalDAV path is tz-independent (`end = start + days * 86400`,
  `core/src/caldav/parse/ical/mod.rs:221-227`) while the Google path
  rides host-local `chrono::Local` midnight / 23:59:59-of-exclusive-
  date (`crates/calendar/src/google.rs:541-566`) - the exact hazard
  the schema comment warns about. Unifying on the CalDAV form is a
  correctness fix, not a regression.
- `chrono_tz` is already in the tree (the legacy CalDAV TZID ladder),
  so consumer-side zone resolution has precedent to lift, not a new
  dependency to justify.

Rulings:

1. **Calendar history model (the § 11 rolling-window fork).** The
   rolling `[-365d, +730d]` pull architecture STANDS, augmented by the
   § 2.6 one-time per-calendar HISTORY BACKFILL of `[now - 1825d,
   now - 365d)` in four 365-day upsert-only slices, marked by the new
   `calendars.history_backfilled_at` column (§ 4.5a). Rationale:
   against the corrected premise, the window + backfill is a strict
   coverage IMPROVEMENT for Google/Graph (legacy: stale-anchored
   -90d) and preserves the product's own "5+ years" number for
   JMAP/CalDAV on fresh setups at zero steady-state cost; rows cached
   by an existing install are never deleted regardless of age (O22).
   Recorded residue, signed off here: beyond-5y archive events are not
   backfilled on a FRESH setup (they survive on existing installs).
   O9 is re-ruled with it: the delta-to-full-pull request-count
   increase is accepted at the bounded window (the old GO/NO-GO
   framing was a predetermined failure and a fork in disguise); the
   sync-bench baselines are recorded on the new path as absolute
   guards. The named FUTURE remedy for both boundaries - not a B7a
   prerequisite - is a bifrost `events_delta` side-quest: a
   capability-flagged changed-since primitive (Google syncToken, JMAP
   `CalendarEvent/changes`, CalDAV snapshot-cursor diff over the
   existing `EventSnapshot` machinery, Graph `calendarView` delta),
   which would restore delta economics and unbounded coverage in one
   move if real-world cost or history demand ever warrants it.
2. **O7 - reminders.** Bifrost side-quest (SQ-3), a B7a prerequisite.
   Feature-preserving § 1 forbids dropping a displayed surface; § 2
   places the fix in bifrost. Scope includes JMAP alerts, not only
   CalDAV VALARM (premise correction above). No sign-off-to-drop.
3. **O15 - multi-VEVENT.** Bifrost side-quest (SQ-2), a B7a
   prerequisite. Recurrence overrides and CANCEL exceptions are core
   CalDAV semantics; first-VEVENT-only projection is a bifrost defect
   by any reading. No sign-off-to-drop.
4. **O16/O3 - deletion safety.** Composite: the authoritative
   per-resource failed set comes from bifrost (SQ-4) and is subtracted
   from the reconcile's absence computation; the legacy empty-pull
   skip guard is reproduced provider-agnostically as defense-in-depth
   (it is today's shipped behavior, and no calendar protocol offers a
   server-attested completeness signal that could replace it). The
   remote-delta gate distinguishes preserve-vs-propagate cases.
5. **O19 - time translation.** Composite: bifrost normalizes its OWN
   inconsistency (SQ-1: remove the caldav all-day decrement, document
   the `EventTime` contract on the type); ratatoskr owns epoch
   semantics via the single pinned `event_time_to_epoch` rule (§ 4.4),
   unified on the tz-independent legacy-CalDAV all-day convention with
   explicit DST gap/ambiguity policy.
6. **CalDAV aggregate fidelity (§ 11).** Preserved, not signed away -
   the direct consequence of rulings 2-5. The two survivors (O11
   single-page load; O22 out-of-window staleness) are non-regressions
   versus shipped behavior and are recorded as such.

Prerequisite side-quests, in landing order (the orchestrator executes
these under the § 2 side-quest protocol - one Opus agent in
`./research/bifrost` / `./research/saehrimnir`, orchestrator reviews,
validates in place, commits, promotes via the bridge scripts; SQ-1
through SQ-4 may land as one bifrost commit series with a single freeze
advance, ordered internally as below because 2-4 build on 1's corrected
projection):

- **SQ-1 (bifrost: `types` + `caldav`) - EventTime contract.** Remove
  the all-day `DTEND` inclusive-decrement in
  `caldav/src/ical.rs::format_ical_time` (all-day end becomes the
  EXCLUSIVE date uniformly, matching google/graph passthrough), and
  document the `EventTime` interpretation contract on
  `bifrost_types::EventTime` (value is RFC 3339 with offset/`Z`, OR
  bare wall-clock interpreted in `timezone`, OR floating; all-day is
  date-only with exclusive end). Unit test: a one-day all-day VEVENT
  projects start-date + exclusive end-date.
- **SQ-2 (bifrost: `caldav`) - multi-VEVENT projection + recurrence-
  aware range filter.** `ical.rs::parse_vevent`'s first-VEVENT `break`
  (lines 325-329) becomes an all-VEVENTs projection: `events_in_range`
  yields one `CalendarEvent` per VEVENT (master + overrides + CANCELLED
  exceptions), each with `recurrence.recurrence_id` from RECURRENCE-ID
  and `status` from STATUS. AND the post-query local filter
  (`account.rs::event_in_range`, lines 1273-1285) becomes
  recurrence-aware: a recurring master whose rule could intersect the
  range (conservatively, non-empty recurrence with
  `DTSTART <= range_end`) passes even when its own DTSTART/DTEND
  interval precedes the range - the O2 CalDAV defect. Unit tests: a
  master+override+CANCEL resource projects three events with distinct
  recurrence ids; an out-of-range master with an intersecting RRULE
  survives the filter.
- **SQ-3 (bifrost: `types` + `caldav` + `jmap`) - reminders.** Add
  `reminders: Vec<EventReminder>` to `bifrost_types::CalendarEvent`
  (an `EventReminder { minutes_before: i64, method: Option<...> }`
  shape mirroring ratatoskr's `calendar_reminders` columns); caldav
  projects VALARM (relative TRIGGER to minutes-before-start; absolute
  triggers resolved against DTSTART; DISPLAY/EMAIL action to method);
  bifrost-jmap projects JSCalendar `alerts` equivalently. Google/Graph
  leave it empty (legacy parity). Unit tests on both projections.
- **SQ-4 (bifrost: `caldav`, `types` if the page shape changes) -
  per-resource failure surfacing.** `events_in_range` stops
  `filter_map(... .ok())`-swallowing resources: any resource that
  could not be fetched or projected is surfaced as a failed set on the
  result (reusing the existing `failed_hrefs` snapshot machinery,
  `account.rs:146`; concrete surface shape - e.g. an events-page type
  or an additive field defaulting empty - is settled in the side-quest
  against bifrost's own conventions, with the contract that consumers
  can map failed entries back to native ids). Unit test: a failing
  resource appears in the failed set and not as silent absence.
- **SQ-5 (saehrimnir) - calendar pull-shape + fixture coverage.**
  Scope is SIZED BY the O0 survey (run the survey first; it is the
  gating schedule risk): ensure the four calendar mocks serve the
  bifrost pull shapes (Graph non-delta ranged `calendarView` at the
  1095-day span, JMAP `CalendarEvent/query` + `/get`, CalDAV
  time-range `REPORT`/`calendar-query`, Google `timeMin`/`timeMax`
  list), plus the § 6 fixture affordances: multi-VEVENT resource,
  VALARM and JSCalendar alert fixtures, bare-wall-clock+TZID and
  one-day all-day fixtures, per-href 5xx inside a 207, an empty-207
  mode, and events older than 365 days for the backfill gates.

After the SQ-1..4 bifrost batch and SQ-5 saehrimnir promoted, the
bifrost freeze advanced to `be11bbb`; this spec's commit references
(required-reading, § 2.2, § 9) are re-pinned to that freeze before
step 3b, per the existing "re-verify HEAD before citing line numbers"
convention.

Loop resume: side-quests above (SQ-1..4 bifrost batch, then SQ-5
saehrimnir sized by the O0 survey), then step 3b (commit the spec),
then step 4 (implement). The spec is decision-complete: no fork
remains that would force the implementer to choose; the remaining
open items are verify-and-pin confirmations with stated resolution
paths and gates.
