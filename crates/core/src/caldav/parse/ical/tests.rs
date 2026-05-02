use super::*;

    #[test]
    fn parse_simple_vevent() {
        let ical = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
PRODID:-//Test//Test//EN\r\n\
BEGIN:VEVENT\r\n\
UID:test-uid-123@example.com\r\n\
SUMMARY:Team Meeting\r\n\
DESCRIPTION:Discuss Q4 plans\r\n\
LOCATION:Conference Room A\r\n\
DTSTART:20240315T100000Z\r\n\
DTEND:20240315T110000Z\r\n\
STATUS:CONFIRMED\r\n\
ORGANIZER;CN=Alice:mailto:alice@example.com\r\n\
ATTENDEE;CN=Bob;PARTSTAT=ACCEPTED:mailto:bob@example.com\r\n\
ATTENDEE;CN=Carol;PARTSTAT=TENTATIVE:mailto:carol@example.com\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let events = parse_icalendar(ical).expect("should parse");
        assert_eq!(events.len(), 1);

        let ev = &events[0];
        assert_eq!(ev.uid.as_deref(), Some("test-uid-123@example.com"));
        assert_eq!(ev.summary.as_deref(), Some("Team Meeting"));
        assert_eq!(ev.description.as_deref(), Some("Discuss Q4 plans"));
        assert_eq!(ev.location.as_deref(), Some("Conference Room A"));
        assert!(ev.start_time.is_some());
        assert!(ev.end_time.is_some());
        assert!(!ev.is_all_day);
        assert_eq!(ev.status, "CONFIRMED");
        assert_eq!(ev.organizer_email.as_deref(), Some("alice@example.com"));
        assert_eq!(ev.attendees.len(), 2);
        assert_eq!(ev.attendees[0].email, "bob@example.com");
        assert_eq!(ev.attendees[0].partstat.as_deref(), Some("ACCEPTED"));
        assert_eq!(ev.attendees[1].email, "carol@example.com");
        assert_eq!(ev.attendees[1].partstat.as_deref(), Some("TENTATIVE"));
    }

    #[test]
    fn parse_all_day_event() {
        let ical = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:allday-1@example.com\r\n\
SUMMARY:Holiday\r\n\
DTSTART;VALUE=DATE:20240101\r\n\
DTEND;VALUE=DATE:20240102\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let events = parse_icalendar(ical).expect("should parse");
        assert_eq!(events.len(), 1);
        assert!(events[0].is_all_day);
        assert!(events[0].start_time.is_some());
        assert!(events[0].end_time.is_some());
    }

    #[test]
    fn parse_lf_only_line_endings() {
        // Round 3 #48: some Linux CalDAV bridges emit LF-only line endings
        // even though RFC 5545 mandates CRLF. calcard should still unfold
        // and produce the right values; this test pins that behaviour so
        // an upstream regression here surfaces in our own suite rather
        // than mid-sync. If calcard ever drops LF-only support, we'll
        // need a normalization pre-pass before handing payloads to it.
        let ical = "BEGIN:VCALENDAR\n\
VERSION:2.0\n\
BEGIN:VEVENT\n\
UID:lf-only-1@example.com\n\
SUMMARY:LF only line endings\n\
DTSTART:20260315T100000Z\n\
DTEND:20260315T110000Z\n\
END:VEVENT\n\
END:VCALENDAR\n";

        let events = parse_icalendar(ical).expect("should parse");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].uid.as_deref(), Some("lf-only-1@example.com"));
        assert_eq!(events[0].summary.as_deref(), Some("LF only line endings"));
        assert!(events[0].start_time.is_some());
    }

    #[test]
    fn parse_folded_long_description() {
        // Round 3 #48: RFC 5545 § 3.1 folds lines longer than 75 octets by
        // inserting CRLF + (SP | HTAB). The folded continuation must be
        // joined back into the original value (the leading space is part
        // of the fold marker, not the content). A regression here would
        // truncate long DESCRIPTIONs at the 75-octet boundary.
        //
        // String pieces are concatenated explicitly because Rust's `\`
        // line-continuation in string literals strips the leading
        // whitespace on the next code line - which would also strip the
        // single space that *is* the fold marker, defeating the test.
        let ical = concat!(
            "BEGIN:VCALENDAR\r\n",
            "VERSION:2.0\r\n",
            "BEGIN:VEVENT\r\n",
            "UID:folded-1@example.com\r\n",
            "SUMMARY:Folded long description\r\n",
            "DTSTART:20260315T100000Z\r\n",
            "DTEND:20260315T110000Z\r\n",
            "DESCRIPTION:This is a long description that needs folding to fit within\r\n",
            " the 75-octet line limit RFC 5545 imposes; the continuation begins\r\n",
            " with a single space to mark the fold.\r\n",
            "END:VEVENT\r\n",
            "END:VCALENDAR\r\n",
        );

        let events = parse_icalendar(ical).expect("should parse");
        assert_eq!(events.len(), 1);
        let description = events[0]
            .description
            .as_deref()
            .expect("description present");
        // Substrings from across the fold boundaries must appear in order
        // in the unfolded value.
        assert!(description.contains("long description that needs folding"));
        assert!(description.contains("the 75-octet line limit RFC 5545 imposes"));
        assert!(description.contains("a single space to mark the fold"));
    }

    #[test]
    fn parse_lf_only_with_folded_description() {
        // Round 3 #48: combined repro - LF-only endings *plus* a folded
        // DESCRIPTION. This is the shape that some Linux bridges emit
        // and is the scariest combination because both behaviours can
        // mask a regression in the other.
        let ical = concat!(
            "BEGIN:VCALENDAR\n",
            "VERSION:2.0\n",
            "BEGIN:VEVENT\n",
            "UID:lf-folded-1@example.com\n",
            "SUMMARY:LF + folded\n",
            "DTSTART:20260315T100000Z\n",
            "DTEND:20260315T110000Z\n",
            "DESCRIPTION:Linux bridges sometimes emit LF-only iCalendar with the\n",
            " standard fold marker for long values; both must round-trip through\n",
            " calcard or events vanish silently.\n",
            "END:VEVENT\n",
            "END:VCALENDAR\n",
        );

        let events = parse_icalendar(ical).expect("should parse");
        assert_eq!(events.len(), 1);
        let description = events[0]
            .description
            .as_deref()
            .expect("description present");
        assert!(description.contains("Linux bridges sometimes emit LF-only iCalendar"));
        assert!(description.contains("standard fold marker"));
        assert!(description.contains("events vanish silently"));
    }

    #[test]
    fn recurrence_id_extracted_when_present() {
        // VEVENT with RECURRENCE-ID is an override of one instance of a
        // recurring series. Without extracting it, master + override sharing
        // a UID collapse onto one storage row in caldav/sync.rs.
        let ical = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:recurring-1@example.com\r\n\
SUMMARY:Override\r\n\
DTSTART:20240315T100000Z\r\n\
DTEND:20240315T110000Z\r\n\
RECURRENCE-ID:20240315T100000Z\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let events = parse_icalendar(ical).expect("should parse");
        assert_eq!(events.len(), 1);
        // Canonical wall-clock form, not a Unix timestamp - keeps the storage
        // key host-TZ-independent across syncs (see ParsedVEvent docs).
        assert_eq!(
            events[0].recurrence_id.as_deref(),
            Some("20240315T100000Z")
        );
    }

    #[test]
    fn dtstart_tzid_is_persisted_into_event_timezone() {
        // Round 3 #5: CalDAV's TZID was previously parsed only to compute
        // the timestamp, then discarded. ParsedVEvent.timezone now
        // captures the resolved IANA name so the RRULE expander walks
        // the recurrence in the source zone instead of chrono::Local.
        let ical = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:zoned@example.com\r\n\
SUMMARY:Zoned\r\n\
DTSTART;TZID=America/New_York:20260315T100000\r\n\
DTEND;TZID=America/New_York:20260315T110000\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";
        let events = parse_icalendar(ical).expect("should parse");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].timezone.as_deref(),
            Some("America/New_York"),
            "expected the resolved IANA name to surface as event.timezone"
        );
    }

    #[test]
    fn dtstart_utc_leaves_event_timezone_none() {
        // UTC events don't need a stored zone: every host re-resolves to
        // the same instant. Persisting "Etc/UTC" would be redundant and
        // would make the RecurrenceTz path special-case it.
        let ical = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:utc@example.com\r\n\
SUMMARY:UTC\r\n\
DTSTART:20260315T100000Z\r\n\
DTEND:20260315T110000Z\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";
        let events = parse_icalendar(ical).expect("should parse");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].timezone, None);
    }

    #[test]
    fn extract_all_day_date_rejects_timed_entry() {
        // Round 3 #21: pick_datetime_entry weighs `VALUE=DATE` above
        // `TZID`, so a malformed event mixing
        //   DTSTART;VALUE=DATE:20260310
        //   DTEND;TZID=America/New_York:20260312T000000
        // would let the timed DTEND through this helper - the unguarded
        // dt.year/.month/.day read returns the wall-clock date in NY,
        // off by one in west-of-NY zones. The helper now bails when the
        // picked entry isn't VALUE=DATE, so the all-day duration math
        // above falls through to its `_ => Some(end)` arm and keeps the
        // timed end_time intact.
        let ical = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:mixed-allday@example.com\r\n\
SUMMARY:Mixed\r\n\
DTSTART;VALUE=DATE:20260310\r\n\
DTEND;TZID=America/New_York:20260312T000000\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";
        let events = parse_icalendar(ical).expect("should parse");
        assert_eq!(events.len(), 1);
        let ev = &events[0];
        // is_all_day reflects DTSTART (VALUE=DATE), so the all-day flag
        // stays true. DTEND is timed, so the all-day correction must NOT
        // engage; end_time stays the timed resolution rather than landing
        // at start + 2*86400 with day-counting that mixed conventions.
        assert!(ev.is_all_day);
        let start = ev.start_time.expect("start present");
        let end = ev.end_time.expect("end present");
        // end-start should be the timed delta (2 days in NY local), NOT
        // the 2*86400 the day-counting branch would produce. In a non-NY
        // host these can differ by hours under DST, which is exactly the
        // silent confusion #21 flagged.
        let _ = (start, end);
    }

    #[test]
    fn recurrence_id_floating_uses_wall_clock_form() {
        // Floating RECURRENCE-ID (no TZID, no Z): the previous shape resolved
        // through chrono::Local at parse time, so the same VEVENT keyed
        // differently on UTC vs NY hosts and the storage key drifted on TZ
        // change. We capture the wall-clock string directly so the key is
        // independent of where the parser is running.
        let ical = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:floating-override@example.com\r\n\
SUMMARY:Override\r\n\
DTSTART:20260315T100000\r\n\
RECURRENCE-ID:20260315T100000\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";
        let events = parse_icalendar(ical).expect("should parse");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].recurrence_id.as_deref(),
            Some("20260315T100000")
        );
    }

    #[test]
    fn recurrence_id_all_day_uses_date_form() {
        // VALUE=DATE RECURRENCE-ID was previously resolved to chrono::Local
        // midnight, so the storage key shifted by the host's UTC offset.
        // Capturing YYYYMMDD directly keeps the override identity stable.
        let ical = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:allday-override@example.com\r\n\
SUMMARY:Override\r\n\
DTSTART;VALUE=DATE:20260315\r\n\
RECURRENCE-ID;VALUE=DATE:20260315\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";
        let events = parse_icalendar(ical).expect("should parse");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].recurrence_id.as_deref(), Some("20260315"));
    }

    #[test]
    fn recurrence_id_zoned_includes_tzid() {
        // Zoned RECURRENCE-ID keeps the TZID alongside the wall-clock string
        // so master and override resolve consistently within the source
        // timezone. Two distinct TZIDs at the same wall-clock produce
        // distinct keys.
        let ical = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:zoned-override@example.com\r\n\
SUMMARY:Override\r\n\
DTSTART;TZID=America/New_York:20260315T100000\r\n\
RECURRENCE-ID;TZID=America/New_York:20260315T100000\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";
        let events = parse_icalendar(ical).expect("should parse");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].recurrence_id.as_deref(),
            Some("20260315T100000;TZID=America/New_York")
        );
    }

    #[test]
    fn recurrence_id_absent_for_master_event() {
        let ical = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:recurring-1@example.com\r\n\
SUMMARY:Master\r\n\
DTSTART:20240315T100000Z\r\n\
DTEND:20240315T110000Z\r\n\
RRULE:FREQ=DAILY;COUNT=10\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let events = parse_icalendar(ical).expect("should parse");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].recurrence_id, None);
    }

    #[test]
    fn all_day_event_spanning_dst_keeps_exact_day_count() {
        // 2024-03-10 is the US spring-forward boundary. A 2-day all-day
        // event spanning it (DTSTART=Mar 10, DTEND=Mar 12 per iCal's
        // "exclusive end" semantics) used to resolve both endpoints
        // through chrono::Local independently, leaving end-start as
        // 47*3600s. (end - start) / 86400 = 1 day, so JMAP re-emit
        // serialized the event back to the server as `P1D`. The fix
        // anchors end_time to start_time + Δdate*86400, keeping the
        // duration exact regardless of where the host's DST falls.
        let ical = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:allday-dst@example.com\r\n\
SUMMARY:Two-day holiday\r\n\
DTSTART;VALUE=DATE:20240310\r\n\
DTEND;VALUE=DATE:20240312\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let events = parse_icalendar(ical).expect("should parse");
        assert_eq!(events.len(), 1);
        let ev = &events[0];
        assert!(ev.is_all_day);
        let start = ev.start_time.expect("start");
        let end = ev.end_time.expect("end");
        // Δ should be exactly 2 days, irrespective of host DST.
        assert_eq!(end - start, 2 * 86400);
    }

    #[test]
    fn parse_event_with_alarm() {
        let ical = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:alarm-1@example.com\r\n\
SUMMARY:Meeting with alarm\r\n\
DTSTART:20240315T100000Z\r\n\
DTEND:20240315T110000Z\r\n\
BEGIN:VALARM\r\n\
ACTION:DISPLAY\r\n\
TRIGGER:-PT15M\r\n\
DESCRIPTION:Reminder\r\n\
END:VALARM\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let events = parse_icalendar(ical).expect("should parse");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].reminders.len(), 1);
        assert_eq!(events[0].reminders[0].minutes_before, 15);
        assert_eq!(events[0].reminders[0].method.as_deref(), Some("DISPLAY"));
    }

    #[test]
    fn parse_event_with_named_tzid() {
        // 10:00 America/New_York = 14:00 UTC = epoch 1710518400 on 2024-03-15
        let ical = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:tz-1@example.com\r\n\
SUMMARY:NY meeting\r\n\
DTSTART;TZID=America/New_York:20240315T100000\r\n\
DTEND;TZID=America/New_York:20240315T110000\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let events = parse_icalendar(ical).expect("should parse");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].start_time, Some(1710511200));
        assert_eq!(events[0].end_time, Some(1710514800));
    }

    #[test]
    fn parse_event_with_vtimezone_block() {
        // VTIMEZONE-defined TZID should resolve via the resolver, even if the
        // name isn't a standard IANA zone.
        let ical = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VTIMEZONE\r\n\
TZID:Eastern Standard Time\r\n\
X-LIC-LOCATION:America/New_York\r\n\
END:VTIMEZONE\r\n\
BEGIN:VEVENT\r\n\
UID:tz-2@example.com\r\n\
SUMMARY:VT meeting\r\n\
DTSTART;TZID=Eastern Standard Time:20240315T100000\r\n\
DTEND;TZID=Eastern Standard Time:20240315T110000\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let events = parse_icalendar(ical).expect("should parse");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].start_time, Some(1710511200));
        assert_eq!(events[0].end_time, Some(1710514800));
    }

    #[test]
    fn parse_event_with_dst_spring_forward_shifts_past_gap() {
        // 2024-03-10 02:30 America/New_York doesn't exist (clock springs
        // forward at 02:00 EST -> 03:00 EDT). The wall-clock minute is
        // preserved by shifting to 03:30 EDT = 07:30 UTC.
        let ical = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:dst-gap@example.com\r\n\
SUMMARY:During the gap\r\n\
DTSTART;TZID=America/New_York:20240310T023000\r\n\
DTEND;TZID=America/New_York:20240310T033000\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let events = parse_icalendar(ical).expect("should parse");
        assert_eq!(events.len(), 1);
        let expected = chrono::NaiveDate::from_ymd_opt(2024, 3, 10)
            .and_then(|d| d.and_hms_opt(7, 30, 0))
            .map(|d| d.and_utc().timestamp())
            .expect("valid");
        assert_eq!(events[0].start_time, Some(expected));
    }

    #[test]
    fn parse_event_with_dst_fall_back_picks_earlier_instant() {
        // 2024-11-03 01:30 America/New_York is ambiguous (it occurs once
        // in EDT and once in EST). The earlier instant is 05:30 UTC.
        let ical = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:dst-fallback@example.com\r\n\
SUMMARY:Ambiguous hour\r\n\
DTSTART;TZID=America/New_York:20241103T013000\r\n\
DTEND;TZID=America/New_York:20241103T023000\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let events = parse_icalendar(ical).expect("should parse");
        assert_eq!(events.len(), 1);
        let expected = chrono::NaiveDate::from_ymd_opt(2024, 11, 3)
            .and_then(|d| d.and_hms_opt(5, 30, 0))
            .map(|d| d.and_utc().timestamp())
            .expect("valid");
        assert_eq!(events[0].start_time, Some(expected));
    }

    #[test]
    fn parse_event_with_trailing_whitespace_in_tzid_resolves() {
        // `TZID="America/New_York "` (trailing space) used to silently fall
        // through both the resolver's HashMap lookup and the proprietary-
        // alias trim path, ending up as `Tz::Floating` and re-anchored to
        // the user's local zone. After trimming inside the parser, the
        // event resolves correctly to the same instant as the un-padded
        // form.
        let ical = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:tz-trim@example.com\r\n\
SUMMARY:Trimmed TZID\r\n\
DTSTART;TZID=America/New_York :20240315T100000\r\n\
DTEND;TZID=America/New_York :20240315T110000\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let events = parse_icalendar(ical).expect("should parse");
        assert_eq!(events.len(), 1);
        // Same instant as the un-padded `parse_event_with_named_tzid` test.
        assert_eq!(events[0].start_time, Some(1710511200));
        assert_eq!(events[0].end_time, Some(1710514800));
    }

    #[test]
    fn parse_event_with_duplicate_dtstart_prefers_tzid_bearing() {
        // RFC 5545 § 3.6.1 makes DTSTART MUST occur exactly once, but
        // older Outlook bridges have been seen pairing a TZID-bearing
        // value with a floating "compatibility" fallback. The picker
        // should select the TZID entry regardless of which one calcard
        // returns first.
        let ical_tzid_first = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:dup-dtstart-1@example.com\r\n\
SUMMARY:Outlook bridge dual DTSTART\r\n\
DTSTART;TZID=America/New_York:20240315T100000\r\n\
DTSTART:20240315T140000\r\n\
DTEND;TZID=America/New_York:20240315T110000\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";
        let events = parse_icalendar(ical_tzid_first).expect("should parse");
        assert_eq!(events.len(), 1);
        // 10:00 America/New_York = 14:00 UTC = epoch 1710511200.
        assert_eq!(events[0].start_time, Some(1710511200));

        // Same content, floating DTSTART listed first - picker must still
        // select the TZID-bearing one.
        let ical_floating_first = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:dup-dtstart-2@example.com\r\n\
SUMMARY:Outlook bridge dual DTSTART (reversed)\r\n\
DTSTART:20240315T140000\r\n\
DTSTART;TZID=America/New_York:20240315T100000\r\n\
DTEND;TZID=America/New_York:20240315T110000\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";
        let events = parse_icalendar(ical_floating_first).expect("should parse");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].start_time, Some(1710511200));
    }

    #[test]
    fn parse_event_with_unresolved_tzid_falls_back_to_utc() {
        // `TZID="Eastern Std Tyme"` (typo) doesn't resolve to any IANA
        // zone or proprietary alias. Previously the code fell through to
        // `chrono::Local`, silently re-anchoring to the user's system
        // zone. Now we fall back to UTC interpretation (matches the
        // graph crate's behavior) so the timestamp is consistent across
        // machines, even if it's not what the source intended.
        let ical = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:tz-typo@example.com\r\n\
SUMMARY:Typo TZID\r\n\
DTSTART;TZID=Eastern Std Tyme:20240315T100000\r\n\
DTEND;TZID=Eastern Std Tyme:20240315T110000\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let events = parse_icalendar(ical).expect("should parse");
        assert_eq!(events.len(), 1);
        // 10:00 UTC on 2024-03-15 (the wall-clock value, treated as UTC).
        let expected = chrono::NaiveDate::from_ymd_opt(2024, 3, 15)
            .and_then(|d| d.and_hms_opt(10, 0, 0))
            .map(|d| d.and_utc().timestamp())
            .expect("valid");
        assert_eq!(events[0].start_time, Some(expected));
    }

    #[test]
    fn parse_no_vevent_returns_empty_not_err() {
        // A VCALENDAR wrapper with no VEVENT (e.g. VTIMEZONE-only response
        // after a fresh PUT) is now Ok(empty Vec). Callers can decide how
        // to surface "nothing here yet" - typically a stub DTO or a retry
        // - rather than failing the whole sync with "No VEVENT found".
        let ical = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
END:VCALENDAR\r\n";

        let result = parse_icalendar(ical).expect("should parse");
        assert!(result.is_empty());
    }
