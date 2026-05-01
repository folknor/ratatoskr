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
    pub fn new(base_url: &str, username: &str, password: &str, auth_method: AuthMethod) -> Self {
        // Match reqwest's default redirect ceiling (10) rather than imposing a
        // tighter `limited(5)`. Some hosted Zimbra and SSO-fronted Exchange
        // installs chain six or seven hops through the IdP before landing on
        // the DAV root; capping at 5 caused first-time discovery to fail on
        // those deployments where the previous (default-config) client
        // succeeded.
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(10))
            .timeout(crate::constants::DAV_CLIENT_TIMEOUT)
            .build()
            .unwrap_or_else(|err| {
                // `unwrap_or_default` would swap in `reqwest::Client::default()`,
                // silently abandoning both the timeout and the redirect cap with
                // no diagnostic. Log so an operator running into this can see
                // the swap.
                log::warn!(
                    "CalDAV client builder failed ({err}); falling back to default reqwest client"
                );
                reqwest::Client::new()
            });

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

    /// Override the principal URL so `discover()` can skip the
    /// `current-user-principal` PROPFIND. Useful when the principal was
    /// resolved on a previous sync and persisted to DB.
    pub fn set_principal_url(&mut self, url: &str) {
        self.principal_url = Some(url.to_string());
    }

    /// Return the discovered (or manually set) calendar-home-set URL.
    pub fn calendar_home_url(&self) -> Option<&str> {
        self.calendar_home_url.as_deref()
    }

    /// Return the discovered (or manually set) principal URL. Useful for
    /// callers that want to persist it for the next sync to reuse.
    pub fn principal_url(&self) -> Option<&str> {
        self.principal_url.as_deref()
    }

    // -----------------------------------------------------------------------
    // Discovery
    // -----------------------------------------------------------------------

    /// Auto-discover the principal and calendar-home-set URLs.
    ///
    /// 1. PROPFIND `current-user-principal` against `base_url`.
    /// 2. If that fails (or doesn't return a principal), retry against
    ///    `{base_url}/.well-known/caldav` as a fallback hint.
    /// 3. PROPFIND the principal for `calendar-home-set`.
    ///
    /// The base-URL-first ordering matters because many enterprise Exchange
    /// front-ends respond `200 OK` with a normal HTML 404 page (or a redirect
    /// to a login portal) for `/.well-known/caldav`. A naive "well-known
    /// first" probe interprets that as a successful discovery with no
    /// principal, then falls into a confusing retry loop. Probing the base
    /// URL first makes the well-known probe a true fallback - we only hit it
    /// when base-URL PROPFIND outright fails.
    ///
    /// Both `principal_url` and `calendar_home_url` may be set ahead of time
    /// by `set_principal_url` / `set_calendar_home_url`; the corresponding
    /// PROPFINDs are skipped when those are already populated.
    pub async fn discover(&mut self) -> Result<(), String> {
        // Step 1: principal lookup. Skip if a caller already set it from a
        // persisted value.
        if self.principal_url.is_none() {
            self.principal_url = Some(self.discover_principal().await?);
        }

        // Step 2: home-set lookup. Skip if already set.
        if self.calendar_home_url.is_none() {
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
        }

        log::info!(
            "CalDAV discovery complete: principal={}, calendar-home={}",
            self.principal_url.as_deref().unwrap_or("?"),
            self.calendar_home_url.as_deref().unwrap_or("?")
        );

        Ok(())
    }

    /// Resolve `current-user-principal` by probing `base_url` first and
    /// `.well-known/caldav` only as a fallback. Returns the absolute principal
    /// URL on success.
    async fn discover_principal(&self) -> Result<String, String> {
        // Try the base URL directly. This works for the vast majority of
        // CalDAV endpoints (DAViCal, Radicale, SOGo, iCloud, Fastmail, ...).
        let mut last_error = match self
            .propfind_raw(&self.base_url, "0", PROPFIND_PRINCIPAL)
            .await
        {
            Ok((_, body)) => {
                if let Some(principal) = extract_href_property(&body, "current-user-principal") {
                    return Ok(self.resolve_url(&principal));
                }
                "PROPFIND on base URL returned no current-user-principal".to_string()
            }
            Err(e) => format!("PROPFIND on base URL failed: {e}"),
        };

        // Fallback: probe `.well-known/caldav`. RFC 6764 § 6 recommends this
        // as a discovery hint when the client doesn't know the DAV root. We
        // only reach it if the base URL didn't yield a principal.
        let well_known_url = format!("{}/.well-known/caldav", self.base_url);
        match self
            .propfind_raw(&well_known_url, "0", PROPFIND_PRINCIPAL)
            .await
        {
            Ok((_, body)) => {
                if let Some(principal) = extract_href_property(&body, "current-user-principal") {
                    return Ok(self.resolve_url(&principal));
                }
                last_error = format!(
                    "{last_error}; .well-known/caldav also returned no current-user-principal"
                );
            }
            Err(e) => {
                last_error = format!("{last_error}; .well-known/caldav also failed: {e}");
            }
        }

        Err(format!(
            "Could not discover current-user-principal: {last_error}"
        ))
    }

    // -----------------------------------------------------------------------
    // Calendar listing
    // -----------------------------------------------------------------------

    /// List all calendars in the calendar-home-set.
    ///
    /// Returns discovered calendars with their href, display name, color, and ctag.
    ///
    /// Calendar hrefs are resolved to absolute URLs against the calendar-home
    /// URL before returning. Many servers emit path-only hrefs in PROPFIND
    /// responses (RFC 5785 § 3); callers store the value as `remote_id` and
    /// later parse it as a URL for create/update, so resolving here keeps
    /// every downstream path simple.
    pub async fn list_calendars(&self) -> Result<Vec<DiscoveredCalendar>, String> {
        let url = self.require_calendar_home_url()?;
        let (_, body) = self
            .propfind_raw(&url, "1", PROPFIND_CALENDARS)
            .await
            .map_err(|e| format!("PROPFIND calendars failed: {e}"))?;

        let mut calendars = parse::parse_propfind_calendars(&body);
        for cal in &mut calendars {
            cal.href = self.resolve_url(&cal.href);
        }
        Ok(calendars)
    }

    // -----------------------------------------------------------------------
    // Event listing
    // -----------------------------------------------------------------------

    /// List all events in a calendar (URIs + ETags).
    ///
    /// Event URIs are resolved to absolute URLs against the calendar URL for
    /// the same reason `list_calendars` does so for calendar hrefs.
    pub async fn list_events(&self, calendar_url: &str) -> Result<Vec<CalDavEventEntry>, String> {
        let url = self.resolve_url(calendar_url);
        let (_, body) = self
            .propfind_raw(&url, "1", PROPFIND_EVENTS)
            .await
            .map_err(|e| format!("PROPFIND events failed: {e}"))?;

        let mut entries = parse::parse_propfind_events(&body);
        for entry in &mut entries {
            entry.uri = self.resolve_url_against(&url, &entry.uri);
        }
        Ok(entries)
    }

    // -----------------------------------------------------------------------
    // iCalendar fetching
    // -----------------------------------------------------------------------

    /// Batch-fetch iCalendar data by URI using calendar-multiget REPORT.
    ///
    /// URIs are batched in groups of 50 to avoid overwhelming the server.
    /// Returns `Vec<(uri, ical_data)>`.
    pub async fn fetch_events(
        &self,
        calendar_url: &str,
        uris: &[&str],
    ) -> Result<Vec<(String, String)>, String> {
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

    /// Fetch a single event's iCalendar payload via GET.
    ///
    /// Returns `(ical_text, etag)`. Useful for re-reading server canonicalization
    /// after a PUT (e.g. for ETag refresh).
    ///
    /// Plain GET rather than a calendar-multiget REPORT: a few servers
    /// (Yahoo, certain Kerio installs) require a REPORT to return the
    /// canonicalized iCal post-PUT, but this matches the deleted ad-hoc
    /// path's behavior and the create/update flow now prefers the PUT-side
    /// ETag (`finalize_event` in `crates/calendar/src/caldav/mod.rs`) so the
    /// GET is only a best-effort canonicalization fetch.
    pub async fn get_event_ical(
        &self,
        event_url: &str,
    ) -> Result<(String, Option<String>), String> {
        let url = self.resolve_url(event_url);

        let resp = self
            .http
            .get(&url)
            .header(reqwest::header::ACCEPT, "text/calendar, application/calendar+xml, */*")
            .headers(self.auth_headers())
            .send()
            .await
            .map_err(|e| format!("GET {url}: {e}"))?;

        let status = resp.status();
        // Store the ETag verbatim. See `parse.rs::parse_propfind_events` for
        // the rationale - the quotes (and the optional `W/` weak indicator)
        // are part of the value per RFC 7232, not framing we can strip.
        let etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);

        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("GET {url} returned {status}: {text}"));
        }

        let body = resp.text().await.map_err(|e| format!("read body: {e}"))?;
        Ok((body, etag))
    }

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

        if let Some(etag_val) = etag
            && let Ok(val) = normalize_if_match_etag(etag_val).parse::<reqwest::header::HeaderValue>()
        {
            req = req.header(IF_MATCH, val);
        }

        let resp = req.send().await.map_err(|e| format!("PUT {url}: {e}"))?;
        let status = resp.status();

        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("PUT {url} returned {status}: {text}"));
        }

        // Extract ETag from response headers verbatim (see PROPFIND parsing
        // for the rationale).
        let new_etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);

        Ok(new_etag)
    }

    /// Delete an event via DELETE.
    ///
    /// If `etag` is provided, sends an `If-Match` header for conflict detection.
    pub async fn delete_event(&self, event_url: &str, etag: Option<&str>) -> Result<(), String> {
        let url = self.resolve_url(event_url);

        let mut req = self.http.delete(&url).headers(self.auth_headers());

        if let Some(etag_val) = etag
            && let Ok(val) = normalize_if_match_etag(etag_val).parse::<reqwest::header::HeaderValue>()
        {
            req = req.header(IF_MATCH, val);
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
            .request(
                Method::from_bytes(b"PROPFIND").map_err(|e| format!("method: {e}"))?,
                url,
            )
            .header(CONTENT_TYPE, "application/xml; charset=utf-8")
            .header("Depth", depth)
            .headers(self.auth_headers())
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| format!("PROPFIND {url}: {e}"))?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| format!("read body: {e}"))?;

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
            .request(
                Method::from_bytes(b"REPORT").map_err(|e| format!("method: {e}"))?,
                url,
            )
            .header(CONTENT_TYPE, "application/xml; charset=utf-8")
            .header("Depth", "1")
            .headers(self.auth_headers())
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| format!("REPORT {url}: {e}"))?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| format!("read body: {e}"))?;

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
                let credentials = base64::Engine::encode(
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
        self.resolve_url_against(&self.base_url, href)
    }

    /// Resolve a possibly-relative URL against an arbitrary base. Used for
    /// resolving event URIs against their containing calendar URL, which may
    /// live at a different path than `self.base_url`.
    fn resolve_url_against(&self, base: &str, href: &str) -> String {
        if href.starts_with("http://") || href.starts_with("https://") {
            return href.to_string();
        }
        if let Ok(base_url) = url::Url::parse(base)
            && let Ok(resolved) = base_url.join(href)
        {
            return resolved.to_string();
        }
        format!("{base}{href}")
    }

    /// Get the calendar-home-set URL or return an error.
    fn require_calendar_home_url(&self) -> Result<String, String> {
        self.calendar_home_url.clone().ok_or_else(|| {
            "No calendar-home-set URL - call discover() or set_calendar_home_url() first"
                .to_string()
        })
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
    use quick_xml::escape::unescape;
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
                if let Ok(raw) = std::str::from_utf8(e.as_ref())
                    && let Ok(text) = unescape(raw)
                {
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

/// Format a stored ETag value for the `If-Match` header.
///
/// New code stores ETags verbatim including quotes (`"abc"`, `W/"abc"`), so the
/// fast path is to send the value through unchanged. We tolerate two legacy
/// shapes for backwards compatibility with rows written before this fix:
///
/// - **Bare strong ETag** (`abc`): stored without RFC 7232 quotes by the old
///   `trim_matches('"')` path. Wrap into `"abc"` so the server accepts it.
/// - **Corrupted weak ETag** (`W/abc`): the old code stripped the inner quote;
///   we cannot reliably reconstruct the original, but wrapping the whole token
///   produces `"W/abc"` which the server will reject - triggering a 412
///   response, an automatic re-fetch, and recovery to a clean verbatim ETag on
///   next sync. Better than silently sending malformed headers.
fn normalize_if_match_etag(stored: &str) -> String {
    let s = stored.trim();
    if s.starts_with('"') || s.starts_with("W/\"") {
        s.to_string()
    } else {
        format!("\"{s}\"")
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_if_match_etag;

    #[test]
    fn preserves_already_quoted_strong_etag() {
        assert_eq!(normalize_if_match_etag("\"abc\""), "\"abc\"");
    }

    #[test]
    fn preserves_already_quoted_weak_etag() {
        assert_eq!(normalize_if_match_etag("W/\"abc\""), "W/\"abc\"");
    }

    #[test]
    fn wraps_legacy_bare_etag() {
        // Pre-fix rows have unquoted strong ETags. Wrap them so they conform
        // to RFC 7232 before going on the wire.
        assert_eq!(normalize_if_match_etag("abc"), "\"abc\"");
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(normalize_if_match_etag("  \"abc\"  "), "\"abc\"");
    }
}
