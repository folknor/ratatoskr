use rusqlite::OptionalExtension;
use tauri::State;

use crate::db::DbState;
use crate::provider::crypto::AppCryptoState;

use super::types::{CalendarEventDto, CalendarInfoDto, CalendarSyncResultDto};
use super::{CALDAV_NS, shared_http_client};

pub(super) struct CaldavAccountConfig {
    server_url: String,
    username: String,
    password: String,
    principal_url: Option<String>,
    home_url: Option<String>,
}

#[tauri::command]
pub async fn caldav_list_calendars(
    account_id: String,
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
) -> Result<Vec<CalendarInfoDto>, String> {
    caldav_list_calendars_impl(&account_id, &db, crypto.encryption_key()).await
}

pub(super) async fn caldav_list_calendars_impl(
    account_id: &str,
    db: &DbState,
    encryption_key: &[u8; 32],
) -> Result<Vec<CalendarInfoDto>, String> {
    let config = load_caldav_account_config(db, encryption_key, account_id).await?;
    let client = shared_http_client();
    let home_url = resolve_caldav_home_url(client, &config).await?;
    list_caldav_calendars(client, &config, &home_url).await
}

#[tauri::command]
pub async fn caldav_fetch_events(
    account_id: String,
    calendar_remote_id: String,
    time_min: String,
    time_max: String,
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
) -> Result<Vec<CalendarEventDto>, String> {
    let config = load_caldav_account_config(&db, crypto.encryption_key(), &account_id).await?;
    let client = shared_http_client();
    fetch_caldav_events(client, &config, &calendar_remote_id, &time_min, &time_max).await
}

#[tauri::command]
pub async fn caldav_sync_events(
    account_id: String,
    calendar_remote_id: String,
    _sync_token: Option<String>,
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
) -> Result<CalendarSyncResultDto, String> {
    caldav_sync_events_impl(&account_id, &calendar_remote_id, &db, crypto.encryption_key()).await
}

pub(super) async fn caldav_sync_events_impl(
    account_id: &str,
    calendar_remote_id: &str,
    db: &DbState,
    encryption_key: &[u8; 32],
) -> Result<CalendarSyncResultDto, String> {
    let config = load_caldav_account_config(db, encryption_key, account_id).await?;
    let client = shared_http_client();
    let time_min = (chrono::Utc::now() - chrono::Duration::days(90)).to_rfc3339();
    let time_max = (chrono::Utc::now() + chrono::Duration::days(365)).to_rfc3339();
    let created =
        fetch_caldav_events(client, &config, calendar_remote_id, &time_min, &time_max).await?;
    Ok(CalendarSyncResultDto {
        created,
        updated: Vec::new(),
        deleted_remote_ids: Vec::new(),
        new_sync_token: None,
        new_ctag: None,
    })
}

#[tauri::command]
pub async fn caldav_test_connection(
    account_id: String,
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
) -> Result<serde_json::Value, String> {
    let config = load_caldav_account_config(&db, crypto.encryption_key(), &account_id).await?;
    let client = shared_http_client();
    let result = match resolve_caldav_home_url(client, &config).await {
        Ok(home_url) => list_caldav_calendars(client, &config, &home_url).await,
        Err(error) => Err(error),
    };

    match result {
        Ok(calendars) => Ok(serde_json::json!({
            "success": true,
            "message": format!(
                "Connected — found {} calendar{}",
                calendars.len(),
                if calendars.len() == 1 { "" } else { "s" }
            )
        })),
        Err(error) => Ok(serde_json::json!({
            "success": false,
            "message": error,
        })),
    }
}

#[tauri::command]
pub async fn caldav_create_event(
    account_id: String,
    calendar_remote_id: String,
    event: serde_json::Value,
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
) -> Result<CalendarEventDto, String> {
    let config = load_caldav_account_config(&db, crypto.encryption_key(), &account_id).await?;
    let client = shared_http_client();
    let input = parse_caldav_event_input(event)?;
    let uid = uuid::Uuid::new_v4().to_string();
    let ical_data = build_caldav_ical_event(&input, Some(&uid));
    let remote_event_id = join_url_path(&calendar_remote_id, &format!("{uid}.ics"))?;

    caldav_request_with_headers(
        client,
        &config,
        "PUT",
        &remote_event_id,
        Some(&ical_data),
        None,
        &[("Content-Type", "text/calendar; charset=utf-8")],
    )
    .await?;

    fetch_caldav_event_by_href(client, &config, &remote_event_id).await
}

#[tauri::command]
pub async fn caldav_update_event(
    account_id: String,
    _calendar_remote_id: String,
    remote_event_id: String,
    event: serde_json::Value,
    etag: Option<String>,
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
) -> Result<CalendarEventDto, String> {
    let config = load_caldav_account_config(&db, crypto.encryption_key(), &account_id).await?;
    let client = shared_http_client();
    let input = parse_caldav_event_input(event)?;
    let existing = fetch_caldav_event_by_href(client, &config, &remote_event_id).await?;
    let merged = merge_caldav_event_input(&existing, &input);
    let ical_data = build_caldav_ical_event(&merged, existing.uid.as_deref());

    let mut headers = vec![("Content-Type", "text/calendar; charset=utf-8")];
    if let Some(etag_value) = etag.as_deref() {
        headers.push(("If-Match", etag_value));
    }

    caldav_request_with_headers(
        client,
        &config,
        "PUT",
        &remote_event_id,
        Some(&ical_data),
        None,
        &headers,
    )
    .await?;

    fetch_caldav_event_by_href(client, &config, &remote_event_id).await
}

#[tauri::command]
pub async fn caldav_delete_event(
    account_id: String,
    _calendar_remote_id: String,
    remote_event_id: String,
    etag: Option<String>,
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
) -> Result<(), String> {
    let config = load_caldav_account_config(&db, crypto.encryption_key(), &account_id).await?;
    let client = shared_http_client();
    let mut headers = Vec::new();
    if let Some(etag_value) = etag.as_deref() {
        headers.push(("If-Match", etag_value));
    }
    caldav_request_with_headers(
        client,
        &config,
        "DELETE",
        &remote_event_id,
        None,
        None,
        &headers,
    )
    .await?;
    Ok(())
}

pub(super) async fn load_caldav_account_config(
    db: &DbState,
    encryption_key: &[u8; 32],
    account_id: &str,
) -> Result<CaldavAccountConfig, String> {
    let key = *encryption_key;
    let account_id = account_id.to_string();
    db.with_conn(move |conn| {
        let row = conn
            .query_row(
                "SELECT email, caldav_url, caldav_username, caldav_password, caldav_principal_url, caldav_home_url
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
        let password = if crate::provider::crypto::is_encrypted(&password_raw) {
            crate::provider::crypto::decrypt_value(&key, &password_raw).unwrap_or(password_raw)
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

async fn resolve_caldav_home_url(
    client: &reqwest::Client,
    config: &CaldavAccountConfig,
) -> Result<String, String> {
    if let Some(home_url) = config.home_url.as_ref() {
        return Ok(home_url.clone());
    }

    let principal_url = if let Some(principal) = config.principal_url.as_ref() {
        principal.clone()
    } else {
        let body = r#"<?xml version="1.0" encoding="utf-8" ?>
<d:propfind xmlns:d="DAV:">
  <d:prop>
    <d:current-user-principal />
  </d:prop>
</d:propfind>"#;
        let response = caldav_request(
            client,
            config,
            "PROPFIND",
            &config.server_url,
            Some(body),
            Some("0"),
        )
        .await?;
        let xml = response
            .text()
            .await
            .map_err(|e| format!("read principal response: {e}"))?;
        let href = extract_first_href_for_property(&xml, &["current-user-principal"]).ok_or_else(
            || "CalDAV discovery failed: current-user-principal not found".to_string(),
        )?;
        resolve_href(&config.server_url, &href)?
    };

    let body = format!(
        r#"<?xml version="1.0" encoding="utf-8" ?>
<d:propfind xmlns:d="DAV:" xmlns:c="{CALDAV_NS}">
  <d:prop>
    <c:calendar-home-set />
  </d:prop>
</d:propfind>"#
    );
    let response = caldav_request(
        client,
        config,
        "PROPFIND",
        &principal_url,
        Some(&body),
        Some("0"),
    )
    .await?;
    let xml = response
        .text()
        .await
        .map_err(|e| format!("read home response: {e}"))?;
    let href = extract_first_href_for_property(&xml, &["calendar-home-set"])
        .ok_or_else(|| "CalDAV discovery failed: calendar-home-set not found".to_string())?;
    resolve_href(&principal_url, &href)
}

async fn list_caldav_calendars(
    client: &reqwest::Client,
    config: &CaldavAccountConfig,
    home_url: &str,
) -> Result<Vec<CalendarInfoDto>, String> {
    let body = format!(
        r#"<?xml version="1.0" encoding="utf-8" ?>
<d:propfind xmlns:d="DAV:" xmlns:c="{CALDAV_NS}" xmlns:cs="http://calendarserver.org/ns/">
  <d:prop>
    <d:displayname />
    <cs:calendar-color />
    <d:resourcetype />
  </d:prop>
</d:propfind>"#
    );
    let response =
        caldav_request(client, config, "PROPFIND", home_url, Some(&body), Some("1")).await?;
    let xml = response
        .text()
        .await
        .map_err(|e| format!("read calendars response: {e}"))?;
    let responses = split_xml_responses(&xml);
    let mut calendars = Vec::new();

    for response_xml in responses {
        if !contains_any_tag(response_xml, &["calendar"]) {
            continue;
        }

        let Some(href) = extract_first_tag_value(response_xml, &["href"]) else {
            continue;
        };
        let remote_id = resolve_href(home_url, &href)?;
        if normalize_url_for_compare(&remote_id) == normalize_url_for_compare(home_url) {
            continue;
        }

        let display_name = extract_first_tag_value(response_xml, &["displayname"])
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| format!("Calendar {}", calendars.len() + 1));
        let color = extract_first_tag_value(response_xml, &["calendar-color"]);

        calendars.push(CalendarInfoDto {
            remote_id,
            display_name,
            color,
            is_primary: calendars.is_empty(),
        });
    }

    Ok(calendars)
}

async fn fetch_caldav_events(
    client: &reqwest::Client,
    config: &CaldavAccountConfig,
    calendar_remote_id: &str,
    time_min: &str,
    time_max: &str,
) -> Result<Vec<CalendarEventDto>, String> {
    let time_min = chrono::DateTime::parse_from_rfc3339(time_min)
        .map_err(|e| format!("invalid CalDAV timeMin: {e}"))?
        .with_timezone(&chrono::Utc)
        .format("%Y%m%dT%H%M%SZ")
        .to_string();
    let time_max = chrono::DateTime::parse_from_rfc3339(time_max)
        .map_err(|e| format!("invalid CalDAV timeMax: {e}"))?
        .with_timezone(&chrono::Utc)
        .format("%Y%m%dT%H%M%SZ")
        .to_string();
    let body = format!(
        r#"<?xml version="1.0" encoding="utf-8" ?>
<c:calendar-query xmlns:d="DAV:" xmlns:c="{CALDAV_NS}">
  <d:prop>
    <d:getetag />
    <c:calendar-data />
  </d:prop>
  <c:filter>
    <c:comp-filter name="VCALENDAR">
      <c:comp-filter name="VEVENT">
        <c:time-range start="{time_min}" end="{time_max}" />
      </c:comp-filter>
    </c:comp-filter>
  </c:filter>
</c:calendar-query>"#
    );
    let response = caldav_request(
        client,
        config,
        "REPORT",
        calendar_remote_id,
        Some(&body),
        Some("1"),
    )
    .await?;
    let xml = response
        .text()
        .await
        .map_err(|e| format!("read events response: {e}"))?;
    let responses = split_xml_responses(&xml);
    let mut events = Vec::new();

    for response_xml in responses {
        let Some(calendar_data) = extract_first_tag_value(response_xml, &["calendar-data"]) else {
            continue;
        };
        let Some(href) = extract_first_tag_value(response_xml, &["href"]) else {
            continue;
        };
        let remote_event_id = resolve_href(calendar_remote_id, &href)?;
        let etag = extract_first_tag_value(response_xml, &["getetag"]);
        let mut event = parse_caldav_ical_event(&calendar_data, &remote_event_id)?;
        event.etag = etag;
        events.push(event);
    }

    Ok(events)
}

async fn caldav_request(
    client: &reqwest::Client,
    config: &CaldavAccountConfig,
    method: &str,
    url: &str,
    body: Option<&str>,
    depth: Option<&str>,
) -> Result<reqwest::Response, String> {
    caldav_request_with_headers(client, config, method, url, body, depth, &[]).await
}

async fn caldav_request_with_headers(
    client: &reqwest::Client,
    config: &CaldavAccountConfig,
    method: &str,
    url: &str,
    body: Option<&str>,
    depth: Option<&str>,
    headers: &[(&str, &str)],
) -> Result<reqwest::Response, String> {
    let method = reqwest::Method::from_bytes(method.as_bytes())
        .map_err(|e| format!("invalid CalDAV method {method}: {e}"))?;
    let mut request = client
        .request(method, url)
        .basic_auth(&config.username, Some(&config.password))
        .header("Accept", "application/xml, text/xml, */*");

    if let Some(depth_value) = depth {
        request = request.header("Depth", depth_value);
    }
    if let Some(body_value) = body {
        let caller_has_content_type = headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("content-type"));
        if !caller_has_content_type {
            request = request.header("Content-Type", "application/xml; charset=utf-8");
        }
        request = request.body(body_value.to_string());
    }
    for (name, value) in headers {
        request = request.header(*name, *value);
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("CalDAV request failed: {e}"))?;

    let status = response.status();
    if status.is_success() || status.as_u16() == 207 {
        return Ok(response);
    }

    let body = response.text().await.unwrap_or_default();
    Err(format!("CalDAV error: {status} {body}"))
}

fn xml_ns_prefixes_for<'a>(xml: &'a str, ns_uri: &str) -> Vec<std::borrow::Cow<'a, str>> {
    let mut prefixes: Vec<std::borrow::Cow<'a, str>> = vec!["".into()];
    let mut pos = 0;
    while let Some(rel) = xml[pos..].find("xmlns") {
        pos += rel + 5;
        let rest = &xml[pos..];
        let (prefix_colon, value_start) = if rest.starts_with(':') {
            let colon_end = rest[1..]
                .find(['=', ' ', '\t', '\r', '\n', '>'])
                .unwrap_or(rest.len());
            let prefix = &rest[1..colon_end + 1];
            let after = &rest[colon_end + 1..];
            let after = after.trim_start_matches(['=', ' ', '\t']);
            (Some(prefix), after)
        } else if rest.starts_with('=') {
            (None, &rest[1..])
        } else {
            continue;
        };
        let value_start = value_start.trim_start();
        let (value, _) = if value_start.starts_with('"') {
            let end = value_start[1..].find('"').unwrap_or(value_start.len());
            (&value_start[1..end + 1], &value_start[end + 2..])
        } else if value_start.starts_with('\'') {
            let end = value_start[1..].find('\'').unwrap_or(value_start.len());
            (&value_start[1..end + 1], &value_start[end + 2..])
        } else {
            continue;
        };
        if value == ns_uri
            && let Some(prefix) = prefix_colon
        {
            prefixes.push(format!("{prefix}:").into());
        }
    }
    prefixes
}

fn split_xml_responses(xml: &str) -> Vec<&str> {
    let dav_prefixes = xml_ns_prefixes_for(xml, "DAV:");
    let mut responses = Vec::new();
    let mut search_start = 0;
    let xml_lower = xml.to_lowercase();

    while let Some(start_rel) = xml_lower[search_start..].find('<') {
        let start = search_start + start_rel;
        let after_lt = &xml_lower[start + 1..];

        let matched_prefix: Option<&str> = dav_prefixes.iter().find_map(|prefix| {
            let open = format!("{prefix}response");
            if after_lt.starts_with(open.as_str()) {
                let rest = &after_lt[open.len()..];
                if matches!(
                    rest.as_bytes().first(),
                    Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\r') | Some(b'\n')
                ) {
                    Some(prefix.as_ref())
                } else {
                    None
                }
            } else {
                None
            }
        });

        let Some(prefix) = matched_prefix else {
            search_start = start + 1;
            continue;
        };

        let close = format!("</{prefix}response>");
        let Some(end_rel) = xml_lower[start..].find(&close) else {
            break;
        };
        let end = start + end_rel + close.len();
        responses.push(&xml[start..end]);
        search_start = end;
    }

    responses
}

fn extract_first_href_for_property(xml: &str, property_names: &[&str]) -> Option<String> {
    for property_name in property_names {
        if let Some(section) = extract_first_element(xml, property_name)
            && let Some(href) = extract_first_tag_value(section, &["href"])
        {
            return Some(href);
        }
    }
    None
}

fn extract_first_tag_value(xml: &str, tag_names: &[&str]) -> Option<String> {
    tag_names
        .iter()
        .find_map(|tag_name| extract_tag_value(xml, tag_name))
}

fn extract_tag_value(xml: &str, tag_name: &str) -> Option<String> {
    extract_first_element(xml, tag_name).and_then(extract_element_text)
}

fn extract_first_element<'a>(xml: &'a str, tag_name: &str) -> Option<&'a str> {
    let xml_lower = xml.to_lowercase();
    let tag_lower = tag_name.to_lowercase();
    let all_prefixes = {
        let mut prefixes: Vec<std::borrow::Cow<'_, str>> = Vec::new();
        for ns in [
            "DAV:",
            "urn:ietf:params:xml:ns:caldav",
            "http://calendarserver.org/ns/",
            "http://apple.com/ns/ical/",
        ] {
            prefixes.extend(xml_ns_prefixes_for(xml, ns));
        }
        let mut seen = std::collections::HashSet::new();
        prefixes.retain(|value| seen.insert(value.to_string()));
        prefixes
    };
    for prefix in &all_prefixes {
        let open = format!("<{prefix}{tag_lower}");
        let close = format!("</{prefix}{tag_lower}>");
        if let Some(start) = xml_lower.find(&open) {
            let after_name = &xml_lower[start + open.len()..];
            if !matches!(
                after_name.as_bytes().first(),
                Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\r') | Some(b'\n')
            ) {
                continue;
            }
            if let Some(end_rel) = xml_lower[start..].find(&close) {
                let end = start + end_rel + close.len();
                return Some(&xml[start..end]);
            }
        }
    }
    None
}

fn contains_any_tag(xml: &str, tag_names: &[&str]) -> bool {
    tag_names
        .iter()
        .any(|tag_name| extract_first_element(xml, tag_name).is_some())
}

fn resolve_href(base: &str, href: &str) -> Result<String, String> {
    reqwest::Url::parse(base)
        .map_err(|e| format!("invalid base url: {e}"))?
        .join(href)
        .map(|url| url.to_string())
        .map_err(|e| format!("invalid CalDAV href {href}: {e}"))
}

fn normalize_url_for_compare(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

fn extract_element_text(element: &str) -> Option<String> {
    let mut reader = quick_xml::Reader::from_str(element);
    reader.config_mut().trim_text(true);
    let mut depth = 0usize;
    let mut text = String::new();

    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(_)) => depth += 1,
            Ok(quick_xml::events::Event::Text(event)) => {
                if depth >= 1
                    && let Ok(unescaped) = event.unescape()
                {
                    text.push_str(&unescaped);
                }
            }
            Ok(quick_xml::events::Event::CData(event)) => {
                if depth >= 1
                    && let Ok(decoded) = event.decode()
                {
                    text.push_str(&decoded);
                }
            }
            Ok(quick_xml::events::Event::End(_)) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            }
            Ok(quick_xml::events::Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    if text.is_empty() { None } else { Some(text) }
}

fn parse_caldav_ical_event(ical_data: &str, href: &str) -> Result<CalendarEventDto, String> {
    let lines = unfold_ical_lines(ical_data);
    let mut uid = None;
    let mut summary = None;
    let mut description = None;
    let mut location = None;
    let mut dtstart: Option<String> = None;
    let mut dtstart_tzid: Option<String> = None;
    let mut dtend: Option<String> = None;
    let mut dtend_tzid: Option<String> = None;
    let mut status = "confirmed".to_string();
    let mut organizer_email = None;
    let mut is_all_day = false;
    let mut attendees = Vec::<serde_json::Value>::new();

    for line in lines {
        let mut parts = line.splitn(2, ':');
        let Some(name_with_params) = parts.next() else {
            continue;
        };
        let value = parts.next().unwrap_or_default();
        let mut name_parts = name_with_params.split(';');
        let prop_name = name_parts.next().unwrap_or_default().to_uppercase();
        let params = name_parts.collect::<Vec<_>>().join(";").to_uppercase();

        match prop_name.as_str() {
            "UID" => uid = Some(value.to_string()),
            "SUMMARY" => summary = Some(unescape_ical_text(value)),
            "DESCRIPTION" => description = Some(unescape_ical_text(value)),
            "LOCATION" => location = Some(unescape_ical_text(value)),
            "DTSTART" => {
                dtstart = Some(value.to_string());
                dtstart_tzid = extract_param_value(name_with_params, "TZID");
                if params.contains("VALUE=DATE") && !params.contains("VALUE=DATE-TIME") {
                    is_all_day = true;
                }
            }
            "DTEND" => {
                dtend = Some(value.to_string());
                dtend_tzid = extract_param_value(name_with_params, "TZID");
            }
            "STATUS" => status = value.to_lowercase(),
            "ORGANIZER" => {
                if let Some(email) = value
                    .strip_prefix("mailto:")
                    .or_else(|| value.strip_prefix("MAILTO:"))
                {
                    organizer_email = Some(email.to_string());
                }
            }
            "ATTENDEE" => {
                if let Some(email) = value
                    .strip_prefix("mailto:")
                    .or_else(|| value.strip_prefix("MAILTO:"))
                {
                    let display_name = extract_param_value(name_with_params, "CN");
                    let response_status = extract_param_value(name_with_params, "PARTSTAT")
                        .map(|value| value.to_lowercase());
                    attendees.push(serde_json::json!({
                        "email": email,
                        "displayName": display_name,
                        "responseStatus": response_status,
                    }));
                }
            }
            _ => {}
        }
    }

    let start_time = dtstart
        .as_deref()
        .map(|value| parse_ical_datetime(value, is_all_day, dtstart_tzid.as_deref()))
        .transpose()?
        .unwrap_or(0);
    let end_time = dtend
        .as_deref()
        .map(|value| parse_ical_datetime(value, is_all_day, dtend_tzid.as_deref()))
        .transpose()?
        .unwrap_or(start_time + 3600);

    Ok(CalendarEventDto {
        remote_event_id: href.to_string(),
        uid,
        etag: None,
        summary,
        description,
        location,
        start_time,
        end_time,
        is_all_day,
        status,
        organizer_email,
        attendees_json: if attendees.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(&attendees)
                    .map_err(|e| format!("serialize CalDAV attendees: {e}"))?,
            )
        },
        html_link: None,
        ical_data: Some(ical_data.to_string()),
    })
}

fn parse_caldav_event_input(
    value: serde_json::Value,
) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    value
        .as_object()
        .cloned()
        .ok_or_else(|| "invalid CalDAV event payload".to_string())
}

fn merge_caldav_event_input(
    existing: &CalendarEventDto,
    updates: &serde_json::Map<String, serde_json::Value>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut merged = serde_json::Map::new();
    merged.insert(
        "summary".to_string(),
        serde_json::Value::String(
            updates
                .get("summary")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string)
                .unwrap_or_else(|| existing.summary.clone().unwrap_or_default()),
        ),
    );
    if let Some(description) = updates
        .get("description")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
        .or_else(|| existing.description.clone())
    {
        merged.insert(
            "description".to_string(),
            serde_json::Value::String(description),
        );
    }
    if let Some(location) = updates
        .get("location")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
        .or_else(|| existing.location.clone())
    {
        merged.insert("location".to_string(), serde_json::Value::String(location));
    }
    merged.insert(
        "startTime".to_string(),
        serde_json::Value::String(
            updates
                .get("startTime")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string)
                .unwrap_or_else(|| {
                    chrono::DateTime::<chrono::Utc>::from_timestamp(existing.start_time, 0)
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_default()
                }),
        ),
    );
    merged.insert(
        "endTime".to_string(),
        serde_json::Value::String(
            updates
                .get("endTime")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string)
                .unwrap_or_else(|| {
                    chrono::DateTime::<chrono::Utc>::from_timestamp(existing.end_time, 0)
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_default()
                }),
        ),
    );
    merged.insert(
        "isAllDay".to_string(),
        serde_json::Value::Bool(
            updates
                .get("isAllDay")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(existing.is_all_day),
        ),
    );
    merged
}

fn build_caldav_ical_event(
    input: &serde_json::Map<String, serde_json::Value>,
    uid: Option<&str>,
) -> String {
    let event_uid = uid
        .map(ToString::to_string)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let now = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//Ratatoskr//CalDAV Client//EN".to_string(),
        "BEGIN:VEVENT".to_string(),
        format!("UID:{event_uid}"),
        format!("DTSTAMP:{now}"),
    ];

    if let Some(summary) = input.get("summary").and_then(serde_json::Value::as_str) {
        lines.push(format!("SUMMARY:{}", escape_ical_text(summary)));
    }

    let start_time = input.get("startTime").and_then(serde_json::Value::as_str);
    let end_time = input.get("endTime").and_then(serde_json::Value::as_str);
    let is_all_day = input
        .get("isAllDay")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if let (Some(start), Some(end)) = (start_time, end_time) {
        if is_all_day {
            lines.push(format!("DTSTART;VALUE=DATE:{}", format_ical_date(start)));
            lines.push(format!("DTEND;VALUE=DATE:{}", format_ical_date(end)));
        } else {
            lines.push(format!("DTSTART:{}", format_ical_datetime(start)));
            lines.push(format!("DTEND:{}", format_ical_datetime(end)));
        }
    }

    if let Some(description) = input.get("description").and_then(serde_json::Value::as_str) {
        lines.push(format!("DESCRIPTION:{}", escape_ical_text(description)));
    }
    if let Some(location) = input.get("location").and_then(serde_json::Value::as_str) {
        lines.push(format!("LOCATION:{}", escape_ical_text(location)));
    }
    if let Some(attendees) = input.get("attendees").and_then(serde_json::Value::as_array) {
        for attendee in attendees {
            if let Some(email) = attendee.get("email").and_then(serde_json::Value::as_str) {
                lines.push(format!("ATTENDEE;RSVP=TRUE:mailto:{email}"));
            }
        }
    }

    lines.push("END:VEVENT".to_string());
    lines.push("END:VCALENDAR".to_string());
    lines.join("\r\n")
}

fn escape_ical_text(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
}

fn format_ical_datetime(value: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|date| {
            date.with_timezone(&chrono::Utc)
                .format("%Y%m%dT%H%M%SZ")
                .to_string()
        })
        .unwrap_or_else(|_| value.to_string())
}

fn format_ical_date(value: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|date| date.format("%Y%m%d").to_string())
        .or_else(|_| {
            chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .map(|date| date.format("%Y%m%d").to_string())
        })
        .unwrap_or_else(|_| value.replace('-', ""))
}

async fn fetch_caldav_event_by_href(
    client: &reqwest::Client,
    config: &CaldavAccountConfig,
    remote_event_id: &str,
) -> Result<CalendarEventDto, String> {
    let response =
        caldav_request_with_headers(client, config, "GET", remote_event_id, None, None, &[])
            .await?;
    let etag = response
        .headers()
        .get("ETag")
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let ical_data = response
        .text()
        .await
        .map_err(|e| format!("read CalDAV event: {e}"))?;
    let mut event = parse_caldav_ical_event(&ical_data, remote_event_id)?;
    event.etag = etag;
    Ok(event)
}

fn join_url_path(base: &str, segment: &str) -> Result<String, String> {
    let base = if base.ends_with('/') {
        base.to_string()
    } else {
        format!("{base}/")
    };
    resolve_href(&base, segment)
}

fn unfold_ical_lines(ical_data: &str) -> Vec<String> {
    ical_data
        .replace("\r\n ", "")
        .replace("\r\n\t", "")
        .replace("\n ", "")
        .replace("\n\t", "")
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .split('\n')
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn unescape_ical_text(value: &str) -> String {
    value
        .replace("\\n", "\n")
        .replace("\\N", "\n")
        .replace("\\,", ",")
        .replace("\\;", ";")
        .replace("\\\\", "\\")
}

fn extract_param_value(name_with_params: &str, key: &str) -> Option<String> {
    for param in name_with_params.split(';').skip(1) {
        let mut parts = param.splitn(2, '=');
        let param_name = parts.next()?.trim();
        let param_value = parts.next()?.trim();
        if param_name.eq_ignore_ascii_case(key) {
            return Some(param_value.trim_matches('"').to_string());
        }
    }
    None
}

fn parse_ical_datetime(value: &str, is_all_day: bool, _tzid: Option<&str>) -> Result<i64, String> {
    if is_all_day {
        let date = chrono::NaiveDate::parse_from_str(value, "%Y%m%d")
            .map_err(|e| format!("invalid all-day CalDAV date {value}: {e}"))?;
        return date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| "invalid all-day CalDAV time".to_string())
            .map(|date_time| date_time.and_utc().timestamp());
    }

    if let Some(cleaned) = value.strip_suffix('Z') {
        return chrono::NaiveDateTime::parse_from_str(cleaned, "%Y%m%dT%H%M%S")
            .map_err(|e| format!("invalid UTC CalDAV datetime {value}: {e}"))
            .map(|date_time| date_time.and_utc().timestamp());
    }

    chrono::NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S")
        .map_err(|e| format!("invalid CalDAV datetime {value}: {e}"))
        .map(|dt| {
            dt.and_local_timezone(chrono::Local)
                .single()
                .map(|dt| dt.timestamp())
                .unwrap_or_else(|| dt.and_utc().timestamp())
        })
}

#[cfg(test)]
mod tests {
    use super::extract_tag_value;
    use quick_xml::escape::unescape;

    #[test]
    fn extract_tag_value_flattens_nested_text() {
        let xml = r#"<d:displayname xmlns:d="DAV:">Work <b>Calendar</b></d:displayname>"#;
        assert_eq!(
            extract_tag_value(xml, "displayname").as_deref(),
            Some("Work Calendar")
        );
    }

    #[test]
    fn html_unescape_handles_named_and_numeric_entities() {
        assert_eq!(
            unescape("Tom &amp; &quot;Jerry&quot; &#39;ok&#39; &#x2603;")
                .map(std::borrow::Cow::into_owned)
                .unwrap(),
            "Tom & \"Jerry\" 'ok' \u{2603}"
        );
    }
}
