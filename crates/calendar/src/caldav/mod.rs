mod ical;
mod xml;

use rusqlite::OptionalExtension;

use rtsk::db::DbState;

use super::types::{CalendarEventDto, CalendarInfoDto, CalendarSyncResultDto};
use super::{CALDAV_NS, shared_http_client};

pub struct CaldavAccountConfig {
    server_url: String,
    username: String,
    password: String,
    principal_url: Option<String>,
    home_url: Option<String>,
}

pub async fn caldav_list_calendars_impl(
    account_id: &str,
    db: &DbState,
    encryption_key: &[u8; 32],
) -> Result<Vec<CalendarInfoDto>, String> {
    let config = load_caldav_account_config(db, encryption_key, account_id).await?;
    let client = shared_http_client();
    let home_url = resolve_caldav_home_url(client, &config).await?;
    list_caldav_calendars(client, &config, &home_url).await
}

pub async fn caldav_fetch_events_impl(
    db: &DbState,
    encryption_key: &[u8; 32],
    account_id: &str,
    calendar_remote_id: &str,
    time_min: &str,
    time_max: &str,
) -> Result<Vec<CalendarEventDto>, String> {
    let config = load_caldav_account_config(db, encryption_key, account_id).await?;
    let client = shared_http_client();
    fetch_caldav_events(client, &config, calendar_remote_id, time_min, time_max).await
}

pub async fn caldav_sync_events_impl(
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

pub async fn caldav_test_connection_impl(
    db: &DbState,
    encryption_key: &[u8; 32],
    account_id: &str,
) -> Result<serde_json::Value, String> {
    let config = load_caldav_account_config(db, encryption_key, account_id).await?;
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

pub async fn caldav_create_event_impl(
    db: &DbState,
    encryption_key: &[u8; 32],
    account_id: &str,
    calendar_remote_id: &str,
    event: serde_json::Value,
) -> Result<CalendarEventDto, String> {
    let config = load_caldav_account_config(db, encryption_key, account_id).await?;
    let client = shared_http_client();
    let input = ical::parse_caldav_event_input(&event)?;
    let uid = uuid::Uuid::new_v4().to_string();
    let ical_data = ical::build_caldav_ical_event(&input, Some(&uid));
    let remote_event_id = xml::join_url_path(calendar_remote_id, &format!("{uid}.ics"))?;

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

#[allow(clippy::too_many_arguments)]
pub async fn caldav_update_event_impl(
    db: &DbState,
    encryption_key: &[u8; 32],
    account_id: &str,
    remote_event_id: &str,
    event: serde_json::Value,
    etag: Option<String>,
) -> Result<CalendarEventDto, String> {
    let config = load_caldav_account_config(db, encryption_key, account_id).await?;
    let client = shared_http_client();
    let input = ical::parse_caldav_event_input(&event)?;
    let existing = fetch_caldav_event_by_href(client, &config, remote_event_id).await?;
    let merged = ical::merge_caldav_event_input(&existing, &input);
    let ical_data = ical::build_caldav_ical_event(&merged, existing.uid.as_deref());

    let mut headers = vec![("Content-Type", "text/calendar; charset=utf-8")];
    if let Some(etag_value) = etag.as_deref() {
        headers.push(("If-Match", etag_value));
    }

    caldav_request_with_headers(
        client,
        &config,
        "PUT",
        remote_event_id,
        Some(&ical_data),
        None,
        &headers,
    )
    .await?;

    fetch_caldav_event_by_href(client, &config, remote_event_id).await
}

pub async fn caldav_delete_event_impl(
    db: &DbState,
    encryption_key: &[u8; 32],
    account_id: &str,
    remote_event_id: &str,
    etag: Option<String>,
) -> Result<(), String> {
    let config = load_caldav_account_config(db, encryption_key, account_id).await?;
    let client = shared_http_client();
    let mut headers = Vec::new();
    if let Some(etag_value) = etag.as_deref() {
        headers.push(("If-Match", etag_value));
    }
    caldav_request_with_headers(
        client,
        &config,
        "DELETE",
        remote_event_id,
        None,
        None,
        &headers,
    )
    .await?;
    Ok(())
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

// ---------------------------------------------------------------------------
// CalDAV discovery and listing
// ---------------------------------------------------------------------------

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
        let xml_text = response
            .text()
            .await
            .map_err(|e| format!("read principal response: {e}"))?;
        let href = xml::extract_first_href_for_property(&xml_text, &["current-user-principal"])
            .ok_or_else(|| {
                "CalDAV discovery failed: current-user-principal not found".to_string()
            })?;
        xml::resolve_href(&config.server_url, &href)?
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
    let xml_text = response
        .text()
        .await
        .map_err(|e| format!("read home response: {e}"))?;
    let href = xml::extract_first_href_for_property(&xml_text, &["calendar-home-set"])
        .ok_or_else(|| "CalDAV discovery failed: calendar-home-set not found".to_string())?;
    xml::resolve_href(&principal_url, &href)
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
    let xml_text = response
        .text()
        .await
        .map_err(|e| format!("read calendars response: {e}"))?;
    let responses = xml::split_xml_responses(&xml_text);
    let mut calendars = Vec::new();

    for response_xml in responses {
        if !xml::contains_any_tag(response_xml, &["calendar"]) {
            continue;
        }

        let Some(href) = xml::extract_first_tag_value(response_xml, &["href"]) else {
            continue;
        };
        let remote_id = xml::resolve_href(home_url, &href)?;
        if xml::normalize_url_for_compare(&remote_id) == xml::normalize_url_for_compare(home_url) {
            continue;
        }

        let display_name = xml::extract_first_tag_value(response_xml, &["displayname"])
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| format!("Calendar {}", calendars.len() + 1));
        let color = xml::extract_first_tag_value(response_xml, &["calendar-color"]);

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
    let xml_text = response
        .text()
        .await
        .map_err(|e| format!("read events response: {e}"))?;
    let responses = xml::split_xml_responses(&xml_text);
    let mut events = Vec::new();

    for response_xml in responses {
        let Some(calendar_data) = xml::extract_first_tag_value(response_xml, &["calendar-data"])
        else {
            continue;
        };
        let Some(href) = xml::extract_first_tag_value(response_xml, &["href"]) else {
            continue;
        };
        let remote_event_id = xml::resolve_href(calendar_remote_id, &href)?;
        let etag = xml::extract_first_tag_value(response_xml, &["getetag"]);
        let mut event = ical::parse_caldav_ical_event(&calendar_data, &remote_event_id)?;
        event.etag = etag;
        events.push(event);
    }

    Ok(events)
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

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
    let mut event = ical::parse_caldav_ical_event(&ical_data, remote_event_id)?;
    event.etag = etag;
    Ok(event)
}

#[cfg(test)]
mod tests {
    use super::xml::extract_tag_value;
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
