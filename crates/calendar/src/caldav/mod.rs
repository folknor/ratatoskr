mod ical;

use rusqlite::OptionalExtension;

use rtsk::caldav::client::{AuthMethod, CalDavClient};
use rtsk::caldav::parse::parse_icalendar;
use rtsk::db::DbState;

use super::types::{CalendarEventDto, CalendarInfoDto};

pub struct CaldavAccountConfig {
    server_url: String,
    username: String,
    password: String,
    home_url: Option<String>,
}

impl CaldavAccountConfig {
    pub fn server_url(&self) -> &str {
        &self.server_url
    }
    pub fn username(&self) -> &str {
        &self.username
    }
    pub fn password(&self) -> &str {
        &self.password
    }
    pub fn home_url(&self) -> Option<&str> {
        self.home_url.as_deref()
    }
}

pub async fn caldav_list_calendars_impl(
    account_id: &str,
    db: &DbState,
    encryption_key: &[u8; 32],
) -> Result<Vec<CalendarInfoDto>, String> {
    let config = load_caldav_account_config(db, encryption_key, account_id).await?;
    let client = build_client(&config).await?;
    let discovered = client.list_calendars().await?;

    Ok(discovered
        .into_iter()
        .enumerate()
        .map(|(idx, cal)| CalendarInfoDto {
            display_name: cal
                .display_name
                .unwrap_or_else(|| format!("Calendar {}", idx + 1)),
            color: cal.color,
            is_primary: idx == 0,
            remote_id: cal.href,
            // CalDAV doesn't return per-calendar privileges in the standard
            // PROPFIND we issue; assume editable until we wire
            // <D:current-user-privilege-set>.
            can_edit: true,
        })
        .collect())
}

pub async fn caldav_create_event_impl(
    db: &DbState,
    encryption_key: &[u8; 32],
    account_id: &str,
    calendar_remote_id: &str,
    event: serde_json::Value,
) -> Result<CalendarEventDto, String> {
    let config = load_caldav_account_config(db, encryption_key, account_id).await?;
    let client = build_client(&config).await?;

    let input = ical::parse_caldav_event_input(&event)?;
    let uid = uuid::Uuid::new_v4().to_string();
    let ical_data = ical::build_caldav_ical_event(&input, Some(&uid));
    let event_url = join_calendar_path(calendar_remote_id, &format!("{uid}.ics"))?;

    client.put_event(&event_url, &ical_data, None).await?;
    fetch_caldav_event(&client, &event_url).await
}

pub async fn caldav_update_event_impl(
    db: &DbState,
    encryption_key: &[u8; 32],
    account_id: &str,
    remote_event_id: &str,
    event: serde_json::Value,
    etag: Option<String>,
) -> Result<CalendarEventDto, String> {
    let config = load_caldav_account_config(db, encryption_key, account_id).await?;
    let client = build_client(&config).await?;

    let input = ical::parse_caldav_event_input(&event)?;
    let existing = fetch_caldav_event(&client, remote_event_id).await?;
    let merged = ical::merge_caldav_event_input(&existing, &input);
    let ical_data = ical::build_caldav_ical_event(&merged, existing.uid.as_deref());

    client
        .put_event(remote_event_id, &ical_data, etag.as_deref())
        .await?;
    fetch_caldav_event(&client, remote_event_id).await
}

pub async fn caldav_delete_event_impl(
    db: &DbState,
    encryption_key: &[u8; 32],
    account_id: &str,
    remote_event_id: &str,
    etag: Option<String>,
) -> Result<(), String> {
    let config = load_caldav_account_config(db, encryption_key, account_id).await?;
    let client = build_client(&config).await?;
    client.delete_event(remote_event_id, etag.as_deref()).await
}

pub async fn load_caldav_account_config(
    db: &DbState,
    encryption_key: &[u8; 32],
    account_id: &str,
) -> Result<CaldavAccountConfig, String> {
    let key = *encryption_key;
    let account_id = account_id.to_string();
    db.with_conn(move |conn| {
        let row = conn
            .query_row(
                "SELECT email, caldav_url, caldav_username, caldav_password, caldav_home_url
                 FROM accounts WHERE id = ?1",
                rusqlite::params![account_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| format!("query caldav account: {e}"))?
            .ok_or_else(|| "Account not found".to_string())?;

        let server_url = row
            .1
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "CalDAV credentials not configured".to_string())?;
        let password_raw = row
            .3
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "CalDAV credentials not configured".to_string())?;
        let password = if rtsk::provider::crypto::is_encrypted(&password_raw) {
            rtsk::provider::crypto::decrypt_value(&key, &password_raw).unwrap_or(password_raw)
        } else {
            password_raw
        };
        let username = row
            .2
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(row.0);

        Ok(CaldavAccountConfig {
            server_url,
            username,
            password,
            home_url: row.4.filter(|value| !value.trim().is_empty()),
        })
    })
    .await
}

async fn build_client(config: &CaldavAccountConfig) -> Result<CalDavClient, String> {
    let mut client = CalDavClient::new(
        &config.server_url,
        &config.username,
        &config.password,
        AuthMethod::Basic,
    );
    if let Some(home) = config.home_url.as_deref() {
        client.set_calendar_home_url(home);
    } else {
        client.discover().await?;
    }
    Ok(client)
}

async fn fetch_caldav_event(
    client: &CalDavClient,
    event_url: &str,
) -> Result<CalendarEventDto, String> {
    let (ical_data, etag) = client.get_event_ical(event_url).await?;
    let mut events = parse_icalendar(&ical_data)?;
    let parsed = events
        .pop()
        .ok_or_else(|| format!("no VEVENT in CalDAV response for {event_url}"))?;

    let attendees_json = if parsed.attendees.is_empty() {
        None
    } else {
        let values: Vec<serde_json::Value> = parsed
            .attendees
            .iter()
            .map(|a| {
                serde_json::json!({
                    "email": a.email,
                    "displayName": a.name,
                    "responseStatus": a.partstat.as_deref().map(str::to_lowercase),
                })
            })
            .collect();
        serde_json::to_string(&values).ok()
    };

    let start_time = parsed.start_time.unwrap_or(0);
    let end_time = parsed.end_time.unwrap_or(start_time + 3600);
    let status = if parsed.status.is_empty() {
        "confirmed".to_string()
    } else {
        parsed.status.to_lowercase()
    };

    Ok(CalendarEventDto {
        remote_event_id: event_url.to_string(),
        uid: parsed.uid,
        etag,
        summary: parsed.summary.clone(),
        title: parsed.summary,
        description: parsed.description,
        location: parsed.location,
        start_time,
        end_time,
        is_all_day: parsed.is_all_day,
        status,
        organizer_email: parsed.organizer_email,
        attendees_json,
        html_link: None,
        ical_data: Some(ical_data),
        recurrence_rule: parsed.rrule,
        organizer_name: parsed.organizer_name,
        ..CalendarEventDto::default()
    })
}

fn join_calendar_path(base: &str, segment: &str) -> Result<String, String> {
    let base_with_slash = if base.ends_with('/') {
        base.to_string()
    } else {
        format!("{base}/")
    };
    reqwest::Url::parse(&base_with_slash)
        .map_err(|e| format!("invalid calendar URL: {e}"))?
        .join(segment)
        .map(|u| u.to_string())
        .map_err(|e| format!("invalid CalDAV path {segment}: {e}"))
}
