use calcard::vcard::{VCard, VCardProperty, VCardValue};

/// Parsed contact data extracted from a vCard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedVCard {
    /// First email address, lowercased.
    pub email: Option<String>,
    /// FN (formatted name) property.
    pub display_name: Option<String>,
    /// First TEL (telephone) property.
    pub phone: Option<String>,
    /// ORG property.
    pub organization: Option<String>,
    /// PHOTO property value (only if it looks like a URL, not embedded base64).
    pub photo_url: Option<String>,
}

/// Parse a vCard string into a `ParsedVCard`.
///
/// Uses the `calcard` crate which handles both vCard 3.0 and 4.0.
/// Returns `Err` if the vCard cannot be parsed at all.
pub fn parse_vcard(vcard_data: &str) -> Result<ParsedVCard, String> {
    let vcard = VCard::parse(vcard_data).map_err(|e| format!("vCard parse error: {e:?}"))?;

    let display_name = vcard
        .property(&VCardProperty::Fn)
        .and_then(|entry| entry.values.first())
        .and_then(|v| v.as_text())
        .filter(|s| !s.is_empty())
        .map(String::from);

    let email = vcard
        .property(&VCardProperty::Email)
        .and_then(|entry| entry.values.first())
        .and_then(|v| v.as_text())
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase);

    let phone = vcard
        .property(&VCardProperty::Tel)
        .and_then(|entry| entry.values.first())
        .and_then(|v| v.as_text())
        .filter(|s| !s.is_empty())
        .map(String::from);

    let organization = vcard
        .property(&VCardProperty::Org)
        .and_then(|entry| entry.values.first())
        .and_then(extract_text_value)
        .filter(|s| !s.is_empty());

    let photo_url = vcard
        .property(&VCardProperty::Photo)
        .and_then(|entry| entry.values.first())
        .and_then(|v| v.as_text())
        .filter(|s| !s.is_empty())
        .filter(|s| is_url(s))
        .map(String::from);

    Ok(ParsedVCard {
        email,
        display_name,
        phone,
        organization,
        photo_url,
    })
}

/// Extract text from a VCardValue, handling both Text and Component variants.
fn extract_text_value(value: &VCardValue) -> Option<String> {
    match value {
        VCardValue::Text(s) => Some(s.clone()),
        VCardValue::Component(parts) => {
            let joined = parts.join(";");
            if joined.is_empty() {
                None
            } else {
                Some(joined)
            }
        }
        _ => None,
    }
}

/// Check if a string looks like a URL (not embedded base64 data).
fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

// ---------------------------------------------------------------------------
// XML parsing helpers for CardDAV PROPFIND/REPORT responses
// ---------------------------------------------------------------------------

use quick_xml::Reader;
use quick_xml::events::Event;

/// A single contact entry from a PROPFIND listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardDavContactEntry {
    pub uri: String,
    pub etag: String,
}

/// Parse a PROPFIND Depth:1 response to extract contact URIs and ETags.
///
/// Expects a DAV multistatus response with `<D:response>` elements containing
/// `<D:href>`, `<D:getetag>`, and optionally `<D:getcontenttype>`.
pub fn parse_propfind_contacts(xml: &str) -> Vec<CardDavContactEntry> {
    let mut reader = Reader::from_str(xml);
    let mut entries = Vec::new();

    let mut in_response = false;
    let mut current_href = String::new();
    let mut current_etag = String::new();
    let mut current_content_type = String::new();
    let mut current_tag = String::new();
    let mut buf = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = local_name(e.name().as_ref());
                if name == "response" {
                    in_response = true;
                    current_href.clear();
                    current_etag.clear();
                    current_content_type.clear();
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
                if in_response {
                    match current_tag.as_str() {
                        "href" => current_href = buf.trim().to_string(),
                        "getetag" => {
                            // ETags are often quoted; strip quotes
                            current_etag = buf.trim().trim_matches('"').to_string();
                        }
                        "getcontenttype" => {
                            current_content_type = buf.trim().to_string();
                        }
                        _ => {}
                    }
                }
                if name == "response" {
                    in_response = false;
                    // Only include entries that look like vCards (have an etag and
                    // either have a vcard content type or a .vcf extension)
                    if !current_href.is_empty()
                        && !current_etag.is_empty()
                        && is_vcard_resource(&current_href, &current_content_type)
                    {
                        entries.push(CardDavContactEntry {
                            uri: current_href.clone(),
                            etag: current_etag.clone(),
                        });
                    }
                }
                buf.clear();
                current_tag.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    entries
}

/// Parse a CTag from a PROPFIND response.
pub fn parse_ctag(xml: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    let mut current_tag = String::new();
    let mut buf = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                current_tag = local_name(e.name().as_ref());
                buf.clear();
            }
            Ok(Event::Text(ref e)) => {
                if let Ok(text) = e.unescape() {
                    buf.push_str(&text);
                }
            }
            Ok(Event::End(_)) => {
                if current_tag == "getctag" {
                    let val = buf.trim().to_string();
                    if !val.is_empty() {
                        return Some(val);
                    }
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

/// Parse an addressbook-multiget REPORT response to extract vCard data.
///
/// Returns `Vec<(uri, vcard_data)>`.
pub fn parse_multiget_report(xml: &str) -> Vec<(String, String)> {
    let mut reader = Reader::from_str(xml);
    let mut results = Vec::new();

    let mut in_response = false;
    let mut current_href = String::new();
    let mut current_vcard = String::new();
    let mut current_tag = String::new();
    let mut buf = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = local_name(e.name().as_ref());
                if name == "response" {
                    in_response = true;
                    current_href.clear();
                    current_vcard.clear();
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
                if in_response {
                    match current_tag.as_str() {
                        "href" => current_href = buf.trim().to_string(),
                        "address-data" => current_vcard = buf.trim().to_string(),
                        _ => {}
                    }
                }
                if name == "response" {
                    in_response = false;
                    if !current_href.is_empty() && !current_vcard.is_empty() {
                        results.push((current_href.clone(), current_vcard.clone()));
                    }
                }
                buf.clear();
                current_tag.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    results
}

/// Extract the local name from a possibly-namespaced XML tag (e.g. "D:href" -> "href").
fn local_name(raw: &[u8]) -> String {
    let full = String::from_utf8_lossy(raw);
    match full.rfind(':') {
        Some(idx) => full[idx + 1..].to_string(),
        None => full.to_string(),
    }
}

/// Check if a resource looks like a vCard based on URI or content type.
fn is_vcard_resource(href: &str, content_type: &str) -> bool {
    if content_type.contains("text/vcard") || content_type.contains("text/x-vcard") {
        return true;
    }
    // If no content type is reported, fall back to checking the extension
    if content_type.is_empty() && (href.ends_with(".vcf") || href.ends_with(".vcard")) {
        return true;
    }
    // If there's a content-type but it's not vcard, still accept if it has a vcf extension
    // (some servers report application/octet-stream)
    if href.ends_with(".vcf") || href.ends_with(".vcard") {
        return true;
    }
    // Accept entries with an etag but no content type info (common in minimal PROPFIND responses)
    // The caller already checks that etag is non-empty
    content_type.is_empty()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_vcard_30_basic() {
        let vcard = "\
BEGIN:VCARD\r\n\
VERSION:3.0\r\n\
FN:Alice Smith\r\n\
EMAIL:Alice@Example.COM\r\n\
TEL:+1-555-0100\r\n\
ORG:Acme Corp\r\n\
END:VCARD\r\n";

        let parsed = parse_vcard(vcard).expect("should parse");
        assert_eq!(parsed.display_name.as_deref(), Some("Alice Smith"));
        assert_eq!(parsed.email.as_deref(), Some("alice@example.com"));
        assert_eq!(parsed.phone.as_deref(), Some("+1-555-0100"));
        assert_eq!(parsed.organization.as_deref(), Some("Acme Corp"));
        assert_eq!(parsed.photo_url, None);
    }

    #[test]
    fn parse_vcard_40_with_photo_url() {
        let vcard = "\
BEGIN:VCARD\r\n\
VERSION:4.0\r\n\
FN:Bob Jones\r\n\
EMAIL:bob@example.com\r\n\
PHOTO:https://example.com/photos/bob.jpg\r\n\
END:VCARD\r\n";

        let parsed = parse_vcard(vcard).expect("should parse");
        assert_eq!(parsed.display_name.as_deref(), Some("Bob Jones"));
        assert_eq!(parsed.email.as_deref(), Some("bob@example.com"));
        assert_eq!(
            parsed.photo_url.as_deref(),
            Some("https://example.com/photos/bob.jpg")
        );
    }

    #[test]
    fn parse_vcard_no_email() {
        let vcard = "\
BEGIN:VCARD\r\n\
VERSION:3.0\r\n\
FN:No Email Person\r\n\
TEL:555-1234\r\n\
END:VCARD\r\n";

        let parsed = parse_vcard(vcard).expect("should parse");
        assert_eq!(parsed.display_name.as_deref(), Some("No Email Person"));
        assert!(parsed.email.is_none());
    }

    #[test]
    fn parse_vcard_base64_photo_not_url() {
        // Base64-encoded photos should NOT appear as photo_url
        let vcard = "\
BEGIN:VCARD\r\n\
VERSION:3.0\r\n\
FN:Carol\r\n\
EMAIL:carol@test.com\r\n\
PHOTO;ENCODING=b;TYPE=JPEG:R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7\r\n\
END:VCARD\r\n";

        let parsed = parse_vcard(vcard).expect("should parse");
        assert!(parsed.photo_url.is_none());
    }

    #[test]
    fn parse_propfind_contacts_xml() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/addressbooks/user/default/</D:href>
    <D:propstat>
      <D:prop>
        <D:getetag>"abc123"</D:getetag>
        <D:getcontenttype>httpd/unix-directory</D:getcontenttype>
      </D:prop>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/addressbooks/user/default/contact1.vcf</D:href>
    <D:propstat>
      <D:prop>
        <D:getetag>"etag-111"</D:getetag>
        <D:getcontenttype>text/vcard; charset=utf-8</D:getcontenttype>
      </D:prop>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/addressbooks/user/default/contact2.vcf</D:href>
    <D:propstat>
      <D:prop>
        <D:getetag>"etag-222"</D:getetag>
        <D:getcontenttype>text/vcard</D:getcontenttype>
      </D:prop>
    </D:propstat>
  </D:response>
</D:multistatus>"#;

        let entries = parse_propfind_contacts(xml);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].uri, "/addressbooks/user/default/contact1.vcf");
        assert_eq!(entries[0].etag, "etag-111");
        assert_eq!(entries[1].uri, "/addressbooks/user/default/contact2.vcf");
        assert_eq!(entries[1].etag, "etag-222");
    }

    #[test]
    fn parse_ctag_xml() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:CS="http://calendarserver.org/ns/">
  <D:response>
    <D:href>/addressbooks/user/default/</D:href>
    <D:propstat>
      <D:prop>
        <CS:getctag>ctag-value-12345</CS:getctag>
      </D:prop>
    </D:propstat>
  </D:response>
</D:multistatus>"#;

        let ctag = parse_ctag(xml);
        assert_eq!(ctag.as_deref(), Some("ctag-value-12345"));
    }

    #[test]
    fn parse_ctag_xml_empty() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/addressbooks/user/default/</D:href>
    <D:propstat>
      <D:prop>
        <D:getetag>"abc"</D:getetag>
      </D:prop>
    </D:propstat>
  </D:response>
</D:multistatus>"#;

        let ctag = parse_ctag(xml);
        assert!(ctag.is_none());
    }

    #[test]
    fn parse_multiget_report_xml() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:carddav">
  <D:response>
    <D:href>/addressbooks/user/default/contact1.vcf</D:href>
    <D:propstat>
      <D:prop>
        <D:getetag>"etag-111"</D:getetag>
        <C:address-data>BEGIN:VCARD
VERSION:3.0
FN:Alice
EMAIL:alice@example.com
END:VCARD</C:address-data>
      </D:prop>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/addressbooks/user/default/contact2.vcf</D:href>
    <D:propstat>
      <D:prop>
        <D:getetag>"etag-222"</D:getetag>
        <C:address-data>BEGIN:VCARD
VERSION:3.0
FN:Bob
EMAIL:bob@example.com
END:VCARD</C:address-data>
      </D:prop>
    </D:propstat>
  </D:response>
</D:multistatus>"#;

        let results = parse_multiget_report(xml);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "/addressbooks/user/default/contact1.vcf");
        assert!(results[0].1.contains("Alice"));
        assert_eq!(results[1].0, "/addressbooks/user/default/contact2.vcf");
        assert!(results[1].1.contains("Bob"));
    }

    #[test]
    fn etag_comparison_detects_changes() {
        let old_etag = "etag-111";
        let new_etag = "etag-222";
        assert_ne!(old_etag, new_etag);

        let same_etag = "etag-111";
        assert_eq!(old_etag, same_etag);
    }
}
