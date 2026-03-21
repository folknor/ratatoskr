use reqwest::header::{CONTENT_TYPE, IF_MATCH};
use reqwest::{Method, StatusCode};

use super::parse::{self, CalDavEventEntry};

/// Authentication method for the CalDAV server.
#[derive(Debug, Clone)]
pub enum AuthMethod {
    /// HTTP Basic authentication (username + password).
    Basic,
    /// OAuth2 Bearer token (the password field contains the access token).
    OAuth2,
}

/// A minimal CalDAV client using raw reqwest + quick-xml.
///
/// Supports the subset of CalDAV needed for calendar sync:
/// `PROPFIND` for calendar discovery, `REPORT` (calendar-multiget / calendar-query)
/// for fetching events, and `PUT`/`DELETE` for creating/updating/removing events.
#[derive(Debug, Clone)]
pub struct CalDavClient {
    http: reqwest::Client,
    base_url: String,
    username: String,
    password: String,
    auth_method: AuthMethod,
    /// Discovered principal URL.
    principal_url: Option<String>,
    /// Discovered calendar-home-set URL.
    calendar_home_url: Option<String>,
}

/// Discovered calendar collection from PROPFIND.
#[derive(Debug, Clone)]
pub struct DiscoveredCalendar {
    pub href: String,
    pub display_name: Option<String>,
    pub color: Option<String>,
    pub ctag: Option<String>,
}

/// Batch size for calendar-multiget REPORT requests.
const MULTIGET_BATCH_SIZE: usize = 50;

impl CalDavClient {
    /// Create a new `CalDavClient` with the given credentials.
    ///
    /// Call [`discover`] after construction to auto-detect the principal and
    /// calendar-home-set URLs.
    pub fn new(
        base_url: &str,
        username: &str,
        password: &str,
        auth_method: AuthMethod,
    ) -> Self {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(5))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            username: username.to_string(),
            password: password.to_string(),
            auth_method,
            principal_url: None,
            calendar_home_url: None,
        }
    }

    /// Override the calendar-home-set URL (skip discovery).
    pub fn set_calendar_home_url(&mut self, url: &str) {
        self.calendar_home_url = Some(url.to_string());
    }

    /// Return the discovered (or manually set) calendar-home-set URL.
    pub fn calendar_home_url(&self) -> Option<&str> {
        self.calendar_home_url.as_deref()
    }

    // -----------------------------------------------------------------------
    // Discovery
    // -----------------------------------------------------------------------

    /// Auto-discover the principal and calendar-home-set URLs.
    ///
    /// 1. `GET {base_url}/.well-known/caldav` (follow redirects)
    /// 2. `PROPFIND` for `current-user-principal`
    /// 3. `PROPFIND` on the principal for `calendar-home-set`
    /// 4. Store the discovered URLs
    pub async fn discover(&mut self) -> Result<(), String> {
        // Step 1: Try .well-known/caldav to find the DAV root
        let well_known_url = format!("{}/.well-known/caldav", self.base_url);
        let dav_root = match self.propfind_raw(&well_known_url, "0", PROPFIND_PRINCIPAL).await {
            Ok((_, body)) => {
                if let Some(principal) = extract_href_property(&body, "current-user-principal") {
                    self.principal_url = Some(self.resolve_url(&principal));
                    self.resolve_url(&principal)
                } else {
                    well_known_url.clone()
                }
            }
            Err(_) => {
                // .well-known not available, try the base URL directly
                self.base_url.clone()
            }
        };

        // Step 2: If we don't have a principal yet, PROPFIND on the DAV root
        if self.principal_url.is_none() {
            let (_, body) = self
                .propfind_raw(&dav_root, "0", PROPFIND_PRINCIPAL)
                .await
                .map_err(|e| format!("PROPFIND for principal failed: {e}"))?;

            if let Some(principal) = extract_href_property(&body, "current-user-principal") {
                self.principal_url = Some(self.resolve_url(&principal));
            } else {
                return Err("Could not discover current-user-principal".to_string());
            }
        }

        // Step 3: PROPFIND on the principal for calendar-home-set
        let principal = self
            .principal_url
            .as_ref()
            .ok_or("No principal URL")?
            .clone();

        let (_, body) = self
            .propfind_raw(&principal, "0", PROPFIND_CALENDAR_HOME)
            .await
            .map_err(|e| format!("PROPFIND for calendar-home-set failed: {e}"))?;

        if let Some(home) = extract_href_property(&body, "calendar-home-set") {
            self.calendar_home_url = Some(self.resolve_url(&home));
        } else {
            return Err("Could not discover calendar-home-set".to_string());
        }

        log::info!(
            "CalDAV discovery complete: calendar-home={}",
            self.calendar_home_url.as_deref().unwrap_or("?")
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Calendar listing
    // -----------------------------------------------------------------------

    /// List all calendars in the calendar-home-set.
    ///
    /// Returns discovered calendars with their href, display name, color, and ctag.
    pub async fn list_calendars(&self) -> Result<Vec<DiscoveredCalendar>, String> {
        let url = self.require_calendar_home_url()?;
        let (_, body) = self
            .propfind_raw(&url, "1", PROPFIND_CALENDARS)
            .await
            .map_err(|e| format!("PROPFIND calendars failed: {e}"))?;

        Ok(parse::parse_propfind_calendars(&body))
    }

    // -----------------------------------------------------------------------
    // Event listing
    // -----------------------------------------------------------------------

    /// List all events in a calendar (URIs + ETags).
    pub async fn list_events(&self, calendar_url: &str) -> Result<Vec<CalDavEventEntry>, String> {
        let url = self.resolve_url(calendar_url);
        let (_, body) = self
            .propfind_raw(&url, "1", PROPFIND_EVENTS)
            .await
            .map_err(|e| format!("PROPFIND events failed: {e}"))?;

        Ok(parse::parse_propfind_events(&body))
    }

    // -----------------------------------------------------------------------
    // iCalendar fetching
    // -----------------------------------------------------------------------

    /// Batch-fetch iCalendar data by URI using calendar-multiget REPORT.
    ///
    /// URIs are batched in groups of 50 to avoid overwhelming the server.
    /// Returns `Vec<(uri, ical_data)>`.
    pub async fn fetch_events(&self, calendar_url: &str, uris: &[&str]) -> Result<Vec<(String, String)>, String> {
        if uris.is_empty() {
            return Ok(Vec::new());
        }

        let resolved_calendar_url = self.resolve_url(calendar_url);
        let mut all_results = Vec::new();

        for chunk in uris.chunks(MULTIGET_BATCH_SIZE) {
            let mut href_elements = String::new();
            for uri in chunk {
                href_elements.push_str(&format!("  <D:href>{uri}</D:href>\n"));
            }

            let body = format!(
                r#"<?xml version="1.0" encoding="utf-8"?>
<C:calendar-multiget xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:prop>
    <D:getetag/>
    <C:calendar-data/>
  </D:prop>
{href_elements}</C:calendar-multiget>"#
            );

            let (_, response_body) = self
                .report_raw(&resolved_calendar_url, &body)
                .await
                .map_err(|e| format!("REPORT multiget failed: {e}"))?;

            let parsed = parse::parse_multiget_report(&response_body);
            all_results.extend(parsed);
        }

        Ok(all_results)
    }

    /// Fetch all events in a calendar within a time range using calendar-query REPORT.
    ///
    /// `time_start` and `time_end` are RFC 5545 UTC date-time strings (e.g. "20240101T000000Z").
    pub async fn fetch_events_in_range(
        &self,
        calendar_url: &str,
        time_start: &str,
        time_end: &str,
    ) -> Result<Vec<(String, String)>, String> {
        let resolved_url = self.resolve_url(calendar_url);

        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<C:calendar-query xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:prop>
    <D:getetag/>
    <C:calendar-data/>
  </D:prop>
  <C:filter>
    <C:comp-filter name="VCALENDAR">
      <C:comp-filter name="VEVENT">
        <C:time-range start="{time_start}" end="{time_end}"/>
      </C:comp-filter>
    </C:comp-filter>
  </C:filter>
</C:calendar-query>"#
        );

        let (_, response_body) = self
            .report_raw(&resolved_url, &body)
            .await
            .map_err(|e| format!("REPORT calendar-query failed: {e}"))?;

        Ok(parse::parse_multiget_report(&response_body))
    }

    // -----------------------------------------------------------------------
    // CTag
    // -----------------------------------------------------------------------

    /// Get the collection CTag for a specific calendar URL.
    pub async fn get_ctag(&self, calendar_url: &str) -> Result<Option<String>, String> {
        let url = self.resolve_url(calendar_url);
        let (_, body) = self
            .propfind_raw(&url, "0", PROPFIND_CTAG)
            .await
            .map_err(|e| format!("PROPFIND ctag failed: {e}"))?;

        Ok(parse::parse_ctag(&body))
    }

    // -----------------------------------------------------------------------
    // Write operations (for future use)
    // -----------------------------------------------------------------------

    /// Create or update an event via PUT.
    ///
    /// If `etag` is provided, sends an `If-Match` header for conflict detection.
    /// Returns the new ETag from the response (if the server provides it).
    pub async fn put_event(
        &self,
        event_url: &str,
        ical_data: &str,
        etag: Option<&str>,
    ) -> Result<Option<String>, String> {
        let url = self.resolve_url(event_url);

        let mut req = self
            .http
            .put(&url)
            .header(CONTENT_TYPE, "text/calendar; charset=utf-8")
            .headers(self.auth_headers())
            .body(ical_data.to_string());

        if let Some(etag_val) = etag {
            if let Ok(val) = format!("\"{etag_val}\"").parse::<reqwest::header::HeaderValue>() {
                req = req.header(IF_MATCH, val);
            }
        }

        let resp = req.send().await.map_err(|e| format!("PUT {url}: {e}"))?;
        let status = resp.status();

        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("PUT {url} returned {status}: {text}"));
        }

        // Extract ETag from response headers
        let new_etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim_matches('"').to_string());

        Ok(new_etag)
    }

    /// Delete an event via DELETE.
    ///
    /// If `etag` is provided, sends an `If-Match` header for conflict detection.
    pub async fn delete_event(
        &self,
        event_url: &str,
        etag: Option<&str>,
    ) -> Result<(), String> {
        let url = self.resolve_url(event_url);

        let mut req = self
            .http
            .delete(&url)
            .headers(self.auth_headers());

        if let Some(etag_val) = etag {
            if let Ok(val) = format!("\"{etag_val}\"").parse::<reqwest::header::HeaderValue>() {
                req = req.header(IF_MATCH, val);
            }
        }

        let resp = req.send().await.map_err(|e| format!("DELETE {url}: {e}"))?;
        let status = resp.status();

        if !status.is_success() && status != StatusCode::NOT_FOUND {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("DELETE {url} returned {status}: {text}"));
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // HTTP helpers
    // -----------------------------------------------------------------------

    /// Send a PROPFIND request and return `(status, body)`.
    async fn propfind_raw(
        &self,
        url: &str,
        depth: &str,
        body: &str,
    ) -> Result<(StatusCode, String), String> {
        let resp = self
            .http
            .request(Method::from_bytes(b"PROPFIND").map_err(|e| format!("method: {e}"))?, url)
            .header(CONTENT_TYPE, "application/xml; charset=utf-8")
            .header("Depth", depth)
            .headers(self.auth_headers())
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| format!("PROPFIND {url}: {e}"))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("read body: {e}"))?;

        if status.is_success() || status == StatusCode::MULTI_STATUS {
            Ok((status, text))
        } else {
            Err(format!("PROPFIND {url} returned {status}: {text}"))
        }
    }

    /// Send a REPORT request and return `(status, body)`.
    async fn report_raw(&self, url: &str, body: &str) -> Result<(StatusCode, String), String> {
        let resp = self
            .http
            .request(Method::from_bytes(b"REPORT").map_err(|e| format!("method: {e}"))?, url)
            .header(CONTENT_TYPE, "application/xml; charset=utf-8")
            .header("Depth", "1")
            .headers(self.auth_headers())
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| format!("REPORT {url}: {e}"))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("read body: {e}"))?;

        if status.is_success() || status == StatusCode::MULTI_STATUS {
            Ok((status, text))
        } else {
            Err(format!("REPORT {url} returned {status}: {text}"))
        }
    }

    /// Build authentication headers based on the auth method.
    fn auth_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        match self.auth_method {
            AuthMethod::Basic => {
                let credentials =
                    base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        format!("{}:{}", self.username, self.password),
                    );
                if let Ok(val) = format!("Basic {credentials}").parse() {
                    headers.insert(reqwest::header::AUTHORIZATION, val);
                }
            }
            AuthMethod::OAuth2 => {
                if let Ok(val) = format!("Bearer {}", self.password).parse() {
                    headers.insert(reqwest::header::AUTHORIZATION, val);
                }
            }
        }
        headers
    }

    /// Resolve a possibly-relative URL against the base URL.
    fn resolve_url(&self, href: &str) -> String {
        if href.starts_with("http://") || href.starts_with("https://") {
            return href.to_string();
        }
        if let Ok(base) = url::Url::parse(&self.base_url)
            && let Ok(resolved) = base.join(href)
        {
            return resolved.to_string();
        }
        format!("{}{href}", self.base_url)
    }

    /// Get the calendar-home-set URL or return an error.
    fn require_calendar_home_url(&self) -> Result<String, String> {
        self.calendar_home_url
            .clone()
            .ok_or_else(|| "No calendar-home-set URL — call discover() or set_calendar_home_url() first".to_string())
    }
}

// ---------------------------------------------------------------------------
// XML request bodies
// ---------------------------------------------------------------------------

const PROPFIND_PRINCIPAL: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:">
  <D:prop>
    <D:current-user-principal/>
  </D:prop>
</D:propfind>"#;

const PROPFIND_CALENDAR_HOME: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:prop>
    <C:calendar-home-set/>
  </D:prop>
</D:propfind>"#;

const PROPFIND_CALENDARS: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"
            xmlns:CS="http://calendarserver.org/ns/"
            xmlns:IC="http://apple.com/ns/ical/">
  <D:prop>
    <D:resourcetype/>
    <D:displayname/>
    <CS:getctag/>
    <IC:calendar-color/>
  </D:prop>
</D:propfind>"#;

const PROPFIND_EVENTS: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:">
  <D:prop>
    <D:getetag/>
    <D:getcontenttype/>
  </D:prop>
</D:propfind>"#;

const PROPFIND_CTAG: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:" xmlns:CS="http://calendarserver.org/ns/">
  <D:prop>
    <CS:getctag/>
  </D:prop>
</D:propfind>"#;

// ---------------------------------------------------------------------------
// XML response extraction helpers
// ---------------------------------------------------------------------------

/// Extract an `<D:href>` value nested inside a named property element.
fn extract_href_property(xml: &str, property_name: &str) -> Option<String> {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let mut reader = Reader::from_str(xml);
    let mut in_property = false;
    let mut current_tag = String::new();
    let mut buf = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = local_name(e.name().as_ref());
                if name == property_name {
                    in_property = true;
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
                if in_property && current_tag == "href" {
                    let val = buf.trim().to_string();
                    if !val.is_empty() {
                        return Some(val);
                    }
                }
                if name == property_name {
                    in_property = false;
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

/// Extract the local name from a possibly-namespaced XML tag.
fn local_name(raw: &[u8]) -> String {
    let full = String::from_utf8_lossy(raw);
    match full.rfind(':') {
        Some(idx) => full[idx + 1..].to_string(),
        None => full.to_string(),
    }
}
