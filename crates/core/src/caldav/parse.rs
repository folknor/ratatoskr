use calcard::icalendar::{
    ICalendarComponentType, ICalendarParameterName, ICalendarProperty, ICalendarValue,
};
use calcard::Parser;

/// Parsed event data extracted from an iCalendar VEVENT.
#[derive(Debug, Clone)]
pub struct ParsedVEvent {
    /// UID of the event (globally unique identifier).
    pub uid: Option<String>,
    /// SUMMARY — event title.
    pub summary: Option<String>,
    /// DESCRIPTION — event body.
    pub description: Option<String>,
    /// LOCATION — where the event takes place.
    pub location: Option<String>,
    /// DTSTART as a Unix timestamp.
    pub start_time: Option<i64>,
    /// DTEND as a Unix timestamp.
    pub end_time: Option<i64>,
    /// Whether this is an all-day event (DATE value type, no time component).
    pub is_all_day: bool,
    /// STATUS (CONFIRMED, TENTATIVE, CANCELLED).
    pub status: String,
    /// ORGANIZER email (stripped of mailto: prefix).
    pub organizer_email: Option<String>,
    /// ORGANIZER display name (CN parameter).
    pub organizer_name: Option<String>,
    /// Parsed attendees.
    pub attendees: Vec<ParsedAttendee>,
    /// RRULE as raw text (for display/storage — not expanded here).
    pub rrule: Option<String>,
    /// VALARM reminders extracted as minutes-before values.
    pub reminders: Vec<ParsedReminder>,
}

/// A single attendee parsed from a VEVENT.
#[derive(Debug, Clone)]
pub struct ParsedAttendee {
    /// Email address (stripped of mailto: prefix).
    pub email: String,
    /// CN parameter — display name.
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
/// Uses the `calcard` crate for parsing.
/// Returns `Err` if the iCalendar data cannot be parsed at all.
pub fn parse_icalendar(ical_data: &str) -> Result<Vec<ParsedVEvent>, String> {
    let mut parser = Parser::new(ical_data);
    let mut events = Vec::new();

    loop {
        let entry = parser.entry();
        match entry {
            calcard::Entry::ICalendar(ical) => {
                for component in &ical.components {
                    if component.component_type == ICalendarComponentType::VEvent {
                        events.push(extract_vevent(component, &ical));
                    }
                }
            }
            calcard::Entry::Eof => break,
            calcard::Entry::InvalidLine(_) => continue,
            _ => continue,
        }
    }

    if events.is_empty() {
        return Err("No VEVENT found in iCalendar data".to_string());
    }

    Ok(events)
}

/// Extract event fields from an `ICalendarComponent` of type VEVENT.
fn extract_vevent(
    component: &calcard::icalendar::ICalendarComponent,
    ical: &calcard::icalendar::ICalendar,
) -> ParsedVEvent {
    let uid = component.uid().map(String::from);

    let summary = component
        .property(&ICalendarProperty::Summary)
        .and_then(|e| e.values.first())
        .and_then(|v| v.as_text())
        .filter(|s| !s.is_empty())
        .map(String::from);

    let description = component
        .property(&ICalendarProperty::Description)
        .and_then(|e| e.values.first())
        .and_then(|v| v.as_text())
        .filter(|s| !s.is_empty())
        .map(String::from);

    let location = component
        .property(&ICalendarProperty::Location)
        .and_then(|e| e.values.first())
        .and_then(|v| v.as_text())
        .filter(|s| !s.is_empty())
        .map(String::from);

    let (start_time, is_all_day) = extract_datetime(component, &ICalendarProperty::Dtstart);
    let (end_time, _) = extract_datetime(component, &ICalendarProperty::Dtend);

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

    ParsedVEvent {
        uid,
        summary,
        description,
        location,
        start_time,
        end_time,
        is_all_day,
        status,
        organizer_email,
        organizer_name,
        attendees,
        rrule,
        reminders,
    }
}

/// Extract a datetime from a DTSTART or DTEND property, returning
/// `(timestamp, is_all_day)`.
fn extract_datetime(
    component: &calcard::icalendar::ICalendarComponent,
    prop: &ICalendarProperty,
) -> (Option<i64>, bool) {
    let Some(entry) = component.property(prop) else {
        return (None, false);
    };

    // Check if it's a DATE-only value (all-day event)
    let is_date_only = entry
        .parameter(&ICalendarParameterName::Value)
        .and_then(|v| v.as_text())
        .is_some_and(|t| t.eq_ignore_ascii_case("DATE"));

    let timestamp = entry
        .values
        .first()
        .and_then(|v| match v {
            ICalendarValue::PartialDateTime(dt) => dt.to_timestamp(),
            _ => None,
        });

    // For DATE-only values without time, the PartialDateTime may not have
    // hour/minute/second set. `to_timestamp()` handles this via `to_datetime()`
    // which requires has_time(). For date-only values, we need to compute
    // the timestamp from the date parts directly.
    let timestamp = timestamp.or_else(|| {
        entry.values.first().and_then(|v| match v {
            ICalendarValue::PartialDateTime(dt) => {
                // Build a NaiveDate from year/month/day and convert to timestamp at midnight UTC
                let year = dt.year? as i32;
                let month = dt.month? as u32;
                let day = dt.day? as u32;
                chrono::NaiveDate::from_ymd_opt(year, month, day)
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
                    .map(|ndt| ndt.and_utc().timestamp())
            }
            _ => None,
        })
    });

    (timestamp, is_date_only)
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

// ---------------------------------------------------------------------------
// XML parsing helpers for CalDAV PROPFIND/REPORT responses
// ---------------------------------------------------------------------------

use quick_xml::Reader;
use quick_xml::events::Event;

use super::client::DiscoveredCalendar;

/// A single event entry from a PROPFIND listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalDavEventEntry {
    pub uri: String,
    pub etag: String,
}

/// Parse a PROPFIND Depth:1 response to extract calendar collections.
///
/// Looks for responses that have `<D:resourcetype><C:calendar/></D:resourcetype>`.
pub fn parse_propfind_calendars(xml: &str) -> Vec<DiscoveredCalendar> {
    let mut reader = Reader::from_str(xml);
    let mut calendars = Vec::new();

    let mut in_response = false;
    let mut in_resourcetype = false;
    let mut is_calendar = false;
    let mut current_href = String::new();
    let mut current_displayname = String::new();
    let mut current_ctag = String::new();
    let mut current_color = String::new();
    let mut current_tag = String::new();
    let mut buf = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = local_name(e.name().as_ref());
                if name == "response" {
                    in_response = true;
                    current_href.clear();
                    current_displayname.clear();
                    current_ctag.clear();
                    current_color.clear();
                    is_calendar = false;
                } else if name == "resourcetype" {
                    in_resourcetype = true;
                }
                current_tag = name;
                buf.clear();
            }
            Ok(Event::Empty(ref e)) => {
                let name = local_name(e.name().as_ref());
                // <C:calendar/> or <cal:calendar/> inside resourcetype
                if in_resourcetype && name == "calendar" {
                    is_calendar = true;
                }
            }
            Ok(Event::Text(ref e)) => {
                if let Ok(text) = e.unescape() {
                    buf.push_str(&text);
                }
            }
            Ok(Event::End(ref e)) => {
                let name = local_name(e.name().as_ref());
                if name == "resourcetype" {
                    in_resourcetype = false;
                }
                if in_response {
                    match current_tag.as_str() {
                        "href" => current_href = buf.trim().to_string(),
                        "displayname" => current_displayname = buf.trim().to_string(),
                        "getctag" => current_ctag = buf.trim().to_string(),
                        "calendar-color" => current_color = buf.trim().to_string(),
                        _ => {}
                    }
                }
                if name == "response" {
                    in_response = false;
                    if is_calendar && !current_href.is_empty() {
                        calendars.push(DiscoveredCalendar {
                            href: current_href.clone(),
                            display_name: if current_displayname.is_empty() {
                                None
                            } else {
                                Some(current_displayname.clone())
                            },
                            color: if current_color.is_empty() {
                                None
                            } else {
                                Some(current_color.clone())
                            },
                            ctag: if current_ctag.is_empty() {
                                None
                            } else {
                                Some(current_ctag.clone())
                            },
                        });
                    }
                }
                buf.clear();
                current_tag.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    calendars
}

/// Parse a PROPFIND Depth:1 response to extract event URIs and ETags.
pub fn parse_propfind_events(xml: &str) -> Vec<CalDavEventEntry> {
    let mut reader = Reader::from_str(xml);
    let mut entries = Vec::new();

    let mut in_response = false;
    let mut current_href = String::new();
    let mut current_etag = String::new();
    let mut current_content_type = String::new();
    let mut current_tag = String::new();
    let mut buf = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = local_name(e.name().as_ref());
                if name == "response" {
                    in_response = true;
                    current_href.clear();
                    current_etag.clear();
                    current_content_type.clear();
                }
                current_tag = name;
                buf.clear();
            }
            Ok(Event::Text(ref e)) => {
                if let Ok(text) = e.unescape() {
                    buf.push_str(&text);
                }
            }
            Ok(Event::End(ref e)) => {
                let name = local_name(e.name().as_ref());
                if in_response {
                    match current_tag.as_str() {
                        "href" => current_href = buf.trim().to_string(),
                        "getetag" => {
                            current_etag = buf.trim().trim_matches('"').to_string();
                        }
                        "getcontenttype" => {
                            current_content_type = buf.trim().to_string();
                        }
                        _ => {}
                    }
                }
                if name == "response" {
                    in_response = false;
                    if !current_href.is_empty()
                        && !current_etag.is_empty()
                        && is_icalendar_resource(&current_href, &current_content_type)
                    {
                        entries.push(CalDavEventEntry {
                            uri: current_href.clone(),
                            etag: current_etag.clone(),
                        });
                    }
                }
                buf.clear();
                current_tag.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    entries
}

/// Parse a CTag from a PROPFIND response.
pub fn parse_ctag(xml: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    let mut current_tag = String::new();
    let mut buf = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                current_tag = local_name(e.name().as_ref());
                buf.clear();
            }
            Ok(Event::Text(ref e)) => {
                if let Ok(text) = e.unescape() {
                    buf.push_str(&text);
                }
            }
            Ok(Event::End(_)) => {
                if current_tag == "getctag" {
                    let val = buf.trim().to_string();
                    if !val.is_empty() {
                        return Some(val);
                    }
                }
                buf.clear();
                current_tag.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    None
}

/// Parse a calendar-multiget or calendar-query REPORT response.
///
/// Returns `Vec<(uri, ical_data)>`.
pub fn parse_multiget_report(xml: &str) -> Vec<(String, String)> {
    let mut reader = Reader::from_str(xml);
    let mut results = Vec::new();

    let mut in_response = false;
    let mut current_href = String::new();
    let mut current_ical = String::new();
    let mut current_tag = String::new();
    let mut buf = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = local_name(e.name().as_ref());
                if name == "response" {
                    in_response = true;
                    current_href.clear();
                    current_ical.clear();
                }
                current_tag = name;
                buf.clear();
            }
            Ok(Event::Text(ref e)) => {
                if let Ok(text) = e.unescape() {
                    buf.push_str(&text);
                }
            }
            Ok(Event::End(ref e)) => {
                let name = local_name(e.name().as_ref());
                if in_response {
                    match current_tag.as_str() {
                        "href" => current_href = buf.trim().to_string(),
                        "calendar-data" => current_ical = buf.trim().to_string(),
                        _ => {}
                    }
                }
                if name == "response" {
                    in_response = false;
                    if !current_href.is_empty() && !current_ical.is_empty() {
                        results.push((current_href.clone(), current_ical.clone()));
                    }
                }
                buf.clear();
                current_tag.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    results
}

/// Extract the local name from a possibly-namespaced XML tag.
fn local_name(raw: &[u8]) -> String {
    let full = String::from_utf8_lossy(raw);
    match full.rfind(':') {
        Some(idx) => full[idx + 1..].to_string(),
        None => full.to_string(),
    }
}

/// Check if a resource looks like an iCalendar resource.
fn is_icalendar_resource(href: &str, content_type: &str) -> bool {
    if content_type.contains("text/calendar") {
        return true;
    }
    if href.ends_with(".ics") {
        return true;
    }
    // Accept entries with an etag but no content type info
    content_type.is_empty()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
    fn parse_no_vevent() {
        let ical = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
END:VCALENDAR\r\n";

        let result = parse_icalendar(ical);
        assert!(result.is_err());
    }

    #[test]
    fn parse_propfind_calendars_xml() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"
               xmlns:CS="http://calendarserver.org/ns/"
               xmlns:IC="http://apple.com/ns/ical/">
  <D:response>
    <D:href>/calendars/user/</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection/></D:resourcetype>
        <D:displayname>User Calendars</D:displayname>
      </D:prop>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/calendars/user/personal/</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection/><C:calendar/></D:resourcetype>
        <D:displayname>Personal</D:displayname>
        <CS:getctag>ctag-abc-123</CS:getctag>
        <IC:calendar-color>#0000FFFF</IC:calendar-color>
      </D:prop>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/calendars/user/work/</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection/><C:calendar/></D:resourcetype>
        <D:displayname>Work</D:displayname>
        <CS:getctag>ctag-def-456</CS:getctag>
      </D:prop>
    </D:propstat>
  </D:response>
</D:multistatus>"#;

        let calendars = parse_propfind_calendars(xml);
        assert_eq!(calendars.len(), 2);
        assert_eq!(calendars[0].href, "/calendars/user/personal/");
        assert_eq!(calendars[0].display_name.as_deref(), Some("Personal"));
        assert_eq!(calendars[0].ctag.as_deref(), Some("ctag-abc-123"));
        assert_eq!(calendars[0].color.as_deref(), Some("#0000FFFF"));
        assert_eq!(calendars[1].href, "/calendars/user/work/");
        assert_eq!(calendars[1].display_name.as_deref(), Some("Work"));
        assert!(calendars[1].color.is_none());
    }

    #[test]
    fn parse_propfind_events_xml() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/calendars/user/personal/</D:href>
    <D:propstat>
      <D:prop>
        <D:getetag>"collection-etag"</D:getetag>
      </D:prop>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/calendars/user/personal/event1.ics</D:href>
    <D:propstat>
      <D:prop>
        <D:getetag>"etag-111"</D:getetag>
        <D:getcontenttype>text/calendar; charset=utf-8</D:getcontenttype>
      </D:prop>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/calendars/user/personal/event2.ics</D:href>
    <D:propstat>
      <D:prop>
        <D:getetag>"etag-222"</D:getetag>
        <D:getcontenttype>text/calendar</D:getcontenttype>
      </D:prop>
    </D:propstat>
  </D:response>
</D:multistatus>"#;

        let entries = parse_propfind_events(xml);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].uri, "/calendars/user/personal/event1.ics");
        assert_eq!(entries[0].etag, "etag-111");
        assert_eq!(entries[1].uri, "/calendars/user/personal/event2.ics");
        assert_eq!(entries[1].etag, "etag-222");
    }

    #[test]
    fn parse_ctag_xml() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:CS="http://calendarserver.org/ns/">
  <D:response>
    <D:href>/calendars/user/personal/</D:href>
    <D:propstat>
      <D:prop>
        <CS:getctag>ctag-value-12345</CS:getctag>
      </D:prop>
    </D:propstat>
  </D:response>
</D:multistatus>"#;

        let ctag = parse_ctag(xml);
        assert_eq!(ctag.as_deref(), Some("ctag-value-12345"));
    }

    #[test]
    fn parse_multiget_report_xml() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:response>
    <D:href>/calendars/user/personal/event1.ics</D:href>
    <D:propstat>
      <D:prop>
        <D:getetag>"etag-111"</D:getetag>
        <C:calendar-data>BEGIN:VCALENDAR
VERSION:2.0
BEGIN:VEVENT
UID:test-1@example.com
SUMMARY:Test Event
DTSTART:20240315T100000Z
DTEND:20240315T110000Z
END:VEVENT
END:VCALENDAR</C:calendar-data>
      </D:prop>
    </D:propstat>
  </D:response>
</D:multistatus>"#;

        let results = parse_multiget_report(xml);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "/calendars/user/personal/event1.ics");
        assert!(results[0].1.contains("Test Event"));
    }
}
