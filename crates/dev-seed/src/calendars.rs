use chrono::{Duration, Local, NaiveDate, TimeZone};
use rand::RngExt;
use rusqlite::{Connection, params};

use crate::accounts::Account;

struct SeededCalendar {
    id: String,
    account_id: String,
    provider: String,
    remote_id: String,
    display_name: String,
    color: String,
    is_primary: bool,
    is_visible: bool,
    sort_order: i64,
}

struct SeededEvent {
    id: String,
    account_id: String,
    calendar_id: String,
    google_event_id: String,
    title: String,
    description: String,
    location: String,
    start_time: i64,
    end_time: i64,
    is_all_day: bool,
    organizer_name: String,
    organizer_email: String,
    rsvp_status: String,
    availability: String,
    visibility: String,
}

pub fn seed_calendars(
    conn: &Connection,
    rng: &mut impl RngExt,
    accounts: &[Account],
) -> Result<(), String> {
    let today = Local::now().date_naive();

    for (idx, account) in accounts.iter().enumerate() {
        let primary = SeededCalendar {
            id: crate::next_uuid(rng),
            account_id: account.id.clone(),
            provider: account.provider.clone(),
            remote_id: format!("primary-{}", account.account_name.to_lowercase()),
            display_name: account.account_name.clone(),
            color: account.color.clone(),
            is_primary: true,
            is_visible: true,
            sort_order: 0,
        };
        insert_calendar(conn, &primary)?;
        seed_primary_events(conn, rng, account, &primary, today, idx)?;

        if let Some(secondary) = secondary_calendar_for_account(rng, account) {
            insert_calendar(conn, &secondary)?;
            seed_secondary_events(conn, rng, account, &secondary, today, idx)?;
        }
    }

    Ok(())
}

fn secondary_calendar_for_account(rng: &mut impl RngExt, account: &Account) -> Option<SeededCalendar> {
    let (name, color) = match account.account_name.as_str() {
        "Personal" => ("Travel", "#00acc1"),
        "Work" => ("Team", "#7e57c2"),
        _ => return None,
    };

    Some(SeededCalendar {
        id: crate::next_uuid(rng),
        account_id: account.id.clone(),
        provider: account.provider.clone(),
        remote_id: format!("{}-{}", account.account_name.to_lowercase(), name.to_lowercase()),
        display_name: name.to_string(),
        color: color.to_string(),
        is_primary: false,
        is_visible: true,
        sort_order: 1,
    })
}

fn seed_primary_events(
    conn: &Connection,
    rng: &mut impl RngExt,
    account: &Account,
    calendar: &SeededCalendar,
    today: NaiveDate,
    idx: usize,
) -> Result<(), String> {
    let base_day = today + Duration::days(i64::try_from(idx).unwrap_or(0));

    let (title, location, start_h, start_m, end_h, end_m, description) = match account.account_name.as_str() {
        "Personal" => (
            "Dinner with Nora",
            "Grunerlokka",
            18,
            30,
            20,
            0,
            "Catch up over dinner and plan the weekend.",
        ),
        "Work" => (
            "Sprint Planning",
            "HQ - Fjord Room",
            9,
            30,
            10,
            30,
            "Review priorities for the next sprint and confirm owners.",
        ),
        "Office" => (
            "Budget Review",
            "Teams",
            14,
            0,
            15,
            0,
            "Quarterly budget review with finance and operations.",
        ),
        _ => (
            "Design Crit",
            "Studio",
            11,
            0,
            12,
            0,
            "Walk through the latest product work and collect feedback.",
        ),
    };

    let event = SeededEvent {
        id: crate::next_uuid(rng),
        account_id: account.id.clone(),
        calendar_id: calendar.id.clone(),
        google_event_id: format!("devseed-{}-primary", account.account_name.to_lowercase()),
        title: title.to_string(),
        description: description.to_string(),
        location: location.to_string(),
        start_time: local_timestamp(base_day, start_h, start_m)?,
        end_time: local_timestamp(base_day, end_h, end_m)?,
        is_all_day: false,
        organizer_name: account.display_name.clone(),
        organizer_email: account.email.clone(),
        rsvp_status: "accepted".to_string(),
        availability: "busy".to_string(),
        visibility: "default".to_string(),
    };
    insert_event(conn, &event)?;
    insert_default_event_details(conn, &event, account, false)?;

    let all_day_date = today + Duration::days(3 + i64::try_from(idx).unwrap_or(0));
    let all_day = SeededEvent {
        id: crate::next_uuid(rng),
        account_id: account.id.clone(),
        calendar_id: calendar.id.clone(),
        google_event_id: format!("devseed-{}-allday", account.account_name.to_lowercase()),
        title: format!("{} Day", account.account_name),
        description: format!("Reserved day on the {} calendar.", account.account_name),
        location: String::new(),
        start_time: local_timestamp(all_day_date, 0, 0)?,
        end_time: local_timestamp(all_day_date + Duration::days(1), 0, 0)?,
        is_all_day: true,
        organizer_name: account.display_name.clone(),
        organizer_email: account.email.clone(),
        rsvp_status: "accepted".to_string(),
        availability: "busy".to_string(),
        visibility: "private".to_string(),
    };
    insert_event(conn, &all_day)?;
    insert_default_event_details(conn, &all_day, account, true)?;

    Ok(())
}

fn seed_secondary_events(
    conn: &Connection,
    rng: &mut impl RngExt,
    account: &Account,
    calendar: &SeededCalendar,
    today: NaiveDate,
    idx: usize,
) -> Result<(), String> {
    let (title, location, offset_days, start_h, start_m, end_h, end_m, description) =
        match calendar.display_name.as_str() {
            "Travel" => (
                "Flight to Copenhagen",
                "OSL",
                10,
                8,
                15,
                9,
                25,
                "Morning flight for the long weekend.",
            ),
            _ => (
                "Architecture Sync",
                "Team Room",
                4,
                13,
                0,
                14,
                0,
                "Cross-team architecture review and decision log update.",
            ),
        };

    let date = today + Duration::days(offset_days + i64::try_from(idx).unwrap_or(0));
    let event = SeededEvent {
        id: crate::next_uuid(rng),
        account_id: account.id.clone(),
        calendar_id: calendar.id.clone(),
        google_event_id: format!(
            "devseed-{}-{}",
            account.account_name.to_lowercase(),
            calendar.display_name.to_lowercase()
        ),
        title: title.to_string(),
        description: description.to_string(),
        location: location.to_string(),
        start_time: local_timestamp(date, start_h, start_m)?,
        end_time: local_timestamp(date, end_h, end_m)?,
        is_all_day: false,
        organizer_name: account.display_name.clone(),
        organizer_email: account.email.clone(),
        rsvp_status: "tentative".to_string(),
        availability: "busy".to_string(),
        visibility: "default".to_string(),
    };
    insert_event(conn, &event)?;
    insert_default_event_details(conn, &event, account, false)?;

    Ok(())
}

fn insert_calendar(conn: &Connection, calendar: &SeededCalendar) -> Result<(), String> {
    conn.execute(
        "INSERT INTO calendars (
             id, account_id, provider, remote_id, display_name, color,
             is_primary, is_visible, created_at, updated_at, sort_order, is_default
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, unixepoch(), unixepoch(), ?9, ?10)",
        params![
            calendar.id,
            calendar.account_id,
            calendar.provider,
            calendar.remote_id,
            calendar.display_name,
            calendar.color,
            calendar.is_primary as i64,
            calendar.is_visible as i64,
            calendar.sort_order,
            calendar.is_primary as i64,
        ],
    )
    .map_err(|e| format!("insert calendar: {e}"))?;

    Ok(())
}

fn insert_event(conn: &Connection, event: &SeededEvent) -> Result<(), String> {
    conn.execute(
        "INSERT INTO calendar_events (
             id, account_id, google_event_id, summary, description, location,
             start_time, end_time, is_all_day, status, organizer_email,
             calendar_id, title, timezone, recurrence_rule, organizer_name,
             rsvp_status, created_at, updated_at, availability, visibility
         ) VALUES (
             ?1, ?2, ?3, ?4, ?5, ?6,
             ?7, ?8, ?9, 'confirmed', ?10,
             ?11, ?12, 'Europe/Oslo', NULL, ?13,
             ?14, unixepoch(), unixepoch(), ?15, ?16
         )",
        params![
            event.id,
            event.account_id,
            event.google_event_id,
            event.title,
            event.description,
            event.location,
            event.start_time,
            event.end_time,
            event.is_all_day as i64,
            event.organizer_email,
            event.calendar_id,
            event.title,
            event.organizer_name,
            event.rsvp_status,
            event.availability,
            event.visibility,
        ],
    )
    .map_err(|e| format!("insert calendar event: {e}"))?;

    Ok(())
}

fn insert_default_event_details(
    conn: &Connection,
    event: &SeededEvent,
    account: &Account,
    all_day: bool,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO calendar_attendees (event_id, account_id, email, name, rsvp_status, is_organizer)
         VALUES (?1, ?2, ?3, ?4, ?5, 1)",
        params![
            event.id,
            event.account_id,
            event.organizer_email,
            event.organizer_name,
            event.rsvp_status,
        ],
    )
    .map_err(|e| format!("insert organizer attendee: {e}"))?;

    let guest_email = guest_email_for_account(account);
    conn.execute(
        "INSERT INTO calendar_attendees (event_id, account_id, email, name, rsvp_status, is_organizer)
         VALUES (?1, ?2, ?3, ?4, 'accepted', 0)",
        params![
            event.id,
            event.account_id,
            guest_email,
            guest_name_for_account(account),
        ],
    )
    .map_err(|e| format!("insert guest attendee: {e}"))?;

    let reminder_minutes = if all_day { 60 * 24 } else { 15 };
    conn.execute(
        "INSERT INTO calendar_reminders (event_id, account_id, minutes_before, method)
         VALUES (?1, ?2, ?3, 'popup')",
        params![event.id, event.account_id, reminder_minutes],
    )
    .map_err(|e| format!("insert calendar reminder: {e}"))?;

    Ok(())
}

fn local_timestamp(date: NaiveDate, hour: u32, minute: u32) -> Result<i64, String> {
    let naive = date
        .and_hms_opt(hour, minute, 0)
        .ok_or_else(|| format!("invalid local time {date} {hour}:{minute:02}"))?;
    Local
        .from_local_datetime(&naive)
        .single()
        .or_else(|| Local.from_local_datetime(&naive).earliest())
        .map(|dt| dt.timestamp())
        .ok_or_else(|| format!("ambiguous local time {date} {hour}:{minute:02}"))
}

fn guest_email_for_account(account: &Account) -> &'static str {
    match account.account_name.as_str() {
        "Personal" => "nora@example.com",
        "Work" => "team-lead@company.io",
        "Office" => "finance@outlook.example",
        _ => "design@example.com",
    }
}

fn guest_name_for_account(account: &Account) -> &'static str {
    match account.account_name.as_str() {
        "Personal" => "Nora",
        "Work" => "Priya Shah",
        "Office" => "Finance Team",
        _ => "Design Review Crew",
    }
}
