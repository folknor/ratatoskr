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
    principal_url: Option<String>,
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
    pub fn principal_url(&self) -> Option<&str> {
        self.principal_url.as_deref()
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
    if config.home_url().is_none() {
        persist_discovery_results(
            db,
            account_id,
            client.principal_url(),
            client.calendar_home_url(),
        )
        .await;
    }
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

    let put_etag = client.put_event(&event_url, &ical_data, None).await?;
    finalize_event(&client, &event_url, &input, &ical_data, Some(&uid), put_etag).await
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

    let put_etag = client
        .put_event(remote_event_id, &ical_data, etag.as_deref())
        .await?;
    finalize_event(
        &client,
        remote_event_id,
        &merged,
        &ical_data,
        existing.uid.as_deref(),
        put_etag,
    )
    .await
}

/// Resolve a CalendarEventDto for the response of a successful PUT.
///
/// When the server returned an ETag on the PUT response we can build the
/// DTO directly from what we sent + that ETag, avoiding a follow-up GET
/// that some eventually-consistent backends (Exchange front-ends, certain
/// hosted CalDAV providers) race against the write. The GET path is still
/// preferred when the server didn't include an ETag - that gives us
/// server-side canonicalization for free, and any GET error surfaces as a
/// user-actionable error.
async fn finalize_event(
    client: &CalDavClient,
    event_url: &str,
    input: &serde_json::Map<String, serde_json::Value>,
    ical_data: &str,
    uid: Option<&str>,
    put_etag: Option<String>,
) -> Result<CalendarEventDto, String> {
    if let Some(etag) = put_etag {
        return Ok(synthesize_event_dto(event_url, input, ical_data, uid, etag));
    }
    fetch_caldav_event(client, event_url).await
}

fn synthesize_event_dto(
    event_url: &str,
    input: &serde_json::Map<String, serde_json::Value>,
    ical_data: &str,
    uid: Option<&str>,
    etag: String,
) -> CalendarEventDto {
    let summary = input
        .get("summary")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let description = input
        .get("description")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let location = input
        .get("location")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let start_time = input
        .get("startTime")
        .and_then(serde_json::Value::as_str)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.timestamp())
        .unwrap_or_default();
    let end_time = input
        .get("endTime")
        .and_then(serde_json::Value::as_str)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.timestamp())
        .unwrap_or(start_time + 3600);
    let is_all_day = input
        .get("isAllDay")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

    CalendarEventDto {
        remote_event_id: event_url.to_string(),
        uid: uid.map(str::to_string),
        etag: Some(etag),
        summary: summary.clone(),
        title: summary,
        description,
        location,
        start_time,
        end_time,
        is_all_day,
        status: "confirmed".to_string(),
        ical_data: Some(ical_data.to_string()),
        ..CalendarEventDto::default()
    }
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
        // `caldav_principal_url` is read alongside `caldav_home_url` so a
        // previously discovered principal can be reused on cold-start sync.
        // Without this the client redoes the principal-discovery PROPFIND
        // every time, which is two extra round-trips and a regression on
        // servers where principal discovery from the bare base URL fails
        // (e.g. some DAViCal installs require the per-user path).
        let row = conn
            .query_row(
                "SELECT email, caldav_url, caldav_username, caldav_password,
                        caldav_principal_url, caldav_home_url
                 FROM accounts WHERE id = ?1",
                rusqlite::params![account_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
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
            principal_url: row.4.filter(|value| !value.trim().is_empty()),
            home_url: row.5.filter(|value| !value.trim().is_empty()),
        })
    })
    .await
}

async fn build_client(config: &CaldavAccountConfig) -> Result<CalDavClient, String> {
    build_client_from_config(config).await
}

/// Construct a `CalDavClient` from a `CaldavAccountConfig`, replaying any
/// previously discovered principal / home URLs so the second-and-later
/// PROPFIND round-trips are skipped. If `home_url` was not persisted, runs
/// full discovery.
///
/// Public so the sync path can use the same wiring without duplicating
/// the build-then-discover dance.
pub async fn build_client_from_config(
    config: &CaldavAccountConfig,
) -> Result<CalDavClient, String> {
    let mut client = CalDavClient::new(
        config.server_url(),
        config.username(),
        config.password(),
        AuthMethod::Basic,
    );
    if let Some(principal) = config.principal_url() {
        client.set_principal_url(principal);
    }
    if let Some(home) = config.home_url() {
        client.set_calendar_home_url(home);
    }
    if config.home_url().is_none() {
        client.discover().await?;
    }
    Ok(client)
}

/// Clear the persisted principal / home URLs for an account, forcing the
/// next `build_client_from_config` call to run full RFC 6764 discovery.
/// Used by the sync layer as a recovery step when persisted URLs go stale
/// (server migration, principal deletion, the user moving to a new
/// hosting provider that kept the same credentials but changed the DAV
/// root). Best-effort: a write failure is logged and swallowed so the
/// caller can still attempt rediscovery in-memory.
pub async fn clear_persisted_caldav_urls(db: &DbState, account_id: &str) {
    let account_id = account_id.to_string();
    let result = db
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE accounts
                    SET caldav_principal_url = NULL,
                        caldav_home_url = NULL
                  WHERE id = ?1",
                rusqlite::params![account_id],
            )
            .map_err(|e| e.to_string())
        })
        .await;
    if let Err(e) = result {
        log::warn!("Failed to clear persisted CalDAV URLs: {e}");
    }
}

/// Persist freshly discovered principal / home URLs back to the `accounts`
/// table so the next sync can skip discovery. Called after `build_client`
/// completes for accounts that didn't already have these cached.
///
/// This is best-effort: a write failure is logged but not propagated, since
/// discovery already succeeded and the operation that triggered the build
/// can proceed regardless.
pub async fn persist_discovery_results(
    db: &DbState,
    account_id: &str,
    principal_url: Option<&str>,
    home_url: Option<&str>,
) {
    if principal_url.is_none() && home_url.is_none() {
        return;
    }
    let account_id = account_id.to_string();
    let principal = principal_url.map(String::from);
    let home = home_url.map(String::from);
    let result = db
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE accounts
                    SET caldav_principal_url = COALESCE(?2, caldav_principal_url),
                        caldav_home_url = COALESCE(?3, caldav_home_url)
                  WHERE id = ?1",
                rusqlite::params![account_id, principal, home],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .await;
    if let Err(err) = result {
        log::warn!("CalDAV: failed to persist discovery URLs for account: {err}");
    }
}

async fn fetch_caldav_event(
    client: &CalDavClient,
    event_url: &str,
) -> Result<CalendarEventDto, String> {
    let (ical_data, etag) = client.get_event_ical(event_url).await?;
    let mut events = parse_icalendar(&ical_data)?;
    // VTIMEZONE-only responses are returned post-PUT by some servers when
    // they're still propagating the new VEVENT. Surface a stub DTO so the
    // caller can record what we sent (event_url + etag + raw ical_data)
    // without failing the operation; the next sync replaces this with the
    // canonical event.
    let Some(parsed) = events.pop() else {
        log::warn!(
            "CalDAV: GET {event_url} returned VCALENDAR with no VEVENT; returning stub DTO"
        );
        return Ok(CalendarEventDto {
            remote_event_id: event_url.to_string(),
            etag,
            ical_data: Some(ical_data),
            ..CalendarEventDto::default()
        });
    };

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
