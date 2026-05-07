//! Phase 7: per-mime text extraction from cached attachment bytes.
//!
//! Pure functions; no I/O; no async. The Service-side `ExtractRuntime`
//! (lands in 7-4) owns reading bytes from `<app_data>/attachment_cache/`
//! and persisting results into `attachment_extracted_text`. Extractors
//! receive the bytes directly and return a tagged `ExtractionOutcome`.
//!
//! All extractors run inside `tokio::task::spawn_blocking` from the
//! runtime's perspective, but the functions themselves are sync because
//! pdf-extract / quick-xml / encoding_rs are sync libraries.
//!
//! ## Cap policy (two distinct caps, deliberately named separately)
//!
//! - `MAX_INPUT_BYTES` (50 MB): skip BEFORE invoking the underlying
//!   extractor. Bytes are never read into the extractor's working set
//!   beyond this. Outcome: `Skipped { reason: OversizeFile }`.
//!
//! - `MAX_EXTRACTED_TEXT_BYTES` (100 KB): truncate AFTER the extractor
//!   returns text. Truncation lands on a UTF-8 character boundary via
//!   `floor_char_boundary`-shaped logic; naive byte slicing would panic
//!   on multi-byte input. Truncated text gets a ` ... [truncated]`
//!   marker.
//!
//! ## Skip taxonomy
//!
//! Status strings persisted to `attachment_extracted_text.status` map
//! 1:1 to `SkipReason` variants here. The split between "permanent"
//! (no retry on next enqueue) and "retry-eligible" lives in
//! `crates/db/src/db/schema/02_mail.sql`'s schema doc-comment and is
//! enforced by the worker pre-flight check in 7-4. Permanent reasons
//! reflect a property of the bytes themselves (encryption, oversize,
//! opaque mime); retry-eligible reasons reflect a transient runtime
//! condition (timeout, bytes_gone, transient I/O).
//!
//! ## Sub-modules
//!
//! - `plain` (7-2a): `text/plain` / `text/csv` / `text/markdown` via
//!   `encoding_rs` BOM + heuristic sniff; `text/html` stripped to text.
//! - `pdf` (7-2b, lands separately): `/Encrypt` head-inspection pre-
//!   flight + `pdf-extract` dispatch.
//! - `ooxml` (7-2c, lands separately): `zip` walk + `quick-xml` text
//!   extraction with decompressed-size cap and entities-off contract.

pub(crate) mod plain;

/// Cap on input bytes. Files larger than this skip extraction entirely.
pub(crate) const MAX_INPUT_BYTES: usize = 50 * 1024 * 1024;

/// Cap on extracted text post-extraction. Truncate to this many bytes
/// on a UTF-8 char boundary. Bounds DB row size + Tantivy heap pressure.
pub(crate) const MAX_EXTRACTED_TEXT_BYTES: usize = 100 * 1024;

/// Per-extraction wallclock cap. The runtime layer (`ExtractRuntime`,
/// 7-4) wraps the spawn_blocking call in `tokio::time::timeout(...)`;
/// extractors themselves do not enforce this. Constant lives here so the
/// worker and the doc-comment have one source of truth.
#[allow(dead_code)] // Consumed in 7-4 when ExtractRuntime lands.
pub(crate) const PER_EXTRACTION_TIMEOUT_SECS: u64 = 30;

/// Outcome of an extraction attempt. Maps directly to the persisted
/// `attachment_extracted_text.status` taxonomy.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Consumed in 7-4.
pub(crate) enum ExtractionOutcome {
    Indexed { text: String },
    Skipped { reason: SkipReason },
    Failed { error: String },
}

/// Reason an extraction was skipped. The `Permanent` group never retries
/// on a re-enqueue; the `Retry` group re-extracts on next enqueue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Consumed in 7-4.
pub(crate) enum SkipReason {
    /// Permanent: image/audio/video/archive/executable mime or extension.
    OpaqueMime,
    /// Permanent: PDF `/Encrypt` detected pre-flight.
    Encrypted,
    /// Permanent: input bytes > MAX_INPUT_BYTES.
    OversizeFile,
    /// Retry-eligible: extractor exceeded PER_EXTRACTION_TIMEOUT_SECS.
    /// (Enforced by the runtime layer, not by the extractor itself.)
    Timeout,
    /// Permanent: text/* with no detectable encoding (invalid char
    /// ratio above threshold across UTF-8 / Windows-1252).
    EncodingInvalid,
    /// Permanent: extractor ran but produced no extractable text.
    EmptyContent,
    /// Permanent: image with no OCR backend available.
    OcrUnavailable,
    /// Retry-eligible: `attachment_cache/<hash>` ENOENT during read.
    /// Worker re-enqueues on the next attachment.fetch.
    BytesGone,
    /// Permanent: mime not in dispatch table.
    UnknownMime,
    /// Permanent: text/calendar (.ics) - privacy-relevant attendee /
    /// organizer / addresses skip-listed by policy.
    PrivacyExempt,
    /// Permanent: OOXML decompressed size > 2 * MAX_INPUT_BYTES, OR a
    /// containing zip's compression ratio crosses a sane threshold.
    ZipBomb,
}

impl SkipReason {
    /// Map to the persisted `attachment_extracted_text.status` string.
    #[allow(dead_code)] // Consumed in 7-4.
    pub(crate) fn status_string(self) -> &'static str {
        match self {
            Self::OpaqueMime      => "skipped:opaque",
            Self::Encrypted       => "skipped:encrypted",
            Self::OversizeFile    => "skipped:oversize",
            Self::Timeout         => "skipped:timeout",
            Self::EncodingInvalid => "skipped:encoding",
            Self::EmptyContent    => "skipped:empty",
            Self::OcrUnavailable  => "skipped:ocr",
            Self::BytesGone       => "skipped:bytes_gone",
            Self::UnknownMime     => "skipped:unknown_mime",
            Self::PrivacyExempt   => "skipped:privacy",
            Self::ZipBomb         => "skipped:zipbomb",
        }
    }

    /// `true` if a row with this skip status should be re-tried on a
    /// later enqueue. Worker pre-flight in 7-4 reads this.
    #[allow(dead_code)] // Consumed in 7-4.
    pub(crate) fn is_retry_eligible(self) -> bool {
        matches!(self, Self::Timeout | Self::BytesGone)
    }
}

/// Canonical mime tags. Provider-reported variants
/// (`application/pdf` / `application/x-pdf` / missing-mime + `.pdf`)
/// collapse here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mime {
    Pdf,
    Docx,
    Xlsx,
    Pptx,
    PlainText,
    Html,
    Calendar,
    Unknown,
}

/// Skip-list mimes (no bytes read into memory). Image / audio / video
/// / archive / executable. The list is generous: the cost of skipping
/// a borderline mime that COULD have extracted is much lower than the
/// cost of running the dispatch on something that will produce noise.
#[allow(dead_code)] // Consumed in 7-4.
pub(crate) fn is_opaque_by_mime_or_extension(mime: &str, filename: &str) -> bool {
    let mime_lower = mime.to_ascii_lowercase();
    if mime_lower.starts_with("image/")
        || mime_lower.starts_with("audio/")
        || mime_lower.starts_with("video/")
        || matches!(
            mime_lower.as_str(),
            "application/x-executable"
                | "application/x-msdownload"
                | "application/octet-stream"
                | "application/zip"
                | "application/x-tar"
                | "application/gzip"
                | "application/x-7z-compressed"
                | "application/x-bzip2"
        )
    {
        return true;
    }

    // Fall back to extension if the mime is generic / missing.
    if let Some(ext) = filename.rsplit('.').next() {
        let ext_lower = ext.to_ascii_lowercase();
        if matches!(
            ext_lower.as_str(),
            "exe" | "dll" | "so" | "dylib"
            | "zip" | "tar" | "gz" | "tgz" | "7z" | "bz2" | "rar"
            | "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac"
            | "mp4" | "mkv" | "mov" | "avi" | "webm"
            | "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic"
            | "tiff" | "bmp" | "ico" | "svg"
        ) {
            return true;
        }
    }
    false
}

/// Map a (mime, filename) pair to the canonical `Mime` tag. Falls
/// back to the filename extension when the mime is missing or generic.
#[allow(dead_code)] // Consumed in 7-4.
pub(crate) fn canonicalize_mime(mime: &str, filename: &str) -> Mime {
    let mime_lower = mime.to_ascii_lowercase();
    let mime_tag = match mime_lower.as_str() {
        "application/pdf" | "application/x-pdf" => Some(Mime::Pdf),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => Some(Mime::Docx),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => Some(Mime::Xlsx),
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => Some(Mime::Pptx),
        "text/plain" | "text/csv" | "text/markdown" | "text/x-markdown" => Some(Mime::PlainText),
        "text/html" | "application/xhtml+xml" => Some(Mime::Html),
        "text/calendar" | "application/ics" => Some(Mime::Calendar),
        _ => None,
    };
    if let Some(tag) = mime_tag {
        return tag;
    }
    // Fallback by extension (covers missing or generic mime).
    if let Some(ext) = filename.rsplit('.').next() {
        match ext.to_ascii_lowercase().as_str() {
            "pdf"   => return Mime::Pdf,
            "docx"  => return Mime::Docx,
            "xlsx"  => return Mime::Xlsx,
            "pptx"  => return Mime::Pptx,
            "txt" | "log" | "csv" | "md" | "markdown" => return Mime::PlainText,
            "html" | "htm" | "xhtml" => return Mime::Html,
            "ics"   => return Mime::Calendar,
            _ => {}
        }
    }
    Mime::Unknown
}

/// Extract text from `bytes` according to mime + filename. Pure
/// function; no I/O. Output, if any, is truncated to
/// `MAX_EXTRACTED_TEXT_BYTES` on a UTF-8 char boundary.
#[allow(dead_code)] // Consumed in 7-4.
pub(crate) fn extract(bytes: &[u8], mime: &str, filename: &str) -> ExtractionOutcome {
    if is_opaque_by_mime_or_extension(mime, filename) {
        return ExtractionOutcome::Skipped { reason: SkipReason::OpaqueMime };
    }
    if bytes.len() > MAX_INPUT_BYTES {
        return ExtractionOutcome::Skipped { reason: SkipReason::OversizeFile };
    }
    let outcome = match canonicalize_mime(mime, filename) {
        Mime::Pdf => {
            // 7-2b: replace with `pdf::extract(bytes)`.
            ExtractionOutcome::Failed { error: "pdf extractor not yet wired (lands in 7-2b)".into() }
        }
        Mime::Docx | Mime::Xlsx | Mime::Pptx => {
            // 7-2c: replace with `ooxml::extract_*(bytes)`.
            ExtractionOutcome::Failed { error: "ooxml extractor not yet wired (lands in 7-2c)".into() }
        }
        Mime::PlainText => plain::extract_plain(bytes),
        Mime::Html      => plain::extract_html(bytes),
        Mime::Calendar  => ExtractionOutcome::Skipped { reason: SkipReason::PrivacyExempt },
        Mime::Unknown   => ExtractionOutcome::Skipped { reason: SkipReason::UnknownMime },
    };
    match outcome {
        ExtractionOutcome::Indexed { text } => ExtractionOutcome::Indexed {
            text: truncate_on_char_boundary(text, MAX_EXTRACTED_TEXT_BYTES),
        },
        other => other,
    }
}

/// Truncate `text` to at most `max_bytes` bytes, at a UTF-8 char
/// boundary. Stable Rust does not yet expose `floor_char_boundary` on
/// `str`; the manual loop is the contract-stable equivalent.
pub(crate) fn truncate_on_char_boundary(mut text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }
    let mut cut = max_bytes;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
    text.push_str(" ... [truncated]");
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_preserves_input_when_under_cap() {
        let s = "hello".to_string();
        assert_eq!(truncate_on_char_boundary(s, 100), "hello");
    }

    #[test]
    fn truncate_lands_on_char_boundary_on_multibyte_input() {
        // 4-byte chars - cutting at byte 5 would split the second char.
        // Constant 6 bytes ("aabb" + truncation marker) is well-formed UTF-8.
        let four_byte_char = "\u{1F600}"; // grinning face emoji = 4 bytes
        let s = format!("{four_byte_char}{four_byte_char}{four_byte_char}");
        // Length is 12 bytes. Cap at 6: must land on byte 4 (one whole char).
        let out = truncate_on_char_boundary(s, 6);
        // Should be exactly one emoji + the marker.
        assert!(out.starts_with(four_byte_char));
        assert!(out.contains("[truncated]"));
        // Crucially: out must be valid UTF-8 (else String::push_str panics).
        // The test passing is itself proof.
    }

    #[test]
    fn opaque_mime_skips_image() {
        let outcome = extract(b"fake-png-bytes", "image/png", "x.png");
        assert_eq!(outcome, ExtractionOutcome::Skipped { reason: SkipReason::OpaqueMime });
    }

    #[test]
    fn opaque_mime_skips_archive_by_extension() {
        let outcome = extract(b"fake-zip-bytes", "application/octet-stream", "x.zip");
        assert_eq!(outcome, ExtractionOutcome::Skipped { reason: SkipReason::OpaqueMime });
    }

    #[test]
    fn oversize_file_skips_before_dispatch() {
        // Allocate just over the cap. Use a vec of zeros so we don't actually
        // strain memory - it stays in a single allocation.
        let big = vec![0u8; MAX_INPUT_BYTES + 1];
        let outcome = extract(&big, "application/pdf", "x.pdf");
        assert_eq!(outcome, ExtractionOutcome::Skipped { reason: SkipReason::OversizeFile });
    }

    #[test]
    fn calendar_mime_is_privacy_exempt() {
        let outcome = extract(b"BEGIN:VCALENDAR\nEND:VCALENDAR", "text/calendar", "x.ics");
        assert_eq!(outcome, ExtractionOutcome::Skipped { reason: SkipReason::PrivacyExempt });
    }

    #[test]
    fn unknown_mime_is_skipped() {
        let outcome = extract(b"...", "application/x-nobody-knows", "x.bin");
        assert_eq!(outcome, ExtractionOutcome::Skipped { reason: SkipReason::UnknownMime });
    }

    #[test]
    fn pdf_dispatches_to_stub_until_7_2b() {
        // Phase 7-2a placeholder: dispatch table maps Mime::Pdf to a
        // Failed stub; phase 7-2b replaces with the real extractor.
        let outcome = extract(b"%PDF-1.4 stub", "application/pdf", "x.pdf");
        assert!(matches!(outcome, ExtractionOutcome::Failed { .. }));
    }

    #[test]
    fn canonicalize_mime_falls_back_to_extension() {
        assert_eq!(canonicalize_mime("application/octet-stream", "x.docx"), Mime::Docx);
        assert_eq!(canonicalize_mime("", "x.pdf"), Mime::Pdf);
        assert_eq!(canonicalize_mime("application/garbage", "x"), Mime::Unknown);
    }

    #[test]
    fn skip_reason_status_string_round_trips() {
        // Each variant must produce a unique status string. A future
        // addition that forgets the new arm causes a compile error in
        // status_string (exhaustive match) AND fails this test if the
        // string collides.
        let all = [
            SkipReason::OpaqueMime,
            SkipReason::Encrypted,
            SkipReason::OversizeFile,
            SkipReason::Timeout,
            SkipReason::EncodingInvalid,
            SkipReason::EmptyContent,
            SkipReason::OcrUnavailable,
            SkipReason::BytesGone,
            SkipReason::UnknownMime,
            SkipReason::PrivacyExempt,
            SkipReason::ZipBomb,
        ];
        let strings: Vec<&str> = all.iter().map(|r| r.status_string()).collect();
        let mut deduped = strings.clone();
        deduped.sort_unstable();
        deduped.dedup();
        assert_eq!(deduped.len(), all.len(), "status strings must be unique");
        for s in &strings {
            assert!(s.starts_with("skipped:"), "got {s:?}");
        }
    }

    #[test]
    fn retry_eligible_set_matches_schema_doc() {
        // Permanent reasons: must NOT be retry-eligible.
        for reason in [
            SkipReason::OpaqueMime,
            SkipReason::Encrypted,
            SkipReason::OversizeFile,
            SkipReason::EncodingInvalid,
            SkipReason::EmptyContent,
            SkipReason::OcrUnavailable,
            SkipReason::UnknownMime,
            SkipReason::PrivacyExempt,
            SkipReason::ZipBomb,
        ] {
            assert!(!reason.is_retry_eligible(), "{reason:?} should be permanent");
        }
        // Retry-eligible reasons.
        for reason in [SkipReason::Timeout, SkipReason::BytesGone] {
            assert!(reason.is_retry_eligible(), "{reason:?} should retry");
        }
    }
}
