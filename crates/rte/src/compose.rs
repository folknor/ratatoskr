//! Compose document assembly helpers for email composition.
//!
//! Provides functions to build compose documents with optional signatures and
//! quoted content (replies/forwards). Works purely with HTML strings and
//! `Document` types — no database or app crate dependency.

use std::sync::Arc;

use crate::document::{Block, Document, InlineStyle, StyledRun};
use crate::html_parse::from_html;

// ── Types ───────────────────────────────────────────────

/// Content to quote in a reply or forward.
pub struct QuotedContent {
    /// Attribution line, e.g., "On Mar 19, 2026, Alice Smith wrote:"
    pub attribution: String,
    /// The quoted message's HTML body.
    pub body_html: String,
}

/// Result of assembling a compose document.
pub struct ComposeDocumentAssembly {
    /// The assembled document.
    pub document: Document,
    /// Block index of the signature separator (`HorizontalRule`), if a signature
    /// was inserted. Everything from this index to the attribution line (or end
    /// of document if no quoted content) is the signature region.
    pub signature_separator_index: Option<usize>,
    /// The ID of the signature that was inserted, for change detection when
    /// switching From accounts.
    pub active_signature_id: Option<String>,
}

// ── Assembly ────────────────────────────────────────────

/// Assemble a compose document with optional signature and quoted content.
///
/// The resulting document structure:
/// ```text
/// Block 0:    Empty paragraph (cursor starts here)
/// ...
/// Block N:    HorizontalRule (signature separator)
/// Block N+1:  First block of signature
/// ...
/// Block M:    Attribution line paragraph (italic)
/// Block M+1:  BlockQuote containing quoted content
/// ```
///
/// If `signature_html` is `None`, the separator and signature blocks are omitted.
/// If `quoted_content` is `None`, the attribution and blockquote are omitted.
pub fn assemble_compose_document(
    signature_html: Option<&str>,
    signature_id: Option<String>,
    quoted_content: Option<QuotedContent>,
) -> ComposeDocumentAssembly {
    let mut blocks: Vec<Block> = Vec::new();
    let mut sig_sep_index = None;
    let mut active_sig_id = None;

    // 1. Initial empty paragraph for user content.
    blocks.push(Block::empty_paragraph());

    // 2. Signature (if any, and non-blank).
    if let Some(sig_html) = signature_html
        && !is_blank_html(sig_html)
    {
        sig_sep_index = Some(blocks.len());
        blocks.push(Block::HorizontalRule);
        active_sig_id = signature_id;

        let sig_doc = from_html(sig_html);
        for block in sig_doc.blocks {
            blocks.push(Arc::unwrap_or_clone(block));
        }
    }

    // 3. Quoted content (if reply/forward).
    if let Some(quoted) = quoted_content {
        blocks.push(build_attribution_block(&quoted.attribution));
        let quoted_doc = from_html(&quoted.body_html);
        blocks.push(Block::BlockQuote {
            blocks: quoted_doc.blocks,
        });
    }

    ComposeDocumentAssembly {
        document: Document::from_blocks(blocks),
        signature_separator_index: sig_sep_index,
        active_signature_id: active_sig_id,
    }
}

// ── Helpers ──────────────────────────────────────────────

/// Check whether an HTML string is effectively blank (empty, whitespace-only,
/// or parses to a document with only empty/whitespace blocks).
///
/// A document containing an Image block is never blank, even if the alt text
/// is empty.
fn is_blank_html(html: &str) -> bool {
    if html.trim().is_empty() {
        return true;
    }
    let doc = from_html(html);
    doc.blocks.iter().all(|b| {
        // Image blocks are never blank — they represent visible content.
        if matches!(b.as_ref(), Block::Image { .. }) {
            return false;
        }
        b.flattened_text().trim().is_empty()
    })
}

// ── Attribution / forward header builders ───────────────

/// Build an attribution block from a pre-formatted attribution string.
///
/// Returns a `Block::Paragraph` with the attribution text in italic.
fn build_attribution_block(attribution: &str) -> Block {
    Block::Paragraph {
        runs: vec![StyledRun::styled(attribution, InlineStyle::ITALIC)],
    }
}

/// Build a reply attribution line, e.g., "On Mar 19, 2026, Alice Smith wrote:"
///
/// Returns a `Block::Paragraph` with the attribution text in italic.
pub fn build_reply_attribution_block(date: &str, sender_name: &str) -> Block {
    let text = format!("On {date}, {sender_name} wrote:");
    Block::Paragraph {
        runs: vec![StyledRun::styled(text, InlineStyle::ITALIC)],
    }
}

/// Build a forward header block.
///
/// Returns a `Block::Paragraph` with "---------- Forwarded message ----------"
pub fn build_forward_header() -> Block {
    Block::Paragraph {
        runs: vec![StyledRun::plain("---------- Forwarded message ----------")],
    }
}

// ── Signature manipulation ──────────────────────────────

/// Insert a signature into an existing document at the given position.
///
/// Inserts a `HorizontalRule` separator followed by the signature blocks.
/// Returns the index of the separator, or `None` if the signature HTML is
/// blank/empty.
///
/// `at_index` is clamped to the document's block count (inserting at the end
/// if out of range).
pub fn insert_signature(
    document: &mut Document,
    at_index: usize,
    signature_html: &str,
) -> Option<usize> {
    if is_blank_html(signature_html) {
        return None;
    }

    let at_index = at_index.min(document.block_count());
    let sig_doc = from_html(signature_html);

    document.insert_block(at_index, Block::HorizontalRule);

    for (i, block) in sig_doc.blocks.into_iter().enumerate() {
        let block = Arc::unwrap_or_clone(block);
        document.insert_block(at_index + 1 + i, block);
    }

    Some(at_index)
}

/// Remove a signature region from a document.
///
/// Removes blocks from `separator_index` up to (but not including)
/// `end_index`. If `end_index` is `None`, removes to the end of the document
/// (but keeps at least one block).
///
/// Out-of-range indices are clamped to the document's block count.
pub fn remove_signature(document: &mut Document, separator_index: usize, end_index: Option<usize>) {
    let count = document.block_count();
    let start = separator_index.min(count);
    let end = end_index.unwrap_or(count).min(count);

    // Remove from end to start to avoid index shifting.
    for i in (start..end).rev() {
        // `remove_block` refuses to remove the last block, which is the
        // safety valve we need.
        document.remove_block(i);
    }
}

/// Replace a signature in a document.
///
/// Removes the old signature region, then inserts the new one at the same
/// position. Returns the new separator index, or `None` if no new signature.
pub fn replace_signature(
    document: &mut Document,
    old_separator_index: usize,
    old_end_index: Option<usize>,
    new_signature_html: Option<&str>,
) -> Option<usize> {
    remove_signature(document, old_separator_index, old_end_index);

    new_signature_html
        .and_then(|sig_html| insert_signature(document, old_separator_index, sig_html))
}

// ── Tests ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Block, InlineStyle};

    #[test]
    fn assemble_no_signature_no_quoted() {
        let result = assemble_compose_document(None, None, None);
        assert_eq!(result.document.block_count(), 1);
        assert_eq!(result.document.block(0), Some(&Block::empty_paragraph()));
        assert_eq!(result.signature_separator_index, None);
    }

    #[test]
    fn assemble_with_signature_only() {
        let result =
            assemble_compose_document(Some("<p>Best regards,</p><p>Alice</p>"), None, None);
        // Block 0: empty paragraph
        // Block 1: HorizontalRule
        // Block 2: "Best regards,"
        // Block 3: "Alice"
        assert_eq!(result.document.block_count(), 4);
        assert_eq!(result.document.block(0), Some(&Block::empty_paragraph()));
        assert_eq!(result.document.block(1), Some(&Block::HorizontalRule));
        assert_eq!(result.signature_separator_index, Some(1));

        let text2 = result.document.block(2).map(Block::flattened_text);
        assert_eq!(text2.as_deref(), Some("Best regards,"));

        let text3 = result.document.block(3).map(Block::flattened_text);
        assert_eq!(text3.as_deref(), Some("Alice"));
    }

    #[test]
    fn assemble_with_signature_and_reply() {
        let result = assemble_compose_document(
            Some("<p>Cheers,</p>"),
            None,
            Some(QuotedContent {
                attribution: "On Mar 19, 2026, Bob wrote:".to_owned(),
                body_html: "<p>Original message</p>".to_owned(),
            }),
        );
        // Block 0: empty paragraph
        // Block 1: HorizontalRule
        // Block 2: "Cheers,"
        // Block 3: attribution (italic)
        // Block 4: BlockQuote
        assert_eq!(result.document.block_count(), 5);
        assert_eq!(result.signature_separator_index, Some(1));

        // Check attribution is italic.
        let attr_block = result.document.block(3).expect("attribution block");
        if let Block::Paragraph { runs } = attr_block {
            assert_eq!(runs.len(), 1);
            assert_eq!(runs[0].text, "On Mar 19, 2026, Bob wrote:");
            assert_eq!(runs[0].style, InlineStyle::ITALIC);
        } else {
            panic!("expected paragraph for attribution, got {attr_block:?}");
        }

        // Check blockquote.
        let quote_block = result.document.block(4).expect("blockquote");
        assert!(
            matches!(quote_block, Block::BlockQuote { .. }),
            "expected BlockQuote, got {quote_block:?}"
        );
    }

    #[test]
    fn assemble_with_quoted_content_only() {
        let result = assemble_compose_document(
            None,
            None,
            Some(QuotedContent {
                attribution: "On Mar 19, 2026, Carol wrote:".to_owned(),
                body_html: "<p>Hello there</p>".to_owned(),
            }),
        );
        // Block 0: empty paragraph
        // Block 1: attribution (italic)
        // Block 2: BlockQuote
        assert_eq!(result.document.block_count(), 3);
        assert_eq!(result.signature_separator_index, None);

        let attr_block = result.document.block(1).expect("attribution");
        if let Block::Paragraph { runs } = attr_block {
            assert_eq!(runs[0].style, InlineStyle::ITALIC);
            assert_eq!(runs[0].text, "On Mar 19, 2026, Carol wrote:");
        } else {
            panic!("expected paragraph");
        }
    }

    #[test]
    fn build_reply_attribution_produces_italic() {
        let block = build_reply_attribution_block("Mar 19, 2026", "Alice Smith");
        if let Block::Paragraph { runs } = &block {
            assert_eq!(runs.len(), 1);
            assert_eq!(runs[0].text, "On Mar 19, 2026, Alice Smith wrote:");
            assert_eq!(runs[0].style, InlineStyle::ITALIC);
        } else {
            panic!("expected paragraph, got {block:?}");
        }
    }

    #[test]
    fn build_forward_header_produces_expected_text() {
        let block = build_forward_header();
        if let Block::Paragraph { runs } = &block {
            assert_eq!(runs.len(), 1);
            assert_eq!(runs[0].text, "---------- Forwarded message ----------");
            assert_eq!(runs[0].style, InlineStyle::empty());
        } else {
            panic!("expected paragraph, got {block:?}");
        }
    }

    #[test]
    fn insert_signature_into_existing_document() {
        let mut doc = Document::from_blocks(vec![
            Block::paragraph("Hello, world!"),
            Block::paragraph("Some more text"),
        ]);
        let sep_idx = insert_signature(&mut doc, 2, "<p>Best,</p><p>Alice</p>");
        // Block 0: "Hello, world!"
        // Block 1: "Some more text"
        // Block 2: HorizontalRule
        // Block 3: "Best,"
        // Block 4: "Alice"
        assert_eq!(sep_idx, Some(2));
        assert_eq!(doc.block_count(), 5);
        assert_eq!(doc.block(2), Some(&Block::HorizontalRule));
        assert_eq!(
            doc.block(3).map(Block::flattened_text).as_deref(),
            Some("Best,")
        );
        assert_eq!(
            doc.block(4).map(Block::flattened_text).as_deref(),
            Some("Alice")
        );
    }

    #[test]
    fn insert_signature_at_beginning() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("Content")]);
        let sep_idx = insert_signature(&mut doc, 0, "<p>Sig</p>");
        assert_eq!(sep_idx, Some(0));
        assert_eq!(doc.block_count(), 3);
        assert_eq!(doc.block(0), Some(&Block::HorizontalRule));
        assert_eq!(
            doc.block(1).map(Block::flattened_text).as_deref(),
            Some("Sig")
        );
        assert_eq!(
            doc.block(2).map(Block::flattened_text).as_deref(),
            Some("Content")
        );
    }

    #[test]
    fn remove_signature_with_end_index() {
        let mut doc = Document::from_blocks(vec![
            Block::paragraph("Content"),
            Block::HorizontalRule,
            Block::paragraph("Sig line 1"),
            Block::paragraph("Sig line 2"),
            Block::paragraph("Attribution"),
        ]);
        // Remove blocks 1..4 (HR + sig lines, keep attribution).
        remove_signature(&mut doc, 1, Some(4));
        assert_eq!(doc.block_count(), 2);
        assert_eq!(
            doc.block(0).map(Block::flattened_text).as_deref(),
            Some("Content")
        );
        assert_eq!(
            doc.block(1).map(Block::flattened_text).as_deref(),
            Some("Attribution")
        );
    }

    #[test]
    fn remove_signature_to_end() {
        let mut doc = Document::from_blocks(vec![
            Block::paragraph("Content"),
            Block::HorizontalRule,
            Block::paragraph("Sig"),
        ]);
        remove_signature(&mut doc, 1, None);
        // Should keep at least one block (the content).
        assert_eq!(doc.block_count(), 1);
        assert_eq!(
            doc.block(0).map(Block::flattened_text).as_deref(),
            Some("Content")
        );
    }

    #[test]
    fn remove_signature_preserves_at_least_one_block() {
        let mut doc = Document::from_blocks(vec![Block::HorizontalRule, Block::paragraph("Sig")]);
        remove_signature(&mut doc, 0, None);
        // Document must retain at least one block.
        assert!(doc.block_count() >= 1);
    }

    #[test]
    fn replace_signature_old_to_new() {
        let mut doc = Document::from_blocks(vec![
            Block::paragraph("Content"),
            Block::HorizontalRule,
            Block::paragraph("Old Sig"),
            Block::paragraph("Attribution"),
        ]);
        // Old sig region: blocks 1..3 (HR + "Old Sig"). Attribution at 3.
        let new_idx = replace_signature(&mut doc, 1, Some(3), Some("<p>New Sig</p>"));
        assert_eq!(new_idx, Some(1));
        assert_eq!(doc.block(1), Some(&Block::HorizontalRule));
        assert_eq!(
            doc.block(2).map(Block::flattened_text).as_deref(),
            Some("New Sig")
        );
        assert_eq!(
            doc.block(3).map(Block::flattened_text).as_deref(),
            Some("Attribution")
        );
    }

    #[test]
    fn replace_signature_old_to_none() {
        let mut doc = Document::from_blocks(vec![
            Block::paragraph("Content"),
            Block::HorizontalRule,
            Block::paragraph("Old Sig"),
            Block::paragraph("Attribution"),
        ]);
        let new_idx = replace_signature(&mut doc, 1, Some(3), None);
        assert_eq!(new_idx, None);
        assert_eq!(doc.block_count(), 2);
        assert_eq!(
            doc.block(0).map(Block::flattened_text).as_deref(),
            Some("Content")
        );
        assert_eq!(
            doc.block(1).map(Block::flattened_text).as_deref(),
            Some("Attribution")
        );
    }

    #[test]
    fn replace_signature_none_to_new() {
        // Simulate a document with no signature — old_separator_index points
        // to where the attribution starts, old_end_index == old_separator_index
        // (empty range to remove).
        let mut doc = Document::from_blocks(vec![
            Block::paragraph("Content"),
            Block::paragraph("Attribution"),
        ]);
        let new_idx = replace_signature(&mut doc, 1, Some(1), Some("<p>New Sig</p>"));
        assert_eq!(new_idx, Some(1));
        // Block 0: "Content"
        // Block 1: HorizontalRule
        // Block 2: "New Sig"
        // Block 3: "Attribution"
        assert_eq!(doc.block_count(), 4);
        assert_eq!(doc.block(1), Some(&Block::HorizontalRule));
        assert_eq!(
            doc.block(2).map(Block::flattened_text).as_deref(),
            Some("New Sig")
        );
    }

    #[test]
    fn signature_separator_index_correct_for_various_signatures() {
        // Single-block signature.
        let result = assemble_compose_document(Some("<p>Short</p>"), None, None);
        assert_eq!(result.signature_separator_index, Some(1));
        assert_eq!(result.document.block_count(), 3); // empty + HR + sig

        // Multi-block signature.
        let result =
            assemble_compose_document(Some("<p>Line 1</p><p>Line 2</p><p>Line 3</p>"), None, None);
        assert_eq!(result.signature_separator_index, Some(1));
        assert_eq!(result.document.block_count(), 5); // empty + HR + 3 sig blocks
    }

    #[test]
    fn assemble_signature_with_rich_html() {
        let sig_html = "<p><strong>Alice Smith</strong></p><p>Engineering Lead</p>";
        let result = assemble_compose_document(Some(sig_html), None, None);
        assert_eq!(result.signature_separator_index, Some(1));

        // Verify the bold run.
        let first_sig_block = result.document.block(2).expect("first sig block");
        if let Block::Paragraph { runs } = first_sig_block {
            assert_eq!(runs[0].text, "Alice Smith");
            assert_eq!(runs[0].style, InlineStyle::BOLD);
        } else {
            panic!("expected paragraph");
        }
    }

    #[test]
    fn assemble_full_reply_structure() {
        let result = assemble_compose_document(
            Some("<p>Thanks,</p><p>Alice</p>"),
            None,
            Some(QuotedContent {
                attribution: "On Mar 19, 2026, Bob Jones wrote:".to_owned(),
                body_html: "<p>Hey Alice,</p><p>How are you?</p>".to_owned(),
            }),
        );
        // Block 0: empty paragraph
        // Block 1: HR (sig separator)
        // Block 2: "Thanks,"
        // Block 3: "Alice"
        // Block 4: attribution (italic)
        // Block 5: blockquote
        assert_eq!(result.document.block_count(), 6);
        assert_eq!(result.signature_separator_index, Some(1));

        // Verify quoted content is in a blockquote.
        let bq = result.document.block(5).expect("blockquote");
        if let Block::BlockQuote { blocks } = bq {
            assert_eq!(blocks.len(), 2);
            assert_eq!(blocks[0].flattened_text(), "Hey Alice,");
            assert_eq!(blocks[1].flattened_text(), "How are you?");
        } else {
            panic!("expected BlockQuote, got {bq:?}");
        }
    }

    // ── Blank signature handling ─────────────────────────

    #[test]
    fn assemble_blank_signature_omitted() {
        let result = assemble_compose_document(Some(""), None, None);
        assert_eq!(result.document.block_count(), 1);
        assert!(result.signature_separator_index.is_none());
    }

    #[test]
    fn assemble_whitespace_signature_omitted() {
        let result = assemble_compose_document(Some("   \n  "), None, None);
        assert_eq!(result.document.block_count(), 1);
        assert!(result.signature_separator_index.is_none());
    }

    #[test]
    fn assemble_empty_paragraph_signature_omitted() {
        let result = assemble_compose_document(Some("<p>  </p>"), None, None);
        assert_eq!(result.document.block_count(), 1);
        assert!(result.signature_separator_index.is_none());
    }

    #[test]
    fn insert_blank_signature_returns_none() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        assert!(insert_signature(&mut doc, 1, "").is_none());
        assert_eq!(doc.block_count(), 1);
    }

    // ── Out-of-range index clamping ─────────────────────

    #[test]
    fn insert_signature_out_of_range_clamps() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        let sep = insert_signature(&mut doc, 100, "<p>Sig</p>");
        // Should clamp to end (index 1), not panic.
        assert_eq!(sep, Some(1));
        assert_eq!(doc.block_count(), 3);
    }

    #[test]
    fn is_blank_html_false_for_image() {
        // A document containing only an image should NOT be blank.
        let result = assemble_compose_document(
            Some(r#"<img src="inline-image:abc123" alt="logo">"#),
            None,
            None,
        );
        // Should have: empty paragraph + HR + image block
        assert!(result.signature_separator_index.is_some());
        assert!(result.document.block_count() > 2);
    }

    #[test]
    fn remove_signature_out_of_range_no_panic() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        // Should not panic with out-of-range indices.
        remove_signature(&mut doc, 10, Some(20));
        assert_eq!(doc.block_count(), 1);
    }
}
