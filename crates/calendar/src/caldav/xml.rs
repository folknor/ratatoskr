/// XML namespace prefix discovery and element extraction helpers for CalDAV.
use quick_xml::Reader;
use quick_xml::escape::unescape;
use quick_xml::events::Event;

/// Known CalDAV namespace URIs to search for prefixes.
const KNOWN_NS_URIS: &[&str] = &[
    "DAV:",
    "urn:ietf:params:xml:ns:caldav",
    "http://calendarserver.org/ns/",
    "http://apple.com/ns/ical/",
];

/// Discover all namespace prefixes bound to `ns_uri` in the document.
///
/// Returns prefixes in the form used by `quick_xml` local-name matching:
/// an empty string for the default namespace, or `"prefix:"` for named ones.
fn xml_ns_prefixes_for(xml: &str, ns_uri: &str) -> Vec<String> {
    let mut prefixes = Vec::new();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e) | Event::Empty(ref e)) => {
                for attr in e.attributes().flatten() {
                    let value = String::from_utf8_lossy(&attr.value);
                    if value.as_ref() != ns_uri {
                        continue;
                    }
                    let key = String::from_utf8_lossy(attr.key.as_ref());
                    if key == "xmlns" {
                        // Default namespace binding
                        if !prefixes.contains(&String::new()) {
                            prefixes.push(String::new());
                        }
                    } else if let Some(prefix) = key.strip_prefix("xmlns:") {
                        let entry = format!("{prefix}:");
                        if !prefixes.contains(&entry) {
                            prefixes.push(entry);
                        }
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    // Always include empty prefix as fallback (elements may omit namespace).
    if !prefixes.contains(&String::new()) {
        prefixes.push(String::new());
    }
    prefixes
}

/// Split a CalDAV multistatus XML document into individual `<response>` fragments.
///
/// Returns slices of the original string, each containing one `<…:response>…</…:response>` block.
#[allow(clippy::cast_possible_truncation)]
pub(super) fn split_xml_responses(xml: &str) -> Vec<&str> {
    let dav_prefixes = xml_ns_prefixes_for(xml, "DAV:");
    let mut responses = Vec::new();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    loop {
        let offset_start = reader.buffer_position() as usize;
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let local = local_name_str(e.local_name().as_ref());
                if local.eq_ignore_ascii_case("response")
                    && tag_has_any_prefix(e.name().as_ref(), &dav_prefixes, "response")
                {
                    // Find end of this element by reading to its matching End.
                    let start = offset_start;
                    let mut depth = 1u32;
                    loop {
                        match reader.read_event() {
                            Ok(Event::Start(ref inner))
                                if local_name_str(inner.local_name().as_ref())
                                    .eq_ignore_ascii_case("response") =>
                            {
                                depth += 1;
                            }
                            Ok(Event::End(ref inner))
                                if local_name_str(inner.local_name().as_ref())
                                    .eq_ignore_ascii_case("response") =>
                            {
                                depth -= 1;
                                if depth == 0 {
                                    let end = reader.buffer_position() as usize;
                                    responses.push(&xml[start..end]);
                                    break;
                                }
                            }
                            Ok(Event::Eof) | Err(_) => break,
                            _ => {}
                        }
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    responses
}

/// Extract the text content of the first `<href>` inside a named property element.
pub(super) fn extract_first_href_for_property(
    xml: &str,
    property_names: &[&str],
) -> Option<String> {
    for property_name in property_names {
        if let Some(section) = extract_first_element(xml, property_name)
            && let Some(href) = extract_first_tag_value(section, &["href"])
        {
            return Some(href);
        }
    }
    None
}

pub(super) fn extract_first_tag_value(xml: &str, tag_names: &[&str]) -> Option<String> {
    tag_names
        .iter()
        .find_map(|tag_name| extract_tag_value(xml, tag_name))
}

pub(crate) fn extract_tag_value(xml: &str, tag_name: &str) -> Option<String> {
    extract_first_element(xml, tag_name).and_then(extract_element_text)
}

/// Find the first element whose local name matches `tag_name` (case-insensitive,
/// any namespace prefix) and return the raw XML slice including the element's
/// start and end tags.
#[allow(clippy::cast_possible_truncation)]
fn extract_first_element<'a>(xml: &'a str, tag_name: &str) -> Option<&'a str> {
    let all_prefixes = {
        let mut prefixes = Vec::new();
        for ns in KNOWN_NS_URIS {
            for p in xml_ns_prefixes_for(xml, ns) {
                if !prefixes.contains(&p) {
                    prefixes.push(p);
                }
            }
        }
        prefixes
    };

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    loop {
        let offset_start = reader.buffer_position() as usize;
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let local = local_name_str(e.local_name().as_ref());
                if local.eq_ignore_ascii_case(tag_name)
                    && tag_has_any_prefix(e.name().as_ref(), &all_prefixes, tag_name)
                {
                    let start = offset_start;
                    let mut depth = 1u32;
                    loop {
                        match reader.read_event() {
                            Ok(Event::Start(ref inner))
                                if local_name_str(inner.local_name().as_ref())
                                    .eq_ignore_ascii_case(tag_name) =>
                            {
                                depth += 1;
                            }
                            Ok(Event::End(ref inner))
                                if local_name_str(inner.local_name().as_ref())
                                    .eq_ignore_ascii_case(tag_name) =>
                            {
                                depth -= 1;
                                if depth == 0 {
                                    let end = reader.buffer_position() as usize;
                                    return Some(&xml[start..end]);
                                }
                            }
                            Ok(Event::Eof) | Err(_) => return None,
                            _ => {}
                        }
                    }
                }
            }
            Ok(Event::Empty(ref e)) => {
                let local = local_name_str(e.local_name().as_ref());
                if local.eq_ignore_ascii_case(tag_name)
                    && tag_has_any_prefix(e.name().as_ref(), &all_prefixes, tag_name)
                {
                    let start = offset_start;
                    let end = reader.buffer_position() as usize;
                    return Some(&xml[start..end]);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    None
}

pub(super) fn contains_any_tag(xml: &str, tag_names: &[&str]) -> bool {
    tag_names
        .iter()
        .any(|tag_name| extract_first_element(xml, tag_name).is_some())
}

pub(super) fn resolve_href(base: &str, href: &str) -> Result<String, String> {
    reqwest::Url::parse(base)
        .map_err(|e| format!("invalid base url: {e}"))?
        .join(href)
        .map(|url| url.to_string())
        .map_err(|e| format!("invalid CalDAV href {href}: {e}"))
}

pub(super) fn normalize_url_for_compare(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

fn extract_element_text(element: &str) -> Option<String> {
    let mut reader = Reader::from_str(element);
    reader.config_mut().trim_text(true);
    let mut depth = 0usize;
    let mut text = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(_)) => depth += 1,
            Ok(Event::Text(event)) => {
                if depth >= 1
                    && let Ok(raw) = std::str::from_utf8(event.as_ref())
                    && let Ok(unescaped) = unescape(raw)
                {
                    text.push_str(&unescaped);
                }
            }
            Ok(Event::CData(event)) => {
                if depth >= 1
                    && let Ok(decoded) = event.decode()
                {
                    text.push_str(&decoded);
                }
            }
            Ok(Event::End(_)) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    if text.is_empty() { None } else { Some(text) }
}

pub(super) fn join_url_path(base: &str, segment: &str) -> Result<String, String> {
    let base = if base.ends_with('/') {
        base.to_string()
    } else {
        format!("{base}/")
    };
    resolve_href(&base, segment)
}

/// Get the local name (after any `:`) from a full tag name byte slice.
fn local_name_str(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Check whether the full qualified tag name (e.g. `D:response`) matches any of
/// the given prefixes combined with `local_name`.
fn tag_has_any_prefix(full_name: &[u8], prefixes: &[String], local_name: &str) -> bool {
    let name = String::from_utf8_lossy(full_name);
    prefixes.iter().any(|prefix| {
        let candidate = format!("{prefix}{local_name}");
        name.eq_ignore_ascii_case(&candidate)
    })
}
