//! Editor state: the application-owned mutable state for the rich text editor.
//!
//! Contains the document, selection, undo/redo history, and performs all
//! editing actions (insert, delete, paste, format toggle, movement, clipboard).
//! This module is renderer-agnostic - it has no dependency on iced's widget or
//! paragraph types.

use crate::document::{
    Block, DocPosition, DocSelection, DocSlice, Document, InlineStyle, StyledRun,
};
use crate::html_parse::from_html;
use crate::html_serialize::to_html;
use crate::normalize::{normalize, normalize_blocks};
use crate::operations::EditOp;
use crate::rules::{self, EditAction};
use crate::undo::UndoStack;

use super::cursor::{CursorState, DragState};
use super::input::{self, MoveAction};

// ── Action (events emitted by the widget) ───────────────

/// An action emitted by the rich text editor widget to the application.
///
/// The application should call [`EditorState::perform`] in its `update` method
/// for each action received.
#[derive(Debug, Clone)]
pub enum Action {
    /// An editing action (text input, delete, format toggle, etc.).
    Edit(EditAction),
    /// A cursor movement (the selection should be collapsed to the new position).
    Move(MoveAction),
    /// A selection extension (move focus, keep anchor).
    Select(MoveAction),
    /// Select all text in the document.
    SelectAll,
    /// Undo the last action.
    Undo,
    /// Redo the last undone action.
    Redo,
    /// Copy the current selection to the clipboard.
    Copy,
    /// Cut the current selection to the clipboard.
    Cut,
    /// Paste text from the clipboard.
    Paste(String),
    /// A click at a document position (resolved by the widget via hit testing).
    Click(DocPosition),
    /// A double-click at a document position (select word).
    DoubleClick(DocPosition),
    /// A triple-click at a document position (select block/line).
    TripleClick(DocPosition),
    /// A drag to a document position (extends selection).
    Drag(DocPosition),
    /// A link was clicked.
    LinkClicked(String),
    /// The editor gained focus.
    Focus,
    /// The editor lost focus.
    Blur,
}

// ── Internal clipboard ───────────────────────────────────

/// A structured clipboard entry captured from an internal copy/cut operation.
///
/// Stores the `DocSlice` alongside the plain-text representation that was
/// written to the system clipboard. On paste, if the system clipboard text
/// still matches `plain_text`, the structured `slice` is used instead of
/// plain-text insertion -- preserving block structure and inline formatting.
#[derive(Debug, Clone)]
struct InternalClipboard {
    /// The structured document slice that was copied.
    slice: DocSlice,
    /// The plain-text that was written to the system clipboard at copy time.
    plain_text: String,
}

// ── EditorState (application-owned mutable state) ───────

/// The mutable state of a rich text editor, owned by the application.
///
/// The widget renders this state immutably; mutations happen in the app's
/// `update()` via [`EditorState::perform`].
#[derive(Debug, Clone)]
pub struct EditorState {
    /// The document being edited.
    pub document: Document,
    /// Current cursor / selection.
    pub selection: DocSelection,
    /// Undo/redo history.
    pub undo_stack: UndoStack,
    /// Pending inline style (for typing at a collapsed caret after toggling
    /// a format shortcut before typing).
    pub pending_style: InlineStyle,
    /// Visual cursor state (blink, focus).
    pub(super) cursor: CursorState,
    /// Active mouse drag state.
    pub(super) drag: Option<DragState>,
    /// Internal clipboard: stores a structured slice from the last copy/cut
    /// within this editor, enabling formatted paste.
    internal_clipboard: Option<InternalClipboard>,
}

impl EditorState {
    /// Create a new editor state with an empty document.
    pub fn new() -> Self {
        Self {
            document: Document::new(),
            selection: DocSelection::caret(DocPosition::zero()),
            undo_stack: UndoStack::default(),
            pending_style: InlineStyle::empty(),
            cursor: CursorState::new(),
            drag: None,
            internal_clipboard: None,
        }
    }

    /// Create an editor state from an existing document.
    pub fn from_document(doc: Document) -> Self {
        Self {
            document: doc,
            selection: DocSelection::caret(DocPosition::zero()),
            undo_stack: UndoStack::default(),
            pending_style: InlineStyle::empty(),
            cursor: CursorState::new(),
            drag: None,
            internal_clipboard: None,
        }
    }

    /// Create an editor state by parsing HTML.
    pub fn from_html(html: &str) -> Self {
        Self::from_document(from_html(html))
    }

    /// Serialize the document to HTML.
    pub fn to_html(&self) -> String {
        to_html(&self.document)
    }

    /// Get the plain text of the current selection, or an empty string
    /// if the selection is collapsed.
    pub fn selection_text(&self) -> String {
        if self.selection.is_collapsed() {
            return String::new();
        }
        let start = self.selection.start();
        let end = self.selection.end();
        self.document
            .slice(start, end)
            .map_or_else(String::new, |slice| {
                let mut buf = String::new();
                for (i, block) in slice.blocks.iter().enumerate() {
                    if i > 0 {
                        buf.push('\n');
                    }
                    buf.push_str(&block.flattened_text());
                }
                buf
            })
    }

    /// Set the selection explicitly.
    pub fn set_selection(&mut self, sel: DocSelection) {
        self.selection = sel;
        self.cursor.reset_blink();
        self.cursor.clear_target_x();
        self.cursor.clear_target_column();
    }

    /// Whether the editor currently has focus.
    pub fn is_focused(&self) -> bool {
        self.cursor.is_focused()
    }

    /// Perform an action (called by the app in its `update` method).
    pub fn perform(&mut self, action: Action) {
        match action {
            Action::Edit(edit_action) => self.apply_action(edit_action),
            Action::Move(move_action) => self.apply_move(move_action, false),
            Action::Select(move_action) => self.apply_move(move_action, true),
            Action::SelectAll => {
                let end = self.document.end_position();
                self.selection = DocSelection::range(DocPosition::zero(), end);
                self.cursor.reset_blink();
            }
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            Action::Copy | Action::Cut => {
                // Capture the structured slice into the internal clipboard.
                if !self.selection.is_collapsed() {
                    let plain_text = self.selection_text();
                    let start = self.selection.start();
                    let end = self.selection.end();
                    if let Some(slice) = self.document.slice(start, end) {
                        self.internal_clipboard = Some(InternalClipboard { slice, plain_text });
                    }
                }
                // The widget's update() method writes to the system clipboard.
                // For Cut, also delete the selection.
                if matches!(action, Action::Cut) && !self.selection.is_collapsed() {
                    self.apply_action(EditAction::DeleteSelection);
                }
            }
            Action::Paste(text) => {
                // Check if we have a structured internal clipboard whose
                // plain text matches what came from the system clipboard.
                // If so, paste with structure preservation; otherwise fall
                // back to plain-text insertion.
                let structured_slice = self.internal_clipboard.as_ref().and_then(|ic| {
                    if ic.plain_text == text {
                        Some(ic.slice.clone())
                    } else {
                        None
                    }
                });
                if let Some(slice) = structured_slice {
                    self.paste_slice(&slice);
                } else {
                    self.apply_action(EditAction::InsertText(text));
                }
            }
            Action::Click(doc_pos) => {
                self.handle_click(doc_pos);
            }
            Action::DoubleClick(doc_pos) => {
                self.handle_double_click(doc_pos);
            }
            Action::TripleClick(doc_pos) => {
                self.handle_triple_click(doc_pos);
            }
            Action::Drag(doc_pos) => {
                self.handle_drag(doc_pos);
            }
            Action::LinkClicked(_) => {
                // The app handles this in its update() method.
            }
            Action::Focus => {
                self.cursor.focus();
            }
            Action::Blur => {
                self.cursor.unfocus();
                self.drag = None;
            }
        }
    }

    /// Apply a high-level edit action through the rules engine.
    pub fn apply_action(&mut self, action: EditAction) {
        let cursor_before = self.selection;

        // Special case: ToggleInlineStyle at a collapsed caret toggles pending style.
        if let EditAction::ToggleInlineStyle(style) = &action
            && self.selection.is_collapsed()
        {
            self.pending_style.toggle(*style);
            self.undo_stack.break_group();
            return;
        }

        let ops = rules::resolve(&self.document, self.selection, action, self.pending_style);

        if ops.is_empty() {
            return;
        }

        // Apply all ops.
        let mut dirty_blocks: Vec<usize> = Vec::new();
        for op in &ops {
            let pos_map = op.apply(&mut self.document);
            // Track dirty blocks for normalization.
            dirty_blocks.push(pos_map.block_index);
            // Map the selection through the edit.
            self.selection.anchor = pos_map.map(self.selection.anchor);
            self.selection.focus = pos_map.map(self.selection.focus);
        }

        // Compute cursor_after based on the operation type.
        // For insert: cursor moves past inserted text.
        // For delete: cursor collapses to start.
        // For split: cursor at start of new block.
        // For merge: cursor at merge point.
        self.update_cursor_after_ops(&ops);

        // Normalize dirty blocks.
        dirty_blocks.sort_unstable();
        dirty_blocks.dedup();
        normalize_blocks(&mut self.document, &dirty_blocks);

        // Clamp selection to valid bounds after normalization.
        self.selection.anchor = self.document.clamp_position(self.selection.anchor);
        self.selection.focus = self.document.clamp_position(self.selection.focus);

        let cursor_after = self.selection;

        // Push to undo stack.
        self.undo_stack.push(ops, cursor_before, cursor_after);

        // Clear pending style after any edit (it's consumed by the insertion).
        self.pending_style = InlineStyle::empty();

        // Reset blink on edit.
        self.cursor.reset_blink();
        self.cursor.clear_target_x();
        self.cursor.clear_target_column();
    }

    /// Paste a structured `DocSlice` into the document, preserving block
    /// structure and inline formatting.
    ///
    /// The algorithm:
    /// 1. Delete the current selection (if non-collapsed).
    /// 2. For a single-block slice with `open_start && open_end` (inline
    ///    fragment): insert each run's text individually, applying style
    ///    toggles where the run style differs from what would be inherited.
    /// 3. For multi-block slices: split the current block at the cursor,
    ///    merge the first slice block's runs into the left half, insert
    ///    middle blocks, and merge the last slice block's runs into the
    ///    right half.
    fn paste_slice(&mut self, slice: &DocSlice) {
        if slice.blocks.is_empty() {
            return;
        }

        let cursor_before = self.selection;

        // Step 1: delete selection if non-collapsed.
        let mut all_ops: Vec<EditOp> = Vec::new();
        let insert_pos = if !self.selection.is_collapsed() {
            let delete_ops = rules::resolve(
                &self.document,
                self.selection,
                EditAction::DeleteSelection,
                InlineStyle::empty(),
            );
            for op in &delete_ops {
                let pos_map = op.apply(&mut self.document);
                self.selection.anchor = pos_map.map(self.selection.anchor);
                self.selection.focus = pos_map.map(self.selection.focus);
            }
            let pos = self.selection.start();
            self.selection = DocSelection::caret(pos);
            all_ops.extend(delete_ops);
            pos
        } else {
            self.selection.focus
        };

        // Step 2: determine paste strategy based on slice shape.
        if slice.blocks.len() == 1 && slice.open_start && slice.open_end {
            // Inline fragment: insert runs preserving their styles.
            self.paste_inline_runs(insert_pos, &slice.blocks[0], &mut all_ops);
        } else if slice.blocks.iter().all(|b| !b.is_inline_block()) {
            // All blocks are non-inline (Image, HR, etc.) - insert as
            // complete blocks. These have no runs to merge.
            self.paste_complete_blocks(insert_pos, &slice.blocks, &mut all_ops);
        } else {
            // Multi-block paste: split, insert blocks, merge edges.
            self.paste_multi_block(insert_pos, slice, &mut all_ops);
        }

        // Normalize the entire document after paste.
        normalize(&mut self.document);

        // Clamp selection.
        self.selection.anchor = self.document.clamp_position(self.selection.anchor);
        self.selection.focus = self.document.clamp_position(self.selection.focus);

        let cursor_after = self.selection;

        // Push to undo stack as one group.
        if !all_ops.is_empty() {
            self.undo_stack.push(all_ops, cursor_before, cursor_after);
        }

        self.pending_style = InlineStyle::empty();
        self.cursor.reset_blink();
        self.cursor.clear_target_x();
        self.cursor.clear_target_column();
    }

    /// Paste an inline fragment (single block, both ends open) at `pos`.
    ///
    /// Inserts each run's text individually, applying style toggles where
    /// the run's style differs from the inherited style at the insertion
    /// point.
    fn paste_inline_runs(&mut self, pos: DocPosition, source_block: &Block, ops: &mut Vec<EditOp>) {
        let Some(paste_runs) = source_block.runs() else {
            return;
        };
        let paste_runs: Vec<StyledRun> = paste_runs
            .iter()
            .filter(|r| !r.text.is_empty())
            .cloned()
            .collect();
        if paste_runs.is_empty() {
            return;
        }

        // Block-swap strategy: clone the target block, splice in the pasted
        // runs (with correct styles AND links), then swap via RemoveBlock +
        // InsertBlock. Both ops are recorded, so redo replays correctly.
        splice_runs_into_block(
            &mut self.document,
            pos.block_index,
            pos.offset,
            &paste_runs,
            ops,
        );

        let total_chars: usize = paste_runs.iter().map(StyledRun::char_len).sum();
        self.selection =
            DocSelection::caret(DocPosition::new(pos.block_index, pos.offset + total_chars));
    }

    /// Insert atomic blocks like images and horizontal rules as complete blocks.
    fn paste_complete_blocks(&mut self, pos: DocPosition, blocks: &[Block], ops: &mut Vec<EditOp>) {
        // If the cursor is mid-block, split first so blocks insert cleanly.
        let mut insert_idx = if pos.offset > 0 {
            let split_op = EditOp::SplitBlock { position: pos };
            split_op.apply(&mut self.document);
            ops.push(split_op);
            pos.block_index + 1
        } else {
            pos.block_index
        };

        for block in blocks {
            let insert_op = EditOp::InsertBlock {
                index: insert_idx,
                block: block.clone(),
            };
            insert_op.apply(&mut self.document);
            ops.push(insert_op);
            insert_idx += 1;
        }

        // Place cursor at the start of the block after the last inserted block.
        let cursor_idx = insert_idx.min(self.document.block_count().saturating_sub(1));
        self.selection = DocSelection::caret(DocPosition::new(cursor_idx, 0));
    }

    /// Paste a multi-block slice at `pos`, merging edges and inserting middles.
    fn paste_multi_block(&mut self, pos: DocPosition, slice: &DocSlice, ops: &mut Vec<EditOp>) {
        let block_count = slice.blocks.len();

        // Split at cursor to create left and right halves.
        let split_op = EditOp::SplitBlock { position: pos };
        split_op.apply(&mut self.document);
        ops.push(split_op);

        // After split: left half at pos.block_index, right half at pos.block_index + 1.
        let left_idx = pos.block_index;

        // Merge first slice block into the left half.
        // If the first block is inline, merge its runs. Otherwise insert as a
        // complete block (e.g., Image, HR).
        let mut insert_idx = pos.block_index + 1;
        if let Some(runs) = slice.blocks[0].runs() {
            self.append_runs_to_block(left_idx, runs, ops);
        } else {
            // Non-inline first block: insert it after the left half.
            let insert_op = EditOp::InsertBlock {
                index: insert_idx,
                block: slice.blocks[0].clone(),
            };
            insert_op.apply(&mut self.document);
            ops.push(insert_op);
            insert_idx += 1;
        }

        // Insert middle blocks (indices 1..block_count-1).
        if block_count > 2 {
            for block in &slice.blocks[1..block_count - 1] {
                let insert_op = EditOp::InsertBlock {
                    index: insert_idx,
                    block: block.clone(),
                };
                insert_op.apply(&mut self.document);
                ops.push(insert_op);
                insert_idx += 1;
            }
        }

        // Handle the last block.
        let cursor_block;
        let cursor_offset;

        if block_count > 1 {
            let last_block = &slice.blocks[block_count - 1];
            let right_idx = insert_idx;
            if let Some(runs) = last_block.runs() {
                // Inline last block: merge runs into the right half (prepend).
                let pasted_len: usize = runs.iter().map(StyledRun::char_len).sum();
                self.prepend_runs_to_block(right_idx, runs, ops);
                cursor_block = right_idx;
                cursor_offset = pasted_len;
            } else {
                // Non-inline last block: insert as a complete block.
                let insert_op = EditOp::InsertBlock {
                    index: right_idx,
                    block: last_block.clone(),
                };
                insert_op.apply(&mut self.document);
                ops.push(insert_op);
                cursor_block = right_idx + 1;
                cursor_offset = 0;
            }
        } else {
            cursor_block = left_idx;
            cursor_offset = self.document.block(left_idx).map_or(0, Block::char_len);
        }

        self.selection = DocSelection::caret(DocPosition::new(cursor_block, cursor_offset));
    }

    /// Append styled runs to the end of a block, preserving formatting + links.
    ///
    /// Uses the block-swap strategy (RemoveBlock + InsertBlock) so all changes
    /// are captured as ops and survive redo.
    fn append_runs_to_block(
        &mut self,
        block_idx: usize,
        runs: &[StyledRun],
        ops: &mut Vec<EditOp>,
    ) {
        let non_empty: Vec<StyledRun> = runs.iter().filter(|r| !r.is_empty()).cloned().collect();
        if non_empty.is_empty() {
            return;
        }
        let offset = self.document.block(block_idx).map_or(0, Block::char_len);
        splice_runs_into_block(&mut self.document, block_idx, offset, &non_empty, ops);
    }

    /// Prepend styled runs to the start of a block, preserving formatting + links.
    fn prepend_runs_to_block(
        &mut self,
        block_idx: usize,
        runs: &[StyledRun],
        ops: &mut Vec<EditOp>,
    ) {
        let non_empty: Vec<StyledRun> = runs.iter().filter(|r| !r.is_empty()).cloned().collect();
        if non_empty.is_empty() {
            return;
        }
        splice_runs_into_block(&mut self.document, block_idx, 0, &non_empty, ops);
    }

    /// Update the cursor position after applying ops.
    fn update_cursor_after_ops(&mut self, ops: &[EditOp]) {
        // Use the last op to determine final cursor position.
        let Some(last_op) = ops.last() else {
            return;
        };

        match last_op {
            EditOp::InsertText { position, text } => {
                let char_count = text.chars().count();
                let new_pos = DocPosition::new(position.block_index, position.offset + char_count);
                self.selection = DocSelection::caret(new_pos);
            }
            EditOp::DeleteRange { start, .. } => {
                self.selection = DocSelection::caret(*start);
            }
            EditOp::SplitBlock { position } => {
                // Cursor at start of the new (second) block.
                self.selection = DocSelection::caret(DocPosition::new(position.block_index + 1, 0));
            }
            EditOp::MergeBlocks {
                merge_offset,
                block_index,
                ..
            } => {
                // Cursor at the merge point in the previous block.
                let target_block = block_index.saturating_sub(1);
                self.selection = DocSelection::caret(DocPosition::new(target_block, *merge_offset));
            }
            EditOp::ToggleInlineStyle { .. }
            | EditOp::SetBlockType { .. }
            | EditOp::SetBlockAttrs { .. }
            | EditOp::InsertBlock { .. }
            | EditOp::RemoveBlock { .. } => {
                // These don't move the cursor.
            }
        }
    }

    /// Undo the last action.
    pub fn undo(&mut self) {
        let Some(group) = self.undo_stack.undo() else {
            return;
        };

        // Apply inverse ops in reverse order. Use `invert_with_doc` so
        // that InsertText captures the actual styled runs from the document
        // (the text is still present before applying the inverse).
        for op in group.ops.iter().rev() {
            op.invert_with_doc(&self.document).apply(&mut self.document);
        }

        normalize(&mut self.document);
        self.selection = group.cursor_before;
        self.selection.anchor = self.document.clamp_position(self.selection.anchor);
        self.selection.focus = self.document.clamp_position(self.selection.focus);
        self.cursor.reset_blink();
    }

    /// Redo the last undone action.
    pub fn redo(&mut self) {
        let Some(group) = self.undo_stack.redo() else {
            return;
        };

        // Re-apply ops in order.
        for op in &group.ops {
            op.apply(&mut self.document);
        }

        normalize(&mut self.document);
        self.selection = group.cursor_after;
        self.selection.anchor = self.document.clamp_position(self.selection.anchor);
        self.selection.focus = self.document.clamp_position(self.selection.focus);
        self.cursor.reset_blink();
    }

    /// Apply a cursor movement.
    fn apply_move(&mut self, move_action: MoveAction, extend_selection: bool) {
        let doc = &self.document;
        let focus = self.selection.focus;

        let new_focus = match move_action {
            MoveAction::Left => {
                // If there's a non-collapsed selection and not extending, collapse to start.
                if !extend_selection && !self.selection.is_collapsed() {
                    self.selection.start()
                } else {
                    input::move_left(doc, focus)
                }
            }
            MoveAction::Right => {
                if !extend_selection && !self.selection.is_collapsed() {
                    self.selection.end()
                } else {
                    input::move_right(doc, focus)
                }
            }
            MoveAction::WordLeft => input::word_left(doc, focus),
            MoveAction::WordRight => input::word_right(doc, focus),
            MoveAction::Home => input::home(focus),
            MoveAction::End => input::end(doc, focus),
            MoveAction::DocumentStart => input::document_start(),
            MoveAction::DocumentEnd => input::document_end(doc),
            MoveAction::Up | MoveAction::Down => {
                // Vertical movement requires paragraph layout info which is
                // renderer-specific. Without renderer access we approximate
                // column preservation using a saved character offset
                // (`target_column`). On the first vertical move the current
                // offset is remembered; subsequent vertical moves use that
                // remembered offset so that traversing a short block and then
                // a long block returns to the original column.
                let desired_offset = self.cursor.target_column().unwrap_or(focus.offset);

                // Save target_column on first vertical move.
                if self.cursor.target_column().is_none() {
                    self.cursor.set_target_column(focus.offset);
                }

                match move_action {
                    MoveAction::Up => {
                        if focus.block_index > 0 {
                            let prev_len =
                                doc.block(focus.block_index - 1).map_or(0, Block::char_len);
                            DocPosition::new(focus.block_index - 1, prev_len.min(desired_offset))
                        } else {
                            DocPosition::new(0, 0)
                        }
                    }
                    MoveAction::Down => {
                        if focus.block_index + 1 < doc.block_count() {
                            let next_len =
                                doc.block(focus.block_index + 1).map_or(0, Block::char_len);
                            DocPosition::new(focus.block_index + 1, next_len.min(desired_offset))
                        } else {
                            doc.end_position()
                        }
                    }
                    _ => focus,
                }
            }
        };

        if extend_selection {
            self.selection = DocSelection::range(self.selection.anchor, new_focus);
        } else {
            self.selection = DocSelection::caret(new_focus);
        }

        self.cursor.reset_blink();

        // Clear target_x and target_column on horizontal movement, preserve on vertical.
        if !matches!(move_action, MoveAction::Up | MoveAction::Down) {
            self.cursor.clear_target_x();
            self.cursor.clear_target_column();
        }
    }

    /// Handle a click at a resolved document position.
    fn handle_click(&mut self, doc_pos: DocPosition) {
        self.cursor.focus();
        self.cursor.reset_blink();

        let doc_pos = self.document.clamp_position(doc_pos);
        self.selection = DocSelection::caret(doc_pos);
        self.drag = Some(DragState::start(doc_pos));
        self.pending_style = InlineStyle::empty();
    }

    /// Handle a double-click: select the word at the clicked position.
    fn handle_double_click(&mut self, doc_pos: DocPosition) {
        self.cursor.focus();
        self.cursor.reset_blink();
        let doc_pos = self.document.clamp_position(doc_pos);
        let (start, end) = input::word_at(&self.document, doc_pos);
        self.selection = DocSelection::range(start, end);
        self.drag = None;
        self.pending_style = InlineStyle::empty();
    }

    /// Handle a triple-click: select the entire block at the clicked position.
    fn handle_triple_click(&mut self, doc_pos: DocPosition) {
        self.cursor.focus();
        self.cursor.reset_blink();
        let doc_pos = self.document.clamp_position(doc_pos);
        let (start, end) = input::select_block(&self.document, doc_pos);
        self.selection = DocSelection::range(start, end);
        self.drag = None;
        self.pending_style = InlineStyle::empty();
    }

    /// Handle a drag to a resolved document position by extending the selection.
    fn handle_drag(&mut self, doc_pos: DocPosition) {
        let doc_pos = self.document.clamp_position(doc_pos);
        if let Some(drag) = &self.drag {
            self.selection = DocSelection::range(drag.anchor, doc_pos);
        }
        self.cursor.reset_blink();
    }
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}

// ── Paste style helper ───────────────────────────────────

/// Splice styled runs (with links) into a block at `offset` using the
/// block-swap strategy: RemoveBlock + InsertBlock. Both ops are recorded,
/// so redo replays correctly - no side-channel mutations needed.
fn splice_runs_into_block(
    doc: &mut Document,
    block_idx: usize,
    offset: usize,
    runs: &[StyledRun],
    ops: &mut Vec<EditOp>,
) {
    let Some(original) = doc.block(block_idx) else {
        return;
    };
    let original = original.clone();

    // Build the new block by splicing the pasted runs into the original.
    let mut new_block = original.clone();
    if let Some(existing_runs) = new_block.runs_mut() {
        // Split existing runs at the insertion offset.
        let split_idx = crate::document::split_runs_at_char_offset(existing_runs, offset);

        // Insert the pasted runs at the split point.
        for (insert_idx, run) in (split_idx..).zip(runs.iter()) {
            existing_runs.insert(insert_idx, run.clone());
        }
    }

    // Emit RemoveBlock + InsertBlock (the block-swap pattern).
    let remove_op = EditOp::RemoveBlock {
        index: block_idx,
        saved: original,
    };
    // RemoveBlock won't remove the last block, so handle that edge case.
    // If the document has only one block, we replace instead.
    if doc.block_count() <= 1 {
        // Can't use RemoveBlock; use replace_block directly and record
        // as InsertBlock at 0 + RemoveBlock at 1 (after insert).
        let insert_op = EditOp::InsertBlock {
            index: block_idx,
            block: new_block.clone(),
        };
        insert_op.apply(doc);
        ops.push(insert_op);

        // Now remove the old block (which shifted to block_idx + 1).
        if doc.block_count() > 1 {
            let remove_old = EditOp::RemoveBlock {
                index: block_idx + 1,
                saved: doc
                    .block(block_idx + 1)
                    .cloned()
                    .unwrap_or_else(Block::empty_paragraph),
            };
            remove_old.apply(doc);
            ops.push(remove_old);
        }
    } else {
        remove_op.apply(doc);
        ops.push(remove_op);

        let insert_op = EditOp::InsertBlock {
            index: block_idx,
            block: new_block,
        };
        insert_op.apply(doc);
        ops.push(insert_op);
    }
}

// ── Tests ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Block, DocPosition, DocSelection, Document, InlineStyle};

    // ── EditorState::new ─────────────────────────────────

    #[test]
    fn new_editor_has_empty_document() {
        let state = EditorState::new();
        assert_eq!(state.document.block_count(), 1);
        assert_eq!(state.document.block(0).map(Block::char_len), Some(0));
        assert!(state.selection.is_collapsed());
        assert_eq!(state.selection.focus, DocPosition::zero());
    }

    #[test]
    fn from_document_preserves_blocks() {
        let doc = Document::from_blocks(vec![Block::paragraph("hello"), Block::paragraph("world")]);
        let state = EditorState::from_document(doc);
        assert_eq!(state.document.block_count(), 2);
        assert_eq!(
            state
                .document
                .block(0)
                .map(Block::flattened_text)
                .as_deref(),
            Some("hello")
        );
        assert_eq!(
            state
                .document
                .block(1)
                .map(Block::flattened_text)
                .as_deref(),
            Some("world")
        );
    }

    #[test]
    fn from_html_parses() {
        let state = EditorState::from_html("<p>hello</p>");
        assert_eq!(
            state
                .document
                .block(0)
                .map(Block::flattened_text)
                .as_deref(),
            Some("hello")
        );
    }

    // ── EditorState::to_html ─────────────────────────────

    #[test]
    fn to_html_round_trips() {
        let state = EditorState::from_html("<p>hello</p><p>world</p>");
        let html = state.to_html();
        assert_eq!(html, "<p>hello</p><p>world</p>");
    }

    // ── EditorState::selection_text ──────────────────────

    #[test]
    fn selection_text_collapsed_is_empty() {
        let state = EditorState::new();
        assert!(state.selection_text().is_empty());
    }

    #[test]
    fn selection_text_single_block() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![Block::paragraph(
            "hello world",
        )]));
        state.selection = DocSelection::range(DocPosition::new(0, 0), DocPosition::new(0, 5));
        assert_eq!(state.selection_text(), "hello");
    }

    #[test]
    fn selection_text_cross_block() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
            Block::paragraph("world"),
        ]));
        state.selection = DocSelection::range(DocPosition::new(0, 3), DocPosition::new(1, 2));
        let text = state.selection_text();
        assert!(text.contains("lo"));
        assert!(text.contains("wo"));
    }

    // ── EditorState::apply_action - insert ───────────────

    #[test]
    fn apply_action_insert_text() {
        let mut state = EditorState::new();
        state.apply_action(EditAction::InsertText("hello".into()));
        assert_eq!(
            state
                .document
                .block(0)
                .map(Block::flattened_text)
                .as_deref(),
            Some("hello"),
        );
        assert_eq!(state.selection.focus, DocPosition::new(0, 5));
    }

    #[test]
    fn apply_action_insert_multiple_chars() {
        let mut state = EditorState::new();
        state.apply_action(EditAction::InsertText("h".into()));
        state.apply_action(EditAction::InsertText("e".into()));
        state.apply_action(EditAction::InsertText("l".into()));
        assert_eq!(
            state
                .document
                .block(0)
                .map(Block::flattened_text)
                .as_deref(),
            Some("hel"),
        );
        assert_eq!(state.selection.focus, DocPosition::new(0, 3));
    }

    // ── EditorState::apply_action - delete ───────────────

    #[test]
    fn apply_action_delete_backward() {
        let mut state =
            EditorState::from_document(Document::from_blocks(vec![Block::paragraph("hello")]));
        state.selection = DocSelection::caret(DocPosition::new(0, 5));
        state.apply_action(EditAction::DeleteBackward);
        assert_eq!(
            state
                .document
                .block(0)
                .map(Block::flattened_text)
                .as_deref(),
            Some("hell"),
        );
        assert_eq!(state.selection.focus, DocPosition::new(0, 4));
    }

    #[test]
    fn apply_action_delete_forward() {
        let mut state =
            EditorState::from_document(Document::from_blocks(vec![Block::paragraph("hello")]));
        state.selection = DocSelection::caret(DocPosition::new(0, 0));
        state.apply_action(EditAction::DeleteForward);
        assert_eq!(
            state
                .document
                .block(0)
                .map(Block::flattened_text)
                .as_deref(),
            Some("ello"),
        );
    }

    #[test]
    fn apply_action_delete_selection() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![Block::paragraph(
            "hello world",
        )]));
        state.selection = DocSelection::range(DocPosition::new(0, 5), DocPosition::new(0, 11));
        state.apply_action(EditAction::DeleteSelection);
        assert_eq!(
            state
                .document
                .block(0)
                .map(Block::flattened_text)
                .as_deref(),
            Some("hello"),
        );
    }

    // ── EditorState::apply_action - split block ──────────

    #[test]
    fn apply_action_split_block() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![Block::paragraph(
            "hello world",
        )]));
        state.selection = DocSelection::caret(DocPosition::new(0, 5));
        state.apply_action(EditAction::SplitBlock);
        assert_eq!(state.document.block_count(), 2);
        assert_eq!(
            state
                .document
                .block(0)
                .map(Block::flattened_text)
                .as_deref(),
            Some("hello"),
        );
        assert_eq!(
            state
                .document
                .block(1)
                .map(Block::flattened_text)
                .as_deref(),
            Some(" world"),
        );
        // Cursor should be at start of new block.
        assert_eq!(state.selection.focus, DocPosition::new(1, 0));
    }

    // ── EditorState::apply_action - toggle inline style ──

    #[test]
    fn toggle_style_at_caret_sets_pending() {
        let mut state = EditorState::new();
        state.apply_action(EditAction::ToggleInlineStyle(InlineStyle::BOLD));
        assert!(state.pending_style.contains(InlineStyle::BOLD));
    }

    #[test]
    fn toggle_style_at_caret_toggles_off() {
        let mut state = EditorState::new();
        state.apply_action(EditAction::ToggleInlineStyle(InlineStyle::BOLD));
        assert!(state.pending_style.contains(InlineStyle::BOLD));
        state.apply_action(EditAction::ToggleInlineStyle(InlineStyle::BOLD));
        assert!(!state.pending_style.contains(InlineStyle::BOLD));
    }

    #[test]
    fn toggle_style_with_selection_applies() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![Block::paragraph(
            "hello world",
        )]));
        state.selection = DocSelection::range(DocPosition::new(0, 0), DocPosition::new(0, 5));
        state.apply_action(EditAction::ToggleInlineStyle(InlineStyle::BOLD));

        let runs = state.document.block(0).and_then(Block::runs).expect("runs");
        // The first part should be bold.
        assert!(runs[0].style.contains(InlineStyle::BOLD));
    }

    // ── EditorState::undo / redo ─────────────────────────

    #[test]
    fn undo_reverts_insert() {
        let mut state = EditorState::new();
        state.apply_action(EditAction::InsertText("hello".into()));
        assert_eq!(
            state
                .document
                .block(0)
                .map(Block::flattened_text)
                .as_deref(),
            Some("hello"),
        );

        state.undo();
        assert_eq!(
            state
                .document
                .block(0)
                .map(Block::flattened_text)
                .as_deref(),
            Some(""),
        );
    }

    #[test]
    fn redo_reapplies_insert() {
        let mut state = EditorState::new();
        state.apply_action(EditAction::InsertText("hello".into()));
        state.undo();
        state.redo();
        assert_eq!(
            state
                .document
                .block(0)
                .map(Block::flattened_text)
                .as_deref(),
            Some("hello"),
        );
    }

    #[test]
    fn undo_redo_preserves_cursor() {
        let mut state = EditorState::new();
        let cursor_before = state.selection;
        state.apply_action(EditAction::InsertText("hello".into()));
        let cursor_after = state.selection;

        state.undo();
        assert_eq!(state.selection, cursor_before);

        state.redo();
        assert_eq!(state.selection, cursor_after);
    }

    #[test]
    fn multiple_undo_redo() {
        let mut state = EditorState::new();
        state.apply_action(EditAction::InsertText("a".into()));
        state.undo_stack.break_group();
        state.apply_action(EditAction::InsertText("b".into()));
        state.undo_stack.break_group();
        state.apply_action(EditAction::InsertText("c".into()));

        assert_eq!(
            state
                .document
                .block(0)
                .map(Block::flattened_text)
                .as_deref(),
            Some("abc"),
        );

        state.undo(); // remove "c"
        assert_eq!(
            state
                .document
                .block(0)
                .map(Block::flattened_text)
                .as_deref(),
            Some("ab"),
        );

        state.undo(); // remove "b"
        assert_eq!(
            state
                .document
                .block(0)
                .map(Block::flattened_text)
                .as_deref(),
            Some("a"),
        );

        state.redo(); // re-add "b"
        assert_eq!(
            state
                .document
                .block(0)
                .map(Block::flattened_text)
                .as_deref(),
            Some("ab"),
        );
    }

    // ── EditorState::apply_move ──────────────────────────

    #[test]
    fn move_left() {
        let mut state =
            EditorState::from_document(Document::from_blocks(vec![Block::paragraph("hello")]));
        state.selection = DocSelection::caret(DocPosition::new(0, 3));
        state.perform(Action::Move(MoveAction::Left));
        assert_eq!(state.selection.focus, DocPosition::new(0, 2));
        assert!(state.selection.is_collapsed());
    }

    #[test]
    fn move_right() {
        let mut state =
            EditorState::from_document(Document::from_blocks(vec![Block::paragraph("hello")]));
        state.selection = DocSelection::caret(DocPosition::new(0, 2));
        state.perform(Action::Move(MoveAction::Right));
        assert_eq!(state.selection.focus, DocPosition::new(0, 3));
    }

    #[test]
    fn move_left_collapses_selection_to_start() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![Block::paragraph(
            "hello world",
        )]));
        state.selection = DocSelection::range(DocPosition::new(0, 2), DocPosition::new(0, 7));
        state.perform(Action::Move(MoveAction::Left));
        assert!(state.selection.is_collapsed());
        assert_eq!(state.selection.focus, DocPosition::new(0, 2));
    }

    #[test]
    fn move_right_collapses_selection_to_end() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![Block::paragraph(
            "hello world",
        )]));
        state.selection = DocSelection::range(DocPosition::new(0, 2), DocPosition::new(0, 7));
        state.perform(Action::Move(MoveAction::Right));
        assert!(state.selection.is_collapsed());
        assert_eq!(state.selection.focus, DocPosition::new(0, 7));
    }

    #[test]
    fn select_extends_selection() {
        let mut state =
            EditorState::from_document(Document::from_blocks(vec![Block::paragraph("hello")]));
        state.selection = DocSelection::caret(DocPosition::new(0, 2));
        state.perform(Action::Select(MoveAction::Right));
        assert!(!state.selection.is_collapsed());
        assert_eq!(state.selection.anchor, DocPosition::new(0, 2));
        assert_eq!(state.selection.focus, DocPosition::new(0, 3));
    }

    #[test]
    fn select_all() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
            Block::paragraph("world"),
        ]));
        state.perform(Action::SelectAll);
        assert_eq!(state.selection.start(), DocPosition::zero());
        assert_eq!(state.selection.end(), DocPosition::new(1, 5));
    }

    #[test]
    fn move_home_end() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![Block::paragraph(
            "hello world",
        )]));
        state.selection = DocSelection::caret(DocPosition::new(0, 5));

        state.perform(Action::Move(MoveAction::Home));
        assert_eq!(state.selection.focus, DocPosition::new(0, 0));

        state.perform(Action::Move(MoveAction::End));
        assert_eq!(state.selection.focus, DocPosition::new(0, 11));
    }

    #[test]
    fn move_document_start_end() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
            Block::paragraph("world"),
        ]));
        state.selection = DocSelection::caret(DocPosition::new(0, 3));

        state.perform(Action::Move(MoveAction::DocumentEnd));
        assert_eq!(state.selection.focus, DocPosition::new(1, 5));

        state.perform(Action::Move(MoveAction::DocumentStart));
        assert_eq!(state.selection.focus, DocPosition::zero());
    }

    #[test]
    fn move_up_down() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
            Block::paragraph("world"),
            Block::paragraph("foo"),
        ]));
        state.selection = DocSelection::caret(DocPosition::new(1, 3));

        state.perform(Action::Move(MoveAction::Up));
        assert_eq!(state.selection.focus.block_index, 0);

        state.selection = DocSelection::caret(DocPosition::new(1, 3));
        state.perform(Action::Move(MoveAction::Down));
        assert_eq!(state.selection.focus.block_index, 2);
    }

    #[test]
    fn vertical_move_preserves_offset_through_short_block() {
        // Three blocks: long (15 chars), short (2 chars), long (20 chars).
        // Start at offset 10 in block 0, move down through block 1 (short)
        // to block 2. The offset should recover to 10 in block 2.
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("abcdefghijklmno"),      // len 15
            Block::paragraph("xy"),                   // len 2
            Block::paragraph("12345678901234567890"), // len 20
        ]));
        state.selection = DocSelection::caret(DocPosition::new(0, 10));

        // Move down to block 1 - offset clamped to 2
        state.perform(Action::Move(MoveAction::Down));
        assert_eq!(state.selection.focus, DocPosition::new(1, 2));

        // Move down to block 2 - offset should recover to 10 (the saved target)
        state.perform(Action::Move(MoveAction::Down));
        assert_eq!(state.selection.focus, DocPosition::new(2, 10));
    }

    #[test]
    fn vertical_move_preserves_offset_through_short_block_upward() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("12345678901234567890"), // len 20
            Block::paragraph("ab"),                   // len 2
            Block::paragraph("abcdefghijklmno"),      // len 15
        ]));
        state.selection = DocSelection::caret(DocPosition::new(2, 12));

        // Move up to block 1 - clamped to 2
        state.perform(Action::Move(MoveAction::Up));
        assert_eq!(state.selection.focus, DocPosition::new(1, 2));

        // Move up to block 0 - offset recovers to 12
        state.perform(Action::Move(MoveAction::Up));
        assert_eq!(state.selection.focus, DocPosition::new(0, 12));
    }

    #[test]
    fn horizontal_move_clears_target_column() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("abcdefghij"), // len 10
            Block::paragraph("xy"),         // len 2
            Block::paragraph("abcdefghij"), // len 10
        ]));
        state.selection = DocSelection::caret(DocPosition::new(0, 8));

        // Move down - saves target_column = 8, lands at (1, 2)
        state.perform(Action::Move(MoveAction::Down));
        assert_eq!(state.selection.focus, DocPosition::new(1, 2));

        // Horizontal move clears target_column
        state.perform(Action::Move(MoveAction::Left));
        assert_eq!(state.selection.focus, DocPosition::new(1, 1));

        // Move down again - target_column is now 1 (the current offset)
        state.perform(Action::Move(MoveAction::Down));
        assert_eq!(state.selection.focus, DocPosition::new(2, 1));
    }

    #[test]
    fn vertical_move_at_boundary_clamps_correctly() {
        let mut state =
            EditorState::from_document(Document::from_blocks(vec![Block::paragraph("abc")]));
        state.selection = DocSelection::caret(DocPosition::new(0, 2));

        // Move up at top boundary - goes to (0, 0)
        state.perform(Action::Move(MoveAction::Up));
        assert_eq!(state.selection.focus, DocPosition::new(0, 0));

        // Move down at bottom boundary - goes to end position
        state.selection = DocSelection::caret(DocPosition::new(0, 2));
        state.perform(Action::Move(MoveAction::Down));
        assert_eq!(state.selection.focus, DocPosition::new(0, 3));
    }

    // ── EditorState::set_selection ───────────────────────

    #[test]
    fn set_selection() {
        let mut state =
            EditorState::from_document(Document::from_blocks(vec![Block::paragraph("hello")]));
        let sel = DocSelection::range(DocPosition::new(0, 1), DocPosition::new(0, 4));
        state.set_selection(sel);
        assert_eq!(state.selection, sel);
    }

    // ── EditorState::perform - focus / blur ──────────────

    #[test]
    fn focus_and_blur() {
        let mut state = EditorState::new();
        assert!(!state.is_focused());

        state.perform(Action::Focus);
        assert!(state.is_focused());

        state.perform(Action::Blur);
        assert!(!state.is_focused());
    }

    // ── Pending style cleared after edit ─────────────────

    #[test]
    fn pending_style_cleared_after_insert() {
        let mut state = EditorState::new();
        state.apply_action(EditAction::ToggleInlineStyle(InlineStyle::BOLD));
        assert!(state.pending_style.contains(InlineStyle::BOLD));

        state.apply_action(EditAction::InsertText("x".into()));
        assert!(state.pending_style.is_empty());
    }

    // ── Insert replaces selection ────────────────────────

    #[test]
    fn insert_replaces_selection() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![Block::paragraph(
            "hello world",
        )]));
        state.selection = DocSelection::range(DocPosition::new(0, 5), DocPosition::new(0, 11));
        state.apply_action(EditAction::InsertText("!".into()));
        assert_eq!(
            state
                .document
                .block(0)
                .map(Block::flattened_text)
                .as_deref(),
            Some("hello!"),
        );
    }

    // ── Backspace at block start merges ──────────────────

    #[test]
    fn backspace_at_block_start_merges() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
            Block::paragraph("world"),
        ]));
        state.selection = DocSelection::caret(DocPosition::new(1, 0));
        state.apply_action(EditAction::DeleteBackward);
        assert_eq!(state.document.block_count(), 1);
        assert_eq!(
            state
                .document
                .block(0)
                .map(Block::flattened_text)
                .as_deref(),
            Some("helloworld"),
        );
        assert_eq!(state.selection.focus, DocPosition::new(0, 5));
    }

    // ── Undo stack grouping ─────────────────────────────

    #[test]
    fn consecutive_inserts_group_in_undo() {
        let mut state = EditorState::new();
        state.apply_action(EditAction::InsertText("a".into()));
        state.apply_action(EditAction::InsertText("b".into()));
        state.apply_action(EditAction::InsertText("c".into()));

        // All consecutive inserts should merge into one undo group.
        assert_eq!(state.undo_stack.undo_len(), 1);

        state.undo();
        assert_eq!(
            state
                .document
                .block(0)
                .map(Block::flattened_text)
                .as_deref(),
            Some(""),
        );
    }

    // ── Default trait ────────────────────────────────────

    #[test]
    fn default_creates_empty() {
        let state = EditorState::default();
        assert_eq!(state.document.block_count(), 1);
        assert!(state.selection.is_collapsed());
    }

    // ── Pending style applied to inserted text ──────────

    #[test]
    fn pending_bold_applied_to_inserted_text() {
        let mut state = EditorState::new();
        // Toggle bold at caret (sets pending style).
        state.apply_action(EditAction::ToggleInlineStyle(InlineStyle::BOLD));
        assert!(state.pending_style.contains(InlineStyle::BOLD));

        // Type 'x' - should be bold.
        state.apply_action(EditAction::InsertText("x".into()));
        assert!(
            state.pending_style.is_empty(),
            "pending style should be cleared after insert"
        );

        let runs = state.document.block(0).and_then(Block::runs).expect("runs");
        // The 'x' run should be bold.
        let bold_runs: Vec<_> = runs.iter().filter(|r| !r.is_empty()).collect();
        assert!(
            bold_runs
                .iter()
                .all(|r| r.style.contains(InlineStyle::BOLD)),
            "all non-empty runs should be bold, got: {bold_runs:?}",
        );
    }

    // ── Clipboard: internal copy/paste ───────────────────

    #[test]
    fn copy_captures_internal_clipboard() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![Block::paragraph(
            "hello world",
        )]));
        state.selection = DocSelection::range(DocPosition::new(0, 0), DocPosition::new(0, 5));
        state.perform(Action::Copy);
        assert!(state.internal_clipboard.is_some());
        let ic = state.internal_clipboard.as_ref().expect("clipboard");
        assert_eq!(ic.plain_text, "hello");
        assert!(ic.slice.open_start);
        assert!(ic.slice.open_end);
    }

    #[test]
    fn copy_paste_plain_text_within_single_block() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![Block::paragraph(
            "hello world",
        )]));
        // Copy "world"
        state.selection = DocSelection::range(DocPosition::new(0, 6), DocPosition::new(0, 11));
        state.perform(Action::Copy);

        // Move cursor to start and paste
        state.selection = DocSelection::caret(DocPosition::new(0, 0));
        state.perform(Action::Paste("world".into()));

        assert_eq!(
            state
                .document
                .block(0)
                .map(Block::flattened_text)
                .as_deref(),
            Some("worldhello world"),
        );
    }

    #[test]
    fn copy_paste_preserves_bold_formatting() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![Block::Paragraph {
            runs: vec![
                StyledRun::plain("hello "),
                StyledRun::styled("bold", InlineStyle::BOLD),
                StyledRun::plain(" text"),
            ],
        }]));
        // Copy "bold" (offsets 6..10)
        state.selection = DocSelection::range(DocPosition::new(0, 6), DocPosition::new(0, 10));
        state.perform(Action::Copy);

        // Move cursor to end and paste
        state.selection = DocSelection::caret(DocPosition::new(0, 15));
        state.perform(Action::Paste("bold".into()));

        // The pasted "bold" at the end should be bold.
        let runs = state.document.block(0).and_then(Block::runs).expect("runs");
        let total_text: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(total_text, "hello bold textbold");

        // Find the last "bold" - it should have BOLD style.
        let mut pos = 0;
        let mut found_pasted_bold = false;
        for run in runs {
            let rlen = run.char_len();
            let rend = pos + rlen;
            // The pasted text starts at offset 15.
            if pos >= 15 && rlen > 0 {
                assert!(
                    run.style.contains(InlineStyle::BOLD),
                    "pasted run '{}' at [{pos}..{rend}) should be bold",
                    run.text,
                );
                found_pasted_bold = true;
            }
            pos = rend;
        }
        assert!(found_pasted_bold, "should have found the pasted bold run");
    }

    #[test]
    fn copy_paste_cross_block() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("first"),
            Block::paragraph("second"),
            Block::paragraph("third"),
        ]));

        // Copy "first\nsecond" (blocks 0 and 1 entirely)
        state.selection = DocSelection::range(DocPosition::new(0, 0), DocPosition::new(1, 6));
        state.perform(Action::Copy);

        // Paste at end of "third"
        state.selection = DocSelection::caret(DocPosition::new(2, 5));
        state.perform(Action::Paste("first\nsecond".into()));

        // Should now have: "first", "second", "thirdfirst", "second"
        assert_eq!(state.document.block_count(), 4);
        assert_eq!(
            state
                .document
                .block(2)
                .map(Block::flattened_text)
                .as_deref(),
            Some("thirdfirst"),
        );
        assert_eq!(
            state
                .document
                .block(3)
                .map(Block::flattened_text)
                .as_deref(),
            Some("second"),
        );
    }

    #[test]
    fn cut_captures_and_deletes() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![Block::paragraph(
            "hello world",
        )]));
        state.selection = DocSelection::range(DocPosition::new(0, 5), DocPosition::new(0, 11));
        state.perform(Action::Cut);

        // Text should be deleted.
        assert_eq!(
            state
                .document
                .block(0)
                .map(Block::flattened_text)
                .as_deref(),
            Some("hello"),
        );

        // Internal clipboard should have the cut content.
        assert!(state.internal_clipboard.is_some());
        let ic = state.internal_clipboard.as_ref().expect("clipboard");
        assert_eq!(ic.plain_text, " world");
    }

    #[test]
    fn paste_external_text_when_clipboard_differs() {
        let mut state =
            EditorState::from_document(Document::from_blocks(vec![Block::paragraph("hello")]));
        // Copy "hello"
        state.selection = DocSelection::range(DocPosition::new(0, 0), DocPosition::new(0, 5));
        state.perform(Action::Copy);

        // Now paste something different from the system clipboard
        // (simulating the user copying from another app).
        state.selection = DocSelection::caret(DocPosition::new(0, 5));
        state.perform(Action::Paste("external".into()));

        assert_eq!(
            state
                .document
                .block(0)
                .map(Block::flattened_text)
                .as_deref(),
            Some("helloexternal"),
        );
    }

    #[test]
    fn paste_replaces_selection() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![Block::paragraph(
            "hello world",
        )]));
        // Copy "hello"
        state.selection = DocSelection::range(DocPosition::new(0, 0), DocPosition::new(0, 5));
        state.perform(Action::Copy);

        // Select " world" and paste "hello" over it.
        state.selection = DocSelection::range(DocPosition::new(0, 5), DocPosition::new(0, 11));
        state.perform(Action::Paste("hello".into()));

        assert_eq!(
            state
                .document
                .block(0)
                .map(Block::flattened_text)
                .as_deref(),
            Some("hellohello"),
        );
    }

    #[test]
    fn copy_paste_preserves_multi_block_structure_at_cursor_mid_block() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("aaa"),
            Block::paragraph("bbb"),
            Block::paragraph("target text"),
        ]));

        // Copy "aaa\nbbb" (two complete blocks)
        state.selection = DocSelection::range(DocPosition::new(0, 0), DocPosition::new(1, 3));
        state.perform(Action::Copy);

        // Paste in the middle of "target text" at offset 6
        state.selection = DocSelection::caret(DocPosition::new(2, 6));
        state.perform(Action::Paste("aaa\nbbb".into()));

        // Expected: "target" + "aaa" content merged, then "bbb" + " text" merged
        // Block 0: "aaa", Block 1: "bbb", Block 2: "targetaaa", Block 3: "bbb text"
        assert!(state.document.block_count() >= 4);
        assert_eq!(
            state
                .document
                .block(2)
                .map(Block::flattened_text)
                .as_deref(),
            Some("targetaaa"),
        );
        assert_eq!(
            state
                .document
                .block(3)
                .map(Block::flattened_text)
                .as_deref(),
            Some("bbb text"),
        );
    }

    #[test]
    fn collapsed_copy_does_not_capture() {
        let mut state =
            EditorState::from_document(Document::from_blocks(vec![Block::paragraph("hello")]));
        state.selection = DocSelection::caret(DocPosition::new(0, 3));
        state.perform(Action::Copy);
        assert!(state.internal_clipboard.is_none());
    }

    // ── Scroll helpers ──────────────────────────────────────

    #[test]
    fn ensure_cursor_visible_no_op_when_in_view() {
        let mut offset = 50.0;
        super::super::ensure_cursor_visible(&mut offset, 60.0, 20.0, 200.0, 500.0);
        assert!((offset - 50.0).abs() < f32::EPSILON);
    }

    #[test]
    fn ensure_cursor_visible_scrolls_up() {
        let mut offset = 100.0;
        // Cursor is at y=80, above viewport start (100).
        super::super::ensure_cursor_visible(&mut offset, 80.0, 20.0, 200.0, 500.0);
        assert!((offset - 80.0).abs() < f32::EPSILON);
    }

    #[test]
    fn ensure_cursor_visible_scrolls_down() {
        let mut offset = 0.0;
        // Cursor at y=250, height=20, viewport=200. Bottom = 270 > 200.
        super::super::ensure_cursor_visible(&mut offset, 250.0, 20.0, 200.0, 500.0);
        // Should scroll so cursor bottom is at viewport bottom: 270 - 200 = 70.
        assert!((offset - 70.0).abs() < f32::EPSILON);
    }

    #[test]
    fn ensure_cursor_visible_clamps_to_zero() {
        let mut offset = 50.0;
        // Cursor at y=0 - should scroll to 0.
        super::super::ensure_cursor_visible(&mut offset, 0.0, 20.0, 200.0, 500.0);
        assert!(offset.abs() < f32::EPSILON);
    }

    #[test]
    fn ensure_cursor_visible_clamps_to_max() {
        let mut offset = 0.0;
        // Cursor at y=490, total=500, viewport=200. Max scroll = 300.
        // Needed: 490 + 20 - 200 = 310, but max = 300.
        super::super::ensure_cursor_visible(&mut offset, 490.0, 20.0, 200.0, 500.0);
        assert!((offset - 300.0).abs() < f32::EPSILON);
    }

    #[test]
    fn ensure_cursor_visible_no_scroll_when_content_fits() {
        let mut offset = 0.0;
        // Total content (100) fits within viewport (200) - max_scroll = 0.
        super::super::ensure_cursor_visible(&mut offset, 50.0, 20.0, 200.0, 100.0);
        assert!(offset.abs() < f32::EPSILON);
    }

    // ── Double-click / triple-click ──────────────────────

    #[test]
    fn double_click_selects_word() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![Block::paragraph(
            "hello world",
        )]));
        state.perform(Action::DoubleClick(DocPosition::new(0, 3)));
        assert_eq!(state.selection.start(), DocPosition::new(0, 0));
        assert_eq!(state.selection.end(), DocPosition::new(0, 5));
    }

    #[test]
    fn double_click_selects_second_word() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![Block::paragraph(
            "hello world",
        )]));
        state.perform(Action::DoubleClick(DocPosition::new(0, 8)));
        assert_eq!(state.selection.start(), DocPosition::new(0, 6));
        assert_eq!(state.selection.end(), DocPosition::new(0, 11));
    }

    #[test]
    fn triple_click_selects_block() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![Block::paragraph(
            "hello world",
        )]));
        state.perform(Action::TripleClick(DocPosition::new(0, 3)));
        assert_eq!(state.selection.start(), DocPosition::new(0, 0));
        assert_eq!(state.selection.end(), DocPosition::new(0, 11));
    }

    #[test]
    fn triple_click_second_block() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("first"),
            Block::paragraph("second block"),
        ]));
        state.perform(Action::TripleClick(DocPosition::new(1, 4)));
        assert_eq!(state.selection.start(), DocPosition::new(1, 0));
        assert_eq!(state.selection.end(), DocPosition::new(1, 12));
    }

    #[test]
    fn double_click_clears_drag() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![Block::paragraph(
            "hello world",
        )]));
        state.perform(Action::DoubleClick(DocPosition::new(0, 3)));
        assert!(state.drag.is_none());
    }
}
