# Calendar Code Review Findings

Captured 2026-05-01 for Round 1, expanded 2026-05-01 with Round 2's open
backlog, then again 2026-05-01 after Round 3. Round 3 was a per-lens
fan-out via the `review` CLI (bugs / arch / perf, each addressed by both
the Anthropic and OpenAI providers - six reviewer outputs total).
Findings are tagged with the lens(es) and provider(s) that flagged them
in `[bugs/claude, arch/codex, ...]` form; multi-tag entries are the
highest-confidence signals.

---

## Round 1 (closed)

All findings addressed. Lenses and targets:

- **RRULE expansion** - `crates/db/src/db/queries_extra/calendars.rs::expand_recurrence` and helpers (`parse_rrule`, `parse_byday`, `expand_{daily,weekly,monthly,yearly}`, weekday/window helpers).
- **TZID + Graph datetime resolution** - `crates/core/src/caldav/parse.rs::extract_datetime` (and the `parse_icalendar` / `extract_vevent` paths) plus `crates/graph/src/calendar_sync.rs::parse_graph_datetime` and `resolve_graph_tz`.
- **CalDAV consolidation** - post-consolidation CalDAV stack: `crates/calendar/src/caldav/` delegating to `rtsk::caldav::client::CalDavClient` and `rtsk::caldav::sync`.
- **Outside Reviewer A** - combined RRULE / TZ / CalDAV pass.

---

## Round 2 (closed)

Addressed in commits 4b5c6c97..5401618b. Topics, by area of fix:

- RRULE expansion respects `event.timezone` end-to-end via the
  `RecurrenceTz` enum; wall-clock duration replaces raw seconds. The
  CalDAV producer now plumbs the resolved IANA name through into
  `ParsedVEvent.timezone` and onto the row's `timezone` column, so the
  expander walks recurrences in the source zone for CalDAV (matching
  Graph's existing behavior).
- All-day event end_time anchored to `start + days*86400` in CalDAV
  parse and Graph map paths; recurring all-day duration computed in
  event zone via a date-delta wall_duration so spring-forward weeks
  don't inherit a 25-hour duration on every subsequent instance.
- RECURRENCE-ID folded into the CalDAV sync key, with the canonical
  wall-clock form as the discriminator. Override rows stay distinct from
  the master, master expansion subtracts override slots at load time,
  and abandoned overrides get reaped when a multi-VEVENT resource
  shrinks. The canonical-form key is host-TZ-independent for floating
  and all-day RECURRENCE-IDs.
- `time.rs::resolve_local_to_timestamp` uses dual-walk gap-width
  detection (1-min, 48h bound). Lord Howe 30-min and Pacific/Apia 24h
  cases pass.
- ETag handling: `prepare_if_match_etag` drops weak ETags from `If-Match`
  per RFC 7232 § 2.3.2; `response_etag` recovers non-ASCII via lossy
  UTF-8.
- CalDAV discovery: `propfind_with_final_url` for redirect-aware
  principal resolution; `ensure_collection_trailing_slash` for URL join
  under collections; `relativize_for_multiget` for SOGo compat. *(See
  Round 3 #30, #31, #32 - only the well-known path got the final-URL
  fix; the base-URL probe and `list_calendars` resolution still resolve
  against `base_url`.)*
- CalDAV parse: reject sub-collections via `<D:resourcetype><D:collection/>`,
  accept `application/calendar+xml`, strip query before `.ics` check,
  fold Apple ARGB color to RGB. *(See Round 3 #23, #25 - third-arm
  fallback still ignores query strings, self-closed empty privilege-set
  flips can_edit=false.)*
- `current-user-privilege-set` parsed; `DiscoveredCalendar.can_edit:
  Option<bool>` honored by `sync_caldav_calendars`. *(See Round 3 #40 -
  the calendar-crate facade `caldav_list_calendars_impl` still
  hard-codes `can_edit: true`.)*
- Graph fractional-second truncation preserves trailing offset.
- `sync_caldav_calendar_account` retries with rediscovery on stale
  persisted URLs. *(See Round 3 #32 - retry only fires when sync fails,
  not when client construction fails on the persisted principal.)*
- Misc lows: `parse_until_date` rejects years outside 1..=9999;
  `YEARLY_MAX_STEPS=80_000`; explanatory comments on WEEKLY+BYDAY
  DTSTART semantics and on the WKST/BYWEEKNO coupling.

---

## Round 3 (open) - by code area

Six reviewer outputs across three lenses. Findings are grouped by code
area; each entry carries `[lens/provider]` tags showing who flagged it.
Multi-tag entries are cross-flagged and the highest-confidence signals.

### `crates/db/src/db/queries_extra/calendars.rs`

#### `expand_recurrence` / load path

6. **Medium** [bugs/claude M1] - **Windows zone names fall through to
   `chrono::Local`.** `RecurrenceTz::from_event_timezone` (`:1085-1103`)
   only resolves IANA via `chrono_tz::Tz::from_str`. Microsoft display
   names ("Pacific Standard Time") that the CalDAV/Graph parse layer
   resolves at parse time get stored as the verbatim string in
   `event.timezone`, and recurrence expansion then falls back to
   `RecurrenceTz::Local`. For a Mon-Fri 09:00 series with
   `event.timezone="Pacific Standard Time"` viewed from a NY-based
   user: series start anchored correctly, but daily expansion advances
   in NY local; across the staggered DST transitions of NY vs PST the
   wall-clock-in-PST drifts by an hour. Fix: route through the graph
   crate's `resolve_graph_tz` or pull calcard's alias map into the db
   crate.

7. **Low** [arch/claude Low] - **`parse_until_date` ignores the event's
   RecurrenceTz.** `:1965-2013`. `expand_recurrence` threads
   `RecurrenceTz` through every helper, but `parse_until_date` is
   called from `parse_rrule` *before* tz is known and unconditionally
   anchors floating + DATE-only UNTIL in `chrono::Local`. NY event
   with `RRULE:FREQ=DAILY;UNTIL=20260315` from a host in Pacific/Auckland:
   expected last instance Mar 15 NY-local; observed Mar 14 NY-local.
   Apple/Google anchor in event zone. Fix: keep a raw `Until` enum
   (`Date(NaiveDate)` / `DateTime(NaiveDateTime)` / `Utc(i64)`) and
   resolve inside `expand_recurrence` once tz is in hand.

8. **Note** [bugs/claude notes] - **`parse_until_date` DATE-only +
   TZID-bearing DTSTART boundary clipping.** RFC 5545 says DATE-only
   UNTIL is only legal alongside floating DTSTART, but some Outlook
   bridges emit DATE-only UNTIL alongside TZID-bearing DTSTART (RFC
   violation, real in the wild). Off-by-some-hours UNTIL clipping for
   west/east-of-UTC users.

9. **Note** [arch/claude notes] - **`expand_recurrence` `wall_duration`
   collapses to 0 when the master event lives entirely inside a DST
   gap.** `DTSTART;TZID=America/New_York:20260308T023000` +
   `DTEND;TZID=...:20260308T033000` - both 02:30 and 03:30 resolve to
   03:30 EDT post-shift, raw `end-start=0`, every recurring instance
   inherits zero-length end. Pre-existing parse-time issue
   (`extract_datetime` resolves both endpoints through the gap), not
   introduced in Round 2; flagged for follow-up.

10. **Cosmetic** [arch/claude notes] - **`expand_yearly` overflow guard
    has dead code.** `i32::try_from(rule.interval).unwrap_or(1).max(1)`
    at `:1779` - `parse_rrule` already clamps interval to ≥1, so the
    `.max(1)` is unreachable.

11. **Medium (perf)** [perf/claude M5] - **`expand_daily` allocates a
    fresh `Vec<Weekday>` per iteration.** `:1518-1530`. Up to 12,000
    Vec allocations per `expand_daily`. Hoist the `Vec<Weekday>` (or
    change `matches_weekday` to take `&[ByDay]`) before the loop.

12. **Medium (perf)** [perf/claude M6] - **`expand_yearly` clones
    `rule.bymonth` per year.** `:1797-1801`, inside the
    80,000-iteration loop. Hoist; pass as `&[u32]`.

13. **Medium (perf)** [perf/claude M7] - **`with_year_month_day`
    recomputes `tz.naive(start)` every call.** `:1841-1852`. `start`
    is the constant master timestamp, so `tz.naive(start)` is
    invariant across the entire expansion. In `expand_yearly` it's
    resolved up to 80k × 12 × ~30 = ~30M times. Compute the
    wall-clock `time()` once before the year loop and pass it in.

14. **Medium (perf)** [perf/claude M8] - **`weekday_occurrences_in_month`
    does ≤31 `from_ymd_opt` per call.** `:1727-1735`. Replace with:
    compute the weekday of day 1 once, then iterate
    `(1..=dim).filter(|d| target == day_1_weekday + (d-1) mod 7)`.
    Drops 30 date constructions per month-with-BYDAY.

15. **Medium (perf)** [perf/claude M9] - **`expand_monthly` /
    `expand_yearly` allocate a 1-element `vec![original_day]` per
    iteration.** `:1640-1644, 1804-1810`. Default-day MONTHLY/YEARLY
    allocates a 1-elem Vec, sorts, dedups, iterates. Inline the
    single-day check.

16. **Note (perf)** [perf/codex Medium implicit, perf/claude reference] -
    **`YEARLY_MAX_STEPS=80_000` is large but no longer a render-path
    concern.** Sparse YEARLY rules
    (`BYMONTH=2;BYMONTHDAY=29;COUNT=10000`) walk up to 40k years before
    the count cap fires; expansion now runs off the connection mutex via
    `expand_view_events`, so this only burns a CPU thread for a single
    pathological row rather than blocking sync workers and IPC.

17. **Low** [bugs/claude L1] - **`start_of_week` fallback returns the
    un-walked timestamp; `shift_to_weekday` then trusts that anchor.**
    `time.rs:65 + calendars.rs:1872-1898 + :1900-1930`. For a
    Sunday-anchored week (`WKST=SU`) where `start_of_week` fell back
    to its Wednesday input, `shift_to_weekday(anchor=Wed, target=SU,
    wkst=SU)` computes `target_offset=0` and returns Wednesday - silent
    weekday-emission error. Trigger requires a 24h-skipped day in the
    walk path; less likely after the time.rs gap-walk fix, but the
    fallback shape is now silently *more* wrong than the previous
    "weekly instances are off by some days" behavior. Fix: return
    `None` from `start_of_week` and skip the iteration, or have
    `shift_to_weekday` recompute offset from the actual weekday of
    the anchor.

### `crates/db/src/db/time.rs`

18. **Note (perf)** [perf/claude L17] - **`resolve_through_gap` walks
    1-minute steps, up to ~2880 per resolve.** `:60-107`. For
    Pacific/Apia 2011-12-30 the input lands inside a 1440-minute gap;
    forward walk may probe up to 1440 minutes with a
    `from_local_datetime` each step. ~1441 transition lookups for
    that one resolve. In recurrence loops only fires when an instance
    lands inside a gap (rare). Not a bug; flagged as a known cost.
    Coarse pre-walk via binary search would shave to log time but the
    simplicity is probably the right tradeoff.

### `crates/core/src/caldav/parse.rs`

23. **Low** [perf/claude L13, arch/claude Low] -
    **`is_icalendar_resource` third-arm fallback ignores
    query/fragment.** `:1196-1199`. The `.ics` check now strips
    `?…`/`#…` (good), but the fallback at `:1198-1199` still inspects
    raw `href`: `content_type.is_empty() && !href.ends_with('/')`. A
    collection URL with a query string (`/cal/folder/?revision=1`)
    doesn't end with `/`, so it falls through to "accept" if no
    content-type. The new collection-resourcetype gate at `:869-872`
    catches typical Davical / Bedework, but a server emitting no
    `<resourcetype>` and a query string still leaks. Same trim before
    suffix check would close it.

24. **Low (perf)** [perf/claude L16] - **`pick_datetime_entry` runs
    twice per all-day endpoint.** `:144-145, 175-188`.
    `extract_datetime(Dtstart)` calls `pick_datetime_entry(Dtstart)`,
    then `extract_all_day_date(Dtstart)` calls it again; same for
    Dtend. For all-day events that's 4 walks of
    `component.properties(prop)` instead of 2. Fix: have
    `pick_datetime_entry` return the picked entry alongside an
    `is_date_only` flag, and have `extract_vevent` pass it into both
    downstream helpers.

25. **Low** [arch/claude Low] - **`pending_privilege_set_seen` flips
    `can_edit=false` on a self-closed empty privilege set.**
    `:663-666`. `<D:current-user-privilege-set/>` (self-closed, no
    children) is unusual but real (some test mocks emit it for
    "unknown ACL") and lands the calendar at `can_edit=Some(false)`.
    The sync layer then suppresses edit affordances. The
    `<privilege>` ancestor check at `:669-673` doesn't help if a
    server emits privileges without the wrapper. Fix: only set
    `pending_privilege_set_seen` after observing at least one
    `<privilege>` child.

### `crates/core/src/caldav/sync.rs`

27. **Medium** [bugs/codex #3] - **CalDAV attendee/reminder removals
    are ignored when the new lists are empty.** `:355, :394` +
    `caldav_sync.rs:78, :136`. The DB helpers implement replacement
    semantics by deleting existing attendees/reminders before
    inserting the new list, but the sync layer returns early when the
    parsed list is empty. A remote update that removes all `ATTENDEE`
    or `VALARM` entries leaves stale local rows.

28. **Medium (perf)** [perf/claude M11] - **Multi-VEVENT iCal:
    redundant URI→ETag map writes per resource.** `:191-203,
    309-323`. For a recurring series shipped as one href with K
    VEVENTs (master + N overrides), `upsert_parsed_event` runs K
    times for the same URI, calling `upsert_caldav_event_map_sync` K
    times with the same `(uri, cal_id, uid, etag)` tuple. Lift the
    map upsert out of `upsert_parsed_event` and call it once after
    `for event in &events` in `sync_calendar_events`.

29. **Low** [perf/claude L14] - **`make_google_event_id` synthesized-
    from-URI path collides with UID-shaped-as-URI.** `:241-246`.
    `let uid = event.uid.clone().unwrap_or_else(|| uri.to_string())`
    then `caldav:{uid}`. If a real UID equaled another resource's URI,
    keys collide. No emitter does this but the type system permits.
    Cheap fix: distinct synthesized form, e.g. `caldav:href={uri}`.

### `crates/core/src/caldav/client.rs`

30. **Medium** [bugs/claude M2] - **`discover_principal`'s base-URL try
    doesn't capture the final URL.** `:243-259`. The well-known
    fallback path uses `propfind_with_final_url` and resolves the
    principal href against the post-redirect URL (the b8398928 fix).
    The base-URL try at `:246-258` still uses `propfind_raw` and
    `self.resolve_url(&principal)`, which resolves against
    `self.base_url`. If the base-URL PROPFIND gets host-redirected
    (Apple sometimes 30x's per-shard, hosted Exchange bridges rewrite
    to per-tenant DAV root) and the principal href is relative, the
    next PROPFIND lands on the original host. Fix: replace
    `propfind_raw(&self.base_url, ...)` with `propfind_with_final_url`
    and resolve via `resolve_url_against(&final_url, &principal)`.

31. **Medium** [arch/codex Medium] - **`list_calendars` resolves
    calendar hrefs against `base_url`, not `calendar_home_url`.**
    `:318` + `:726`. Loads `calendar_home_url` then calls
    `self.resolve_url(&cal.href)` which uses `self.base_url`. For
    hosts where the home-set lives on a different host than the base
    URL (Fastmail's `caldav.fastmail.com`, hosted Exchange bridges,
    redirect from `login.example/dav` to `cal.example/calendars/`),
    stored calendar URLs point at the wrong host. Later event
    PROPFIND/PUT/DELETE hit the wrong origin or path.

32. **Medium** [arch/codex Medium] - **CalDAV rediscovery retry doesn't
    cover stale persisted principal URLs.** `sync.rs:342, :364` +
    `mod.rs:294` + `client.rs:166`. `sync_caldav_calendar_account`
    calls `build_client_from_config(&config).await?` *before* the
    retry block, and the retry is gated on `!needed_discovery`.
    `build_client_from_config` seeds a persisted principal,
    `discover()` skips principal discovery when one is already set.
    Repro: DB has stale `caldav_principal_url` and
    `caldav_home_url=NULL`. `build_client_from_config` runs
    `discover()` (because home is None), which fails on the stale
    principal PROPFIND, `?` propagates from before the retry block,
    and `clear_persisted_caldav_urls` is never reached.

33. **Low** [arch/claude Low] - **`ensure_collection_trailing_slash`
    correctly slashes the path, but the subsequent `Url::join` drops
    query strings.** `:733-755` + `Url::parse(...).join(href)`. The
    helper handles a base with `?q=x` by inserting the slash before
    the query, but the very next `base_url.join(href)` drops the
    base's query per RFC 3986 § 5.3 unless `href` overrides it. A
    calendar URL of `https://h/cal/?token=auth` joined with
    `event.ics` resolves to `https://h/cal/event.ics`, no token.
    CalDAV usually authenticates via the `Authorization` header so
    this is rarely the failure mode in practice - but silent if a
    host *is* relying on a query token.

34. **Medium (perf)** [perf/claude M10] - **`relativize_for_multiget`
    re-parses the request URL per href.** `:996-1016`. `fetch_events`
    calls it for every URI in a 50-element batch; each call does
    `url::Url::parse(request_url)`. Parse the request URL once
    outside the inner `for uri in chunk` loop and pass the parsed
    `&Url` in.

35. **Low (perf)** [perf/claude L12] -
    **`extract_hrefs_property` allocates a `Vec<String>` for
    single-href callers.** `:835-886`. `current-user-principal`
    callers always do `.into_iter().next()`. Either return early when
    the first href closes for the single-href case, or split into
    `extract_first_href_property` / `extract_all_hrefs_property`.

36. **Low (perf)** [perf/claude L15] - **`response_etag` ASCII fast
    path makes an extra `to_string`.** `:1028-1043`. `to_str()`
    returns `&str`, which is `to_string()`'d to `String`. Equivalent
    to `val.to_str().ok().map(str::to_owned)`. Trivial; flagged
    because the function is on every PUT/DELETE/GET response path.

37. **Note** [arch/claude notes] - **Lossy ETag round-trip → If-Match
    silently omitted.** `:1028-1043`. With non-ASCII bytes the lossy
    String contains U+FFFD, which `HeaderValue::from_str` rejects, so
    `If-Match` is silently omitted. Net behavior is correct (never
    sends a corrupt validator), but the comment should be tightened:
    "we may degrade concurrency" → "If-Match will be omitted on the
    next write."

38. **Note** [arch/claude notes] - **Weak ETag dropped from `If-Match`
    operational impact.** `:1050-1053`. Correct per RFC 7232 § 2.3.2,
    but on Apache `mod_dav` with `FileETag MTime Size` *every* PUT
    becomes last-write-wins. Worth a CalDAV setup-doc note rather than
    a code change.

39. **Note** [bugs/claude notes] - **`prepare_if_match_etag` converts
    iCloud writes to last-write-wins.** iCloud uses weak ETags
    pervasively. Trade-off acknowledged in doc comment; flag for the
    racing-edits user complaint when it lands.

### `crates/calendar/src/caldav/mod.rs`

40. **Medium** [arch/codex Medium] - **CalDAV listing adapter discards
    parsed editability.** Parser records
    `current-user-privilege-set` at `parse.rs:729`, but
    `caldav_list_calendars_impl` hard-codes `can_edit: true` at
    `mod.rs:68`. If that DTO is passed to the shared upsert path at
    `sync.rs:105`, read-only calendars are persisted as editable.
    UI / action layer sees `can_edit=true` and later gets 403/405 on
    PUT/DELETE.

41. **Low** (carry-over from Round 2) - `join_calendar_path`
    (`:413-424`) drops query strings from calendar URLs via
    `Url::join` semantics. Edge case for shared-hosting CalDAV
    servers requiring routing query parameters.

### `crates/graph/src/calendar_sync.rs`

42. **Low** [arch/claude Low] - **Graph all-day correction silently
    disagrees with TZID for malformed payloads.** `:442-455`.
    `map_graph_event` recomputes end via `parse_graph_all_day_date`
    (which only reads the date from the dateTime string), independent
    of `time_zone`. If Graph ever returns an all-day event whose
    `start.timeZone` and `end.timeZone` differ (cross-zone "all-day"),
    the correction overrides whatever the per-side resolve produced.
    Probably the right call (start drives the zone) but worth a
    comment.

43. **Medium** (carry-over from Round 2) - `resolve_graph_tz` falls
    through to `None` for unknown Windows zone names (`:574`); the
    fallback now logs WARN with the offending zone name. Underlying
    calcard alias gap is the real fix. Repro: `2024-06-15T10:00:00`
    in `Africa/Juba` ("South Sudan Standard Time", not in calcard)
    becomes 10:00Z instead of 08:00Z (with a log line now).

### Carry-overs from Round 2 still open

45. **Medium** - **Inverted error signaling in `expand_recurrence`.**
    Malformed rules and rules using unsupported BY-rules now log a
    WARN and return `vec![event.clone()]` (single instance);
    well-formed-but-zero-instance rules (UNTIL in past, BYDAY excludes
    everything) still return `vec![]`. The malformed-vs-zero-instance
    signaling is no longer silent on the malformed side, but the
    caller still can't tell the two apart at the data level.
    Surfacing a "broken rule" indicator to the UI would close this
    fully.

46. **Medium** - **YEARLY+ordinal BYDAY without explicit BYMONTH** now
    WARN-logs and falls back to the master instance instead of
    silently emitting zero instances. Implementing the actual
    year-scope ordinal walk (n-th weekday of the year, walking across
    all 12 months) is the real fix and a follow-up.

47. **Medium (upstream)** - **Empty SUMMARY / DESCRIPTION / LOCATION**
    are still indistinguishable from absent. Local code is ready for
    when calcard surfaces empty values; user-cleared-title support
    requires an upstream calcard change before it can land.

48. **Medium** - **Folded-line + CRLF handling depends on calcard.**
    LF-only line endings (some Linux bridges) may fail to unfold;
    long DESCRIPTION lines get truncated. Worth a unit test covering
    LF-only + folded long line.

49. **Medium** - **PROPFIND 207 with zero `<response>` children**
    returns empty Vec with no log; indistinguishable from "no
    calendars provisioned" / first-login race / server-side error
    misreported as 207.

50. **Medium** - **Multi-href delegation home-sets** are collected by
    `extract_hrefs_property` (returns `Vec<String>`), but only the
    first is consumed by the discovery flow. Reaching the rest
    requires plumbing `Vec<String>` through `calendar_home_url`
    (single `Option<String>` today), the persisted DB column, and
    `list_calendars`. Multi-href encounters now log a WARN.

51. **Medium** - **Empty remote set guard** in `sync_calendar_events`
    skips the deletion phase when remote returns 0 entries against a
    non-empty local cache. Still open: the propstat-status-not-
    inspected case where individual entries get filtered out as
    "absent" mid-batch even though the response was 207-OK overall.

### Test gaps flagged

52. **Note** [perf/claude notes] - **`RRULE COUNT=0`** parsed as
    `Some(0)`, inner cap=0, expand returns empty Vec, master event
    silently dropped from the view. Probably acceptable, but no test
    pins the behavior.

53. **Note** [perf/claude notes] - **`expand_recurrence` with
    `event.end_time < event.start_time`** produces negative
    `wall_duration`, which propagates negative
    `end_time_for_instance` outputs. Untested.

54. **Note** [perf/claude notes] - **`expand_yearly` with
    `INTERVAL=2_000_000_000`**: `i32::try_from` fails → fallback to
    1, then the 2-year-window/COUNT/UNTIL bounds limit. No test for
    the silent-degradation path.

55. **Note** [perf/claude notes] - **`discover_principal` against a
    base URL that itself redirects** - `propfind_raw` doesn't track
    final URL, so a relative principal href resolves against the
    original `base_url` rather than the redirect target. Same shape
    as #30. Less common but the fix shape mirrors
    `propfind_with_final_url` and is small.

---

## Verified non-issues

Behaviors that look bug-like in review but were checked and found correct.
Listed here so they don't get re-flagged on every future pass; each is a
candidate for a brief `// reviewed: ...` code comment at the call site.

### `crates/db/src/db/time.rs::resolve_local_to_timestamp`

- `Tz::Fixed` (fixed-offset zones from VTIMEZONE) routes through the generic resolver correctly: `Tz::Fixed` never produces `LocalResult::None` or `LocalResult::Ambiguous`, so the gap/ambiguous fallbacks never engage.

### `crates/db/src/db/queries_extra/calendars.rs::collect_monthly_days` (1457-1468)

- `BYMONTHDAY` negative values: `dim_i + d + 1` correctly resolves `-31` to day 1 in 31-day months and to `<1` (filtered out at `:1461`) in shorter months. MONTHLY-only emission in 31-day months is the intended behavior.
- `BYMONTHDAY=29;BYMONTH=2` (leap day): `days_in_month(non-leap, 2) = 28` makes the bound check at `:1461` fail, no candidate emitted - matches dateutil's skip-non-leap behavior. Default-day path at `:1551-1556` handles Feb 29 starts the same way.

### `crates/db/src/db/queries_extra/calendars.rs::parse_byday` (1266-1303)

- Empty `BYDAY=` value: `val.split(',')` yields one empty string, `parse_byday("")` returns `None`, `filter_map` discards it; `out.byday` ends up `vec![]` and rule expansion proceeds as if BYDAY were absent. Tolerant - strict parsers would reject the rule entirely.
- `BYDAY=,MO,` (leading/trailing commas) silently drops the empty entries and keeps `Mo`.

### `crates/db/src/db/queries_extra/calendars.rs` BYDAY mixing

- `BYDAY=2WE,-1FR` (two distinct ordinals): each entry is independently resolved in `collect_monthly_days` (`:1445-1453`); `sort_unstable + dedup` (`:1412-1413`) keeps both in calendar order.
- `BYDAY=MO,1FR` (bare + ordinal mixed): bare `MO` flat-maps to all Mondays, `1FR` resolves to the first Friday only. Matches RFC 5545.

### `crates/db/src/db/queries_extra/calendars.rs::expand_weekly` complex traces

- `FREQ=WEEKLY;BYDAY=MO,WE,FR;INTERVAL=2;COUNT=10` from a Monday DTSTART traces correctly: emits Mon/Wed/Fri of the start week, then advances `week_anchor` by 14 days, etc. Matches dateutil.
- `FREQ=MONTHLY;BYDAY=2WE,-1FR` with a normal month anchor traces correctly (e.g. 2026-01-14, 2026-01-30, 2026-02-11, 2026-02-27).

### Schema-level: sub-second DTSTART (i64 timestamps)

- `CalendarViewEvent.start_time` is `i64` seconds and the entire pipeline operates at second precision. Sub-second DTSTART (`19700101T000000.500Z`) is truncated by the field carrying it in; not an expander concern. RFC 5545 itself uses second precision throughout.

### `crates/core/src/caldav/parse.rs::parse_propfind_calendars` (multi-`<propstat>`)

- The parser doesn't accumulate across multiple `<propstat>` blocks at the data-structure level, but on second read this works because each `<prop>` block overwrites only the `current_*` fields it contains, all initialized at `<response>`. The real concern is the missing `<status>` inspection - captured under the High-severity finding above.

### `crates/core/src/caldav/parse.rs::parse_propfind_events` ETag content

- ETags with embedded colons and slashes (`"abc/def:1"`) are preserved verbatim including the surrounding quotes. RFC 7232 allows these as long as the quote is preserved; the storage path doesn't try to re-parse, so they round-trip correctly.

### `crates/core/src/caldav/parse.rs` HTML entity handling

- `unescape()` on `Event::Text` correctly decodes `&amp;`, `&#x2014;` (em dash), and other XML entities for display names and other text fields. CDATA-wrapped element content is also handled.
