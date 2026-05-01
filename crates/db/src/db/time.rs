//! Timezone-aware datetime helpers.
//!
//! Centralizes the conversion of a wall-clock `NaiveDateTime` plus a
//! `chrono::TimeZone` into a Unix timestamp, including correct handling of
//! the two pathological cases that wreck calendar correctness:
//!
//! - **Spring-forward gap**: the wall clock skips an hour (e.g. 02:00 -> 03:00
//!   in America/New_York on the second Sunday of March). A datetime *inside*
//!   the gap (02:30 in this example) does not exist as a unique instant.
//!   `chrono::TimeZone::from_local_datetime` returns `LocalResult::None`.
//! - **Fall-back ambiguity**: the wall clock repeats an hour (e.g. 01:30 in
//!   America/New_York on the first Sunday of November names two distinct UTC
//!   instants, one in EDT and one in EST). `from_local_datetime` returns
//!   `LocalResult::Ambiguous(early, late)`.
//!
//! Naive `.single()` callers silently lose data on both. The resolver here
//! picks the earlier instant for ambiguous (matches RFC 5545's documented
//! behavior and what Outlook/Google Calendar do) and shifts past the gap for
//! non-existent (preserves the calendar invariant that an event that "starts
//! at 02:30" still produces a concrete timestamp on a DST day).

use chrono::{Duration, LocalResult, NaiveDateTime, TimeZone};

/// Convert a wall-clock `NaiveDateTime` in `tz` to a Unix timestamp.
///
/// Returns `None` only when the gap surrounding `naive` is wider than the
/// 48-hour walk bound below. That covers Pacific/Apia's 2011-12-30 24-hour
/// skip; the only realistic input it cannot handle is a hypothetical
/// multi-day skip not present in any TZ database.
pub fn resolve_local_to_timestamp<Tz: TimeZone>(naive: NaiveDateTime, tz: &Tz) -> Option<i64> {
    // reviewed (R3 verified non-issue): fixed-offset zones (chrono::FixedOffset,
    // chrono_tz::Tz fixed entries, anything constructed from VTIMEZONE
    // STANDARD-only blocks) only ever return LocalResult::Single -- they have
    // no DST so the Ambiguous and None arms below cannot fire. The generic
    // path is correct; do not special-case Fixed.
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(t) => Some(t.timestamp()),
        // Fall-back: pick the earlier instant. RFC 5545 Section 3.3.5 doesn't
        // mandate this but it matches what Outlook, Google Calendar, and
        // Apple Calendar all do, and it preserves "first occurrence wins"
        // ordering for events that get scheduled near the transition.
        LocalResult::Ambiguous(early, _late) => Some(early.timestamp()),
        // Spring-forward / day-skip: shift forward by the gap width so the
        // user's intended wall-clock minute is preserved past the gap. The
        // gap width is detected by walking back to the last valid wall
        // clock and forward to the first valid wall clock - so the
        // 60-minute gap of US/Europe DST, the 30-minute gap of Lord Howe,
        // and the 24-hour skip of Pacific/Apia 2011-12-30 all produce the
        // same shape of answer:
        //
        //   gap_width ≈ backward + forward - 1   (minute resolution)
        //   output = resolve(naive + gap_width)
        //
        // The previous shape tried +60 first, then +30, then walked
        // minute-by-minute and accepted the first valid hit. That picked
        // the WRONG answer for sub-hour gaps (Lord Howe 02:15 -> 03:15
        // LHST instead of 02:45 LHDT) and bailed entirely on 24-hour
        // skips, leaving callers (`calendars.rs::add_days_in_zone` etc) to
        // fall back to raw-seconds arithmetic that silently shifts every
        // subsequent recurring instance by the gap offset.
        LocalResult::None => resolve_through_gap(naive, tz),
    }
}

fn resolve_through_gap<Tz: TimeZone>(naive: NaiveDateTime, tz: &Tz) -> Option<i64> {
    // 48 hours each way: enough headroom for the 24h Pacific/Apia skip
    // plus a safety margin for any double-transition that might land us in
    // a second gap during the walk. Probe resolution is 1 minute, matching
    // the smallest DST step IANA carries.
    const MAX_PROBE_MINUTES: i64 = 60 * 48;

    let mut backward = 0i64;
    let gap_start_offset = loop {
        backward += 1;
        if backward > MAX_PROBE_MINUTES {
            return None;
        }
        match tz.from_local_datetime(&(naive - Duration::minutes(backward))) {
            LocalResult::None => continue,
            _ => break backward,
        }
    };

    let mut forward = 0i64;
    let gap_end_offset = loop {
        forward += 1;
        if forward > MAX_PROBE_MINUTES {
            return None;
        }
        match tz.from_local_datetime(&(naive + Duration::minutes(forward))) {
            LocalResult::None => continue,
            _ => break forward,
        }
    };

    // gap_width is the number of minutes the wall clock skipped - if naive
    // sits k minutes into the gap, post-gap output should be at the same
    // post-gap offset, i.e. naive + gap_width preserves the wall-clock
    // minute past the gap.
    let gap_width = gap_start_offset + gap_end_offset - 1;
    let shifted = naive.checked_add_signed(Duration::minutes(gap_width))?;
    match tz.from_local_datetime(&shifted) {
        LocalResult::Single(t) => Some(t.timestamp()),
        // If the post-gap instant is itself ambiguous (a spring-forward gap
        // chained into a fall-back within the walk window), pick the LATER
        // instant so the result lies past every transition the walk
        // observed, preserving the "shift forward through all transitions"
        // intent. Antarctica/Casey 2009-10 is the documented case.
        LocalResult::Ambiguous(_early, late) => Some(late.timestamp()),
        LocalResult::None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use chrono_tz::Tz;

    fn naive(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(year, month, day)
            .expect("valid date")
            .and_hms_opt(hour, minute, 0)
            .expect("valid time")
    }

    #[test]
    fn single_local_time_resolves_directly() {
        // 2024-03-15 10:00 America/New_York is unambiguous.
        let ts = resolve_local_to_timestamp(naive(2024, 3, 15, 10, 0), &Tz::America__New_York)
            .expect("resolves");
        assert_eq!(ts, 1710511200);
    }

    #[test]
    fn fall_back_ambiguous_picks_earlier_instant() {
        // 2024-11-03 01:30 America/New_York is ambiguous: it occurs once at
        // 05:30 UTC (EDT, "early") and again at 06:30 UTC (EST, "late"). The
        // resolver returns the earlier instant.
        let ts = resolve_local_to_timestamp(naive(2024, 11, 3, 1, 30), &Tz::America__New_York)
            .expect("resolves");
        let early = NaiveDate::from_ymd_opt(2024, 11, 3)
            .and_then(|d| d.and_hms_opt(5, 30, 0))
            .map(|d| d.and_utc().timestamp())
            .expect("valid");
        assert_eq!(ts, early);
    }

    #[test]
    fn spring_forward_gap_shifts_past_the_gap() {
        // 2024-03-10 02:30 America/New_York doesn't exist (clock jumps from
        // 02:00 EST to 03:00 EDT). The resolver shifts forward to 03:30 EDT
        // = 07:30 UTC.
        let ts = resolve_local_to_timestamp(naive(2024, 3, 10, 2, 30), &Tz::America__New_York)
            .expect("resolves");
        let after_gap = NaiveDate::from_ymd_opt(2024, 3, 10)
            .and_then(|d| d.and_hms_opt(7, 30, 0))
            .map(|d| d.and_utc().timestamp())
            .expect("valid");
        assert_eq!(ts, after_gap);
    }

    #[test]
    fn utc_zone_passes_through() {
        let ts = resolve_local_to_timestamp(naive(2024, 6, 15, 10, 30), &chrono::Utc)
            .expect("resolves");
        let expected = NaiveDate::from_ymd_opt(2024, 6, 15)
            .and_then(|d| d.and_hms_opt(10, 30, 0))
            .map(|d| d.and_utc().timestamp())
            .expect("valid");
        assert_eq!(ts, expected);
    }

    #[test]
    fn lord_howe_30min_gap_preserves_wall_clock_minute() {
        // Lord Howe Island runs a 30-minute DST. Spring-forward 2024 was
        // 2024-10-06: 02:00 LHST jumps to 02:30 LHDT, so 02:15 is in the
        // gap. The pre-fix walker tried +60 first, landed on 03:15 LHDT,
        // and silently shifted the user's intended minute by 30 minutes
        // past where they wrote it down. Width-based shift puts 02:15 at
        // 02:45 LHDT (= 02:15 + 30 min gap width).
        let ts = resolve_local_to_timestamp(naive(2024, 10, 6, 2, 15), &Tz::Australia__Lord_Howe)
            .expect("resolves through gap");
        // 02:45 LHDT = (02:45 - 11:00) UTC = 15:45 UTC the previous day.
        let expected = NaiveDate::from_ymd_opt(2024, 10, 5)
            .and_then(|d| d.and_hms_opt(15, 45, 0))
            .map(|d| d.and_utc().timestamp())
            .expect("valid");
        assert_eq!(ts, expected);
    }

    #[test]
    fn pacific_apia_24h_skip_resolves_at_post_skip_instant() {
        // Pacific/Apia skipped 2011-12-30 entirely (jumped from
        // 2011-12-29 23:59:59 -10:00 to 2011-12-31 00:00:00 +14:00). A
        // wall clock anywhere on Dec 30 is in a 24-hour gap. The pre-fix
        // walker capped probes at 120 minutes and returned None for the
        // whole day, leaving callers to fall back to raw-seconds
        // arithmetic that silently mis-anchored every subsequent
        // recurring instance. With the wider walk, the returned instant
        // is the same wall-clock-of-day on Dec 31.
        let ts = resolve_local_to_timestamp(naive(2011, 12, 30, 12, 0), &Tz::Pacific__Apia)
            .expect("24h-skipped day resolves");
        // 2011-12-31 12:00 +14:00 = 2011-12-30 22:00 UTC.
        let expected = NaiveDate::from_ymd_opt(2011, 12, 30)
            .and_then(|d| d.and_hms_opt(22, 0, 0))
            .map(|d| d.and_utc().timestamp())
            .expect("valid");
        assert_eq!(ts, expected);
    }
}
