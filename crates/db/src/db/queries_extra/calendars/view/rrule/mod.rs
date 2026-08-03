use std::collections::HashSet;

use jiff::civil;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Span, Timestamp};

use super::CalendarViewEvent;

/// Hard cap on COUNT to bound allocation. Real-world recurring events stay
/// well under this; a remote server emitting `COUNT=4294967295` cannot pin
/// us to a multi-GB Vec.
const RRULE_MAX_COUNT: usize = 10_000;

/// Hard cap on iteration steps inside any single expander, regardless of how
/// many instances actually get produced. Defends against the "BYDAY filter
/// matches nothing" / "BYMONTHDAY=31 in only February" infinite-loop pattern
/// where `out.len()` never grows. Set well above any legitimate workload:
/// ~30 years of daily checks. If a real RRULE legitimately needs more, COUNT
/// or UNTIL will terminate first.
const RRULE_MAX_STEPS: usize = 12_000;

/// Pick the per-expander instance cap.
///
/// When `rule.count` is present, that's the explicit upper bound (already
/// clamped to `RRULE_MAX_COUNT` at parse time). When `rule.until` is
/// present without COUNT, we let the expander run up to `RRULE_MAX_COUNT`:
/// the time bound (window_end / UNTIL) terminates the loop, and a per-
/// expander default of 800 silently truncated long-UNTIL rules far below
/// what the user asked for - e.g. `FREQ=YEARLY;UNTIL=22000101T000000Z`
/// from 2026 emitted 60 instances and stopped 114 years short of the
/// requested UNTIL. (Round 3 #2.)
///
/// When neither COUNT nor UNTIL is set, we fall back to the
/// `default_unbounded` cap. The 2-year synthesised window
/// (`two_year_window_end`) is the time bound there, but dense BY-rules
/// like `FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR` (5/wk × 104wk = 520
/// emissions) would be truncated by a smaller default - the standup
/// vanishes 17 months in. The default below is chosen to cover the
/// densest realistic 2-year span. (Round 3 #4.)
fn instance_cap(rule: &Rrule, default_unbounded: usize) -> usize {
    if let Some(n) = rule.count {
        return n.max(1);
    }
    if rule.until.is_some() {
        return RRULE_MAX_COUNT;
    }
    default_unbounded.clamp(1, RRULE_MAX_COUNT)
}

/// The wall-clock zone an event recurs in. RFC 5545 § 3.3.10: recurring
/// events keep their wall-clock time across DST transitions and across
/// instances - 09:00 every day means 09:00 *in that zone*, not 09:00 in
/// `chrono::Local` (the previous behavior, which silently shifted every
/// recurring event by the user's UTC offset relative to its source zone).
///
/// `Iana` covers any TZID we can resolve through jiff's bundled tzdb. `Local`
/// covers floating events (no TZID stored) and any TZID we couldn't parse -
/// notably Windows zone names like "Pacific Standard Time" that the parse layer
/// resolves at sync time but stores in `event.timezone` as-is. Threading
/// calcard's resolver into expansion would honor those, but that pulls
/// calcard into the db crate; the warn-and-fall-back path keeps the previous
/// behavior for the unresolved tail without infecting the dep graph.
///
/// The two arms are NOT redundant now that both hold the same jiff `TimeZone`
/// type: the discriminant is what tells `canonical_recurrence_slot` whether to
/// emit a `TZID=` suffix. A floating event that happens to resolve to the
/// host's zone must still serialize without a TZID, so the distinction is
/// semantic and survives the migration.
///
/// `Clone` rather than `Copy`: jiff's `TimeZone` is a handle to shared tzdb
/// data, so it is cheap to clone but not a bare value. Callers take it by
/// reference.
#[derive(Debug, Clone)]
enum RecurrenceTz {
    Iana(TimeZone),
    Local,
}

impl RecurrenceTz {
    fn from_event_timezone(event_tz: Option<&str>) -> Self {
        let Some(raw) = event_tz else {
            return Self::Local;
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Self::Local;
        }
        match TimeZone::get(trimmed) {
            Ok(tz) => Self::Iana(tz),
            Err(_) => {
                log::debug!(
                    "RRULE expansion: event.timezone={trimmed:?} did not parse as IANA; \
                     falling back to local zone"
                );
                Self::Local
            }
        }
    }

    /// The zone to compute in. `Local` resolves the host zone at call time;
    /// jiff falls back to UTC (with its own warning) on a host with no
    /// discoverable zone, which matches chrono's previous behavior here.
    fn zone(&self) -> TimeZone {
        match self {
            Self::Iana(tz) => tz.clone(),
            Self::Local => TimeZone::system(),
        }
    }

    fn naive(&self, timestamp: i64) -> Option<civil::DateTime> {
        Timestamp::from_second(timestamp)
            .ok()
            .map(|instant| instant.to_zoned(self.zone()).datetime())
    }

    fn resolve(&self, naive: civil::DateTime) -> Option<i64> {
        crate::db::time::resolve_local_to_timestamp(naive, &self.zone())
    }
}

/// jiff's civil types use narrow integer widths (`i16` year, `i8` month and
/// day) while this module's year/month cursor arithmetic is `i32`/`u32`
/// throughout. Convert at the boundary rather than rewriting the arithmetic:
/// the widening direction is always lossless, and the cursor logic below is
/// heavily reviewed against dateutil, so leaving it byte-identical keeps the
/// migration honest.
fn ymd_of(dt: civil::DateTime) -> (i32, u32, u32) {
    (
        i32::from(dt.year()),
        u32::from(dt.month().unsigned_abs()),
        u32::from(dt.day().unsigned_abs()),
    )
}

/// Narrowing inverse of [`ymd_of`]. `None` when the values fall outside jiff's
/// representable civil range - which every caller already treats as "skip this
/// candidate", the same way the chrono `from_ymd_opt` it replaces did.
fn civil_date(year: i32, month: u32, day: u32) -> Option<civil::Date> {
    civil::Date::new(
        i16::try_from(year).ok()?,
        i8::try_from(month).ok()?,
        i8::try_from(day).ok()?,
    )
    .ok()
}

/// Weekday offset from Monday, in the `i64` the week arithmetic below wants.
/// jiff spells this `to_monday_zero_offset`; chrono spelled it
/// `num_days_from_monday`. Same value, different name.
fn days_from_monday(day: civil::Weekday) -> i64 {
    i64::from(day.to_monday_zero_offset())
}

/// Expand a recurring event into concrete instances based on its RRULE.
///
/// Supports a useful subset of RFC 5545 RRULE:
/// - FREQ: DAILY, WEEKLY, MONTHLY, YEARLY
/// - INTERVAL, COUNT, UNTIL
/// - BYDAY (e.g. `BYDAY=MO,WE,FR` on FREQ=WEEKLY/DAILY)
/// - BYMONTHDAY (FREQ=MONTHLY, picks specific day-of-month)
/// - BYMONTH (FREQ=YEARLY, picks specific month)
///
/// Generates instances within a ~2 year window from the event's original
/// start time. EXDATE handling is not yet wired (EXDATE is stored on a
/// separate iCal property, not part of the RRULE string).
///
/// Convenience wrapper for tests that don't need to subtract override
/// slots; routes through `expand_recurrence_with_overrides` with an
/// empty set. Production callers go through the `_with_overrides` form
/// directly so the override set the load-path built actually flows in.
#[cfg(test)]
fn expand_recurrence(event: &CalendarViewEvent, rrule_str: &str) -> Vec<CalendarViewEvent> {
    expand_recurrence_with_overrides(event, rrule_str, &HashSet::new())
}

/// Expand a recurring event into concrete instances, subtracting slots
/// already claimed by RECURRENCE-ID override rows.
///
/// `overrides` carries the canonical wall-clock RECURRENCE-ID strings of
/// the override rows for this UID. For each candidate timestamp produced
/// by the master expansion, we re-canonicalise the candidate using the
/// master's `event.timezone` and `event.all_day` and drop it when the
/// resulting string sits in the override set. The override row itself
/// already exists as a separate non-recurring row (its own start_time
/// drives display), so without this subtraction the user sees both the
/// untouched master slot AND the moved override - the phantom-duplication
/// regression flagged as #1 in the calendar review findings.
pub(super) fn expand_recurrence_with_overrides(
    event: &CalendarViewEvent,
    rrule_str: &str,
    overrides: &HashSet<String>,
) -> Vec<CalendarViewEvent> {
    // View path: no explicit horizon, so an unbounded rule falls back to the
    // DTSTART-anchored 2-year synthesised window (`two_year_window_end`).
    expand_recurrence_windowed(event, rrule_str, overrides, None)
}

/// Core expansion, parameterised by an optional expansion horizon for
/// unbounded rules.
///
/// `expansion_horizon` only affects the `(no UNTIL, no COUNT)` case: when
/// `Some(h)`, an unbounded rule expands up to `h` instead of the
/// DTSTART-anchored `two_year_window_end`. The windowed-deletion reconcile
/// (`recurring_master_intersects_window`) passes the active window's end so a
/// master whose DTSTART sits far before the window still expands INTO it,
/// rather than stopping two years past a long-ago DTSTART. UNTIL- and
/// COUNT-bounded rules ignore the horizon (their own bound terminates them).
fn expand_recurrence_windowed(
    event: &CalendarViewEvent,
    rrule_str: &str,
    overrides: &HashSet<String>,
    expansion_horizon: Option<i64>,
) -> Vec<CalendarViewEvent> {
    let rule = parse_rrule(rrule_str);
    let tz = RecurrenceTz::from_event_timezone(event.timezone.as_deref());
    let Some(freq) = Freq::parse(&rule.freq) else {
        // FREQ is missing or unrecognized. We fall back to a single instance
        // (the master event) so the operator at least sees the event on the
        // calendar; logging here surfaces the malformed rule.
        //
        // Logged at debug rather than warn because this branch fires from
        // every view render via `load_calendar_events_for_view_sync`. A
        // calendar with N malformed RRULEs (and Outlook bridges + Apple
        // Calendar exports do produce these) would otherwise emit N WARN
        // lines per refresh, drowning out actual operational signal. The
        // sync-time parse pass already records the same VEVENTs, so the
        // signal isn't lost - just not repeated. (Round 3 #44.)
        log::debug!(
            "RRULE has unrecognized or missing FREQ; emitting only master instance: {rrule_str}"
        );
        return vec![event.clone()];
    };
    if !rule.unsupported_parts.is_empty() {
        // Recognized but unimplemented BY-rules (e.g. BYSETPOS, BYWEEKNO).
        // Falling through to the expanders would produce a wildly wrong
        // expansion (~22 days/month for `BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1`).
        // Emit only the master instance so the user sees the event in the
        // right place without the noise. Debug-level for the same
        // render-loop reason as the FREQ branch above.
        log::debug!(
            "RRULE uses unsupported parts ({:?}); emitting only master instance: {rrule_str}",
            rule.unsupported_parts
        );
        return vec![event.clone()];
    }
    if matches!(freq, Freq::Yearly)
        && rule.bymonth.is_empty()
        && rule.byday.iter().any(|b| b.ordinal.is_some())
    {
        // YEARLY + ordinal BYDAY without BYMONTH means "the n-th weekday of
        // the year" (RFC 5545 § 3.3.10). The expander only walks per-month
        // ordinals (`nth_weekday_in_month`), so a rule like
        // `FREQ=YEARLY;BYDAY=20MO` would silently emit zero instances - no
        // single month has 20 Mondays. Emit the master instance and log;
        // the year-scope ordinal walk is a real feature, not a defensive
        // tweak, and is left as a follow-up. Debug-level (see above).
        log::debug!(
            "RRULE FREQ=YEARLY with ordinal BYDAY and no BYMONTH would require a year-scope ordinal walk; emitting only master instance: {rrule_str}"
        );
        return vec![event.clone()];
    }

    // Wall-clock duration in the event's zone, not raw UTC seconds. Captures
    // the user's intent ("the meeting is 1 hour long") rather than the UTC
    // span of the master instance ("the meeting is 0 or 2 hours long if it
    // happens to span the DST transition"). Each recurring instance then
    // resolves end_time = start_naive + wall_duration in the event's zone,
    // so an all-day event ending at midnight stays at midnight on every
    // instance regardless of DST gaps - and a timed event stays the same
    // wall-clock length whether or not the master spanned DST.
    //
    // All-day events get a separate path: the parse layer already anchored
    // end_time to `start + days*86400`, so when the master spans the
    // spring-forward boundary `end_naive` lands at `01:00` the day after
    // `end_date()`. A naive `end_naive - start_naive` then gives 25 hours,
    // which propagated to every subsequent recurring instance shows them
    // ending at 01:00 instead of midnight. Compute the duration from the
    // date delta directly so 1 calendar day stays 1 calendar day across
    // DST. Symmetric for fall-back (avoids 23h drift). (Round 3 #22.)
    let raw_duration = event.end_time - event.start_time;
    let wall_duration = if event.all_day {
        match (tz.naive(event.start_time), tz.naive(event.end_time)) {
            (Some(s), Some(e)) => {
                let mut days = (e.date() - s.date()).get_days();
                // If end_naive landed before midnight (DST fall-back, where
                // start+86400 sits at 23:00 the same day), the date delta
                // would underreport by one. Round up so a 1-day event stays
                // 1 day. The condition is: end_naive's clock sits past
                // midnight relative to start_naive (i.e. raw seconds floor-
                // divided by 86400 is at least one).
                if days == 0 && raw_duration > 0 {
                    days = 1;
                }
                // Civil arithmetic, so a "day" here is a literal 24h of wall
                // clock - the whole point of computing from the date delta is
                // that the zone-aware re-resolution happens later, in
                // `end_time_for_instance`.
                SignedDuration::from_hours(24 * i64::from(days.max(0)))
            }
            _ => SignedDuration::from_secs(raw_duration),
        }
    } else {
        match (tz.naive(event.start_time), tz.naive(event.end_time)) {
            (Some(s), Some(e)) => e.duration_since(s),
            _ => SignedDuration::from_secs(raw_duration),
        }
    };

    // Outer cap: lets explicit COUNT through, lets UNTIL-bounded rules run
    // to RRULE_MAX_COUNT (the time bound terminates), and falls back to a
    // 2-year-window-sized default when neither is set. See `instance_cap`
    // for the rationale and review-finding cross-references.
    let max_instances = instance_cap(&rule, 800);
    if rule.count.is_some() && rule.until.is_some() {
        // RFC 5545 § 3.3.10: COUNT and UNTIL are mutually exclusive. Some
        // emitters send both anyway; we apply BOTH as upper bounds (the
        // intersection is always a subset of either, so the result stays
        // within the more permissive interpretation either rule alone would
        // permit). Logged so an operator can spot misbehaving servers.
        log::debug!(
            "RRULE has both COUNT and UNTIL (mutually exclusive per RFC 5545); applying both as bounds"
        );
    }
    // Window bounds:
    // - UNTIL set: hard bound, applies regardless of COUNT.
    // - COUNT set without UNTIL: no time bound; COUNT alone limits output.
    // - Neither: synthesize a 2-year fallback window so an unbounded rule
    //   doesn't run away.
    //
    // Resolve UNTIL through the event's recurrence zone. Floating and
    // DATE-only UNTIL values were stored raw at parse time so the anchor
    // matches the event's zone rather than the host's local zone; UTC
    // UNTIL values are pre-resolved and unaffected. (Round 3 #7, #8.)
    let until_ts = rule.until.and_then(|u| u.resolve(&tz));
    let window_end = match (until_ts, rule.count) {
        (Some(until), _) => until,
        (None, Some(_)) => i64::MAX,
        (None, None) => {
            expansion_horizon.unwrap_or_else(|| two_year_window_end(event.start_time, &tz))
        }
    };

    let mut instances = Vec::with_capacity(max_instances);

    let candidate_starts = match freq {
        Freq::Daily => expand_daily(event.start_time, &rule, &tz),
        Freq::Weekly => expand_weekly(event.start_time, &rule, &tz),
        Freq::Monthly => expand_monthly(event.start_time, &rule, &tz),
        Freq::Yearly => expand_yearly(event.start_time, &rule, &tz),
    };

    for (idx, start) in candidate_starts.into_iter().enumerate() {
        if start > window_end {
            break;
        }
        if instances.len() >= max_instances {
            break;
        }
        // Phantom-override dedup: when a stored row carries a
        // RECURRENCE-ID matching this candidate's wall-clock slot, the
        // override row will render in its place (its own start_time drives
        // the displayed time). Emitting the master candidate too produces
        // two events for the same slot - the user sees the original *and*
        // the moved instance.
        if !overrides.is_empty() {
            let canonical = canonical_recurrence_slot(start, &tz, event.all_day);
            if overrides.contains(&canonical) {
                continue;
            }
        }
        let mut instance = event.clone();
        if idx > 0 {
            instance.id = format!("{}__recur_{idx}", event.id);
        }
        instance.start_time = start;
        instance.end_time = end_time_for_instance(start, wall_duration, &tz, raw_duration);
        // Recurring instances inherit the master's identity; never their
        // own override key. Stash uid alongside so that future code paths
        // (e.g. clicking through to an instance) keep the master link.
        instance.recurrence_id_canonical = None;
        instances.push(instance);
    }

    // Note: when the RRULE produces zero instances (e.g. UNTIL is in the
    // past, or every BYxxx filter rejects every visited candidate), we
    // return an empty Vec rather than synthesizing the original event.
    // The previous fallback hid genuine "this rule expires in the past"
    // states from the caller.
    instances
}

/// Format a master expansion candidate the same way an override's
/// RECURRENCE-ID is canonicalised at parse time, so the two strings
/// compare equal and the load-path can subtract phantoms.
///
/// The format must match `parse::extract_recurrence_id_canonical` exactly:
///   - all-day  -> `YYYYMMDD`
///   - zoned    -> `YYYYMMDDTHHMMSS;TZID=<id>`
///   - floating -> `YYYYMMDDTHHMMSS`
///
/// We intentionally don't emit the `Z` form here: master expansion
/// candidates were resolved through the master's TZID context (whatever
/// `event.timezone` was set to). When the iCal source carried a UTC
/// DTSTART (no TZID), `event.timezone` is None and the master walks in
/// `chrono::Local` - producing a wall-clock candidate that lines up with
/// a floating-form override, which is the convention sane emitters use
/// across master/override pairs anyway. UTC-form override + non-UTC
/// master is a malformed feed; we don't try to dedup that combination.
fn canonical_recurrence_slot(timestamp: i64, tz: &RecurrenceTz, all_day: bool) -> String {
    let Some(naive) = tz.naive(timestamp) else {
        // Resolution failed (timestamp out of chrono's range). Return a
        // sentinel that won't collide with any real iCal canonical form;
        // the worst case is one missed dedup on a pathological event.
        return format!("__unresolvable_slot_{timestamp}");
    };
    if all_day {
        return naive.strftime("%Y%m%d").to_string();
    }
    let body = naive.strftime("%Y%m%dT%H%M%S").to_string();
    match tz {
        // `iana_name` is `Option` because a jiff `TimeZone` can also be a
        // fixed offset or a POSIX rule. This arm is only ever constructed
        // from `TimeZone::get`, which always yields a named zone, so the
        // fallback is unreachable in practice - but emitting the body
        // without a TZID beats emitting `TZID=` with nothing after it,
        // which would never match a real override's canonical form.
        RecurrenceTz::Iana(zone) => match zone.iana_name() {
            Some(name) => format!("{body};TZID={name}"),
            None => body,
        },
        RecurrenceTz::Local => body,
    }
}

/// Compute end_time for a recurring instance by walking `wall_duration` in
/// the event's wall-clock zone, falling back to raw-seconds arithmetic if
/// the wall-clock walk overflows or hits a non-resolvable zone state. The
/// fallback is the previous behavior; the new path prevents an all-day
/// recurring event whose master spans DST from inheriting a 23h/47h
/// duration on every subsequent instance.
fn end_time_for_instance(
    start: i64,
    wall_duration: SignedDuration,
    tz: &RecurrenceTz,
    raw_duration: i64,
) -> i64 {
    tz.naive(start)
        .and_then(|n| n.checked_add(wall_duration).ok())
        .and_then(|n| tz.resolve(n))
        .unwrap_or(start + raw_duration)
}

/// Compute the 2-year window-end timestamp using calendar arithmetic so
/// leap years are accounted for. Falls back to a 730-day approximation if
/// the start timestamp is somehow out of chrono's representable range.
fn two_year_window_end(start: i64, tz: &RecurrenceTz) -> i64 {
    tz.naive(start)
        .and_then(|n| n.with().year(n.year() + 2).build().ok())
        .and_then(|n| tz.resolve(n))
        .unwrap_or(start + 730 * 86400)
}

/// Does the recurrence set of `(event, rrule_str)` produce at least one
/// occurrence intersecting `[window_start, window_end)` (Unix seconds)?
///
/// This is the reconcile-side occurrence-intersection test. It differs from a
/// plain `expand_recurrence_with_overrides` + clip in two ways that matter when
/// the master's `DTSTART` sits far BEFORE the window (the common case for a
/// long-running unbounded series - a standup running since 2024 seen against a
/// 2027 window):
///
/// 1. It expands with the reconcile `window_end` as the horizon, so an
///    unbounded rule is NOT truncated at the DTSTART-anchored 2-year synthetic
///    window (`two_year_window_end`), which for a far-past DTSTART never
///    reaches the window at all.
/// 2. For day-cadence rules (DAILY/WEEKLY) it re-anchors DTSTART FORWARD by a
///    whole number of base-period steps onto the same recurrence lattice, so
///    the per-expander 800-instance count cap (~2.2y of DAILY, ~15y of WEEKLY)
///    cannot exhaust before the walk reaches the window. Advancing by whole
///    `interval`-day / `interval`-week steps lands on a genuine lattice point,
///    so the expansion from the new anchor is exactly the tail of the original
///    recurrence set at or after the new anchor - the intersection answer is
///    preserved. MONTHLY/YEARLY are not re-anchored: their 800-instance caps
///    span ~66y / ~800y, so no realistic DTSTART exhausts them, and the horizon
///    override alone suffices.
///
/// An empty overrides set is correct: the reconcile only asks about the
/// master's own expansion. A malformed / unsupported rule falls back to the
/// single master instance (see `expand_recurrence_windowed`), kept only if the
/// master interval itself overlaps - the conservative "preserve when uncertain"
/// outcome.
pub(super) fn master_intersects_window(
    event: &CalendarViewEvent,
    rrule_str: &str,
    window_start: i64,
    window_end: i64,
) -> bool {
    let rule = parse_rrule(rrule_str);
    let tz = RecurrenceTz::from_event_timezone(event.timezone.as_deref());
    let anchor = match Freq::parse(&rule.freq) {
        Some(freq @ (Freq::Daily | Freq::Weekly)) => {
            reanchor_day_cadence(event.start_time, &rule, freq, &tz, window_start)
        }
        // MONTHLY/YEARLY: count cap never bites within a realistic window; the
        // horizon override below is sufficient. Malformed FREQ: leave as-is,
        // the fallback path keeps the single master only if it overlaps.
        _ => event.start_time,
    };
    let master = if anchor == event.start_time {
        event.clone()
    } else {
        let mut m = event.clone();
        let span = event.end_time - event.start_time;
        m.start_time = anchor;
        m.end_time = anchor + span;
        m
    };
    expand_recurrence_windowed(&master, rrule_str, &HashSet::new(), Some(window_end))
        .into_iter()
        .any(|inst| inst.start_time < window_end && inst.end_time > window_start)
}

/// Re-anchor a DAILY/WEEKLY `DTSTART` forward by a whole number of base-period
/// steps so it lands at or just before `window_start` while staying on the
/// original recurrence lattice. Returns `start` unchanged when the window opens
/// at or before `DTSTART` (nothing to skip) or when zone arithmetic overflows.
fn reanchor_day_cadence(
    start: i64,
    rule: &Rrule,
    freq: Freq,
    tz: &RecurrenceTz,
    window_start: i64,
) -> i64 {
    if window_start <= start {
        return start;
    }
    let interval = rule.interval.max(1);
    let step_days = match freq {
        Freq::Weekly => interval * 7,
        _ => interval,
    };
    // Whole steps to jump. Floor-division keeps the new anchor at or before
    // window_start, and the multiple of `step_days` keeps it on the lattice.
    let gap_days = (window_start - start).div_euclid(86_400);
    let steps = gap_days / step_days;
    if steps <= 0 {
        return start;
    }
    add_days_in_zone(start, steps * step_days, tz).unwrap_or(start)
}

/// A single BYDAY entry. The ordinal prefix (e.g. `1MO`, `-1FR`) is captured
/// alongside the bare weekday so `FREQ=MONTHLY;BYDAY=1MO` ("first Monday of
/// the month") and `FREQ=YEARLY;BYDAY=-1SU` ("last Sunday of the year")
/// expand correctly. For DAILY/WEEKLY/UNTIL the ordinal is ignored (RFC 5545
/// § 3.3.10 says it's only meaningful in MONTHLY/YEARLY).
#[derive(Debug, Clone, Copy)]
struct ByDay {
    /// `None` means "every occurrence of `day` in the period", `Some(n)`
    /// means "the n-th occurrence" (negative counts from the end).
    ordinal: Option<i32>,
    day: civil::Weekday,
}

/// Raw UNTIL value from an RRULE, before zone resolution.
///
/// `parse_rrule` runs before `RecurrenceTz` is known (the event's TZID
/// lives on the row, not in the RRULE string), so floating and DATE-only
/// UNTIL values are stored as raw wall-clock data and resolved at expand
/// time against the event's recurrence zone. Without this split, a NY
/// event with `UNTIL=20260315` from a host in Pacific/Auckland used to
/// anchor at Auckland local 23:59:59 instead of NY local - clipping the
/// last day for west-of-source hosts and over-including for east-of-
/// source hosts. Apple/Google anchor in the event zone; we now match.
/// (Round 3 #7, closes #8 since DATE-only UNTIL alongside TZID DTSTART
/// also resolves through the event zone.)
#[derive(Debug, Clone, Copy)]
enum Until {
    /// `YYYYMMDD` form. RFC 5545 § 3.3.10 says DATE-only UNTIL is only
    /// valid alongside floating DTSTART; some Outlook CalDAV bridges emit
    /// it alongside TZID-bearing DTSTART anyway. Either way, resolves at
    /// 23:59:59 in the event's zone.
    Date(civil::Date),
    /// `YYYYMMDDTHHMMSS` form. RFC 5545: floating, only legal when DTSTART
    /// is floating. Resolved in the event's zone at expand time.
    Floating(civil::DateTime),
    /// `YYYYMMDDTHHMMSSZ` form. Already an absolute UTC instant; no
    /// zone-aware resolution needed.
    Utc(i64),
}

impl Until {
    fn resolve(self, tz: &RecurrenceTz) -> Option<i64> {
        match self {
            Self::Date(date) => {
                let dt = date.at(23, 59, 59, 0);
                tz.resolve(dt)
            }
            Self::Floating(dt) => tz.resolve(dt),
            Self::Utc(ts) => Some(ts),
        }
    }
}

/// Parsed pieces of an RRULE string. Unknown parts are ignored silently
/// unless they're in the documented "unsupported but recognized" set
/// (`unsupported_parts`), in which case the rule is treated as malformed
/// rather than silently mis-expanded.
#[derive(Debug, Default)]
struct Rrule {
    freq: String,
    interval: i64,
    count: Option<usize>,
    until: Option<Until>,
    byday: Vec<ByDay>,
    bymonthday: Vec<i32>,
    bymonth: Vec<u32>,
    /// Week-start day. `None` means "use the default" - we treat that as
    /// Monday (RFC 5545 § 3.3.10 default), which matches what most weekly
    /// recurrence views expect. Set explicitly via `WKST=SU` etc.
    ///
    /// Currently consumed only by `expand_weekly`; YEARLY and MONTHLY
    /// ignore it. That's correct only as long as BYWEEKNO stays
    /// unsupported. RFC 5545 § 3.3.10: BYWEEKNO numbers weeks of the year,
    /// where the first week is the one containing the wkst's first
    /// occurrence. Adding BYWEEKNO without plumbing wkst into the YEARLY
    /// expander would silently shift week-1 anchoring by up to 6 days for
    /// any rule that does not opt into the default WKST=MO.
    wkst: Option<civil::Weekday>,
    /// RFC 5545 BY-rules we recognize but don't yet implement. Populated by
    /// `parse_rrule` so `expand_recurrence` can short-circuit instead of
    /// silently producing wrong expansions (e.g. `BYSETPOS=-1` filtering
    /// only the last weekday of the month would otherwise emit ~22 days
    /// per month). Each entry is the bare key name (`"BYSETPOS"` etc).
    unsupported_parts: Vec<&'static str>,
}

#[derive(Debug, Clone, Copy)]
enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl Freq {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "DAILY" => Some(Self::Daily),
            "WEEKLY" => Some(Self::Weekly),
            "MONTHLY" => Some(Self::Monthly),
            "YEARLY" => Some(Self::Yearly),
            _ => None,
        }
    }
}

fn parse_rrule(rrule_str: &str) -> Rrule {
    let body = rrule_str.strip_prefix("RRULE:").unwrap_or(rrule_str);
    let mut out = Rrule {
        interval: 1,
        ..Rrule::default()
    };
    for part in body.split(';') {
        if let Some(val) = part.strip_prefix("FREQ=") {
            out.freq = val.to_string();
        } else if let Some(val) = part.strip_prefix("INTERVAL=") {
            let raw = val.parse::<i64>().unwrap_or(1);
            if raw < 1 {
                log::debug!("RRULE INTERVAL={raw} (RFC 5545 requires >=1); clamping to 1");
            }
            out.interval = raw.max(1);
        } else if let Some(val) = part.strip_prefix("COUNT=") {
            // Clamp untrusted COUNT values to a sane upper bound so a remote
            // server cannot trigger pathological allocation. Anything above
            // RRULE_MAX_COUNT lands at the cap; legitimate recurring events
            // never come close.
            let raw = val.parse::<usize>().ok();
            if let Some(n) = raw
                && n > RRULE_MAX_COUNT
            {
                log::debug!(
                    "RRULE COUNT={n} exceeds RRULE_MAX_COUNT={RRULE_MAX_COUNT}; truncating expansion"
                );
            }
            out.count = raw.map(|n| n.min(RRULE_MAX_COUNT));
        } else if let Some(val) = part.strip_prefix("UNTIL=") {
            out.until = parse_until_date(val);
        } else if let Some(val) = part.strip_prefix("BYDAY=") {
            // reviewed (R3 verified non-issue): tolerant comma split.
            // `BYDAY=`        -> one empty entry, parse_byday("") -> None,
            //                   filter_map drops it, byday ends up `vec![]`
            //                   and expansion proceeds as if BYDAY were
            //                   absent. Strict parsers reject the rule;
            //                   we deliberately don't.
            // `BYDAY=,MO,`    -> empty entries dropped, `Mo` kept.
            out.byday = val.split(',').filter_map(parse_byday).collect();
        } else if let Some(val) = part.strip_prefix("BYMONTHDAY=") {
            let raw_count = val.split(',').count();
            out.bymonthday = val
                .split(',')
                .filter_map(|s| s.trim().parse::<i32>().ok())
                .filter(|d| {
                    let mag = d.unsigned_abs();
                    (1..=31).contains(&mag)
                })
                .collect();
            if out.bymonthday.len() != raw_count {
                log::debug!(
                    "RRULE BYMONTHDAY=`{val}` had {} of {raw_count} entries dropped (RFC 5545: magnitude must be 1..=31)",
                    raw_count - out.bymonthday.len()
                );
            }
        } else if let Some(val) = part.strip_prefix("BYMONTH=") {
            let raw_count = val.split(',').count();
            out.bymonth = val
                .split(',')
                .filter_map(|s| s.trim().parse::<u32>().ok())
                .filter(|m| (1..=12).contains(m))
                .collect();
            if out.bymonth.len() != raw_count {
                log::debug!(
                    "RRULE BYMONTH=`{val}` had {} of {raw_count} entries dropped (RFC 5545: must be 1..=12)",
                    raw_count - out.bymonth.len()
                );
            }
        } else if let Some(val) = part.strip_prefix("WKST=") {
            out.wkst = parse_weekday_code(val.trim());
        } else {
            // Recognize-but-flag the BY-rules we can't honor. Listing them
            // explicitly (rather than treating any unknown key as malformed)
            // keeps the door open to vendor extensions and future-spec keys
            // without breaking compatibility, while still catching the cases
            // that produce the worst silent expansions.
            for unsupported in [
                "BYSETPOS=",
                "BYWEEKNO=",
                "BYYEARDAY=",
                "BYHOUR=",
                "BYMINUTE=",
                "BYSECOND=",
            ] {
                if part.starts_with(unsupported) {
                    let key = &unsupported[..unsupported.len() - 1];
                    if !out.unsupported_parts.contains(&key) {
                        out.unsupported_parts.push(key);
                    }
                    break;
                }
            }
        }
    }
    out
}

/// Parse a bare iCal weekday token (no ordinal prefix). Used for `WKST=`
/// and as a helper for the BYDAY parser.
fn parse_weekday_code(code: &str) -> Option<civil::Weekday> {
    match code {
        "MO" => Some(civil::Weekday::Monday),
        "TU" => Some(civil::Weekday::Tuesday),
        "WE" => Some(civil::Weekday::Wednesday),
        "TH" => Some(civil::Weekday::Thursday),
        "FR" => Some(civil::Weekday::Friday),
        "SA" => Some(civil::Weekday::Saturday),
        "SU" => Some(civil::Weekday::Sunday),
        _ => None,
    }
}

/// Parse a BYDAY entry, including the optional ordinal prefix.
///
/// `MO` -> ordinal=None, day=Mon (every Monday in the period).
/// `1MO` -> ordinal=Some(1), day=Mon (first Monday).
/// `-1FR` -> ordinal=Some(-1), day=Fri (last Friday).
fn parse_byday(spec: &str) -> Option<ByDay> {
    let trimmed = spec.trim();
    let bytes = trimmed.as_bytes();
    let mut idx = 0;
    let sign: i32 = match bytes.first() {
        Some(b'-') => {
            idx += 1;
            -1
        }
        Some(b'+') => {
            idx += 1;
            1
        }
        _ => 1,
    };
    let digit_start = idx;
    while bytes.get(idx).is_some_and(u8::is_ascii_digit) {
        idx += 1;
    }
    let ordinal = if idx > digit_start {
        let n = std::str::from_utf8(&bytes[digit_start..idx])
            .ok()?
            .parse::<i32>()
            .ok()?;
        // RFC 5545 § 3.3.10: BYDAY ordinal magnitude is 1..=53 (or
        // -53..=-1). Out-of-range values produce no instances at expansion
        // time anyway (no month has 99 Mondays), but the rule then bounds
        // out via `RRULE_MAX_STEPS=12_000` after a noticeable amount of
        // wasted work. Reject upfront with a debug log so the operator
        // can attribute the dropped rule.
        if n == 0 {
            return None;
        }
        if n.unsigned_abs() > 53 {
            log::debug!("RRULE BYDAY ordinal {n} out of range (RFC 5545: 1..=53); dropping entry");
            return None;
        }
        Some(sign * n)
    } else {
        None
    };
    let code = std::str::from_utf8(&bytes[idx..]).ok()?;
    parse_weekday_code(code).map(|day| ByDay { ordinal, day })
}

fn expand_daily(start: i64, rule: &Rrule, tz: &RecurrenceTz) -> Vec<i64> {
    // Default unbounded cap matches the 2-year fallback window's worst case
    // for FREQ=DAILY (730 days). UNTIL-bounded rules run to RRULE_MAX_COUNT
    // and let the time bound terminate; see `instance_cap`.
    let cap = instance_cap(rule, 800);
    let mut out = Vec::with_capacity(cap);
    let mut current = start;
    // Hoist the weekday filter out of the loop. The previous shape
    // collected `rule.byday.iter().map(|b| b.day)` per iteration -
    // RRULE_MAX_STEPS=12_000 allocations of a single Vec for every daily
    // expansion, all carrying the same content. (Round 3 #11.)
    let byday_filter: Vec<civil::Weekday> = rule.byday.iter().map(|b| b.day).collect();
    // Step-bounded iteration: a BYDAY filter can reject 6 of every 7
    // candidates, and pathological filters (e.g. `BYDAY=TU` on a daily rule
    // with `INTERVAL=7` starting on Monday) match nothing - without a step
    // cap we spin forever.
    for _ in 0..RRULE_MAX_STEPS {
        if out.len() >= cap {
            break;
        }
        if byday_filter.is_empty() || matches_weekday(current, &byday_filter, tz) {
            out.push(current);
        }
        // Advance in calendar days, not raw seconds, so wall-clock time is
        // preserved across DST transitions. A 09:00 daily event spans the
        // spring-forward gap as 09:00 each day, not 10:00 from the
        // transition forward.
        current =
            add_days_in_zone(current, rule.interval, tz).unwrap_or(current + rule.interval * 86400);
    }
    out
}

// reviewed (R3 verified non-issue): hand-traced against dateutil for
// `FREQ=WEEKLY;BYDAY=MO,WE,FR;INTERVAL=2;COUNT=10` from a Monday DTSTART
// (week 1: Mon/Wed/Fri; week 3: Mon/Wed/Fri; ...) and
// `FREQ=MONTHLY;BYDAY=2WE,-1FR` from a normal-month anchor
// (2026-01-14, 2026-01-30, 2026-02-11, 2026-02-27, ...). Output matched.
fn expand_weekly(start: i64, rule: &Rrule, tz: &RecurrenceTz) -> Vec<i64> {
    // Bumped from 366 to 800: the previous default truncated dense BY-rules
    // (e.g. `BYDAY=MO,TU,WE,TH,FR` = 5/wk × 104wk = 520 emissions) inside
    // the 2-year synthesised fallback window. The standup vanished from
    // the calendar 17 months in. (Round 3 #4.) UNTIL-bounded rules run to
    // RRULE_MAX_COUNT.
    let cap = instance_cap(rule, 800);
    let mut out = Vec::with_capacity(cap);
    let interval_days = rule.interval * 7;

    if rule.byday.is_empty() {
        // Plain weekly recurrence on the same weekday as the start.
        let mut current = start;
        for _ in 0..RRULE_MAX_STEPS {
            if out.len() >= cap {
                break;
            }
            out.push(current);
            // Calendar-day arithmetic (not raw seconds) so the wall-clock
            // time stays put across DST transitions.
            current = add_days_in_zone(current, interval_days, tz)
                .unwrap_or(current + interval_days * 86400);
        }
        return out;
    }

    let wkst = rule.wkst.unwrap_or(civil::Weekday::Monday);
    // RFC 5545 § 3.8.5.3 says DTSTART is always part of the recurrence set;
    // a strict reading therefore requires the WEEKLY+BYDAY shape to emit
    // DTSTART even when its weekday is not in BYDAY (e.g. a Tuesday DTSTART
    // with BYDAY=MO,WE). dateutil drops DTSTART in that case and most
    // operational calendars (Apple Calendar, Google Calendar, Outlook)
    // match dateutil. The existing implementation matches that behavior
    // by filtering candidates against `start` below; preserved deliberately
    // here so the calendar matches what users see in the leading
    // implementations.
    // WEEKLY ignores BYDAY ordinals (RFC 5545 § 3.3.10) so we only
    // consider the bare weekday. Sort by week-start anchored offset so
    // each week emits in chronological order rather than Mon-first.
    let mut days: Vec<civil::Weekday> = rule.byday.iter().map(|b| b.day).collect();
    days.sort_by_key(|d| (days_from_monday(*d) - days_from_monday(wkst)).rem_euclid(7));

    let week_start = start_of_week(start, wkst, tz);
    let mut week_anchor = week_start;
    // Step-bounded: each "step" is one anchored week. Same DoS guard
    // rationale as `expand_daily`.
    for _ in 0..RRULE_MAX_STEPS {
        if out.len() >= cap {
            break;
        }
        for &wd in &days {
            let candidate = shift_to_weekday(week_anchor, wd, wkst, start, tz);
            if candidate < start {
                continue;
            }
            out.push(candidate);
            if out.len() >= cap {
                break;
            }
        }
        week_anchor = add_days_in_zone(week_anchor, interval_days, tz)
            .unwrap_or(week_anchor + interval_days * 86400);
    }
    out
}

fn expand_monthly(start: i64, rule: &Rrule, tz: &RecurrenceTz) -> Vec<i64> {
    // Bumped from 120 to 800: dense BY-rules
    // (`FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR` ~ 22/month) would otherwise
    // truncate to ~5.5 months inside the 2-year fallback window. (Round 3
    // #4.) UNTIL-bounded rules run to RRULE_MAX_COUNT.
    let cap = instance_cap(rule, 800);
    let mut out = Vec::with_capacity(cap);
    let Some(start_dt) = tz.naive(start) else {
        return out;
    };
    // Widen once, here: the month/year cursor arithmetic below is i32/u32.
    let (start_year, start_month, original_day) = ymd_of(start_dt);
    // Hoist the wall-clock time out of the per-month loop. `with_ymd_time`
    // takes the pre-resolved time so it doesn't redo `tz.naive(start)` per
    // candidate. (Round 4 #13.)
    let start_time = start_dt.time();

    // Year+month cursors advance by `interval` calendar months per step. The
    // previous shape stepped via `advance_months(current, interval)`, which
    // walks forward to find a month containing `original_day` - correct
    // for default-day MONTHLY (Jan 31 -> Mar 31, never Feb 28) but wrong
    // when explicit BYMONTHDAY/BYDAY is set: e.g. `BYMONTHDAY=1,-1`
    // starting Jan 31 wants Feb 1 / Feb 28 / Apr 1 / Apr 30, but
    // `advance_months` skipped Feb and April entirely because they don't
    // contain day 31. With a cursor we visit every interval-th month and
    // the per-month `collect_monthly_days` / default-day check decides
    // what (if anything) to emit there.
    let mut year = start_year;
    let mut month = start_month;
    // Step-bounded: filters that no visited month satisfies (e.g.
    // BYMONTHDAY=31 with INTERVAL=12 starting in February) would otherwise
    // never grow `out` and would loop forever.
    for _ in 0..RRULE_MAX_STEPS {
        if out.len() >= cap {
            return out;
        }

        if rule.byday.is_empty() && rule.bymonthday.is_empty() {
            // Default-day path: emit the start's day-of-month if this month
            // has it (Jan 31 monthly emits Jan/Mar/May/... and skips short
            // months rather than clamping). Inline the single candidate to
            // skip the `vec![original_day]` allocation per iteration.
            // (Round 4 #15.)
            if days_in_month(year, month) >= original_day
                && let Some(ts) = with_ymd_time(start_time, year, month, original_day, tz)
                && ts >= start
            {
                out.push(ts);
                if out.len() >= cap {
                    return out;
                }
            }
        } else {
            let mut day_candidates =
                collect_monthly_days(year, month, &rule.byday, &rule.bymonthday);
            day_candidates.sort_unstable();
            day_candidates.dedup();
            for day in day_candidates {
                if let Some(ts) = with_ymd_time(start_time, year, month, day, tz)
                    && ts >= start
                {
                    out.push(ts);
                    if out.len() >= cap {
                        return out;
                    }
                }
            }
        }

        // Advance month cursor by `interval` calendar months.
        let total = i64::from(month) - 1 + rule.interval;
        let new_month = u32::try_from(total.rem_euclid(12) + 1).unwrap_or(1);
        let year_step = i32::try_from(total.div_euclid(12)).unwrap_or(0);
        year = match year.checked_add(year_step) {
            Some(y) => y,
            None => break,
        };
        month = new_month;
    }
    out
}

/// Resolve a month's candidate day-of-month values from BYDAY + BYMONTHDAY.
///
/// - BYDAY without an ordinal: every occurrence of that weekday in the month.
/// - BYDAY with an ordinal: only the n-th occurrence (positive: from start;
///   negative: from end). Returns no days if the n-th doesn't exist.
/// - BYMONTHDAY: explicit days (negative counts from end of month).
/// - Both set: intersection (RFC 5545 § 3.3.10).
fn collect_monthly_days(year: i32, month: u32, byday: &[ByDay], bymonthday: &[i32]) -> Vec<u32> {
    let dim = days_in_month(year, month);

    // reviewed (R3 verified non-issue): mixed BYDAY shapes resolve correctly
    // via per-entry flat_map. `BYDAY=2WE,-1FR` (two distinct ordinals) emits
    // two days; `BYDAY=MO,1FR` (bare + ordinal) emits all Mondays plus the
    // first Friday. Caller (`expand_monthly`/`expand_yearly`) sorts+dedups so
    // duplicates from overlapping rules collapse and emission order matches
    // calendar order. Matches RFC 5545 §3.3.10.
    let byday_days: Vec<u32> = byday
        .iter()
        .flat_map(|b| match b.ordinal {
            None => weekday_occurrences_in_month(year, month, b.day),
            Some(n) => nth_weekday_in_month(year, month, b.day, n)
                .into_iter()
                .collect(),
        })
        .collect();

    #[allow(clippy::cast_possible_wrap)]
    let dim_i = dim as i32;
    // reviewed (R3 verified non-issue): negative BYMONTHDAY resolves correctly:
    // -31 + 31 + 1 = 1 in 31-day months (emits day 1), -31 + 30 + 1 = 0 in
    // 30-day months (filtered by the `< 1` check, no candidate). Same shape
    // for BYMONTHDAY=29;BYMONTH=2 in non-leap years: dim=28 fails the
    // `> dim_i` bound, no candidate -- matches dateutil's skip-non-leap.
    let bymonthday_days: Vec<u32> = bymonthday
        .iter()
        .filter_map(|d| {
            let resolved = if *d < 0 { dim_i + d + 1 } else { *d };
            if resolved < 1 || resolved > dim_i {
                None
            } else {
                #[allow(clippy::cast_sign_loss)]
                Some(resolved as u32)
            }
        })
        .collect();

    match (byday.is_empty(), bymonthday.is_empty()) {
        (true, true) => Vec::new(),
        (false, true) => byday_days,
        (true, false) => bymonthday_days,
        // Intersection: the day must satisfy both filters.
        (false, false) => byday_days
            .into_iter()
            .filter(|d| bymonthday_days.contains(d))
            .collect(),
    }
}

/// All days-of-month within `year`/`month` that fall on `weekday`.
///
/// Computes the weekday of day-1 once, then walks day-of-month with
/// modular arithmetic instead of constructing a NaiveDate per day. The
/// outer YEARLY expander can call this up to ~30 times per month per
/// year per BYDAY entry; the previous shape paid 30 `from_ymd_opt`s per
/// call for what is fundamentally a `(d - 1) % 7` check.
fn weekday_occurrences_in_month(year: i32, month: u32, weekday: civil::Weekday) -> Vec<u32> {
    let dim = days_in_month(year, month);
    let Some(day1) = civil_date(year, month, 1) else {
        return Vec::new();
    };
    // `to_monday_zero_offset` is 0..=6 by construction, so `unsigned_abs`
    // widens without a lossy cast.
    let day1_weekday = u32::from(day1.weekday().to_monday_zero_offset().unsigned_abs());
    let target = u32::from(weekday.to_monday_zero_offset().unsigned_abs());
    (1..=dim)
        .filter(|&d| {
            let offset = (d - 1) % 7;
            (day1_weekday + offset) % 7 == target
        })
        .collect()
}

/// The n-th occurrence of `weekday` in `year`/`month`. Positive `n` counts
/// from the start of the month; negative counts from the end.
fn nth_weekday_in_month(year: i32, month: u32, weekday: civil::Weekday, n: i32) -> Option<u32> {
    let occurrences = weekday_occurrences_in_month(year, month, weekday);
    if n > 0 {
        let idx = usize::try_from(n - 1).ok()?;
        occurrences.get(idx).copied()
    } else if n < 0 {
        let from_end = usize::try_from(-n - 1).ok()?;
        occurrences.iter().rev().nth(from_end).copied()
    } else {
        None
    }
}

fn expand_yearly(start: i64, rule: &Rrule, tz: &RecurrenceTz) -> Vec<i64> {
    // YEARLY's unbounded default sits lower than the others because the
    // 2-year fallback window only emits ~2 instances per realistic rule
    // (annual events). 200 covers ~80 years for repeated holidays
    // (`BYMONTH=12;BYMONTHDAY=25;COUNT=...`) without ever hitting in
    // practice. UNTIL-bounded rules - the case
    // `FREQ=YEARLY;UNTIL=22000101T000000Z` from 2026 (which previously
    // emitted 60 of 174 instances) - run to RRULE_MAX_COUNT and let UNTIL
    // do the work. (Round 3 #2.)
    let cap = instance_cap(rule, 200);
    let mut out = Vec::with_capacity(cap);
    let Some(start_dt) = tz.naive(start) else {
        return out;
    };
    // Widen once, here: the month/year cursor arithmetic below is i32/u32.
    let (start_year, original_month, original_day) = ymd_of(start_dt);
    // Hoist the wall-clock time. Without this, `expand_yearly` was paying
    // ~80k * 12 * ~30 = ~30M `tz.naive(start)` resolves on the inner
    // `with_year_month_day` call; the time is invariant across the whole
    // expansion. (Round 4 #13.)
    let start_time = start_dt.time();

    // Year cursor advances by `interval` years per step. Previous shape stepped
    // via `advance_months(current, interval * 12)`, which walks forward to
    // find a month that contains the original day-of-month - correct for
    // MONTHLY (Jan 31 -> Mar 31) but wrong for YEARLY (Feb 29 -> March 29
    // of the next non-leap year). With a year cursor and the per-iteration
    // `days_in_month` check below, Feb 29 yearly correctly skips non-leap
    // years and emits only on real leap years.
    // RRULE INTERVAL is bounded above by what callers can plausibly emit; an
    // i32 is more than enough for any real recurrence and `try_from` keeps a
    // wedged INTERVAL=2_000_000_000 from silently casting to a negative
    // step. On overflow we step by 1 year and let the COUNT/UNTIL/RRULE_MAX
    // bounds terminate. (`parse_rrule` already clamps interval to >=1, so
    // no further `.max(1)` is needed here.)
    let interval_years: i32 = i32::try_from(rule.interval).unwrap_or(1);
    let mut year = start_year;
    // Hoist the months slice. The default path uses a single-element stack
    // array; explicit BYMONTH borrows the rule's slice. Previously cloned a
    // Vec<u32> per year inside the 80k-iteration loop. (Round 4 #12.)
    let default_months = [original_month];
    let months: &[u32] = if rule.bymonth.is_empty() {
        &default_months
    } else {
        &rule.bymonth
    };
    // Sparse YEARLY rules (e.g. `BYMONTH=2;BYMONTHDAY=29`, the leap-day-
    // only case) emit one instance every four calendar years walked. The
    // shared 12_000-step bound bottoms out at 3_000 emissions for those,
    // so a `COUNT=10000` request silently truncates before reaching the
    // cap. Use a YEARLY-specific upper bound large enough that even the
    // sparsest realistic rule (every 8 years for an 8-year-cycle holiday)
    // can still hit RRULE_MAX_COUNT before this fires. Each step here is
    // O(1) calendar arithmetic so the step bound is cheap to raise.
    const YEARLY_MAX_STEPS: usize = 80_000;
    for _ in 0..YEARLY_MAX_STEPS {
        if out.len() >= cap {
            return out;
        }

        for &month in months {
            if rule.byday.is_empty() && rule.bymonthday.is_empty() {
                // Default-day path: same DOM as start, skipped if the target
                // month doesn't have that day (Feb 29 in non-leap years).
                // Inline the single candidate to skip the `vec![original_day]`
                // allocation per (year, month). (Round 4 #15.)
                if days_in_month(year, month) >= original_day
                    && let Some(ts) = with_ymd_time(start_time, year, month, original_day, tz)
                    && ts >= start
                {
                    out.push(ts);
                    if out.len() >= cap {
                        return out;
                    }
                }
            } else {
                let mut days = collect_monthly_days(year, month, &rule.byday, &rule.bymonthday);
                days.sort_unstable();
                days.dedup();
                for day in days {
                    if let Some(ts) = with_ymd_time(start_time, year, month, day, tz)
                        && ts >= start
                    {
                        out.push(ts);
                        if out.len() >= cap {
                            return out;
                        }
                    }
                }
            }
        }
        year = match year.checked_add(interval_years) {
            Some(y) => y,
            None => break,
        };
    }
    out
}

/// Resolve a wall-clock instant on a specific calendar date, preserving a
/// pre-computed time-of-day in the event's recurrence zone.
///
/// The caller hoists `tz.naive(start).time()` outside the expansion loop and
/// passes it in. The previous shape (`with_year_month_day`) re-resolved the
/// invariant start timestamp through `tz.naive(...)` per candidate; the
/// YEARLY expander was paying ~30M of those for a single dense rule.
fn with_ymd_time(
    time: civil::Time,
    year: i32,
    month: u32,
    day: u32,
    tz: &RecurrenceTz,
) -> Option<i64> {
    let new_date = civil_date(year, month, day)?;
    tz.resolve(new_date.to_datetime(time))
}

fn matches_weekday(timestamp: i64, days: &[civil::Weekday], tz: &RecurrenceTz) -> bool {
    let Some(naive) = tz.naive(timestamp) else {
        return false;
    };
    let wd = naive.date().weekday();
    days.contains(&wd)
}

/// Advance `timestamp` by `days` calendar days in the event's recurrence
/// zone, preserving wall-clock time across DST transitions. Returns `None`
/// only if the resulting NaiveDateTime or zone resolution overflows
/// (essentially unreachable for any plausible recurrence window).
fn add_days_in_zone(timestamp: i64, days: i64, tz: &RecurrenceTz) -> Option<i64> {
    let naive = tz.naive(timestamp)?;
    // Civil-day arithmetic: adding a day to a civil datetime moves the
    // calendar day and leaves the clock alone, which is exactly the
    // wall-clock-preserving step this wants. The zone re-enters in
    // `tz.resolve`.
    let new_naive = naive.checked_add(Span::new().days(days)).ok()?;
    tz.resolve(new_naive)
}

fn start_of_week(timestamp: i64, week_start: civil::Weekday, tz: &RecurrenceTz) -> i64 {
    let Some(naive) = tz.naive(timestamp) else {
        return timestamp;
    };
    let current = naive.date().weekday();
    // Modular distance from `week_start` to `current`, walking forward
    // through the week (so a Sun-anchored week with current=Sat -> 6 days
    // back, and a Mon-anchored week with current=Sun -> 6 days back).
    let from = days_from_monday(week_start);
    let to = days_from_monday(current);
    let days_back = (to - from).rem_euclid(7);
    add_days_in_zone(timestamp, -days_back, tz).unwrap_or_else(|| {
        // `add_days_in_zone` only returns None when the resulting
        // NaiveDateTime or zone resolution overflows - in practice that
        // requires walking back across a 24-hour-skipped day (Pacific/Apia
        // Dec 30 2011). Returning the un-walked timestamp here used to
        // silently mis-anchor `shift_to_weekday`: that helper computes
        // `target_offset` from the SUPPLIED week_start, but the un-walked
        // input is on `current` (a Wednesday in the Apia case), so for
        // `WKST=SU` and `target=SU`, `target_offset=0` resolved to
        // Wednesday and the rule emitted Wed instances every "Sunday."
        // Logged so the operator can attribute the weird emission to a
        // zone-skip; the recovery is now in `shift_to_weekday` which
        // falls back to a current-anchor offset rather than trusting
        // this anchor. (Round 3 #17.)
        log::debug!(
            "start_of_week: add_days_in_zone(-{days_back}) failed (likely walking through a 24h-skipped day); shift_to_weekday will recompute from the un-walked anchor"
        );
        timestamp
    })
}

fn shift_to_weekday(
    week_anchor: i64,
    target: civil::Weekday,
    week_start: civil::Weekday,
    time_source: i64,
    tz: &RecurrenceTz,
) -> i64 {
    let Some(anchor_naive) = tz.naive(week_anchor) else {
        return week_anchor;
    };
    let Some(time_naive) = tz.naive(time_source) else {
        return week_anchor;
    };
    // Modular offset from `week_anchor`'s actual weekday to `target`.
    // Computing from `week_start` instead silently mis-anchors when
    // `start_of_week` fell back to its un-walked input (a 24h-skipped
    // day in the Apia case): `target_offset` against the assumed
    // anchor was 0 for the start-day even though anchor was on a
    // different weekday entirely. (Round 3 #17.)
    let anchor_weekday = days_from_monday(anchor_naive.date().weekday());
    let target_to = days_from_monday(target);
    let target_offset = (target_to - anchor_weekday).rem_euclid(7);
    // `week_start` is no longer used for the offset math - kept in the
    // signature for caller compatibility, since week_anchor is supposed
    // to *be* the start-of-week and the two should agree on
    // well-resolved inputs. The rebuilt offset above is the safe
    // fallback when they don't.
    let _ = week_start;
    // Day arithmetic in calendar units, not raw seconds. Then reattach the
    // intended wall-clock time and re-resolve in the event's zone, falling
    // through gap/ambiguous via `resolve_local_to_timestamp`.
    let Ok(target_date) = anchor_naive
        .date()
        .checked_add(Span::new().days(target_offset))
    else {
        return week_anchor;
    };
    let new_naive = target_date.to_datetime(time_naive.time());
    tz.resolve(new_naive).unwrap_or(week_anchor)
}

/// Days in a given month.
fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

/// Parse an UNTIL value (RFC 5545 § 3.3.10).
///
/// Three valid forms per spec:
/// - `YYYYMMDD` (DATE only) - "everything up to end of that local day." We
///   anchor end-of-day in `chrono::Local` because DATE-only UNTIL implies
///   floating DTSTART (which RFC 5545 § 3.3.5 says is interpreted in the
///   user's calendar zone). Anchoring to UTC midnight 23:59:59 clips
///   evening occurrences in west-of-UTC zones and includes next-day
///   occurrences in east-of-UTC zones.
/// - `YYYYMMDDTHHMMSSZ` (DATE-TIME, UTC) - the wall-clock instant in UTC.
///   We preserve the exact time, not collapse to 23:59:59.
/// - `YYYYMMDDTHHMMSS` (DATE-TIME, floating) - per RFC 5545 only valid
///   when DTSTART is floating. Anchored in `chrono::Local` for the same
///   reason as DATE-only.
///
/// Anything else (offset like `+0100`, sub-minute precision, trailing
/// garbage) is rejected with `None` rather than silently mis-anchored.
///
/// Returns the raw `Until` shape; zone resolution happens in
/// `Until::resolve` once the event's `RecurrenceTz` is in scope. (Round 3
/// #7.)
fn parse_until_date(val: &str) -> Option<Until> {
    let date_part = val.get(..8)?;
    let year: i32 = date_part.get(0..4)?.parse().ok()?;
    let month: u32 = date_part.get(4..6)?.parse().ok()?;
    let day: u32 = date_part.get(6..8)?.parse().ok()?;
    // Reject obviously bogus calendar years. RFC 5545 § 3.3.5 doesn't fix a
    // range, but iCalendar in practice carries only Gregorian dates and
    // year 0 / negatives produce a deeply negative UTC instant - the rule
    // then emits zero instances, which is bounded but a confusing way for
    // a malformed UNTIL to manifest. 9999 is chrono's outer year for
    // representable timestamps; values past that round-trip into chrono::
    // MAX and silently land elsewhere.
    if !(1..=9999).contains(&year) {
        log::debug!("RRULE UNTIL year {year} outside 1..=9999; rejecting");
        return None;
    }
    let date = civil_date(year, month, day)?;

    // DATE-only form: exactly 8 chars.
    if val.len() == 8 {
        return Some(Until::Date(date));
    }

    // DATE-TIME form must be exactly 15 (floating) or 16 (UTC) chars and
    // have a `T` at index 8.
    if val.as_bytes().get(8) != Some(&b'T') {
        log::debug!("RRULE UNTIL has unrecognized form: {val}");
        return None;
    }
    let time_part = val.get(9..15)?;
    let hour: i8 = time_part.get(0..2)?.parse().ok()?;
    let minute: i8 = time_part.get(2..4)?.parse().ok()?;
    let second: i8 = time_part.get(4..6)?.parse().ok()?;
    let dt = date.at(hour, minute, second, 0);

    match (val.len(), val.as_bytes().get(15)) {
        // Floating: 15 chars, no trailing character.
        (15, None) => Some(Until::Floating(dt)),
        // UTC: 16 chars, trailing 'Z'.
        // UTC: 16 chars, trailing 'Z'. The `Z` really is UTC here - this is
        // RFC 5545 wire syntax, not a Temporal-style timestamp string, so it
        // is resolved explicitly against `TimeZone::UTC` rather than parsed.
        (16, Some(&b'Z')) => TimeZone::UTC
            .to_zoned(dt)
            .ok()
            .map(|zoned| Until::Utc(zoned.timestamp().as_second())),
        // Anything else (offset like +0100, fractional seconds, trailing
        // garbage) is malformed; rejecting prevents silent UTC mis-anchor.
        _ => {
            log::debug!("RRULE UNTIL has unsupported trailing characters: {val}");
            None
        }
    }
}

#[cfg(test)]
mod tests;
