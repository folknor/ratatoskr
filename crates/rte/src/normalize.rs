//! Normalization pass: merge adjacent same-style runs, enforce structural invariants.
//!
//! Runs after every edit (or batched for multi-op transactions). Only processes
//! dirty blocks to keep cost minimal.
//!
//! Invariants enforced:
//! 1. Adjacent `StyledRun`s with identical `(style, link)` pairs are merged.
//! 2. Empty runs are removed, but at least one run per inline block is kept
//!    (for cursor anchoring).
//! 3. Every inline block (`Paragraph`, `Heading`) has at least one run.
//! 4. Every `ListItem` has at least one block.
//! 5. Every `BlockQuote` has at least one block.
//! 6. The document has at least one block.
//!
//! Modeled after Slate's normalization with a safety valve: max iterations =
//! dirty_count × 42 to prevent infinite loops from buggy normalizers.

use std::sync::Arc;

use crate::document::{Block, Document, StyledRun};

/// Safety valve multiplier (from Slate). Max iterations = dirty_count × this.
const SAFETY_MULTIPLIER: usize = 42;

/// Normalize the entire document, fixing any invariant violations.
///
/// - Merges adjacent `StyledRun`s with identical `(style, link)` pairs
/// - Removes empty runs (but keeps one empty run per block for cursor anchoring)
/// - Ensures every inline block has at least one run
/// - Ensures `List` items each contain at least one block
/// - Ensures `BlockQuote` contains at least one block
/// - Ensures the document has at least one block
pub fn normalize(doc: &mut Document) {
    // Ensure at least one block.
    if doc.blocks.is_empty() {
        doc.blocks.push(Arc::new(Block::empty_paragraph()));
    }

    let indices: Vec<usize> = (0..doc.blocks.len()).collect();
    normalize_blocks(doc, &indices);
}

/// Normalize only the blocks at the given dirty indices.
///
/// This is the fast path — most edits only dirty 1–2 blocks. Indices that are
/// out of bounds are silently skipped.
pub fn normalize_blocks(doc: &mut Document, dirty: &[usize]) {
    // Ensure at least one block (document-level invariant).
    if doc.blocks.is_empty() {
        doc.blocks.push(Arc::new(Block::empty_paragraph()));
    }

    let max_iterations = dirty.len().saturating_mul(SAFETY_MULTIPLIER).max(1);
    let mut iterations = 0;

    // We may need multiple passes if normalizing one block creates new dirty
    // state (unlikely in practice, but the safety valve handles it).
    let mut pending: Vec<usize> = dirty.to_vec();

    while !pending.is_empty() && iterations < max_iterations {
        let current_batch: Vec<usize> = std::mem::take(&mut pending);

        for &idx in &current_batch {
            if idx >= doc.blocks.len() {
                continue;
            }

            iterations += 1;
            if iterations > max_iterations {
                break;
            }

            let block = (*doc.blocks[idx]).clone();
            let normalized = normalize_block(block);
            doc.blocks[idx] = Arc::new(normalized);
        }
    }

    // Final document-level invariant: must have at least one block.
    if doc.blocks.is_empty() {
        doc.blocks.push(Arc::new(Block::empty_paragraph()));
    }
}

/// Normalize a single block, returning the normalized version.
fn normalize_block(block: Block) -> Block {
    match block {
        Block::Paragraph { runs } => Block::Paragraph {
            runs: normalize_runs(runs),
        },
        Block::Heading { level, runs } => Block::Heading {
            level,
            runs: normalize_runs(runs),
        },
        Block::ListItem {
            ordered,
            indent_level,
            runs,
        } => Block::ListItem {
            ordered,
            indent_level,
            runs: normalize_runs(runs),
        },
        Block::BlockQuote { blocks } => {
            let blocks = normalize_child_blocks(blocks);
            Block::BlockQuote { blocks }
        }
        Block::HorizontalRule => Block::HorizontalRule,
        Block::Image { .. } => block,
    }
}

/// Normalize the runs of an inline block.
///
/// 1. Remove empty runs (unless that would leave zero runs).
/// 2. Merge adjacent runs with identical formatting.
/// 3. Ensure at least one run exists (insert empty plain run if needed).
fn normalize_runs(runs: Vec<StyledRun>) -> Vec<StyledRun> {
    if runs.is_empty() {
        return vec![StyledRun::plain(String::new())];
    }

    // First pass: remove empty runs, but collect all runs for merging.
    let non_empty: Vec<StyledRun> = runs.into_iter().filter(|r| !r.is_empty()).collect();

    // If all runs were empty, keep one empty run for cursor anchoring.
    if non_empty.is_empty() {
        return vec![StyledRun::plain(String::new())];
    }

    // Second pass: merge adjacent runs with identical formatting.
    let mut merged: Vec<StyledRun> = Vec::with_capacity(non_empty.len());

    for run in non_empty {
        if let Some(last) = merged.last_mut()
            && last.same_formatting(&run)
        {
            last.text.push_str(&run.text);
            continue;
        }
        merged.push(run);
    }

    // Should not be empty at this point, but guard anyway.
    if merged.is_empty() {
        merged.push(StyledRun::plain(String::new()));
    }

    merged
}

/// Normalize child blocks of a container (BlockQuote): ensure at least one
/// block, and normalize each recursively.
fn normalize_child_blocks(mut blocks: Vec<Arc<Block>>) -> Vec<Arc<Block>> {
    if blocks.is_empty() {
        blocks.push(Arc::new(Block::empty_paragraph()));
    }

    blocks
        .into_iter()
        .map(|arc_block| {
            let block = (*arc_block).clone();
            Arc::new(normalize_block(block))
        })
        .collect()
}

// ── Tests ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::document::{Block, Document, HeadingLevel, InlineStyle, StyledRun};

    use super::{normalize, normalize_blocks};

    // ── Merge adjacent same-style runs ──────────────────

    #[test]
    fn merge_adjacent_same_style_runs() {
        let mut doc = Document {
            blocks: vec![Arc::new(Block::Paragraph {
                runs: vec![
                    StyledRun::plain("hello"),
                    StyledRun::plain(" "),
                    StyledRun::plain("world"),
                ],
            })],
        };
        normalize(&mut doc);

        let runs = doc.blocks[0].runs().expect("should have runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "hello world");
    }

    #[test]
    fn merge_adjacent_bold_runs() {
        let mut doc = Document {
            blocks: vec![Arc::new(Block::Paragraph {
                runs: vec![
                    StyledRun::styled("foo", InlineStyle::BOLD),
                    StyledRun::styled("bar", InlineStyle::BOLD),
                ],
            })],
        };
        normalize(&mut doc);

        let runs = doc.blocks[0].runs().expect("should have runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "foobar");
        assert_eq!(runs[0].style, InlineStyle::BOLD);
    }

    #[test]
    fn no_merge_different_style_runs() {
        let mut doc = Document {
            blocks: vec![Arc::new(Block::Paragraph {
                runs: vec![
                    StyledRun::plain("hello"),
                    StyledRun::styled(" world", InlineStyle::BOLD),
                ],
            })],
        };
        normalize(&mut doc);

        let runs = doc.blocks[0].runs().expect("should have runs");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].text, "hello");
        assert_eq!(runs[1].text, " world");
    }

    #[test]
    fn no_merge_different_links() {
        let mut doc = Document {
            blocks: vec![Arc::new(Block::Paragraph {
                runs: vec![
                    StyledRun::linked("a", InlineStyle::empty(), "https://a.com"),
                    StyledRun::linked("b", InlineStyle::empty(), "https://b.com"),
                ],
            })],
        };
        normalize(&mut doc);

        let runs = doc.blocks[0].runs().expect("should have runs");
        assert_eq!(runs.len(), 2);
    }

    #[test]
    fn merge_same_links() {
        let mut doc = Document {
            blocks: vec![Arc::new(Block::Paragraph {
                runs: vec![
                    StyledRun::linked("click", InlineStyle::empty(), "https://example.com"),
                    StyledRun::linked(" here", InlineStyle::empty(), "https://example.com"),
                ],
            })],
        };
        normalize(&mut doc);

        let runs = doc.blocks[0].runs().expect("should have runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "click here");
        assert_eq!(runs[0].link.as_deref(), Some("https://example.com"));
    }

    // ── Remove empty runs ───────────────────────────────

    #[test]
    fn remove_empty_runs_keep_last() {
        let mut doc = Document {
            blocks: vec![Arc::new(Block::Paragraph {
                runs: vec![
                    StyledRun::plain(""),
                    StyledRun::plain(""),
                    StyledRun::plain(""),
                ],
            })],
        };
        normalize(&mut doc);

        let runs = doc.blocks[0].runs().expect("should have runs");
        assert_eq!(runs.len(), 1);
        assert!(runs[0].is_empty());
    }

    #[test]
    fn remove_empty_runs_keep_non_empty() {
        let mut doc = Document {
            blocks: vec![Arc::new(Block::Paragraph {
                runs: vec![
                    StyledRun::plain(""),
                    StyledRun::styled("hello", InlineStyle::BOLD),
                    StyledRun::plain(""),
                    StyledRun::plain(" world"),
                ],
            })],
        };
        normalize(&mut doc);

        let runs = doc.blocks[0].runs().expect("should have runs");
        // "hello" (bold) + " world" (plain) — different styles, won't merge.
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].text, "hello");
        assert_eq!(runs[0].style, InlineStyle::BOLD);
        assert_eq!(runs[1].text, " world");
    }

    // ── Empty blocks get an empty run ───────────────────

    #[test]
    fn empty_paragraph_gets_empty_run() {
        let mut doc = Document {
            blocks: vec![Arc::new(Block::Paragraph { runs: vec![] })],
        };
        normalize(&mut doc);

        let runs = doc.blocks[0].runs().expect("should have runs");
        assert_eq!(runs.len(), 1);
        assert!(runs[0].is_empty());
    }

    #[test]
    fn empty_heading_gets_empty_run() {
        let mut doc = Document {
            blocks: vec![Arc::new(Block::Heading {
                level: HeadingLevel::H2,
                runs: vec![],
            })],
        };
        normalize(&mut doc);

        let runs = doc.blocks[0].runs().expect("should have runs");
        assert_eq!(runs.len(), 1);
        assert!(runs[0].is_empty());
    }

    // ── List items get runs normalized ────────────────

    #[test]
    fn empty_list_item_gets_empty_run() {
        let mut doc = Document {
            blocks: vec![Arc::new(Block::ListItem {
                ordered: false,
                indent_level: 0,
                runs: vec![],
            })],
        };
        normalize(&mut doc);

        let runs = doc.blocks[0].runs().expect("should have runs");
        assert_eq!(runs.len(), 1);
        assert!(runs[0].is_empty());
    }

    #[test]
    fn list_item_runs_are_normalized() {
        let mut doc = Document {
            blocks: vec![Arc::new(Block::ListItem {
                ordered: true,
                indent_level: 0,
                runs: vec![StyledRun::plain("a"), StyledRun::plain("b")],
            })],
        };
        normalize(&mut doc);

        let runs = doc.blocks[0].runs().expect("should have runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "ab");
    }

    // ── Empty blockquotes get a paragraph ───────────────

    #[test]
    fn empty_blockquote_gets_paragraph() {
        let mut doc = Document {
            blocks: vec![Arc::new(Block::BlockQuote { blocks: vec![] })],
        };
        normalize(&mut doc);

        if let Block::BlockQuote { blocks } = &*doc.blocks[0] {
            assert_eq!(blocks.len(), 1);
            assert!(blocks[0].is_inline_block());
        } else {
            panic!("expected BlockQuote block");
        }
    }

    #[test]
    fn blockquote_children_are_normalized() {
        let mut doc = Document {
            blocks: vec![Arc::new(Block::BlockQuote {
                blocks: vec![Arc::new(Block::Paragraph {
                    runs: vec![
                        StyledRun::styled("x", InlineStyle::ITALIC),
                        StyledRun::styled("y", InlineStyle::ITALIC),
                    ],
                })],
            })],
        };
        normalize(&mut doc);

        if let Block::BlockQuote { blocks } = &*doc.blocks[0] {
            let runs = blocks[0].runs().expect("should have runs");
            assert_eq!(runs.len(), 1);
            assert_eq!(runs[0].text, "xy");
            assert_eq!(runs[0].style, InlineStyle::ITALIC);
        } else {
            panic!("expected BlockQuote block");
        }
    }

    // ── Empty document gets a paragraph ─────────────────

    #[test]
    fn empty_document_gets_paragraph() {
        let mut doc = Document { blocks: vec![] };
        normalize(&mut doc);

        assert_eq!(doc.blocks.len(), 1);
        assert!(doc.blocks[0].is_inline_block());
    }

    // ── Mixed scenarios: some blocks clean, some dirty ──

    #[test]
    fn normalize_blocks_only_dirty() {
        let mut doc = Document {
            blocks: vec![
                Arc::new(Block::Paragraph {
                    runs: vec![StyledRun::plain("a"), StyledRun::plain("b")],
                }),
                Arc::new(Block::Paragraph {
                    runs: vec![StyledRun::plain("clean")],
                }),
                Arc::new(Block::Paragraph {
                    runs: vec![
                        StyledRun::styled("x", InlineStyle::BOLD),
                        StyledRun::styled("y", InlineStyle::BOLD),
                    ],
                }),
            ],
        };

        // Keep a reference to the clean block.
        let clean_arc = Arc::clone(&doc.blocks[1]);

        // Only normalize indices 0 and 2.
        normalize_blocks(&mut doc, &[0, 2]);

        // Block 0 should be merged.
        let runs0 = doc.blocks[0].runs().expect("runs");
        assert_eq!(runs0.len(), 1);
        assert_eq!(runs0[0].text, "ab");

        // Block 1 should be untouched (same Arc pointer).
        assert!(Arc::ptr_eq(&doc.blocks[1], &clean_arc));

        // Block 2 should be merged.
        let runs2 = doc.blocks[2].runs().expect("runs");
        assert_eq!(runs2.len(), 1);
        assert_eq!(runs2[0].text, "xy");
    }

    #[test]
    fn normalize_blocks_skips_out_of_bounds() {
        let mut doc = Document {
            blocks: vec![Arc::new(Block::Paragraph {
                runs: vec![StyledRun::plain("ok")],
            })],
        };

        // Should not panic on out-of-bounds indices.
        normalize_blocks(&mut doc, &[0, 5, 100]);

        assert_eq!(doc.blocks.len(), 1);
    }

    // ── Safety valve terminates ─────────────────────────

    #[test]
    fn safety_valve_terminates() {
        // Even with many dirty indices, the function terminates.
        let mut doc = Document {
            blocks: vec![Arc::new(Block::Paragraph {
                runs: vec![
                    StyledRun::plain("a"),
                    StyledRun::plain("b"),
                    StyledRun::plain("c"),
                ],
            })],
        };

        // Pass the same index many times — the safety valve should prevent
        // unbounded iteration.
        let dirty: Vec<usize> = vec![0; 1000];
        normalize_blocks(&mut doc, &dirty);

        // Should still produce correct output.
        let runs = doc.blocks[0].runs().expect("runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "abc");
    }

    #[test]
    fn safety_valve_with_empty_dirty_list() {
        let mut doc = Document {
            blocks: vec![Arc::new(Block::Paragraph {
                runs: vec![StyledRun::plain("untouched")],
            })],
        };

        normalize_blocks(&mut doc, &[]);

        let runs = doc.blocks[0].runs().expect("runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "untouched");
    }

    // ── Image passes through unchanged ─────────────────

    #[test]
    fn image_passes_through_unchanged() {
        let mut doc = Document {
            blocks: vec![Arc::new(Block::Image {
                src: "https://example.com/img.png".into(),
                alt: "photo".into(),
                width: Some(100),
                height: Some(50),
            })],
        };
        normalize(&mut doc);

        if let Block::Image {
            src,
            alt,
            width,
            height,
        } = &*doc.blocks[0]
        {
            assert_eq!(src, "https://example.com/img.png");
            assert_eq!(alt, "photo");
            assert_eq!(*width, Some(100));
            assert_eq!(*height, Some(50));
        } else {
            panic!("expected Image block");
        }
    }

    // ── HorizontalRule passes through ───────────────────

    #[test]
    fn horizontal_rule_unchanged() {
        let mut doc = Document {
            blocks: vec![Arc::new(Block::HorizontalRule)],
        };
        normalize(&mut doc);

        assert!(matches!(&*doc.blocks[0], Block::HorizontalRule));
    }

    // ── Complex mixed document ──────────────────────────

    #[test]
    fn complex_mixed_document() {
        let mut doc = Document {
            blocks: vec![
                // Paragraph with mergeable runs.
                Arc::new(Block::Paragraph {
                    runs: vec![StyledRun::plain("hello"), StyledRun::plain(" world")],
                }),
                // Heading with empty runs interspersed.
                Arc::new(Block::Heading {
                    level: HeadingLevel::H1,
                    runs: vec![
                        StyledRun::plain(""),
                        StyledRun::styled("title", InlineStyle::BOLD),
                        StyledRun::plain(""),
                    ],
                }),
                // List items with runs to normalize.
                Arc::new(Block::ListItem {
                    ordered: false,
                    indent_level: 0,
                    runs: vec![],
                }),
                Arc::new(Block::ListItem {
                    ordered: false,
                    indent_level: 0,
                    runs: vec![StyledRun::plain("item"), StyledRun::plain(" text")],
                }),
                // Empty blockquote.
                Arc::new(Block::BlockQuote { blocks: vec![] }),
                // Horizontal rule.
                Arc::new(Block::HorizontalRule),
            ],
        };

        normalize(&mut doc);

        // Paragraph: merged to one run.
        let runs0 = doc.blocks[0].runs().expect("runs");
        assert_eq!(runs0.len(), 1);
        assert_eq!(runs0[0].text, "hello world");

        // Heading: empty runs removed, one bold run remains.
        let runs1 = doc.blocks[1].runs().expect("runs");
        assert_eq!(runs1.len(), 1);
        assert_eq!(runs1[0].text, "title");
        assert_eq!(runs1[0].style, InlineStyle::BOLD);

        // ListItem (empty): got an empty run.
        let runs2 = doc.blocks[2].runs().expect("runs");
        assert_eq!(runs2.len(), 1);
        assert!(runs2[0].is_empty());

        // ListItem (non-empty): runs merged.
        let runs3 = doc.blocks[3].runs().expect("runs");
        assert_eq!(runs3.len(), 1);
        assert_eq!(runs3[0].text, "item text");

        // BlockQuote: got a paragraph.
        if let Block::BlockQuote { blocks } = &*doc.blocks[4] {
            assert_eq!(blocks.len(), 1);
            assert!(blocks[0].is_inline_block());
        } else {
            panic!("expected BlockQuote");
        }

        // HorizontalRule: unchanged.
        assert!(matches!(&*doc.blocks[5], Block::HorizontalRule));
    }

    // ── List item runs merge with indent preserved ──────

    #[test]
    fn list_item_preserves_indent() {
        let mut doc = Document {
            blocks: vec![Arc::new(Block::ListItem {
                ordered: true,
                indent_level: 2,
                runs: vec![StyledRun::plain("a"), StyledRun::plain("b")],
            })],
        };

        normalize(&mut doc);

        if let Block::ListItem {
            indent_level, runs, ..
        } = &*doc.blocks[0]
        {
            assert_eq!(*indent_level, 2);
            assert_eq!(runs.len(), 1);
            assert_eq!(runs[0].text, "ab");
        } else {
            panic!("expected ListItem");
        }
    }

    // ── Multiple merge passes in one block ──────────────

    #[test]
    fn multiple_adjacent_groups_merge() {
        let mut doc = Document {
            blocks: vec![Arc::new(Block::Paragraph {
                runs: vec![
                    StyledRun::plain("a"),
                    StyledRun::plain("b"),
                    StyledRun::styled("c", InlineStyle::BOLD),
                    StyledRun::styled("d", InlineStyle::BOLD),
                    StyledRun::plain("e"),
                    StyledRun::plain("f"),
                ],
            })],
        };
        normalize(&mut doc);

        let runs = doc.blocks[0].runs().expect("runs");
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].text, "ab");
        assert_eq!(runs[0].style, InlineStyle::empty());
        assert_eq!(runs[1].text, "cd");
        assert_eq!(runs[1].style, InlineStyle::BOLD);
        assert_eq!(runs[2].text, "ef");
        assert_eq!(runs[2].style, InlineStyle::empty());
    }
}
