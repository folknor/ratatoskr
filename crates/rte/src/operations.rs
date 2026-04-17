//! Editing operations: `EditOp`, `PosMap`, apply/invert, format toggle, run splitting.
//!
//! Every user action creates an `EditOp` that knows how to apply and reverse itself.
//! Each `apply()` returns a `PosMap` describing what shifted, enabling cursor mapping.
//!
//! # Panics
//!
//! Operations panic on invariant violations (out-of-bounds block indices, operations
//! on non-inline blocks). These represent programming errors in the calling code,
//! not user-input errors.

use std::sync::Arc;

use crate::document::{
    Block, BlockKind, DocPosition, Document, InlineStyle, StyledRun, isolate_runs,
};

// ── Position map ────────────────────────────────────────

/// Position map produced by an edit operation. Describes what shifted so that
/// cursors and selections can be mapped through edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PosMap {
    /// The block index where the change occurred.
    pub block_index: usize,
    /// Changes within the block (char-level).
    pub entries: Vec<PosMapEntry>,
    /// Block-level structural change (if any).
    pub structural: Option<StructuralChange>,
}

/// A single char-level mapping entry within a block.
///
/// Represents: at `old_offset`, `old_len` characters were replaced by `new_len`
/// characters. Characters before `old_offset` are unaffected; characters after
/// `old_offset + old_len` shift by `(new_len - old_len)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PosMapEntry {
    pub old_offset: usize,
    pub old_len: usize,
    pub new_len: usize,
}

/// Block-level structural changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuralChange {
    /// A block was split at `block_index` at char offset `split_offset`,
    /// creating a new block at `block_index + 1`.
    Split {
        block_index: usize,
        split_offset: usize,
    },
    /// Block at `block_index` was merged into `block_index - 1`.
    /// `merge_offset` is the char length of block `block_index - 1` before the merge.
    Merge {
        block_index: usize,
        merge_offset: usize,
    },
    /// One or more blocks were inserted starting at `block_index`.
    Insert { block_index: usize, count: usize },
    /// A block was removed at `block_index`.
    Remove { block_index: usize },
    /// Multiple blocks were removed/merged in a cross-block delete.
    /// `start_block` is the first affected block, `removed_count` is how many
    /// blocks after it were removed (merged into `start_block`).
    /// `start_offset` is the char offset within `start_block` where deletion began.
    CrossBlockDelete {
        start_block: usize,
        removed_count: usize,
        start_offset: usize,
    },
}

/// Content captured during a delete operation, sufficient to reconstruct
/// the original structure on undo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedContent {
    /// The blocks (or partial blocks) that were deleted.
    ///
    /// For a single-block delete: one block containing only the deleted runs.
    /// For a cross-block delete: first entry = tail of start block that was removed,
    /// middle entries = fully deleted blocks (if any), last entry = head of end block
    /// that was removed.
    pub blocks: Vec<Block>,
}

// ── Edit operation ──────────────────────────────────────

/// An editing operation. Each variant carries enough data to apply and invert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOp {
    /// Insert text at a position within an inline block.
    InsertText { position: DocPosition, text: String },

    /// Delete a range of content. `deleted` captures enough to reconstruct on undo.
    DeleteRange {
        start: DocPosition,
        end: DocPosition,
        deleted: DeletedContent,
    },

    /// Split a block at a position (Enter key). Creates a new block after the
    /// current one. The new block inherits the current block's type.
    SplitBlock { position: DocPosition },

    /// Merge a block with the previous one (Backspace at block start).
    ///
    /// - `saved` stores the original block at `block_index` for undo.
    /// - `merge_offset` is the char length of block `block_index - 1` before the
    ///   merge. Needed so `invert()` can produce a `SplitBlock` at the correct
    ///   offset without access to the document.
    MergeBlocks {
        block_index: usize,
        saved: Block,
        merge_offset: usize,
    },

    /// Toggle an inline style on a range. If ALL text in the range already has the
    /// style, removes it; otherwise adds it.
    ToggleInlineStyle {
        start: DocPosition,
        end: DocPosition,
        style_bit: InlineStyle,
    },

    /// Change a block's type (e.g., paragraph to heading).
    SetBlockType {
        block_index: usize,
        old: BlockKind,
        new: BlockKind,
    },

    /// Set block-level attributes (alignment, indent level) without changing
    /// the block type.
    SetBlockAttrs {
        block_index: usize,
        old: crate::document::BlockAttrs,
        new: crate::document::BlockAttrs,
    },

    /// Insert a new block at an index.
    InsertBlock { index: usize, block: Block },

    /// Remove a block at an index.
    RemoveBlock { index: usize, saved: Block },
}

// ── PosMap ──────────────────────────────────────────────

impl PosMap {
    /// A no-op position map that changes nothing.
    pub fn identity() -> Self {
        Self {
            block_index: 0,
            entries: Vec::new(),
            structural: None,
        }
    }

    /// Map a `DocPosition` through this edit.
    ///
    /// Positions before the edit are unaffected. Positions within deleted regions
    /// collapse to the deletion point. Positions after the edit shift by the delta.
    pub fn map(&self, pos: DocPosition) -> DocPosition {
        let pos = self.map_structural(pos);
        self.map_entries(pos)
    }

    fn map_structural(&self, pos: DocPosition) -> DocPosition {
        match self.structural {
            None => pos,

            Some(StructuralChange::Split {
                block_index,
                split_offset,
            }) => {
                if pos.block_index == block_index && pos.offset > split_offset {
                    // Position is in the split block, after the split point:
                    // remap to the new block with adjusted offset.
                    DocPosition::new(block_index + 1, pos.offset - split_offset)
                } else if pos.block_index > block_index {
                    DocPosition::new(pos.block_index + 1, pos.offset)
                } else {
                    pos
                }
            }

            Some(StructuralChange::Merge {
                block_index,
                merge_offset,
            }) => {
                if pos.block_index == block_index {
                    // Position is in the merged-away block: remap to prev block
                    // with offset shifted past the surviving block's content.
                    DocPosition::new(block_index - 1, pos.offset + merge_offset)
                } else if pos.block_index > block_index {
                    DocPosition::new(pos.block_index - 1, pos.offset)
                } else {
                    pos
                }
            }

            Some(StructuralChange::Insert { block_index, count }) => {
                if pos.block_index >= block_index {
                    DocPosition::new(pos.block_index + count, pos.offset)
                } else {
                    pos
                }
            }

            Some(StructuralChange::Remove { block_index }) => {
                if pos.block_index == block_index {
                    DocPosition::new(block_index, 0)
                } else if pos.block_index > block_index {
                    DocPosition::new(pos.block_index - 1, pos.offset)
                } else {
                    pos
                }
            }

            Some(StructuralChange::CrossBlockDelete {
                start_block,
                removed_count,
                start_offset,
            }) => {
                let end_block = start_block + removed_count;
                if pos.block_index > start_block && pos.block_index <= end_block {
                    // Position is in a deleted block: collapse to the deletion point.
                    DocPosition::new(start_block, start_offset)
                } else if pos.block_index > end_block {
                    DocPosition::new(pos.block_index - removed_count, pos.offset)
                } else {
                    pos
                }
            }
        }
    }

    fn map_entries(&self, pos: DocPosition) -> DocPosition {
        if pos.block_index != self.block_index || self.entries.is_empty() {
            return pos;
        }

        let mut offset = pos.offset;
        for entry in &self.entries {
            if offset <= entry.old_offset {
                break;
            }
            if offset < entry.old_offset + entry.old_len {
                offset = entry.old_offset + entry.new_len;
                break;
            }
            // Safe: offset is always >= old_offset + old_len here, so the addition
            // of (new_len - old_len) won't underflow when new_len >= old_len, and
            // when new_len < old_len the subtraction from offset is always valid
            // because offset > old_offset + old_len > old_len - new_len.
            if entry.new_len >= entry.old_len {
                offset += entry.new_len - entry.old_len;
            } else {
                offset -= entry.old_len - entry.new_len;
            }
        }

        DocPosition::new(pos.block_index, offset)
    }
}

// ── EditOp ──────────────────────────────────────────────

impl EditOp {
    /// Apply this operation to a document, returning the position map.
    ///
    /// **Note:** `apply()` does *not* normalize the document afterward.
    /// Adjacent runs with identical formatting may remain un-merged.
    /// Callers should call [`normalize_blocks()`](crate::normalize::normalize_blocks)
    /// after applying operations to restore the normalization invariant.
    /// `EditorState::apply_action()` does this automatically.
    ///
    /// # Panics
    ///
    /// Panics if the operation references out-of-bounds block indices or
    /// operates on blocks without inline runs.
    pub fn apply(&self, doc: &mut Document) -> PosMap {
        match self {
            Self::InsertText { position, text } => apply_insert_text(doc, *position, text),
            Self::DeleteRange {
                start,
                end,
                deleted,
            } => apply_delete_range(doc, *start, *end, deleted),
            Self::SplitBlock { position } => apply_split_block(doc, *position),
            Self::MergeBlocks {
                block_index,
                merge_offset,
                ..
            } => apply_merge_blocks(doc, *block_index, *merge_offset),
            Self::ToggleInlineStyle {
                start,
                end,
                style_bit,
            } => apply_toggle_inline_style(doc, *start, *end, *style_bit),
            Self::SetBlockType {
                block_index, new, ..
            } => apply_set_block_type(doc, *block_index, *new),
            Self::SetBlockAttrs {
                block_index, new, ..
            } => apply_set_block_attrs(doc, *block_index, *new),
            Self::InsertBlock { index, block } => apply_insert_block(doc, *index, block),
            Self::RemoveBlock { index, .. } => apply_remove_block(doc, *index),
        }
    }

    /// Return the inverse operation (for undo).
    ///
    /// For `InsertText`, the captured `DeletedContent` uses plain (unstyled)
    /// runs because `invert()` has no document access. Use
    /// [`invert_with_doc`] when the document is available to capture the
    /// actual styled runs at the insertion range.
    pub fn invert(&self) -> Self {
        self.invert_inner(None)
    }

    /// Return the inverse operation, using the document to capture styled
    /// runs when inverting `InsertText`. This produces a correct
    /// `DeletedContent` so that redo-after-undo restores formatting.
    pub fn invert_with_doc(&self, doc: &Document) -> Self {
        self.invert_inner(Some(doc))
    }

    fn invert_inner(&self, doc: Option<&Document>) -> Self {
        match self {
            Self::InsertText { position, text } => {
                let char_count = text.chars().count();
                let end = DocPosition::new(position.block_index, position.offset + char_count);

                // When the document is available, capture the actual styled
                // runs so redo-after-undo preserves formatting.
                let deleted_runs = doc
                    .and_then(|d| d.block(position.block_index))
                    .and_then(|b| b.runs())
                    .map(|runs| {
                        extract_runs_from_range(runs, position.offset, position.offset + char_count)
                    })
                    .unwrap_or_else(|| vec![StyledRun::plain(text.clone())]);

                Self::DeleteRange {
                    start: *position,
                    end,
                    deleted: DeletedContent {
                        blocks: vec![Block::Paragraph { runs: deleted_runs }],
                    },
                }
            }

            Self::DeleteRange {
                start,
                end,
                deleted,
            } => invert_delete_range(*start, *end, deleted),

            Self::SplitBlock { position } => Self::MergeBlocks {
                block_index: position.block_index + 1,
                saved: Block::empty_paragraph(),
                merge_offset: position.offset,
            },

            Self::MergeBlocks {
                block_index,
                merge_offset,
                ..
            } => Self::SplitBlock {
                position: DocPosition::new(block_index - 1, *merge_offset),
            },

            Self::ToggleInlineStyle {
                start,
                end,
                style_bit,
            } => Self::ToggleInlineStyle {
                start: *start,
                end: *end,
                style_bit: *style_bit,
            },

            Self::SetBlockType {
                block_index,
                old,
                new,
            } => Self::SetBlockType {
                block_index: *block_index,
                old: *new,
                new: *old,
            },

            Self::SetBlockAttrs {
                block_index,
                old,
                new,
            } => Self::SetBlockAttrs {
                block_index: *block_index,
                old: *new,
                new: *old,
            },

            Self::InsertBlock { index, block } => Self::RemoveBlock {
                index: *index,
                saved: block.clone(),
            },

            Self::RemoveBlock { index, saved } => Self::InsertBlock {
                index: *index,
                block: saved.clone(),
            },
        }
    }
}

// ── Apply: InsertText ───────────────────────────────────

fn apply_insert_text(doc: &mut Document, position: DocPosition, text: &str) -> PosMap {
    let block = doc
        .block(position.block_index)
        .expect("InsertText: block_index out of bounds")
        .clone();

    let mut block = block;
    let runs = block
        .runs_mut()
        .expect("InsertText: block has no inline runs");

    insert_text_into_runs(runs, position.offset, text);
    doc.replace_block(position.block_index, block);

    let char_count = text.chars().count();
    PosMap {
        block_index: position.block_index,
        entries: vec![PosMapEntry {
            old_offset: position.offset,
            old_len: 0,
            new_len: char_count,
        }],
        structural: None,
    }
}

/// Insert `text` into a run list at the given flattened char offset.
/// The text is inserted into whichever run contains that offset, inheriting its style.
fn insert_text_into_runs(runs: &mut [StyledRun], offset: usize, text: &str) {
    let mut pos = 0;
    for run in runs.iter_mut() {
        let run_len = run.char_len();
        if offset >= pos && offset <= pos + run_len {
            let local = offset - pos;
            let byte_offset = run.char_to_byte_offset(local);
            run.text.insert_str(byte_offset, text);
            return;
        }
        pos += run_len;
    }
    if let Some(last) = runs.last_mut() {
        last.text.push_str(text);
    }
}

// ── Apply: DeleteRange ──────────────────────────────────

fn apply_delete_range(
    doc: &mut Document,
    start: DocPosition,
    end: DocPosition,
    deleted: &DeletedContent,
) -> PosMap {
    assert!(start <= end, "DeleteRange: start > end");

    // Sentinel: start == end with deleted content means "reconstruct"
    // (this is the inverse of a delete — both single-block and cross-block).
    if start == end && !deleted.blocks.is_empty() {
        return apply_restore_deleted(doc, start, deleted);
    }

    if start.block_index == end.block_index {
        return apply_single_block_delete(doc, start, end);
    }

    apply_cross_block_delete(doc, start, end)
}

fn apply_single_block_delete(doc: &mut Document, start: DocPosition, end: DocPosition) -> PosMap {
    let block = doc
        .block(start.block_index)
        .expect("DeleteRange: block_index out of bounds")
        .clone();

    let mut block = block;
    let runs = block
        .runs_mut()
        .expect("DeleteRange: block has no inline runs");

    let range = isolate_runs(runs, start.offset, end.offset);
    if range.start < range.end {
        runs.drain(range);
    }

    if runs.is_empty() {
        runs.push(StyledRun::plain(String::new()));
    }

    let old_len = end.offset - start.offset;
    doc.replace_block(start.block_index, block);

    PosMap {
        block_index: start.block_index,
        entries: vec![PosMapEntry {
            old_offset: start.offset,
            old_len,
            new_len: 0,
        }],
        structural: None,
    }
}

fn apply_cross_block_delete(doc: &mut Document, start: DocPosition, end: DocPosition) -> PosMap {
    // Collect the end block's tail (content after end.offset).
    let end_block = doc
        .block(end.block_index)
        .expect("DeleteRange: end block out of bounds")
        .clone();
    let end_tail_runs = extract_runs_from_offset(end_block.runs().unwrap_or(&[]), end.offset);

    // Modify start block: truncate at start.offset, append end tail.
    let start_block = doc
        .block(start.block_index)
        .expect("DeleteRange: start block out of bounds")
        .clone();
    let mut new_block = start_block;
    if let Some(runs) = new_block.runs_mut() {
        truncate_runs(runs, start.offset);
        runs.extend(end_tail_runs);
        if runs.is_empty() {
            runs.push(StyledRun::plain(String::new()));
        }
    }
    doc.replace_block(start.block_index, new_block);

    // Remove blocks from (start+1) through end (inclusive).
    let blocks_to_remove = end.block_index - start.block_index;
    for _ in 0..blocks_to_remove {
        doc.blocks.remove(start.block_index + 1);
    }

    if doc.blocks.is_empty() {
        doc.blocks.push(Arc::new(Block::empty_paragraph()));
    }

    PosMap {
        block_index: start.block_index,
        entries: vec![PosMapEntry {
            old_offset: start.offset,
            old_len: 0,
            new_len: 0,
        }],
        structural: if blocks_to_remove > 0 {
            Some(StructuralChange::CrossBlockDelete {
                start_block: start.block_index,
                removed_count: blocks_to_remove,
                start_offset: start.offset,
            })
        } else {
            None
        },
    }
}

/// Reconstruct deleted content (inverse of a delete).
///
/// **Single-block**: `deleted.blocks` has exactly 1 entry containing the deleted
/// runs. We re-insert those runs at `position.offset` within the existing block.
///
/// **Cross-block**: the document has one merged block at `position.block_index`
/// (start head + end tail). The `deleted.blocks` contain:
/// [0] = tail of original start block, [middle] = full middle blocks,
/// [last] = head of original end block.
/// We split the merged block at `position.offset`, reconstruct the original blocks,
/// and insert them.
fn apply_restore_deleted(
    doc: &mut Document,
    position: DocPosition,
    deleted: &DeletedContent,
) -> PosMap {
    let deleted_count = deleted.blocks.len();

    // Single-block restore: splice the deleted runs back at the insertion offset,
    // preserving their original styling (bold, italic, links, etc.).
    if deleted_count == 1 {
        let deleted_runs = deleted.blocks[0].runs().unwrap_or(&[]);
        let char_count: usize = deleted_runs.iter().map(StyledRun::char_len).sum();

        let block = doc
            .block(position.block_index)
            .expect("RestoreDeleted: block out of bounds")
            .clone();
        let mut block = block;
        if let Some(runs) = block.runs_mut() {
            let head = extract_runs_up_to(runs, position.offset);
            let tail = extract_runs_from_offset(runs, position.offset);
            let mut new_runs = head;
            new_runs.extend(deleted_runs.iter().cloned());
            new_runs.extend(tail);
            if new_runs.is_empty() {
                new_runs.push(StyledRun::plain(String::new()));
            }
            *runs = new_runs;
        }
        doc.replace_block(position.block_index, block);

        return PosMap {
            block_index: position.block_index,
            entries: vec![PosMapEntry {
                old_offset: position.offset,
                old_len: 0,
                new_len: char_count,
            }],
            structural: None,
        };
    }

    // Cross-block restore.
    let merged_block = doc
        .block(position.block_index)
        .expect("RestoreDeleted: block out of bounds")
        .clone();

    let merged_runs = merged_block.runs().unwrap_or(&[]);
    let head_runs = extract_runs_up_to(merged_runs, position.offset);
    let tail_runs = extract_runs_from_offset(merged_runs, position.offset);

    let mut new_blocks: Vec<Block> = Vec::with_capacity(deleted_count + 1);

    // Restored start block: head + deleted[0]'s runs.
    let first_deleted_runs = deleted.blocks[0].runs().unwrap_or(&[]);
    let mut start_runs = head_runs;
    start_runs.extend(first_deleted_runs.iter().cloned());
    if start_runs.is_empty() {
        start_runs.push(StyledRun::plain(String::new()));
    }
    new_blocks.push(match &merged_block {
        Block::Heading { level, .. } => Block::Heading {
            level: *level,
            runs: start_runs,
        },
        _ => Block::Paragraph { runs: start_runs },
    });

    // Middle blocks (indices 1..deleted_count-1).
    for block in &deleted.blocks[1..deleted_count - 1] {
        new_blocks.push(block.clone());
    }

    // Restored end block: deleted[last]'s runs + tail.
    let last_deleted = &deleted.blocks[deleted_count - 1];
    let last_deleted_runs = last_deleted.runs().unwrap_or(&[]);
    let mut end_runs: Vec<StyledRun> = last_deleted_runs.to_vec();
    end_runs.extend(tail_runs);
    if end_runs.is_empty() {
        end_runs.push(StyledRun::plain(String::new()));
    }
    new_blocks.push(match last_deleted {
        Block::Heading { level, .. } => Block::Heading {
            level: *level,
            runs: end_runs,
        },
        _ => Block::Paragraph { runs: end_runs },
    });

    // Replace the merged block with the restored blocks.
    doc.blocks.remove(position.block_index);
    for (i, block) in new_blocks.into_iter().enumerate() {
        doc.blocks.insert(position.block_index + i, Arc::new(block));
    }

    PosMap {
        block_index: position.block_index,
        entries: Vec::new(),
        structural: Some(StructuralChange::Insert {
            block_index: position.block_index,
            count: deleted_count,
        }),
    }
}

// ── Apply: SplitBlock ───────────────────────────────────

fn apply_split_block(doc: &mut Document, position: DocPosition) -> PosMap {
    let block = doc
        .block(position.block_index)
        .expect("SplitBlock: block out of bounds")
        .clone();

    let (first, second) = split_block_at(&block, position.offset);

    doc.replace_block(position.block_index, first);
    doc.blocks
        .insert(position.block_index + 1, Arc::new(second));

    PosMap {
        block_index: position.block_index,
        entries: Vec::new(),
        structural: Some(StructuralChange::Split {
            block_index: position.block_index,
            split_offset: position.offset,
        }),
    }
}

/// Split a block into two at a char offset. Both halves inherit the block's type.
fn split_block_at(block: &Block, offset: usize) -> (Block, Block) {
    match block {
        Block::Paragraph { runs } => {
            let (left, right) = split_runs_at_offset(runs, offset);
            (
                Block::Paragraph { runs: left },
                Block::Paragraph { runs: right },
            )
        }
        Block::Heading { level, runs } => {
            let (left, right) = split_runs_at_offset(runs, offset);
            (
                Block::Heading {
                    level: *level,
                    runs: left,
                },
                Block::Heading {
                    level: *level,
                    runs: right,
                },
            )
        }
        Block::ListItem {
            ordered,
            indent_level,
            runs,
        } => {
            let (left, right) = split_runs_at_offset(runs, offset);
            (
                Block::ListItem {
                    ordered: *ordered,
                    indent_level: *indent_level,
                    runs: left,
                },
                Block::ListItem {
                    ordered: *ordered,
                    indent_level: *indent_level,
                    runs: right,
                },
            )
        }
        _ => (block.clone(), Block::empty_paragraph()),
    }
}

/// Split a run list at a char offset, returning `(left, right)`.
/// Both sides are guaranteed to have at least one run.
fn split_runs_at_offset(runs: &[StyledRun], offset: usize) -> (Vec<StyledRun>, Vec<StyledRun>) {
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut pos = 0;
    let mut split_done = false;

    for run in runs {
        let run_len = run.char_len();
        if split_done {
            right.push(run.clone());
            continue;
        }
        if pos + run_len <= offset {
            left.push(run.clone());
            pos += run_len;
            continue;
        }
        if pos >= offset {
            right.push(run.clone());
            split_done = true;
            continue;
        }
        let local = offset - pos;
        let (l, r) = run.split_at(local);
        left.push(l);
        right.push(r);
        split_done = true;
    }

    if left.is_empty() {
        left.push(StyledRun::plain(String::new()));
    }
    if right.is_empty() {
        right.push(StyledRun::plain(String::new()));
    }

    (left, right)
}

// ── Apply: MergeBlocks ──────────────────────────────────

fn apply_merge_blocks(doc: &mut Document, block_index: usize, _merge_offset: usize) -> PosMap {
    assert!(
        block_index > 0,
        "MergeBlocks: cannot merge block 0 (no previous block)"
    );
    assert!(
        block_index < doc.block_count(),
        "MergeBlocks: block_index out of bounds"
    );

    let prev_block = doc
        .block(block_index - 1)
        .expect("MergeBlocks: prev block out of bounds")
        .clone();
    let current_block = doc
        .block(block_index)
        .expect("MergeBlocks: block out of bounds")
        .clone();

    let prev_char_len = prev_block.char_len();
    let merged = merge_two_blocks(&prev_block, &current_block);

    doc.replace_block(block_index - 1, merged);
    doc.blocks.remove(block_index);

    PosMap {
        block_index: block_index - 1,
        entries: vec![PosMapEntry {
            old_offset: prev_char_len,
            old_len: 0,
            new_len: 0,
        }],
        structural: Some(StructuralChange::Merge {
            block_index,
            merge_offset: prev_char_len,
        }),
    }
}

/// Merge two inline blocks. The result keeps the first block's type.
fn merge_two_blocks(first: &Block, second: &Block) -> Block {
    let first_runs = first.runs().unwrap_or(&[]);
    let second_runs = second.runs().unwrap_or(&[]);

    let mut merged: Vec<StyledRun> = first_runs.to_vec();
    merged.extend(second_runs.iter().cloned());

    if merged.is_empty() {
        merged.push(StyledRun::plain(String::new()));
    }

    match first {
        Block::Heading { level, .. } => Block::Heading {
            level: *level,
            runs: merged,
        },
        Block::ListItem {
            ordered,
            indent_level,
            ..
        } => Block::ListItem {
            ordered: *ordered,
            indent_level: *indent_level,
            runs: merged,
        },
        _ => Block::Paragraph { runs: merged },
    }
}

// ── Apply: ToggleInlineStyle ────────────────────────────

fn apply_toggle_inline_style(
    doc: &mut Document,
    start: DocPosition,
    end: DocPosition,
    style_bit: InlineStyle,
) -> PosMap {
    assert!(start < end, "ToggleInlineStyle: empty range");

    let should_remove = all_text_has_style(doc, start, end, style_bit);

    for bi in start.block_index..=end.block_index {
        let block = match doc.block(bi) {
            Some(b) if b.is_inline_block() => b.clone(),
            _ => continue,
        };

        let block_start = if bi == start.block_index {
            start.offset
        } else {
            0
        };
        let block_end = if bi == end.block_index {
            end.offset
        } else {
            block.char_len()
        };

        if block_start >= block_end {
            continue;
        }

        let mut block = block;
        if let Some(runs) = block.runs_mut() {
            let range = isolate_runs(runs, block_start, block_end);
            for run in &mut runs[range] {
                if should_remove {
                    run.style.remove(style_bit);
                } else {
                    run.style.insert(style_bit);
                }
            }
        }

        doc.replace_block(bi, block);
    }

    PosMap {
        block_index: start.block_index,
        entries: Vec::new(),
        structural: None,
    }
}

/// Check whether ALL non-empty text in `[start..end)` already has `style_bit`.
fn all_text_has_style(
    doc: &Document,
    start: DocPosition,
    end: DocPosition,
    style_bit: InlineStyle,
) -> bool {
    for bi in start.block_index..=end.block_index {
        let block = match doc.block(bi) {
            Some(b) => b,
            None => continue,
        };
        let runs = match block.runs() {
            Some(r) => r,
            None => continue,
        };

        let block_start = if bi == start.block_index {
            start.offset
        } else {
            0
        };
        let block_end = if bi == end.block_index {
            end.offset
        } else {
            block.char_len()
        };

        let mut pos = 0;
        for run in runs {
            let run_len = run.char_len();
            let run_end = pos + run_len;

            if run_end <= block_start {
                pos = run_end;
                continue;
            }
            if pos >= block_end {
                break;
            }

            if run_len > 0 && !run.style.contains(style_bit) {
                return false;
            }

            pos = run_end;
        }
    }
    true
}

// ── Apply: SetBlockType ─────────────────────────────────

fn apply_set_block_type(doc: &mut Document, block_index: usize, new: BlockKind) -> PosMap {
    let block = doc
        .block(block_index)
        .expect("SetBlockType: block out of bounds");

    // Extract inline runs from the source block. For container blocks
    // (BlockQuote) that have no direct runs, pull from the first child.
    let runs = match block {
        Block::BlockQuote { blocks } => blocks
            .first()
            .and_then(|b| b.runs())
            .unwrap_or(&[])
            .to_vec(),
        _ => block.runs().unwrap_or(&[]).to_vec(),
    };
    let flattened = block.flattened_text();
    let new_runs = if runs.is_empty() {
        vec![StyledRun::plain(String::new())]
    } else {
        runs
    };

    let new_block = match new {
        BlockKind::Paragraph => Block::Paragraph { runs: new_runs },
        BlockKind::Heading(level) => Block::Heading {
            level,
            runs: new_runs,
        },
        BlockKind::ListItem { ordered } => Block::ListItem {
            ordered,
            indent_level: 0,
            runs: new_runs,
        },
        BlockKind::BlockQuote => Block::BlockQuote {
            blocks: vec![Arc::new(Block::Paragraph { runs: new_runs })],
        },
        BlockKind::HorizontalRule => Block::HorizontalRule,
        BlockKind::Image => {
            // Converting to Image via SetBlockType doesn't make semantic sense,
            // but handle gracefully by creating a placeholder image with the
            // block's text as alt text.
            Block::Image {
                src: String::new(),
                alt: flattened,
                width: None,
                height: None,
            }
        }
    };

    doc.replace_block(block_index, new_block);

    PosMap {
        block_index,
        entries: Vec::new(),
        structural: None,
    }
}

// ── Apply: SetBlockAttrs ────────────────────────────────

fn apply_set_block_attrs(
    doc: &mut Document,
    block_index: usize,
    new: crate::document::BlockAttrs,
) -> PosMap {
    let block = doc
        .block(block_index)
        .expect("SetBlockAttrs: block out of bounds");

    if let Some(new_block) = block.with_attrs(new) {
        doc.replace_block(block_index, new_block);
    }

    PosMap {
        block_index,
        entries: Vec::new(),
        structural: None,
    }
}

// ── Apply: InsertBlock / RemoveBlock ────────────────────

fn apply_insert_block(doc: &mut Document, index: usize, block: &Block) -> PosMap {
    doc.insert_block(index, block.clone());

    PosMap {
        block_index: index,
        entries: Vec::new(),
        structural: Some(StructuralChange::Insert {
            block_index: index,
            count: 1,
        }),
    }
}

fn apply_remove_block(doc: &mut Document, index: usize) -> PosMap {
    doc.remove_block(index);

    PosMap {
        block_index: index,
        entries: Vec::new(),
        structural: Some(StructuralChange::Remove { block_index: index }),
    }
}

// ── Run range extraction ────────────────────────────────

/// Extract styled runs covering `[start_offset..end_offset)` from a run list.
/// Preserves style and link information. Returns at least one (possibly empty) run.
fn extract_runs_from_range(
    runs: &[StyledRun],
    start_offset: usize,
    end_offset: usize,
) -> Vec<StyledRun> {
    if start_offset >= end_offset {
        return vec![StyledRun::plain(String::new())];
    }
    let mut result = Vec::new();
    let mut pos = 0;
    for run in runs {
        let run_len = run.char_len();
        let run_start = pos;
        let run_end = pos + run_len;
        if run_end <= start_offset {
            pos = run_end;
            continue;
        }
        if run_start >= end_offset {
            break;
        }
        let overlap_start = start_offset.max(run_start) - run_start;
        let overlap_end = end_offset.min(run_end) - run_start;
        let byte_start = run.char_to_byte_offset(overlap_start);
        let byte_end = run.char_to_byte_offset(overlap_end);
        let text = run.text[byte_start..byte_end].to_owned();
        result.push(StyledRun {
            text,
            style: run.style,
            link: run.link.clone(),
        });
        pos = run_end;
    }
    if result.is_empty() {
        result.push(StyledRun::plain(String::new()));
    }
    result
}

// ── Invert helpers ──────────────────────────────────────

/// Invert a `DeleteRange`.
///
/// Always produces a sentinel `DeleteRange` with `start == end` and the original
/// deleted content. `apply_delete_range` detects this pattern and dispatches to
/// `apply_restore_deleted`, which faithfully reconstructs the original run structure
/// (preserving styles and links) for both single-block and cross-block deletes.
fn invert_delete_range(start: DocPosition, _end: DocPosition, deleted: &DeletedContent) -> EditOp {
    EditOp::DeleteRange {
        start,
        end: start,
        deleted: DeletedContent {
            blocks: deleted.blocks.clone(),
        },
    }
}

// ── Run utilities ───────────────────────────────────────

/// Extract runs from `offset` to end of the run list.
fn extract_runs_from_offset(runs: &[StyledRun], offset: usize) -> Vec<StyledRun> {
    let mut result = Vec::new();
    let mut pos = 0;
    for run in runs {
        let run_len = run.char_len();
        let run_end = pos + run_len;
        if run_end <= offset {
            pos = run_end;
            continue;
        }
        if pos >= offset {
            result.push(run.clone());
        } else {
            let local = offset - pos;
            let (_, right) = run.split_at(local);
            if !right.is_empty() {
                result.push(right);
            }
        }
        pos = run_end;
    }
    result
}

/// Extract runs from start up to (not including) `offset`.
fn extract_runs_up_to(runs: &[StyledRun], offset: usize) -> Vec<StyledRun> {
    let mut result = Vec::new();
    let mut pos = 0;
    for run in runs {
        let run_len = run.char_len();
        if pos >= offset {
            break;
        }
        if pos + run_len <= offset {
            result.push(run.clone());
        } else {
            let local = offset - pos;
            let (left, _) = run.split_at(local);
            if !left.is_empty() {
                result.push(left);
            }
        }
        pos += run_len;
    }
    result
}

/// Truncate a run list in-place, keeping only `[0..offset)`.
fn truncate_runs(runs: &mut Vec<StyledRun>, offset: usize) {
    let mut pos = 0;
    let mut trunc_idx = runs.len();
    for (i, run) in runs.iter_mut().enumerate() {
        let run_len = run.char_len();
        if pos + run_len <= offset {
            pos += run_len;
            continue;
        }
        if pos >= offset {
            trunc_idx = i;
            break;
        }
        let local = offset - pos;
        let (left, _) = run.split_at(local);
        *run = left;
        trunc_idx = i + 1;
        break;
    }
    runs.truncate(trunc_idx);
}

// ── Tests ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Block, DocPosition, Document, HeadingLevel, InlineStyle, StyledRun};

    fn block_text(doc: &Document, idx: usize) -> String {
        doc.block(idx)
            .map_or_else(String::new, Block::flattened_text)
    }

    // ── InsertText ──────────────────────────────────────

    #[test]
    fn insert_text_into_empty() {
        let mut doc = Document::new();
        let op = EditOp::InsertText {
            position: DocPosition::zero(),
            text: "hello".into(),
        };
        let pm = op.apply(&mut doc);
        assert_eq!(block_text(&doc, 0), "hello");
        assert_eq!(pm.entries[0].new_len, 5);
    }

    #[test]
    fn insert_text_mid_block() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("helo")]);
        EditOp::InsertText {
            position: DocPosition::new(0, 3),
            text: "l".into(),
        }
        .apply(&mut doc);
        assert_eq!(block_text(&doc, 0), "hello");
    }

    #[test]
    fn insert_text_preserves_other_blocks() {
        let mut doc =
            Document::from_blocks(vec![Block::paragraph("first"), Block::paragraph("second")]);
        let original_second = Arc::clone(&doc.blocks[1]);
        EditOp::InsertText {
            position: DocPosition::new(0, 5),
            text: "!".into(),
        }
        .apply(&mut doc);
        assert_eq!(block_text(&doc, 0), "first!");
        assert!(Arc::ptr_eq(&doc.blocks[1], &original_second));
    }

    #[test]
    fn insert_text_invert() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        let op = EditOp::InsertText {
            position: DocPosition::new(0, 5),
            text: " world".into(),
        };
        op.apply(&mut doc);
        assert_eq!(block_text(&doc, 0), "hello world");
        op.invert().apply(&mut doc);
        assert_eq!(block_text(&doc, 0), "hello");
    }

    #[test]
    fn insert_text_into_bold_undo_redo_preserves_style() {
        // Insert plain text into a bold run. The text inherits bold on apply.
        // Undo (via invert_with_doc) should capture the bold runs, so redo
        // (restore) splices bold runs back — not plain text.
        let mut doc = Document::from_blocks(vec![Block::Paragraph {
            runs: vec![StyledRun::styled("hello", InlineStyle::BOLD)],
        }]);

        let op = EditOp::InsertText {
            position: DocPosition::new(0, 5),
            text: " world".into(),
        };
        op.apply(&mut doc);
        assert_eq!(block_text(&doc, 0), "hello world");

        // Undo: invert_with_doc captures the actual bold runs from the doc.
        let inverse = op.invert_with_doc(&doc);
        inverse.apply(&mut doc);
        assert_eq!(block_text(&doc, 0), "hello");

        // Redo: invert the inverse → restore. The deleted content should
        // carry bold styling, so the restored text is bold, not plain.
        inverse.invert().apply(&mut doc);
        assert_eq!(block_text(&doc, 0), "hello world");
        let runs = doc.block(0).and_then(|b| b.runs()).expect("runs");
        for run in runs {
            if !run.is_empty() {
                assert!(
                    run.style.contains(InlineStyle::BOLD),
                    "run {:?} should be bold after redo",
                    run.text
                );
            }
        }
    }

    #[test]
    fn insert_text_unicode() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("caf\u{00e9}")]);
        EditOp::InsertText {
            position: DocPosition::new(0, 4),
            text: "!".into(),
        }
        .apply(&mut doc);
        assert_eq!(block_text(&doc, 0), "caf\u{00e9}!");
    }

    #[test]
    fn insert_into_styled_run() {
        let mut doc = Document::from_blocks(vec![Block::Paragraph {
            runs: vec![StyledRun::styled("hllo", InlineStyle::BOLD)],
        }]);
        EditOp::InsertText {
            position: DocPosition::new(0, 1),
            text: "e".into(),
        }
        .apply(&mut doc);
        let runs = doc.block(0).and_then(|b| b.runs()).expect("runs");
        assert_eq!(runs[0].text, "hello");
        assert_eq!(runs[0].style, InlineStyle::BOLD);
    }

    // ── DeleteRange (single block) ──────────────────────

    #[test]
    fn delete_single_block() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello world")]);
        EditOp::DeleteRange {
            start: DocPosition::new(0, 5),
            end: DocPosition::new(0, 11),
            deleted: DeletedContent {
                blocks: vec![Block::paragraph(" world")],
            },
        }
        .apply(&mut doc);
        assert_eq!(block_text(&doc, 0), "hello");
    }

    #[test]
    fn delete_single_block_invert() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello world")]);
        let op = EditOp::DeleteRange {
            start: DocPosition::new(0, 5),
            end: DocPosition::new(0, 11),
            deleted: DeletedContent {
                blocks: vec![Block::paragraph(" world")],
            },
        };
        op.apply(&mut doc);
        op.invert().apply(&mut doc);
        assert_eq!(block_text(&doc, 0), "hello world");
    }

    #[test]
    fn delete_entire_content() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("abc")]);
        EditOp::DeleteRange {
            start: DocPosition::new(0, 0),
            end: DocPosition::new(0, 3),
            deleted: DeletedContent {
                blocks: vec![Block::paragraph("abc")],
            },
        }
        .apply(&mut doc);
        assert_eq!(block_text(&doc, 0), "");
        assert_eq!(doc.block_count(), 1);
    }

    #[test]
    fn delete_preserves_styles() {
        let mut doc = Document::from_blocks(vec![Block::Paragraph {
            runs: vec![
                StyledRun::styled("aa", InlineStyle::BOLD),
                StyledRun::plain("xxx"),
                StyledRun::styled("bb", InlineStyle::ITALIC),
            ],
        }]);
        EditOp::DeleteRange {
            start: DocPosition::new(0, 2),
            end: DocPosition::new(0, 5),
            deleted: DeletedContent {
                blocks: vec![Block::Paragraph {
                    runs: vec![StyledRun::plain("xxx")],
                }],
            },
        }
        .apply(&mut doc);
        let runs = doc.block(0).and_then(|b| b.runs()).expect("runs");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].text, "aa");
        assert_eq!(runs[0].style, InlineStyle::BOLD);
        assert_eq!(runs[1].text, "bb");
        assert_eq!(runs[1].style, InlineStyle::ITALIC);
    }

    #[test]
    fn delete_unicode() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("\u{1f600}hello")]);
        EditOp::DeleteRange {
            start: DocPosition::new(0, 0),
            end: DocPosition::new(0, 1),
            deleted: DeletedContent {
                blocks: vec![Block::paragraph("\u{1f600}")],
            },
        }
        .apply(&mut doc);
        assert_eq!(block_text(&doc, 0), "hello");
    }

    // ── DeleteRange (cross-block) ───────────────────────

    #[test]
    fn delete_cross_two_blocks() {
        let mut doc =
            Document::from_blocks(vec![Block::paragraph("hello"), Block::paragraph(" world")]);
        EditOp::DeleteRange {
            start: DocPosition::new(0, 3),
            end: DocPosition::new(1, 4),
            deleted: DeletedContent {
                blocks: vec![Block::paragraph("lo"), Block::paragraph(" wor")],
            },
        }
        .apply(&mut doc);
        assert_eq!(doc.block_count(), 1);
        assert_eq!(block_text(&doc, 0), "helld");
    }

    #[test]
    fn delete_cross_with_middle() {
        let mut doc = Document::from_blocks(vec![
            Block::paragraph("aaa"),
            Block::paragraph("bbb"),
            Block::paragraph("ccc"),
        ]);
        EditOp::DeleteRange {
            start: DocPosition::new(0, 1),
            end: DocPosition::new(2, 2),
            deleted: DeletedContent {
                blocks: vec![
                    Block::paragraph("aa"),
                    Block::paragraph("bbb"),
                    Block::paragraph("cc"),
                ],
            },
        }
        .apply(&mut doc);
        assert_eq!(doc.block_count(), 1);
        assert_eq!(block_text(&doc, 0), "ac");
    }

    #[test]
    fn delete_cross_invert_reconstructs() {
        let mut doc = Document::from_blocks(vec![
            Block::paragraph("aaa"),
            Block::paragraph("bbb"),
            Block::paragraph("ccc"),
        ]);
        let op = EditOp::DeleteRange {
            start: DocPosition::new(0, 1),
            end: DocPosition::new(2, 2),
            deleted: DeletedContent {
                blocks: vec![
                    Block::paragraph("aa"),
                    Block::paragraph("bbb"),
                    Block::paragraph("cc"),
                ],
            },
        };
        op.apply(&mut doc);
        assert_eq!(block_text(&doc, 0), "ac");

        op.invert().apply(&mut doc);
        assert_eq!(doc.block_count(), 3);
        assert_eq!(block_text(&doc, 0), "aaa");
        assert_eq!(block_text(&doc, 1), "bbb");
        assert_eq!(block_text(&doc, 2), "ccc");
    }

    // ── SplitBlock ──────────────────────────────────────

    #[test]
    fn split_mid() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello world")]);
        EditOp::SplitBlock {
            position: DocPosition::new(0, 5),
        }
        .apply(&mut doc);
        assert_eq!(doc.block_count(), 2);
        assert_eq!(block_text(&doc, 0), "hello");
        assert_eq!(block_text(&doc, 1), " world");
    }

    #[test]
    fn split_at_start() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        EditOp::SplitBlock {
            position: DocPosition::new(0, 0),
        }
        .apply(&mut doc);
        assert_eq!(doc.block_count(), 2);
        assert_eq!(block_text(&doc, 0), "");
        assert_eq!(block_text(&doc, 1), "hello");
    }

    #[test]
    fn split_at_end() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        EditOp::SplitBlock {
            position: DocPosition::new(0, 5),
        }
        .apply(&mut doc);
        assert_eq!(doc.block_count(), 2);
        assert_eq!(block_text(&doc, 0), "hello");
        assert_eq!(block_text(&doc, 1), "");
    }

    #[test]
    fn split_heading_preserves_type() {
        let mut doc = Document::from_blocks(vec![Block::Heading {
            level: HeadingLevel::H1,
            runs: vec![StyledRun::plain("My Title")],
        }]);
        EditOp::SplitBlock {
            position: DocPosition::new(0, 3),
        }
        .apply(&mut doc);
        assert!(matches!(
            doc.block(0),
            Some(Block::Heading {
                level: HeadingLevel::H1,
                ..
            })
        ));
        assert!(matches!(
            doc.block(1),
            Some(Block::Heading {
                level: HeadingLevel::H1,
                ..
            })
        ));
        assert_eq!(block_text(&doc, 0), "My ");
        assert_eq!(block_text(&doc, 1), "Title");
    }

    #[test]
    fn split_invert_merges_back() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello world")]);
        let op = EditOp::SplitBlock {
            position: DocPosition::new(0, 5),
        };
        op.apply(&mut doc);
        op.invert().apply(&mut doc);
        assert_eq!(doc.block_count(), 1);
        assert_eq!(block_text(&doc, 0), "hello world");
    }

    #[test]
    fn split_empty() {
        let mut doc = Document::new();
        EditOp::SplitBlock {
            position: DocPosition::new(0, 0),
        }
        .apply(&mut doc);
        assert_eq!(doc.block_count(), 2);
    }

    // ── MergeBlocks ─────────────────────────────────────

    #[test]
    fn merge_basic() {
        let mut doc =
            Document::from_blocks(vec![Block::paragraph("hello"), Block::paragraph(" world")]);
        EditOp::MergeBlocks {
            block_index: 1,
            saved: Block::paragraph(" world"),
            merge_offset: 5,
        }
        .apply(&mut doc);
        assert_eq!(doc.block_count(), 1);
        assert_eq!(block_text(&doc, 0), "hello world");
    }

    #[test]
    fn merge_preserves_first_type() {
        let mut doc = Document::from_blocks(vec![
            Block::Heading {
                level: HeadingLevel::H2,
                runs: vec![StyledRun::plain("Title")],
            },
            Block::paragraph(" extra"),
        ]);
        EditOp::MergeBlocks {
            block_index: 1,
            saved: Block::paragraph(" extra"),
            merge_offset: 5,
        }
        .apply(&mut doc);
        assert!(matches!(
            doc.block(0),
            Some(Block::Heading {
                level: HeadingLevel::H2,
                ..
            })
        ));
        assert_eq!(block_text(&doc, 0), "Title extra");
    }

    #[test]
    fn merge_invert_splits_back() {
        let mut doc =
            Document::from_blocks(vec![Block::paragraph("hello"), Block::paragraph(" world")]);
        let op = EditOp::MergeBlocks {
            block_index: 1,
            saved: Block::paragraph(" world"),
            merge_offset: 5,
        };
        op.apply(&mut doc);
        op.invert().apply(&mut doc);
        assert_eq!(doc.block_count(), 2);
        assert_eq!(block_text(&doc, 0), "hello");
        assert_eq!(block_text(&doc, 1), " world");
    }

    #[test]
    fn merge_empty_blocks() {
        let mut doc =
            Document::from_blocks(vec![Block::empty_paragraph(), Block::empty_paragraph()]);
        EditOp::MergeBlocks {
            block_index: 1,
            saved: Block::empty_paragraph(),
            merge_offset: 0,
        }
        .apply(&mut doc);
        assert_eq!(doc.block_count(), 1);
    }

    // ── ToggleInlineStyle ───────────────────────────────

    #[test]
    fn toggle_bold_adds() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello world")]);
        EditOp::ToggleInlineStyle {
            start: DocPosition::new(0, 0),
            end: DocPosition::new(0, 5),
            style_bit: InlineStyle::BOLD,
        }
        .apply(&mut doc);

        let runs = doc.block(0).and_then(|b| b.runs()).expect("runs");
        let mut pos = 0;
        for run in runs {
            let rlen = run.char_len();
            if pos < 5 && pos + rlen > 0 && rlen > 0 {
                assert!(run.style.contains(InlineStyle::BOLD));
            }
            pos += rlen;
        }
    }

    #[test]
    fn toggle_bold_removes_when_all_bold() {
        let mut doc = Document::from_blocks(vec![Block::Paragraph {
            runs: vec![StyledRun::styled("hello", InlineStyle::BOLD)],
        }]);
        EditOp::ToggleInlineStyle {
            start: DocPosition::new(0, 0),
            end: DocPosition::new(0, 5),
            style_bit: InlineStyle::BOLD,
        }
        .apply(&mut doc);

        for run in doc.block(0).and_then(|b| b.runs()).expect("runs") {
            assert!(!run.style.contains(InlineStyle::BOLD));
        }
    }

    #[test]
    fn toggle_partial_run_overlap() {
        // "hello" bold + " world" plain. Toggle bold on [3..8).
        let mut doc = Document::from_blocks(vec![Block::Paragraph {
            runs: vec![
                StyledRun::styled("hello", InlineStyle::BOLD),
                StyledRun::plain(" world"),
            ],
        }]);
        EditOp::ToggleInlineStyle {
            start: DocPosition::new(0, 3),
            end: DocPosition::new(0, 8),
            style_bit: InlineStyle::BOLD,
        }
        .apply(&mut doc);

        let runs = doc.block(0).and_then(|b| b.runs()).expect("runs");
        let mut pos = 0;
        for run in runs {
            let rlen = run.char_len();
            let rend = pos + rlen;
            if pos < 8 && rend > 3 && rlen > 0 {
                assert!(
                    run.style.contains(InlineStyle::BOLD),
                    "run '{}' at [{pos}..{rend}) should be bold",
                    run.text,
                );
            }
            pos = rend;
        }
    }

    #[test]
    fn toggle_self_inverse() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        let op = EditOp::ToggleInlineStyle {
            start: DocPosition::new(0, 0),
            end: DocPosition::new(0, 5),
            style_bit: InlineStyle::BOLD,
        };
        op.apply(&mut doc);
        op.invert().apply(&mut doc);
        for run in doc.block(0).and_then(|b| b.runs()).expect("runs") {
            assert!(!run.style.contains(InlineStyle::BOLD));
        }
    }

    #[test]
    fn toggle_cross_block() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("aaa"), Block::paragraph("bbb")]);
        EditOp::ToggleInlineStyle {
            start: DocPosition::new(0, 1),
            end: DocPosition::new(1, 2),
            style_bit: InlineStyle::ITALIC,
        }
        .apply(&mut doc);

        // Block 0: "a" plain, "aa" italic.
        let mut pos = 0;
        for run in doc.block(0).and_then(|b| b.runs()).expect("runs") {
            if pos >= 1 && run.char_len() > 0 {
                assert!(run.style.contains(InlineStyle::ITALIC));
            }
            pos += run.char_len();
        }

        // Block 1: "bb" italic, "b" plain.
        pos = 0;
        for run in doc.block(1).and_then(|b| b.runs()).expect("runs") {
            if pos < 2 && run.char_len() > 0 {
                assert!(run.style.contains(InlineStyle::ITALIC));
            }
            pos += run.char_len();
        }
    }

    #[test]
    fn toggle_adds_to_all_runs() {
        let mut doc = Document::from_blocks(vec![Block::Paragraph {
            runs: vec![
                StyledRun::plain("aaa"),
                StyledRun::styled("bbb", InlineStyle::BOLD),
                StyledRun::plain("ccc"),
            ],
        }]);
        EditOp::ToggleInlineStyle {
            start: DocPosition::new(0, 0),
            end: DocPosition::new(0, 9),
            style_bit: InlineStyle::BOLD,
        }
        .apply(&mut doc);

        for run in doc.block(0).and_then(|b| b.runs()).expect("runs") {
            if !run.is_empty() {
                assert!(run.style.contains(InlineStyle::BOLD));
            }
        }
    }

    // ── SetBlockType ────────────────────────────────────

    #[test]
    fn set_type_paragraph_to_heading() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        EditOp::SetBlockType {
            block_index: 0,
            old: BlockKind::Paragraph,
            new: BlockKind::Heading(HeadingLevel::H1),
        }
        .apply(&mut doc);
        assert!(matches!(
            doc.block(0),
            Some(Block::Heading {
                level: HeadingLevel::H1,
                ..
            })
        ));
        assert_eq!(block_text(&doc, 0), "hello");
    }

    #[test]
    fn set_type_invert() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        let op = EditOp::SetBlockType {
            block_index: 0,
            old: BlockKind::Paragraph,
            new: BlockKind::Heading(HeadingLevel::H1),
        };
        op.apply(&mut doc);
        op.invert().apply(&mut doc);
        assert!(matches!(doc.block(0), Some(Block::Paragraph { .. })));
    }

    // ── InsertBlock / RemoveBlock ───────────────────────

    #[test]
    fn insert_block() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("first")]);
        EditOp::InsertBlock {
            index: 1,
            block: Block::paragraph("second"),
        }
        .apply(&mut doc);
        assert_eq!(doc.block_count(), 2);
        assert_eq!(block_text(&doc, 1), "second");
    }

    #[test]
    fn remove_block() {
        let mut doc =
            Document::from_blocks(vec![Block::paragraph("first"), Block::paragraph("second")]);
        EditOp::RemoveBlock {
            index: 1,
            saved: Block::paragraph("second"),
        }
        .apply(&mut doc);
        assert_eq!(doc.block_count(), 1);
    }

    #[test]
    fn insert_remove_inverse() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("first")]);
        let op = EditOp::InsertBlock {
            index: 1,
            block: Block::paragraph("second"),
        };
        op.apply(&mut doc);
        op.invert().apply(&mut doc);
        assert_eq!(doc.block_count(), 1);
    }

    #[test]
    fn remove_wont_remove_last() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("only")]);
        EditOp::RemoveBlock {
            index: 0,
            saved: Block::paragraph("only"),
        }
        .apply(&mut doc);
        assert_eq!(doc.block_count(), 1);
    }

    // ── PosMap::map ─────────────────────────────────────

    #[test]
    fn posmap_identity() {
        let pm = PosMap::identity();
        assert_eq!(pm.map(DocPosition::new(3, 7)), DocPosition::new(3, 7));
    }

    #[test]
    fn posmap_insert_shifts() {
        let pm = PosMap {
            block_index: 0,
            entries: vec![PosMapEntry {
                old_offset: 5,
                old_len: 0,
                new_len: 3,
            }],
            structural: None,
        };
        assert_eq!(pm.map(DocPosition::new(0, 3)), DocPosition::new(0, 3));
        assert_eq!(pm.map(DocPosition::new(0, 5)), DocPosition::new(0, 5));
        assert_eq!(pm.map(DocPosition::new(0, 7)), DocPosition::new(0, 10));
        assert_eq!(pm.map(DocPosition::new(1, 7)), DocPosition::new(1, 7));
    }

    #[test]
    fn posmap_delete_collapses() {
        let pm = PosMap {
            block_index: 0,
            entries: vec![PosMapEntry {
                old_offset: 3,
                old_len: 4,
                new_len: 0,
            }],
            structural: None,
        };
        assert_eq!(pm.map(DocPosition::new(0, 2)), DocPosition::new(0, 2));
        assert_eq!(pm.map(DocPosition::new(0, 5)), DocPosition::new(0, 3));
        assert_eq!(pm.map(DocPosition::new(0, 10)), DocPosition::new(0, 6));
    }

    #[test]
    fn posmap_split() {
        // Split block 1 at offset 5: "helloworld" -> "hello" | "world"
        let pm = PosMap {
            block_index: 1,
            entries: Vec::new(),
            structural: Some(StructuralChange::Split {
                block_index: 1,
                split_offset: 5,
            }),
        };
        // Block 0: unchanged
        assert_eq!(pm.map(DocPosition::new(0, 5)), DocPosition::new(0, 5));
        // Block 1, offset 3 (before split): unchanged
        assert_eq!(pm.map(DocPosition::new(1, 3)), DocPosition::new(1, 3));
        // Block 1, offset 5 (at split point): stays in block 1
        assert_eq!(pm.map(DocPosition::new(1, 5)), DocPosition::new(1, 5));
        // Block 1, offset 8 (after split): remapped to new block 2, offset 3
        assert_eq!(pm.map(DocPosition::new(1, 8)), DocPosition::new(2, 3));
        // Block 2: shifted to block 3
        assert_eq!(pm.map(DocPosition::new(2, 0)), DocPosition::new(3, 0));
    }

    #[test]
    fn posmap_split_at_zero() {
        // Split block 0 at offset 0: everything moves to block 1
        let pm = PosMap {
            block_index: 0,
            entries: Vec::new(),
            structural: Some(StructuralChange::Split {
                block_index: 0,
                split_offset: 0,
            }),
        };
        // Offset 0 stays (not > 0)
        assert_eq!(pm.map(DocPosition::new(0, 0)), DocPosition::new(0, 0));
        // Offset 3 moves to new block
        assert_eq!(pm.map(DocPosition::new(0, 3)), DocPosition::new(1, 3));
    }

    #[test]
    fn posmap_merge() {
        // Merge block 1 into block 0. Block 0 had length 5.
        let pm = PosMap {
            block_index: 0,
            entries: vec![PosMapEntry {
                old_offset: 5,
                old_len: 0,
                new_len: 0,
            }],
            structural: Some(StructuralChange::Merge {
                block_index: 1,
                merge_offset: 5,
            }),
        };
        // Block 0 position: unchanged
        assert_eq!(pm.map(DocPosition::new(0, 3)), DocPosition::new(0, 3));
        // Block 1, offset 2: remapped to block 0, offset 2 + 5 = 7
        assert_eq!(pm.map(DocPosition::new(1, 2)), DocPosition::new(0, 7));
        // Block 2: shifted down to block 1
        assert_eq!(pm.map(DocPosition::new(2, 0)), DocPosition::new(1, 0));
    }

    #[test]
    fn posmap_merge_empty_prev() {
        // Merge block 1 into block 0, where block 0 was empty (length 0).
        let pm = PosMap {
            block_index: 0,
            entries: vec![PosMapEntry {
                old_offset: 0,
                old_len: 0,
                new_len: 0,
            }],
            structural: Some(StructuralChange::Merge {
                block_index: 1,
                merge_offset: 0,
            }),
        };
        // Block 1, offset 3: remapped to block 0, offset 3 + 0 = 3
        assert_eq!(pm.map(DocPosition::new(1, 3)), DocPosition::new(0, 3));
    }

    #[test]
    fn posmap_insert_block() {
        let pm = PosMap {
            block_index: 2,
            entries: Vec::new(),
            structural: Some(StructuralChange::Insert {
                block_index: 2,
                count: 1,
            }),
        };
        assert_eq!(pm.map(DocPosition::new(1, 5)), DocPosition::new(1, 5));
        assert_eq!(pm.map(DocPosition::new(2, 0)), DocPosition::new(3, 0));
    }

    #[test]
    fn posmap_insert_multiple_blocks() {
        let pm = PosMap {
            block_index: 1,
            entries: Vec::new(),
            structural: Some(StructuralChange::Insert {
                block_index: 1,
                count: 3,
            }),
        };
        assert_eq!(pm.map(DocPosition::new(0, 5)), DocPosition::new(0, 5));
        assert_eq!(pm.map(DocPosition::new(1, 0)), DocPosition::new(4, 0));
    }

    #[test]
    fn posmap_remove_block() {
        let pm = PosMap {
            block_index: 2,
            entries: Vec::new(),
            structural: Some(StructuralChange::Remove { block_index: 2 }),
        };
        assert_eq!(pm.map(DocPosition::new(1, 5)), DocPosition::new(1, 5));
        assert_eq!(pm.map(DocPosition::new(2, 3)), DocPosition::new(2, 0));
        assert_eq!(pm.map(DocPosition::new(3, 5)), DocPosition::new(2, 5));
    }

    #[test]
    fn posmap_cross_block_delete() {
        // Delete from (0, 3) spanning 2 blocks removed.
        let pm = PosMap {
            block_index: 0,
            entries: vec![PosMapEntry {
                old_offset: 3,
                old_len: 0,
                new_len: 0,
            }],
            structural: Some(StructuralChange::CrossBlockDelete {
                start_block: 0,
                removed_count: 2,
                start_offset: 3,
            }),
        };
        // Before deletion point in start block: unchanged
        assert_eq!(pm.map(DocPosition::new(0, 2)), DocPosition::new(0, 2));
        // Position in deleted block 1: collapses to deletion point (0, 3)
        assert_eq!(pm.map(DocPosition::new(1, 5)), DocPosition::new(0, 3));
        // Position in deleted block 2: collapses to deletion point (0, 3)
        assert_eq!(pm.map(DocPosition::new(2, 5)), DocPosition::new(0, 3));
        // Block 3 (after deleted range): shifted down by removed_count
        assert_eq!(pm.map(DocPosition::new(3, 1)), DocPosition::new(1, 1));
    }

    #[test]
    fn posmap_cross_block_delete_mid_document() {
        // Delete from (1, 2) spanning 1 block removed (block 2 merged into block 1).
        let pm = PosMap {
            block_index: 1,
            entries: vec![PosMapEntry {
                old_offset: 2,
                old_len: 0,
                new_len: 0,
            }],
            structural: Some(StructuralChange::CrossBlockDelete {
                start_block: 1,
                removed_count: 1,
                start_offset: 2,
            }),
        };
        // Block 0: unchanged
        assert_eq!(pm.map(DocPosition::new(0, 5)), DocPosition::new(0, 5));
        // Block 1, before deletion: unchanged
        assert_eq!(pm.map(DocPosition::new(1, 1)), DocPosition::new(1, 1));
        // Block 2 (deleted): collapses to (1, 2)
        assert_eq!(pm.map(DocPosition::new(2, 7)), DocPosition::new(1, 2));
        // Block 3: shifted down
        assert_eq!(pm.map(DocPosition::new(3, 0)), DocPosition::new(2, 0));
    }

    // ── Round-trip tests ────────────────────────────────

    #[test]
    fn round_trip_insert() {
        let original = Document::from_blocks(vec![Block::paragraph("hello")]);
        let mut doc = original.clone();
        let op = EditOp::InsertText {
            position: DocPosition::new(0, 5),
            text: " world".into(),
        };
        op.apply(&mut doc);
        op.invert().apply(&mut doc);
        assert_eq!(doc.flattened_text(), original.flattened_text());
    }

    #[test]
    fn round_trip_split_merge() {
        let original = Document::from_blocks(vec![Block::paragraph("hello world")]);
        let mut doc = original.clone();
        let op = EditOp::SplitBlock {
            position: DocPosition::new(0, 5),
        };
        op.apply(&mut doc);
        op.invert().apply(&mut doc);
        assert_eq!(doc.block_count(), 1);
        assert_eq!(doc.flattened_text(), original.flattened_text());
    }

    #[test]
    fn round_trip_merge_split() {
        let original =
            Document::from_blocks(vec![Block::paragraph("hello"), Block::paragraph(" world")]);
        let mut doc = original.clone();
        let op = EditOp::MergeBlocks {
            block_index: 1,
            saved: Block::paragraph(" world"),
            merge_offset: 5,
        };
        op.apply(&mut doc);
        op.invert().apply(&mut doc);
        assert_eq!(doc.block_count(), 2);
        assert_eq!(block_text(&doc, 0), "hello");
        assert_eq!(block_text(&doc, 1), " world");
    }

    #[test]
    fn round_trip_set_type() {
        let original = Document::from_blocks(vec![Block::paragraph("hello")]);
        let mut doc = original.clone();
        let op = EditOp::SetBlockType {
            block_index: 0,
            old: BlockKind::Paragraph,
            new: BlockKind::Heading(HeadingLevel::H2),
        };
        op.apply(&mut doc);
        op.invert().apply(&mut doc);
        assert!(matches!(doc.block(0), Some(Block::Paragraph { .. })));
        assert_eq!(doc.flattened_text(), original.flattened_text());
    }

    #[test]
    fn round_trip_insert_remove_block() {
        let original = Document::from_blocks(vec![Block::paragraph("first")]);
        let mut doc = original.clone();
        let op = EditOp::InsertBlock {
            index: 1,
            block: Block::paragraph("inserted"),
        };
        op.apply(&mut doc);
        op.invert().apply(&mut doc);
        assert_eq!(doc.block_count(), 1);
        assert_eq!(doc.flattened_text(), original.flattened_text());
    }

    // ── Cursor stability ────────────────────────────────

    #[test]
    fn cursor_stable_after_insert_before() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello world")]);
        let pm = EditOp::InsertText {
            position: DocPosition::new(0, 3),
            text: "xyz".into(),
        }
        .apply(&mut doc);
        assert_eq!(pm.map(DocPosition::new(0, 8)), DocPosition::new(0, 11));
    }

    #[test]
    fn cursor_stable_after_delete_before() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello world")]);
        let pm = EditOp::DeleteRange {
            start: DocPosition::new(0, 1),
            end: DocPosition::new(0, 4),
            deleted: DeletedContent {
                blocks: vec![Block::paragraph("ell")],
            },
        }
        .apply(&mut doc);
        assert_eq!(pm.map(DocPosition::new(0, 8)), DocPosition::new(0, 5));
    }

    #[test]
    fn cursor_within_delete_collapses() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello world")]);
        let pm = EditOp::DeleteRange {
            start: DocPosition::new(0, 2),
            end: DocPosition::new(0, 7),
            deleted: DeletedContent {
                blocks: vec![Block::paragraph("llo w")],
            },
        }
        .apply(&mut doc);
        assert_eq!(pm.map(DocPosition::new(0, 5)), DocPosition::new(0, 2));
    }

    // ── SetBlockAttrs ─────────────────────────────────────

    #[test]
    fn set_block_attrs_changes_indent_level() {
        use crate::document::BlockAttrs;

        let mut doc = Document::from_blocks(vec![Block::list_item("item", false)]);
        let old = doc.block(0).map(Block::attrs).unwrap_or_default();
        assert_eq!(old.indent_level, 0);

        EditOp::SetBlockAttrs {
            block_index: 0,
            old,
            new: BlockAttrs {
                indent_level: 2,
                ..old
            },
        }
        .apply(&mut doc);

        match doc.block(0) {
            Some(Block::ListItem { indent_level, .. }) => {
                assert_eq!(*indent_level, 2);
            }
            other => panic!("Expected ListItem, got {other:?}"),
        }
    }

    #[test]
    fn set_block_attrs_invert_restores() {
        use crate::document::BlockAttrs;

        let mut doc = Document::from_blocks(vec![Block::list_item_with_indent("item", false, 1)]);
        let old = BlockAttrs {
            indent_level: 1,
            ..Default::default()
        };
        let new = BlockAttrs {
            indent_level: 3,
            ..Default::default()
        };
        let op = EditOp::SetBlockAttrs {
            block_index: 0,
            old,
            new,
        };
        op.apply(&mut doc);
        assert_eq!(
            doc.block(0).map(Block::attrs).map(|a| a.indent_level),
            Some(3)
        );

        op.invert().apply(&mut doc);
        assert_eq!(
            doc.block(0).map(Block::attrs).map(|a| a.indent_level),
            Some(1)
        );
    }

    #[test]
    fn set_block_attrs_no_op_for_non_list() {
        use crate::document::BlockAttrs;

        let mut doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        let old = BlockAttrs::default();
        let new = BlockAttrs {
            indent_level: 2,
            ..Default::default()
        };
        EditOp::SetBlockAttrs {
            block_index: 0,
            old,
            new,
        }
        .apply(&mut doc);

        // Paragraph doesn't have indent_level, so attrs remain default
        assert_eq!(doc.block(0).map(Block::attrs), Some(BlockAttrs::default()));
    }

    #[test]
    fn set_block_attrs_posmap_is_no_op() {
        use crate::document::BlockAttrs;

        let mut doc = Document::from_blocks(vec![Block::list_item("test", false)]);
        let pm = EditOp::SetBlockAttrs {
            block_index: 0,
            old: BlockAttrs::default(),
            new: BlockAttrs {
                indent_level: 1,
                ..Default::default()
            },
        }
        .apply(&mut doc);

        // SetBlockAttrs should not produce structural changes.
        assert!(pm.structural.is_none());
        assert!(pm.entries.is_empty());
    }
}
