# Calendar Code Review Findings

Captured from review passes on 2026-05-01. Findings are verbatim from each
reviewer (no synthesis, no severity assignments - that's a separate pass).

All findings from the first round have been addressed in code (fixed,
guarded, or commented in place to discourage future re-flagging). The
sections below are kept as scaffolding for future review passes - paste
new findings under the corresponding section as they come in.

---

## Review 1 - RRULE expansion (Opus, bugs lens)

Target: `crates/db/src/db/queries_extra/calendars.rs::expand_recurrence` and
its helpers (`parse_rrule`, `parse_byday`, `expand_daily`, `expand_weekly`,
`expand_monthly`, `expand_yearly`, `matches_weekday`, `start_of_week`,
`shift_to_weekday`, `advance_months`, `days_in_month`, `parse_until_date`).

_(no open findings)_

---

## Review 2 - TZID + Graph datetime resolution (Opus, bugs lens)

Targets:
- `crates/core/src/caldav/parse.rs::extract_datetime` (and the `parse_icalendar` / `extract_vevent` paths that feed it).
- `crates/graph/src/calendar_sync.rs::parse_graph_datetime` and `resolve_graph_tz`.

_(no open findings)_

---

## Review 3 - CalDAV consolidation (Opus, arch lens)

Target: post-consolidation CalDAV stack - `crates/calendar/src/caldav/`
delegating to `rtsk::caldav::client::CalDavClient` and `rtsk::caldav::sync`.

_(no open findings)_

---

## Outside Reviews

### Reviewer A - combined RRULE / TZ / CalDAV pass

_(no open findings)_
