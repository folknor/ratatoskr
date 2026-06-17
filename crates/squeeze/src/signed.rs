//! Detect digital signatures in attachment bytes.
//!
//! Re-packing a signed PDF, OOXML, or ODF document is byte-changing
//! (lopdf rewrites object streams; ZIP re-pack changes deflate output),
//! which invalidates the signature. This module sniffs for signature
//! markers before compression so signed content passes through unchanged.
//!
//! Detection is biased toward false positives: a false positive skips
//! compression on an unsigned doc (harmless); a false negative re-packs
//! a signed doc and silently breaks verification (high-cost, low-frequency,
//! only surfaces during compliance audits or court evidence). Any
//! signature-shaped marker wins the bypass.
//!
//! S/MIME envelopes (`application/pkcs7-mime`, `application/pkcs7-signature`)
//! are not handled here because they already fall through as
//! `Format::Unsupported` and pass through unchanged via the main dispatch.

use crate::detect::Format;

/// Which signature marker triggered the bypass. Returned for logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignedMarker {
    /// PDF `/Type /Sig` or `/Type/Sig` token.
    PdfTypeSig,
    /// PDF `/ByteRange[` token (signature byte-range covers the document).
    PdfByteRange,
    /// PDF `/Type /DocTimeStamp` token (RFC 3161 timestamp signature).
    PdfDocTimeStamp,
    /// OOXML ZIP entry under `_xmlsignatures/`.
    OoxmlXmlSignatures,
    /// ODF ZIP entry `META-INF/documentsignatures.xml` (or `macrosignatures.xml`).
    OdfDocumentSignatures,
}

/// Return the first signature marker detected, or `None` if the bytes
/// don't look signed for the given format.
///
/// Only formats we re-pack (PDF, OOXML, ODF) are inspected. Other formats
/// return `None` without scanning.
#[must_use]
pub fn detect_signature(format: Format, data: &[u8]) -> Option<SignedMarker> {
    match format {
        Format::Pdf => detect_pdf_signature(data),
        Format::Ooxml(_) | Format::Odf(_) => detect_archive_signature(data),
        _ => None,
    }
}

/// Byte-scan a PDF for signature markers.
///
/// PDF tokens are whitespace-delimited but the syntax allows any of
/// space / tab / CR / LF / FF / NUL between tokens, and `/Name/Name`
/// is also valid (no whitespace). We check both forms.
fn detect_pdf_signature(data: &[u8]) -> Option<SignedMarker> {
    if contains_subsequence(data, b"/ByteRange[") || contains_subsequence(data, b"/ByteRange ") {
        return Some(SignedMarker::PdfByteRange);
    }
    if contains_subsequence(data, b"/Type /Sig")
        || contains_subsequence(data, b"/Type/Sig")
        || contains_subsequence(data, b"/Type\n/Sig")
        || contains_subsequence(data, b"/Type\r/Sig")
        || contains_subsequence(data, b"/Type\t/Sig")
    {
        if contains_subsequence(data, b"DocTimeStamp") {
            return Some(SignedMarker::PdfDocTimeStamp);
        }
        return Some(SignedMarker::PdfTypeSig);
    }
    None
}

/// Peek inside a ZIP-based archive (OOXML or ODF) for signature entries.
///
/// Reads the central directory only - no entry decompression. Malformed
/// or non-ZIP input returns `None` (caller falls through to normal compression,
/// which will error or pass through on its own).
fn detect_archive_signature(data: &[u8]) -> Option<SignedMarker> {
    let cursor = std::io::Cursor::new(data);
    let mut archive = zip::read::ZipArchive::new(cursor).ok()?;
    for i in 0..archive.len() {
        let Ok(entry) = archive.by_index_raw(i) else {
            continue;
        };
        let name = entry.name();
        if name.starts_with("_xmlsignatures/") {
            return Some(SignedMarker::OoxmlXmlSignatures);
        }
        if name == "META-INF/documentsignatures.xml" || name == "META-INF/macrosignatures.xml" {
            return Some(SignedMarker::OdfDocumentSignatures);
        }
    }
    None
}

/// Naive substring search. Fine for our sizes - PDFs are scanned once
/// per compress call, and the markers are short.
fn contains_subsequence(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::{OdfKind, OoxmlKind};

    #[test]
    fn pdf_with_byterange_is_signed() {
        let pdf = b"%PDF-1.7\n... /ByteRange[0 100 200 300] ...";
        assert_eq!(
            detect_signature(Format::Pdf, pdf),
            Some(SignedMarker::PdfByteRange)
        );
    }

    #[test]
    fn pdf_with_type_sig_is_signed() {
        let pdf = b"%PDF-1.7\n<< /Type /Sig /Filter /Adobe.PPKLite >>";
        assert_eq!(
            detect_signature(Format::Pdf, pdf),
            Some(SignedMarker::PdfTypeSig)
        );
    }

    #[test]
    fn pdf_with_type_sig_no_space_is_signed() {
        let pdf = b"%PDF-1.7\n<</Type/Sig/Filter/Adobe.PPKLite>>";
        assert_eq!(
            detect_signature(Format::Pdf, pdf),
            Some(SignedMarker::PdfTypeSig)
        );
    }

    #[test]
    fn pdf_with_doctimestamp_is_signed() {
        let pdf = b"%PDF-1.7\n<< /Type /Sig /SubFilter /ETSI.RFC3161 /DocTimeStamp >>";
        assert_eq!(
            detect_signature(Format::Pdf, pdf),
            Some(SignedMarker::PdfDocTimeStamp)
        );
    }

    #[test]
    fn plain_pdf_is_not_signed() {
        let pdf = b"%PDF-1.4\n<< /Type /Catalog /Pages 1 0 R >>\n";
        assert_eq!(detect_signature(Format::Pdf, pdf), None);
    }

    #[test]
    fn empty_pdf_bytes_do_not_panic() {
        assert_eq!(detect_signature(Format::Pdf, &[]), None);
    }

    #[test]
    fn ooxml_with_xmlsignatures_is_signed() {
        let zip = build_zip(&[
            ("[Content_Types].xml", b"<types/>"),
            ("_xmlsignatures/sig1.xml", b"<sig/>"),
            ("word/document.xml", b"<doc/>"),
        ]);
        assert_eq!(
            detect_signature(Format::Ooxml(OoxmlKind::Docx), &zip),
            Some(SignedMarker::OoxmlXmlSignatures)
        );
    }

    #[test]
    fn ooxml_without_signatures_is_not_signed() {
        let zip = build_zip(&[
            ("[Content_Types].xml", b"<types/>"),
            ("word/document.xml", b"<doc/>"),
        ]);
        assert_eq!(detect_signature(Format::Ooxml(OoxmlKind::Docx), &zip), None);
    }

    #[test]
    fn odf_with_documentsignatures_is_signed() {
        let zip = build_zip(&[
            ("mimetype", b"application/vnd.oasis.opendocument.text"),
            ("META-INF/documentsignatures.xml", b"<sig/>"),
            ("content.xml", b"<doc/>"),
        ]);
        assert_eq!(
            detect_signature(Format::Odf(OdfKind::Odt), &zip),
            Some(SignedMarker::OdfDocumentSignatures)
        );
    }

    #[test]
    fn odf_with_macrosignatures_is_signed() {
        let zip = build_zip(&[
            ("mimetype", b"application/vnd.oasis.opendocument.text"),
            ("META-INF/macrosignatures.xml", b"<sig/>"),
        ]);
        assert_eq!(
            detect_signature(Format::Odf(OdfKind::Odt), &zip),
            Some(SignedMarker::OdfDocumentSignatures)
        );
    }

    #[test]
    fn malformed_zip_does_not_panic() {
        let garbage = b"this is not a zip file at all";
        assert_eq!(
            detect_signature(Format::Ooxml(OoxmlKind::Docx), garbage),
            None
        );
    }

    #[test]
    fn empty_archive_bytes_do_not_panic() {
        assert_eq!(detect_signature(Format::Ooxml(OoxmlKind::Docx), &[]), None);
    }

    #[test]
    fn non_pdf_non_archive_formats_return_none() {
        let signed_looking = b"<< /Type /Sig /ByteRange[0 1 2 3] >>";
        assert_eq!(detect_signature(Format::Jpeg, signed_looking), None);
        assert_eq!(detect_signature(Format::Png, signed_looking), None);
        assert_eq!(detect_signature(Format::Svg, signed_looking), None);
        assert_eq!(detect_signature(Format::Unsupported, signed_looking), None);
    }

    fn build_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write;
        let mut buf = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut buf);
            let mut writer = zip::ZipWriter::new(cursor);
            let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (name, data) in entries {
                writer.start_file(*name, opts).expect("start_file");
                writer.write_all(data).expect("write_all");
            }
            writer.finish().expect("finish");
        }
        buf
    }
}
