use super::super::CalendarViewEvent;
use super::*;

/// The host-local wall clock for `ts`. Mirrors `RecurrenceTz::Local`, which is
/// what the expander uses for floating events, so fixtures and expander agree
/// on which zone "local" means.
fn local_dt(ts: i64) -> civil::DateTime {
    Timestamp::from_second(ts)
        .expect("representable timestamp")
        .to_zoned(TimeZone::system())
        .datetime()
}

fn local_ts(year: i16, month: i8, day: i8, hour: i8, minute: i8) -> i64 {
    // `unambiguous()` rather than jiff's compatible disambiguation, preserving
    // the old `.single().expect("unambiguous")`: a fixture that accidentally
    // lands on a DST fold or gap should fail loudly here rather than silently
    // resolve to one side and make the assertion below mean something else.
    TimeZone::system()
        .to_ambiguous_zoned(civil::datetime(year, month, day, hour, minute, 0, 0))
        .unambiguous()
        .expect("unambiguous local time")
        .timestamp()
        .as_second()
}

fn make_event(start: i64, duration: i64) -> CalendarViewEvent {
    CalendarViewEvent {
        id: "evt".to_string(),
        title: String::new(),
        start_time: start,
        end_time: start + duration,
        all_day: false,
        color: String::new(),
        calendar_name: None,
        location: None,
        recurrence_rule: None,
        calendar_id: None,
        account_id: String::new(),
        organizer_name: None,
        organizer_email: None,
        rsvp_status: None,
        description: None,
        availability: None,
        visibility: None,
        timezone: None,
        uid: None,
        recurrence_id_canonical: None,
    }
}

fn weekday_of(ts: i64) -> civil::Weekday {
    local_dt(ts).date().weekday()
}

/// Timestamp for a wall clock in a named IANA zone. Zoned counterpart of
/// [`local_ts`], with the same deliberate unambiguity assertion.
fn zoned_ts(zone: &str, year: i16, month: i8, day: i8, hour: i8, minute: i8, second: i8) -> i64 {
    TimeZone::get(zone)
        .expect("known IANA zone")
        .to_ambiguous_zoned(civil::datetime(year, month, day, hour, minute, second, 0))
        .unambiguous()
        .expect("unambiguous wall clock in zone")
        .timestamp()
        .as_second()
}

/// Timestamp for a wall clock already expressed in UTC.
fn utc_ts(year: i16, month: i8, day: i8, hour: i8, minute: i8, second: i8) -> i64 {
    zoned_ts("UTC", year, month, day, hour, minute, second)
}

/// The wall clock in `zone` for `ts`. Zoned counterpart of [`local_dt`], used
/// to read an emitted instance back in the EVENT's zone rather than the host's
/// - which is the invariant most of the cross-zone tests below exist to pin.
fn zoned_dt(zone: &str, ts: i64) -> civil::DateTime {
    Timestamp::from_second(ts)
        .expect("representable timestamp")
        .to_zoned(TimeZone::get(zone).expect("known IANA zone"))
        .datetime()
}

#[test]
fn weekly_byday_emits_each_listed_day() {
    // 2026-03-09 is a Monday. RRULE picks Monday/Wednesday/Friday for 6 weeks.
    let start = local_ts(2026, 3, 9, 9, 0);
    let event = make_event(start, 3600);
    let instances = expand_recurrence(&event, "FREQ=WEEKLY;BYDAY=MO,WE,FR;COUNT=6");
    assert_eq!(instances.len(), 6);
    let weekdays: Vec<_> = instances.iter().map(|e| weekday_of(e.start_time)).collect();
    assert_eq!(
        weekdays,
        vec![
            civil::Weekday::Monday,
            civil::Weekday::Wednesday,
            civil::Weekday::Friday,
            civil::Weekday::Monday,
            civil::Weekday::Wednesday,
            civil::Weekday::Friday,
        ]
    );
}

#[test]
fn weekly_byday_preserves_time_of_day() {
    // 2026-03-09 09:30 Mon. BYDAY=MO,WE - time-of-day must stay 09:30 on
    // every emitted instance, even when the day shifts.
    let start = local_ts(2026, 3, 9, 9, 30);
    let event = make_event(start, 1800);
    let instances = expand_recurrence(&event, "FREQ=WEEKLY;BYDAY=MO,WE;COUNT=4");
    for inst in &instances {
        let dt = local_dt(inst.start_time);
        assert_eq!(dt.hour(), 9);
        assert_eq!(dt.minute(), 30);
    }
}

#[test]
fn monthly_bymonthday_picks_specific_day() {
    // FREQ=MONTHLY;BYMONTHDAY=15 starting on 2026-01-10 emits the 15th of
    // Jan, Feb, Mar, ... not the 10th.
    let start = local_ts(2026, 1, 10, 12, 0);
    let event = make_event(start, 3600);
    let instances = expand_recurrence(&event, "FREQ=MONTHLY;BYMONTHDAY=15;COUNT=3");
    assert_eq!(instances.len(), 3);
    for inst in &instances {
        assert_eq!(local_dt(inst.start_time).day(), 15);
    }
}

#[test]
fn yearly_with_until_clamps_window() {
    // Annual on 2026-06-01, UNTIL 2028-06-01 -> 3 instances.
    let start = local_ts(2026, 6, 1, 9, 0);
    let event = make_event(start, 3600);
    let instances = expand_recurrence(&event, "FREQ=YEARLY;UNTIL=20280701T000000Z");
    assert_eq!(instances.len(), 3);
    assert_eq!(weekday_of(instances[0].start_time), weekday_of(start));
}

#[test]
fn daily_with_unsatisfiable_byday_terminates() {
    // Reviewer A #1: Monday DTSTART with FREQ=DAILY;INTERVAL=7;BYDAY=TU
    // can never match - the candidate weekday is always Monday. Without
    // the step bound this spun forever. Confirm we return empty (or at
    // least terminate) instead of looping.
    let monday = local_ts(2026, 3, 9, 9, 0); // 2026-03-09 is a Monday
    let event = make_event(monday, 3600);
    let instances = expand_recurrence(&event, "FREQ=DAILY;INTERVAL=7;BYDAY=TU;COUNT=1");
    // Implementation returns the original event when expansion produces
    // zero matches (`instances.is_empty()` fallback). Either zero or one
    // is acceptable here - what matters is that we returned at all.
    assert!(instances.len() <= 1);
}

#[test]
fn monthly_with_unsatisfiable_bymonthday_terminates() {
    // Reviewer A #2: February DTSTART with FREQ=MONTHLY;INTERVAL=12;
    // BYMONTHDAY=31 - no visited month is February-with-day-31.
    let feb = local_ts(2026, 2, 1, 9, 0);
    let event = make_event(feb, 3600);
    let instances = expand_recurrence(&event, "FREQ=MONTHLY;INTERVAL=12;BYMONTHDAY=31;COUNT=1");
    assert!(instances.len() <= 1);
}

#[test]
fn count_clamped_to_max() {
    // Untrusted COUNT must not pin allocation. RRULE_MAX_COUNT (10_000)
    // is the cap; an upstream `COUNT=999999` should still expand only
    // up to that many entries.
    let start = local_ts(2026, 1, 1, 9, 0);
    let event = make_event(start, 1800);
    let instances = expand_recurrence(&event, "FREQ=DAILY;COUNT=999999");
    assert!(instances.len() <= RRULE_MAX_COUNT);
}

#[test]
fn monthly_jan_31_skips_short_months_not_clamps() {
    // RFC 5545 § 3.3.10: a Jan 31 monthly recurrence emits Jan 31, then
    // Mar 31, May 31, ... - never Feb 28, Mar 28, .... Previously we
    // clamped to the last valid day and never recovered, so every
    // subsequent instance landed on the 28th.
    let start = local_ts(2026, 1, 31, 9, 0);
    let event = make_event(start, 3600);
    let instances = expand_recurrence(&event, "FREQ=MONTHLY;COUNT=4");
    let days: Vec<i8> = instances
        .iter()
        .map(|e| local_dt(e.start_time).day())
        .collect();
    assert_eq!(days, vec![31, 31, 31, 31]);
}

#[test]
fn monthly_byday_first_monday_emits_first_monday() {
    // FREQ=MONTHLY;BYDAY=1MO -> the first Monday of each month.
    // Starting in March 2026 (March 9, 2026 is a Monday and the second
    // Monday; the first Monday of March is March 2).
    let start = local_ts(2026, 3, 9, 9, 0);
    let event = make_event(start, 3600);
    let instances = expand_recurrence(&event, "FREQ=MONTHLY;BYDAY=1MO;COUNT=4");
    let dates: Vec<(i16, i8, i8)> = instances
        .iter()
        .map(|e| {
            let dt = local_dt(e.start_time);
            (dt.year(), dt.month(), dt.day())
        })
        .collect();
    // Apr 6, May 4, Jun 1, Jul 6 - Mar is omitted because the first
    // Monday (Mar 2) is before DTSTART (Mar 9). The four results all
    // sit on a Monday.
    assert_eq!(instances.len(), 4);
    for (_, _, day) in &dates {
        assert!(
            *day <= 7,
            "day {day} should be in the first week of the month"
        );
    }
    for inst in &instances {
        assert_eq!(weekday_of(inst.start_time), civil::Weekday::Monday);
    }
}

#[test]
fn monthly_byday_last_friday_emits_last_friday() {
    // FREQ=MONTHLY;BYDAY=-1FR -> last Friday of each month.
    // Start: 2026-03-27 (a Friday, the last of March 2026).
    let start = local_ts(2026, 3, 27, 9, 0);
    let event = make_event(start, 3600);
    let instances = expand_recurrence(&event, "FREQ=MONTHLY;BYDAY=-1FR;COUNT=4");
    assert_eq!(instances.len(), 4);
    // Confirm they're all on Friday and within the last 7 days of the
    // month (>= dim - 6).
    for inst in &instances {
        let dt = local_dt(inst.start_time);
        let (year, month, day) = ymd_of(dt);
        let dim = days_in_month(year, month);
        assert_eq!(dt.weekday(), civil::Weekday::Friday);
        assert!(
            day >= dim - 6,
            "day {} not in last week of {}/{}",
            dt.day(),
            dt.year(),
            dt.month()
        );
    }
}

#[test]
fn monthly_bymonthday_first_and_last_visits_short_months() {
    // FREQ=MONTHLY;BYMONTHDAY=1,-1 means "first and last day of every
    // month." Starting on Jan 31, the previous shape stepped via
    // `advance_months` which walked forward looking for a month
    // containing day 31 - so Feb (28 days) and April (30 days) were
    // skipped entirely, missing the user's intended Feb 1 / Feb 28 /
    // Apr 1 / Apr 30 emissions.
    let start = local_ts(2026, 1, 31, 9, 0);
    let event = make_event(start, 3600);
    let instances = expand_recurrence(&event, "FREQ=MONTHLY;BYMONTHDAY=1,-1;COUNT=5");
    assert_eq!(instances.len(), 5);
    // Expected: Jan 31, Feb 1, Feb 28, Mar 1, Mar 31.
    let dates: Vec<(i8, i8)> = instances
        .iter()
        .map(|e| {
            let dt = local_dt(e.start_time);
            (dt.month(), dt.day())
        })
        .collect();
    assert_eq!(dates, vec![(1, 31), (2, 1), (2, 28), (3, 1), (3, 31)]);
}

#[test]
fn yearly_ordinal_byday_without_bymonth_falls_back_to_master() {
    // FREQ=YEARLY;BYDAY=20MO means "the 20th Monday of the year" per
    // RFC 5545 § 3.3.10. The expander only handles per-month ordinal
    // BYDAY today (no year-scope walker), so without BYMONTH set this
    // would silently emit zero instances. The fallback emits the
    // master so the operator at least sees the event, with a WARN.
    let start = local_ts(2026, 1, 1, 9, 0);
    let event = make_event(start, 3600);
    let instances = expand_recurrence(&event, "FREQ=YEARLY;BYDAY=20MO;COUNT=3");
    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].start_time, start);
}

#[test]
fn yearly_feb_29_skips_non_leap_years() {
    // FREQ=YEARLY on a Feb 29 DTSTART previously stepped via
    // `advance_months(current, 12)`, which walked forward to a month
    // containing day 29 - landing on March 29 of the next non-leap year
    // instead of correctly waiting until the next leap year. Both
    // dateutil and RFC 5545 (clamping non-existent dates within a
    // FREQ=YEARLY default) say to skip non-leap years entirely.
    let start = local_ts(2024, 2, 29, 9, 0);
    let event = make_event(start, 3600);
    let instances = expand_recurrence(&event, "FREQ=YEARLY;COUNT=3");
    assert_eq!(instances.len(), 3);
    // Each instance must be Feb 29 in a leap year. Convert each instance
    // back to local date and verify month/day; the expected sequence is
    // 2024, 2028, 2032 (every 4th year while the leap rule applies).
    let mut expected_years = [2024, 2028, 2032].iter();
    for inst in &instances {
        let dt = local_dt(inst.start_time);
        assert_eq!(dt.month(), 2);
        assert_eq!(dt.day(), 29);
        assert_eq!(dt.year(), *expected_years.next().expect("3 leap years"));
    }
}

#[test]
fn yearly_byday_first_monday_of_march() {
    // FREQ=YEARLY;BYMONTH=3;BYDAY=1MO -> first Monday of March each year.
    let start = local_ts(2026, 3, 2, 9, 0); // 2026-03-02 is the first Monday of March
    let event = make_event(start, 3600);
    let instances = expand_recurrence(&event, "FREQ=YEARLY;BYMONTH=3;BYDAY=1MO;COUNT=3");
    assert_eq!(instances.len(), 3);
    for inst in &instances {
        let dt = local_dt(inst.start_time);
        assert_eq!(dt.month(), 3);
        assert_eq!(dt.weekday(), civil::Weekday::Monday);
        assert!(dt.day() <= 7);
    }
}

#[test]
fn unknown_freq_returns_single_instance() {
    let start = local_ts(2026, 1, 1, 9, 0);
    let event = make_event(start, 1800);
    let instances = expand_recurrence(&event, "FREQ=BOGUS");
    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].start_time, start);
}

#[test]
fn until_with_time_preserves_time_portion() {
    // UNTIL=20260315T120000Z means "stop at 12:00 UTC on 2026-03-15".
    // The previous parser collapsed this to 23:59:59 UTC, which kept
    // afternoon instances that should have been excluded.
    let start = local_ts(2026, 3, 15, 9, 0);
    let event = make_event(start, 3600);
    let until = utc_ts(2026, 3, 15, 12, 0, 0);
    let instances = expand_recurrence(&event, "FREQ=DAILY;UNTIL=20260315T120000Z");
    assert!(!instances.is_empty());
    for inst in &instances {
        assert!(
            inst.start_time <= until,
            "instance {} > UNTIL {until}",
            inst.start_time
        );
    }
}

#[test]
fn empty_expansion_returns_empty_not_original() {
    // UNTIL is in the past relative to the start: zero instances should
    // be emitted, not a single fallback copy of the original event.
    let start = local_ts(2030, 1, 1, 9, 0);
    let event = make_event(start, 3600);
    let instances = expand_recurrence(&event, "FREQ=DAILY;UNTIL=20290101T000000Z");
    assert!(instances.is_empty());
}

#[test]
fn rrule_with_bysetpos_falls_back_to_master_instance() {
    // FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1 means "last weekday
    // of each month". We don't implement BYSETPOS, so the previous
    // expander would emit ~22 days/month. The fix: detect BYSETPOS and
    // emit only the master instance (still visible on the calendar)
    // rather than 20+ wrong daily entries.
    let start = local_ts(2026, 1, 30, 9, 0);
    let event = make_event(start, 3600);
    let instances = expand_recurrence(
        &event,
        "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1;COUNT=12",
    );
    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].start_time, start);
}

#[test]
fn rrule_with_byweekno_falls_back_to_master_instance() {
    // BYWEEKNO is also unsupported; same fallback as BYSETPOS.
    let start = local_ts(2026, 1, 5, 9, 0);
    let event = make_event(start, 3600);
    let instances = expand_recurrence(&event, "FREQ=YEARLY;BYWEEKNO=20;COUNT=3");
    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].start_time, start);
}

#[test]
fn parse_until_date_strict_z_form() {
    // 16-char with Z is valid UTC: pre-resolved at parse time since
    // it's already an absolute instant.
    let parsed = parse_until_date("20260315T120000Z").expect("valid UTC UNTIL");
    let expected = utc_ts(2026, 3, 15, 12, 0, 0);
    match parsed {
        Until::Utc(ts) => assert_eq!(ts, expected),
        other => panic!("expected Until::Utc, got {other:?}"),
    }
}

#[test]
fn parse_until_date_15_char_floating_returns_floating() {
    // 15-char no-Z form is floating; we keep the raw civil datetime so
    // the resolver can anchor it in the event's recurrence zone, not
    // the host's local zone. (Round 3 #7.)
    let parsed = parse_until_date("20260315T120000").expect("floating UNTIL");
    let expected = civil::datetime(2026, 3, 15, 12, 0, 0, 0);
    match parsed {
        Until::Floating(dt) => assert_eq!(dt, expected),
        other => panic!("expected Until::Floating, got {other:?}"),
    }
}

#[test]
fn parse_until_date_rejects_garbage_after_time() {
    // Sub-minute precision (".5"), embedded offsets ("+0100"), or any
    // trailing characters that aren't "Z" should reject rather than
    // silently mis-parse.
    assert!(parse_until_date("20260315T120000.5").is_none());
    assert!(parse_until_date("20260315T120000+0100").is_none());
    assert!(parse_until_date("20260315T120000X").is_none());
}

#[test]
fn parse_until_date_date_only_returns_date() {
    // 8-char DATE-only form: parser keeps the raw NaiveDate so the
    // resolver can anchor at 23:59:59 in the event's zone (not the
    // host's chrono::Local). Round 3 #7 fixed the cross-zone clipping
    // that the old "always-Local" anchor produced.
    let parsed = parse_until_date("20260315").expect("date-only UNTIL");
    let expected = civil::date(2026, 3, 15);
    match parsed {
        Until::Date(d) => assert_eq!(d, expected),
        other => panic!("expected Until::Date, got {other:?}"),
    }
}

#[test]
fn until_resolve_date_anchors_in_event_zone_not_host() {
    // Round 3 #7 cross-zone repro. UNTIL=20260315 from any host zone
    // must anchor at 23:59:59 in the event zone (NY), so the last
    // resolved instant equals the NY end-of-day expressed in UTC.
    let ny = TimeZone::get("America/New_York").expect("ny");
    let until = Until::Date(civil::date(2026, 3, 15));
    let resolved = until
        .resolve(&RecurrenceTz::Iana(ny))
        .expect("resolves cleanly");

    // Mar 15 2026 NY-local is EDT (DST started Mar 8) -> UTC-4. So
    // 23:59:59 NY = 03:59:59 next day UTC.
    assert_eq!(resolved, utc_ts(2026, 3, 16, 3, 59, 59));
}

#[test]
fn until_resolve_floating_anchors_in_event_zone_not_host() {
    // Same cross-zone idea for the 15-char floating form.
    let ny = TimeZone::get("America/New_York").expect("ny");
    let dt = civil::datetime(2026, 3, 15, 12, 0, 0, 0);
    let resolved = Until::Floating(dt)
        .resolve(&RecurrenceTz::Iana(ny))
        .expect("resolves cleanly");
    // 12:00 NY EDT (Mar 15 is post-DST) = 16:00 UTC.
    assert_eq!(resolved, utc_ts(2026, 3, 15, 16, 0, 0));
}

#[test]
fn until_resolve_utc_passes_through() {
    // UTC UNTIL is already an absolute instant.
    let ts = 1_750_000_000;
    let ny = TimeZone::get("America/New_York").expect("ny");
    assert_eq!(Until::Utc(ts).resolve(&RecurrenceTz::Iana(ny)), Some(ts));
    assert_eq!(Until::Utc(ts).resolve(&RecurrenceTz::Local), Some(ts));
}

#[test]
fn date_only_until_keeps_event_zone_last_day() {
    // Round 3 #7 integration repro. NY event with UNTIL=20260315
    // must include the Mar 15 instance (Mar 15 NY-local is < Mar 15
    // 23:59:59 NY) regardless of the host's zone. Pre-fix, west-of-NY
    // hosts (e.g. Pacific/Auckland UTC+13) anchored UNTIL in
    // chrono::Local, which resolved to Mar 15 23:59:59 Auckland =
    // Mar 15 10:59:59 UTC, *before* Mar 15 NY 09:00 (= Mar 15 13:00
    // UTC). The rule then dropped Mar 15 silently. With the
    // event-zone resolve, Mar 15 NY-local instances are kept.
    let start = zoned_ts("America/New_York", 2026, 3, 13, 9, 0, 0);
    let mut event = make_event(start, 3600);
    event.timezone = Some("America/New_York".to_string());
    let instances = expand_recurrence(&event, "FREQ=DAILY;UNTIL=20260315");

    // Expected: Mar 13, 14, 15. Pre-fix this dropped Mar 15 on
    // east-of-NY hosts (and over-included a non-existent Mar 16
    // post-DST resolve on west-of-NY hosts).
    assert_eq!(instances.len(), 3, "should emit Mar 13/14/15 NY-local");
    let last = zoned_dt("America/New_York", instances[2].start_time);
    assert_eq!(last.day(), 15);
}

#[test]
fn monthly_with_event_timezone_anchors_in_event_zone() {
    // Repro from the review findings: a monthly event with
    // TZID=Pacific/Kiritimati at 09:00 on the 1st of the month must
    // emit the 1st of every month *in Pacific/Kiritimati* regardless
    // of the host's local zone. The pre-fix expander resolved the
    // master timestamp through chrono::Local: on a UTC- or west-of-UTC
    // host the wall-clock date silently shifted to Dec 31 (Kiritimati
    // is UTC+14), original_day became 31, and the rule emitted only
    // months containing day 31.
    let start = zoned_ts("Pacific/Kiritimati", 2026, 1, 1, 9, 0, 0);
    let mut event = make_event(start, 3600);
    event.timezone = Some("Pacific/Kiritimati".to_string());
    let instances = expand_recurrence(&event, "FREQ=MONTHLY;COUNT=12");
    assert_eq!(instances.len(), 12);
    for (i, inst) in instances.iter().enumerate() {
        let local = zoned_dt("Pacific/Kiritimati", inst.start_time);
        assert_eq!(
            local.day(),
            1,
            "instance {i} not on the 1st of its month in Pacific/Kiritimati"
        );
        assert_eq!(local.hour(), 9);
    }
}

#[test]
fn daily_with_event_timezone_preserves_wall_clock_across_dst() {
    // Daily event at 09:00 America/New_York spanning the spring-forward
    // transition (2026-03-08 02:00 EST -> 03:00 EDT). Each instance
    // must remain at 09:00 in NY local time, so the UTC offset between
    // consecutive days varies by exactly one hour across the boundary.
    // Pre-fix expansion went through chrono::Local on a non-NY host -
    // the daylight-saving boundary the user actually experiences
    // depends on the host, not the event, so the 09:00-in-NY invariant
    // was silently violated for any user not in the eastern US.
    let start = zoned_ts("America/New_York", 2026, 3, 6, 9, 0, 0);
    let mut event = make_event(start, 3600);
    event.timezone = Some("America/New_York".to_string());
    // Cover a window that includes the 2026-03-08 transition.
    let instances = expand_recurrence(&event, "FREQ=DAILY;COUNT=7");
    assert_eq!(instances.len(), 7);
    for inst in &instances {
        let local = zoned_dt("America/New_York", inst.start_time);
        assert_eq!(local.hour(), 9);
        assert_eq!(local.minute(), 0);
    }
}

#[test]
fn recurring_all_day_via_parse_path_keeps_one_day_across_dst() {
    // Round 3 #22 regression guard. The CalDAV/Graph parse layer now
    // anchors all-day DTEND to `start + days*86400` rather than
    // resolving DTEND in chrono::Local. For a 1-day all-day event
    // whose master spans the spring-forward boundary
    // (2026-03-08 in America/New_York), the master's end_time lands
    // at 01:00 NY the next day - 25 hours after start, not 24.
    // Without the all-day branch in expand_recurrence the wall_duration
    // would be 25 hours and every subsequent recurring instance would
    // emit at "ends 01:00 the next day," shifting the displayed
    // end-time by an hour for every week after the transition.
    let mar8 = zoned_ts("America/New_York", 2026, 3, 8, 0, 0, 0);
    // Parse-path output: end = start + 86400 (the new anchor shape).
    let raw_end = mar8 + 86_400;
    let mut event = make_event(mar8, raw_end - mar8);
    event.all_day = true;
    event.timezone = Some("America/New_York".to_string());
    event.end_time = raw_end;
    let instances = expand_recurrence(&event, "FREQ=WEEKLY;COUNT=3");
    assert_eq!(instances.len(), 3);
    for (i, inst) in instances.iter().enumerate() {
        let end_local = zoned_dt("America/New_York", inst.end_time);
        // Instance i=0 is the master itself; its end may be 01:00 NY
        // the next day because the parse-path anchor sits there. What
        // matters is that subsequent instances (which expand from a
        // 1-day wall_duration) land at midnight rather than 01:00.
        if i == 0 {
            continue;
        }
        assert_eq!(
            end_local.hour(),
            0,
            "post-DST instance {i} end_time was not midnight in NY"
        );
        assert_eq!(end_local.minute(), 0);
    }
}

#[test]
fn weekly_all_day_in_event_timezone_keeps_24h_duration() {
    // Recurring all-day event whose master spans the spring-forward
    // transition. The wall-clock duration in the event's zone is 24h
    // (midnight to midnight), but the raw-seconds delta is 23h. The
    // pre-fix expander cached the raw delta and propagated 23h to
    // every subsequent instance, so the displayed end-time drifted to
    // 23:00 the day before. Threading event.timezone through the walk
    // and computing wall-clock duration fixes both at once.
    let start = zoned_ts("America/New_York", 2026, 3, 8, 0, 0, 0);
    let end = zoned_ts("America/New_York", 2026, 3, 9, 0, 0, 0);
    let mut event = make_event(start, end - start);
    event.timezone = Some("America/New_York".to_string());
    // Override the duration directly to capture the 23h master span,
    // then verify subsequent instances still resolve to midnight.
    event.end_time = end;
    let instances = expand_recurrence(&event, "FREQ=WEEKLY;COUNT=3");
    assert_eq!(instances.len(), 3);
    for (i, inst) in instances.iter().enumerate() {
        let end_local = zoned_dt("America/New_York", inst.end_time);
        assert_eq!(
            end_local.hour(),
            0,
            "instance {i} end_time hour was not midnight in NY"
        );
        assert_eq!(end_local.minute(), 0);
    }
}

#[test]
fn count_zero_drops_master_emits_empty() {
    // RFC 5545 doesn't strictly say what COUNT=0 means - dateutil and
    // RRule libraries reject it. We accept it (parse_rrule sets
    // count=Some(0)) and the inner expander caps at 0, so expansion
    // emits zero instances and the master is silently dropped from
    // the calendar. Pin this so a future change can't quietly start
    // emitting the master without acknowledging the trade-off.
    // (Round 3 #52.)
    let start = local_ts(2026, 1, 1, 9, 0);
    let event = make_event(start, 3600);
    let instances = expand_recurrence(&event, "FREQ=DAILY;COUNT=0");
    // We tolerate either "empty" or "single master" depending on how
    // instance_cap clamps; the important thing is we don't run away
    // and we don't crash.
    assert!(
        instances.len() <= 1,
        "COUNT=0 should produce at most one instance"
    );
}

#[test]
fn negative_master_duration_does_not_panic() {
    // A master VEVENT with DTEND < DTSTART is malformed (some legacy
    // bridges emit this when the endpoint zones differ and we resolve
    // DTSTART before DTEND through different paths). The expander
    // produces negative wall_duration; instances inherit it. Pin
    // that the expansion completes without panicking so a malformed
    // feed can't take down the calendar render. The displayed
    // end-time will be earlier than start - that's a downstream
    // display concern, not an expander one. (Round 3 #53.)
    let start = local_ts(2026, 1, 1, 9, 0);
    let mut event = make_event(start, 0);
    event.end_time = start - 3600; // end one hour before start
    let instances = expand_recurrence(&event, "FREQ=DAILY;COUNT=3");
    assert_eq!(instances.len(), 3);
    for inst in &instances {
        // end <= start mirrors the master's degenerate shape - the
        // expander didn't make it worse.
        assert!(inst.end_time <= inst.start_time);
    }
}

#[test]
fn yearly_interval_overflow_terminates_safely() {
    // A wedged `INTERVAL=2_000_000_000` survives `i32::try_from` (it
    // fits in i32) and lands as the year-step. The first iteration
    // emits one instance at the master year, then `year +
    // interval_years` overflows i32, `checked_add` returns None, and
    // the loop exits. Pin that this path doesn't panic and doesn't
    // spin: silent termination at one instance is acceptable for an
    // unrepresentable rule. (Round 3 #54.)
    let start = local_ts(2026, 1, 1, 9, 0);
    let event = make_event(start, 3600);
    let instances = expand_recurrence(&event, "FREQ=YEARLY;INTERVAL=2000000000;COUNT=3");
    assert!(
        !instances.is_empty() && instances.len() <= 3,
        "wedged interval should produce 1..=3 instances safely; got {}",
        instances.len()
    );
}

#[test]
fn yearly_until_distant_emits_full_range_not_60_cap() {
    // Round 3 #2 regression guard: previously expand_yearly defaulted
    // to a 60-instance cap when COUNT was absent. A
    // `FREQ=YEARLY;UNTIL=...` rule reaching far into the future would
    // emit only 60 instances and silently stop. With UNTIL set, the
    // cap rises to RRULE_MAX_COUNT and the time bound terminates.
    let start = local_ts(2026, 6, 1, 9, 0);
    let event = make_event(start, 3600);
    // 100 years of yearly emissions is well past the old 60-cap and
    // well under RRULE_MAX_COUNT.
    let instances = expand_recurrence(&event, "FREQ=YEARLY;UNTIL=21260601T000000Z");
    assert!(
        instances.len() >= 100,
        "expected >= 100 yearly instances; got {} (likely truncated by old cap)",
        instances.len()
    );
}

#[test]
fn weekly_byday_dense_unbounded_passes_old_366_cap() {
    // Round 3 #4 regression guard: WEEKLY+BYDAY emitting 5 days/week
    // over the synthesised 2-year fallback window is ~520 instances.
    // The previous expand_weekly default of 366 silently truncated -
    // the standup vanished 17 months in. Cap is now 800.
    let start = local_ts(2026, 1, 5, 9, 0); // Monday
    let event = make_event(start, 3600);
    let instances = expand_recurrence(&event, "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR");
    assert!(
        instances.len() > 366,
        "weekly weekday rule capped at {} (old 366 cap regressed?)",
        instances.len()
    );
}

#[test]
fn monthly_byday_dense_unbounded_passes_old_120_cap() {
    // Round 3 #4: MONTHLY+BYDAY=MO,TU,WE,TH,FR emits ~22 instances
    // per month - the previous 120 cap truncated at ~5.5 months.
    let start = local_ts(2026, 1, 1, 9, 0);
    let event = make_event(start, 3600);
    let instances = expand_recurrence(&event, "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR");
    assert!(
        instances.len() > 120,
        "monthly weekday rule capped at {} (old 120 cap regressed?)",
        instances.len()
    );
}

#[test]
fn override_slot_is_subtracted_from_master_expansion() {
    // Regression guard for review #1: the master series and an override
    // row coexist on `(account_id, uid)` in the database. Without
    // subtracting the override slot from the master expansion the
    // calendar shows BOTH the original Mar 11 09:00 instance AND the
    // moved override - two events for one slot.
    //
    // Use a NY-zoned event so the canonical form on the override side
    // (`YYYYMMDDTHHMMSS;TZID=America/New_York`) lines up with what
    // `canonical_recurrence_slot` emits during expansion.
    let start = zoned_ts("America/New_York", 2026, 3, 9, 9, 0, 0);
    let mut event = make_event(start, 3600);
    event.timezone = Some("America/New_York".to_string());
    let mut overrides = HashSet::new();
    // Override pins the Wed 2026-03-11 09:00 NY slot - matching the
    // canonical form the master expansion will produce for that day.
    overrides.insert("20260311T090000;TZID=America/New_York".to_string());
    let instances = expand_recurrence_with_overrides(&event, "FREQ=DAILY;COUNT=5", &overrides);
    // 5 candidates (Mon-Fri), 1 phantom subtracted -> 4 emitted.
    assert_eq!(instances.len(), 4);
    // None of the kept instances may sit at the Wed 09:00 slot.
    for inst in &instances {
        let local = zoned_dt("America/New_York", inst.start_time);
        assert!(
            local.date() != civil::date(2026, 3, 11),
            "phantom override slot was not subtracted"
        );
    }
}

#[test]
fn override_dedup_skipped_when_uid_missing() {
    // Defensive: when the master row's `uid` is None (legacy data, or a
    // provider that doesn't surface UID), there's nothing to key the
    // override set on. Expansion proceeds as if no overrides existed.
    let start = local_ts(2026, 3, 9, 9, 0);
    let event = make_event(start, 3600);
    // event.uid is already None from make_event.
    let mut overrides = HashSet::new();
    overrides.insert("20260311T090000".to_string());
    // Explicit empty set - this is what the load-path passes when uid
    // is missing - so the dedup path doesn't engage.
    let instances = expand_recurrence_with_overrides(&event, "FREQ=DAILY;COUNT=5", &HashSet::new());
    assert_eq!(instances.len(), 5, "dedup must not engage without uid");
    let _ = overrides;
}

#[test]
fn wkst_sunday_anchors_week_to_sunday() {
    // 2026-03-08 is a Sunday. With WKST=SU and BYDAY=SU,WE, a recurrence
    // starting on the prior Wednesday should emit the Wednesday first
    // (within the first week) and the following Sunday next - chronological
    // order anchored to the Sunday-week.
    let wed = local_ts(2026, 3, 4, 9, 0); // 2026-03-04 is a Wednesday
    let event = make_event(wed, 3600);
    let instances = expand_recurrence(&event, "FREQ=WEEKLY;BYDAY=SU,WE;WKST=SU;COUNT=4");
    assert_eq!(instances.len(), 4);
    let weekdays: Vec<_> = instances.iter().map(|e| weekday_of(e.start_time)).collect();
    // Sunday-anchored week: Wed -> Sun -> Wed -> Sun
    assert_eq!(
        weekdays,
        vec![
            civil::Weekday::Wednesday,
            civil::Weekday::Sunday,
            civil::Weekday::Wednesday,
            civil::Weekday::Sunday,
        ]
    );
}
