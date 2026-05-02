// ---------------------------------------------------------------------------
// XML parsing helpers for CalDAV PROPFIND/REPORT responses
// ---------------------------------------------------------------------------
//
// reviewed (R3 verified non-issue): every `Event::Text` handler in this
// module funnels through `quick_xml::escape::unescape`, which decodes the
// full XML entity set (`&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;`, plus
// numeric `&#x2014;` / `&#8212;` for em-dash etc). Display names, ETags,
// and href values that arrive entity-encoded come out as plain text. CDATA
// sections are accumulated into the same `buf` as text via the parallel
// `Event::CData` arm, so CDATA-wrapped values round-trip too.

use quick_xml::Reader;
use quick_xml::escape::unescape;
use quick_xml::events::Event;

use super::super::client::DiscoveredCalendar;

/// A single event entry from a PROPFIND listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalDavEventEntry {
    pub uri: String,
    pub etag: String,
}

/// Parse a PROPFIND Depth:1 response to extract calendar collections.
///
/// Looks for responses whose `<resourcetype>` contains a `<calendar>` marker
/// (either self-closed `<C:calendar/>` or open-close `<C:calendar></C:calendar>`).
///
/// Field reads are scoped to the expected XML parent to avoid clobbering:
/// `<href>` is read only as a direct child of `<response>`; `<displayname>`,
/// `<getctag>`, and `<calendar-color>` are read only as direct children of
/// `<prop>`. This keeps a `<href>` nested inside a `<privilege>` descriptor
/// (returned by SOGo / Radicale alongside the prop block) from overwriting
/// the calendar's own href.
///
/// Both `Event::Text` and `Event::CData` are accumulated into the field
/// buffer, since some servers wrap `calendar-data` and other large text in
/// CDATA sections.
pub fn parse_propfind_calendars(xml: &str) -> Vec<DiscoveredCalendar> {
    let mut reader = Reader::from_str(xml);
    let mut calendars = Vec::new();

    let mut stack: Vec<String> = Vec::new();
    let mut buf = String::new();

    // Response-level state, populated only from 2xx propstats and emitted at
    // </response> if the response itself isn't an error.
    let mut response_href = String::new();
    let mut response_status = String::new();
    let mut response_is_calendar = false;
    let mut response_displayname = String::new();
    let mut response_ctag = String::new();
    let mut response_color = String::new();
    // None when the server didn't emit current-user-privilege-set; Some
    // when we observed the block and could decide. The sync layer treats
    // None as "assume editable" for compatibility with older servers that
    // don't surface privileges.
    let mut response_can_edit: Option<bool> = None;

    // Per-propstat pending state, committed to response-level state only when
    // the closing `<status>` says 2xx. Mixed propstat blocks (one 200 OK for
    // the props the server can serve, one 404 Not Found for the ones it
    // can't) are handled by this commit gate; the previous shape merged
    // values from both blocks regardless of status.
    //
    // reviewed (R3 verified non-issue): the parser doesn't accumulate values
    // across multiple `<propstat>` blocks at the data-structure level, but
    // it doesn't need to -- response-level fields are reset at `<response>`
    // and each `<prop>` only overwrites the fields it actually contains
    // (via the `(Some("prop"), name)` matches below). With the 2xx commit
    // gate, that's exactly the RFC 4918 behavior.
    let mut propstat_status = String::new();
    let mut pending_is_calendar = false;
    let mut pending_displayname: Option<String> = None;
    let mut pending_ctag: Option<String> = None;
    let mut pending_color: Option<String> = None;
    // Tracks whether we observed a current-user-privilege-set block at all
    // (privilege_set_seen) and whether it included a write-class privilege
    // (write_seen). RFC 3744 § 5.3: we look for `write`, `write-content`,
    // or `all` - any of those imply write access. The full enumeration
    // (write-properties, write-acl, etc) doesn't matter for our edit-or-
    // not decision.
    let mut pending_privilege_set_seen = false;
    let mut pending_write_seen = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = local_name(e.name().as_ref());
                if name == "response" {
                    response_href.clear();
                    response_status.clear();
                    response_is_calendar = false;
                    response_displayname.clear();
                    response_ctag.clear();
                    response_color.clear();
                    response_can_edit = None;
                }
                if name == "propstat" {
                    propstat_status.clear();
                    pending_is_calendar = false;
                    pending_displayname = None;
                    pending_ctag = None;
                    pending_color = None;
                    pending_privilege_set_seen = false;
                    pending_write_seen = false;
                }
                // Open-close `<calendar></calendar>` form, scoped to inside
                // `<resourcetype>`.
                if name == "calendar" && stack.iter().any(|s| s == "resourcetype") {
                    pending_is_calendar = true;
                }
                // We deliberately do NOT mark `pending_privilege_set_seen`
                // when the `<current-user-privilege-set>` element opens.
                // Some test mocks and a handful of real servers emit a
                // self-closed empty privilege-set as the "unknown ACL"
                // sentinel, which the previous shape interpreted as
                // "explicit empty -> no privileges granted -> read-only"
                // and silently flipped can_edit to false. Anchor the
                // signal on the first `<privilege>` ancestor we see
                // instead, so a privilege-set with no children stays as
                // "unknown" (which the sync layer treats as editable for
                // compatibility with older servers). (Round 3 #25.)
                if name == "privilege" {
                    pending_privilege_set_seen = true;
                }
                if (name == "write" || name == "write-content" || name == "all")
                    && stack.iter().any(|s| s == "privilege")
                {
                    pending_write_seen = true;
                }
                stack.push(name);
                buf.clear();
            }
            Ok(Event::Empty(ref e)) => {
                let name = local_name(e.name().as_ref());
                // Self-closed `<calendar/>` form.
                if name == "calendar" && stack.iter().any(|s| s == "resourcetype") {
                    pending_is_calendar = true;
                }
                // Self-closed `<privilege/>` (rare but legal: the server
                // is asserting the privilege block exists without
                // enumerating sub-elements). Same anchor as above.
                if name == "privilege" {
                    pending_privilege_set_seen = true;
                }
                // Self-closed `<D:write/>`, `<D:write-content/>`, or `<D:all/>`
                // inside `<D:privilege>`.
                if (name == "write" || name == "write-content" || name == "all")
                    && stack.iter().any(|s| s == "privilege")
                {
                    pending_write_seen = true;
                }
            }
            Ok(Event::Text(ref e)) => {
                if let Ok(raw) = std::str::from_utf8(e.as_ref())
                    && let Ok(text) = unescape(raw)
                {
                    buf.push_str(&text);
                }
            }
            Ok(Event::CData(ref e)) => {
                match e.decode() {
                    Ok(text) => buf.push_str(&text),
                    Err(err) => log::warn!("CalDAV PROPFIND CDATA decode failed: {err}"),
                }
            }
            Ok(Event::End(ref e)) => {
                let name = local_name(e.name().as_ref());
                let parent = stack.iter().rev().nth(1).map(String::as_str);
                match (parent, name.as_str()) {
                    (Some("response"), "href") => {
                        response_href = buf.trim().to_string();
                    }
                    (Some("response"), "status") => {
                        // Top-level <status> sibling of <propstat> applies
                        // to the whole resource (e.g. a 404 for a resource
                        // that vanished between PROPFIND and REPORT).
                        response_status = buf.trim().to_string();
                    }
                    (Some("propstat"), "status") => {
                        propstat_status = buf.trim().to_string();
                    }
                    (Some("prop"), "displayname") => {
                        pending_displayname = Some(buf.trim().to_string());
                    }
                    (Some("prop"), "getctag") => {
                        pending_ctag = Some(buf.trim().to_string());
                    }
                    (Some("prop"), "calendar-color") => {
                        pending_color = Some(normalize_calendar_color(buf.trim()));
                    }
                    _ => {}
                }
                if name == "propstat" {
                    if propstat_status_is_ok(&propstat_status) {
                        if pending_is_calendar {
                            response_is_calendar = true;
                        }
                        if let Some(v) = pending_displayname.take() {
                            response_displayname = v;
                        }
                        if let Some(v) = pending_ctag.take() {
                            response_ctag = v;
                        }
                        if let Some(v) = pending_color.take() {
                            response_color = v;
                        }
                        if pending_privilege_set_seen {
                            response_can_edit = Some(pending_write_seen);
                        }
                    } else {
                        log::debug!(
                            "Skipping propstat with non-2xx status: {propstat_status}"
                        );
                    }
                    propstat_status.clear();
                    pending_is_calendar = false;
                    pending_displayname = None;
                    pending_ctag = None;
                    pending_color = None;
                    pending_privilege_set_seen = false;
                    pending_write_seen = false;
                }
                if name == "response"
                    && response_is_calendar
                    && !response_href.is_empty()
                    && response_status_is_ok(&response_status)
                {
                    calendars.push(DiscoveredCalendar {
                        href: response_href.clone(),
                        display_name: if response_displayname.is_empty() {
                            None
                        } else {
                            Some(response_displayname.clone())
                        },
                        color: if response_color.is_empty() {
                            None
                        } else {
                            Some(response_color.clone())
                        },
                        ctag: if response_ctag.is_empty() {
                            None
                        } else {
                            Some(response_ctag.clone())
                        },
                        can_edit: response_can_edit,
                    });
                }
                stack.pop();
                buf.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    calendars
}

/// Status-line ok-ness for a `<propstat><status>` element.
///
/// Lenient on absence: some servers omit the status line entirely when it
/// would be 200 OK (RFC 4918 violation but real). We treat absence as OK so
/// pre-existing parser behavior on test fixtures and well-behaved servers is
/// preserved. Explicit non-2xx codes are honored: a `<propstat>` with status
/// `HTTP/1.1 404 Not Found` no longer leaks its (empty/cached) prop values
/// into the response-level state.
///
/// The status line per RFC 7230 § 3.1.2 is `HTTP/version SP code SP reason`;
/// the code is exactly three ASCII digits. We parse it strictly here rather
/// than the looser "second whitespace-separated token starts with '2'" so a
/// crafted server can't slip e.g. `HTTP/1.1 2xx Custom` past the gate.
/// Empty stays OK; anything that isn't an explicit 200..=299 falls through.
fn propstat_status_is_ok(status: &str) -> bool {
    if status.is_empty() {
        return true;
    }
    let Some(code_token) = status.split_whitespace().nth(1) else {
        return false;
    };
    if code_token.len() != 3 || !code_token.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    code_token
        .parse::<u16>()
        .ok()
        .is_some_and(|n| (200..=299).contains(&n))
}

/// Status-line ok-ness for a top-level `<response><status>` element.
///
/// Used by the multiget parser to skip resources that returned an error at
/// the response level (e.g. a 404 for a resource that vanished between
/// PROPFIND and REPORT, or a 500 from SOGo on a resource that failed to
/// parse server-side). Same lenient-on-absence semantics as
/// `propstat_status_is_ok`.
fn response_status_is_ok(status: &str) -> bool {
    propstat_status_is_ok(status)
}

/// Result of `parse_propfind_events`. Splits successfully-parsed event
/// entries from hrefs the server explicitly reported as failing, so the
/// sync layer can preserve local copies for the failed set rather than
/// mistaking a server-reported error for "absent" and deleting locally.
/// (Round 3 #51.)
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PropfindEventsResult {
    pub entries: Vec<CalDavEventEntry>,
    /// Hrefs whose `<response>` carried either a non-2xx response-level
    /// status or had no successful `<propstat>` block. The local copy must
    /// be preserved across this sync round - the server reported a
    /// failure, not an absence.
    pub failed_uris: Vec<String>,
}

/// Parse a PROPFIND Depth:1 response to extract event URIs and ETags.
///
/// Field reads are parent-scoped (see `parse_propfind_calendars`) and CDATA
/// sections are accumulated alongside text. ETag values are preserved verbatim
/// including the RFC 7232 quotes / weak indicator.
pub fn parse_propfind_events(xml: &str) -> PropfindEventsResult {
    let mut reader = Reader::from_str(xml);
    let mut entries = Vec::new();
    let mut failed_uris: Vec<String> = Vec::new();

    let mut stack: Vec<String> = Vec::new();
    let mut buf = String::new();

    let mut response_href = String::new();
    let mut response_status = String::new();
    let mut response_etag = String::new();
    let mut response_content_type = String::new();
    let mut response_is_collection = false;
    // True when at least one `<propstat>` in the current `<response>` came
    // back with a 2xx status. If a response has propstat blocks but none
    // are 2xx, we flag the href as failed so the sync layer doesn't
    // mistake it for "absent". (Round 3 #51.)
    let mut any_ok_propstat = false;
    // True if we observed any `<propstat>` at all this response. Lets us
    // distinguish "response carried no propstat blocks" from "every
    // propstat was non-2xx".
    let mut any_propstat_seen = false;

    let mut propstat_status = String::new();
    let mut pending_etag: Option<String> = None;
    let mut pending_content_type: Option<String> = None;
    let mut pending_is_collection = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = local_name(e.name().as_ref());
                if name == "response" {
                    response_href.clear();
                    response_status.clear();
                    response_etag.clear();
                    response_content_type.clear();
                    response_is_collection = false;
                    any_ok_propstat = false;
                    any_propstat_seen = false;
                }
                if name == "propstat" {
                    propstat_status.clear();
                    pending_etag = None;
                    pending_content_type = None;
                    pending_is_collection = false;
                    any_propstat_seen = true;
                }
                // RFC 4918 § 14.10: <D:resourcetype><D:collection/></D:resourcetype>
                // marks a CalDAV sub-collection. Davical, Bedework, and old
                // Zimbra emit collection URIs without a trailing slash and may
                // also expose a getetag, so without this guard the
                // is_icalendar_resource third-arm fallback would admit them
                // as event resources. The follow-up REPORT then 403s on the
                // collection and can fail the whole batch.
                if name == "collection"
                    && stack.iter().rev().any(|n| n == "resourcetype")
                {
                    pending_is_collection = true;
                }
                stack.push(name);
                buf.clear();
            }
            Ok(Event::Empty(ref e)) => {
                // Self-closing form: <D:collection/> shows up here, not in
                // the Start branch.
                let name = local_name(e.name().as_ref());
                if name == "collection"
                    && stack.iter().rev().any(|n| n == "resourcetype")
                {
                    pending_is_collection = true;
                }
            }
            Ok(Event::Text(ref e)) => {
                if let Ok(raw) = std::str::from_utf8(e.as_ref())
                    && let Ok(text) = unescape(raw)
                {
                    buf.push_str(&text);
                }
            }
            Ok(Event::CData(ref e)) => {
                match e.decode() {
                    Ok(text) => buf.push_str(&text),
                    Err(err) => log::warn!("CalDAV PROPFIND CDATA decode failed: {err}"),
                }
            }
            Ok(Event::End(ref e)) => {
                let name = local_name(e.name().as_ref());
                let parent = stack.iter().rev().nth(1).map(String::as_str);
                match (parent, name.as_str()) {
                    (Some("response"), "href") => {
                        response_href = buf.trim().to_string();
                    }
                    (Some("response"), "status") => {
                        response_status = buf.trim().to_string();
                    }
                    (Some("propstat"), "status") => {
                        propstat_status = buf.trim().to_string();
                    }
                    (Some("prop"), "getetag") => {
                        // ETag preserved verbatim - see RFC 7232.
                        // reviewed (R3 verified non-issue): RFC 7232 allows
                        // arbitrary opaque-tag content inside the quotes,
                        // including `:` and `/` (e.g. `"abc/def:1"`). We do
                        // no further parsing on the storage path, so those
                        // round-trip cleanly through `If-Match`.
                        pending_etag = Some(buf.trim().to_string());
                    }
                    (Some("prop"), "getcontenttype") => {
                        pending_content_type = Some(buf.trim().to_string());
                    }
                    _ => {}
                }
                if name == "propstat" {
                    if propstat_status_is_ok(&propstat_status) {
                        any_ok_propstat = true;
                        if let Some(v) = pending_etag.take() {
                            response_etag = v;
                        }
                        if let Some(v) = pending_content_type.take() {
                            response_content_type = v;
                        }
                        if pending_is_collection {
                            response_is_collection = true;
                        }
                    } else {
                        log::debug!(
                            "Skipping propstat with non-2xx status: {propstat_status}"
                        );
                    }
                    propstat_status.clear();
                    pending_etag = None;
                    pending_content_type = None;
                    pending_is_collection = false;
                }
                if name == "response" {
                    let response_ok = response_status_is_ok(&response_status);
                    let pushed_entry = response_ok
                        && !response_href.is_empty()
                        && !response_etag.is_empty()
                        && !response_is_collection
                        && is_icalendar_resource(&response_href, &response_content_type);
                    if pushed_entry {
                        entries.push(CalDavEventEntry {
                            uri: response_href.clone(),
                            etag: response_etag.clone(),
                        });
                    } else if !response_href.is_empty() {
                        // Flag as failed only when the server explicitly
                        // reported an error for this href - either at the
                        // response level (non-2xx status) or via every
                        // propstat block returning non-2xx. Hrefs we
                        // skipped for our own reasons (collection,
                        // non-iCal content type) are NOT failures - they
                        // were never event resources to begin with, and
                        // adding them to failed_uris would hold non-event
                        // hrefs in the local cache forever. (Round 3 #51.)
                        let response_level_failed = !response_ok;
                        let propstat_level_failed =
                            any_propstat_seen && !any_ok_propstat;
                        if response_level_failed || propstat_level_failed {
                            failed_uris.push(response_href.clone());
                        }
                    }
                }
                stack.pop();
                buf.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    PropfindEventsResult {
        entries,
        failed_uris,
    }
}

/// Parse a CTag from a PROPFIND response.
///
/// Scoped to direct child of `<prop>` and accumulates both Text and CData.
pub fn parse_ctag(xml: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<String> = Vec::new();
    let mut buf = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                stack.push(local_name(e.name().as_ref()));
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
            Ok(Event::End(ref e)) => {
                let name = local_name(e.name().as_ref());
                let parent = stack.iter().rev().nth(1).map(String::as_str);
                if parent == Some("prop") && name == "getctag" {
                    let val = buf.trim().to_string();
                    if !val.is_empty() {
                        return Some(val);
                    }
                }
                stack.pop();
                buf.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    None
}

/// Parse a calendar-multiget or calendar-query REPORT response.
///
/// Returns `Vec<(uri, ical_data)>`. `<calendar-data>` is the prime case where
/// servers wrap large iCalendar payloads in `<![CDATA[...]]>`; we accumulate
/// both Text and CData arms so either shape parses correctly.
pub fn parse_multiget_report(xml: &str) -> Vec<(String, String)> {
    let mut reader = Reader::from_str(xml);
    let mut results = Vec::new();

    let mut stack: Vec<String> = Vec::new();
    let mut buf = String::new();

    let mut response_href = String::new();
    let mut response_status = String::new();
    let mut response_ical = String::new();

    let mut propstat_status = String::new();
    let mut pending_ical: Option<String> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = local_name(e.name().as_ref());
                if name == "response" {
                    response_href.clear();
                    response_status.clear();
                    response_ical.clear();
                }
                if name == "propstat" {
                    propstat_status.clear();
                    pending_ical = None;
                }
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
                match e.decode() {
                    Ok(text) => buf.push_str(&text),
                    Err(err) => log::warn!("CalDAV multiget CDATA decode failed: {err}"),
                }
            }
            Ok(Event::End(ref e)) => {
                let name = local_name(e.name().as_ref());
                let parent = stack.iter().rev().nth(1).map(String::as_str);
                match (parent, name.as_str()) {
                    (Some("response"), "href") => {
                        response_href = buf.trim().to_string();
                    }
                    (Some("response"), "status") => {
                        // Top-level status: applies to the whole resource.
                        // SOGo's failure mode is to emit a 500 here while
                        // also echoing stale calendar-data inside a 200
                        // propstat - the response-level rejection here is
                        // what blocks the stale data from landing locally.
                        response_status = buf.trim().to_string();
                    }
                    (Some("propstat"), "status") => {
                        propstat_status = buf.trim().to_string();
                    }
                    (Some("prop"), "calendar-data") => {
                        // calendar-data: trim only outer whitespace so that
                        // intentional CRLF folding inside the iCal payload
                        // is preserved.
                        pending_ical = Some(buf.trim().to_string());
                    }
                    _ => {}
                }
                if name == "propstat" {
                    if propstat_status_is_ok(&propstat_status) {
                        if let Some(v) = pending_ical.take() {
                            response_ical = v;
                        }
                    } else {
                        log::debug!(
                            "Skipping multiget propstat with non-2xx status: {propstat_status}"
                        );
                    }
                    propstat_status.clear();
                    pending_ical = None;
                }
                if name == "response"
                    && response_status_is_ok(&response_status)
                    && !response_href.is_empty()
                    && !response_ical.is_empty()
                {
                    results.push((response_href.clone(), response_ical.clone()));
                } else if name == "response"
                    && !response_status_is_ok(&response_status)
                    && !response_href.is_empty()
                {
                    // Response had an explicit non-2xx status. Log the
                    // resource so an operator chasing "this event silently
                    // disappeared from sync" can see which entries were
                    // dropped.
                    log::debug!(
                        "Multiget response for {response_href} returned non-2xx status: {response_status}; dropping"
                    );
                }
                stack.pop();
                buf.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    results
}

/// Extract the local name from a possibly-namespaced XML tag.
///
/// We accept any namespace prefix here rather than restricting to the four
/// well-known DAV / CalDAV / calendarserver / iCal URIs. In practice the
/// element scoping (`<response>`, `<prop>`, `<resourcetype>`) provides the
/// disambiguation: a stray `xyz:href` outside a `<response>` context is
/// ignored by every parser, and a `xyz:href` inside one would be a
/// malformed multistatus response anyway. The wider acceptance trades a
/// theoretical false-positive risk for forgiveness with bridges that
/// remap namespaces (Davical's "DAV1", Apple's `CALDAV` aliases).
fn local_name(raw: &[u8]) -> String {
    let full = String::from_utf8_lossy(raw);
    match full.rfind(':') {
        Some(idx) => full[idx + 1..].to_string(),
        None => full.to_string(),
    }
}

/// Count `<response>` elements at any depth in a multistatus body.
///
/// Used to disambiguate a "207 with zero `<response>` children" (server bug
/// or first-login race) from a "207 with responses but none are calendars"
/// (home-set legitimately empty, or only non-calendar resources visible).
/// Cheap second pass over an already-fetched body, only invoked on the
/// empty-result path. (Round 3 #49.)
pub fn count_propfind_response_children(xml: &str) -> usize {
    let mut reader = Reader::from_str(xml);
    let mut count = 0;
    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) if local_name(e.name().as_ref()) == "response" => {
                count += 1;
            }
            Ok(Event::Eof) | Err(_) => return count,
            _ => {}
        }
    }
}

/// Check if a resource looks like an iCalendar resource.
///
/// Content-type matching is case-insensitive (RFC 7231 § 3.1.1.1: media-type
/// comparison is case-insensitive); some servers emit `TEXT/CALENDAR` and
/// callers used to silently skip every event resource on those servers.
/// `.ics` matching ignores case for the same reason - servers occasionally
/// emit `.ICS` upstream and we shouldn't second-guess that.
/// Normalize a `calendar-color` value to a `#RRGGBB` form.
///
/// Apple Calendar emits an ARGB form (`#RRGGBBAA`, e.g. `#0000FFFF` for
/// fully-opaque blue), but most other servers emit `#RRGGBB`. UI consumers
/// then have to handle both shapes; rather than divergent handling at
/// every render site, fold the alpha at parse time. Bytes outside the
/// well-known 7- and 9-character forms pass through verbatim - vendors
/// have shipped names (`blue`) and uncommon-length hex strings, and
/// silently rewriting those would be worse than leaving them as-is.
fn normalize_calendar_color(raw: &str) -> String {
    let s = raw.trim();
    if s.len() == 9
        && s.starts_with('#')
        && s.bytes().skip(1).all(|b| b.is_ascii_hexdigit())
    {
        // Apple ARGB form: drop the trailing alpha pair. We could also
        // honor opacity by multiplying RGB into the background, but the
        // user-visible UI treats labels as opaque so the rest of the
        // stack already assumes RGB.
        return s[..7].to_string();
    }
    s.to_string()
}

fn is_icalendar_resource(href: &str, content_type: &str) -> bool {
    let ct_lower = content_type.to_ascii_lowercase();
    if ct_lower.contains("text/calendar") {
        return true;
    }
    // RFC 6321 (xCal): the CalDAV-XML form is also a valid event
    // representation. Some servers emit `application/calendar+xml`; without
    // matching it here we'd silently skip those events and the listing
    // would come up short.
    if ct_lower.contains("application/calendar+xml") {
        return true;
    }
    // Strip a query string before looking at the extension. Some servers
    // append revision tokens or auth parameters to event hrefs
    // (`/cal/event.ics?revision=42`); without trimming the query the path
    // ends with neither `.ics` nor `/` and falls through to the third-arm
    // fallback only if the content-type was also empty.
    let path_tail = href.split(['?', '#']).next().unwrap_or(href);
    if path_tail.to_ascii_lowercase().ends_with(".ics") {
        return true;
    }
    // Accept entries with an etag but no content type info. Test the
    // *path tail* (already query/fragment-stripped above), not the raw
    // href: a collection URL like `/cal/folder/?revision=1` doesn't
    // end with `/` once the query is in the way, so the previous
    // `!href.ends_with('/')` admitted collections through this arm
    // when the server emitted no content-type. The
    // `<resourcetype><collection/>` gate at the parser level catches
    // most of these, but a server that omits resourcetype entirely and
    // a query string in the URL slipped through. (Round 3 #23.)
    content_type.is_empty() && !path_tail.ends_with('/')
}

#[cfg(test)]
mod tests;
