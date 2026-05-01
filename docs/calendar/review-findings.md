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

---

## Review 2 - TZID + Graph datetime resolution (Opus, bugs lens)

Targets:
- `crates/core/src/caldav/parse.rs::extract_datetime` (and the `parse_icalendar` / `extract_vevent` paths that feed it).
- `crates/graph/src/calendar_sync.rs::parse_graph_datetime` and `resolve_graph_tz`.

- `crates/core/src/caldav/parse.rs:257-265` - Z-suffix + TZID combination: with both, parsing yields `tz_hour: Some(0)`, and `to_date_time_with_tz` correctly uses the embedded UTC offset and ignores the resolved Tz - but if the resolved Tz is non-floating, the result silently reports UTC even though TZID claimed otherwise; if a server intended the value to be local in the named zone (a known Apple/Outlook bug), Ratatoskr quietly stores the UTC interpretation. Repro: `DTSTART;TZID=America/New_York:20240315T100000Z` is stored as 10:00 UTC, not 10:00 NY.

---

## Review 3 - CalDAV consolidation (Opus, arch lens)

Target: post-consolidation CalDAV stack - `crates/calendar/src/caldav/` now
delegating to `rtsk::caldav::client::CalDavClient` and `rtsk::caldav::sync`.
Compared against the deleted ad-hoc client and XML parser via git history.

- `crates/calendar/src/caldav/mod.rs:75` (create) and `:96` (update) - after `put_event`, the code immediately calls `fetch_caldav_event` which issues a fresh GET. The OLD code did the same GET (`fetch_caldav_event_by_href`), so this is parity, BUT `put_event` already returns the new ETag from response headers and that ETag is discarded; then `get_event_ical` re-derives one from a 200 GET. If a server returns the ETag on PUT but is slow/eventually-consistent on GET (common with Exchange front-ends), the GET may race ahead of the write and return stale data or a 404, surfacing as a confusing user-facing error. The OLD path had the same "GET after PUT" race, so this is not a regression - but the new code adds zero defense despite having a perfectly good `Option<String>` ETag in hand.

- `crates/core/src/caldav/client.rs:50-55` - `CalDavClient::new` calls `.unwrap_or_default()` if the builder fails. `reqwest::Client::default()` has a redirect policy of `limited(10)`, so a builder failure silently changes redirect behavior from `limited(5)` to `limited(10)` and disables the 30s timeout. OLD `shared_http_client` was `reqwest::Client::new()` (also unconfigured), so silent fallback isn't worse than before - but the explicit timeout the new code is trying to set vanishes on failure with no log.

- `crates/core/src/caldav/parse.rs:594-600` (`local_name`) strips after the **last** `:`. Tag names with no namespace prefix work fine; namespaced tags like `D:href` work. But xml content with attributes like `xmlns:foo` will never appear as element names, so this is fine in practice. Note however that old `xml.rs` only matched local names whose namespace prefix was bound to one of four known DAV URIs - the new code matches `<href>` from ANY namespace. For mixed-namespace responses (unusual but legal), this could match an unrelated `xyz:href`. Low-risk regression but a tightness loss.

- `crates/calendar/src/caldav/mod.rs:188-191` - `parse_icalendar` returns `Err` if the iCalendar payload has zero VEVENTs (parse.rs:85-87). For a freshly-created event the server occasionally returns the VCALENDAR wrapper with the VEVENT relocated to a VTIMEZONE-only response when the timezone is unrecognized; old `parse_caldav_ical_event` handled empty/partial VEVENTs gracefully by returning a stub DTO. New code surfaces this as a hard error to the user even though the PUT succeeded.

- `crates/core/src/caldav/client.rs:302, 344, 376` - error messages include the full URL but not the request body or response headers. OLD `caldav_request_with_headers:485` returned `format!("CalDAV error: {status} {body}")` truncated to body only. New format `"PUT {url} returned {status}: {text}"` is comparable; no regression in diagnosability, slight improvement.

- `crates/core/src/caldav/client.rs:288-317` (`get_event_ical`) - does not send Depth or Accept text/calendar via PROPFIND/REPORT; relies on plain GET. Some servers (Yahoo, certain Kerio installs) require a calendar-multiget REPORT to return the canonicalized iCal post-PUT and serve the original PUT body via GET. Behavior parity with OLD `fetch_caldav_event_by_href` (also a plain GET), so not a regression - flagged because get_event_ical is "freshly added" per the brief and the call sites at `mod.rs:76, 98` rely on it for post-PUT canonicalization.

- Behavioral capability deletions: `caldav_test_connection_impl`, `caldav_fetch_events_impl`, `caldav_sync_events_impl` (old `mod.rs:64, 28, 41`) are removed with no replacement. No callers remain in `crates/`, so this is dead-code cleanup, not a regression - but if any plumbing (settings UI "Test connection" button, manual range fetch) was wired to these, it is now silently broken. Worth a grep at the iced layer outside the search scope I had.

- `crates/calendar/src/sync.rs:340-356` and `crates/calendar/src/caldav/mod.rs:168-181` duplicate the same client-build logic (`new` + `set_calendar_home_url` or `discover`). Not a regression, just an immediate refactor target.

---

## Outside Reviews

### Reviewer A - combined RRULE / TZ / CalDAV pass

