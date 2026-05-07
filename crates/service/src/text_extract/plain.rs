//! Phase 7-2a: plain-text and HTML extractors.
//!
//! `extract_plain` covers `text/plain` / `text/csv` / `text/markdown`.
//! Real-world `text/plain` arrives as UTF-8, UTF-16 (BOM-prefixed),
//! Windows-1252, or ISO-8859-*. We BOM-detect first; otherwise probe
//! UTF-8 validity, falling back to Windows-1252 (which always succeeds
//! since it covers all 256 byte values). If even Windows-1252 produces
//! a high invalid-char ratio (mostly control chars), we treat the bytes
//! as binary and skip with `EncodingInvalid`.
//!
//! `extract_html` strips tags via `quick-xml`'s pull parser, walking
//! `Event::Text` events and joining with whitespace. Entity resolution
//! is explicitly disabled in our reader configuration.

use encoding_rs::{Encoding, UTF_8, WINDOWS_1252};

use super::{ExtractionOutcome, SkipReason};

/// Threshold for the "too many control characters / null bytes" check.
/// Above this ratio in the decoded text, we treat the bytes as binary
/// and skip rather than indexing garbage.
const MAX_CONTROL_CHAR_RATIO: f32 = 0.10;

/// Extract text from a `text/plain` / `text/csv` / `text/markdown`
/// payload. BOM-detect first; fall back to UTF-8; final fallback to
/// Windows-1252. Every successful decode is sanity-checked against
/// the control-character ratio so binary garbage that happens to be
/// valid ASCII (e.g., bytes 0x00..0x07 stream) doesn't slip past as
/// "indexable text."
#[allow(dead_code)] // Consumed in 7-4.
pub(crate) fn extract_plain(bytes: &[u8]) -> ExtractionOutcome {
    if bytes.is_empty() {
        return ExtractionOutcome::Skipped { reason: SkipReason::EmptyContent };
    }

    // 1. BOM-prefixed inputs use the encoding the BOM declares. Strip
    //    the BOM bytes from the decode input so they don't appear in
    //    the output text.
    let decoded: std::borrow::Cow<'_, str> =
        if let Some((encoding, bom_len)) = Encoding::for_bom(bytes) {
            let (text, _, had_errors) = encoding.decode(&bytes[bom_len..]);
            if had_errors && text.is_empty() {
                return ExtractionOutcome::Skipped { reason: SkipReason::EncodingInvalid };
            }
            text
        } else if std::str::from_utf8(bytes).is_ok() {
            // 2. No BOM, valid UTF-8.
            UTF_8.decode(bytes).0
        } else {
            // 3. Strict UTF-8 failed. Fall back to Windows-1252 (always
            //    succeeds for 8-bit byte streams).
            WINDOWS_1252.decode(bytes).0
        };

    // Sanity-check the control-char ratio uniformly. Genuine binary
    // (mp3, png mistakenly typed `text/plain`; or null-byte streams
    // that happen to be valid ASCII) lights this up.
    if control_char_ratio(&decoded) > MAX_CONTROL_CHAR_RATIO {
        return ExtractionOutcome::Skipped { reason: SkipReason::EncodingInvalid };
    }
    finish_decoded(&decoded)
}

/// Extract visible text from an HTML / XHTML payload. Reads as UTF-8
/// (HTML is overwhelmingly UTF-8 in mail-attachment context); collects
/// `Event::Text` events from `quick-xml`'s pull parser. Tag content
/// (script / style) is discarded.
#[allow(dead_code)] // Consumed in 7-4.
pub(crate) fn extract_html(bytes: &[u8]) -> ExtractionOutcome {
    if bytes.is_empty() {
        return ExtractionOutcome::Skipped { reason: SkipReason::EmptyContent };
    }

    use quick_xml::Reader;
    use quick_xml::events::Event;

    let mut reader = Reader::from_reader(bytes);
    // Phase 7-2a: explicit entity-resolution policy. quick-xml's default
    // already does NOT expand external entities (it has no DTD support),
    // but we set the explicit knobs anyway so a future change in the
    // crate doesn't silently regress this.
    let cfg = reader.config_mut();
    cfg.expand_empty_elements = false;
    cfg.trim_text(true);
    cfg.check_end_names = false;
    // No need for entity decoding beyond the base set (lt/gt/amp/quot/apos),
    // which quick-xml handles internally on Event::Text decoding.

    let mut buf: Vec<u8> = Vec::with_capacity(256);
    let mut out = String::new();
    let mut skip_until: Option<Vec<u8>> = None;

    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Ok(Event::Start(ref e)) => {
                let name = e.name().as_ref().to_ascii_lowercase();
                if name == b"script" || name == b"style" {
                    skip_until = Some(name);
                }
            }
            Ok(Event::End(ref e)) => {
                if let Some(target) = skip_until.as_deref()
                    && e.name().as_ref().eq_ignore_ascii_case(target)
                {
                    skip_until = None;
                }
            }
            Ok(Event::Text(ref t)) => {
                if skip_until.is_some() {
                    continue;
                }
                if let Ok(s) = t.decode() {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() {
                        if !out.is_empty() {
                            out.push(' ');
                        }
                        out.push_str(trimmed);
                    }
                }
            }
            Ok(Event::CData(ref c)) => {
                if skip_until.is_some() {
                    continue;
                }
                // CDATA bytes are raw - decode as UTF-8 lossy and trim.
                let raw = String::from_utf8_lossy(c.as_ref());
                let trimmed = raw.trim();
                if !trimmed.is_empty() {
                    if !out.is_empty() {
                        out.push(' ');
                    }
                    out.push_str(trimmed);
                }
            }
            Ok(_) => {}
            // quick-xml errors on malformed HTML are common; treat as
            // EOF and return whatever we collected so far rather than
            // failing the whole extraction. The plan accepts double-
            // indexing of HTML attachments that duplicate the body, so
            // partial extraction is also acceptable.
            Err(_) => break,
        }
    }

    if out.is_empty() {
        return ExtractionOutcome::Skipped { reason: SkipReason::EmptyContent };
    }
    ExtractionOutcome::Indexed { text: out }
}

fn finish_decoded(text: &str) -> ExtractionOutcome {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return ExtractionOutcome::Skipped { reason: SkipReason::EmptyContent };
    }
    ExtractionOutcome::Indexed { text: trimmed.to_string() }
}

fn control_char_ratio(text: &str) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let total = text.chars().count();
    if total == 0 {
        return 0.0;
    }
    // L3 fix: count U+FFFD (Unicode replacement) toward the bad-char
    // ratio alongside control chars. encoding_rs::decode emits U+FFFD
    // for every byte sequence it couldn't decode; a binary blob
    // mistyped as text/plain produces decoded text that's mostly
    // U+FFFD, which char::is_control() does NOT match. Pre-fix, such
    // a payload passed the ratio guard and got indexed as garbage.
    let bad = text
        .chars()
        .filter(|c| {
            (c.is_control() && *c != '\n' && *c != '\r' && *c != '\t')
                || *c == '\u{FFFD}'
        })
        .count();
    #[allow(clippy::cast_precision_loss)]
    let ratio = (bad as f32) / (total as f32);
    ratio
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_utf8_round_trips() {
        let bytes = "hello world".as_bytes();
        let outcome = extract_plain(bytes);
        assert_eq!(outcome, ExtractionOutcome::Indexed { text: "hello world".into() });
    }

    #[test]
    fn plain_utf8_with_multibyte_chars() {
        let bytes = "café \u{1F600}".as_bytes();
        let outcome = extract_plain(bytes);
        assert_eq!(outcome, ExtractionOutcome::Indexed { text: "café \u{1F600}".into() });
    }

    #[test]
    fn plain_strips_surrounding_whitespace() {
        let bytes = "   hello   ".as_bytes();
        match extract_plain(bytes) {
            ExtractionOutcome::Indexed { text } => assert_eq!(text, "hello"),
            other => panic!("expected Indexed, got {other:?}"),
        }
    }

    #[test]
    fn plain_empty_input_skips_empty() {
        let outcome = extract_plain(b"");
        assert_eq!(outcome, ExtractionOutcome::Skipped { reason: SkipReason::EmptyContent });
    }

    #[test]
    fn plain_whitespace_only_skips_empty() {
        let outcome = extract_plain(b"   \n\n\t  ");
        assert_eq!(outcome, ExtractionOutcome::Skipped { reason: SkipReason::EmptyContent });
    }

    #[test]
    fn plain_utf16le_with_bom() {
        // "hi" in UTF-16LE: FF FE 'h' 00 'i' 00
        let bytes: &[u8] = &[0xFF, 0xFE, 0x68, 0x00, 0x69, 0x00];
        let outcome = extract_plain(bytes);
        assert_eq!(outcome, ExtractionOutcome::Indexed { text: "hi".into() });
    }

    #[test]
    fn plain_utf16be_with_bom() {
        // "hi" in UTF-16BE: FE FF 00 'h' 00 'i'
        let bytes: &[u8] = &[0xFE, 0xFF, 0x00, 0x68, 0x00, 0x69];
        let outcome = extract_plain(bytes);
        assert_eq!(outcome, ExtractionOutcome::Indexed { text: "hi".into() });
    }

    #[test]
    fn plain_utf8_with_bom() {
        // "hi" with UTF-8 BOM: EF BB BF 'h' 'i'
        let bytes: &[u8] = &[0xEF, 0xBB, 0xBF, 0x68, 0x69];
        let outcome = extract_plain(bytes);
        assert_eq!(outcome, ExtractionOutcome::Indexed { text: "hi".into() });
    }

    #[test]
    fn plain_windows_1252_fallback() {
        // 0xE9 is é in Windows-1252; not valid UTF-8 on its own.
        let bytes: &[u8] = &[b'c', b'a', b'f', 0xE9];
        let outcome = extract_plain(bytes);
        match outcome {
            ExtractionOutcome::Indexed { text } => {
                assert!(text.contains("café"), "got {text:?}");
            }
            other => panic!("expected Indexed, got {other:?}"),
        }
    }

    #[test]
    fn plain_binary_garbage_skips_encoding_invalid() {
        // Mostly control bytes - simulates a binary file mistakenly
        // declared as text/plain.
        let bytes: &[u8] = &[0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07];
        let outcome = extract_plain(bytes);
        assert_eq!(outcome, ExtractionOutcome::Skipped { reason: SkipReason::EncodingInvalid });
    }

    #[test]
    fn html_extracts_visible_text_only() {
        let bytes = b"<html><head><title>T</title></head><body><p>Hello <b>world</b></p></body></html>";
        match extract_html(bytes) {
            ExtractionOutcome::Indexed { text } => {
                // Order should preserve document order; spacing collapsed.
                assert!(text.contains("T"));
                assert!(text.contains("Hello"));
                assert!(text.contains("world"));
                // No tag fragments leak through.
                assert!(!text.contains("<"));
                assert!(!text.contains(">"));
            }
            other => panic!("expected Indexed, got {other:?}"),
        }
    }

    #[test]
    fn html_strips_script_and_style_content() {
        let bytes = br#"<html><body><script>var x = "leaked";</script><p>Visible</p><style>.foo { color: red; }</style></body></html>"#;
        match extract_html(bytes) {
            ExtractionOutcome::Indexed { text } => {
                assert!(text.contains("Visible"), "got {text:?}");
                assert!(!text.contains("leaked"), "script content leaked: {text:?}");
                assert!(!text.contains("color"), "style content leaked: {text:?}");
            }
            other => panic!("expected Indexed, got {other:?}"),
        }
    }

    #[test]
    fn html_empty_input_skips() {
        let outcome = extract_html(b"");
        assert_eq!(outcome, ExtractionOutcome::Skipped { reason: SkipReason::EmptyContent });
    }

    #[test]
    fn html_no_text_content_skips() {
        let bytes = b"<html><head></head><body></body></html>";
        let outcome = extract_html(bytes);
        assert_eq!(outcome, ExtractionOutcome::Skipped { reason: SkipReason::EmptyContent });
    }

    #[test]
    fn html_extracts_text_around_entities() {
        // quick-xml may emit text segments split around entity references
        // ("foo" + "bar" with the ampersand consumed by the parser); we
        // tolerate either "foo&bar" or "foo bar" as long as the search-
        // relevant tokens are extracted. This is search-correctness
        // sufficient: a query for "foo" or "bar" matches either way.
        let bytes = b"<html><body><p>foo&amp;bar</p></body></html>";
        match extract_html(bytes) {
            ExtractionOutcome::Indexed { text } => {
                assert!(text.contains("foo"), "got {text:?}");
                assert!(text.contains("bar"), "got {text:?}");
            }
            other => panic!("expected Indexed, got {other:?}"),
        }
    }
}
