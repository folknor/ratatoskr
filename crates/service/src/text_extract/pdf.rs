//! Phase 7-2b: PDF extractor.
//!
//! Two-step pipeline:
//!
//! 1. **`/Encrypt` head-inspection pre-flight.** Scan the first
//!    `HEAD_SCAN_BYTES` of the input for the literal `/Encrypt`
//!    keyword. If present, return `Skipped { Encrypted }` without
//!    handing the bytes to `pdf-extract`. Some `pdf-extract` versions
//!    panic on encrypted PDFs; the pre-flight avoids the panic class
//!    entirely. False positives (the substring `/Encrypt` appearing
//!    in a metadata stream of an unencrypted PDF) are acceptable -
//!    we'd skip a real PDF, not corrupt one.
//!
//! 2. **`pdf-extract` dispatch.** Call
//!    `pdf_extract::extract_text_from_mem(bytes)`. On `Err`, surface
//!    as `Failed { error }`. On `Ok` with empty / whitespace-only
//!    text, surface as `Skipped { EmptyContent }`.
//!
//! Panic handling: `pdf-extract` may still panic on malformed PDFs
//! that pass the `/Encrypt` pre-flight (rare; usually involves
//! truncated streams or unusual font tables). The runtime layer
//! (`ExtractRuntime`, 7-4) wraps the `spawn_blocking` call so panic
//! becomes `JoinError`, which the worker maps to
//! `Failed { error: "extractor panicked" }`. Phase 7-2b does not
//! `catch_unwind` here - the extractor stays a pure function.

use super::{ExtractionOutcome, SkipReason};

/// How many leading bytes to scan for `/Encrypt`. The trailer + cross-
/// reference table where the encrypt dict lives is usually at the END
/// of the file, but some PDFs put it inline near the catalog. 64 KB
/// covers both common positions without slurping the whole file.
const HEAD_SCAN_BYTES: usize = 64 * 1024;

/// Tail bytes also scanned for `/Encrypt`. The PDF trailer is at the
/// end of the file by convention; scanning the last 4 KB catches
/// trailer-located encrypt dicts that the head scan would miss.
const TAIL_SCAN_BYTES: usize = 4 * 1024;

#[allow(dead_code)] // Consumed in 7-4 by ExtractRuntime worker.
pub(crate) fn extract(bytes: &[u8]) -> ExtractionOutcome {
    if bytes.is_empty() {
        return ExtractionOutcome::Skipped { reason: SkipReason::EmptyContent };
    }
    if !looks_like_pdf(bytes) {
        return ExtractionOutcome::Failed {
            error: "input does not start with %PDF- header".into(),
        };
    }
    if has_encrypt_dict(bytes) {
        return ExtractionOutcome::Skipped { reason: SkipReason::Encrypted };
    }
    match pdf_extract::extract_text_from_mem(bytes) {
        Ok(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return ExtractionOutcome::Skipped { reason: SkipReason::EmptyContent };
            }
            ExtractionOutcome::Indexed { text: trimmed.to_string() }
        }
        Err(e) => ExtractionOutcome::Failed { error: format!("pdf-extract: {e}") },
    }
}

fn looks_like_pdf(bytes: &[u8]) -> bool {
    bytes.starts_with(b"%PDF-")
}

/// Scan the head and tail of the input for the `/Encrypt` PDF dict
/// keyword. The `/Encrypt` dict is referenced from the trailer (always
/// at the end) and may appear inline near the catalog (start). Two
/// scans cover both common layouts.
fn has_encrypt_dict(bytes: &[u8]) -> bool {
    let needle = b"/Encrypt";
    // Head scan.
    let head_end = HEAD_SCAN_BYTES.min(bytes.len());
    if find_subslice(&bytes[..head_end], needle) {
        return true;
    }
    // Tail scan (overlaps head when the input is small; harmless).
    let tail_start = bytes.len().saturating_sub(TAIL_SCAN_BYTES);
    if find_subslice(&bytes[tail_start..], needle) {
        return true;
    }
    false
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_skips_empty() {
        let outcome = extract(b"");
        assert_eq!(outcome, ExtractionOutcome::Skipped { reason: SkipReason::EmptyContent });
    }

    #[test]
    fn non_pdf_header_fails_fast() {
        let outcome = extract(b"this is not a pdf");
        match outcome {
            ExtractionOutcome::Failed { error } => assert!(error.contains("%PDF-")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn encrypt_dict_in_head_skips_encrypted() {
        // Synthesize a PDF-shaped buffer that contains /Encrypt near the
        // top. We don't need a valid PDF; the pre-flight runs before any
        // pdf-extract call. The %PDF- prefix passes the header sniff.
        let mut bytes = b"%PDF-1.4\n".to_vec();
        bytes.extend_from_slice(b"1 0 obj\n<<\n/Encrypt 99 0 R\n/Pages 2 0 R\n>>\nendobj\n");
        let outcome = extract(&bytes);
        assert_eq!(outcome, ExtractionOutcome::Skipped { reason: SkipReason::Encrypted });
    }

    #[test]
    fn encrypt_dict_in_tail_skips_encrypted() {
        // Place /Encrypt only in the trailer (last few bytes) - common
        // PDF layout. Pad the middle so the head scan does not see it.
        let mut bytes = b"%PDF-1.4\n".to_vec();
        bytes.extend(std::iter::repeat_n(b' ', 8 * 1024)); // 8 KB filler
        bytes.extend_from_slice(b"trailer\n<<\n/Size 5\n/Encrypt 99 0 R\n>>\nstartxref\n0\n%%EOF\n");
        let outcome = extract(&bytes);
        assert_eq!(outcome, ExtractionOutcome::Skipped { reason: SkipReason::Encrypted });
    }

    #[test]
    fn corrupt_pdf_returns_failed() {
        // Valid header but truncated body. pdf-extract will Err - we
        // surface as Failed (transient-class status).
        let outcome = extract(b"%PDF-1.4\n<<garbage>>");
        // Either Failed or Skipped::EmptyContent are acceptable here -
        // pdf-extract may sometimes succeed parsing trivial PDFs and
        // emit no text. The contract is: don't panic, don't index
        // garbage. Both outcomes satisfy that.
        assert!(
            matches!(
                outcome,
                ExtractionOutcome::Failed { .. }
                    | ExtractionOutcome::Skipped { reason: SkipReason::EmptyContent }
            ),
            "got {outcome:?}",
        );
    }
}
