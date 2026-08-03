//! Timezone-aware datetime helpers.
//!
//! Centralizes the conversion of a wall-clock `civil::DateTime` plus a
//! `TimeZone` into a Unix timestamp, including correct handling of the two
//! pathological cases that wreck calendar correctness:
//!
//! - **Spring-forward gap**: the wall clock skips an hour (e.g. 02:00 -> 03:00
//!   in America/New_York on the second Sunday of March). A datetime *inside*
//!   the gap (02:30 in this example) does not exist as a unique instant.
//! - **Fall-back ambiguity**: the wall clock repeats an hour (e.g. 01:30 in
//!   America/New_York on the first Sunday of November names two distinct UTC
//!   instants, one in EDT and one in EST).
//!
//! Both are resolved by jiff's "compatible" disambiguation, which is what
//! [`TimeZone::to_zoned`] applies: an ambiguous (fold) wall clock takes the
//! EARLIER instant, and a nonexistent (gap) wall clock shifts FORWARD by the
//! width of the gap. That is exactly the policy this module used to hand-roll
//! against chrono - the earlier instant matches RFC 5545 and what Outlook,
//! Google Calendar, and Apple Calendar all do, and shifting past the gap
//! preserves the calendar invariant that an event that "starts at 02:30" still
//! produces a concrete timestamp on a DST day.
//!
//! The hand-rolled version walked minute-by-minute to measure the gap width
//! and gave up past a 48-hour bound. jiff resolves an arbitrary gap in one
//! step, so both the walk and its bound are gone; Pacific/Apia's 24-hour skip
//! is not a special case here, it is just a wide gap.

use jiff::civil;
use jiff::tz::TimeZone;

/// Convert a wall-clock `civil::DateTime` in `tz` to a Unix timestamp.
///
/// Returns `None` only when the resolved instant falls outside jiff's
/// representable timestamp range.
pub fn resolve_local_to_timestamp(naive: civil::DateTime, tz: &TimeZone) -> Option<i64> {
    // reviewed (R3 verified non-issue): fixed-offset zones (`TimeZone::UTC`,
    // `TimeZone::fixed`, anything constructed from VTIMEZONE STANDARD-only
    // blocks) are never ambiguous and never have gaps, so the disambiguation
    // below is a no-op for them. The general path is correct; do not
    // special-case fixed offsets.
    tz.to_zoned(naive)
        .ok()
        .map(|zoned| zoned.timestamp().as_second())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(name: &str) -> TimeZone {
        TimeZone::get(name).expect("known IANA zone")
    }

    /// The UTC instant for a wall clock already expressed in UTC. Used to state
    /// each expectation as "this UTC wall clock" rather than a bare epoch int.
    fn utc_ts(year: i16, month: i8, day: i8, hour: i8, minute: i8) -> i64 {
        TimeZone::UTC
            .to_zoned(civil::datetime(year, month, day, hour, minute, 0, 0))
            .expect("valid UTC instant")
            .timestamp()
            .as_second()
    }

    #[test]
    fn single_local_time_resolves_directly() {
        // 2024-03-15 10:00 America/New_York is unambiguous.
        let ts = resolve_local_to_timestamp(
            civil::datetime(2024, 3, 15, 10, 0, 0, 0),
            &zone("America/New_York"),
        )
        .expect("resolves");
        assert_eq!(ts, 1_710_511_200);
    }

    #[test]
    fn fall_back_ambiguous_picks_earlier_instant() {
        // 2024-11-03 01:30 America/New_York is ambiguous: it occurs once at
        // 05:30 UTC (EDT, "early") and again at 06:30 UTC (EST, "late"). The
        // resolver returns the earlier instant.
        let ts = resolve_local_to_timestamp(
            civil::datetime(2024, 11, 3, 1, 30, 0, 0),
            &zone("America/New_York"),
        )
        .expect("resolves");
        assert_eq!(ts, utc_ts(2024, 11, 3, 5, 30));
    }

    #[test]
    fn spring_forward_gap_shifts_past_the_gap() {
        // 2024-03-10 02:30 America/New_York doesn't exist (clock jumps from
        // 02:00 EST to 03:00 EDT). The resolver shifts forward to 03:30 EDT
        // = 07:30 UTC.
        let ts = resolve_local_to_timestamp(
            civil::datetime(2024, 3, 10, 2, 30, 0, 0),
            &zone("America/New_York"),
        )
        .expect("resolves");
        assert_eq!(ts, utc_ts(2024, 3, 10, 7, 30));
    }

    #[test]
    fn utc_zone_passes_through() {
        let ts =
            resolve_local_to_timestamp(civil::datetime(2024, 6, 15, 10, 30, 0, 0), &TimeZone::UTC)
                .expect("resolves");
        assert_eq!(ts, utc_ts(2024, 6, 15, 10, 30));
    }

    #[test]
    fn lord_howe_30min_gap_preserves_wall_clock_minute() {
        // Lord Howe Island runs a 30-minute DST. Spring-forward 2024 was
        // 2024-10-06: 02:00 LHST jumps to 02:30 LHDT, so 02:15 is in the gap.
        // A naive "+1 hour" fixup would land on 03:15 LHDT and silently shift
        // the user's intended minute by 30 minutes past where they wrote it
        // down. Shifting by the ACTUAL gap width puts 02:15 at 02:45 LHDT.
        let ts = resolve_local_to_timestamp(
            civil::datetime(2024, 10, 6, 2, 15, 0, 0),
            &zone("Australia/Lord_Howe"),
        )
        .expect("resolves through gap");
        // 02:45 LHDT = (02:45 - 11:00) UTC = 15:45 UTC the previous day.
        assert_eq!(ts, utc_ts(2024, 10, 5, 15, 45));
    }

    #[test]
    fn pacific_apia_24h_skip_resolves_at_post_skip_instant() {
        // Pacific/Apia skipped 2011-12-30 entirely (jumped from
        // 2011-12-29 23:59:59 -10:00 to 2011-12-31 00:00:00 +14:00). A wall
        // clock anywhere on Dec 30 sits in a 24-hour gap. The old walker
        // capped its probe at 120 minutes and returned None for the whole day,
        // leaving callers to fall back to raw-seconds arithmetic that silently
        // mis-anchored every subsequent recurring instance. Shifting by the
        // real gap width puts the same wall-clock-of-day on Dec 31.
        let ts = resolve_local_to_timestamp(
            civil::datetime(2011, 12, 30, 12, 0, 0, 0),
            &zone("Pacific/Apia"),
        )
        .expect("24h-skipped day resolves");
        // 2011-12-31 12:00 +14:00 = 2011-12-30 22:00 UTC.
        assert_eq!(ts, utc_ts(2011, 12, 30, 22, 0));
    }
}
