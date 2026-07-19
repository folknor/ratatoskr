//! Translation between bifrost calendar values and Ratatoskr's calendar cache.
//!
//! This is deliberately the single consumer-side place that understands
//! protocol identity.  In particular Google and Graph expose composite
//! `calendar_id::event_id` ids while the existing cache and write path use the
//! bare event id.

use super::actions::CalendarEventInput;
use bifrost_types::{
    Calendar, CalendarEvent, CalendarId, EventAvailability, EventCreate, EventPatch,
    EventRecurrence, EventReminder, EventStatus, EventTime, EventVisibility, ProtocolKind,
    ReminderTrigger, RsvpStatus,
};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Offset, SecondsFormat, TimeZone, Utc};
use chrono_tz::Tz;
use db::db::queries_extra::calendar_contacts_writes::{
    CalDavAttendee, CalDavReminder, CalendarEventRow, DiscoveredCalendar,
};

pub fn calendar_remote_id(calendar: &Calendar) -> &str {
    &calendar.native_id
}

pub fn provider_name(provider: ProtocolKind) -> &'static str {
    match provider {
        ProtocolKind::Gmail => "google",
        ProtocolKind::Graph => "graph",
        ProtocolKind::Jmap => "jmap",
        ProtocolKind::CalDav => "caldav",
        ProtocolKind::Imap | ProtocolKind::CardDav | _ => "unknown",
    }
}

pub fn to_discovered_calendar<'a>(
    account_id: &'a str,
    calendar: &'a Calendar,
) -> DiscoveredCalendar<'a> {
    DiscoveredCalendar {
        account_id,
        provider: provider_name(calendar.provenance.provider),
        remote_id: calendar_remote_id(calendar),
        display_name: Some(&calendar.name),
        color: calendar.color.as_deref(),
        is_primary: calendar.is_default,
        can_edit: calendar.can_update_events,
    }
}

pub fn href_synthetic_uid(uri: &str) -> String {
    format!("href={uri}")
}

pub fn make_google_event_id(uid: &str, recurrence_id: Option<&str>) -> String {
    match recurrence_id {
        Some(recurrence_id) => format!("caldav:{uid}::recurrence-id={recurrence_id}"),
        None => format!("caldav:{uid}"),
    }
}

fn native_event_id(event: &CalendarEvent) -> &str {
    match event.provenance.provider {
        ProtocolKind::Gmail | ProtocolKind::Graph => event
            .native_id
            .rsplit_once("::")
            .map_or(&event.native_id, |(_, event_id)| event_id),
        _ => &event.native_id,
    }
}

/// Remove the composite calendar prefix used by Gmail and Graph event ids.
pub fn strip_event_id(provider: ProtocolKind, event_id: &bifrost_types::EventId) -> String {
    match provider {
        ProtocolKind::Gmail | ProtocolKind::Graph => event_id
            .0
            .rsplit_once("::")
            .map_or_else(|| event_id.0.clone(), |(_, id)| id.to_string()),
        _ => event_id.0.clone(),
    }
}

pub fn event_dedup_key(event: &CalendarEvent) -> String {
    if event.provenance.provider == ProtocolKind::CalDav {
        let uid = event
            .uid
            .clone()
            .unwrap_or_else(|| href_synthetic_uid(&event.native_id));
        make_google_event_id(&uid, event.recurrence.recurrence_id.as_deref())
    } else {
        native_event_id(event).to_string()
    }
}

pub fn event_remote_id(event: &CalendarEvent) -> String {
    native_event_id(event).to_string()
}

pub fn event_id_for_writeback(
    provider: ProtocolKind,
    remote_event_id: &str,
    _etag: Option<&str>,
    calendar_remote_id: Option<&str>,
) -> (bifrost_types::EventId, Option<CalendarId>) {
    let calendar = calendar_remote_id.map(|id| CalendarId(id.to_string()));
    let id = match provider {
        ProtocolKind::Gmail | ProtocolKind::Graph => calendar_remote_id
            .map(|calendar_id| format!("{calendar_id}::{remote_event_id}"))
            .unwrap_or_else(|| remote_event_id.to_string()),
        _ => remote_event_id.to_string(),
    };
    (bifrost_types::EventId(id), calendar)
}

pub fn event_time_to_epoch(time: &EventTime, is_all_day: bool) -> Option<i64> {
    if is_all_day {
        let date = NaiveDate::parse_from_str(&time.value, "%Y-%m-%d").ok()?;
        return Some(
            Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0)?)
                .timestamp(),
        );
    }
    if let Ok(value) = DateTime::parse_from_rfc3339(&time.value) {
        return Some(value.timestamp());
    }
    let local = NaiveDateTime::parse_from_str(&time.value, "%Y-%m-%dT%H:%M:%S").ok()?;
    if let Some(zone) = time
        .timezone
        .as_deref()
        .and_then(|zone| zone.parse::<Tz>().ok())
    {
        return match zone.from_local_datetime(&local) {
            chrono::LocalResult::Single(value) => Some(value.timestamp()),
            chrono::LocalResult::Ambiguous(early, _) => Some(early.timestamp()),
            chrono::LocalResult::None => {
                for minute in 1..=(48 * 60) {
                    let candidate = local.checked_add_signed(chrono::Duration::minutes(minute))?;
                    if let chrono::LocalResult::Single(value)
                    | chrono::LocalResult::Ambiguous(value, _) =
                        zone.from_local_datetime(&candidate)
                    {
                        return Some(value.timestamp());
                    }
                }
                None
            }
        };
    }
    Some(Utc.from_utc_datetime(&local).timestamp())
}

/// Inverse of `event_time_to_epoch`. All-day values use UTC dates and retain
/// bifrost's exclusive-end convention; callers must pass an exclusive end.
pub fn epoch_to_event_time(epoch_secs: i64, is_all_day: bool, timezone: Option<&str>) -> EventTime {
    if is_all_day {
        return EventTime {
            value: Utc
                .timestamp_opt(epoch_secs, 0)
                .single()
                .unwrap_or_else(|| Utc.timestamp_nanos(0))
                .format("%Y-%m-%d")
                .to_string(),
            timezone: None,
        };
    }
    if let Some(zone) = timezone.and_then(|zone| zone.parse::<Tz>().ok()) {
        let zoned = zone
            .timestamp_opt(epoch_secs, 0)
            .single()
            .unwrap_or_else(|| zone.timestamp_nanos(0));
        // A zero-offset zone (e.g. "UTC") emits the canonical `Z` form rather
        // than `+00:00`, matching the legacy wire format providers received.
        let value = if zoned.offset().fix().local_minus_utc() == 0 {
            zoned
                .with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Secs, true)
        } else {
            zoned.to_rfc3339()
        };
        return EventTime {
            value,
            timezone: timezone.map(str::to_string),
        };
    }
    EventTime {
        value: Utc
            .timestamp_opt(epoch_secs, 0)
            .single()
            .unwrap_or_else(|| Utc.timestamp_nanos(0))
            .to_rfc3339_opts(SecondsFormat::Secs, true),
        timezone: None,
    }
}

/// Parse form values for write payloads. Missing and unrecognised values share
/// the neutral defaults because bifrost write enums are non-optional.
pub fn parse_availability(value: &str) -> EventAvailability {
    match value.to_ascii_lowercase().as_str() {
        "free" => EventAvailability::Free,
        "tentative" => EventAvailability::Tentative,
        "out_of_office" | "outofoffice" => EventAvailability::OutOfOffice,
        _ => EventAvailability::Busy,
    }
}
pub fn parse_visibility(value: &str) -> EventVisibility {
    match value.to_ascii_lowercase().as_str() {
        "public" => EventVisibility::Public,
        "private" => EventVisibility::Private,
        "confidential" => EventVisibility::Confidential,
        _ => EventVisibility::Default,
    }
}
pub fn parse_rsvp(value: &str) -> RsvpStatus {
    match value.to_ascii_lowercase().as_str() {
        "accepted" => RsvpStatus::Accepted,
        "declined" => RsvpStatus::Declined,
        "tentative" => RsvpStatus::Tentative,
        "delegated" => RsvpStatus::Delegated,
        _ => RsvpStatus::NeedsAction,
    }
}

pub fn input_to_event_create(calendar_remote_id: &str, input: &CalendarEventInput) -> EventCreate {
    EventCreate {
        calendar_id: CalendarId(calendar_remote_id.to_string()),
        title: Some(input.title.clone()),
        description: Some(input.description.clone()),
        location: Some(input.location.clone()),
        start: epoch_to_event_time(
            input.start_time,
            input.is_all_day,
            input.timezone.as_deref(),
        ),
        end: epoch_to_event_time(input.end_time, input.is_all_day, input.timezone.as_deref()),
        is_all_day: input.is_all_day,
        status: EventStatus::Confirmed,
        availability: input
            .availability
            .as_deref()
            .map(parse_availability)
            .unwrap_or(EventAvailability::Busy),
        visibility: input
            .visibility
            .as_deref()
            .map(parse_visibility)
            .unwrap_or(EventVisibility::Default),
        organizer: None,
        attendees: Vec::new(),
        recurrence: EventRecurrence {
            rrule: input.recurrence_rule.clone(),
            ..EventRecurrence::default()
        },
    }
}

pub fn input_to_event_patch(input: &CalendarEventInput) -> EventPatch {
    EventPatch {
        calendar_id: None,
        title: Some(Some(input.title.clone())),
        description: Some(Some(input.description.clone())),
        location: Some(Some(input.location.clone())),
        start: Some(epoch_to_event_time(
            input.start_time,
            input.is_all_day,
            input.timezone.as_deref(),
        )),
        end: Some(epoch_to_event_time(
            input.end_time,
            input.is_all_day,
            input.timezone.as_deref(),
        )),
        is_all_day: Some(input.is_all_day),
        status: None,
        availability: Some(
            input
                .availability
                .as_deref()
                .map(parse_availability)
                .unwrap_or(EventAvailability::Busy),
        ),
        visibility: Some(
            input
                .visibility
                .as_deref()
                .map(parse_visibility)
                .unwrap_or(EventVisibility::Default),
        ),
        attendees: None,
        recurrence: Some(EventRecurrence {
            rrule: input.recurrence_rule.clone(),
            ..EventRecurrence::default()
        }),
    }
}

fn recurrence_id(event: &CalendarEvent) -> Option<String> {
    // Graph's seriesMasterId is not an RFC5545 RECURRENCE-ID.
    (event.provenance.provider != ProtocolKind::Graph)
        .then(|| {
            event
                .recurrence
                .recurrence_id
                .as_deref()
                .map(canonical_recurrence_id)
        })
        .flatten()
}

/// Keep recurrence identity in the schema's host-independent RFC 5545 form.
/// Bifrost providers normally supply this already, but normalizing the common
/// RFC 3339 spellings here keeps the cache key stable at the boundary.
fn canonical_recurrence_id(value: &str) -> String {
    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return date.format("%Y%m%d").to_string();
    }
    if let Ok(date_time) = DateTime::parse_from_rfc3339(value) {
        return date_time.format("%Y%m%dT%H%M%SZ").to_string();
    }
    if let Ok(date_time) = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S") {
        return date_time.format("%Y%m%dT%H%M%S").to_string();
    }
    value.to_string()
}

fn status(status: EventStatus) -> String {
    match status {
        EventStatus::Confirmed => "confirmed",
        EventStatus::Tentative => "tentative",
        EventStatus::Cancelled => "cancelled",
        EventStatus::Unknown | _ => "unknown",
    }
    .to_string()
}

pub fn availability(value: EventAvailability) -> String {
    match value {
        EventAvailability::Busy => "busy",
        EventAvailability::Free => "free",
        EventAvailability::Tentative => "tentative",
        EventAvailability::OutOfOffice => "out_of_office",
        EventAvailability::Unknown | _ => "unknown",
    }
    .to_string()
}

pub fn visibility(value: EventVisibility) -> String {
    match value {
        EventVisibility::Default => "default",
        EventVisibility::Public => "public",
        EventVisibility::Private => "private",
        EventVisibility::Confidential => "confidential",
        _ => "unknown",
    }
    .to_string()
}

pub fn rsvp(value: RsvpStatus) -> String {
    match value {
        RsvpStatus::NeedsAction => "needsaction",
        RsvpStatus::Accepted => "accepted",
        RsvpStatus::Declined => "declined",
        RsvpStatus::Tentative => "tentative",
        RsvpStatus::Delegated => "delegated",
        RsvpStatus::Unknown | _ => "unknown",
    }
    .to_string()
}

pub fn to_event_row(
    account_id: &str,
    calendar_id: &str,
    event: &CalendarEvent,
) -> Option<CalendarEventRow> {
    let start_time = event_time_to_epoch(&event.start, event.is_all_day)?;
    let end_time = event_time_to_epoch(&event.end, event.is_all_day)?;
    let attendees_json = (!event.attendees.is_empty())
        .then(|| {
            serde_json::to_string(
                &event
                    .attendees
                    .iter()
                    .map(|attendee| {
                        serde_json::json!({
                            "email": attendee.email,
                            "displayName": attendee.name,
                            "responseStatus": rsvp(attendee.status),
                        })
                    })
                    .collect::<Vec<_>>(),
            )
            .ok()
        })
        .flatten();
    Some(CalendarEventRow {
        account_id: account_id.to_string(),
        google_event_id: event_dedup_key(event),
        remote_event_id: event_remote_id(event),
        calendar_id: calendar_id.to_string(),
        summary: event.title.clone(),
        description: event.description.clone(),
        location: event.location.clone(),
        start_time,
        end_time,
        is_all_day: event.is_all_day,
        status: status(event.status),
        organizer_email: event
            .organizer
            .as_ref()
            .map(|organizer| organizer.email.clone()),
        attendees_json,
        html_link: event.html_link.clone(),
        etag: event.etag.clone(),
        ical_data: event.raw_ical.clone(),
        uid: event.uid.clone(),
        title: event.title.clone(),
        timezone: event.start.timezone.clone(),
        recurrence_rule: event.recurrence.rrule.clone(),
        organizer_name: event
            .organizer
            .as_ref()
            .and_then(|organizer| organizer.name.clone()),
        rsvp_status: Some(rsvp(event.self_response)),
        availability: Some(availability(event.availability)),
        visibility: Some(visibility(event.visibility)),
        recurrence_id: recurrence_id(event),
    })
}

pub fn to_attendees(event: &CalendarEvent) -> Vec<CalDavAttendee> {
    event
        .attendees
        .iter()
        .map(|attendee| CalDavAttendee {
            email: attendee.email.clone(),
            name: attendee.name.clone(),
            partstat: Some(rsvp(attendee.status)),
            is_organizer: false,
        })
        .collect()
}

pub fn to_reminders(event: &CalendarEvent) -> Vec<CalDavReminder> {
    event.reminders.iter().filter_map(reminder_to_row).collect()
}

fn reminder_to_row(reminder: &EventReminder) -> Option<CalDavReminder> {
    // Only relative triggers map to `minutes_before`; absolute triggers have no
    // minutes-before-start form and are dropped (legacy parity - the legacy
    // VALARM parser ignored non-DURATION triggers too).
    let ReminderTrigger::Relative { offset, .. } = &reminder.trigger else {
        return None;
    };
    Some(CalDavReminder {
        minutes_before: iso8601_duration_minutes(offset)?,
        method: reminder.action.clone(),
    })
}

/// Convert an RFC 5545 / ISO 8601 duration (`-PT15M`, `-PT1H`, `-P1DT2H`,
/// `-P1W`, ...) into whole minutes. Matches the legacy VALARM convention: the
/// magnitude in minutes regardless of sign (a leading `-` denotes "before the
/// event start", the common case). Sub-minute precision truncates. Returns
/// `None` for a malformed duration. Bifrost preserves the raw TRIGGER text in
/// `ReminderTrigger::Relative.offset`, so an hour/day/week/compound offset must
/// parse here rather than being silently dropped.
fn iso8601_duration_minutes(value: &str) -> Option<i64> {
    let body = value.strip_prefix(['-', '+']).unwrap_or(value);
    let mut total_seconds: i64 = 0;
    let mut in_time = false;
    let mut saw_field = false;
    let mut number = String::new();
    for ch in body.strip_prefix('P')?.chars() {
        match ch {
            'T' => in_time = true,
            '0'..='9' => number.push(ch),
            unit => {
                let count: i64 = number.parse().ok()?;
                number.clear();
                saw_field = true;
                let seconds = match (in_time, unit) {
                    (false, 'W') => count.checked_mul(7 * 86_400)?,
                    (false, 'D') => count.checked_mul(86_400)?,
                    (true, 'H') => count.checked_mul(3_600)?,
                    (true, 'M') => count.checked_mul(60)?,
                    (true, 'S') => count,
                    _ => return None,
                };
                total_seconds = total_seconds.checked_add(seconds)?;
            }
        }
    }
    // A trailing number with no unit (`-PT15`) or a duration with no fields
    // (`-P`, `-PT`) is malformed.
    if !saw_field || !number.is_empty() {
        return None;
    }
    Some(total_seconds.abs() / 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_types::{
        AttendeeRole, CalendarId, CalendarProvenance, EventAttendee, EventId, EventRecurrence,
        EventStatus, ProtocolKind, ReminderRelativeTo,
    };

    fn event(provider: ProtocolKind, native_id: &str) -> CalendarEvent {
        CalendarEvent {
            id: EventId(native_id.to_string()),
            calendar_id: CalendarId("calendar".to_string()),
            native_id: native_id.to_string(),
            uid: Some("uid@example.test".to_string()),
            etag: None,
            provenance: CalendarProvenance {
                provider,
                native: native_id.to_string(),
                calendar_native: Some("calendar".to_string()),
            },
            title: None,
            description: None,
            location: None,
            start: EventTime {
                value: "2026-01-01T00:00:00Z".to_string(),
                timezone: Some("UTC".to_string()),
            },
            end: EventTime {
                value: "2026-01-01T01:00:00Z".to_string(),
                timezone: Some("UTC".to_string()),
            },
            is_all_day: false,
            status: EventStatus::Confirmed,
            availability: EventAvailability::Busy,
            visibility: EventVisibility::Default,
            self_response: RsvpStatus::NeedsAction,
            organizer: None,
            attendees: Vec::new(),
            reminders: Vec::new(),
            recurrence: EventRecurrence::default(),
            html_link: None,
            raw_ical: None,
        }
    }

    #[test]
    fn idmap_event_dedup_key_caldav_preserves_caldav_prefix() {
        let mut event = event(ProtocolKind::CalDav, "/calendar/event.ics");
        event.recurrence.recurrence_id = Some("20260101T000000Z".to_string());
        assert_eq!(
            event_dedup_key(&event),
            "caldav:uid@example.test::recurrence-id=20260101T000000Z"
        );
    }

    #[test]
    fn idmap_event_dedup_key_native_uses_native_id() {
        assert_eq!(
            event_dedup_key(&event(ProtocolKind::Gmail, "calendar::event")),
            "event"
        );
        assert_eq!(
            event_dedup_key(&event(ProtocolKind::Jmap, "event")),
            "event"
        );
    }

    #[test]
    fn idmap_event_time_all_day_parses_to_midnight() {
        assert_eq!(
            event_time_to_epoch(
                &EventTime {
                    value: "2026-01-02".to_string(),
                    timezone: Some("Europe/Oslo".to_string()),
                },
                true,
            ),
            Some(1_767_312_000)
        );
    }

    #[test]
    fn idmap_event_time_rfc3339_parses_to_epoch() {
        assert_eq!(
            event_time_to_epoch(
                &EventTime {
                    value: "2026-01-02T03:04:05+01:00".to_string(),
                    timezone: None,
                },
                false,
            ),
            Some(1_767_319_445)
        );
    }

    #[test]
    fn idmap_event_time_all_day_one_day_has_86400_duration() {
        let start = EventTime {
            value: "2026-01-02".to_string(),
            timezone: None,
        };
        let end = EventTime {
            value: "2026-01-03".to_string(),
            timezone: None,
        };
        let start = event_time_to_epoch(&start, true).expect("all-day start parses");
        let end = event_time_to_epoch(&end, true).expect("all-day end parses");
        assert_eq!(end - start, 86_400);
    }

    #[test]
    fn idmap_recurrence_id_canonical_form_is_host_tz_independent() {
        assert_eq!(canonical_recurrence_id("2026-01-02"), "20260102");
        assert_eq!(
            canonical_recurrence_id("2026-01-02T03:04:05"),
            "20260102T030405"
        );
        assert_eq!(
            canonical_recurrence_id("2026-01-02T03:04:05Z"),
            "20260102T030405Z"
        );
        assert_eq!(
            canonical_recurrence_id("20260102T030405;TZID=Europe/Oslo"),
            "20260102T030405;TZID=Europe/Oslo"
        );
    }

    #[test]
    fn idmap_event_time_bare_wall_clock_with_tzid_resolves_zone() {
        let time = EventTime {
            value: "2026-01-02T03:04:05".to_string(),
            timezone: Some("Europe/Oslo".to_string()),
        };
        assert_eq!(event_time_to_epoch(&time, false), Some(1_767_319_445));
    }

    #[test]
    fn idmap_event_time_dst_gap_shifts_forward_and_ambiguity_picks_earlier() {
        let gap = EventTime {
            value: "2026-03-29T02:30:00".to_string(),
            timezone: Some("Europe/Oslo".to_string()),
        };
        let ambiguous = EventTime {
            value: "2026-10-25T02:30:00".to_string(),
            timezone: Some("Europe/Oslo".to_string()),
        };
        assert_eq!(event_time_to_epoch(&gap, false), Some(1_774_746_000));
        assert_eq!(event_time_to_epoch(&ambiguous, false), Some(1_792_888_200));
    }

    #[test]
    fn idmap_event_time_floating_parses_as_utc() {
        let time = EventTime {
            value: "2026-01-02T03:04:05".to_string(),
            timezone: None,
        };
        assert_eq!(event_time_to_epoch(&time, false), Some(1_767_323_045));
    }

    #[test]
    fn idmap_attendees_json_matches_legacy_shape() {
        let mut value = event(ProtocolKind::Gmail, "calendar::event");
        value.attendees.push(EventAttendee {
            email: "attendee@example.test".to_string(),
            name: Some("Attendee".to_string()),
            role: AttendeeRole::Required,
            status: RsvpStatus::Accepted,
        });
        let row = to_event_row("account", "calendar", &value).expect("event row");
        let attendees: serde_json::Value =
            serde_json::from_str(row.attendees_json.as_deref().expect("attendees JSON"))
                .expect("valid attendees JSON");
        assert_eq!(
            attendees,
            serde_json::json!([{
                "email": "attendee@example.test",
                "displayName": "Attendee",
                "responseStatus": "accepted",
            }])
        );
    }

    #[test]
    fn idmap_reminder_offset_parses_minutes_hours_days_and_compound() {
        let cases = [
            ("-PT15M", Some(15)),
            ("-PT1H", Some(60)),
            ("-PT1H30M", Some(90)),
            ("-P1D", Some(1_440)),
            ("-P1W", Some(10_080)),
            ("-P1DT2H", Some(1_560)),
            ("-PT30S", Some(0)),
            ("PT15M", Some(15)), // after-start magnitude, legacy parity
            ("-PT15", None),     // trailing number, no unit
            ("-P", None),        // no fields
            ("15M", None),       // missing leading P
            ("garbage", None),
        ];
        for (offset, expected) in cases {
            let reminder = EventReminder {
                trigger: ReminderTrigger::Relative {
                    offset: offset.to_string(),
                    relative_to: ReminderRelativeTo::Start,
                },
                action: Some("DISPLAY".to_string()),
            };
            assert_eq!(
                reminder_to_row(&reminder).map(|row| row.minutes_before),
                expected,
                "offset {offset}"
            );
        }
    }

    #[test]
    fn idmap_calendar_can_edit_maps_from_can_update_events() {
        let calendar = Calendar {
            id: CalendarId("calendar".to_string()),
            native_id: "calendar".to_string(),
            name: "Calendar".to_string(),
            color: None,
            provenance: CalendarProvenance {
                provider: ProtocolKind::Gmail,
                native: "calendar".to_string(),
                calendar_native: None,
            },
            is_default: false,
            can_create_events: true,
            can_update_events: false,
            can_delete_events: true,
        };
        assert!(!to_discovered_calendar("account", &calendar).can_edit);
    }

    fn sample_input() -> CalendarEventInput {
        CalendarEventInput {
            title: "Launch".to_string(),
            description: "desc".to_string(),
            location: "Room 1".to_string(),
            start_time: 1_770_112_800, // 2026-02-03T10:00:00Z
            end_time: 1_770_114_600,
            is_all_day: false,
            timezone: Some("UTC".to_string()),
            recurrence_rule: Some("FREQ=WEEKLY".to_string()),
            availability: Some("free".to_string()),
            visibility: None,
        }
    }

    #[test]
    fn idmap_epoch_to_event_time_roundtrips_all_day() {
        let epoch = 1_767_312_000; // 2026-01-02 UTC midnight
        let time = epoch_to_event_time(epoch, true, Some("Europe/Oslo"));
        assert_eq!(time.value, "2026-01-02");
        assert_eq!(time.timezone, None);
        assert_eq!(event_time_to_epoch(&time, true), Some(epoch));
    }

    #[test]
    fn idmap_epoch_to_event_time_roundtrips_zoned_and_floating() {
        let epoch = 1_767_319_445; // 2026-01-02T02:04:05Z
        // A UTC zone emits the canonical `Z` form, not `+00:00`.
        let utc = epoch_to_event_time(epoch, false, Some("UTC"));
        assert_eq!(utc.value, "2026-01-02T02:04:05Z");
        assert_eq!(event_time_to_epoch(&utc, false), Some(epoch));
        // Floating (no timezone) also emits `Z` and round-trips.
        let floating = epoch_to_event_time(epoch, false, None);
        assert_eq!(floating.value, "2026-01-02T02:04:05Z");
        assert_eq!(event_time_to_epoch(&floating, false), Some(epoch));
        // A non-UTC zone keeps its offset and still round-trips.
        let oslo = epoch_to_event_time(epoch, false, Some("Europe/Oslo"));
        assert_eq!(oslo.value, "2026-01-02T03:04:05+01:00");
        assert_eq!(event_time_to_epoch(&oslo, false), Some(epoch));
    }

    #[test]
    fn idmap_parse_availability_visibility_rsvp_inverse() {
        for value in [
            EventAvailability::Busy,
            EventAvailability::Free,
            EventAvailability::Tentative,
            EventAvailability::OutOfOffice,
        ] {
            assert_eq!(parse_availability(&availability(value)), value);
        }
        for value in [
            EventVisibility::Default,
            EventVisibility::Public,
            EventVisibility::Private,
            EventVisibility::Confidential,
        ] {
            assert_eq!(parse_visibility(&visibility(value)), value);
        }
        for value in [
            RsvpStatus::NeedsAction,
            RsvpStatus::Accepted,
            RsvpStatus::Declined,
            RsvpStatus::Tentative,
            RsvpStatus::Delegated,
        ] {
            assert_eq!(parse_rsvp(&rsvp(value)), value);
        }
        // Unknown strings collapse to the neutral write defaults.
        assert_eq!(parse_availability("nonsense"), EventAvailability::Busy);
        assert_eq!(parse_visibility("nonsense"), EventVisibility::Default);
        assert_eq!(parse_rsvp("nonsense"), RsvpStatus::NeedsAction);
    }

    #[test]
    fn idmap_input_to_event_create_maps_recurrence_and_defaults() {
        let create = input_to_event_create("cal-remote", &sample_input());
        assert_eq!(create.calendar_id, CalendarId("cal-remote".to_string()));
        assert_eq!(create.title, Some("Launch".to_string()));
        assert_eq!(create.status, EventStatus::Confirmed);
        assert_eq!(create.availability, EventAvailability::Free);
        // `visibility: None` from the form collapses to the neutral default.
        assert_eq!(create.visibility, EventVisibility::Default);
        assert!(create.organizer.is_none());
        assert!(create.attendees.is_empty());
        assert_eq!(create.recurrence.rrule, Some("FREQ=WEEKLY".to_string()));
        assert_eq!(create.start.value, "2026-02-03T10:00:00Z");
    }

    #[test]
    fn idmap_input_to_event_patch_sends_all_form_fields() {
        let patch = input_to_event_patch(&sample_input());
        assert_eq!(patch.calendar_id, None);
        assert_eq!(patch.title, Some(Some("Launch".to_string())));
        assert_eq!(patch.description, Some(Some("desc".to_string())));
        assert_eq!(patch.location, Some(Some("Room 1".to_string())));
        assert_eq!(patch.is_all_day, Some(false));
        assert_eq!(patch.availability, Some(EventAvailability::Free));
        assert_eq!(patch.visibility, Some(EventVisibility::Default));
        assert!(patch.start.is_some());
        assert!(patch.end.is_some());
        // Fields the form does not edit stay `None` (patch semantics).
        assert!(patch.status.is_none());
        assert!(patch.attendees.is_none());
        assert_eq!(
            patch.recurrence.as_ref().and_then(|r| r.rrule.clone()),
            Some("FREQ=WEEKLY".to_string())
        );
    }

    #[test]
    fn idmap_strip_event_id_reverses_composite_for_gmail_graph() {
        assert_eq!(
            strip_event_id(ProtocolKind::Gmail, &EventId("cal::evt".to_string())),
            "evt"
        );
        assert_eq!(
            strip_event_id(ProtocolKind::Graph, &EventId("cal::evt".to_string())),
            "evt"
        );
        // JMAP / CalDAV ids are bare and pass through unchanged.
        assert_eq!(
            strip_event_id(ProtocolKind::Jmap, &EventId("evt".to_string())),
            "evt"
        );
        assert_eq!(
            strip_event_id(ProtocolKind::CalDav, &EventId("/cal/evt.ics".to_string())),
            "/cal/evt.ics"
        );
    }

    #[test]
    fn idmap_event_id_for_writeback_composes_gmail_graph_and_passes_bare() {
        let (gmail, gmail_cal) =
            event_id_for_writeback(ProtocolKind::Gmail, "evt", None, Some("cal"));
        assert_eq!(gmail, EventId("cal::evt".to_string()));
        assert_eq!(gmail_cal, Some(CalendarId("cal".to_string())));
        let (jmap, _) = event_id_for_writeback(ProtocolKind::Jmap, "evt", None, Some("cal"));
        assert_eq!(jmap, EventId("evt".to_string()));
        // Gmail without a calendar id falls back to the bare event id.
        let (bare, _) = event_id_for_writeback(ProtocolKind::Gmail, "evt", None, None);
        assert_eq!(bare, EventId("evt".to_string()));
    }
}
