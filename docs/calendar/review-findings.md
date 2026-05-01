# Calendar Code Review Findings

Captured 2026-05-01 from review passes; consolidated and reorganized by code
area on the same date so the file drives fix work rather than recording who
caught what. Items independently flagged by more than one reviewer carry a
`(flagged Nx)` tag - those are the highest-confidence signals.

Round 1 (per-lens passes plus an outside reviewer) is closed; Round 2 is the
open backlog and makes up the body of this file. Verbatim per-reviewer text
from Round 2 is no longer preserved separately - the consolidated entries
below carry each finding's substance plus file:line and a one-line repro.

---

## Round 1 (closed)

All findings addressed. Lenses and targets:

- **RRULE expansion** - `crates/db/src/db/queries_extra/calendars.rs::expand_recurrence` and helpers (`parse_rrule`, `parse_byday`, `expand_{daily,weekly,monthly,yearly}`, weekday/window helpers).
- **TZID + Graph datetime resolution** - `crates/core/src/caldav/parse.rs::extract_datetime` (and the `parse_icalendar` / `extract_vevent` paths) plus `crates/graph/src/calendar_sync.rs::parse_graph_datetime` and `resolve_graph_tz`.
- **CalDAV consolidation** - post-consolidation CalDAV stack: `crates/calendar/src/caldav/` delegating to `rtsk::caldav::client::CalDavClient` and `rtsk::caldav::sync`.
- **Outside Reviewer A** - combined RRULE / TZ / CalDAV pass.

---

## Round 2 (open) - by code area

### `crates/db/src/db/time.rs::resolve_local_to_timestamp` (42-58)

- **Low** (flagged 2x) - Spring-forward walker tries `+60min` first; lands wrong for sub-hour DST gaps. Repro: `Australia/Lord_Howe`, input `02:15` on the 30-minute spring-forward day returns `03:15 LHST` instead of `02:45 LHDT`.
- **Low** - Tied ambiguous result on the minute-by-minute walk picks the pre-fall-back instant. Triggered by zones with double transitions in close sequence (Antarctica/Casey 2009-10).
- **Medium** (flagged 2x) - Returns `None` for 24-hour skipped days; callers fall back to raw-seconds arithmetic. Repro: `Pacific/Apia` Dec 30 2011 in `add_days_local` for the weekly anchor (`calendars.rs:1346, 1383`) silently emits instances 24h offset.
- **Latent** - Same raw-seconds fallback path: any DST-gap day where `add_days_local` returns `None` causes daily wall-clock-preserving events to slip from 09:00 to 10:00 from that day forward (`calendars.rs:1326`).

### `crates/db/src/db/queries_extra/calendars.rs::expand_recurrence` (1052-1136)

- **High** (flagged 4x) - Expansion uses `chrono::Local` everywhere; `event.timezone` is never read. Repro: `DTSTART;TZID=Pacific/Kiritimati:20260101T090000` with `FREQ=MONTHLY` on an Oslo machine emits Jan 1, Feb 1, Apr 1, Jun 1 (Oslo sees DTSTART as Dec 31; `advance_months` preserves day 31 and skips short months). Daily TZID-anchored events also drift across DST.
- **Medium** - Inverted error signaling: malformed rule (unknown FREQ, empty FREQ field) and rules using unsupported BY-rules (BYSETPOS, BYWEEKNO, BYYEARDAY, BYHOUR, BYMINUTE, BYSECOND) now log a WARN and return `vec![event.clone()]` (single instance); well-formed-but-zero-instance rules (UNTIL in past, BYDAY excludes everything) still return `vec![]`. The malformed-vs-zero-instance signaling is no longer silent on the malformed side, but the caller still can't tell the two apart at the data level. Surfacing a "broken rule" indicator to the UI would close this fully.
- **Medium** (flagged 2x) - Recurring all-day events store duration as raw elapsed seconds (`:1083`); the first occurrence spanning DST is 47h or 23h and that wrong duration propagates. Repro: all-day weekly `DTSTART;VALUE=DATE:20240331;DTEND;VALUE=DATE:20240402;RRULE:FREQ=WEEKLY;COUNT=2` in a DST-springing zone emits the next instance ending 23:00 local instead of midnight.
- **Medium** - Unbounded `FREQ=DAILY` does not honor the advertised two-year fallback window. `window_end` is two years out but `max_instances` defaults to 365 and `expand_daily` defaults to 366 candidates (`:1100, :1106, :1306`). Repro: `FREQ=DAILY` from 2026-01-01 stops at 2026-12-31 instead of 2028-01-01.

### `crates/db/src/db/queries_extra/calendars.rs::expand_yearly` (1525-1579)

- **High** (flagged 2x) - YEARLY+ordinal BYDAY without explicit BYMONTH visits only `dt.month()` and asks for the n-th weekday of *that month*. Repro: `FREQ=YEARLY;BYDAY=20MO` (the 20th Monday of the year) emits nothing because no month has 20 Mondays. Fix scope: implement year-scope ordinal lookup, or reject ordinal BYDAY when FREQ=YEARLY and BYMONTH is unset.
- **High** - Yearly Feb 29 drifts to March 29 in non-leap years via `advance_months(Feb 29, 12)` (`:1576`). Repro: `DTSTART=20240229T090000;RRULE:FREQ=YEARLY;COUNT=5` emits 2024-02-29, 2025-03-29, 2026-03-29, 2027-03-29, 2028-03-29 instead of 2024/2028/2032/2036/2040.
- **Low** - Sparse YEARLY rules (e.g. leap-day-only) silently truncate before COUNT cap; outer step is yearly with `RRULE_MAX_STEPS=12_000`, so `COUNT=10000` never realized for `BYMONTH=2;BYMONTHDAY=29`.

### `crates/db/src/db/queries_extra/calendars.rs::expand_monthly` (1425, 1686)

- **High** - `advance_months(current, interval)` skips months that don't contain the original day-of-month, even when explicit BYMONTHDAY/BYDAY supplies the actual candidates. Repro: `DTSTART=20260131T090000;RRULE:FREQ=MONTHLY;BYMONTHDAY=1,-1;COUNT=5` emits 2026-01-31, 2026-03-01, 2026-03-31, 2026-05-01, 2026-05-31 (Feb and April skipped because the anchor stays "31").

### `crates/db/src/db/queries_extra/calendars.rs::expand_weekly`

- **Low** - WEEKLY+BYDAY excluding DTSTART silently drops DTSTART. Matches dateutil; deviates from the strict RFC 5545 reading that DTSTART is always in the recurrence set. No comment in code; worth a deliberate decision.

### `crates/db/src/db/queries_extra/calendars.rs::start_of_week` (1613-1626)

- **Low** - `add_days_local(...).unwrap_or(timestamp)` returns the original timestamp on failure, used as the week-start anchor; downstream weekly expansion emits instances one entire week off, no caller indication. Triggered by zones with skipped days (Apia Dec 30 2011) crossed during walk-back.

### `crates/db/src/db/queries_extra/calendars.rs::parse_rrule` (1200-1244)

- **Low** (flagged 2x) - `INTERVAL=0` silently clamped to 1 (RFC says >=1; defensible but no signal to caller).
- **Low** (flagged 2x) - `BYMONTHDAY=32` / `BYMONTH=13` silently dropped, falls back to the default same-day-of-month / same-month-as-start (instead of zero-instance). User typing "32nd of month" gets a working monthly recurrence on the DTSTART day.
- **Latent** - WKST only consulted in WEEKLY path; YEARLY/MONTHLY ignore it. OK while BYWEEKNO is unsupported; will silently break if BYWEEKNO is added without plumbing WKST through.
- **Low** (flagged 2x) - COUNT capped at `RRULE_MAX_COUNT=10_000` with no signal to caller.

### `crates/db/src/db/queries_extra/calendars.rs::parse_byday` (1266-1303)

- **Trivial** - Ordinals > 53 (`BYDAY=99MO`) not rejected; `nth_weekday_in_month` returns `None` and the rule iterates 12000 times before bounding out (DoS-resistant by accident).

### Year-bounds sanity (multiple files)

- **Low** - `chrono::NaiveDate::from_ymd_opt` accepts year 0 / 9999 / negative; `parse_until_date` accepts year 0 (UNTIL becomes a large negative timestamp; rule emits zero instances - bounded but surprising). YEARLY rule with `INTERVAL=10000` blows past chrono max-year, falls back to `start + 730*86400` (loop terminates safely).

---

### `crates/core/src/caldav/parse.rs::extract_datetime` (236-305)

- **High** (flagged 2x) - All-day events spanning DST get the wrong duration. `build_local_midnight` resolves both DTSTART/DTEND midnights through `chrono::Local`, so a Mar 9 -> Mar 11 EST/EDT-spanning all-day event has 23h instead of 48h, breaking `(end-start)/86400` consumers. Same root cause in `graph/calendar_sync.rs:508-517`.

### `crates/core/src/caldav/parse.rs::extract_vevent` / `parse_icalendar` (69-90, 98-121)

- **Medium** (upstream) - Empty SUMMARY / DESCRIPTION / LOCATION are still indistinguishable from absent. Removed the local `.filter(|s| !s.is_empty())` step (forward-compat for any future calcard release that surfaces empty values), but calcard's parser drops `SUMMARY:` from the entries list before our chain sees it, so user-cleared-title support requires an upstream calcard change. Tracking here so the local code is ready when it lands.
- **Medium** - Folded-line + CRLF handling depends on calcard. LF-only line endings (some Linux bridges) may fail to unfold; long DESCRIPTION lines get truncated. Worth a unit test covering LF-only + folded long line.
- **Medium** - Missing or unparseable DTSTART/DTEND yields `start_time=0` (Unix epoch) downstream (`sync.rs:234`), not a logged parse failure. Users see broken events at epoch instead of skipped/diagnosed bad items.
### `crates/core/src/caldav/parse.rs::is_icalendar_resource` (711-720)

- **Medium** - Sub-collection URIs without trailing slash treated as event resources via the third-arm fallback (Davical, Bedework, old Zimbra emit collection URIs without `/`). Triggers REPORT 403 on the collection, which can fail the whole batch.
- **Low** - Hrefs with query strings (`/cal/event.ics?revision=42`) end with neither `.ics` nor `/`; pass third arm only with empty content-type. CalDAV-XML form (`application/calendar+xml`, RFC 6321) silently skipped.

### `crates/core/src/caldav/parse.rs::parse_propfind_calendars` / `parse_propfind_events` / `parse_multiget_report` (402-690)

- **High** - Recurring resources with master + override VEVENTs collapse: sync key is `caldav:{UID}` (`sync.rs:212, 289`); same-UID different-RECURRENCE-ID instances overwrite each other. User sees only one occurrence or the exception replacing the master. Fold RECURRENCE-ID into the key.
- **Medium** - 207 with zero `<response>` children returns empty Vec with no log; indistinguishable from "no calendars provisioned" / first-login race / server-side error misreported as 207.
- **Low** - `parse_propfind_calendars` reads `calendar-color` verbatim and doesn't normalize Apple's ARGB form (`#0000FFFF`) to RGB hex (`#0000FF`) that most other servers emit. UI code consuming the color sees two different formats and must handle both. Either normalize at parse time or document the divergence at the UI consumer.

---

### `crates/core/src/caldav/client.rs::extract_hrefs_property` (formerly `extract_href_property`)

- **Medium** - Multi-href delegation home-sets are now collected (function returns `Vec<String>`), but only the first is currently consumed by the discovery flow. Reaching the rest requires plumbing `Vec<String>` through `CalDavClient::calendar_home_url` (single `Option<String>` today), the persisted home_url DB column, and `list_calendars` (currently iterates one home). When a multi-href home-set is encountered the `discover` path now logs a WARN so an operator can see the delegation case is hitting; full delegation support is a follow-up.

### `crates/core/src/caldav/client.rs` ETag handling (391-395, 441-445, 708-715)

- **Medium** (flagged 2x) - `normalize_if_match_etag` mangles the legacy weak-ETag form. Stored value `W/abc` (no inner quote) gets wrapped to `"W/abc"` (literal characters); strict servers 412 on every update; user retries hit the same 412 because the stored value never refreshes without a full resync. Better: drop `If-Match` when the stored value matches `W/[^"]`.
- **Medium** - Servers emitting weak ETags (`W/"..."`) and enforcing RFC strong comparison for `If-Match` reject every update/delete with 412.
- **Medium** - ETags with non-ASCII bytes silently dropped via `to_str()` (Yahoo, Kerio, Zimbra). Causes an extra GET round-trip on save and breaks optimistic concurrency on the next PUT.

### `crates/core/src/caldav/client.rs::discover` / `discover_principal` (128-207)

- **Medium** - Relative principal/home hrefs returned by a redirected `.well-known/caldav` are resolved against the original base URL, not the redirect target.
- **Medium** - Scheme-downgrade redirect risk: depends on reqwest's version stripping `Authorization` on cross-origin redirects. Verify in `Cargo.toml`.
- **Medium** - `build_client_from_config` (`mod.rs:294-300`) replays persisted principal/home; if home is present, discovery is skipped entirely. No rediscovery fallback when persisted URLs go stale (server migration, principal deletion, etc.).

### `crates/core/src/caldav/client.rs::fetch_events` multiget body (278-292)

- **Medium** - Absolute hrefs in the multiget body when the calendar URL went through a host/scheme redirect; older SOGo rejects absolute hrefs that don't share scheme+host with the request URL. Re-relativize before chunking.

### `crates/core/src/caldav/client.rs::resolve_url_against` (566)

- **Medium** - `Url::join` against a collection href without a trailing slash drops the last segment. Repro: calendar listed as `https://host/cal/user/work` and event hrefs as `event.ics` resolves to `https://host/cal/user/event.ics`, not `.../work/event.ics`. Some servers are loose about trailing slashes.

---

### `crates/core/src/caldav/sync.rs`

(Issues here are downstream of the parse.rs / client.rs findings above; listed for fix-sequencing visibility.)

- **Medium** - Empty remote set is now guarded against wiping the local cache (`sync_calendar_events` skips the deletion phase when remote returns 0 entries against a non-empty local cache). Still open: when the *initial* sync of a calendar legitimately starts empty and the server later begins returning entries, the heuristic doesn't mis-fire (remote=0 with stored=0 is a no-op). The remaining failure mode is the propstat-status-not-inspected case below, where individual entries get filtered out as "absent" mid-batch even though the response was 207-OK overall.
- **High** - Sync key is `caldav:{UID}` (`:212, :289`); master + override VEVENTs collide on `(account_id, google_event_id)`. Need RECURRENCE-ID folded into the key.
- **Medium** - Missing or unparseable DTSTART stores `start_time=0` (`:234`); user sees an epoch event.
- **Low** - `can_edit=true` hard-coded on upsert (`:53, :64`); read-only calendars from iCloud / Fastmail / SOGo show edit affordances and 403 on PUT/DELETE.

---

### `crates/calendar/src/caldav/mod.rs`

- **Low** - `caldav_list_calendars_impl` hard-codes `can_edit: true` (`:65`); CalDAV `<privilege>` inspection not implemented.
- **Low** - `join_calendar_path` (`:413-424`) drops query strings from calendar URLs via `Url::join` semantics. Edge case for shared-hosting CalDAV servers requiring routing query parameters.

---

### `crates/graph/src/calendar_sync.rs`

- **High** (flagged 2x) - All-day events spanning DST get 23h duration in `parse_graph_datetime` all-day branch (`:508-517`). Same root cause and fix as CalDAV `extract_datetime` / `build_local_midnight`.
- **Medium** - `resolve_graph_tz` falls through to `None` for unknown Windows zone names (`:574`); `parse_graph_datetime` then stores the naive wall clock as UTC (`:538`). The fallback now logs WARN with the offending zone name so an operator can see which payloads it's mis-anchoring; the underlying calcard alias gap is still the real fix. Repro: `2024-06-15T10:00:00` in `Africa/Juba` ("South Sudan Standard Time", not in calcard) becomes 10:00Z instead of 08:00Z (with a log line now).
- **Low** - Brittle `'.' before 'T'` truncation logic. Sub-minute precision (`T10:00:00.5+02:00`) silently drops the offset; if Graph ever emits sub-minute precision, parsing silently corrupts the offset.

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

- ETags with embedded colons and slashes (`"abc/def:1"`) are preserved verbatim including the surrounding quotes. RFC 7232 allows these as long as the quote is preserved; `normalize_if_match_etag` doesn't try to re-parse, so they round-trip correctly.

### `crates/core/src/caldav/parse.rs` HTML entity handling

- `unescape()` on `Event::Text` correctly decodes `&amp;`, `&#x2014;` (em dash), and other XML entities for display names and other text fields. CDATA-wrapped element content is also handled.

