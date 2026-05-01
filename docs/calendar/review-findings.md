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

---

# Round 2

Captured 2026-05-01. Three reviewer lenses, each scoped by behavioral surface
rather than file boundary:

- Reviewer 1: adversarial timezone & date arithmetic (weird zones, edge dates)
- Reviewer 2: RRULE expansion as a contract (RFC 5545 semantic correctness)
- Reviewer 3: real-world CalDAV server compatibility

Each lens has both an internal Opus agent run and an outside commissioned
reviewer. Findings are pasted verbatim - no synthesis, no severity normalization
across reviewers. Duplicates between internal/outside on the same lens are the
highest-confidence signals.

---

## Round 2 / Reviewer 1 - Adversarial timezone (Opus internal)

Targets:
- `crates/db/src/db/time.rs` (`resolve_local_to_timestamp`)
- `crates/db/src/db/queries_extra/calendars.rs` (`add_days_local`, `with_year_month_day`, `advance_months`, `days_in_month`, expander helpers)
- `crates/core/src/caldav/parse.rs` (`extract_datetime`, `partial_to_naive`, `build_local_midnight`, `parse_icalendar`)
- `crates/graph/src/calendar_sync.rs` (`parse_graph_datetime`, `resolve_graph_tz`)

### Bug 1 - Spring-forward walker can land BEFORE the input wall-clock (Lord Howe)
**File:** `crates/db/src/db/time.rs:42-58`

The `LocalResult::None` branch tries `+60 min`, then `+30 min`, then walks `+1..=+120` minute by minute. For Australia/Lord_Howe (30-min DST), the gap is 02:00 -> 02:30 on the spring-forward day. If the user's input is `02:15`:

- `+60` -> `03:15`, valid (LHST, +11). Returns `03:15` LHST.

But for the user's *intended* meaning ("starts at 02:15, give me the first valid moment after the gap"), the correct result is `02:30 LHDT` (+10:30), which is `15:00 UTC`. Returning `03:15 LHDT` instead is `15:45 UTC` - the event is now scheduled 45 minutes *later than the gap exit*. This isn't catastrophic but the comment claims it preserves the "user's intended wall-clock minute"; for Lord Howe that intent is meaningless because no `02:15` exists at all. The minute-by-minute fallback in the second loop never runs for Lord Howe because `+60` always succeeds first.

A subtler problem: if the user enters `02:00` exactly (the *boundary*, which IS the gap start in some zones), `+60` yields `03:00`. For Lord Howe this is `03:00 LHDT` - but the actual gap-exit instant was `02:30 LHDT`, 30 min earlier. The event slides forward by 30 minutes versus what most calendar apps do.

### Bug 2 - Tied ambiguous result picks the wrong instant on minute-by-minute fallback
**File:** `crates/db/src/db/time.rs:50-56`

In the 1..=120 minute walk, `LocalResult::Ambiguous` is treated as a valid resolution and the **earlier** instant returned. But this walker only fires for spring-forward gaps - by definition you can't shift forward through a gap and hit ambiguous resolution unless TZ data has two transitions in close sequence (Antarctica/Troll, Antarctica/Casey territory). For zones with double transitions, returning the earlier instant of an ambiguous result that's *itself* a fall-back from the gap-shift is silently wrong: the user wanted the post-gap instant, and you've handed them a pre-fall-back instant from the *next* transition. Real risk for Antarctica/Casey, which had transitions in 2009-10 separated by hours.

### Bug 3 - `is_floating()` check after `resolver.resolve_or_default()` defeats the resolver default
**File:** `crates/core/src/caldav/parse.rs:281-288`

```rust
let tz = resolver.resolve_or_default(Some(tz_id));
if !tz.is_floating() {
    return (common::time::resolve_local_to_timestamp(naive, &tz), false);
}
```

`build_tz_resolver` (calcard timezone.rs:58-63) sets `default: Tz::Floating`. So an unrecognized TZID - say `TZID="Eastern Std Tyme"` (typo), or `TZID="GMT Standard Time"` *with surrounding whitespace* (next bug) - falls through to `Tz::Floating`. The `if !tz.is_floating()` guard then drops to clause 3 (line 301-304), which interprets the wall clock in `chrono::Local`. This is **wrong**: the user explicitly tagged the time with a zone that the parser couldn't interpret. Silently re-anchoring to the user's *system* zone can shift events by many hours for an Australian user reading a US-zoned event with a typo'd TZID.

The fix would be: if the explicit TZID exists but doesn't resolve, return `None` (or fall back through `dt.to_timestamp()` which treats the wall clock as UTC, the legacy behavior). The graph code (`calendar_sync.rs:540`) handles this correctly - falls back to UTC interpretation, not local.

### Bug 4 - TZID with leading/trailing whitespace silently slides events to user's local zone
**File:** `crates/core/src/caldav/parse.rs:269-289`

`entry.tz_id()` (calcard `timezone.rs:110-113`) returns the raw text without trimming. Both lookups inside `resolver.resolve_or_default` exact-string-match: `self.tzs.get(tz_name)` (HashMap lookup, so `" America/New_York"` != `"America/New_York"`), then `Tz::from_str(tz_name)`, which **does** call `s.trim()` *but only inside the proprietary-name fallback branch starting at line 259*. The first attempt at `chrono_tz::Tz::from_str(s)` on line 254 is *not* trimmed.

So `TZID="America/New_York "` (single trailing space) -> `chrono_tz::Tz::from_str` rejects it -> enters the prefix-stripping branch, which only handles `(` and `/` prefixes -> falls through to the proprietary alias `hashify::map!` which is exact-byte-match on `s.as_bytes()` (the *trimmed* `s`, line 259). So the trailing space *is* stripped before alias lookup, but alias lookup also misses for `America/New_York`. Net result: `TZID="America/New_York "` returns `Err(())` from `Tz::from_str`, the resolver falls back to `Tz::Floating`, and Bug 3 kicks in: the event silently lands in the user's system zone.

Worst case: a Norway-based user (Europe/Oslo, UTC+1/+2) reads a calendar event from a US server with a stray trailing whitespace TZID for `America/Los_Angeles`. The wall-clock `09:00` is silently interpreted as `09:00` Oslo time, which is `00:00` LA time - the event displays 9 hours early.

### Bug 5 - Pacific/Apia date-line skip (Dec 30 2011) silently drops to "Dec 31"
**File:** `crates/db/src/db/time.rs:42-58`, `crates/db/src/db/queries_extra/calendars.rs:1604-1611` (`add_days_local`)

Apia skipped Dec 30 2011 entirely (the day did not exist). If a recurring event repeats daily and `add_days_local` lands on `Dec 30 2011 09:00 Apia`:

- `chrono::NaiveDate::from_ymd_opt(2011, 12, 30)` succeeds (NaiveDate is calendar-only, no zone awareness).
- `resolve_local_to_timestamp` calls `tz.from_local_datetime(&naive)`. For Pacific/Apia, this entire day is `LocalResult::None` (24-hour skip).
- `time.rs:42-58` walker: `+60 min` from 09:00 lands on Dec 30 10:00 -> still `None`. `+30 min` -> still `None`. Then minute-walk `1..=120` -> all `None` (gap is 24 hours long).
- Falls through to `None` returned. `add_days_local` then does `unwrap_or(current + days * 86400)` which adds raw seconds, so the new timestamp is `(prev) + 86400 s` - i.e., **the same wall-clock-second that would have existed** if Dec 30 had existed. That seconds-value, decoded in Apia's *new* (post-shift) zone (UTC+14), happens to be **Dec 31 09:00 Apia**, not Dec 30 09:00 (which never existed). So you get the right answer by luck.

**But** the same fallback breaks for `expand_weekly` (line 1346, 1383), which calls `add_days_local(week_anchor, interval_days)`. The `unwrap_or(week_anchor + interval_days * 86400)` raw-seconds fallback assumes a stable UTC offset across the entire interval - wrong for Apia's `+24:00:00` jump. A weekly event whose anchor week straddles Dec 30 2011 will silently emit instances 24 hours offset from where they should land.

### Bug 6 - `start_of_week` clamping to original timestamp on `add_days_local` failure quietly emits wrong-week instances
**File:** `crates/db/src/db/queries_extra/calendars.rs:1613-1626`

```rust
add_days_local(timestamp, -days_back).unwrap_or(timestamp)
```

If `add_days_local` returns `None` (e.g., walking back across Dec 30 2011 in Apia, or across the year-1 boundary in some pre-historic zone), this returns the *original timestamp* as the "week start." The downstream weekly expansion then uses this as the anchor for `shift_to_weekday` calls and emits instances one entire week off - silently, with no caller indication.

### Bug 7 - `parse_until_date` ignores the trailing `Z`, accepts non-UTC UNTIL as UTC
**File:** `crates/db/src/db/queries_extra/calendars.rs:1741-1748`

```rust
if val.len() >= 15 && val.as_bytes().get(8) == Some(&b'T') {
    let time_part = val.get(9..15)?;
    ...
    return Some(dt.and_utc().timestamp());  // unconditionally UTC
}
```

RFC 5545 Section 3.3.10 says when DTSTART is local-with-TZID, UNTIL **MUST** be in UTC (`Z`-suffixed). When DTSTART is floating, UNTIL must also be floating. The code simply assumes UTC. So `UNTIL=20260315T100000` (no `Z`) on a floating DTSTART is parsed as UTC - wrong for any timezone east or west of UTC. Worst case: a recurring event with DTSTART in `Pacific/Kiritimati` (UTC+14) and a floating UNTIL of `20260315T235900` is read as 23:59 UTC = 13:59 Kiritimati, terminating the rule ~10 hours early on the final day.

The 15-char check `val.len() >= 15` matches both `20260315T100000` and `20260315T100000Z` because `Z` would be at index 15 and there's no validation of it.

### Bug 8 - `parse_until_date` fractional seconds slide UNTIL backward by ~14 hours
**File:** `crates/db/src/db/queries_extra/calendars.rs:1732-1753`

The function reads exactly 8 bytes of date, optionally a `T`, then exactly 6 bytes of time. There's no validation that index 15 is `Z` or end-of-string. An input like `20260315T100000.5` parses fine (the `.5` is silently ignored) and is treated as UTC. More dangerous: `20260315` (8 chars only) is treated as DATE form and stored as `23:59:59` of that date. Combined with floating-DTSTART (Bug 7), this is fine. But `20260315T100000+0100` (the rare-but-valid local-time-with-offset) parses as `20260315T100000` UTC, off by exactly 1 hour.

### Bug 9 - `is_all_day` event spanning DST ends up storing wrong DURATION
**File:** `crates/core/src/caldav/parse.rs:236-261`, `crates/graph/src/calendar_sync.rs:508-517`

`build_local_midnight` (CalDAV) and the all-day branch in `parse_graph_datetime` both call `resolve_local_to_timestamp(naive_midnight, &chrono::Local)`. For DTSTART/DTEND on an all-day event spanning DST (e.g., a multi-day Holiday event in `America/New_York` from Mar 9 to Mar 11 2024 - Mar 10 is the spring forward), the start is midnight of Mar 9 (EST, UTC-5) and the end is midnight of Mar 11 (EDT, UTC-4). The duration in seconds is **23 hours**, not 48. Downstream consumers that subtract `end - start` and divide by 86400 to get "number of days" silently get 1 day off.

### Bug 10 - `parse_graph_datetime` decimal `.` before `T` falls through to a different parse error class
**File:** `crates/graph/src/calendar_sync.rs:530-536`

The defensive comment says "if `.` appears before `T` we'd rather surface a parse error than truncate." The code does:
```rust
let clean = match (dt.date_time.find('.'), t_pos) {
    (Some(dot), Some(t)) if dot > t => &dt.date_time[..dot],
    _ => dt.date_time.as_str(),
};
```

But for input `2024.01-15T10:00:00` (dot < T), `clean = "2024.01-15T10:00:00"` (full string). `parse_from_str` then fails with `%Y` not parsing `"2024.01"` - error returned, fine. **However**, `2024-01-15T10:00:00.5` on Apia would produce `clean = "2024-01-15T10:00:00"`, parsed successfully - great. But for `2024-01-15T10:00.5:00` (a malformed Graph payload, dot inside the time portion but not at the fractional-second slot), `clean = "2024-01-15T10:00"` - that's only HH:MM, no `:SS`. `parse_from_str` will fail with `%S` mismatch. So it's safe by accident, but the truncation logic is brittle: if Graph ever emits sub-minute precision (e.g., `T10:00:00.5+02:00`), the offset is silently dropped.

For input `2024-01-15T10:00:00+02.5:00` (decimal in offset), `clean = "2024-01-15T10:00:00+02"` is also a parse failure - fine.

### Bug 11 - Pre-1970 / year-9999 timestamp handling silently inconsistent
**File:** Multiple

- `chrono::Local.timestamp_opt(timestamp, 0)` (used in `add_days_local`, `with_year_month_day`, `with_day_of_month`, `start_of_week`, `expand_monthly`, `expand_yearly`): negative timestamps work; year 9999 works; far-past or far-future may saturate.
- `chrono::NaiveDate::from_ymd_opt(year, month, day)`: accepts years from chrono's `MIN_YEAR` (~`-262144`) to `MAX_YEAR` (~`262143`). So `year = -1` is a valid input - `with_year_month_day(_, -1, 1, 1)` happily resolves.
- In `expand_yearly` (line 1525), the loop never sanity-checks `year`: a YEARLY rule starting in year 100 with `INTERVAL=10000` would advance to year 102000 in two steps, blow past chrono's max year, and `dt.with_year(...)` returns `None` -> `two_year_window_end` falls back to `start + 730 * 86400`. That's "fine" (loop terminates), but the comment-claimed 2-year window doesn't apply.

More concerning: `parse_until_date` (line 1732-1753) accepts `year = 0` and `year = 9999`. `chrono::NaiveDate::from_ymd_opt(0, ...)` succeeds (chrono has year 0). `dt.and_utc().timestamp()` for year 0 is `-62167219200` (negative, ~62 billion seconds before epoch). Comparisons against `start` (a normal positive timestamp) work fine - UNTIL is in the past, recurrence emits zero instances. Surprising-but-arguably-intentional.

### Bug 12 - TZID=UTC explicit + Z-suffix combo: which path?
**File:** `crates/core/src/caldav/parse.rs:269-296`

For `TZID=UTC:20240315T100000Z`:
1. Line 269: `entry.tz_id()` returns `Some("UTC")`.
2. Line 276: `dt.tz_hour.is_some()` - true (Z-suffix sets tz_hour). The debug log fires, then the code falls **out of the if-let** without doing anything because both branches of the inner if/else (line 276 and 280) only act when one or the other is true. Wait - re-reading: the `if dt.tz_hour.is_some()` is inside the `if let Some(tz_id)` block, takes the log branch, then falls through past the if-let entirely.
3. Line 294: `dt.tz_hour.is_some()` is true -> returns `dt.to_timestamp()`. Correct (UTC instant).

But for `TZID="UTC ":20240315T100000` (trailing whitespace, no `Z`-suffix):
1. `entry.tz_id()` returns `Some("UTC ")`.
2. Line 276: `tz_hour` is None (no Z suffix).
3. Line 281: `resolver.resolve_or_default(Some("UTC "))`. `chrono_tz::Tz::from_str("UTC ")` -> fails (no trim before first attempt). Falls into the alias branch where `s = s.trim()` -> `s = "UTC"`. The `hashify::map!` table - does it have `"UTC"`? Let me re-check... the file from before stops at row 459 but I'd need to verify. Looking at the listed entries, "UTC" isn't an obvious alias key (the table targets Microsoft proprietary names). If `UTC` isn't an alias key, `Tz::from_str("UTC ")` returns `Err(())`. Resolver falls back to `Tz::Floating`. Bug 3 kicks in - `chrono::Local` reinterpretation.

Confirming: chrono_tz **does** accept `"UTC"` exactly (it's a valid IANA name). `chrono_tz::Tz::from_str("UTC")` returns `Ok(UTC)`. But `chrono_tz::Tz::from_str("UTC ")` (with trailing space) - chrono_tz's IANA lookup is byte-exact. So this fails. After the trim fallback, alias-table lookup on `"UTC"` - looking at the snippet, I see `"GMT Standard Time"`, `"Greenwich Standard Time"`, but **no bare "UTC"** alias entry. So `TZID="UTC "` fails to resolve, returns `Tz::Floating`, and silently re-anchors to user's local zone. Same as Bug 4 with a different root cause.

### Bug 13 - `parse_byday` accepts `+0XX` (zero ordinal with explicit sign)
**File:** `crates/db/src/db/queries_extra/calendars.rs:1266-1303`

The check `if n == 0 { return None }` rejects `0MO` but the parser will happily produce `n = 0` from `+00MO` or `-0MO` (parsed as `-0 * 1 = 0`, then the n==0 check rejects). OK, so that's actually defended. But `+1MO` *is* legitimately `1MO`. Borderline OK.

A subtler issue: ordinal magnitudes > 53 aren't rejected. `BYDAY=99MO` parses as `Some(99)` with no upper-bound check. `nth_weekday_in_month` returns `None` (the 99th Monday doesn't exist), so the day filter rejects all candidates and `expand_monthly`'s step-bound takes 12000 iterations to terminate - DoS-resistant by accident.

### Bug 14 - `Tz::Fixed` (fixed-offset zones from VTIMEZONE) goes through `resolve_local_to_timestamp` correctly, but `is_utc()` doesn't trigger
**File:** N/A (observation, not a bug per se)

For a VTIMEZONE that resolves to `Tz::Fixed(FixedOffset::east(0))`, `is_floating()` is false -> goes through `resolve_local_to_timestamp`. `Tz::Fixed` never produces `LocalResult::None` or `Ambiguous`, so the resolver passes straight through. Correct.

But note: `resolve_local_to_timestamp<Tz: TimeZone>` is generic - when called via `resolver.resolve_or_default()`, the `Tz` type is `calcard::common::timezone::Tz`. That type does implement `chrono::TimeZone` (verified by usage). Each variant (Fixed, Tz, Floating) routes to its own `from_local_datetime`. No bug, just confirming.

### Summary table

| # | File:line | Severity | Trigger |
|---|-----------|----------|---------|
| 1 | `time.rs:42-58` | Low | Lord Howe Island spring-forward, `02:15` input |
| 2 | `time.rs:50-56` | Low | Antarctica/Casey/Troll, multi-transition gap |
| 3 | `parse.rs:281-288` | **High** | Unresolved TZID silently re-anchors to user's local zone |
| 4 | `parse.rs:269-289` | **High** | TZID with surrounding whitespace |
| 5 | `time.rs:42-58` + `calendars.rs:1346/1383` | Medium | Pacific/Apia Dec 30 2011 in weekly recurrence anchor |
| 6 | `calendars.rs:1625` | Low | `add_days_local` failure swallowed |
| 7 | `calendars.rs:1741-1748` | **High** | `UNTIL=YYYYMMDDTHHMMSS` (no `Z`) silently treated as UTC |
| 8 | `calendars.rs:1732-1753` | Low | UNTIL fractional seconds / non-UTC offsets |
| 9 | `parse.rs:255-261`, `calendar_sync.rs:508-517` | **High** | All-day event spanning DST has 23-hour day |
| 10 | `calendar_sync.rs:530-536` | Low | Sub-minute precision in Graph payload |
| 11 | various | Low | Year 0 / year 9999 |
| 12 | `parse.rs:269-296` | High (= Bug 4) | `TZID="UTC "` |
| 13 | `calendars.rs:1266-1303` | Trivial | BYDAY ordinal > 53 |

The four bugs that look most likely to bite real users:
- **Bug 3 / Bug 12**: Any TZID that fails to resolve becomes a silent local-zone reinterpretation rather than UTC fallback, contradicting the comment at line 220-228 of `parse.rs` and inconsistent with the Graph behavior at `calendar_sync.rs:540`.
- **Bug 4**: TZID whitespace bypasses both the HashMap lookup and the first chrono_tz attempt, then routes through Bug 3.
- **Bug 7**: Floating-DTSTART RRULEs with floating UNTIL get the UNTIL silently treated as UTC, terminating recurrences hours early/late depending on user zone.
- **Bug 9**: Multi-day all-day events spanning DST silently store a duration that's off by one hour, breaking any "how many days" calculation.

---

## Round 2 / Reviewer 1 - Adversarial timezone (Outside)

### Findings

1. `crates/db/src/db/time.rs:42` mishandles non-60-minute spring gaps.

   For Australia/Lord_Howe, the spring DST gap is 30 minutes. A `DTSTART;TZID=Australia/Lord_Howe:20241006T021500` is nonexistent and should shift by the 30-minute gap to 02:45. The resolver tries +1h first at `crates/db/src/db/time.rs:43`, sees 03:15 is valid, and returns that, 30 minutes late. The same bug hits Graph via `crates/graph/src/calendar_sync.rs:538` and CalDAV via `crates/core/src/caldav/parse.rs:283`.

2. `crates/db/src/db/time.rs:50` does not preserve the intended minute for gaps larger than one hour, and cannot handle skipped days.

   For Antarctica/Troll-style 2-hour jumps, an inner gap time like 01:30 walks minute-by-minute and returns the first valid instant after the gap, e.g. 03:00, rather than preserving the wall-clock minute as 03:30. For Pacific/Apia on skipped 2011-12-30, every probe through +120m is still nonexistent, so the helper returns None at `crates/db/src/db/time.rs:58`. Graph then silently falls back to treating the nonexistent local wall time as UTC at `crates/graph/src/calendar_sync.rs:538`, and CalDAV produces None from `crates/core/src/caldav/parse.rs:283`, which later defaults missing times to 0 in sync code.

3. `crates/db/src/db/queries_extra/calendars.rs:1108` expands recurrences in the machine's local timezone, not the event timezone.

   `CalendarViewEvent` carries timezone at `crates/db/src/db/queries_extra/calendars.rs:982`, but expansion passes only raw timestamps, and helpers all use `chrono::Local`: `crates/db/src/db/queries_extra/calendars.rs:1518`, `crates/db/src/db/queries_extra/calendars.rs:1585`, `crates/db/src/db/queries_extra/calendars.rs:1606`, `crates/db/src/db/queries_extra/calendars.rs:1680`. On an Oslo machine, a monthly event at `DTSTART;TZID=Pacific/Kiritimati:20260101T090000` is seen locally as Dec 31, so `advance_months` preserves day 31 and skips short months. That emits Jan 1, Feb 1, Apr 1, Jun 1 in Kiritimati instead of Jan 1, Feb 1, Mar 1, Apr 1. The inverse happens for late-night Pacific/Niue events that become the next local date.

4. `crates/db/src/db/queries_extra/calendars.rs:1727` says DATE-only UNTIL means end of local day, but `crates/db/src/db/queries_extra/calendars.rs:1752` stores 23:59:59 as UTC.

   This is wrong for extreme offsets. `FREQ=DAILY;UNTIL=20260101` on a Pacific/Kiritimati event should stop at 2026-01-01 23:59:59 +14 (09:59:59Z), but code uses 23:59:59Z, so it can include an extra Jan 2 local occurrence. For Pacific/Niue, late Jan 1 occurrences can be excluded because local end-of-day is Jan 2 UTC.

5. `crates/db/src/db/queries_extra/calendars.rs:1083` stores recurring event duration as elapsed seconds, which breaks all-day recurrences spanning DST.

   CalDAV all-day dates are stored as local midnights at `crates/core/src/caldav/parse.rs:255` and `crates/core/src/caldav/parse.rs:317`. An all-day event `DTSTART;VALUE=DATE:20240331`, `DTEND;VALUE=DATE:20240402`, `RRULE:FREQ=WEEKLY;COUNT=2` in a zone that springs forward has a 47-hour first duration. The recurrence code then sets future ends with `start + duration` at `crates/db/src/db/queries_extra/calendars.rs:1127`, so the next two-day all-day instance ends at 23:00 local instead of the following midnight.

6. `crates/graph/src/calendar_sync.rs:552` silently treats unmapped Windows zones as UTC.

   "GMT Standard Time" is covered by calcard, so that example is fine. But modern Windows names not known to calcard or the Daylight->Standard substitution, such as "South Sudan Standard Time", fall through to None at `crates/graph/src/calendar_sync.rs:574`, then `parse_graph_datetime` stores the naive wall clock as UTC at `crates/graph/src/calendar_sync.rs:538`. `2024-06-15T10:00:00` in Africa/Juba becomes 10:00Z instead of 08:00Z.

### Intentional / Not Findings

`TZID=UTC` plus a Z suffix is handled intentionally: CalDAV logs and honors the embedded UTC offset at `crates/core/src/caldav/parse.rs:276`, then returns `dt.to_timestamp()` at `crates/core/src/caldav/parse.rs:294`.

Floating CalDAV times with no TZID and no Z are intentionally interpreted in `chrono::Local` at `crates/core/src/caldav/parse.rs:301`. That is surprising but matches the file's stated policy.

Pre-1970 timestamps are not inherently rejected by these helpers. Negative iCalendar years are not accepted through `calcard::PartialDateTime` because the parsed year is unsigned, so the negative-year risk is mainly DB-injected timestamps, not normal CalDAV parsing.

---

## Round 2 / Reviewer 2 - RRULE semantic correctness (Opus internal)

Target: `crates/db/src/db/queries_extra/calendars.rs` (`expand_recurrence` and helpers, lines 1052-1754).

The expander is a useful subset, but several inputs violate the RFC contract. Findings ordered by severity.

### F1. Malformed-rule vs zero-instance signaling: silent and inverted

**Location:** `expand_recurrence` lines 1077-1136; call site lines 1041-1046.

The function distinguishes two unrelated failures the same way visually, and a third inversely:

1. **Unknown FREQ** (line 1079 `Freq::parse` returns `None`) -> returns `vec![event.clone()]` - the event renders **once**, at its original timestamp, on the calendar. There is no log, no error, no flag.
2. **Empty `freq` field** (e.g. `RRULE:INTERVAL=2` with no `FREQ=`, or `RRULE:` with only whitespace) -> identical behavior, single instance returned (line 1080).
3. **Rule is well-formed but produces zero instances** (UNTIL in the past, BYDAY excludes everything, BYMONTHDAY=31 with INTERVAL=12 starting Feb) -> returns empty `Vec` (line 1136). The event **disappears entirely from the calendar**.

User-visible difference: a malformed rule keeps the event visible (as a one-shot); a *correctly* expired rule erases it. That is the wrong way around. A user who set `UNTIL=20290101` on an event in 2030 sees nothing and has no way to know why; a server that sent gibberish in `FREQ=` gets the event displayed once with no warning. The unit test `empty_expansion_returns_empty_not_original` (line 2071) intentionally enshrines case 3.

There is no error channel back to the caller - `expand_recurrence` returns `Vec`, never `Result`. Logging happens only for the COUNT+UNTIL combination (line 1091). Recommend: malformed-RRULE detection should at minimum log; better, surface "broken rule" so the UI can show the master event with a warning indicator instead of silently swapping behavior.

### F2. `BYDAY` with ordinal under `FREQ=YEARLY` - semantics narrowed without notice

**Location:** `expand_yearly` lines 1525-1579, `collect_monthly_days` lines 1437-1480, `nth_weekday_in_month` lines 1496-1512.

**Input:** `FREQ=YEARLY;BYDAY=20MO` - RFC 5545 Section 3.3.10 explicitly permits ordinals up to +/-53 on YEARLY rules; this means *the 20th Monday of the year*.

**dateutil-correct expansion** (DTSTART 2026-01-01): the 20th Monday of 2026 is **2026-05-18**, then 2027-05-17, 2028-05-15, 2029-05-14, 2030-05-19.

**What the code does:** Line 1559 calls `collect_monthly_days(year, *month, &rule.byday, &rule.bymonthday)` for each month in `BYMONTH`. Since `BYMONTH` is empty, `months = vec![dt.month()]` (line 1542) - only the start's own month is visited. Inside `collect_monthly_days`, `nth_weekday_in_month(year, month, Mon, 20)` (line 1449) asks for the 20th Monday *of that month*. A month has at most 5 Mondays, so this returns `None`. Result: zero instances from this month, the loop steps year-by-year, and the rule emits **nothing**.

The ordinal is silently re-interpreted as "n-th weekday of month" even when `FREQ=YEARLY`. RFC 5545 says positive/negative ordinal must be evaluated against the period set by FREQ - yearly should look across the whole year. There is no codepath in `expand_yearly` that walks all 53 weeks. The constant `53` doesn't appear anywhere in the file; nor does any "year-wide weekday" helper.

**Fix scope:** real and substantive. Either implement year-scope ordinal lookup, or reject (log+drop or log+fall back) ordinal BYDAY entries when the FREQ is YEARLY *and* BYMONTH is unset, so the user sees the failure.

### F3. `BYSETPOS` is not parsed - silently dropped

**Location:** `parse_rrule` lines 1200-1244.

`parse_rrule` only inspects `FREQ`, `INTERVAL`, `COUNT`, `UNTIL`, `BYDAY`, `BYMONTHDAY`, `BYMONTH`, `WKST`. `BYSETPOS` is not in the if/else-if chain (line 1239 ends with WKST). The comment on `Rrule` line 1164 says "Unknown parts are ignored silently."

**Input:** `FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1` - the last weekday of each month.

**dateutil-correct:** 2026-03-31 (Tue), 2026-04-30 (Thu), 2026-05-29 (Fri), 2026-06-30 (Tue), 2026-07-31 (Fri).

**What the code does:** Drops `BYSETPOS` entirely. `collect_monthly_days` now returns *every* Mon-Fri in the month (~22 days). The user gets a calendar full of daily entries that should have been one-per-month - same severity, opposite direction, of F1: instead of vanishing, the event multiplies ~20x.

Same problem applies to `BYHOUR`, `BYMINUTE`, `BYSECOND`, `BYWEEKNO`, `BYYEARDAY`. None are parsed; all are silently dropped. There is no validation that the rule does not contain BY-rules the expander cannot honor - so a remote server's accurate rule can produce a wildly inaccurate local expansion with no diagnostic.

### F4. `BYMONTHDAY=-31` (or any negative within range) - handled correctly *but* silently 0-instance under YEARLY without `BYMONTH=1,3,5,7,8,10,12`

**Location:** `collect_monthly_days` lines 1457-1468.

The parser at line 1224 accepts negative values whose magnitude is in 1..=31, and `collect_monthly_days` resolves them with `dim_i + d + 1` (line 1460). This is correct for monthly: `-31` resolves to day 1 in 31-day months and to a value < 1 (filtered out at 1461) in shorter months - emits only in 31-day months. Good.

For `FREQ=YEARLY;BYMONTHDAY=-31` (visiting only the start's own month), if the start is in February the rule produces nothing for that year and step-bounds out after 12000 yearly iterations (way more than needed). For `FREQ=YEARLY;BYMONTHDAY=29;BYMONTH=2` (leap day): correct - `days_in_month(non-leap, 2) = 28`, the resolved day fails the bound check at 1461, no candidate emitted. Skip-non-leap behavior matches dateutil. The default-day path at line 1551-1556 also handles this correctly for Feb 29 starts. No bug here.

### F5. `INTERVAL=0` clamped to 1 - defensible but undocumented to the user

**Location:** `parse_rrule` line 1210: `out.interval = val.parse().unwrap_or(1).max(1);`.

`INTERVAL=0` is invalid per RFC 5545 Section 3.3.10 (must be >= 1). Clamping is the standard tolerant behavior; dateutil throws. Defensible. But it is silent - combined with F1, an emitter sending `INTERVAL=0` followed by `BYDAY=garbage` will get a daily-cadence rule, not a malformed one. The user has no feedback that anything was clamped.

`val.parse().unwrap_or(1)` also catches negative values (parse fails for unsigned context here? - `interval: i64`, so `INTERVAL=-3` parses to -3, then `.max(1)` clamps to 1). OK.

### F6. `BYDAY=` (empty value) - splits into one empty string, falls through

**Location:** `parse_rrule` line 1223 -> `parse_byday` lines 1266-1303.

`val.split(',')` on an empty string yields a single empty `""`. `parse_byday("")`: `bytes` empty, no sign, no digits, ordinal `None`, `code = ""`, `parse_weekday_code("")` returns `None`. `filter_map` discards it. `out.byday` ends up `vec![]`. So `BYDAY=` is treated as if the BYDAY clause weren't present.

This silently disagrees with strict parsers (which would reject the whole rule), but produces the more permissive "default expansion" - generally fine.

`BYDAY=,MO,` produces `vec![Mo]` - leading/trailing empty entries silently drop. Tolerant; fine.

### F7. `BYMONTHDAY=32` and `BYMONTH=13` - silently dropped, same trap

**Location:** lines 1228-1232 (BYMONTHDAY filter) and lines 1237 (BYMONTH filter).

The magnitude check `(1..=31).contains(&mag)` rejects `32`. `BYMONTH` filter rejects `13`. Both produce `vec![]`. So `FREQ=MONTHLY;BYMONTHDAY=32` becomes equivalent to `FREQ=MONTHLY` with no BYMONTHDAY (default-same-day-of-month-as-start), not to a no-op rule. A user setting "every month on the 32nd" (typo for the last day, perhaps) gets a working monthly recurrence on whatever DTSTART's day is. No warning.

Same shape: `FREQ=YEARLY;BYMONTH=13` falls back to "same month as start." `FREQ=YEARLY;BYMONTH=2,13` keeps the valid ones - `vec![2]`. That last behavior is *defensible* (drop invalid, keep valid) and matches RFC 5545 Section 3.3.3 which says invalid values render the rule invalid, but tolerance is a common choice.

### F8. `WKST` only affects WEEKLY - BYDAY-with-ordinal never consults it

**Location:** `expand_weekly` line 1352, `collect_monthly_days` doesn't take `wkst`, `expand_yearly` doesn't either.

`wkst` is only consulted in `expand_weekly`. RFC 5545 Section 3.3.10 says WKST is significant whenever BYWEEKNO is used or whenever a weekly rule crosses week boundaries. Since `BYWEEKNO` isn't supported (F3), this is mostly OK in practice - but it means the documented `wkst: Option<chrono::Weekday>` field on `Rrule` does nothing for the YEARLY/MONTHLY paths, and there is no comment to that effect. A future addition that wires BYWEEKNO into yearly expansion will silently ignore WKST unless plumbed through.

The Sunday-anchored test (`wkst_sunday_anchors_week_to_sunday`, line 2081) confirms WEEKLY behavior; no test exercises WKST + week-crossing BYDAY in a non-trivial way.

### F9. `FREQ=WEEKLY;COUNT=1` with BYDAY excluding DTSTART - DTSTART silently dropped

**Location:** `expand_weekly` lines 1372-1381.

**Input:** DTSTART = Mon 2026-03-09, `FREQ=WEEKLY;BYDAY=TU;COUNT=1`.

**dateutil-correct:** RFC 5545 says DTSTART is *always* the first instance of the recurrence set even if it doesn't match BYDAY (it's inherent in the set). dateutil with default `rdates=False` actually drops DTSTART silently if it doesn't match, but this is a known dateutil deviation; the strict RFC reading and `vobject`/Outlook behavior is to include DTSTART.

**What the code does:** `week_anchor = start_of_week(start, Mon) = 2026-03-09 (Mon)`, the loop iterates `&days = [Tue]`, `candidate = 2026-03-10` >= start, pushed. So we emit Tue 2026-03-10 instead of (or in addition to) Mon 2026-03-09. Matches dateutil; deviates from a strict "DTSTART always included" reading. Worth a deliberate decision and a code comment, neither of which exists.

### F10. `FREQ=WEEKLY;BYDAY=MO,WE,FR;INTERVAL=2;COUNT=10`

**Input:** DTSTART = Mon 2026-03-09 09:00.

**dateutil-correct:** Mon 2026-03-09, Wed 2026-03-11, Fri 2026-03-13 (week 1), Mon 2026-03-23, Wed 2026-03-25, Fri 2026-03-27 (week 3), Mon 2026-04-06, Wed 2026-04-08, Fri 2026-04-10, Mon 2026-04-20.

**What the code does:** `interval_days = 14`, `wkst = Mon`, `days = [Mon, Wed, Fri]` after sort. `week_anchor = Mon 2026-03-09`. First iteration: emits Mon, Wed, Fri (all >= start). `week_anchor += 14` days = Mon 2026-03-23. Second iter: Mon, Wed, Fri 23/25/27. Third iter: Mon 2026-04-06 etc. Matches dateutil. Good.

### F11. `FREQ=DAILY;UNTIL=19700101T000000Z` (UNTIL strictly before DTSTART)

**Location:** `expand_recurrence` lines 1100-1129.

`window_end = until = 0` (Unix epoch). The loop at 1115: first candidate is `start` (some 2020s timestamp), `start > window_end` true, immediate `break`. `instances` stays empty. `expand_recurrence` returns `vec![]`. Per F1 above, the event vanishes from the calendar. RFC-correct semantics (rule expired); user-hostile signaling.

### F12. `COUNT=100000` - RRULE_MAX_COUNT does fire, but step cap is suspicious

**Location:** parser line 1219 clamps to `RRULE_MAX_COUNT = 10_000`. Test `count_clamped_to_max` (line 1924) exercises this for DAILY.

For `expand_daily` with a fast-matching filter, `cap = 10_000`, but the loop bound is `RRULE_MAX_STEPS = 12_000` (line 1313). For an unfiltered DAILY rule, every step produces an instance, so 10_000 fits comfortably under 12_000. Good.

For `expand_weekly` with BYDAY=MO (one match per week), 10_000 instances need 10_000 weeks of stepping, but the outer `for _ in 0..RRULE_MAX_STEPS` loop counts *weeks* (line 1368) - each step is one week, emitting up to 7 instances. Cap reached well before step cap. Good.

For `expand_monthly` with BYDAY=MO (~4 matches/month), 10_000 instances need 2500 months. Loop counts months. 2500 < 12_000. OK.

For `expand_yearly` with `BYMONTH=2;BYMONTHDAY=29` (leap days only): emits roughly once every 4 years, but the outer loop steps yearly (line 1576: `advance_months(current, rule.interval * 12)`). With `COUNT=10000` we'd need 40,000 years - far past the 12,000-step cap. Result: only ~3000 instances emitted. No warning that COUNT was not honored. This is a low-risk silent truncation, but the comment at 1062 ("~30 years of daily checks") doesn't match the actual yearly-step behavior of `expand_yearly` (which gives 12,000 years of yearly checks).

### F13. DTSTART with sub-second precision

**Location:** entire pipeline operates on `i64` Unix timestamps (seconds). `chrono::Local.timestamp_opt(start, 0)` everywhere passes `0` nanos.

DTSTART with sub-second precision (`19700101T000000.500Z`) is uncommon in iCal (the spec uses second precision throughout) but if a server *did* emit it, the field carrying it into `start_time: i64` already truncated. Not the expander's bug, but worth confirming that the schema column is integer seconds (which it is, given `event.start_time: i64`). No issue at this layer.

### F14. TZID-anchored DTSTART crossing DST

**Location:** `add_days_local` lines 1604-1611, `with_year_month_day` lines 1583-1589, `shift_to_weekday` lines 1628-1659. All use `chrono::Local` and re-resolve via `crate::db::time::resolve_local_to_timestamp`.

The DAILY/WEEKLY paths preserve wall-clock time across DST (calendar-day arithmetic on `naive_local`, comments at 1322-1325 confirm intent). MONTHLY/YEARLY paths do the same via `with_day_of_month` / `with_year_month_day`. Reasonable.

**Two caveats:**

(a) The expander uses `chrono::Local` *everywhere* - there is no path that honors a TZID stored on the event. `event.timezone: Option<String>` is a field on `CalendarViewEvent` (line 1810 in tests), but `expand_recurrence` and helpers never read it. So a meeting marked `TZID=America/New_York` is expanded in the user's local zone. For a user in `Europe/Oslo` whose colleague schedules `09:00 America/New_York every Tuesday`, the displayed times after a DST transition will be off by an hour for the period when New York and Oslo are in different DST states. This is a substantive bug, but the scope is bigger than RRULE - it requires plumbing TZID through to a `chrono_tz::Tz` resolve. Worth flagging.

(b) For `chrono::Local` users, `resolve_local_to_timestamp` is called for every wall-clock-preserving operation. If that function's fallback strategy on the spring-forward gap (a 09:00 event that doesn't exist on the gap day) is to return `None`, then `add_days_local` returns `None` and the caller falls through to `current + interval * 86400` (line 1326) - which uses raw seconds, *not* preserving wall-clock. So on the one day per year that 09:00 doesn't exist in the local zone, a daily 09:00 event silently shifts to 10:00 from that day forward. Recommend reading `resolve_local_to_timestamp` to confirm the gap-day strategy.

### F15. `BYDAY=2WE,-1FR` - mixed ordinal and bare in one rule

**Location:** `parse_byday` lines 1266-1303 (no rejection for mixing), `collect_monthly_days` lines 1445-1453.

**Input:** `FREQ=MONTHLY;BYDAY=2WE,-1FR`.

**dateutil-correct:** the 2nd Wednesday and the last Friday of each month.

**What the code does:** Parser: two `ByDay` entries - `{ordinal: Some(2), day: Wed}` and `{ordinal: Some(-1), day: Fri}`. In `collect_monthly_days` line 1445-1453, each is independently resolved: 2nd Wed -> one day, -1 Fri -> one day. `byday_days = vec![second_wed, last_fri]`. With no `BYMONTHDAY`, both are emitted. After sort_unstable+dedup at line 1412-1413, both kept and emitted in calendar order. Correct.

Mixing bare and ordinal forms in the same BYDAY (`BYDAY=MO,1FR`) is also handled correctly: the bare `MO` flat-maps to all Mondays, the `1FR` to the first Friday only. Correct per RFC 5545.

### F16. `parse_until_date` - DATE-only branch always anchors to UTC

**Location:** lines 1732-1753.

`DATE`-only `UNTIL=20260315`: parsed to `2026-03-15 23:59:59 UTC` (line 1752 -> `dt.and_utc().timestamp()`).

For a user in `Europe/Oslo` (UTC+1 / +2), 23:59:59 UTC on the UNTIL date is 00:59:59 / 01:59:59 *local* on the *next* day. So a `DAILY` event at 14:00 local on the UNTIL date is included (correct - UNTIL is end-of-day inclusive). But a daily 23:30 local event on the day *after* UNTIL would also be included (23:30 local = 22:30 UTC < 23:59:59 UTC of the previous UTC day - wait, depends on timezone). In `Europe/Oslo` summer time (UTC+2): UNTIL = 2026-03-15 23:59:59 UTC = 2026-03-16 01:59:59 local. A daily 09:00-local event on 2026-03-16 -> 07:00 UTC on 2026-03-16 -> comparing as i64 timestamps, 07:00 UTC > 23:59:59 UTC of previous day -> excluded. OK.

In Pacific time (UTC-8): UNTIL = 2026-03-15 23:59:59 UTC = 2026-03-15 15:59:59 local. A daily 09:00-local event on 2026-03-15 -> 17:00 UTC > 23:59:59 UTC of *prior* day = 2026-03-14? Let me redo: UNTIL=20260315 -> 2026-03-15 23:59:59 UTC, that timestamp. A 09:00 PT event on 2026-03-15 = 17:00 UTC on 2026-03-15. 17:00 < 23:59:59 - included. OK. What about 2026-03-15 evening 22:00 PT = 2026-03-16 06:00 UTC > 23:59:59 UTC of 2026-03-15 - excluded. So events later than ~16:00 PT on the UNTIL day are excluded. dateutil with `dtstart.tzinfo` resolves UNTIL=20260315 (DATE only) to 2026-03-15 23:59:59 *in the DTSTART's timezone* = 2026-03-16 07:59:59 UTC - which would *include* the 22:00 PT event.

So **DATE-only UNTIL clips early in west-of-UTC zones and late in east-of-UTC zones**. In US Pacific the user can lose evening occurrences on the UNTIL day. Fix: anchor the DATE-only UNTIL to the local zone (`chrono::Local` or the event's TZID) rather than UTC.

### Summary of severities

| # | Issue | Severity | RFC violation? |
|---|---|---|---|
| F1 | Malformed-rule signal inverted with zero-instance | High | Out-of-spec UX |
| F2 | YEARLY+ordinal-BYDAY produces wrong/empty results | High | Yes |
| F3 | BYSETPOS / BYWEEKNO / BYYEARDAY silently dropped | High | Yes |
| F14a | TZID on event ignored | High (scope: bigger) | Yes |
| F16 | DATE-only UNTIL anchored to UTC, not local zone | Medium | Yes |
| F12 | COUNT=10_000 with sparse YEARLY truncates silently | Low | Tolerable |
| F7 | Invalid BYMONTHDAY/BYMONTH silently fall back to default | Low | Tolerable |
| F5 | INTERVAL=0 silently clamped | Low | Defensible |
| F9 | DTSTART not in BYDAY filter is dropped | Low | Matches dateutil |
| F8 | WKST only consulted in WEEKLY | Latent | OK now |
| F14b | DST-gap fallback uses raw seconds | Latent | Depends on resolve_local_to_timestamp |

The hottest two for a user likely to hit them in practice: **F2** (YEARLY+ordinal silently emits nothing - common pattern in MS Outlook RRULEs like "third Monday of every year"), and **F3** (BYSETPOS - extremely common in real iCal feeds for "last weekday of month" patterns, currently expands ~20x too many instances).

### Aside on call-site signaling

Confirmed: the parsers do NOT inspect `<status>` codes inside `<propstat>` elements at all (cross-reference to Reviewer 3 finding 1). The call site at line 1042 uses `expand_recurrence` directly and appends results. If the rule is malformed and parses to no `Freq`, it returns `vec![event.clone()]` (line 1080) - silently shows the original. If filters reject everything, an empty `Vec` is returned and the event vanishes from the calendar entirely.

---

## Round 2 / Reviewer 2 - RRULE semantic correctness (Outside)

Assumptions for concrete traces: unless noted, `DTSTART=20260101T090000`, local/floating time. Reviewer did not run cargo/brokkr/git or edit files.

### Findings

1. TZID is ignored during expansion. `CalendarViewEvent` carries timezone, but `expand_recurrence` never uses it and all calendar arithmetic uses `chrono::Local`: `crates/db/src/db/queries_extra/calendars.rs:982`, `crates/db/src/db/queries_extra/calendars.rs:1077`, `crates/db/src/db/queries_extra/calendars.rs:1604`.

   Input: `DTSTART;TZID=America/New_York:20240309T090000`, `RRULE:FREQ=DAILY;COUNT=4`.

   dateutil-correct: 2024-03-09 09:00 -05:00, then 2024-03-10/11/12 09:00 -04:00 = UTC 14:00, 13:00, 13:00, 13:00.

   Code on this Europe/Oslo host preserves Oslo wall time from the stored instant, producing UTC 14:00, 14:00, 14:00, 14:00, which is 10:00 in New York after DST starts.

2. YEARLY ordinal BYDAY without BYMONTH is interpreted as "nth weekday of DTSTART's month", not "nth weekday of the year". `expand_yearly` defaults months to `dt.month()` and delegates ordinal BYDAY to the monthly helper: `crates/db/src/db/queries_extra/calendars.rs:1539`, `crates/db/src/db/queries_extra/calendars.rs:1558`, `crates/db/src/db/queries_extra/calendars.rs:1494`.

   Input: `FREQ=YEARLY;BYDAY=20MO;COUNT=5`.

   dateutil-correct: 2026-05-18, 2027-05-17, 2028-05-15, 2029-05-14, 2030-05-20.

   Code visits only January, asks for the 20th Monday of January, finds none, and returns [].

3. Explicit monthly BYxxx rules can skip entire months based on DTSTART's day-of-month. Even when BYMONTHDAY/BYDAY supplies the actual candidate days, `expand_monthly` advances via `advance_months(current, interval)`, whose contract intentionally skips months that do not contain the original `current` day: `crates/db/src/db/queries_extra/calendars.rs:1425`, `crates/db/src/db/queries_extra/calendars.rs:1686`.

   Input: `DTSTART=20260131T090000`, `FREQ=MONTHLY;BYMONTHDAY=1,-1;COUNT=5`.

   dateutil-correct: 2026-01-31, 2026-02-01, 2026-02-28, 2026-03-01, 2026-03-31.

   Code produces 2026-01-31, 2026-03-01, 2026-03-31, 2026-05-01, 2026-05-31, skipping February and April because the month anchor is still "31".

4. Plain yearly Feb 29 recurrences drift to March after the first non-leap year. This is the same `advance_months` issue in yearly default mode: default month/day comes from the mutable `current`, then `advance_months(Feb 29, 12)` advances to March 29, 2025: `crates/db/src/db/queries_extra/calendars.rs:1541`, `crates/db/src/db/queries_extra/calendars.rs:1551`, `crates/db/src/db/queries_extra/calendars.rs:1576`.

   Input: `DTSTART=20240229T090000`, `FREQ=YEARLY;COUNT=5`.

   dateutil-correct: 2024-02-29, 2028-02-29, 2032-02-29, 2036-02-29, 2040-02-29.

   Code produces 2024-02-29, 2025-03-29, 2026-03-29, 2027-03-29, 2028-03-29.

5. BYSETPOS is silently ignored. `parse_rrule` only handles a fixed set of keys; unknown parts are ignored by design: `crates/db/src/db/queries_extra/calendars.rs:1164`, `crates/db/src/db/queries_extra/calendars.rs:1206`.

   Input: `FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1;COUNT=5`.

   dateutil-correct: last weekday of each month: 2026-01-30, 2026-02-27, 2026-03-31, 2026-04-30, 2026-05-29.

   Code ignores BYSETPOS and emits the first five weekdays from January: 2026-01-01, 2026-01-02, 2026-01-05, 2026-01-06, 2026-01-07.

6. Malformed or out-of-range parts are coerced into different valid rules. `INTERVAL=0` is clamped to 1, empty BYDAY becomes no BYDAY filter, and invalid BYMONTHDAY/BYMONTH values are filtered out: `crates/db/src/db/queries_extra/calendars.rs:1210`, `crates/db/src/db/queries_extra/calendars.rs:1222`, `crates/db/src/db/queries_extra/calendars.rs:1228`, `crates/db/src/db/queries_extra/calendars.rs:1237`.

   Inputs and code behavior: `BYDAY=` with weekly count 5 becomes plain weekly Thursdays; `BYMONTHDAY=32` becomes monthly on the DTSTART day; `BYMONTH=13` becomes yearly in the DTSTART month; `INTERVAL=0` becomes `INTERVAL=1`. dateutil rejects empty BYDAY; for `BYMONTHDAY=32`/`BYMONTH=13` it yields no instances; for `INTERVAL=0` dateutil 2.9 repeats DTSTART, while RFC 5545 says interval is a positive integer. In all cases, the code silently changes meaning.

7. COUNT is capped without signaling truncation. `COUNT=100000` is parsed as 10_000: `crates/db/src/db/queries_extra/calendars.rs:1055`, `crates/db/src/db/queries_extra/calendars.rs:1216`.

   Input: `FREQ=DAILY;COUNT=100000`.

   First five match dateutil: 2026-01-01 through 2026-01-05; total does not. dateutil produces 100,000 instances; code produces at most 10,000. That cap is defensible for DoS control, but it is not RFC-semantic expansion unless callers can see "truncated".

8. The advertised two-year fallback window is not honored for unbounded DAILY rules. `window_end` is two years out when no COUNT/UNTIL exists, but `max_instances` defaults to 365 and `expand_daily` defaults to 366 candidates: `crates/db/src/db/queries_extra/calendars.rs:1100`, `crates/db/src/db/queries_extra/calendars.rs:1106`, `crates/db/src/db/queries_extra/calendars.rs:1306`.

   Input: `FREQ=DAILY` from 2026-01-01. Under the function's own two-year-window contract, expected instances run through 2028-01-01 inclusive. Code stops after 365 instances, at 2026-12-31.

### Stress Cases That Matched

`FREQ=MONTHLY;BYDAY=2WE,-1FR` is honored for a normal month anchor: 2026-01-14, 2026-01-30, 2026-02-11, 2026-02-27, etc. `FREQ=MONTHLY;BYMONTHDAY=-31` correctly emits only the first day of 31-day months. `FREQ=YEARLY;BYMONTH=2;BYMONTHDAY=29` correctly skips non-leap years when BYMONTH/BYMONTHDAY are explicit. Weekly `COUNT=1` with BYDAY excluding DTSTART, weekly `MO,WE,FR;INTERVAL=2`, UNTIL before DTSTART, and the `WKST=MO` vs `WKST=SU` crossing-boundary examples all trace correctly.

Sub-second DTSTART is effectively outside this expander's representable contract: event times are `i64` seconds in `CalendarViewEvent` (`crates/db/src/db/queries_extra/calendars.rs:967`), and RFC 5545 DATE-TIME itself is second-precision.

### Malformed vs Zero Instances

The code does not distinguish them. `expand_recurrence` returns only `Vec<CalendarViewEvent>`, unknown FREQ returns the original event, and successful-but-empty expansion returns an empty vector: `crates/db/src/db/queries_extra/calendars.rs:1077`, `crates/db/src/db/queries_extra/calendars.rs:1080`, `crates/db/src/db/queries_extra/calendars.rs:1131`. The view loader just appends whatever comes back, so empty means the event disappears from the view: `crates/db/src/db/queries_extra/calendars.rs:1041`. Malformed rules can therefore disappear, show as the original single event, or expand as a different valid recurrence, with no user-visible error channel.

---

## Round 2 / Reviewer 3 - Real-world CalDAV server compatibility (Opus internal)

Targets:
- `crates/core/src/caldav/parse.rs` (`parse_propfind_calendars`, `parse_propfind_events`, `parse_ctag`, `parse_multiget_report`, `extract_datetime`, `parse_icalendar`, `extract_vevent`, `local_name`)
- `crates/core/src/caldav/client.rs` (`CalDavClient`, `discover`, `discover_principal`, `list_calendars`, `list_events`, `get_event_ical`, `put_event`, `delete_event`, `resolve_url_against`, `normalize_if_match_etag`)
- `crates/calendar/src/caldav/mod.rs` (`CaldavAccountConfig`, `load_caldav_account_config`, `build_client_from_config`, `persist_discovery_results`, `fetch_caldav_event`, `finalize_event`, `synthesize_event_dto`)

### High-severity findings

#### 1. `<propstat><status>` is never inspected - 404/403 props treated as present
**Files:** `crates/core/src/caldav/parse.rs:402-499` (`parse_propfind_calendars`), `:506-575` (`parse_propfind_events`), `:580-621` (`parse_ctag`), `:628-690` (`parse_multiget_report`), `crates/core/src/caldav/client.rs:637-684` (`extract_href_property`)

None of the XML parsers ever read `<D:status>` inside `<D:propstat>`. RFC 4918 Section 13 says a multistatus `<response>` can contain *multiple* `<propstat>` blocks - typically one with HTTP/1.1 200 OK for properties that exist and another with HTTP/1.1 404 Not Found (or 403, 401) for properties that don't. The parsers iterate every `<prop>` regardless of the sibling `<status>`.

Server inputs that trigger this:
- Radicale, SOGo, DAViCal, and Apple Calendar Server all return a 404 `<propstat>` for properties not supported on a given resource (e.g. `getctag` is sometimes only on calendars, not collections; `calendar-color` is iCal-namespace and frequently missing on non-Apple servers; `getcontenttype` returns 404 on collection rows).
- Fastmail's CalDAV returns a 404 `<propstat>` block where the inner `<getetag/>` element is **self-closed and empty**. Because `parse_propfind_events` only checks `current_etag.is_empty()` and never looks at the status, an empty etag from a 404 propstat is rejected - fine in this one case - but the same response shape with a *non-empty* placeholder (some servers emit `<getetag/>` empty in 404 and `<getetag>...</getetag>` in 200; one row may include both blocks) means whichever block is seen *last* wins. For multistatus rows that include both `200/getcontenttype=text/calendar` for the resource and `404/getetag` propstat, you'd silently store the empty/wrong etag.

User-visible consequence:
- For `parse_propfind_calendars` (`:470-489`): a calendar collection that returns a 404 propstat for `displayname` (because the server requires a per-user prop fetch) and a 200 for `resourcetype` - the whole row is still emitted, but `display_name` is `None`. Falls back to "Calendar N" naming in `caldav_list_calendars_impl` (`crates/calendar/src/caldav/mod.rs:60-62`). Cosmetic.
- For `parse_propfind_events` (`:556-565`): if a propstat contains both a 200 block and a 404 block for `getetag`, ordering bugs can surface the 404 placeholder. In a typical Outlook/EAS-bridged CalDAV response (Kerio, MailEnable) the `<getetag>` may legitimately be empty for placeholder occurrences of a recurring series - currently those entries are dropped silently because `current_etag.is_empty()` filters them. Result: occurrences of a recurring event masters never sync until their real etag lands.
- For `parse_multiget_report` (`:675-680`): more important - see finding 2.

#### 2. `parse_multiget_report` returns `(uri, ical)` for resources that errored
**File:** `crates/core/src/caldav/parse.rs:628-690`

Same problem, larger blast radius. A server returning a per-resource error in a multiget (e.g. one of the 50 events in a batch was deleted between the PROPFIND and the REPORT) emits:

```xml
<D:response>
  <D:href>/cal/missing.ics</D:href>
  <D:status>HTTP/1.1 404 Not Found</D:status>
</D:response>
```

...with no `<propstat>` and no `<calendar-data>`. The current parser correctly produces nothing for this row because `current_ical.is_empty()` (`:677`). **But** if the server emits a multistatus row with *both* a 200 propstat carrying `<calendar-data>...</calendar-data>` and a 404 propstat carrying nothing - which iCloud and SOGo do for partial-failure batches when `<expand>` is requested or when one event in the batch is corrupt - the parser does not distinguish the two propstats. As long as *some* `<calendar-data>` was found anywhere inside that `<response>`, it's emitted.

Concretely: SOGo, when a single event in a multiget batch fails to parse server-side, returns the failed resource with `<status>HTTP/1.1 500 Internal Server Error</status>` while still echoing back stale `<calendar-data>` from a cache. We accept the stale ical and write it to the local DB, overwriting potentially-fresher local edits.

User-visible: silent data loss / display of stale event data after a partial server failure.

#### 3. `extract_href_property` is brittle and grabs the first `<href>` inside the property - wrong for multi-host home-sets
**File:** `crates/core/src/caldav/client.rs:637-684`

The function tracks `in_property` as a boolean and `current_tag` as a single string. There is no element-stack; once it enters `<calendar-home-set>`, every subsequent `<href>` end event triggers a return. RFC 4791 Section 6.2.1 explicitly allows `<calendar-home-set>` to contain multiple `<href>` elements (one per shared mailbox / delegated calendar). Apple Calendar Server, Kerio, and Exchange front-ends with delegation return all of them. We pick the first.

Worse: because `current_tag = name` is overwritten on every `Event::Start` (`:654`) without a stack, a nested `<href>` inside, say, an `<owner>` or `<group>` element *that itself appears inside* `<calendar-home-set>` will still match (`current_tag == "href"` and `in_property` is still true). Some bridged servers (DAViCal in delegation mode) return `<calendar-home-set><owner><href>/principals/admin/</href></owner><href>/calendars/me/</href></calendar-home-set>` - we'd grab the principal href as the home-set, then later PROPFIND it for calendars and silently get back nothing useful.

User-visible: with delegation-enabled accounts, the user sees only one calendar home, or sees the wrong principal mistaken for a home-set, leading to "Could not discover calendar-home-set" or empty calendar lists.

#### 4. ETag-from-response is parsed as ASCII via `to_str`, dropping ETags with non-ASCII bytes
**Files:** `crates/core/src/caldav/client.rs:391-395` (GET), `:441-445` (PUT)

```rust
let etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).map(str::to_string);
```

`HeaderValue::to_str` returns `Err` for any byte > 0x7F. Yahoo and some Kerio/Zimbra installs base64-encode opaque internal hashes into the etag and the encoding occasionally leaks 8-bit bytes (because the underlying hash plus framing isn't always pre-ascii-clean). The header is *technically* RFC-violating, but it exists in the wild. We silently drop the ETag and fall through to the eventually-consistent GET path - which may return a different ETag, breaking optimistic concurrency on the next PUT (412 Precondition Failed -> user sees "save failed").

Same problem on PUT - we lose the canonical ETag and `synthesize_event_dto` (`mod.rs:139-141`) is bypassed because `put_etag` is `None`, so we pay the extra GET round-trip every save.

#### 5. ETag verbatim storage, but `normalize_if_match_etag` mangles the legacy weak-etag case
**File:** `crates/core/src/caldav/client.rs:708-715`

The doc-comment admits this:
> Corrupted weak ETag (`W/abc`): the old code stripped the inner quote; we cannot reliably reconstruct the original, but wrapping the whole token produces `"W/abc"` which the server will reject

The wrapper produces `"W/abc"` (a quoted string containing the literal characters `W`, `/`, `a`, `b`, `c`), which a strict server will treat as a strong-comparison match against an entity tag literally named `W/abc`. RFC 7232 Section 2.3 says weak validators MUST start with `W/` *outside* the quoted string. Most servers will simply 412-Precondition-Failed; some will erroneously accept. The recovery comment is correct (412 -> re-fetch -> fix), but in the meantime any update to that event from a legacy row appears to fail to the user, and they may keep retrying - each retry produces the same 412 because the stored value never gets refreshed unless the user does a full sync.

Better behavior: when we see a stored ETag matching `W/[^"]` (no inner quote), drop the If-Match header entirely and accept the conflict risk. That at least lets the save go through.

#### 6. Hrefs with embedded quotes / colons / slashes break the multiget request body
**File:** `crates/core/src/caldav/client.rs:278-292`

```rust
for uri in chunk {
    href_elements.push_str(&format!("  <D:href>{uri}</D:href>\n"));
}
```

The URI is splatted into XML with no escaping. Although per RFC 3986 a URI cannot contain literal `<`, `>`, or `&`, **CalDAV servers DO emit hrefs containing percent-encoded codepoints with mixed case** and **DO emit hrefs that aren't fully URL-percent-encoded** (Davical at one point emitted `%20` un-decoded but `+` literal). More importantly, a server returning `&` in an href (which it shouldn't, but Exchange OWA's CalDAV bridge has been seen to emit `?$filter=...&...` query strings in event hrefs, and the `&` is **not** XML-escaped) would form invalid XML in our request body, which the server would 400 Bad Request, killing the whole batch of 50.

Also: relative hrefs from `parse_propfind_events` come back with `entry.uri = self.resolve_url_against(&url, &entry.uri)` (`client.rs:253`), which *normalizes them to absolute URLs*. We then pass these absolute URLs as `<D:href>` inside a multiget targeting `/cal/`. RFC 4791 Section 7.9 says multiget hrefs SHOULD be relative to the request URI; some servers (older SOGo, in particular) reject absolute hrefs in a multiget that don't share scheme+host with the request URL. If the calendar URL was discovered with one host and the events got resolved against a redirect target, the multiget body will have hrefs the server rejects.

Relevant: `list_events` resolves entry URIs against the calendar URL, but `fetch_events` chunks them without re-relativizing. This works fine for the 99% case but explodes when the discovered calendar URL went through a redirect that changed `host` or `scheme` (e.g. `http://` -> `https://` upgrade returned during PROPFIND).

#### 7. `is_icalendar_resource` accepts any href without a trailing slash
**File:** `crates/core/src/caldav/parse.rs:711-720`

```rust
fn is_icalendar_resource(href: &str, content_type: &str) -> bool {
    if content_type.contains("text/calendar") { return true; }
    if href.ends_with(".ics") { return true; }
    content_type.is_empty() && !href.ends_with('/')
}
```

The third arm - "no content type, doesn't end with `/`" - is a generous fallback. A PROPFIND Depth:1 response can include sub-collections (e.g. `<href>/cal/personal/inbox/</href>`) that *don't* end with `/` if the server emits the inbox URI without a trailing slash. Davical, Bedework, and old Zimbra emit collection URIs without trailing slashes. If such a row also has an etag (Davical does for collections - the collection's getetag is the ctag), we'd treat the inbox itself as an event resource and try to multiget it, leading to a "the resource is a collection" 403 on the REPORT.

Also: hrefs with query strings (rare but valid) like `/cal/event.ics?revision=42` end with neither `.ics` nor `/`. With no content-type, they pass the third arm. With a non-empty content-type that doesn't contain `text/calendar` (e.g. an `application/calendar+xml` from a server that does return CalDAV-XML form), they're rejected. The CalDAV-XML form (RFC 6321) is rare but exists - we'd skip those entirely.

#### 8. `local_name(b':')` is byte-wise, doesn't handle the no-namespace-prefix case for default-namespaced documents
**File:** `crates/core/src/caldav/parse.rs:702-708`, `client.rs:687-693`

```rust
fn local_name(raw: &[u8]) -> String {
    let full = String::from_utf8_lossy(raw);
    match full.rfind(':') {
        Some(idx) => full[idx + 1..].to_string(),
        None => full.to_string(),
    }
}
```

This works for `<D:multistatus>` (returns `multistatus`), `<d:multistatus>` (lowercase), and `<multistatus>` (default namespace declared with `xmlns="DAV:"`). For `<DAV:multistatus xmlns:DAV="DAV:">` it returns `multistatus`. The fall-through from the `local_name` variations *is* handled.

But: `quick_xml::Reader` does not, by default, perform namespace-aware parsing - `e.name().as_ref()` returns the raw qualified name bytes, prefix included. This works in practice because of the hand-rolled split. However, **a server emitting `<x:multistatus xmlns:x="DAV:">` and `<x:href>` with the same prefix** behaves identically to the test fixtures. Real worry: a server emitting `<multistatus xmlns="DAV:">` with default-namespaced children including a *different* default namespace inside, e.g. `<multistatus xmlns="DAV:"><response><href>/cal/</href><propstat><prop xmlns="urn:ietf:params:xml:ns:caldav"><calendar-data>...` - the `<calendar-data>` element appears with no prefix, so `local_name` returns `calendar-data`. Good. But the `<response>` and `<prop>` elements also have no prefix, so the parent-stack disambiguation by *local* name still works.

The case it actually breaks: a server emitting **two different elements with the same local name in different namespaces**. E.g. SabreDAV's response to a privilege query can contain `<DAV:href>` (the response href) and `<auth:href>` (the realm URI) within the same response, where the `<auth:href>` would not be parented by any element our matcher cares about. Today the parent-scoping (`parent == Some("response")`) saves us. But `extract_href_property` (`client.rs:637-684`) does not parent-scope - it only checks `in_property` and the immediately-preceding tag - so within `<calendar-home-set>` any `<x:href>` from any namespace is matched. See finding 3.

### Medium-severity findings

#### 9. Multiple DTSTART lines: undefined behavior, dependent on calcard's first-wins or last-wins
**File:** `crates/core/src/caldav/parse.rs:121` (and `:236-305` in `extract_datetime`)

```rust
let Some(entry) = component.property(prop) else { return (None, false); };
```

`component.property()` is calcard's API; from the surrounding usage it returns the *first* matching property entry. RFC 5545 Section 3.6.1 says DTSTART MUST occur exactly once in a VEVENT, but Outlook bridges have been seen to emit two DTSTART lines (one with TZID, one as floating UTC fallback) for compatibility with old clients. We silently use the first; the second is dropped without a log entry. If the ordering varies between servers, the user sees the event at one of two different times depending on which server they last synced from.

Suggested fix: log when `component.properties(&Dtstart)` returns more than one entry.

#### 10. VEVENT with no UID is accepted
**File:** `crates/core/src/caldav/parse.rs:98`

`uid: component.uid().map(String::from)` -> `Option<String>`. Downstream in `synthesize_event_dto` (`mod.rs:182`) and `fetch_caldav_event` (`mod.rs:393`), `uid` propagates as `None`. The local DB schema almost certainly uses UID for dedup keys; a server that returns a recurring event with no UID (some VTODO/VEVENT mixed bridges do) would generate duplicates on every sync.

The store should either generate a synthetic UID from the href, or skip UID-less events with a warning. Today they pass through as `uid: None` and the sync layer must absorb the consequence.

#### 11. iCal value extraction silently drops empty strings as if absent
**File:** `crates/core/src/caldav/parse.rs:104, 111, 118` (`.filter(|s| !s.is_empty())`)

```rust
let summary = component
    .property(&Summary)
    .and_then(|e| e.values.first())
    .and_then(|v| v.as_text())
    .filter(|s| !s.is_empty())
    .map(String::from);
```

A server-side echo of `SUMMARY:` (empty value) becomes `summary: None`, which on round-trip via `caldav_update_event_impl` -> `merge_caldav_event_input` may cause us to populate a default summary. If the user actually *intended* to clear the title, this resists - they save with empty title, server stores `SUMMARY:`, parse returns None, merger probably keeps the previous title. User can't clear titles.

Different from "absent property" semantically; should be `Option<String>` returning `Some("")` on echo, and the formatter at the UI layer should decide whether to display blank or fallback.

#### 12. Folded-line + CRLF dependence on calcard
**File:** `crates/core/src/caldav/parse.rs:69` (calcard `Parser::new`)

Per RFC 5545 Section 3.1 lines longer than 75 octets MUST be folded with `CRLF + space` (or tab). The `parse_multiget_report` function trims outer whitespace on `current_ical` but otherwise hands raw text to calcard. If `quick_xml`'s text-event delivery splits a fold (CRLF inside a Text event, then the leading space arriving as a separate Text event), the buffer concatenation in `parse_multiget_report` (`:651-654`) preserves order so the fold is intact. However, if the server emits LF-only line endings (some Linux-side calendar bridges normalize incorrectly), calcard's RFC 5545 unfolder may not recognize the continuation and treat each unfolded line as a separate property, losing values.

Empirical behavior depends on calcard. If it's lenient about LF-only, we're fine. If not, any event with a long DESCRIPTION from a Linux server gets a truncated description on display.

Worth a unit test: an iCal blob with LF-only line endings *and* a folded long DESCRIPTION line.

#### 13. Discovery: 200 OK with empty body / 401 / well-known returning HTML 404
**File:** `crates/core/src/caldav/client.rs:128-207`

The doc-comment for `discover_principal` correctly notes that some Exchange front-ends respond 200 OK with HTML for `.well-known/caldav`. The implementation handles this by trying base URL first. Good.

But: `propfind_raw` returns `Ok((status, body))` for any 2xx or 207. A 200-with-HTML response makes it through. `extract_href_property` is called on HTML, finds no `<current-user-principal>` or `<href>`, returns `None`, and we report "PROPFIND on base URL returned no current-user-principal". That's the right user-facing message but it doesn't distinguish "the server isn't speaking CalDAV" from "the server is speaking CalDAV but principal isn't where we expected". Telling those two apart would help operator support.

A stronger check: verify the response has an XML content-type and a `<multistatus>` root before trying to extract properties. Without it, an authenticated SSO portal that returns 200 + HTML on every URL hits an infinite loop of "discover failed -> user retries -> discover failed".

#### 14. `redirect::Policy::limited(10)` is fine, but no scheme-downgrade protection
**File:** `crates/core/src/caldav/client.rs:57-60`

reqwest's default redirect policy will follow `https://server/cal` -> `http://server/cal` if the server returns a 301 with an http target. CalDAV credentials in the Authorization header would then be sent over plaintext on the redirected request. reqwest does NOT strip the Authorization header on cross-origin redirects in older versions; for current reqwest (~0.11+) it DOES strip, but the behavior depends on version. Worth a manual check to confirm the version in `Cargo.toml`. If it's an older one or if the redirect chain stays same-origin while downgrading scheme, the password leaks on first sync.

#### 15. Redirect chain across hosts in discovery: principal_url stored as absolute, home_url then resolved against base_url
**File:** `crates/core/src/caldav/client.rs:146-152`

```rust
if let Some(home) = extract_href_property(&body, "calendar-home-set") {
    self.calendar_home_url = Some(self.resolve_url(&home));
}
```

`self.resolve_url(&home)` calls `resolve_url_against(&self.base_url, &home)` (`:560`). If the principal lives at `https://principal.example.com/p/me/` (where the home-set PROPFIND was sent), and the response says `<href>/calendars/me/</href>` - that path is a path on `principal.example.com`, not on `base_url`. We'd resolve the wrong host.

Fix: resolve against `principal_url`, not `base_url`. The fact that the function `resolve_url_against` exists for exactly this use case (`list_events` uses it correctly at `:253`) but the discovery path does not, is the bug.

User-visible: in any setup where principal and DAV root are on different hosts (common for hosted Exchange + CalDAV bridges, or services like fastmail.com vs. www.fastmail.com vs. caldav.fastmail.com), discovery completes "successfully" but `list_calendars` then PROPFINDs the wrong host and gets 404 or a totally different account's calendars.

#### 16. `principal-URL pointing to a 404` - discovery succeeds, list_calendars fails
**File:** `crates/core/src/caldav/client.rs:128-162`

If the server returns a valid principal URL that 404s when PROPFIND'd for `calendar-home-set`, `propfind_raw` returns the error and we abort discovery. The error message will be "PROPFIND for calendar-home-set failed: 404". OK. But now `principal_url` **was set in step 1** (`:132`). The next call to `discover()` skips step 1 (`if self.principal_url.is_none()`) and re-attempts step 2 - same failure. The persisted principal in the DB never gets cleared. The user is stuck.

Mitigation: on home-set discovery failure, clear `self.principal_url` and clear the DB column too.

#### 17. Calendar home returned with 207 but zero `<response>` children
**File:** `crates/core/src/caldav/parse.rs:402-499`

`parse_propfind_calendars` returns `Vec::new()` for an empty multistatus. `caldav_list_calendars_impl` (`mod.rs:53`) returns `Ok(Vec::new())` to the UI. The user sees "no calendars" with no error. Could be:
- The server hasn't created the user's default calendar yet (first login race)
- The user's principal is right but they have no calendars provisioned (admin issue)
- The server is emitting an empty multistatus due to a server-side error it should have surfaced as 500

We can't distinguish these. At minimum, a log line at WARN level when `list_calendars()` returns an empty Vec would help.

#### 18. Server returns 200 (not 207) for a PROPFIND
**File:** `crates/core/src/caldav/client.rs:503-507`

```rust
if status.is_success() || status == StatusCode::MULTI_STATUS {
    Ok((status, text))
}
```

`status.is_success()` is `2xx`, which includes 200, 201, 204, 207. We accept 200 OK responses as if they were 207. The parsers are forgiving - they'll just return empty results from a 200/HTML or 200/empty response. So no crash, but no data either. Same silent-failure mode as the empty-multistatus case above. A 200 OK on a PROPFIND is RFC-illegal; some servers (a misconfigured nginx in front of a CalDAV server) terminate before reaching the backend and return 200 + a default page. Worth a content-type check.

#### 19. CDATA decode `e.decode()` errors silently lose data
**File:** `crates/core/src/caldav/parse.rs:447-450`, `:535-538`, `:598-601`, `:655-658`

```rust
Ok(Event::CData(ref e)) => {
    if let Ok(text) = e.decode() {
        buf.push_str(&text);
    }
}
```

A CDATA decode failure (invalid UTF-8) silently drops the entire CDATA block. For `<calendar-data>` this means an entire event's iCal payload is silently dropped from the multiget result, the URI is missing from `Vec<(uri, ical)>`, and the sync skips the event entirely without surfacing an error. The Text-event arm has the same problem (`unescape` failing -> silent drop).

A WARN log on decode failure would let support diagnose mysterious "event missing after sync" reports.

#### 20. `join_calendar_path` strips query strings on calendar URLs
**File:** `crates/calendar/src/caldav/mod.rs:413-424`

```rust
reqwest::Url::parse(&base_with_slash).map_err(...)?.join(segment).map(...)
```

If `calendar_remote_id` came back from the server with a query string (`/cal/?charset=utf-8`), `Url::join` resolves the segment relative to it correctly, but `reqwest::Url`-parsed segment `xxx.ics` will produce `/cal/xxx.ics` (query string on the base is ignored in joins per RFC 3986 Section 5.3). This is correct per spec but could lose server-routing query parameters that some shared-hosting CalDAV servers require. Edge case, low probability.

### Low-severity / observations

#### 21. `parse_propfind_calendars` doesn't accumulate across multiple `<propstat>` blocks
If a server splits the prop response into two `<propstat>` blocks (one for `displayname` returning 200, one for `getctag` returning 200, with separate prop wrappers), the second one *should* still register because each `<prop>` block we encounter overwrites only the fields it contains and we save into `current_*` strings that were initialized at `<response>`. Looks correct on second read - no actual bug, but the lack of propstat/status awareness (finding 1) is the underlying gap.

#### 22. ETag colon/slash content
`parse_propfind_events` preserves the entire quoted ETag verbatim including embedded colons and slashes (`"abc/def:1"`). RFC 7232 allows these as long as the quote is preserved. `normalize_if_match_etag` doesn't try to re-parse, so this round-trips fine.

#### 23. HTML entities in display names (e.g. `Personal &amp; Family`)
`unescape()` on the Text event handles this. `&#x2014;` (em dash) decodes too. Good.

#### 24. `calendar-color` accepts `#0000FFFF` (ARGB) verbatim
`parse_propfind_calendars` reads the color as-is. Whether the UI renders 8-hex-digit colors correctly is outside this review; flagging it because Apple emits ARGB while most other servers emit RGB hex.

#### 25. `parse_icalendar` swallows `Entry::InvalidLine(_)` silently
**File:** `crates/core/src/caldav/parse.rs:84`

`continue;` is right behavior, but logging the invalid line at debug level would help debug what real servers actually emit. As-is, an entire event with one bad property is parsed minus that property, with no record.

### Failure modes summary (graceful vs. poisoning)

| Scenario | Graceful? | Path |
|---|---|---|
| Empty multistatus | Yes (returns empty Vec) | `parse_propfind_calendars` |
| Event with no UID | Partially (uid=None propagates, may dedup-collide) | `parse.rs:98` -> `mod.rs:393` |
| Per-resource 404 in multiget without calendar-data | Yes (skipped by `current_ical.is_empty()`) | `parse_multiget_report:677` |
| Per-resource 500 in multiget with stale calendar-data | **No (silently writes stale data)** | finding 2 |
| ETag with non-ASCII bytes | Yes-ish (drops ETag, falls back to GET) | finding 4 |
| ETag in `W/foo` legacy form | **No (412 loop until manual full-resync)** | finding 5 |
| Discovery returns HTML 200 | Yes (logs failure) | finding 13 |
| Principal/home cross-host | **No (silently lists wrong account or 404s)** | finding 15 |
| Principal points to 404 home-set | **No (stuck loop, persisted principal not cleared)** | finding 16 |
| VEVENT with multiple DTSTART | Partially (calcard picks one, no log) | finding 9 |
| `<calendar-home-set>` with multiple `<href>` (delegation) | **No (only first href seen)** | finding 3 |
| LF-only line endings inside iCal | Depends on calcard | finding 12 |
| Sub-collection masquerading as event resource | Partially (triggers REPORT 403, batch fails) | finding 7 |

The "No" rows above are the ones I'd prioritize. **Findings 1/2 (propstat/status) and 3 (extract_href_property) and 15 (resolve against base_url not principal) are the three most important** - they each cause silent wrong-data outcomes against real-world server configs that exist today.

---

## Round 2 / Reviewer 3 - Real-world CalDAV server compatibility (Outside)

### Findings

- **High: empty or non-multistatus PROPFIND results can wipe the local cache for a calendar.** `propfind_raw` accepts any 2xx response, not just 207 Multi-Status (`client.rs:503` / `crates/core/src/caldav/client.rs:503`), and `parse_propfind_events` returns an empty vector on empty XML, HTML login bodies, malformed XML, or 207 with no `<response>` children (`parse.rs:569` / `crates/core/src/caldav/parse.rs:569`, `parse.rs:574` / `crates/core/src/caldav/parse.rs:574`). The sync path interprets an empty remote set as "all stored events were deleted" (`sync.rs:142` / `crates/core/src/caldav/sync.rs:142`, `sync.rs:189` / `crates/core/src/caldav/sync.rs:189`), so a server returning 200 OK with an empty/login/error body for PROPFIND can make all local events for that calendar disappear.

- **High: REPORT hrefs are not normalized back to the listing href identity.** `list_events` resolves event hrefs to absolute URLs (`client.rs:251` / `crates/core/src/caldav/client.rs:251`), but `fetch_events` returns `parse_multiget_report` results as-is (`client.rs:299` / `crates/core/src/caldav/client.rs:299`); `parse_multiget_report` stores the server's raw `<href>` (`parse.rs:664` / `crates/core/src/caldav/parse.rs:664`, `parse.rs:679` / `crates/core/src/caldav/parse.rs:679`). Many servers return `/cal/user/event.ics` in multiget even when the request used `https://host/cal/user/event.ics`; then the ETag lookup misses (`sync.rs:156` / `crates/core/src/caldav/sync.rs:156`, `sync.rs:168` / `crates/core/src/caldav/sync.rs:168`), the event is stored under the path href, and the next sync may classify that path key as deleted.

- **High: hrefs are inserted into calendar-multiget XML without XML escaping.** `fetch_events` writes `<D:href>{uri}</D:href>` directly (`client.rs:279` / `crates/core/src/caldav/client.rs:279`). If a server lists a valid href containing an escaped query string such as `/event.ics?a=1&amp;b=2`, the parser unescapes it to `&`, and the next REPORT body is malformed XML. User-visible result: the whole multiget batch can fail or return nothing, leaving events missing/stale.

- **High: multistatus/propstat status codes are ignored.** The XML parsers never inspect `<status>`; `parse_propfind_events` only checks href, etag, and resource shape (`parse.rs:540` / `crates/core/src/caldav/parse.rs:540`, `parse.rs:556` / `crates/core/src/caldav/parse.rs:556`). In mixed 207 responses with some 200, 403, and 404 entries, inaccessible or missing resources are treated as absent from the remote set, which can delete local copies; multiget partial failures are silently skipped and leave stale or missing events.

- **High: recurring CalDAV resources with multiple VEVENTs collapse incorrectly.** `parse_icalendar` extracts every top-level VEVENT (`parse.rs:77` / `crates/core/src/caldav/parse.rs:77`), but the sync key is only `caldav:{UID}` (`sync.rs:212` / `crates/core/src/caldav/sync.rs:212`, `sync.rs:289` / `crates/core/src/caldav/sync.rs:289`). Real CalDAV resources can contain a master VEVENT plus override VEVENTs with the same UID and different RECURRENCE-ID; those will conflict on `(account_id, google_event_id)` and overwrite each other, so users can see only one instance or the exception replacing the master.

- **Medium: relative event hrefs break when a calendar collection href lacks a trailing slash.** `resolve_url_against` uses `Url::join` directly (`client.rs:566` / `crates/core/src/caldav/client.rs:566`). If a server lists the calendar as `https://host/cal/user/work` and event hrefs as `event.ics`, URL joining resolves to `https://host/cal/user/event.ics`, not `.../work/event.ics`. Some real DAV servers are loose about trailing slashes, so this can fetch/report against the wrong collection.

- **Medium: unknown TZIDs silently fall back to the user's local timezone.** For `TZID=Customized Time Zone 1` or `TZID=tzone://Microsoft/Custom/...` without a resolvable VTIMEZONE, `extract_datetime` falls through to local floating-time interpretation (`parse.rs:269` / `crates/core/src/caldav/parse.rs:269`, `parse.rs:301` / `crates/core/src/caldav/parse.rs:301`). That is correct for true floating times, but wrong for an explicit unknown TZID; events shift by the difference between the intended server zone and the user's machine zone, with no warning.

- **Medium: malformed iCalendar often becomes epoch events, not parse failures.** `parse_icalendar` ignores `InvalidLine` and returns `Ok(events)` unconditionally (`parse.rs:83` / `crates/core/src/caldav/parse.rs:83`, `parse.rs:89` / `crates/core/src/caldav/parse.rs:89`). If DTSTART/DTEND parse as text or are missing, `extract_datetime` returns None (`parse.rs:251` / `crates/core/src/caldav/parse.rs:251`), and sync stores `start_time` as 0 (`sync.rs:234` / `crates/core/src/caldav/sync.rs:234`); users can get broken events at Unix epoch instead of a skipped/logged bad item.

- **Medium: duplicate DTSTART/DTEND is silently first-wins.** `extract_vevent` reads a single `component.property(...)` for DTSTART and DTEND (`parse.rs:121` / `crates/core/src/caldav/parse.rs:121`). A buggy server emitting two DTSTART lines will not be rejected or logged; whichever calcard exposes first becomes the event time, so the UI can show the wrong occurrence.

- **Medium: redirected discovery resolves relative hrefs against the original base URL, not the final redirect URL.** `discover_principal` resolves discovered hrefs through `self.resolve_url` (`client.rs:175` / `crates/core/src/caldav/client.rs:175`, `client.rs:192` / `crates/core/src/caldav/client.rs:192`), and `resolve_url` always uses `self.base_url` (`client.rs:559` / `crates/core/src/caldav/client.rs:559`). If `/.well-known/caldav` redirects to another host/path and returns relative principal/home hrefs, subsequent PROPFINDs can go to the wrong host or path.

- **Medium: stale persisted discovery URLs have no rediscovery fallback.** `build_client_from_config` replays persisted principal/home URLs (`mod.rs:294` / `crates/calendar/src/caldav/mod.rs:294`, `mod.rs:297` / `crates/calendar/src/caldav/mod.rs:297`); if home is present, discovery is skipped entirely (`mod.rs:300` / `crates/calendar/src/caldav/mod.rs:300`). If a server migrates principal/home URLs or a cached principal now returns 404, sync/listing fails rather than clearing discovery state and retrying base/well-known discovery.

- **Low/Medium: weak ETags are sent back in If-Match.** ETags are preserved, and `normalize_if_match_etag` explicitly preserves `W/"..."` (`client.rs:708` / `crates/core/src/caldav/client.rs:708`); PUT/DELETE then send that value (`client.rs:425` / `crates/core/src/caldav/client.rs:425`, `client.rs:458` / `crates/core/src/caldav/client.rs:458`). Servers that enforce RFC strong comparison for If-Match can reject every update/delete with 412 Precondition Failed when they themselves emitted weak ETags.

- **Low: discovery href extraction does not handle CDATA.** The main XML parsers handle `Event::CData`, but `extract_href_property` only consumes `Event::Text` (`client.rs:657` / `crates/core/src/caldav/client.rs:657`). A CDATA-wrapped current-user-principal or calendar-home-set href would make discovery fail even though CDATA-wrapped element content is otherwise tolerated.

- **Low: event content-type detection is case-sensitive.** `is_icalendar_resource` checks `content_type.contains("text/calendar")` and `href.ends_with(".ics")` case-sensitively (`parse.rs:711` / `crates/core/src/caldav/parse.rs:711`). A server returning `TEXT/CALENDAR` for UUID-style event hrefs can have every event skipped, which then feeds into the deletion behavior above.

- **Low: read-only calendars are exposed as editable.** Calendar listing hard-codes `can_edit: true` because privileges are not parsed (`mod.rs:65` / `crates/calendar/src/caldav/mod.rs:65`); sync also upserts CalDAV calendars as editable (`sync.rs:53` / `crates/core/src/caldav/sync.rs:53`, `sync.rs:64` / `crates/core/src/caldav/sync.rs:64`). Shared/read-only calendars from iCloud/Fastmail/SOGo will show edit affordances and then fail on PUT/DELETE.

### Compatibility Notes

Namespace prefixes/default namespaces and display-name entities are handled well by the current `local_name` plus XML unescape approach. CDATA for calendar-data is also handled. The weakest graceful-failure boundary is XML-level parsing: unexpected bodies usually become empty vectors with no log, and in event listing that can poison sync by deleting local cache entries.
