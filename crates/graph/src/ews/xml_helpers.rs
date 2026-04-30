use quick_xml::Reader;
use quick_xml::escape::{resolve_xml_entity, unescape};
use quick_xml::events::{BytesRef, Event};

// ── SOAP envelope ───────────────────────────────────────────

pub(super) fn build_soap_envelope(body_xml: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"
               xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types"
               xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages">
  <soap:Header>
    <t:RequestServerVersion Version="Exchange2016"/>
  </soap:Header>
  <soap:Body>
    {body_xml}
  </soap:Body>
</soap:Envelope>"#
    )
}

// ── XML helpers ─────────────────────────────────────────────

pub(super) fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Strip namespace prefixes from element names for easier matching.
/// e.g. "t:FolderId" -> "FolderId", "soap:Fault" -> "Fault"
pub(super) fn strip_ns(name: &str) -> &str {
    match name.find(':') {
        Some(i) => &name[i + 1..],
        None => name,
    }
}

/// Well-known distinguished folder IDs that EWS treats specially.
pub(super) fn is_distinguished_folder_id(id: &str) -> bool {
    matches!(
        id,
        "publicfoldersroot"
            | "inbox"
            | "drafts"
            | "sentitems"
            | "deleteditems"
            | "junkemail"
            | "outbox"
            | "calendar"
            | "contacts"
            | "tasks"
            | "notes"
            | "root"
            | "msgfolderroot"
    )
}

// ── SOAP fault check ────────────────────────────────────────

pub(super) fn check_soap_fault(xml: &str) -> Result<(), String> {
    let mut reader = Reader::from_str(xml);
    let mut in_fault = false;
    let mut in_faultstring = false;
    let mut fault_message = String::new();
    let mut buf = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let local = strip_ns(&name);
                if local == "Fault" {
                    in_fault = true;
                }
                if in_fault && local == "faultstring" {
                    in_faultstring = true;
                }
                buf.clear();
            }
            Ok(Event::Text(ref e)) => {
                if let Ok(raw) = std::str::from_utf8(e.as_ref())
                    && let Ok(text) = unescape(raw)
                {
                    buf.push_str(&text);
                }
            }
            Ok(Event::GeneralRef(ref e)) => push_general_ref(e, &mut buf),
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let local = strip_ns(&name);
                if in_faultstring && local == "faultstring" {
                    fault_message = buf.trim().to_string();
                    in_faultstring = false;
                }
                if local == "Fault" {
                    break;
                }
                buf.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    if in_fault || !fault_message.is_empty() {
        let msg = if fault_message.is_empty() {
            "Unknown SOAP fault".to_string()
        } else {
            fault_message
        };
        return Err(format!("EWS SOAP Fault: {msg}"));
    }

    Ok(())
}

// quick-xml 0.36+ emits Event::GeneralRef separately from Event::Text, so
// every parser that accumulates body text needs to fold these back in or
// `&lt;` and friends silently vanish.
pub(crate) fn push_general_ref(e: &BytesRef<'_>, buf: &mut String) {
    let Ok(name) = std::str::from_utf8(e.as_ref()) else {
        return;
    };
    if let Some(rest) = name.strip_prefix('#') {
        let codepoint = if let Some(hex) = rest.strip_prefix(['x', 'X']) {
            u32::from_str_radix(hex, 16).ok()
        } else {
            rest.parse::<u32>().ok()
        };
        if let Some(c) = codepoint.and_then(char::from_u32) {
            buf.push(c);
        }
    } else if let Some(s) = resolve_xml_entity(name) {
        buf.push_str(s);
    }
}

// ── Attribute extraction ────────────────────────────────────

pub(super) fn extract_attribute(e: &quick_xml::events::BytesStart, attr_name: &str) -> String {
    for attr in e.attributes().flatten() {
        if String::from_utf8_lossy(attr.key.as_ref()) == attr_name {
            return String::from_utf8_lossy(&attr.value).to_string();
        }
    }
    String::new()
}
