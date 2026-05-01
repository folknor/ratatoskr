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
/// Returns `None` only when the date itself is invalid (something like a
/// fictional 02:30 that remains in a gap even after a full hour shift; this
/// is essentially unreachable for any real IANA zone).
pub fn resolve_local_to_timestamp<Tz: TimeZone>(naive: NaiveDateTime, tz: &Tz) -> Option<i64> {
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(t) => Some(t.timestamp()),
        // Fall-back: pick the earlier instant. RFC 5545 Section 3.3.5 doesn't
        // mandate this but it matches what Outlook, Google Calendar, and
        // Apple Calendar all do, and it preserves "first occurrence wins"
        // ordering for events that get scheduled near the transition.
        LocalResult::Ambiguous(early, _late) => Some(early.timestamp()),
        // Spring-forward: shift forward by the gap duration so the user's
        // intended wall-clock minute is preserved (02:30 -> 03:30, not
        // 03:00). DST gaps are 60 min in nearly every zone; Lord Howe
        // Island runs a 30 min DST. Try +60, then +30, then walk
        // minute-by-minute as a defense against unusual TZ data.
        LocalResult::None => {
            for try_offset in [Duration::hours(1), Duration::minutes(30)] {
                match tz.from_local_datetime(&(naive + try_offset)) {
                    LocalResult::Single(t) => return Some(t.timestamp()),
                    LocalResult::Ambiguous(early, _) => return Some(early.timestamp()),
                    LocalResult::None => {}
                }
            }
            for offset in 1..=120 {
                let probe = naive + Duration::minutes(offset);
                match tz.from_local_datetime(&probe) {
                    LocalResult::Single(t) => return Some(t.timestamp()),
                    LocalResult::Ambiguous(early, _) => return Some(early.timestamp()),
                    LocalResult::None => {}
                }
            }
            None
        }
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
}
