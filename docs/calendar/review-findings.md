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

### `crates/db/src/db/queries_extra/calendars.rs::expand_recurrence` (1052-1136)

- **Medium** - Inverted error signaling: malformed rule (unknown FREQ, empty FREQ field) and rules using unsupported BY-rules (BYSETPOS, BYWEEKNO, BYYEARDAY, BYHOUR, BYMINUTE, BYSECOND) now log a WARN and return `vec![event.clone()]` (single instance); well-formed-but-zero-instance rules (UNTIL in past, BYDAY excludes everything) still return `vec![]`. The malformed-vs-zero-instance signaling is no longer silent on the malformed side, but the caller still can't tell the two apart at the data level. Surfacing a "broken rule" indicator to the UI would close this fully.

### `crates/db/src/db/queries_extra/calendars.rs::expand_yearly` (1525-1579)

- **Medium** - YEARLY+ordinal BYDAY without explicit BYMONTH (e.g. `FREQ=YEARLY;BYDAY=20MO` = "20th Monday of the year") now WARN-logs and falls back to the master instance instead of silently emitting zero instances. Implementing the actual year-scope ordinal walk (n-th weekday of the year, walking across all 12 months) is the real fix and a follow-up.
- **Low** - Sparse YEARLY rules (e.g. leap-day-only) silently truncate before COUNT cap; outer step is yearly with `RRULE_MAX_STEPS=12_000`, so `COUNT=10000` never realized for `BYMONTH=2;BYMONTHDAY=29`.

### `crates/db/src/db/queries_extra/calendars.rs::expand_weekly`

- **Low** - WEEKLY+BYDAY excluding DTSTART silently drops DTSTART. Matches dateutil; deviates from the strict RFC 5545 reading that DTSTART is always in the recurrence set. No comment in code; worth a deliberate decision.

### `crates/db/src/db/queries_extra/calendars.rs::parse_rrule` (1200-1244)

- **Latent** - WKST only consulted in WEEKLY path; YEARLY/MONTHLY ignore it. OK while BYWEEKNO is unsupported; will silently break if BYWEEKNO is added without plumbing WKST through.

### Year-bounds sanity (multiple files)

- **Low** - `chrono::NaiveDate::from_ymd_opt` accepts year 0 / 9999 / negative; `parse_until_date` accepts year 0 (UNTIL becomes a large negative timestamp; rule emits zero instances - bounded but surprising). YEARLY rule with `INTERVAL=10000` blows past chrono max-year, falls back to `start + 730*86400` (loop terminates safely).

---

### `crates/core/src/caldav/parse.rs::extract_vevent` / `parse_icalendar` (69-90, 98-121)

- **Medium** (upstream) - Empty SUMMARY / DESCRIPTION / LOCATION are still indistinguishable from absent. Removed the local `.filter(|s| !s.is_empty())` step (forward-compat for any future calcard release that surfaces empty values), but calcard's parser drops `SUMMARY:` from the entries list before our chain sees it, so user-cleared-title support requires an upstream calcard change. Tracking here so the local code is ready when it lands.
- **Medium** - Folded-line + CRLF handling depends on calcard. LF-only line endings (some Linux bridges) may fail to unfold; long DESCRIPTION lines get truncated. Worth a unit test covering LF-only + folded long line.
### `crates/core/src/caldav/parse.rs::parse_propfind_calendars` / `parse_propfind_events` / `parse_multiget_report` (402-690)

- **Medium** - 207 with zero `<response>` children returns empty Vec with no log; indistinguishable from "no calendars provisioned" / first-login race / server-side error misreported as 207.

---

### `crates/core/src/caldav/client.rs::extract_hrefs_property` (formerly `extract_href_property`)

- **Medium** - Multi-href delegation home-sets are now collected (function returns `Vec<String>`), but only the first is currently consumed by the discovery flow. Reaching the rest requires plumbing `Vec<String>` through `CalDavClient::calendar_home_url` (single `Option<String>` today), the persisted home_url DB column, and `list_calendars` (currently iterates one home). When a multi-href home-set is encountered the `discover` path now logs a WARN so an operator can see the delegation case is hitting; full delegation support is a follow-up.

---

### `crates/core/src/caldav/sync.rs`

(Issues here are downstream of the parse.rs / client.rs findings above; listed for fix-sequencing visibility.)

- **Medium** - Empty remote set is now guarded against wiping the local cache (`sync_calendar_events` skips the deletion phase when remote returns 0 entries against a non-empty local cache). Still open: when the *initial* sync of a calendar legitimately starts empty and the server later begins returning entries, the heuristic doesn't mis-fire (remote=0 with stored=0 is a no-op). The remaining failure mode is the propstat-status-not-inspected case below, where individual entries get filtered out as "absent" mid-batch even though the response was 207-OK overall.
- **Low** - `can_edit=true` hard-coded on upsert (`:53, :64`); read-only calendars from iCloud / Fastmail / SOGo show edit affordances and 403 on PUT/DELETE.

---

### `crates/calendar/src/caldav/mod.rs`

- **Low** - `caldav_list_calendars_impl` hard-codes `can_edit: true` (`:65`); CalDAV `<privilege>` inspection not implemented.
- **Low** - `join_calendar_path` (`:413-424`) drops query strings from calendar URLs via `Url::join` semantics. Edge case for shared-hosting CalDAV servers requiring routing query parameters.

---

### `crates/graph/src/calendar_sync.rs`

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

