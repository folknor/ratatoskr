//! Phase 7-2c: OOXML extractor (.docx / .xlsx / .pptx).
//!
//! Each format is an OPC zip archive containing XML parts. Common
//! shape:
//!
//! - **docx**: text lives in `word/document.xml` inside `<w:t>` runs.
//! - **xlsx**: shared-string table at `xl/sharedStrings.xml` (`<si><t>`)
//!   plus inline-string cells inside `xl/worksheets/sheet*.xml`.
//! - **pptx**: per-slide text at `ppt/slides/slide*.xml` inside `<a:t>`.
//!
//! Defenses:
//!
//! 1. **Decompressed-size cap**: sum the central-directory's claimed
//!    uncompressed sizes. If the sum exceeds `2 * MAX_INPUT_BYTES` we
//!    return `Skipped { ZipBomb }` without ever reading the payload.
//!    A truly malicious archive whose CD lies about sizes is also
//!    bounded by `Read::take(MAX_TOTAL_DECOMPRESSED)` on each entry
//!    (catches archives that claim small sizes but actually expand
//!    huge).
//! 2. **`quick-xml` entity policy**: same explicit `config_mut()`
//!    settings as `plain.rs::extract_html`. quick-xml has no DTD
//!    support so external-entity resolution is structurally absent;
//!    we set the explicit knobs for future-proofing.
//!
//! Zip-format edge cases:
//!
//! - Encrypted entries: the `zip` crate's `by_name()` returns Err on
//!   encrypted streams; surfaced as `Failed`.
//! - Missing expected entries (no `word/document.xml` in a `.docx`):
//!   `Skipped { EmptyContent }`. The archive opens fine but produces
//!   nothing extractable.

use std::io::{Cursor, Read};

use quick_xml::Reader;
use quick_xml::events::Event;

use super::{ExtractionOutcome, MAX_INPUT_BYTES, SkipReason};

/// Total decompressed bytes ceiling across all entries we read.
/// Above this, `Skipped { ZipBomb }`. Set to 2 * MAX_INPUT_BYTES so
/// an honest 50 MB docx with text + a few embedded images decompresses
/// to ~100 MB which is plausible.
const MAX_TOTAL_DECOMPRESSED: u64 = (MAX_INPUT_BYTES as u64) * 2;

#[allow(dead_code)] // Consumed in 7-4 by the worker dispatch.
pub(crate) fn extract_docx(bytes: &[u8]) -> ExtractionOutcome {
    let mut archive = match open_with_size_check(bytes) {
        Ok(a) => a,
        Err(o) => return o,
    };
    match collect_text_from_entry(&mut archive, "word/document.xml", &[b"t"]) {
        Ok(out) if out.text.trim().is_empty() => ExtractionOutcome::Skipped {
            reason: SkipReason::EmptyContent,
        },
        Ok(out) => ExtractionOutcome::Indexed { text: out.text },
        Err(EntryError::NotFound) => ExtractionOutcome::Skipped {
            reason: SkipReason::EmptyContent,
        },
        Err(EntryError::Read(e)) => ExtractionOutcome::Failed { error: e },
        Err(EntryError::ZipBomb) => ExtractionOutcome::Skipped {
            reason: SkipReason::ZipBomb,
        },
    }
}

#[allow(dead_code)] // Consumed in 7-4.
pub(crate) fn extract_xlsx(bytes: &[u8]) -> ExtractionOutcome {
    let mut archive = match open_with_size_check(bytes) {
        Ok(a) => a,
        Err(o) => return o,
    };

    let mut combined = String::new();
    let mut budget = MAX_TOTAL_DECOMPRESSED;

    // Shared strings: <si><t>...</t></si>. Only emitted if the file
    // actually has shared strings (small workbooks may inline-string
    // everything).
    match collect_text_from_entry(&mut archive, "xl/sharedStrings.xml", &[b"t"]) {
        Ok(out) => {
            push_with_separator(&mut combined, &out.text);
            // H3 fix: deduct decompressed bytes, not text length.
            budget = budget.saturating_sub(out.decompressed_bytes);
        }
        Err(EntryError::NotFound) => {}
        Err(EntryError::Read(e)) => return ExtractionOutcome::Failed { error: e },
        Err(EntryError::ZipBomb) => {
            return ExtractionOutcome::Skipped {
                reason: SkipReason::ZipBomb,
            };
        }
    }

    // Per-sheet inline strings + shared-string indices we miss. Walk
    // every xl/worksheets/sheet*.xml and extract <t> content.
    let sheet_names: Vec<String> = archive
        .file_names()
        .filter(|n| n.starts_with("xl/worksheets/sheet") && n.ends_with(".xml"))
        .map(str::to_string)
        .collect();

    for name in sheet_names {
        if budget == 0 {
            return ExtractionOutcome::Skipped {
                reason: SkipReason::ZipBomb,
            };
        }
        match collect_text_from_entry_bounded(&mut archive, &name, &[b"t"], budget) {
            Ok(out) => {
                budget = budget.saturating_sub(out.decompressed_bytes);
                push_with_separator(&mut combined, &out.text);
            }
            Err(EntryError::NotFound) => {}
            Err(EntryError::Read(e)) => return ExtractionOutcome::Failed { error: e },
            Err(EntryError::ZipBomb) => {
                return ExtractionOutcome::Skipped {
                    reason: SkipReason::ZipBomb,
                };
            }
        }
    }

    if combined.trim().is_empty() {
        return ExtractionOutcome::Skipped {
            reason: SkipReason::EmptyContent,
        };
    }
    ExtractionOutcome::Indexed { text: combined }
}

#[allow(dead_code)] // Consumed in 7-4.
pub(crate) fn extract_pptx(bytes: &[u8]) -> ExtractionOutcome {
    let mut archive = match open_with_size_check(bytes) {
        Ok(a) => a,
        Err(o) => return o,
    };

    let slide_names: Vec<String> = archive
        .file_names()
        .filter(|n| n.starts_with("ppt/slides/slide") && n.ends_with(".xml"))
        .map(str::to_string)
        .collect();

    if slide_names.is_empty() {
        return ExtractionOutcome::Skipped {
            reason: SkipReason::EmptyContent,
        };
    }

    let mut combined = String::new();
    let mut budget = MAX_TOTAL_DECOMPRESSED;
    for name in slide_names {
        if budget == 0 {
            return ExtractionOutcome::Skipped {
                reason: SkipReason::ZipBomb,
            };
        }
        match collect_text_from_entry_bounded(&mut archive, &name, &[b"t"], budget) {
            Ok(out) => {
                budget = budget.saturating_sub(out.decompressed_bytes);
                push_with_separator(&mut combined, &out.text);
            }
            Err(EntryError::NotFound) => {}
            Err(EntryError::Read(e)) => return ExtractionOutcome::Failed { error: e },
            Err(EntryError::ZipBomb) => {
                return ExtractionOutcome::Skipped {
                    reason: SkipReason::ZipBomb,
                };
            }
        }
    }

    if combined.trim().is_empty() {
        return ExtractionOutcome::Skipped {
            reason: SkipReason::EmptyContent,
        };
    }
    ExtractionOutcome::Indexed { text: combined }
}

enum EntryError {
    NotFound,
    Read(String),
    ZipBomb,
}

/// Open an OOXML archive after a sanity-check on the central
/// directory's declared total uncompressed size. Catches archives
/// that openly admit to being huge before we read any payload bytes.
fn open_with_size_check(bytes: &[u8]) -> Result<zip::ZipArchive<Cursor<&[u8]>>, ExtractionOutcome> {
    let cursor = Cursor::new(bytes);
    let mut archive = match zip::ZipArchive::new(cursor) {
        Ok(a) => a,
        Err(e) => {
            return Err(ExtractionOutcome::Failed {
                error: format!("zip open: {e}"),
            });
        }
    };

    // Sum claimed uncompressed sizes via the central directory.
    //
    // H5 fix: pre-fix the loop did `let mut clone_archive =
    // archive.clone(); clone_archive.by_index(i).ok().map(|f|
    // f.size())` per entry - O(n^2) memory churn copying the per-entry
    // metadata vec for each peek. Iterating `&mut archive` directly
    // releases its borrow at the end of each loop body, no clone
    // needed; the archive moves into the Ok return after the loop.
    //
    // H4 fix: pre-fix the loop summed via `.sum::<u64>()` - which uses
    // checked + in debug (panic, caught by the per-item supervisor as
    // a transient failure) and *wrapping* + in release. A crafted
    // OOXML with N entries each declaring u64::MAX/2 + 1 wrapped to a
    // small total in release builds and silently bypassed the bomb
    // pre-check. Manual checked_add fold returns ZipBomb on overflow.
    let mut claimed_total: u64 = 0;
    for i in 0..archive.len() {
        let entry_size = match archive.by_index(i) {
            Ok(f) => f.size(),
            Err(_) => continue,
        };
        match claimed_total.checked_add(entry_size) {
            Some(t) => claimed_total = t,
            None => {
                return Err(ExtractionOutcome::Skipped {
                    reason: SkipReason::ZipBomb,
                });
            }
        }
        if claimed_total > MAX_TOTAL_DECOMPRESSED {
            return Err(ExtractionOutcome::Skipped {
                reason: SkipReason::ZipBomb,
            });
        }
    }

    Ok(archive)
}

fn collect_text_from_entry(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    entry: &str,
    target_local_names: &[&[u8]],
) -> Result<EntryReadOutput, EntryError> {
    collect_text_from_entry_bounded(archive, entry, target_local_names, MAX_TOTAL_DECOMPRESSED)
}

/// Output of one zip entry read: extracted text plus the actual
/// decompressed byte count. H3 fix: callers decrement their bomb
/// budget by `decompressed_bytes` (i.e. `buf.len()`), not by
/// `text.len()`. The pre-fix code subtracted text length, but
/// `text.len()` is post-XML-walk - a 100 MB XML stream that produces
/// 1 KB of `<t>` text would only deduct 1 KB from the budget,
/// allowing N entries to each decompress up to MAX_TOTAL_DECOMPRESSED.
#[derive(Debug)]
struct EntryReadOutput {
    text: String,
    decompressed_bytes: u64,
}

fn collect_text_from_entry_bounded(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    entry: &str,
    target_local_names: &[&[u8]],
    byte_budget: u64,
) -> Result<EntryReadOutput, EntryError> {
    let file = match archive.by_name(entry) {
        Ok(f) => f,
        Err(zip::result::ZipError::FileNotFound) => return Err(EntryError::NotFound),
        Err(e) => return Err(EntryError::Read(format!("by_name({entry}): {e}"))),
    };

    // Hard-cap actual decompressed bytes. Prevents zip-bomb archives
    // whose CD size is small but whose stream is huge.
    let limit = byte_budget.min(MAX_TOTAL_DECOMPRESSED);
    let mut limited = file.take(limit);
    let mut buf: Vec<u8> = Vec::with_capacity(8 * 1024);
    if let Err(e) = limited.read_to_end(&mut buf) {
        return Err(EntryError::Read(format!("read {entry}: {e}")));
    }
    // H3 fix: any cap-hit is suspect. The prior `&& limit <
    // MAX_TOTAL_DECOMPRESSED` guard let a single-entry bomb through:
    // when called as the first entry with byte_budget ==
    // MAX_TOTAL_DECOMPRESSED, the limit equalled the cap and the
    // second condition was false, so a 100 MB inflation was treated
    // as legitimate content. Conservative: a stream that fills the
    // cap exactly is more likely zip-bomb than honest content.
    #[allow(clippy::cast_possible_truncation)]
    let decompressed_bytes = buf.len() as u64;
    if decompressed_bytes >= limit {
        return Err(EntryError::ZipBomb);
    }

    let text = collect_target_text(&buf, target_local_names);
    Ok(EntryReadOutput {
        text,
        decompressed_bytes,
    })
}

/// Walk the XML in `bytes`, collect text events that are inside an
/// element whose local-name is in `target_local_names`. Whitespace
/// between target elements becomes a single space in the output.
fn collect_target_text(bytes: &[u8], target_local_names: &[&[u8]]) -> String {
    let mut reader = Reader::from_reader(bytes);
    let cfg = reader.config_mut();
    cfg.expand_empty_elements = false;
    cfg.trim_text(true);
    cfg.check_end_names = false;

    let mut buf: Vec<u8> = Vec::with_capacity(256);
    let mut out = String::new();
    let mut depth_in_target: u32 = 0;

    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Ok(Event::Start(ref e)) => {
                if is_target(e.local_name().as_ref(), target_local_names) {
                    depth_in_target += 1;
                }
            }
            Ok(Event::End(ref e)) => {
                if is_target(e.local_name().as_ref(), target_local_names) {
                    depth_in_target = depth_in_target.saturating_sub(1);
                }
            }
            Ok(Event::Empty(_)) => {
                // Self-closing target element has no text.
            }
            Ok(Event::Text(ref t)) => {
                if depth_in_target == 0 {
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
            Ok(_) => {}
            // Malformed XML: stop and return what we have. Better a
            // partial extraction than no extraction.
            Err(_) => break,
        }
    }
    out
}

fn is_target(local: &[u8], targets: &[&[u8]]) -> bool {
    targets.contains(&local)
}

fn push_with_separator(out: &mut String, addition: &str) {
    let trimmed = addition.trim();
    if trimmed.is_empty() {
        return;
    }
    if !out.is_empty() {
        out.push(' ');
    }
    out.push_str(trimmed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    /// Build a synthetic `.docx`-shaped zip in memory. Only includes
    /// `word/document.xml` (a real `.docx` has more parts but we don't
    /// touch them).
    fn build_docx(document_xml: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let cursor = Cursor::new(&mut buf);
            let mut writer = ZipWriter::new(cursor);
            let opts =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            writer
                .start_file("word/document.xml", opts)
                .expect("start_file");
            writer.write_all(document_xml.as_bytes()).expect("write");
            writer.finish().expect("finish");
        }
        buf
    }

    fn build_pptx(slides: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let cursor = Cursor::new(&mut buf);
            let mut writer = ZipWriter::new(cursor);
            let opts =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (name, body) in slides {
                writer.start_file(*name, opts).expect("start_file");
                writer.write_all(body.as_bytes()).expect("write");
            }
            writer.finish().expect("finish");
        }
        buf
    }

    fn build_xlsx_with_shared_strings(shared: &str, sheets: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let cursor = Cursor::new(&mut buf);
            let mut writer = ZipWriter::new(cursor);
            let opts =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            writer
                .start_file("xl/sharedStrings.xml", opts)
                .expect("start_file");
            writer.write_all(shared.as_bytes()).expect("write");
            for (name, body) in sheets {
                writer.start_file(*name, opts).expect("start_file");
                writer.write_all(body.as_bytes()).expect("write");
            }
            writer.finish().expect("finish");
        }
        buf
    }

    const DOCX_MINIMAL: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:r><w:t>Hello phase seven</w:t></w:r>
      <w:r><w:t xml:space="preserve"> world</w:t></w:r>
    </w:p>
  </w:body>
</w:document>"#;

    #[test]
    fn docx_extracts_w_t_text() {
        let bytes = build_docx(DOCX_MINIMAL);
        match extract_docx(&bytes) {
            ExtractionOutcome::Indexed { text } => {
                assert!(text.contains("Hello phase seven"), "got {text:?}");
                assert!(text.contains("world"), "got {text:?}");
            }
            other => panic!("expected Indexed, got {other:?}"),
        }
    }

    #[test]
    fn docx_missing_document_xml_skips_empty() {
        // Build a zip that has no word/document.xml.
        let mut buf = Vec::new();
        {
            let cursor = Cursor::new(&mut buf);
            let mut writer = ZipWriter::new(cursor);
            let opts = SimpleFileOptions::default();
            writer
                .start_file("other/file.xml", opts)
                .expect("start_file");
            writer.write_all(b"<root/>").expect("write");
            writer.finish().expect("finish");
        }
        let outcome = extract_docx(&buf);
        assert_eq!(
            outcome,
            ExtractionOutcome::Skipped {
                reason: SkipReason::EmptyContent
            }
        );
    }

    #[test]
    fn docx_invalid_zip_fails() {
        let outcome = extract_docx(b"definitely not a zip");
        assert!(matches!(outcome, ExtractionOutcome::Failed { .. }));
    }

    #[test]
    fn pptx_extracts_a_t_text_across_slides() {
        let slide1 = r#"<?xml version="1.0"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:sp><p:txBody><a:p><a:r><a:t>Slide one title</a:t></a:r></a:p></p:txBody></p:sp>
  </p:spTree></p:cSld>
</p:sld>"#;
        let slide2 = r#"<?xml version="1.0"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:sp><p:txBody><a:p><a:r><a:t>Second slide content</a:t></a:r></a:p></p:txBody></p:sp>
  </p:spTree></p:cSld>
</p:sld>"#;
        let bytes = build_pptx(&[
            ("ppt/slides/slide1.xml", slide1),
            ("ppt/slides/slide2.xml", slide2),
        ]);
        match extract_pptx(&bytes) {
            ExtractionOutcome::Indexed { text } => {
                assert!(text.contains("Slide one title"), "got {text:?}");
                assert!(text.contains("Second slide content"), "got {text:?}");
            }
            other => panic!("expected Indexed, got {other:?}"),
        }
    }

    #[test]
    fn pptx_no_slides_skips_empty() {
        let bytes = build_pptx(&[]);
        let outcome = extract_pptx(&bytes);
        assert_eq!(
            outcome,
            ExtractionOutcome::Skipped {
                reason: SkipReason::EmptyContent
            }
        );
    }

    #[test]
    fn xlsx_extracts_shared_strings_and_sheet_inline() {
        let shared = r#"<?xml version="1.0"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <si><t>Apple</t></si>
  <si><t>Banana</t></si>
</sst>"#;
        let sheet1 = r#"<?xml version="1.0"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row><c t="inlineStr"><is><t>Inline value</t></is></c></row>
  </sheetData>
</worksheet>"#;
        let bytes = build_xlsx_with_shared_strings(shared, &[("xl/worksheets/sheet1.xml", sheet1)]);
        match extract_xlsx(&bytes) {
            ExtractionOutcome::Indexed { text } => {
                assert!(text.contains("Apple"), "got {text:?}");
                assert!(text.contains("Banana"), "got {text:?}");
                assert!(text.contains("Inline value"), "got {text:?}");
            }
            other => panic!("expected Indexed, got {other:?}"),
        }
    }

    #[test]
    fn empty_w_t_skips_empty_content() {
        let body = r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body><w:p><w:r><w:t></w:t></w:r></w:p></w:body>
</w:document>"#;
        let bytes = build_docx(body);
        let outcome = extract_docx(&bytes);
        assert_eq!(
            outcome,
            ExtractionOutcome::Skipped {
                reason: SkipReason::EmptyContent
            }
        );
    }
}
