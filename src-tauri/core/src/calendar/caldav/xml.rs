/// XML namespace prefix discovery and element extraction helpers for CalDAV.

pub(super) fn xml_ns_prefixes_for<'a>(xml: &'a str, ns_uri: &str) -> Vec<std::borrow::Cow<'a, str>> {
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

pub(super) fn split_xml_responses(xml: &str) -> Vec<&str> {
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

pub(super) fn join_url_path(base: &str, segment: &str) -> Result<String, String> {
    let base = if base.ends_with('/') {
        base.to_string()
    } else {
        format!("{base}/")
    };
    resolve_href(&base, segment)
}
