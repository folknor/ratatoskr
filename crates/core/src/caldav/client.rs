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
        //
        // Custom policy (vs the simpler `Policy::limited(10)`) for one
        // additional invariant: refuse `https -> http` scheme downgrades.
        // reqwest's default `remove_sensitive_headers` strips `Authorization`
        // on host or *effective port* mismatch, but not on a same-port
        // scheme change. A 301 from `https://host:8443/cal` to
        // `http://host:8443/cal` (real failure mode for hosted Zimbra and
        // some Exchange front-ends on non-default ports) would otherwise
        // leak the Basic / Bearer credential in plaintext on the redirected
        // request. Cross-origin same-scheme redirects stay allowed because
        // legitimate hosted setups (Fastmail with caldav.fastmail.com,
        // hosted Exchange + CalDAV bridges) rely on them; reqwest still
        // strips Authorization on those hops.
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 10 {
                    return attempt.error("CalDAV redirect chain exceeded 10 hops");
                }
                if let Some(prev) = attempt.previous().last()
                    && prev.scheme() == "https"
                    && attempt.url().scheme() == "http"
                {
                    return attempt.error(
                        "CalDAV refusing https -> http scheme downgrade redirect",
                    );
                }
                attempt.follow()
            }))
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

            let body_result = self
                .propfind_raw(&principal, "0", PROPFIND_CALENDAR_HOME)
                .await
                .map_err(|e| format!("PROPFIND for calendar-home-set failed: {e}"));

            match body_result {
                Ok((_, body)) => {
                    // Resolve the returned href against the principal URL,
                    // not `self.base_url`. Hosted setups (Fastmail with a
                    // separate caldav.fastmail.com, hosted Exchange + CalDAV
                    // bridges) put the principal and DAV root on different
                    // hosts; resolving against base_url quietly rebuilds the
                    // home-set URL on the wrong host and the next
                    // PROPFIND lands on a different account or 404s.
                    let homes = extract_hrefs_property(&body, "calendar-home-set");
                    if homes.len() > 1 {
                        // Delegation / shared-account setups (Apple Calendar
                        // Server, Kerio, Exchange-bridged servers) legitimately
                        // return multiple homes. We currently only consume one
                        // - additional homes will not surface their calendars.
                        // Logged so an operator chasing "delegated calendars
                        // missing" can see the multi-href is present.
                        log::warn!(
                            "CalDAV calendar-home-set returned {} hrefs (delegation / shared-accounts); using only the first",
                            homes.len()
                        );
                    }
                    if let Some(home) = homes.into_iter().next() {
                        self.calendar_home_url =
                            Some(self.resolve_url_against(&principal, &home));
                    } else {
                        // The principal returned a valid 207 but no
                        // home-set. Either the persisted principal is
                        // stale (the server moved it) or the server
                        // genuinely has no calendars provisioned for
                        // this user. Clear the in-memory principal so
                        // the next discover() call starts from scratch
                        // rather than re-attempting step 2 with the
                        // same dead value (the previous "stuck loop"
                        // pattern). Persisted state is the caller's
                        // responsibility - they see Err and can decide
                        // whether to clear it from the DB and retry.
                        self.principal_url = None;
                        return Err("Could not discover calendar-home-set".to_string());
                    }
                }
                Err(e) => {
                    self.principal_url = None;
                    return Err(e);
                }
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
                if let Some(principal) =
                    extract_hrefs_property(&body, "current-user-principal").into_iter().next()
                {
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
                if let Some(principal) =
                    extract_hrefs_property(&body, "current-user-principal").into_iter().next()
                {
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
        if calendars.is_empty() {
            // Distinguish "user has no calendars provisioned" from "server
            // returned a parseable but content-free multistatus" (which
            // can happen on first-login races, server-side errors mis-
            // reported as 207, or a parser limitation we haven't seen
            // yet). Log so an operator chasing "where did my calendars
            // go" has a starting point. The caller still gets Ok(empty)
            // - this is informational, not an error.
            log::warn!(
                "CalDAV list_calendars at {url} returned 0 calendars from a {} byte response",
                body.len()
            );
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
                // Older SOGo (and a handful of niche CalDAV servers) reject
                // multiget bodies that name absolute hrefs not sharing
                // scheme+host with the request URL. After a redirect from
                // http -> https or one canonical hostname to another, our
                // stored URIs and the live request URL drift apart even
                // though they target the same resource. Re-relativize
                // absolute hrefs that share an origin with the request, so
                // strict servers see the path-only form they expect.
                let body_href = relativize_for_multiget(&resolved_calendar_url, uri);
                href_elements.push_str(&format!(
                    "  <D:href>{}</D:href>\n",
                    xml_escape_text(&body_href)
                ));
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

            // Normalize response hrefs the same way `list_events` does its
            // listing-side hrefs. Many servers echo relative hrefs in
            // multiget responses even when the listing side returned
            // absolute, and others vary their absolute forms (default port
            // stripping, trailing slash). Without this normalization, the
            // (uri, etag) lookup in `sync_calendar_events::etag_map` misses
            // and the sync layer treats the path-only uri as a new entry,
            // which then fails to delete cleanly on the next pass. Same
            // `resolve_url_against` both halves use keeps them byte-equal.
            for (uri, ical) in parse::parse_multiget_report(&response_body) {
                let normalized = self.resolve_url_against(&resolved_calendar_url, &uri);
                all_results.push((normalized, ical));
            }
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

        // Same listing-shape href normalization as `fetch_events`; see the
        // comment there.
        Ok(parse::parse_multiget_report(&response_body)
            .into_iter()
            .map(|(uri, ical)| (self.resolve_url_against(&resolved_url, &uri), ical))
            .collect())
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
        let etag = response_etag(resp.headers());

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
            && let Some(prepared) = prepare_if_match_etag(etag_val)
            && let Ok(val) = prepared.parse::<reqwest::header::HeaderValue>()
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
        let new_etag = response_etag(resp.headers());

        Ok(new_etag)
    }

    /// Delete an event via DELETE.
    ///
    /// If `etag` is provided, sends an `If-Match` header for conflict detection.
    pub async fn delete_event(&self, event_url: &str, etag: Option<&str>) -> Result<(), String> {
        let url = self.resolve_url(event_url);

        let mut req = self.http.delete(&url).headers(self.auth_headers());

        if let Some(etag_val) = etag
            && let Some(prepared) = prepare_if_match_etag(etag_val)
            && let Ok(val) = prepared.parse::<reqwest::header::HeaderValue>()
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
        let content_type = response_content_type(&resp);
        let text = resp.text().await.map_err(|e| format!("read body: {e}"))?;

        // Some auth flows (SSO portals, misconfigured nginx in front of a
        // CalDAV server) terminate before reaching the backend and return
        // 200 OK with an HTML login page. Without this guard the parser
        // sees an "empty multistatus", returns no calendars/events, and
        // the sync layer interprets that as "all events deleted" - silent
        // local-cache wipe. Rejecting HTML responses here makes the
        // failure visible (caller surfaces an error) instead of silent.
        if content_type
            .as_deref()
            .is_some_and(|ct| ct.eq_ignore_ascii_case("text/html") || ct.starts_with("text/html"))
        {
            return Err(format!(
                "PROPFIND {url} returned non-XML response (content-type: {}); refusing to treat as multistatus",
                content_type.as_deref().unwrap_or("?")
            ));
        }

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
        let content_type = response_content_type(&resp);
        let text = resp.text().await.map_err(|e| format!("read body: {e}"))?;

        if content_type
            .as_deref()
            .is_some_and(|ct| ct.eq_ignore_ascii_case("text/html") || ct.starts_with("text/html"))
        {
            return Err(format!(
                "REPORT {url} returned non-XML response (content-type: {}); refusing to treat as multistatus",
                content_type.as_deref().unwrap_or("?")
            ));
        }

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
        // RFC 3986 § 5.2 relative-reference resolution treats the base path's
        // last segment as a "file" and replaces it when the reference is
        // path-relative. For a calendar listed without trailing slash
        // (`https://host/cal/user/work`) and an event href like `event.ics`
        // that drops `work` and produces `https://host/cal/user/event.ics`.
        // CalDAV collections are conceptually directories regardless of any
        // server's trailing-slash discipline; some servers (Davical,
        // Bedework, old Zimbra, some shared-hosting frontends) emit
        // collection URIs without one and then return event hrefs relative
        // to them. Append `/` to the base path before joining so the
        // relative href lands underneath the collection.
        let join_base = ensure_collection_trailing_slash(base, href);
        if let Ok(base_url) = url::Url::parse(join_base.as_ref())
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

/// Collect every `<href>` value that is a direct child of the named property.
///
/// RFC 4791 § 6.2.1 specifies hrefs as immediate children of the property
/// element. Restricting the match to direct-parent (rather than "any
/// ancestor on the stack matches") filters out nested descriptor hrefs
/// that some bridges emit - notably Davical's delegation mode, which
/// includes `<owner><href>/principals/admin/</href></owner>` alongside
/// the real hrefs. With any-ancestor matching the descriptor href ended
/// up first in the returned vec; since discovery currently consumes
/// `.first()`, that mis-routed discovery to the admin's principal URL.
///
/// Returning a `Vec<String>` rather than `Option<String>` lets callers
/// see all the legitimate top-level hrefs (delegation / shared-account
/// setups on Apple Calendar Server, Kerio, and Exchange-bridged servers
/// emit multiple). Single-href callers (`current-user-principal`) take
/// `.first()`, and multi-href ones can warn or enumerate.
///
/// Both `Event::Text` and `Event::CData` are accumulated into the value
/// buffer; some servers wrap href values in CDATA sections.
fn extract_hrefs_property(xml: &str, property_name: &str) -> Vec<String> {
    use quick_xml::Reader;
    use quick_xml::escape::unescape;
    use quick_xml::events::Event;

    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut hrefs: Vec<String> = Vec::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = local_name(e.name().as_ref());
                stack.push(name);
                buf.clear();
            }
            Ok(Event::Text(ref e)) => {
                if let Ok(raw) = std::str::from_utf8(e.as_ref())
                    && let Ok(text) = unescape(raw)
                {
                    buf.push_str(&text);
                }
            }
            Ok(Event::CData(ref e)) => {
                if let Ok(text) = e.decode() {
                    buf.push_str(&text);
                }
            }
            Ok(Event::End(_)) => {
                let parent_is_property = stack
                    .iter()
                    .rev()
                    .nth(1)
                    .is_some_and(|n| n == property_name);
                let is_href_close = stack.last().is_some_and(|n| n == "href");
                if parent_is_property && is_href_close {
                    let val = buf.trim().to_string();
                    if !val.is_empty() {
                        hrefs.push(val);
                    }
                }
                stack.pop();
                buf.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    hrefs
}

/// Extract the local name from a possibly-namespaced XML tag.
fn local_name(raw: &[u8]) -> String {
    let full = String::from_utf8_lossy(raw);
    match full.rfind(':') {
        Some(idx) => full[idx + 1..].to_string(),
        None => full.to_string(),
    }
}

/// Read the response's `Content-Type` header as a lowercased, parameter-
/// stripped media type (e.g. `application/xml`). Returns `None` if the
/// header is missing, not ASCII-decodable, or empty.
fn response_content_type(resp: &reqwest::Response) -> Option<String> {
    let raw = resp.headers().get(reqwest::header::CONTENT_TYPE)?;
    let s = raw.to_str().ok()?;
    let trimmed = s.split(';').next()?.trim().to_ascii_lowercase();
    if trimmed.is_empty() { None } else { Some(trimmed) }
}

/// Escape `&`, `<`, and `>` for safe inclusion in XML element content.
///
/// Per RFC 3986 a URI cannot contain a literal `<` or `>`, but `&` is legal
/// in query strings and we have seen real-world CalDAV servers (notably
/// Exchange OWA's CalDAV bridge) emit hrefs containing `&` (typically from
/// query-string filter params on the server side). Splatting such an href
/// directly into a multiget request body produces invalid XML and the
/// server 400s the entire batch. This helper guards the multiget request
/// body against that.
fn xml_escape_text(s: &str) -> std::borrow::Cow<'_, str> {
    if !s.bytes().any(|b| matches!(b, b'&' | b'<' | b'>')) {
        return std::borrow::Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    std::borrow::Cow::Owned(out)
}

/// Format a stored ETag value for the `If-Match` header. Returns `None`
/// when the stored ETag is not in a shape RFC 7232 allows on `If-Match`,
/// telling the caller to skip the header entirely (last-write-wins for
/// this request, with self-correction on the next sync round).
///
/// New code stores ETags verbatim including quotes (`"abc"`, `W/"abc"`),
/// so the fast path is to pass the value through unchanged. We also
/// handle two corner cases the review found:
///
/// - **Weak ETag** (`W/"..."`). RFC 7232 § 3.1 / § 2.3.2: weak validators
///   "cannot be used in If-Match". Strict servers (some Apache mod_dav
///   builds, Cyrus, Cyrus-derived) reject every PUT/DELETE with 412 when
///   the request carries a weak ETag in If-Match. The retry sees the same
///   stored value and 412s again, so the client gets stuck until the row
///   refreshes via a full re-sync. Dropping If-Match for weak ETags lets
///   the request through; concurrent edits become last-write-wins for
///   that one request, which is the standard treatment when only weak
///   validators are available.
/// - **Corrupted weak ETag** (`W/abc`, no inner quote). Pre-fix rows
///   written by an earlier `trim_matches('"')` path lost the inner quote
///   and cannot be reconstructed. The previous shape wrapped the whole
///   token into `"W/abc"` (a strong ETag with `W/` as part of its
///   opaque-tag) which the server then 412s on every retry. Drop
///   If-Match for these and recover via a clean verbatim ETag on next
///   write.
///
/// **Bare strong ETag** (`abc`, no quotes) is still wrapped into `"abc"`:
/// stored before the verbatim-storage fix landed, recoverable.
/// Append a trailing slash to a CalDAV collection base URL when the path
/// lacks one and the relative reference about to be joined would otherwise
/// drop the last path segment. See `resolve_url_against` for context.
fn ensure_collection_trailing_slash<'a>(base: &'a str, href: &str) -> std::borrow::Cow<'a, str> {
    if href.starts_with('/') || href.is_empty() {
        return std::borrow::Cow::Borrowed(base);
    }
    if href.starts_with('#') || href.starts_with('?') {
        return std::borrow::Cow::Borrowed(base);
    }
    let parsed = url::Url::parse(base).ok();
    let path_ends_with_slash = match &parsed {
        Some(u) => u.path().ends_with('/'),
        None => base.ends_with('/'),
    };
    if path_ends_with_slash {
        return std::borrow::Cow::Borrowed(base);
    }
    // Insert '/' before any query/fragment so `https://host/path?x=1` ->
    // `https://host/path/?x=1` rather than `https://host/path?x=1/`.
    if let Some(parsed) = parsed
        && (parsed.query().is_some() || parsed.fragment().is_some())
        && let Ok(mut u) = url::Url::parse(base)
    {
        u.set_path(&format!("{}/", u.path()));
        return std::borrow::Cow::Owned(u.to_string());
    }
    let mut owned = base.to_string();
    owned.push('/');
    std::borrow::Cow::Owned(owned)
}

/// Re-relativize an absolute href against the request URL when the two
/// share an origin. Used by `fetch_events` to defend against strict
/// servers (older SOGo) that 400 multiget bodies whose hrefs don't share
/// scheme+host with the URL the REPORT was POSTed to.
fn relativize_for_multiget(request_url: &str, href: &str) -> String {
    if !(href.starts_with("http://") || href.starts_with("https://")) {
        return href.to_string();
    }
    let (Ok(req_url), Ok(href_url)) = (url::Url::parse(request_url), url::Url::parse(href)) else {
        return href.to_string();
    };
    if req_url.scheme() == href_url.scheme()
        && req_url.host_str() == href_url.host_str()
        && req_url.port_or_known_default() == href_url.port_or_known_default()
    {
        let mut path = href_url.path().to_string();
        if let Some(q) = href_url.query() {
            path.push('?');
            path.push_str(q);
        }
        path
    } else {
        href.to_string()
    }
}

/// Extract a server-supplied ETag from a response header map. ETags are
/// supposed to be ASCII (RFC 7232 BNF allows only printable ASCII inside
/// the opaque-tag), but Yahoo / Kerio / Zimbra have been seen emitting
/// non-ASCII bytes. The previous shape called `to_str().ok()` and silently
/// dropped those, breaking optimistic concurrency on the next PUT and
/// triggering an extra GET round-trip for the next save. Use lossy UTF-8
/// so the byte content survives (any subsequent `If-Match` round-trips
/// through `prepare_if_match_etag`'s `parse::<HeaderValue>()` which
/// re-validates), and log when bytes get replaced so an operator chasing
/// "stuck on save" symptoms has a starting point.
fn response_etag(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let val = headers.get("etag")?;
    if let Ok(s) = val.to_str() {
        return Some(s.to_string());
    }
    let bytes = val.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let lossy = String::from_utf8_lossy(bytes).into_owned();
    log::warn!(
        "CalDAV ETag has non-ASCII bytes; storing lossy UTF-8 ({lossy:?}). \
         Optimistic concurrency may be degraded on the next write."
    );
    Some(lossy)
}

fn prepare_if_match_etag(stored: &str) -> Option<String> {
    let s = stored.trim();
    if s.is_empty() {
        return None;
    }
    if s.starts_with("W/") {
        // Weak ETag: RFC 7232 forbids strong-comparison use, so don't send.
        return None;
    }
    if s.starts_with('"') {
        return Some(s.to_string());
    }
    Some(format!("\"{s}\""))
}

#[cfg(test)]
mod tests {
    use super::{
        ensure_collection_trailing_slash, extract_hrefs_property, prepare_if_match_etag,
        relativize_for_multiget, response_etag, xml_escape_text,
    };

    #[test]
    fn preserves_already_quoted_strong_etag() {
        assert_eq!(
            prepare_if_match_etag("\"abc\""),
            Some("\"abc\"".to_string())
        );
    }

    #[test]
    fn drops_weak_etag_per_rfc_7232() {
        // RFC 7232 § 2.3.2: weak validators cannot be used in If-Match.
        // Strict servers reject every PUT/DELETE with 412 if the request
        // includes a weak ETag. Drop If-Match entirely instead.
        assert_eq!(prepare_if_match_etag("W/\"abc\""), None);
    }

    #[test]
    fn drops_legacy_corrupted_weak_etag() {
        // Pre-fix rows had `W/` followed by an opaque-tag with the inner
        // quote stripped. The previous wrapper produced "W/abc" (a strong
        // ETag whose opaque text is `W/abc`) which strict servers 412'd
        // on every retry. Drop If-Match instead of sending nonsense.
        assert_eq!(prepare_if_match_etag("W/abc"), None);
    }

    #[test]
    fn wraps_legacy_bare_etag() {
        // Pre-fix rows have unquoted strong ETags. Wrap them so they conform
        // to RFC 7232 before going on the wire.
        assert_eq!(prepare_if_match_etag("abc"), Some("\"abc\"".to_string()));
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(
            prepare_if_match_etag("  \"abc\"  "),
            Some("\"abc\"".to_string())
        );
    }

    #[test]
    fn empty_or_whitespace_etag_returns_none() {
        assert_eq!(prepare_if_match_etag(""), None);
        assert_eq!(prepare_if_match_etag("   "), None);
    }

    #[test]
    fn ensure_trailing_slash_appends_when_path_missing_one() {
        // Calendar URL has no trailing slash and href is path-relative:
        // append `/` so Url::join lands under the collection rather than
        // replacing its last segment.
        let got = ensure_collection_trailing_slash("https://h/cal/user/work", "event.ics");
        assert_eq!(got, "https://h/cal/user/work/");
    }

    #[test]
    fn ensure_trailing_slash_preserves_existing_slash() {
        let got = ensure_collection_trailing_slash("https://h/cal/user/work/", "event.ics");
        assert_eq!(got, "https://h/cal/user/work/");
    }

    #[test]
    fn ensure_trailing_slash_skips_for_absolute_href() {
        // When the href is absolute-path or absolute-URL, RFC 3986 resolution
        // doesn't drop the base segment - leave the base as-is.
        let got = ensure_collection_trailing_slash("https://h/cal/user/work", "/cal/event.ics");
        assert_eq!(got, "https://h/cal/user/work");
    }

    #[test]
    fn ensure_trailing_slash_handles_query_strings() {
        // For a base with a query string, slash must go before the `?`.
        let got = ensure_collection_trailing_slash("https://h/cal/user/work?token=x", "event.ics");
        assert!(got.contains("/work/"));
        assert!(got.contains("token=x"));
    }

    #[test]
    fn relativize_multiget_strips_origin_when_shared() {
        // Older SOGo rejects absolute hrefs that don't match the request
        // URL's scheme+host. Same-origin absolute hrefs collapse to path.
        let got = relativize_for_multiget(
            "https://cal.example/cal/user/work/",
            "https://cal.example/cal/user/work/event.ics",
        );
        assert_eq!(got, "/cal/user/work/event.ics");
    }

    #[test]
    fn relativize_multiget_keeps_cross_origin_absolute() {
        let got = relativize_for_multiget(
            "https://cal.example/cal/user/work/",
            "https://other.example/cal/event.ics",
        );
        assert_eq!(got, "https://other.example/cal/event.ics");
    }

    #[test]
    fn relativize_multiget_passes_through_relative() {
        let got = relativize_for_multiget("https://cal.example/cal/", "event.ics");
        assert_eq!(got, "event.ics");
    }

    #[test]
    fn response_etag_recovers_non_ascii_bytes() {
        // Yahoo / Kerio / Zimbra have been seen emitting ETags with
        // non-ASCII bytes. The previous shape silently dropped those via
        // `to_str().ok()`; the lossy fallback preserves enough of the
        // value for it to round-trip through If-Match on the next write.
        let mut headers = reqwest::header::HeaderMap::new();
        let bytes: &[u8] = b"\"abc\xff\xff\"";
        headers.insert(
            "etag",
            reqwest::header::HeaderValue::from_bytes(bytes).expect("valid header bytes"),
        );
        let etag = response_etag(&headers).expect("recovered");
        assert!(etag.starts_with('"'));
        assert!(etag.ends_with('"'));
    }

    #[test]
    fn xml_escape_passes_through_safe_strings() {
        // Safe inputs should not allocate.
        assert!(matches!(
            xml_escape_text("/cal/event.ics"),
            std::borrow::Cow::Borrowed(_)
        ));
        assert_eq!(xml_escape_text("/cal/event.ics"), "/cal/event.ics");
    }

    #[test]
    fn extract_hrefs_returns_single_for_normal_principal() {
        let xml = r#"<?xml version="1.0"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/</D:href>
    <D:propstat>
      <D:prop>
        <D:current-user-principal>
          <D:href>/principals/users/alice/</D:href>
        </D:current-user-principal>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#;
        let hrefs = extract_hrefs_property(xml, "current-user-principal");
        assert_eq!(hrefs, vec!["/principals/users/alice/".to_string()]);
    }

    #[test]
    fn extract_hrefs_collects_all_from_multi_home_delegation() {
        // RFC 4791 § 6.2.1 lets calendar-home-set carry multiple hrefs
        // (delegation, shared accounts). Apple Calendar Server, Kerio, and
        // Exchange-bridged servers do this. The previous "first-href wins"
        // shape silently dropped the rest.
        let xml = r#"<?xml version="1.0"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:response>
    <D:href>/principals/users/alice/</D:href>
    <D:propstat>
      <D:prop>
        <C:calendar-home-set>
          <D:href>/calendars/alice/</D:href>
          <D:href>/calendars/team/</D:href>
          <D:href>/calendars/shared/</D:href>
        </C:calendar-home-set>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#;
        let hrefs = extract_hrefs_property(xml, "calendar-home-set");
        assert_eq!(
            hrefs,
            vec![
                "/calendars/alice/".to_string(),
                "/calendars/team/".to_string(),
                "/calendars/shared/".to_string(),
            ]
        );
    }

    #[test]
    fn extract_hrefs_filters_out_nested_owner_href() {
        // Some bridges (Davical in delegation mode) emit `<owner><href/></owner>`
        // alongside the real hrefs inside `<calendar-home-set>`. The matcher
        // requires `<href>` to be a *direct child* of the property element, so
        // the owner's nested href is filtered out and only the real home-set
        // href is collected. Without this filter the owner href - lexically
        // first in the document - would be picked up as the home and mis-route
        // discovery to the admin's principal URL.
        let xml = r#"<?xml version="1.0"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:response>
    <D:href>/principals/users/alice/</D:href>
    <D:propstat>
      <D:prop>
        <C:calendar-home-set>
          <D:owner>
            <D:href>/principals/users/admin/</D:href>
          </D:owner>
          <D:href>/calendars/alice/</D:href>
        </C:calendar-home-set>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#;
        let hrefs = extract_hrefs_property(xml, "calendar-home-set");
        assert_eq!(hrefs, vec!["/calendars/alice/".to_string()]);
    }

    #[test]
    fn extract_hrefs_handles_cdata_wrapped_href() {
        // Sibling parsers in this file accept CDATA-wrapped element content;
        // the prior `extract_href_property` only consumed `Event::Text`, so a
        // CDATA-wrapped principal href silently failed discovery.
        let xml = r#"<?xml version="1.0"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/</D:href>
    <D:propstat>
      <D:prop>
        <D:current-user-principal>
          <D:href><![CDATA[/principals/users/alice/]]></D:href>
        </D:current-user-principal>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#;
        let hrefs = extract_hrefs_property(xml, "current-user-principal");
        assert_eq!(hrefs, vec!["/principals/users/alice/".to_string()]);
    }

    #[test]
    fn extract_hrefs_returns_empty_when_property_absent() {
        let xml = r#"<?xml version="1.0"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/</D:href>
  </D:response>
</D:multistatus>"#;
        assert!(extract_hrefs_property(xml, "calendar-home-set").is_empty());
    }

    #[test]
    fn xml_escape_amps_lt_gt() {
        // `&` shows up in real-world hrefs (Exchange OWA bridge in particular)
        // and must be escaped or the entire multiget batch 400s on the
        // server side as malformed XML.
        assert_eq!(xml_escape_text("/cal/a&b.ics"), "/cal/a&amp;b.ics");
        assert_eq!(xml_escape_text("/cal/a<b>.ics"), "/cal/a&lt;b&gt;.ics");
        assert_eq!(
            xml_escape_text("/cal/a&b<c>d.ics"),
            "/cal/a&amp;b&lt;c&gt;d.ics"
        );
    }
}
