// ---------------------------------------------------------------------------
// XML request bodies
// ---------------------------------------------------------------------------

pub(super) const PROPFIND_PRINCIPAL: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:">
  <D:prop>
    <D:current-user-principal/>
  </D:prop>
</D:propfind>"#;

pub(super) const PROPFIND_CALENDAR_HOME: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:prop>
    <C:calendar-home-set/>
  </D:prop>
</D:propfind>"#;

pub(super) const PROPFIND_CALENDARS: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"
            xmlns:CS="http://calendarserver.org/ns/"
            xmlns:IC="http://apple.com/ns/ical/">
  <D:prop>
    <D:resourcetype/>
    <D:displayname/>
    <CS:getctag/>
    <IC:calendar-color/>
    <D:current-user-privilege-set/>
  </D:prop>
</D:propfind>"#;

pub(super) const PROPFIND_EVENTS: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:">
  <D:prop>
    <D:getetag/>
    <D:getcontenttype/>
  </D:prop>
</D:propfind>"#;

pub(super) const PROPFIND_CTAG: &str = r#"<?xml version="1.0" encoding="utf-8"?>
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
pub(super) fn extract_hrefs_property(xml: &str, property_name: &str) -> Vec<String> {
    collect_hrefs(xml, property_name, usize::MAX)
}

/// Single-href fast path. Stops the parser at the first matching href close,
/// avoiding a full scan + `Vec<String>` allocation for callers that only
/// consume `.first()` (e.g. `current-user-principal`). (Round 4 #35.)
pub(super) fn extract_first_href_property(xml: &str, property_name: &str) -> Option<String> {
    collect_hrefs(xml, property_name, 1).into_iter().next()
}

fn collect_hrefs(xml: &str, property_name: &str, limit: usize) -> Vec<String> {
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
                        if hrefs.len() >= limit {
                            return hrefs;
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
pub(super) fn response_content_type(resp: &reqwest::Response) -> Option<String> {
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
pub(super) fn xml_escape_text(s: &str) -> std::borrow::Cow<'_, str> {
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
pub(super) fn ensure_collection_trailing_slash<'a>(base: &'a str, href: &str) -> std::borrow::Cow<'a, str> {
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

/// Re-relativize an absolute href against an already-parsed request URL.
///
/// Used by `fetch_events` to defend against strict servers (older SOGo)
/// that 400 multiget bodies whose hrefs don't share scheme+host with the
/// URL the REPORT was POSTed to. The parsed-URL variant lets the caller
/// hoist parsing out of the per-href loop. (Round 3 #34.)
pub(super) fn relativize_for_multiget_parsed(request_url: Option<&url::Url>, href: &str) -> String {
    if !(href.starts_with("http://") || href.starts_with("https://")) {
        return href.to_string();
    }
    let Some(req_url) = request_url else {
        return href.to_string();
    };
    let Ok(href_url) = url::Url::parse(href) else {
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

/// Compatibility wrapper retained for tests. Production callers use
/// `relativize_for_multiget_parsed` and parse the request URL once per
/// batch (see `fetch_events`).
#[cfg(test)]
fn relativize_for_multiget(request_url: &str, href: &str) -> String {
    let parsed = url::Url::parse(request_url).ok();
    relativize_for_multiget_parsed(parsed.as_ref(), href)
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
pub(super) fn response_etag(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let val = headers.get("etag")?;
    if let Ok(s) = val.to_str() {
        return Some(s.to_owned());
    }
    let bytes = val.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let lossy = String::from_utf8_lossy(bytes).into_owned();
    // Lossy bytes leave U+FFFD in the stored value; `HeaderValue::from_str`
    // rejects those, so `prepare_if_match_etag` -> `parse::<HeaderValue>()`
    // will fail and we'll skip the `If-Match` header on the next PUT/DELETE
    // (last-write-wins for that one request, then self-correcting on the
    // following sync round). (Round 4 #37.)
    log::warn!(
        "CalDAV ETag has non-ASCII bytes; storing lossy UTF-8 ({lossy:?}). \
         If-Match will be omitted on the next write."
    );
    Some(lossy)
}

pub(super) fn prepare_if_match_etag(stored: &str) -> Option<String> {
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
