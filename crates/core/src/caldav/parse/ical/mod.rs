use calcard::Parser;
use calcard::icalendar::{
    ICalendarComponentType, ICalendarEntry, ICalendarParameterName, ICalendarProperty,
    ICalendarValue, timezone::TzResolver,
};

/// Parsed event data extracted from an iCalendar VEVENT.
#[derive(Debug, Clone)]
pub struct ParsedVEvent {
    /// UID of the event (globally unique identifier).
    pub uid: Option<String>,
    /// SUMMARY - event title.
    pub summary: Option<String>,
    /// DESCRIPTION - event body.
    pub description: Option<String>,
    /// LOCATION - where the event takes place.
    pub location: Option<String>,
    /// DTSTART as a Unix timestamp.
    pub start_time: Option<i64>,
    /// DTEND as a Unix timestamp.
    pub end_time: Option<i64>,
    /// Whether this is an all-day event (DATE value type, no time component).
    pub is_all_day: bool,
    /// IANA-form name of the timezone DTSTART resolved through, when one
    /// was available. `None` for floating events (no TZID, no Z) and for
    /// UTC events (the wall-clock instant is already absolute, no zone
    /// rebase needed at expand time).
    ///
    /// Threaded into `UpsertCalendarEventParams::timezone` so the RRULE
    /// expander downstream (`db::queries_extra::calendars::RecurrenceTz`)
    /// walks the recurrence in the source zone instead of falling back
    /// to `chrono::Local`. Without this, the Round 2 RecurrenceTz fix
    /// was inert on the CalDAV path: every CalDAV row landed with
    /// `timezone = NULL` and the expander reanchored every instance in
    /// the user's local zone, silently shifting the displayed time by
    /// the offset between source and host.
    pub timezone: Option<String>,
    /// STATUS (CONFIRMED, TENTATIVE, CANCELLED).
    pub status: String,
    /// ORGANIZER email (stripped of mailto: prefix).
    pub organizer_email: Option<String>,
    /// ORGANIZER display name (CN parameter).
    pub organizer_name: Option<String>,
    /// Parsed attendees.
    pub attendees: Vec<ParsedAttendee>,
    /// RRULE as raw text (for display/storage - not expanded here).
    pub rrule: Option<String>,
    /// VALARM reminders extracted as minutes-before values.
    pub reminders: Vec<ParsedReminder>,
    /// RECURRENCE-ID in canonical wall-clock form (RFC 5545 § 3.8.4.4),
    /// when present. Identifies a single instance of a recurring event
    /// being overridden or cancelled; the same UID recurs in a calendar
    /// with distinct RECURRENCE-IDs (master + N overrides) and the
    /// storage key must include this discriminator or master and overrides
    /// collapse onto one row.
    ///
    /// Forms (deliberately string, not Unix timestamp):
    /// - `YYYYMMDD` for `VALUE=DATE` (all-day) overrides
    /// - `YYYYMMDDTHHMMSSZ` for `Z`-suffixed UTC datetimes (and any
    ///   numeric offset, normalized to UTC)
    /// - `YYYYMMDDTHHMMSS;TZID=<id>` for zoned datetimes
    /// - `YYYYMMDDTHHMMSS` for floating datetimes (no TZID, no offset)
    ///
    /// Resolving floating and all-day forms to a Unix timestamp at parse
    /// time silently re-anchored them in `chrono::Local`, so a sync run
    /// on a UTC host and another on a NY host produced two distinct
    /// storage keys for the same override - master + override collapsed
    /// to a single row on TZ change. Carrying the wall-clock string
    /// keeps the key host-independent.
    pub recurrence_id: Option<String>,
}

/// A single attendee parsed from a VEVENT.
#[derive(Debug, Clone)]
pub struct ParsedAttendee {
    /// Email address (stripped of mailto: prefix).
    pub email: String,
    /// CN parameter - display name.
    pub name: Option<String>,
    /// PARTSTAT parameter (ACCEPTED, DECLINED, TENTATIVE, NEEDS-ACTION).
    pub partstat: Option<String>,
    /// Whether this attendee is the organizer.
    pub is_organizer: bool,
}

/// A parsed reminder / alarm.
#[derive(Debug, Clone)]
pub struct ParsedReminder {
    /// Number of minutes before the event start.
    pub minutes_before: i64,
    /// Alarm method (DISPLAY, EMAIL, etc.).
    pub method: Option<String>,
}

/// Parse an iCalendar string and extract all VEVENTs.
///
/// Uses the `calcard` crate for parsing. Returns `Ok(empty Vec)` if the
/// payload parses but contains no VEVENT - some servers return a
/// VTIMEZONE-only VCALENDAR wrapper (e.g. when the timezone for a
/// freshly-created event hasn't propagated yet). Callers should treat an
/// empty result as "no event data available right now" rather than a hard
/// error.
pub fn parse_icalendar(ical_data: &str) -> Result<Vec<ParsedVEvent>, String> {
    let mut parser = Parser::new(ical_data);
    let mut events = Vec::new();

    loop {
        let entry = parser.entry();
        match entry {
            calcard::Entry::ICalendar(ical) => {
                let resolver = ical.build_tz_resolver();
                for component in &ical.components {
                    if component.component_type == ICalendarComponentType::VEvent {
                        events.push(extract_vevent(component, &ical, &resolver));
                    }
                }
            }
            calcard::Entry::Eof => break,
            calcard::Entry::InvalidLine(line) => {
                // Logged at debug rather than warn: real calendar feeds
                // (Outlook bridges in particular) emit harmless invalid
                // lines (e.g. malformed X-properties). At debug an
                // operator chasing "event missing after sync" can still
                // find the dropped lines, but the log doesn't fire on
                // every healthy sync.
                log::debug!("calcard parser dropped an invalid iCal line: {line}");
                continue;
            }
            _ => continue,
        }
    }

    Ok(events)
}

/// Extract event fields from an `ICalendarComponent` of type VEVENT.
fn extract_vevent(
    component: &calcard::icalendar::ICalendarComponent,
    ical: &calcard::icalendar::ICalendar,
    resolver: &TzResolver<&str>,
) -> ParsedVEvent {
    let uid = component.uid().map(String::from);

    // The previous `.filter(|s| !s.is_empty())` step was redundant because
    // calcard's parser already drops `SUMMARY:` (no value) from the
    // entries list - empty TEXT-typed properties don't survive parsing
    // far enough for our chain to see them. Keeping the chain unfiltered
    // here is forward-compatible: if calcard later starts surfacing
    // empty-but-present values, our merger will see the protocol
    // distinction (Some("") for echoed-empty vs None for absent) without
    // a code change. Today the practical effect is identical - calcard
    // collapses both into None - so user-cleared-title support requires
    // an upstream calcard change before it can land.
    let summary = component
        .property(&ICalendarProperty::Summary)
        .and_then(|e| e.values.first())
        .and_then(|v| v.as_text())
        .map(String::from);

    let description = component
        .property(&ICalendarProperty::Description)
        .and_then(|e| e.values.first())
        .and_then(|v| v.as_text())
        .map(String::from);

    let location = component
        .property(&ICalendarProperty::Location)
        .and_then(|e| e.values.first())
        .and_then(|v| v.as_text())
        .map(String::from);

    // Pick the source entries once per endpoint and reuse them below for
    // both the resolve and the all-day correction. The previous shape
    // re-walked `component.properties(prop)` inside each helper -- 4 walks
    // per all-day event, 2 per timed. (Round 3 #24.)
    let dtstart_pick = pick_datetime_entry(component, &ICalendarProperty::Dtstart);
    let dtend_pick = pick_datetime_entry(component, &ICalendarProperty::Dtend);

    let (start_time, is_all_day, timezone) = match dtstart_pick {
        Some((entry, is_date_only)) => extract_datetime(entry, is_date_only, resolver),
        None => (None, false, None),
    };
    // DTEND's resolved zone is unused: the master's TZID drives recurrence
    // expansion (RFC 5545 § 3.8.5.3 anchors recurrence on DTSTART), and
    // mixed start/end zones in a single VEVENT are degenerate enough
    // that we'd rather inherit DTSTART's zone for both endpoints. The
    // unused tuple element below drops it intentionally.
    let (end_time, _, _) = match dtend_pick {
        Some((entry, is_date_only)) => extract_datetime(entry, is_date_only, resolver),
        None => (None, false, None),
    };

    // If DTEND is missing but DURATION is present, compute end time
    let end_time = end_time.or_else(|| {
        let start = start_time?;
        let duration = component
            .property(&ICalendarProperty::Duration)
            .and_then(|e| e.values.first())
            .and_then(|v| match v {
                ICalendarValue::Duration(d) => Some(d.as_seconds()),
                _ => None,
            })?;
        Some(start + duration)
    });

    // All-day DST correction. DTSTART and DTEND for `VALUE=DATE` both pass
    // through `build_local_midnight`, so independently resolving DTEND in
    // chrono::Local on a DST-springing day shifts it by ±1 hour: a 2-day
    // all-day event spanning the spring-forward boundary stores end-start
    // as 47*3600s, and `(end - start) / 86400` rounds down to 1 day. JMAP
    // emit at jmap/calendar_sync.rs:810 then re-serializes the event as
    // `P1D`, propagating the truncation to every server-side consumer.
    //
    // Anchor end_time to start_time + Δdate*86400 so the duration is
    // exactly the number of calendar days the source iCal asked for,
    // independent of where DST sits relative to the span. start_time keeps
    // its display-correct local-midnight value; end_time may not land on
    // local midnight in a zone that springs forward inside the span (it
    // sits at 01:00 local instead) but downstream code already special-
    // cases is_all_day, so this is invisible to display.
    let end_time = if is_all_day && let (Some(start), Some(end)) = (start_time, end_time) {
        let start_date = dtstart_pick.and_then(|(e, d)| extract_all_day_date(e, d));
        let end_date = dtend_pick.and_then(|(e, d)| extract_all_day_date(e, d));
        match (start_date, end_date) {
            (Some(start_date), Some(end_date)) => {
                let days = end_date.signed_duration_since(start_date).num_days();
                Some(start + days * 86400)
            }
            _ => Some(end),
        }
    } else {
        end_time
    };

    let status = component
        .property(&ICalendarProperty::Status)
        .and_then(|e| e.values.first())
        .and_then(|v| v.as_text())
        .unwrap_or("CONFIRMED")
        .to_string();

    // Extract organizer
    let organizer_entry = component.property(&ICalendarProperty::Organizer);
    let organizer_email = organizer_entry
        .and_then(|e| e.calendar_address())
        .map(String::from);
    let organizer_name = organizer_entry
        .and_then(|e| e.parameter(&ICalendarParameterName::Cn))
        .and_then(|v| v.as_text())
        .map(String::from);

    // Extract attendees
    let mut attendees = Vec::new();
    for entry in component.properties(&ICalendarProperty::Attendee) {
        if let Some(email) = entry.calendar_address().map(String::from) {
            let name = entry
                .parameter(&ICalendarParameterName::Cn)
                .and_then(|v| v.as_text())
                .map(String::from);
            let partstat = entry
                .parameter(&ICalendarParameterName::Partstat)
                .and_then(|v| v.as_text())
                .map(String::from);
            attendees.push(ParsedAttendee {
                email,
                name,
                partstat,
                is_organizer: false,
            });
        }
    }

    // Mark the organizer in the attendee list
    if let Some(ref org_email) = organizer_email {
        let org_lower = org_email.to_lowercase();
        for att in &mut attendees {
            if att.email.to_lowercase() == org_lower {
                att.is_organizer = true;
            }
        }
    }

    // Extract RRULE
    let rrule = component
        .property(&ICalendarProperty::Rrule)
        .and_then(|e| e.values.first())
        .and_then(|v| match v {
            ICalendarValue::RecurrenceRule(r) => Some(r.to_string()),
            ICalendarValue::Text(t) => Some(t.clone()),
            _ => None,
        });

    // Extract VALARM reminders from sub-components
    let reminders = extract_reminders(component, ical);

    // Extract RECURRENCE-ID if this VEVENT is an override/cancellation of a
    // specific instance of the master series. We capture the wall-clock
    // form rather than resolving to a Unix timestamp here: floating and
    // all-day RECURRENCE-IDs would otherwise re-anchor in `chrono::Local`
    // at parse time (different host TZ -> different timestamp -> different
    // storage key), so the same iCal override would yield distinct rows on
    // UTC vs NY hosts. The string form is what the iCal source carried;
    // the storage key in `caldav/sync.rs` uses it directly.
    let recurrence_id = extract_recurrence_id_canonical(component);

    ParsedVEvent {
        uid,
        summary,
        description,
        location,
        start_time,
        end_time,
        is_all_day,
        timezone,
        status,
        organizer_email,
        organizer_name,
        attendees,
        rrule,
        reminders,
        recurrence_id,
    }
}

/// Extract a datetime from a DTSTART or DTEND property, returning
/// `(timestamp, is_all_day, resolved_tz_name)`.
///
/// `resolved_tz_name` is the IANA name of the zone the value was resolved
/// through, or `None` when the result is in UTC, floating, or all-day
/// (where re-resolution at recurrence-expand time would be a no-op or
/// equivalent to `chrono::Local`). Persisted into the row's `timezone`
/// column so the RRULE expander walks the recurrence in the source zone.
///
/// Honors the TZID parameter via the supplied resolver (which carries the
/// iCalendar's VTIMEZONE blocks and falls back to `Tz::from_str` for IANA or
/// Windows zone names). UTC ('Z') and floating times also work.
///
/// DST handling: ambiguous (fall-back) and non-existent (spring-forward) wall
/// clocks are resolved through `common::time::resolve_local_to_timestamp` -
/// fall-back picks the earlier instant, spring-forward shifts past the gap.
/// Both used to silently fall through to a naive-as-UTC interpretation, off
/// by the zone's full offset.
///
/// Floating times (TZID resolves to `Tz::Floating`, or no TZID at all) are
/// interpreted in `chrono::Local` per RFC 5545 § 3.3.5: "the time is to be
/// associated with the calendar in which the event is stored." For a single-
/// user desktop client that means the user's system zone.
///
/// All-day DATE values are stored as **local** midnight of the date in the
/// user's zone, not UTC midnight - the latter shifts the displayed calendar
/// date for any user not on UTC.
fn extract_datetime(
    entry: &ICalendarEntry,
    is_date_only: bool,
    resolver: &TzResolver<&str>,
) -> (Option<i64>, bool, Option<String>) {
    let Some(ICalendarValue::PartialDateTime(dt)) = entry.values.first() else {
        return (None, is_date_only, None);
    };

    if is_date_only {
        // All-day: build a NaiveDate at midnight LOCAL. Storing midnight UTC
        // displays the wrong calendar date for any user west of UTC (Jan 15
        // UTC midnight = Jan 14 16:00 PST). No tz to thread through; the
        // recurrence expander special-cases all-day to avoid DST drift.
        let timestamp = build_local_midnight(dt);
        return (timestamp, true, None);
    }

    let naive = match partial_to_naive(dt) {
        Some(n) => n,
        None => return (dt.to_timestamp(), false, None),
    };

    // 1. Explicit TZID, resolves to a real zone (IANA or Windows).
    if let Some(tz_id_raw) = entry.tz_id() {
        // RFC 5545 § 3.3.5 says a property value with a TZID parameter MUST
        // NOT also be UTC ("Z"-suffix). Some real-world emitters (older
        // Outlook, some WebDAV bridges) violate this. When both are present
        // we honor the embedded UTC offset (the path the wider calendar
        // ecosystem converges on) but log it so an operator can spot the
        // misbehaving server.
        if dt.tz_hour.is_some() {
            log::debug!(
                "CalDAV property has both TZID={tz_id_raw} and a UTC offset; honoring the offset per common practice"
            );
        } else {
            // Trim the TZID before lookup. calcard's resolver returns the
            // raw text from the iCal payload, but real servers occasionally
            // emit `TZID="America/New_York "` (trailing space) or similar.
            // The first `chrono_tz::Tz::from_str` attempt inside
            // `resolve_or_default` is byte-exact and won't match, and the
            // proprietary-alias fallback only trims for *its* lookup. The
            // net result without a trim here is a silent fall-through to
            // floating mode, which then re-anchors the wall-clock in
            // `chrono::Local` - shifting the event by hours for users
            // whose local zone differs from the (intended) TZID.
            let tz_id = tz_id_raw.trim();
            if !tz_id.is_empty() {
                let tz = resolver.resolve_or_default(Some(tz_id));
                if !tz.is_floating() {
                    // Persist the IANA name (resolver folded Windows
                    // aliases like "Pacific Standard Time" through to
                    // "America/Los_Angeles") so downstream
                    // `RecurrenceTz::from_event_timezone` can `parse()`
                    // it without bringing calcard's alias map into the
                    // db crate. `Tz::Fixed` resolves to an `Etc/GMT<n>`
                    // string which chrono_tz also accepts.
                    let resolved_name = tz.name().map(std::borrow::Cow::into_owned);
                    return (
                        common::time::resolve_local_to_timestamp(naive, &tz),
                        false,
                        resolved_name,
                    );
                }
                // The TZID was specified but did not resolve. Falling
                // through would silently re-anchor in `chrono::Local`,
                // making the event appear at the user's wall-clock time
                // instead of the (intended-but-unknown) source zone. UTC
                // is the safer default here: it matches the graph crate's
                // behavior at `parse_graph_datetime` and at least keeps
                // the displayed time consistent across machines, even if
                // it's offset from what the source meant.
                log::warn!(
                    "CalDAV TZID={tz_id_raw:?} did not resolve; falling back to UTC interpretation"
                );
                return (Some(naive.and_utc().timestamp()), false, None);
            }
        }
    }

    // 2. UTC offset embedded in the value (Z-suffix or +HH:MM). calcard's
    //    `to_timestamp()` already handles these and the result does not
    //    depend on a local zone, so it's safe to defer.
    if dt.tz_hour.is_some() {
        return (dt.to_timestamp(), false, None);
    }

    // 3. Floating time. RFC 5545 § 3.3.5 says interpret in the user's local
    //    zone. We previously fell through to `to_timestamp()` which silently
    //    treated the wall-clock as UTC. No persisted timezone here either:
    //    floating means "interpret in viewer's local zone," and persisting
    //    "Local" as a string would lock the event to the host that synced it.
    (
        common::time::resolve_local_to_timestamp(naive, &chrono::Local),
        false,
        None,
    )
}

/// Select the most-specific DTSTART/DTEND entry when a VEVENT carries more
/// than one. RFC 5545 § 3.6.1 makes DTSTART MUST occur exactly once per
/// VEVENT, but real emitters violate this: older Outlook bridges have been
/// seen pairing a `DTSTART;TZID=...:20240315T100000` (the source-of-truth
/// value) with a floating `DTSTART:20240315T100000` (a "compatibility"
/// fallback for legacy clients). The previous `component.property(prop)`
/// path returned whichever entry happened to come first - calcard's order
/// isn't stable across emitters - so the same event could end up at two
/// different displayed times depending on which server we'd last synced
/// from.
///
/// Layered preference:
///
/// 1. `VALUE=DATE` (all-day) - explicitly tagged semantics win.
/// 2. Explicit TZID parameter - the bridges' source-of-truth shape.
/// 3. UTC offset (Z-suffix or `+HH:MM` numeric) - anchored, so the result
///    is independent of the user's local zone.
/// 4. Floating - lowest, used only when nothing better is offered.
///
/// On tie, calcard's iteration order wins (which is the same as the old
/// "first wins" behavior, so the change is strictly more conservative for
/// well-formed inputs).
fn pick_datetime_entry<'c, 'p: 'c>(
    component: &'c calcard::icalendar::ICalendarComponent,
    prop: &'p ICalendarProperty,
) -> Option<(&'c ICalendarEntry, bool)> {
    let mut iter = component.properties(prop);
    let first = iter.next()?;
    let mut best = first;
    let mut best_score = score_datetime_candidate(best);
    let mut count = 1;
    for entry in iter {
        count += 1;
        let score = score_datetime_candidate(entry);
        if score > best_score {
            best = entry;
            best_score = score;
        }
    }
    if count > 1 {
        log::warn!(
            "VEVENT carries {count} entries for {prop:?} (RFC 5545 violation); selected by precedence: VALUE=DATE > TZID > UTC > floating"
        );
    }
    // VALUE=DATE is the score-4 bucket in `score_datetime_candidate`; surface
    // it to the caller so `extract_datetime` and `extract_all_day_date` can
    // share a single walk per endpoint instead of each running their own
    // `pick_datetime_entry` + `parameter(Value)` lookup. (Round 3 #24.)
    Some((best, best_score == 4))
}

fn score_datetime_candidate(entry: &ICalendarEntry) -> u8 {
    let is_date_only = entry
        .parameter(&ICalendarParameterName::Value)
        .and_then(|v| v.as_text())
        .is_some_and(|t| t.eq_ignore_ascii_case("DATE"));
    if is_date_only {
        return 4;
    }
    if entry.tz_id().is_some_and(|s| !s.trim().is_empty()) {
        return 3;
    }
    let has_offset = matches!(
        entry.values.first(),
        Some(ICalendarValue::PartialDateTime(dt)) if dt.tz_hour.is_some()
    );
    if has_offset {
        return 2;
    }
    1
}

fn partial_to_naive(dt: &calcard::common::PartialDateTime) -> Option<chrono::NaiveDateTime> {
    let year = dt.year? as i32;
    let month = dt.month? as u32;
    let day = dt.day? as u32;
    let hour = dt.hour.unwrap_or(0) as u32;
    let minute = dt.minute.unwrap_or(0) as u32;
    let second = dt.second.unwrap_or(0) as u32;
    chrono::NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(hour, minute, second)
}

fn build_local_midnight(dt: &calcard::common::PartialDateTime) -> Option<i64> {
    let year = dt.year? as i32;
    let month = dt.month? as u32;
    let day = dt.day? as u32;
    let naive = chrono::NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(0, 0, 0)?;
    common::time::resolve_local_to_timestamp(naive, &chrono::Local)
}

/// Build the canonical wall-clock storage form for a VEVENT's RECURRENCE-ID.
///
/// Returns `None` when the property is absent or carries no usable
/// `PartialDateTime` (an empty `RECURRENCE-ID:` value). The four forms
/// directly reflect the four iCal serialisations RFC 5545 § 3.8.4.4
/// permits, so two emitters that disagree on whether a TZID is present
/// produce distinct keys (the alternative would silently collide their
/// overrides). Numeric UTC offsets normalise to `Z` so an emitter that
/// sends `+0000` and one that sends `Z` agree.
///
/// We deliberately do NOT resolve to a Unix timestamp here - that path
/// makes the key host-TZ-dependent for floating and all-day RECURRENCE-IDs,
/// which is exactly the regression the wall-clock form fixes.
fn extract_recurrence_id_canonical(
    component: &calcard::icalendar::ICalendarComponent,
) -> Option<String> {
    let (entry, is_date_only) = pick_datetime_entry(component, &ICalendarProperty::RecurrenceId)?;
    let Some(ICalendarValue::PartialDateTime(dt)) = entry.values.first() else {
        return None;
    };
    // year is `Option<u16>` upstream so the value is always representable
    // as i32 - From is the right conversion. The `try_from` shape would
    // never fail for a parsed iCal but tripped a clippy::unnecessary_
    // fallible_conversions warning.
    let year = i32::from(dt.year?);
    let month = u32::from(dt.month?);
    let day = u32::from(dt.day?);

    if is_date_only {
        return Some(format!("{year:04}{month:02}{day:02}"));
    }

    let hour = u32::from(dt.hour.unwrap_or(0));
    let minute = u32::from(dt.minute.unwrap_or(0));
    let second = u32::from(dt.second.unwrap_or(0));
    let body = format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}");

    // RFC 5545 § 3.3.5 forbids combining TZID with a UTC offset, but real
    // emitters violate this. The DTSTART path (extract_datetime) already
    // chose to honor the offset over TZID; we mirror that here so the
    // master and override resolve consistently within a malformed feed.
    if dt.tz_hour.is_some() {
        // Z-suffix or numeric offset. Both are absolute; normalise the
        // numeric form to UTC so `+0000` and `Z` produce the same key.
        if let Some(naive) = partial_to_naive(dt)
            && (dt.tz_hour != Some(0) || dt.tz_minute.unwrap_or(0) != 0 || dt.tz_minus)
        {
            let secs = i32::from(dt.tz_hour.unwrap_or(0)) * 3600
                + i32::from(dt.tz_minute.unwrap_or(0)) * 60;
            let offset_secs = if dt.tz_minus { -secs } else { secs };
            // Subtract the offset to land on UTC wall-clock, then format.
            let utc_naive = naive
                .checked_sub_signed(chrono::Duration::seconds(i64::from(offset_secs)))
                .unwrap_or(naive);
            return Some(format!("{}Z", utc_naive.format("%Y%m%dT%H%M%S")));
        }
        return Some(format!("{body}Z"));
    }

    if let Some(tz_id_raw) = entry.tz_id() {
        let tz_id = tz_id_raw.trim();
        if !tz_id.is_empty() {
            return Some(format!("{body};TZID={tz_id}"));
        }
    }

    Some(body)
}

/// Extract the underlying calendar date for a `VALUE=DATE` property, before
/// it gets resolved to a midnight-anchored timestamp. Used by the all-day
/// DST correction in `extract_vevent` to compute the date delta directly
/// rather than subtracting two timestamps that may straddle a DST boundary.
///
/// Returns `None` when the picked entry is NOT `VALUE=DATE` (i.e. timed),
/// because `pick_datetime_entry` weighs `VALUE=DATE` (score 4) above
/// `TZID` (score 3) above bare `UTC` (score 2) - so a malformed VEVENT
/// pairing `DTSTART;VALUE=DATE:20240310` with `DTEND;TZID=America/New_York:
/// 20240312T000000` (a real Outlook bridge shape) would otherwise admit
/// the timed DTEND through this helper, return the wall-clock date in
/// the source TZ, and silently mix two date conventions in the all-day
/// duration math. Bailing here lets the caller fall through to its
/// `_ => Some(end)` arm and keep the original timed end_time, which is
/// the safer reading for a malformed feed.
fn extract_all_day_date(entry: &ICalendarEntry, is_date_only: bool) -> Option<chrono::NaiveDate> {
    if !is_date_only {
        return None;
    }
    let Some(ICalendarValue::PartialDateTime(dt)) = entry.values.first() else {
        return None;
    };
    let year = i32::from(dt.year?);
    let month = u32::from(dt.month?);
    let day = u32::from(dt.day?);
    chrono::NaiveDate::from_ymd_opt(year, month, day)
}

/// Extract VALARM reminders from the event's sub-components.
fn extract_reminders(
    component: &calcard::icalendar::ICalendarComponent,
    ical: &calcard::icalendar::ICalendar,
) -> Vec<ParsedReminder> {
    let mut reminders = Vec::new();

    for alarm_id in &component.component_ids {
        let Some(alarm) = ical.component_by_id(*alarm_id) else {
            continue;
        };
        if alarm.component_type != ICalendarComponentType::VAlarm {
            continue;
        }

        // Extract TRIGGER duration
        let trigger_minutes = alarm
            .property(&ICalendarProperty::Trigger)
            .and_then(|e| e.values.first())
            .and_then(|v| match v {
                ICalendarValue::Duration(d) => {
                    let secs = d.as_seconds();
                    // Negative duration means "before the event"
                    Some(if secs < 0 { -secs / 60 } else { secs / 60 })
                }
                _ => None,
            });

        let method = alarm
            .property(&ICalendarProperty::Action)
            .and_then(|e| e.values.first())
            .and_then(|v| v.as_text())
            .map(String::from);

        if let Some(minutes) = trigger_minutes {
            reminders.push(ParsedReminder {
                minutes_before: minutes,
                method,
            });
        }
    }

    reminders
}

#[cfg(test)]
mod tests;
