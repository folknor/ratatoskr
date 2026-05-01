# Calendar Code Review Findings

Captured from review passes on 2026-05-01. Findings are verbatim from each
reviewer (no synthesis, no severity assignments - that's a separate pass).

Outside reviews to be appended below the Opus sections as they come in.

---

## Review 1 - RRULE expansion (Opus, bugs lens)

Target: `crates/db/src/db/queries_extra/calendars.rs::expand_recurrence` and
its helpers (`parse_rrule`, `parse_weekday`, `expand_daily`, `expand_weekly`,
`expand_monthly`, `expand_yearly`, `matches_weekday`, `start_of_week`,
`shift_to_weekday`, `set_day_of_month`, `set_month`, `advance_months`,
`days_in_month`, `parse_until_date`).

- `calendars.rs:1202` - `expand_daily` adds `interval * 86400` to a Unix timestamp; on DST-transition days the resulting instance's local clock-time shifts by ±1h. Repro: `FREQ=DAILY` event at 09:00 spanning 2026-03-08 (US spring-forward) yields a 10:00 instance on 2026-03-09.

- `calendars.rs:1218` - `expand_weekly` empty-BYDAY branch uses `current += interval_secs` (raw seconds), same DST drift across spring-forward/fall-back. Repro: `FREQ=WEEKLY` at 09:00 spanning a DST boundary.

- `calendars.rs:1207-1221` - `expand_weekly` with empty BYDAY never advances by the start's weekday alignment; fine on its own, but combined with the seconds-based step it drifts on DST.

- `calendars.rs:1180-1191` - `parse_weekday` strips digits and signs, so `BYDAY=1MO` and `BYDAY=-1FR` collapse to plain `MO`/`FR`. Under `FREQ=MONTHLY` the rule then falls into the `bymonthday.is_empty()` branch and emits the *start day* every month, not the first/last weekday. Repro: `FREQ=MONTHLY;BYDAY=1MO` starting 2026-03-09 emits the 9th every month, not the first Monday.

- `calendars.rs:1085-1099` - COUNT and UNTIL coexist: `max_instances = count` and `window_end = until` both apply, so the loop stops at whichever is tighter. RFC 5545 forbids both being set; we silently AND them rather than rejecting. Minor, but a correctness deviation.

- `calendars.rs:1316-1326` - `shift_to_weekday` on a DST-transition day: when the candidate date is the spring-forward day and the original time is in the lost hour (02:00-03:00 US), `from_local_datetime(...).single()` returns `None` and the code falls back to `candidate_date` - a midnight UTC-derived timestamp, not the intended local time. Silent wrong answer. Repro: weekly event at 02:30 local that shifts onto 2026-03-08.

- `calendars.rs:1329-1348` - `set_day_of_month`: same `.single()` failure mode; on a DST day the function returns `None` and the instance is *skipped entirely*, not retried with a valid hour. Repro: `BYMONTHDAY=8` event at 02:30 in March 2026 (US).

- `calendars.rs:1336` - negative-day arithmetic: `dim + day + 1`. For `day=-1` in a 31-day month: `31 + (-1) + 1 = 31` ✓. For `day=-31` in February (28 days): `28 + (-31) + 1 = -2` → fails the `< 1` guard and returns `None`, silently dropping the instance instead of falling back per RFC 5545's "skip" semantics. Behavior is consistent with skip, but pre-filter at parse time accepts mag ≤ 31, so users get silent misses in short months.

- `calendars.rs:1072-1074` - `window_end = start + 2*365*86400` ignores leap years; a 2-year window starting 2024-01-01 ends 2025-12-31 (not 2026-01-01). The check at line 1086 is `start > window_end` (strict `>`) so an instance landing exactly on `window_end` is included - minor off-by-one mostly invisible in practice.

- `calendars.rs:1364-1384` - `advance_months` clamps `new_day = day.min(days_in_month(...))` and never recovers: a Jan-31 monthly event becomes Feb-28, then Mar-28 (not Mar-31), Apr-28, etc. RFC 5545 says skip months where the day doesn't exist; we instead permanently degrade. Repro: `FREQ=MONTHLY` starting 2026-01-31 emits Feb 28, Mar 28, Apr 28, ... rather than Jan 31, Mar 31, May 31, ...

- `calendars.rs:1364-1373` - `advance_months` modulo math: `total_months = month-1 + months`. For negative `months` (not currently used, but the signature accepts `i64`), `total_months % 12` is negative in Rust, so `new_month` and `new_year` go wrong. Unconfirmed in current call sites (only positive intervals are passed) but a latent bug.

- `calendars.rs:1302-1310` - `start_of_week` hardcodes Monday (`num_days_from_monday`); RFC 5545 `WKST` is not parsed. For weekly rules where the user expects Sunday-anchored weeks (US default `WKST=SU`), the first emitted week can be off by one when start-of-week lands on Sun and BYDAY includes earlier days. Note rather than bug.

- `calendars.rs:1403-1411` - `parse_until_date` builds `date.and_hms_opt(23,59,59).and_utc().timestamp()` regardless of whether the UNTIL value is `YYYYMMDD` (date-only, floating) or `YYYYMMDDTHHMMSSZ` (UTC datetime). The `T...Z` time portion is discarded and replaced with 23:59:59 UTC. Repro: `UNTIL=20280101T000000Z` is treated as 2028-01-01 23:59:59 UTC, almost a full day late. The existing `yearly_with_until_clamps_window` test happens to pass because the slack matches.

- `calendars.rs:1085-1099` - `instances.is_empty()` fallback: if every candidate is filtered out (e.g. `COUNT=5` with `UNTIL` already past), the function returns the original event as a single instance. That's not what RFC 5545 says - an event whose RRULE produces no instances should produce zero instances. Behavioural surprise, not a crash.

---

## Review 2 - TZID + Graph datetime resolution (Opus, bugs lens)

Targets:
- `crates/core/src/caldav/parse.rs::extract_datetime` (and the `parse_icalendar` / `extract_vevent` paths that feed it).
- `crates/graph/src/calendar_sync.rs::parse_graph_datetime` and `resolve_graph_tz`.

- `crates/core/src/caldav/parse.rs:260-268` - Spring-forward DST: when TZID resolves to a non-floating IANA zone but the wall-clock falls in the spring-forward gap, `to_date_time_with_tz` returns `None` (chrono's `LocalResult::None` -> `.single() = None`); the code then falls through to `dt.to_timestamp()` which interprets the wall-clock as UTC, producing a timestamp wrong by the zone's full offset. Repro: `DTSTART;TZID=America/New_York:20240310T023000` returns naive-as-UTC instead of the correct gap resolution.

- `crates/core/src/caldav/parse.rs:260-268` - Fall-back DST ambiguity: for ambiguous local times, `chrono::Tz::from_local_datetime` returns `Ambiguous(early, late)`, `.single()` returns None, and `extract_datetime` again falls through to `dt.to_timestamp()` (treats as UTC). Repro: `DTSTART;TZID=America/New_York:20241103T013000` produces `01:30 UTC` instead of either of the two valid EDT/EST instants (1 hour wrong).

- `crates/core/src/caldav/parse.rs:257-268` - Floating-fallthrough silently treats wall-clock as UTC: when TZID is set but resolves to `Tz::Floating` (unknown zone, no VTIMEZONE) the `if !tz.is_floating()` guard sends control to `dt.to_timestamp()`; with no `tz_hour` set on the PartialDateTime, calcard/mail-parser interpret the naive wall-clock as UTC. Repro: `DTSTART;TZID=Bogus/Zone:20240315T100000` returns `2024-03-15T10:00 UTC`, off by the user's true local offset.

- `crates/core/src/caldav/parse.rs:242-253` - All-day DATE values are forced to midnight UTC: round-tripping into a non-UTC user view will display the wrong calendar date for users west of UTC and any time-window query against an all-day event misaligns with the user's local day. Repro: an all-day "Holiday" on `20240101` viewed in `America/Los_Angeles` shows as 2023-12-31.

- `crates/graph/src/calendar_sync.rs:498-509` - Graph all-day midnight-as-UTC: `parse_graph_datetime` discards `time_zone` for all-day events and stores midnight UTC, so any user not on UTC gets a date shift on display/queries. Repro: a Graph all-day event with `dateTime=2024-01-15T00:00:00.0000000`, `timeZone=America/Los_Angeles`, `is_all_day=true` becomes `2024-01-15T00:00 UTC` (= 2024-01-14 16:00 LA local), shifting the displayed date back one day for any LA user.

- `crates/graph/src/calendar_sync.rs:526-533` - Graph fall-back ambiguity returns wrong UTC: `tz.from_local_datetime(&naive).single()` is `None` for ambiguous local times, so the `unwrap_or_else` fallback interprets the wall-clock as UTC. Repro: `dateTime=2024-11-03T01:30:00.0000000`, `timeZone=Pacific Standard Time` returns `01:30 UTC` instead of one of the two valid LA instants.

- `crates/graph/src/calendar_sync.rs:526-533` - Graph spring-forward gap returns wrong UTC: same `.single()` -> None pattern for non-existent local times falls back to naive-as-UTC. Repro: `dateTime=2024-03-10T02:30:00.0000000`, `timeZone=Eastern Standard Time` returns `02:30 UTC`, ~5 hours off.

- `crates/graph/src/calendar_sync.rs:538-546` - "Pacific Daylight Time" / "Eastern Daylight Time" not in calcard's alias map: `Tz::from_str` only matches the "Standard Time" forms (parse.rs:496), so `resolve_graph_tz` returns None for daylight names and the code falls back to UTC. Repro: a Graph event emitted with `time_zone="Pacific Daylight Time"` (in any month, including summer) parses as if UTC-naive - 7-8 hours off.

- `crates/core/src/caldav/parse.rs:257-265` - Z-suffix + TZID combination: with both, parsing yields `tz_hour: Some(0)`, and `to_date_time_with_tz` correctly uses the embedded UTC offset and ignores the resolved Tz - but if the resolved Tz is non-floating, the result silently reports UTC even though TZID claimed otherwise; if a server intended the value to be local in the named zone (a known Apple/Outlook bug), Ratatoskr quietly stores the UTC interpretation. Repro: `DTSTART;TZID=America/New_York:20240315T100000Z` is stored as 10:00 UTC, not 10:00 NY.

- `crates/graph/src/calendar_sync.rs:518-522` - Truncation by `find('.')` is fine when the dot belongs to fractional seconds, but does not validate position; an unexpected pre-`T` dot (e.g., a malformed input like `2024.01-15T...`) would silently truncate too early and produce a misleading parse error rather than a clear validation failure. Repro: feed any `dateTime` with a stray `.` before the time portion.

---

## Review 3 - CalDAV consolidation (Opus, arch lens)

Target: post-consolidation CalDAV stack - `crates/calendar/src/caldav/` now
delegating to `rtsk::caldav::client::CalDavClient` and `rtsk::caldav::sync`.
Compared against the deleted ad-hoc client and XML parser via git history.

- `crates/calendar/src/caldav/mod.rs:75` (create) and `:96` (update) - after `put_event`, the code immediately calls `fetch_caldav_event` which issues a fresh GET. The OLD code did the same GET (`fetch_caldav_event_by_href`), so this is parity, BUT `put_event` already returns the new ETag from response headers and that ETag is discarded; then `get_event_ical` re-derives one from a 200 GET. If a server returns the ETag on PUT but is slow/eventually-consistent on GET (common with Exchange front-ends), the GET may race ahead of the write and return stale data or a 404, surfacing as a confusing user-facing error. The OLD path had the same "GET after PUT" race, so this is not a regression - but the new code adds zero defense despite having a perfectly good `Option<String>` ETag in hand.

- `crates/core/src/caldav/client.rs:50-55` - `CalDavClient::new` calls `.unwrap_or_default()` if the builder fails. `reqwest::Client::default()` has a redirect policy of `limited(10)`, so a builder failure silently changes redirect behavior from `limited(5)` to `limited(10)` and disables the 30s timeout. OLD `shared_http_client` was `reqwest::Client::new()` (also unconfigured), so silent fallback isn't worse than before - but the explicit timeout the new code is trying to set vanishes on failure with no log.

- `crates/core/src/caldav/client.rs:52` - redirect policy is `limited(5)`. OLD `shared_http_client` (`reqwest::Client::new()`) used reqwest's default `limited(10)`. Servers with deep `.well-known/caldav` redirect chains (some hosted Zimbra and old Exchange front-ends chain 6-7 hops through SSO before landing on the DAV root) will now fail discovery where they previously succeeded.

- `crates/calendar/src/caldav/mod.rs:11-16` - `CaldavAccountConfig` no longer carries `principal_url`. The DB column `caldav_principal_url` still exists (see `crates/calendar/src/sync.rs:23` which selects from `accounts`), but the load function no longer reads it (`mod.rs:123` selects only 5 columns vs. 6 before). For accounts where principal was previously cached but home was not, `discover()` now redoes the full principal walk (PROPFIND + PROPFIND) on every sync rather than skipping straight to the home-set lookup. Sync still works, but each cold-start hits two extra round trips.

- `crates/core/src/caldav/client.rs:88-107` - `discover()` first tries `/.well-known/caldav` and only falls back to `self.base_url` on a `propfind_raw` error. Many enterprise Exchange fronts return `200 OK` with a normal HTML 404 page (or a redirect to a login portal) for `/.well-known/caldav`, which means `propfind_raw` returns Ok, the `extract_href_property` finds no principal, and the code sets `dav_root = well_known_url` and proceeds to PROPFIND that URL again - wasting a round trip and leaving `principal_url == None` until the second PROPFIND on the broken URL fails. The OLD `resolve_caldav_home_url` issued PROPFIND directly on `config.server_url`, never probing `.well-known`.

- `crates/core/src/caldav/parse.rs:594-600` (`local_name`) strips after the **last** `:`. Tag names with no namespace prefix work fine; namespaced tags like `D:href` work. But xml content with attributes like `xmlns:foo` will never appear as element names, so this is fine in practice. Note however that old `xml.rs` only matched local names whose namespace prefix was bound to one of four known DAV URIs - the new code matches `<href>` from ANY namespace. For mixed-namespace responses (unusual but legal), this could match an unrelated `xyz:href`. Low-risk regression but a tightness loss.

- `crates/calendar/src/caldav/mod.rs:188-191` - `parse_icalendar` returns `Err` if the iCalendar payload has zero VEVENTs (parse.rs:85-87). For a freshly-created event the server occasionally returns the VCALENDAR wrapper with the VEVENT relocated to a VTIMEZONE-only response when the timezone is unrecognized; old `parse_caldav_ical_event` handled empty/partial VEVENTs gracefully by returning a stub DTO. New code surfaces this as a hard error to the user even though the PUT succeeded.

- `crates/core/src/caldav/client.rs:302, 344, 376` - error messages include the full URL but not the request body or response headers. OLD `caldav_request_with_headers:485` returned `format!("CalDAV error: {status} {body}")` truncated to body only. New format `"PUT {url} returned {status}: {text}"` is comparable; no regression in diagnosability, slight improvement.

- `crates/core/src/caldav/client.rs:288-317` (`get_event_ical`) - does not send Depth or Accept text/calendar via PROPFIND/REPORT; relies on plain GET. Some servers (Yahoo, certain Kerio installs) require a calendar-multiget REPORT to return the canonicalized iCal post-PUT and serve the original PUT body via GET. Behavior parity with OLD `fetch_caldav_event_by_href` (also a plain GET), so not a regression - flagged because get_event_ical is "freshly added" per the brief and the call sites at `mod.rs:76, 98` rely on it for post-PUT canonicalization.

- Behavioral capability deletions: `caldav_test_connection_impl`, `caldav_fetch_events_impl`, `caldav_sync_events_impl` (old `mod.rs:64, 28, 41`) are removed with no replacement. No callers remain in `crates/`, so this is dead-code cleanup, not a regression - but if any plumbing (settings UI "Test connection" button, manual range fetch) was wired to these, it is now silently broken. Worth a grep at the iced layer outside the search scope I had.

- `crates/calendar/src/sync.rs:340-356` and `crates/calendar/src/caldav/mod.rs:168-181` duplicate the same client-build logic (`new` + `set_calendar_home_url` or `discover`). Not a regression, just an immediate refactor target.

---

## Outside Reviews

### Reviewer A - combined RRULE / TZ / CalDAV pass

3. `crates/db/src/db/queries_extra/calendars.rs:1202` and `crates/db/src/db/queries_extra/calendars.rs:1218`: daily/plain-weekly recurrence advances by fixed Unix seconds, so wall-clock time shifts across DST. A 09:00 event before spring-forward becomes 10:00 local after the transition.

4. `crates/db/src/db/queries_extra/calendars.rs:1176`: ordinal BYDAY is stripped to a plain weekday, and `crates/db/src/db/queries_extra/calendars.rs:1250` / `crates/db/src/db/queries_extra/calendars.rs:1274` expansion ignores BYDAY anyway. `FREQ=MONTHLY;BYDAY=1MO` emits the DTSTART day-of-month, not the first Monday.

5. `crates/db/src/db/queries_extra/calendars.rs:1403`: `UNTIL=YYYYMMDDTHHMMSSZ` ignores the time and becomes 23:59:59 UTC for that date. Occurrences after the actual UNTIL time are incorrectly included.

6. `crates/core/src/caldav/parse.rs:257` and `crates/graph/src/calendar_sync.rs:526`: DST gaps/ambiguous local times with a resolved timezone fall back to interpreting the naive local time as UTC. The same UTC fallback happens when TZID resolves to `Tz::Floating`.

9. `crates/calendar/src/caldav/mod.rs:123`: `caldav_principal_url` is no longer loaded or used. Accounts that had a stored principal URL but no home URL now must rediscover from `caldav_url`, which regresses servers where principal discovery from the base URL fails.

