#![allow(dead_code)]

use base64::Engine;

const BASE64_STANDARD: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

pub fn encode_utf7_imap(text: &str) -> String {
    let mut result = String::new();
    let escaped = text.replace('&', "&-");
    let mut rest = escaped.as_str();

    while !rest.is_empty() {
        let ascii = take_ascii_prefix(rest);
        result.push_str(ascii);
        rest = &rest[ascii.len()..];

        if rest.is_empty() {
            break;
        }

        let non_ascii = take_non_ascii_prefix(rest);
        result.push_str(&encode_modified_utf7(non_ascii));
        rest = &rest[non_ascii.len()..];
    }

    result
}

pub fn decode_utf7_imap(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(start) = rest.find('&') {
        result.push_str(&rest[..start]);
        rest = &rest[start..];

        if let Some(end) = rest.find('-') {
            result.push_str(&decode_utf7_part(&rest[..=end]));
            rest = &rest[end + 1..];
        } else {
            result.push_str(rest);
            break;
        }
    }

    result.push_str(rest);
    result
}

fn is_ascii_imap(byte: u8) -> bool {
    (0x20..=0x7f).contains(&byte)
}

fn take_ascii_prefix(s: &str) -> &str {
    let bytes = s.as_bytes();
    for (i, &byte) in bytes.iter().enumerate() {
        if !is_ascii_imap(byte) {
            return &s[..i];
        }
    }
    s
}

fn take_non_ascii_prefix(s: &str) -> &str {
    let bytes = s.as_bytes();
    for (i, &byte) in bytes.iter().enumerate() {
        if is_ascii_imap(byte) {
            return &s[..i];
        }
    }
    s
}

fn encode_modified_utf7(text: &str) -> String {
    let mut utf16be = Vec::with_capacity(text.len() * 2);
    for unit in text.encode_utf16() {
        utf16be.extend_from_slice(&unit.to_be_bytes());
    }

    let b64 = BASE64_STANDARD.encode(utf16be);
    let modified = b64.trim_end_matches('=').replace('/', ",");
    format!("&{modified}-")
}

fn decode_utf7_part(text: &str) -> String {
    if text == "&-" {
        return "&".to_string();
    }

    let body = &text[1..text.len() - 1];
    let mut b64 = body.replace(',', "/");
    while b64.len() % 4 != 0 {
        b64.push('=');
    }

    let bytes = BASE64_STANDARD
        .decode(b64)
        .expect("invalid modified UTF-7 mailbox name");

    let mut units = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        units.push(u16::from_be_bytes([chunk[0], chunk[1]]));
    }

    std::char::decode_utf16(units)
        .map(|r| r.expect("invalid UTF-16 in modified UTF-7 mailbox name"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{decode_utf7_imap, encode_utf7_imap};

    #[test]
    fn encode_test() {
        assert_eq!(
            encode_utf7_imap("Отправленные"),
            "&BB4EQgQ,BEAEMAQyBDsENQQ9BD0ESwQ1-"
        );
    }

    #[test]
    fn encode_test_split() {
        assert_eq!(
            encode_utf7_imap("Šiukšliadėžė"),
            "&AWA-iuk&AWE-liad&ARcBfgEX-"
        );
    }

    #[test]
    fn encode_consecutive_accents() {
        assert_eq!(encode_utf7_imap("théâtre"), "th&AOkA4g-tre");
    }

    #[test]
    fn decode_test() {
        assert_eq!(
            decode_utf7_imap("&BB4EQgQ,BEAEMAQyBDsENQQ9BD0ESwQ1-"),
            "Отправленные"
        );
    }

    #[test]
    fn decode_test_split() {
        assert_eq!(decode_utf7_imap("&AWA-iuk&AWE-liad&ARcBfgEX-"), "Šiukšliadėžė");
    }

    #[test]
    fn decode_consecutive_accents() {
        assert_eq!(decode_utf7_imap("th&AOkA4g-tre"), "théâtre");
    }

    #[test]
    fn round_trip_ampersand() {
        let input = "INBOX & Stuff";
        assert_eq!(decode_utf7_imap(&encode_utf7_imap(input)), input);
    }
}
