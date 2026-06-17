use reqwest::header::{CONTENT_TYPE, IF_MATCH};
use reqwest::{Method, StatusCode};

use super::parse;

mod helpers;

use helpers::{
    PROPFIND_CALENDAR_HOME, PROPFIND_CALENDARS, PROPFIND_CTAG, PROPFIND_EVENTS, PROPFIND_PRINCIPAL,
    ensure_collection_trailing_slash, extract_first_href_property, extract_hrefs_property,
    prepare_if_match_etag, relativize_for_multiget_parsed, response_content_type, response_etag,
    xml_escape_text,
};

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
    /// Whether the authenticated user has write/write-content privileges on
    /// this calendar. `None` means the server didn't emit a
    /// `<current-user-privilege-set>` block - older servers commonly omit
    /// it; the sync layer treats `None` as "assume editable" to preserve
    /// pre-fix behavior on those. `Some(false)` is reserved for calendars
    /// that explicitly lack write privileges (iCloud / Fastmail / SOGo
    /// shared-read-only calendars), and the sync layer uses it to suppress
    /// edit affordances at the UI.
    pub can_edit: Option<bool>,
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
                    return attempt
                        .error("CalDAV refusing https -> http scheme downgrade redirect");
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
                        self.calendar_home_url = Some(self.resolve_url_against(&principal, &home));
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
        //
        // Capture the final URL after redirects: hosted Exchange bridges
        // and Apple iCloud occasionally redirect the base URL to a
        // per-tenant or per-shard host (`caldav.icloud.com`, etc.). A
        // relative `current-user-principal` href in that response must
        // resolve against the redirect target, not the original base URL,
        // or the next PROPFIND lands on the wrong host. The well-known
        // fallback below has had this fix since b8398928; bringing it to
        // the base-URL try closes the matching gap. (Round 3 #30.)
        let mut last_error = match self
            .propfind_with_final_url(&self.base_url, "0", PROPFIND_PRINCIPAL)
            .await
        {
            Ok((_, final_url, body)) => {
                if let Some(principal) =
                    extract_first_href_property(&body, "current-user-principal")
                {
                    return Ok(self.resolve_url_against(&final_url, &principal));
                }
                "PROPFIND on base URL returned no current-user-principal".to_string()
            }
            Err(e) => format!("PROPFIND on base URL failed: {e}"),
        };

        // Fallback: probe `.well-known/caldav`. RFC 6764 § 6 recommends this
        // as a discovery hint when the client doesn't know the DAV root. We
        // only reach it if the base URL didn't yield a principal.
        //
        // Capture the final URL after redirects: providers (Apple iCloud,
        // some hosted Exchange bridges) redirect this to a different host
        // (e.g. `caldav.icloud.com`). A relative principal href in the
        // response must resolve against the redirect target, not the
        // original base URL - otherwise the next PROPFIND lands on the
        // wrong host and 404s.
        let well_known_url = format!("{}/.well-known/caldav", self.base_url);
        match self
            .propfind_with_final_url(&well_known_url, "0", PROPFIND_PRINCIPAL)
            .await
        {
            Ok((_, final_url, body)) => {
                if let Some(principal) =
                    extract_first_href_property(&body, "current-user-principal")
                {
                    return Ok(self.resolve_url_against(&final_url, &principal));
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
            // Resolve calendar hrefs against the home-set URL we just
            // PROPFIND'd, not against `self.base_url`. Fastmail
            // (caldav.fastmail.com vs the API host), hosted Exchange
            // bridges, and any setup where login-host and DAV-host
            // differ have a calendar-home-set living on a different
            // origin than base_url. Resolving against base_url silently
            // rebuilt the calendar URL on the wrong host and every
            // event PROPFIND/PUT/DELETE thereafter went to the wrong
            // origin or 404'd. (Round 3 #31.)
            cal.href = self.resolve_url_against(&url, &cal.href);
        }
        if calendars.is_empty() {
            // Distinguish "user has no calendars provisioned" from "server
            // returned a parseable but content-free multistatus" (which
            // can happen on first-login races, server-side errors mis-
            // reported as 207, or a parser limitation we haven't seen
            // yet). Log so an operator chasing "where did my calendars
            // go" has a starting point. The caller still gets Ok(empty)
            // - this is informational, not an error. (Round 3 #49.)
            let response_count = parse::count_propfind_response_children(&body);
            if response_count == 0 {
                log::warn!(
                    "CalDAV list_calendars at {url} returned a 207 with zero <response> children ({} bytes); first-login race or a server-side error misreported as 207",
                    body.len()
                );
            } else {
                log::warn!(
                    "CalDAV list_calendars at {url} parsed {response_count} <response> children but found 0 calendars; the home-set has no calendars or only non-calendar resources are visible"
                );
            }
        }
        Ok(calendars)
    }

    // -----------------------------------------------------------------------
    // Event listing
    // -----------------------------------------------------------------------

    /// List all events in a calendar (URIs + ETags).
    ///
    /// Event URIs are resolved to absolute URLs against the calendar URL for
    /// the same reason `list_calendars` does so for calendar hrefs. The
    /// returned struct also carries hrefs that the server reported as
    /// failing (non-2xx response status, or no successful propstat block);
    /// the sync layer must preserve local copies for those rather than
    /// mistaking server-reported failure for "absent". (Round 3 #51.)
    pub async fn list_events(
        &self,
        calendar_url: &str,
    ) -> Result<parse::PropfindEventsResult, String> {
        let url = self.resolve_url(calendar_url);
        let (_, body) = self
            .propfind_raw(&url, "1", PROPFIND_EVENTS)
            .await
            .map_err(|e| format!("PROPFIND events failed: {e}"))?;

        let mut result = parse::parse_propfind_events(&body);
        for entry in &mut result.entries {
            entry.uri = self.resolve_url_against(&url, &entry.uri);
        }
        for uri in &mut result.failed_uris {
            *uri = self.resolve_url_against(&url, uri);
        }
        Ok(result)
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
        // Parse the request URL once for the entire batch. The previous
        // shape parsed it inside `relativize_for_multiget` for every URI;
        // for a 50-URI batch that's 50 redundant `url::Url::parse`
        // calls per REPORT. (Round 3 #34.)
        let parsed_request_url = url::Url::parse(&resolved_calendar_url).ok();
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
                let body_href = relativize_for_multiget_parsed(parsed_request_url.as_ref(), uri);
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
            .header(
                reqwest::header::ACCEPT,
                "text/calendar, application/calendar+xml, */*",
            )
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
        let (status, _final_url, text) = self.propfind_with_final_url(url, depth, body).await?;
        Ok((status, text))
    }

    /// Variant that also returns the URL the response actually came from.
    /// Used by the well-known fallback in `discover_principal`: a server
    /// that redirects `/.well-known/caldav` to a different host would
    /// otherwise have its relative principal href resolved against the
    /// original `base_url`, landing the next PROPFIND on the wrong host.
    async fn propfind_with_final_url(
        &self,
        url: &str,
        depth: &str,
        body: &str,
    ) -> Result<(StatusCode, String, String), String> {
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
        let final_url = resp.url().to_string();
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
            Ok((status, final_url, text))
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
            && let Ok(mut resolved) = base_url.join(href)
        {
            // RFC 3986 § 5.3 drops the base's query when the reference
            // is relative and carries no query of its own. CalDAV
            // typically authenticates through the Authorization header,
            // so this is rarely a practical failure mode - but
            // shared-hosting front-ends occasionally pass a routing or
            // session token through `?token=...` on the calendar URL
            // and silently dropping that token sends event
            // PROPFINDs/PUTs to a tenant-less route. Re-attach the
            // base's query when the href didn't supply one. (Round 3
            // #33.)
            if resolved.query().is_none()
                && let Some(base_query) = base_url.query()
                && !base_query.is_empty()
            {
                resolved.set_query(Some(base_query));
            }
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
