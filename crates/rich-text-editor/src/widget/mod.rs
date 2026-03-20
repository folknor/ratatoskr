//! Rich text editor widget for iced.
//!
//! Custom `Widget` trait implementation. Renders blocks as `Paragraph::with_spans`,
//! handles keyboard/mouse/IME input, cursor blink, and selection.
//!
//! # Usage
//!
//! ```ignore
//! use ratatoskr_rich_text_editor::widget::{EditorState, RichTextEditor, Action};
//!
//! struct State {
//!     editor: EditorState,
//! }
//!
//! #[derive(Debug, Clone)]
//! enum Message {
//!     EditorAction(Action),
//! }
//!
//! fn view(state: &State) -> Element<'_, Message> {
//!     rich_text_editor(&state.editor)
//!         .on_action(Message::EditorAction)
//!         .into()
//! }
//!
//! fn update(state: &mut State, message: Message) {
//!     match message {
//!         Message::EditorAction(action) => {
//!             state.editor.perform(action);
//!         }
//!     }
//! }
//! ```

pub mod cursor;
pub mod input;
pub mod render;

use crate::document::{Block, DocPosition, DocSelection, DocSlice, Document, InlineStyle, StyledRun};
use crate::html_parse::from_html;
use crate::html_serialize::to_html;
use crate::normalize::{normalize, normalize_blocks};
use crate::operations::EditOp;
use crate::rules::{self, EditAction};
use crate::undo::UndoStack;

use cursor::{
    BlockSelectionKind, CursorState, DragState, SelectionRect, CURSOR_WIDTH, SELECTION_ALPHA,
};
use input::{KeyAction, MoveAction};
use render::ParagraphCache;

use iced::advanced::layout;
use iced::advanced::mouse::click::Click;
use iced::advanced::renderer;
use iced::advanced::renderer::Renderer as _;
use iced::advanced::text::Paragraph as _;
use iced::advanced::text::Renderer as TextRenderer;
use iced::advanced::widget::{self, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::keyboard;
use iced::mouse;
use iced::time::{Duration, Instant};
use iced::window;
use iced::{Color, Element, Event, Font, Length, Padding, Point, Rectangle, Size};

/// The paragraph type used by the iced Renderer.
type IcedParagraph = <iced::Renderer as TextRenderer>::Paragraph;

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
    cursor: CursorState,
    /// Active mouse drag state.
    drag: Option<DragState>,
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
                        self.internal_clipboard = Some(InternalClipboard {
                            slice,
                            plain_text,
                        });
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
                let use_structured = self
                    .internal_clipboard
                    .as_ref()
                    .is_some_and(|ic| ic.plain_text == text);
                if use_structured {
                    let slice = self
                        .internal_clipboard
                        .as_ref()
                        .expect("checked above")
                        .slice
                        .clone();
                    self.paste_slice(&slice);
                } else {
                    self.apply_action(EditAction::InsertText(text));
                }
            }
            Action::Click(doc_pos) => {
                self.handle_click(doc_pos);
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

        let ops = rules::resolve(
            &self.document,
            self.selection,
            action,
            self.pending_style,
        );

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
    fn paste_inline_runs(
        &mut self,
        pos: DocPosition,
        block: &Block,
        ops: &mut Vec<EditOp>,
    ) {
        let Some(runs) = block.runs() else {
            return;
        };

        let mut current_offset = pos.offset;

        for run in runs {
            if run.text.is_empty() {
                continue;
            }

            let char_count = run.text.chars().count();
            let insert_pos = DocPosition::new(pos.block_index, current_offset);

            // Insert the text (inherits the style of the run at the insertion point).
            let insert_op = EditOp::InsertText {
                position: insert_pos,
                text: run.text.clone(),
            };
            insert_op.apply(&mut self.document);
            ops.push(insert_op);

            // Determine the inherited style and toggle any differing bits.
            let inherited = run_style_at_for_paste(&self.document, insert_pos);
            let diff = run.style.symmetric_difference(inherited);
            if !diff.is_empty() {
                let insert_end =
                    DocPosition::new(pos.block_index, current_offset + char_count);
                for bit in diff.iter() {
                    let toggle_op = EditOp::ToggleInlineStyle {
                        start: insert_pos,
                        end: insert_end,
                        style_bit: bit,
                    };
                    toggle_op.apply(&mut self.document);
                    ops.push(toggle_op);
                }
            }

            current_offset += char_count;
        }

        self.selection = DocSelection::caret(DocPosition::new(pos.block_index, current_offset));
    }

    /// Paste a multi-block slice at `pos`.
    ///
    /// Strategy: always merge the first and last slice blocks into the
    /// surrounding content (regardless of `open_start`/`open_end`). This
    /// matches the natural user expectation: pasting "A\nB" at the cursor
    /// appends A's content to the left of the cursor and prepends B's
    /// content to the right.
    ///
    /// 1. Split the block at `pos` into left and right halves.
    /// 2. Append the first slice block's runs to the left half.
    /// 3. Insert middle blocks (indices 1..len-1) between.
    /// 4. Prepend the last slice block's runs to the right half.
    /// 5. Place cursor at the end of the last pasted content.
    fn paste_multi_block(
        &mut self,
        pos: DocPosition,
        slice: &DocSlice,
        ops: &mut Vec<EditOp>,
    ) {
        let block_count = slice.blocks.len();

        // Split at cursor to create left and right halves.
        let split_op = EditOp::SplitBlock { position: pos };
        split_op.apply(&mut self.document);
        ops.push(split_op);

        // After split: left half at pos.block_index, right half at pos.block_index + 1.
        let left_idx = pos.block_index;

        // Merge first slice block's runs into the left half (append at end).
        if let Some(runs) = slice.blocks[0].runs() {
            self.append_runs_to_block(left_idx, runs, ops);
        }

        // Track where we insert middle blocks (between left and right halves).
        let mut insert_idx = pos.block_index + 1;

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

        // Handle the last block (only when there are 2+ blocks).
        let cursor_block;
        let cursor_offset;

        if block_count > 1 {
            let last_block = &slice.blocks[block_count - 1];
            // Merge last slice block's runs into the right half (prepend at start).
            let right_idx = insert_idx;
            if let Some(runs) = last_block.runs() {
                let pasted_len: usize = runs.iter().map(StyledRun::char_len).sum();
                self.prepend_runs_to_block(right_idx, runs, ops);
                cursor_block = right_idx;
                cursor_offset = pasted_len;
            } else {
                cursor_block = right_idx;
                cursor_offset = 0;
            }
        } else {
            // Single block in the slice (shouldn't reach here for multi-block,
            // but handle gracefully).
            cursor_block = left_idx;
            cursor_offset = self
                .document
                .block(left_idx)
                .map_or(0, Block::char_len);
        }

        self.selection = DocSelection::caret(DocPosition::new(cursor_block, cursor_offset));
    }

    /// Append styled runs to the end of a block, preserving their formatting.
    fn append_runs_to_block(
        &mut self,
        block_idx: usize,
        runs: &[StyledRun],
        ops: &mut Vec<EditOp>,
    ) {
        let mut offset = self
            .document
            .block(block_idx)
            .map_or(0, Block::char_len);

        for run in runs {
            if run.text.is_empty() {
                continue;
            }
            let char_count = run.text.chars().count();
            let insert_pos = DocPosition::new(block_idx, offset);

            let insert_op = EditOp::InsertText {
                position: insert_pos,
                text: run.text.clone(),
            };
            insert_op.apply(&mut self.document);
            ops.push(insert_op);

            // Toggle style bits that differ from what was inherited.
            let inherited = run_style_at_for_paste(&self.document, insert_pos);
            let diff = run.style.symmetric_difference(inherited);
            if !diff.is_empty() {
                let end = DocPosition::new(block_idx, offset + char_count);
                for bit in diff.iter() {
                    let toggle_op = EditOp::ToggleInlineStyle {
                        start: insert_pos,
                        end,
                        style_bit: bit,
                    };
                    toggle_op.apply(&mut self.document);
                    ops.push(toggle_op);
                }
            }

            offset += char_count;
        }
    }

    /// Prepend styled runs to the start of a block, preserving their formatting.
    fn prepend_runs_to_block(
        &mut self,
        block_idx: usize,
        runs: &[StyledRun],
        ops: &mut Vec<EditOp>,
    ) {
        let mut offset = 0;

        for run in runs {
            if run.text.is_empty() {
                continue;
            }
            let char_count = run.text.chars().count();
            let insert_pos = DocPosition::new(block_idx, offset);

            let insert_op = EditOp::InsertText {
                position: insert_pos,
                text: run.text.clone(),
            };
            insert_op.apply(&mut self.document);
            ops.push(insert_op);

            // Toggle style bits that differ from what was inherited.
            let inherited = run_style_at_for_paste(&self.document, insert_pos);
            let diff = run.style.symmetric_difference(inherited);
            if !diff.is_empty() {
                let end = DocPosition::new(block_idx, offset + char_count);
                for bit in diff.iter() {
                    let toggle_op = EditOp::ToggleInlineStyle {
                        start: insert_pos,
                        end,
                        style_bit: bit,
                    };
                    toggle_op.apply(&mut self.document);
                    ops.push(toggle_op);
                }
            }

            offset += char_count;
        }
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
                let new_pos =
                    DocPosition::new(position.block_index, position.offset + char_count);
                self.selection = DocSelection::caret(new_pos);
            }
            EditOp::DeleteRange { start, .. } => {
                self.selection = DocSelection::caret(*start);
            }
            EditOp::SplitBlock { position } => {
                // Cursor at start of the new (second) block.
                self.selection =
                    DocSelection::caret(DocPosition::new(position.block_index + 1, 0));
            }
            EditOp::MergeBlocks { merge_offset, block_index, .. } => {
                // Cursor at the merge point in the previous block.
                let target_block = block_index.saturating_sub(1);
                self.selection =
                    DocSelection::caret(DocPosition::new(target_block, *merge_offset));
            }
            EditOp::ToggleInlineStyle { .. }
            | EditOp::SetBlockType { .. }
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

        // Apply inverse ops in reverse order.
        for op in group.ops.iter().rev() {
            op.invert().apply(&mut self.document);
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
                let desired_offset = self
                    .cursor
                    .target_column()
                    .unwrap_or(focus.offset);

                // Save target_column on first vertical move.
                if self.cursor.target_column().is_none() {
                    self.cursor.set_target_column(focus.offset);
                }

                match move_action {
                    MoveAction::Up => {
                        if focus.block_index > 0 {
                            let prev_len = doc
                                .block(focus.block_index - 1)
                                .map_or(0, Block::char_len);
                            DocPosition::new(
                                focus.block_index - 1,
                                prev_len.min(desired_offset),
                            )
                        } else {
                            DocPosition::new(0, 0)
                        }
                    }
                    MoveAction::Down => {
                        if focus.block_index + 1 < doc.block_count() {
                            let next_len = doc
                                .block(focus.block_index + 1)
                                .map_or(0, Block::char_len);
                            DocPosition::new(
                                focus.block_index + 1,
                                next_len.min(desired_offset),
                            )
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

// ── Widget tree state ───────────────────────────────────

/// Internal widget state stored in the iced widget tree. Holds the paragraph
/// cache and focus/blink timing.
struct WidgetState {
    /// Paragraph cache: one entry per document block.
    cache: ParagraphCache<IcedParagraph>,
    /// Focus tracking for cursor blink.
    focus: Option<FocusState>,
    /// Last mouse click for double/triple click detection (future use).
    _last_click: Option<Click>,
    /// Whether a drag is active.
    dragging: bool,
}

/// Focus timing state for cursor blink.
#[derive(Debug, Clone)]
struct FocusState {
    updated_at: Instant,
    now: Instant,
    is_window_focused: bool,
}

impl FocusState {
    const BLINK_INTERVAL_MILLIS: u128 = 500;

    fn now() -> Self {
        let now = Instant::now();
        Self {
            updated_at: now,
            now,
            is_window_focused: true,
        }
    }

    fn is_cursor_visible(&self) -> bool {
        self.is_window_focused
            && ((self.now - self.updated_at).as_millis() / Self::BLINK_INTERVAL_MILLIS)
                .is_multiple_of(2)
    }
}

// ── RichTextEditor widget ───────────────────────────────

/// A rich text editor widget for iced.
///
/// Created via the [`rich_text_editor`] function. Renders a [`Document`] with
/// styled text, handles keyboard and mouse input, and emits [`Action`]s to the
/// application.
pub struct RichTextEditor<'a, Message> {
    state: &'a EditorState,
    on_action: Option<Box<dyn Fn(Action) -> Message + 'a>>,
    font: Font,
    text_color: Color,
    link_color: Color,
    cursor_color: Color,
    selection_color: Color,
    padding: Padding,
    width: Length,
    height: Length,
}

/// Create a [`RichTextEditor`] widget for the given state.
pub fn rich_text_editor<'a, Message>(state: &'a EditorState) -> RichTextEditor<'a, Message> {
    RichTextEditor::new(state)
}

impl<'a, Message> RichTextEditor<'a, Message> {
    /// Create a new rich text editor widget.
    pub fn new(state: &'a EditorState) -> Self {
        Self {
            state,
            on_action: None,
            font: Font::DEFAULT,
            text_color: Color::BLACK,
            link_color: Color::from_rgb(0.2, 0.4, 0.8),
            cursor_color: Color::BLACK,
            selection_color: Color::from_rgba(0.2, 0.4, 0.8, SELECTION_ALPHA),
            padding: Padding::new(8.0),
            width: Length::Fill,
            height: Length::Shrink,
        }
    }

    /// Set the callback for when an action occurs.
    ///
    /// If not set, the editor is disabled (read-only).
    pub fn on_action(mut self, f: impl Fn(Action) -> Message + 'a) -> Self {
        self.on_action = Some(Box::new(f));
        self
    }

    /// Set the base font.
    pub fn font(mut self, font: Font) -> Self {
        self.font = font;
        self
    }

    /// Set the text color.
    pub fn text_color(mut self, color: Color) -> Self {
        self.text_color = color;
        self
    }

    /// Set the link color.
    pub fn link_color(mut self, color: Color) -> Self {
        self.link_color = color;
        self
    }

    /// Set the cursor (caret) color.
    pub fn cursor_color(mut self, color: Color) -> Self {
        self.cursor_color = color;
        self
    }

    /// Set the selection highlight color.
    pub fn selection_color(mut self, color: Color) -> Self {
        self.selection_color = color;
        self
    }

    /// Set the padding.
    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.padding = padding.into();
        self
    }

    /// Set the width.
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Set the height.
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }
}

// ── Widget trait implementation ─────────────────────────

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for RichTextEditor<'_, Message> {
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<WidgetState>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(WidgetState {
            cache: ParagraphCache::new(),
            focus: None,
            _last_click: None,
            dragging: false,
        })
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let widget_state = tree.state.downcast_mut::<WidgetState>();
        let cache = &mut widget_state.cache;

        let limits = limits.width(self.width).height(self.height);
        let max_size = limits.max();
        let available_width = (max_size.width - self.padding.left - self.padding.right).max(0.0);

        // Layout paragraphs using the cache.
        let total_height = cache.layout(
            &self.state.document.blocks,
            available_width,
            self.font,
            self.text_color,
            self.link_color,
        );

        let content_height = total_height + self.padding.top + self.padding.bottom;

        match self.height {
            Length::Fill | Length::FillPortion(_) | Length::Fixed(_) => {
                layout::Node::new(limits.max())
            }
            Length::Shrink => {
                let size = limits
                    .height(Length::Fixed(content_height))
                    .max();
                layout::Node::new(size)
            }
        }
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: iced::advanced::Layout<'_>,
        cursor_pos: mouse::Cursor,
        _renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let Some(on_action) = self.on_action.as_ref() else {
            return;
        };

        let widget_state = tree.state.downcast_mut::<WidgetState>();
        let bounds = layout.bounds();

        // Handle window focus/unfocus and redraw for cursor blink.
        match event {
            Event::Window(window::Event::Unfocused) => {
                if let Some(focus) = &mut widget_state.focus {
                    focus.is_window_focused = false;
                }
            }
            Event::Window(window::Event::Focused) => {
                if let Some(focus) = &mut widget_state.focus {
                    focus.is_window_focused = true;
                    focus.updated_at = Instant::now();
                    shell.request_redraw();
                }
            }
            Event::Window(window::Event::RedrawRequested(now)) => {
                if let Some(focus) = &mut widget_state.focus
                    && focus.is_window_focused
                {
                    focus.now = *now;

                    let elapsed = (focus.now - focus.updated_at).as_millis()
                        % FocusState::BLINK_INTERVAL_MILLIS;
                    let millis_until_redraw =
                        FocusState::BLINK_INTERVAL_MILLIS.saturating_sub(elapsed);

                    shell.request_redraw_at(
                        focus.now
                            + Duration::from_millis(
                                u64::try_from(millis_until_redraw).unwrap_or(500),
                            ),
                    );
                }
            }
            _ => {}
        }

        // Handle keyboard events.
        if let Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            modifiers,
            text,
            ..
        }) = event
            && widget_state.focus.is_some()
        {
                let text_str = text.as_deref();
                let key_action = input::map_key_event(key, *modifiers, text_str);

                match key_action {
                    KeyAction::Edit(edit_action) => {
                        shell.publish(on_action(Action::Edit(edit_action)));
                        shell.capture_event();
                    }
                    KeyAction::Move(move_action) => {
                        shell.publish(on_action(Action::Move(move_action)));
                        shell.capture_event();
                    }
                    KeyAction::Select(move_action) => {
                        shell.publish(on_action(Action::Select(move_action)));
                        shell.capture_event();
                    }
                    KeyAction::SelectAll => {
                        shell.publish(on_action(Action::SelectAll));
                        shell.capture_event();
                    }
                    KeyAction::Copy => {
                        let text = self.state.selection_text();
                        if !text.is_empty() {
                            clipboard.write(iced::advanced::clipboard::Kind::Standard, text);
                        }
                        // Emit Action::Copy so EditorState captures the structured slice.
                        shell.publish(on_action(Action::Copy));
                        shell.capture_event();
                    }
                    KeyAction::Cut => {
                        let text = self.state.selection_text();
                        if !text.is_empty() {
                            clipboard.write(iced::advanced::clipboard::Kind::Standard, text.clone());
                            // Emit Action::Cut so EditorState captures the structured
                            // slice and then deletes the selection.
                            shell.publish(on_action(Action::Cut));
                        }
                        shell.capture_event();
                    }
                    KeyAction::Paste => {
                        if let Some(contents) =
                            clipboard.read(iced::advanced::clipboard::Kind::Standard)
                        {
                            shell.publish(on_action(Action::Paste(contents)));
                        }
                        shell.capture_event();
                    }
                    KeyAction::Undo => {
                        shell.publish(on_action(Action::Undo));
                        shell.capture_event();
                    }
                    KeyAction::Redo => {
                        shell.publish(on_action(Action::Redo));
                        shell.capture_event();
                    }
                    KeyAction::None => {}
                }

                // Reset blink on any handled key.
                if let Some(focus) = &mut widget_state.focus {
                    focus.updated_at = Instant::now();
                }
        }

        // Handle mouse events.
        match event {
            Event::Mouse(mouse::Event::ButtonPressed { button: mouse::Button::Left, .. }) => {
                if let Some(position) = cursor_pos.position_in(bounds) {
                    // Translate to content coordinates (account for padding).
                    let content_pos = Point::new(
                        position.x - self.padding.left,
                        position.y - self.padding.top,
                    );

                    let doc_pos = hit_test_content_point(
                        content_pos,
                        &widget_state.cache,
                        &self.state.document,
                    );

                    widget_state.focus = Some(FocusState::now());
                    widget_state.dragging = true;

                    shell.publish(on_action(Action::Focus));
                    shell.publish(on_action(Action::Click(doc_pos)));
                    shell.capture_event();
                } else if widget_state.focus.is_some() {
                    // Click outside the editor: blur.
                    widget_state.focus = None;
                    widget_state.dragging = false;
                    shell.publish(on_action(Action::Blur));
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if widget_state.dragging
                    && let Some(position) = cursor_pos.position_in(bounds)
                {
                    let content_pos = Point::new(
                        position.x - self.padding.left,
                        position.y - self.padding.top,
                    );
                    let doc_pos = hit_test_content_point(
                        content_pos,
                        &widget_state.cache,
                        &self.state.document,
                    );
                    shell.publish(on_action(Action::Drag(doc_pos)));
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                widget_state.dragging = false;
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: iced::advanced::Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let widget_state = tree.state.downcast_ref::<WidgetState>();
        let bounds = layout.bounds();
        let text_bounds = bounds.shrink(self.padding);
        let cache = &widget_state.cache;

        // Draw each block.
        for (i, block) in self.state.document.blocks.iter().enumerate() {
            let Some(entry) = cache.get(i) else {
                continue;
            };

            let block_origin = Point::new(
                text_bounds.x,
                text_bounds.y + entry.y_offset(),
            );

            match block.as_ref() {
                Block::HorizontalRule => {
                    let hr_bounds = Rectangle::new(
                        block_origin,
                        Size::new(text_bounds.width, entry.height()),
                    );
                    render::draw_horizontal_rule(renderer, hr_bounds, self.text_color);
                }
                Block::BlockQuote { .. } => {
                    let bq_bounds = Rectangle::new(
                        block_origin,
                        Size::new(text_bounds.width, entry.height()),
                    );
                    render::draw_blockquote_border(renderer, bq_bounds, self.text_color);

                    for child in entry.child_paragraphs() {
                        let para_origin = Point::new(
                            block_origin.x + render::BLOCKQUOTE_INDENT,
                            block_origin.y + child.local_y_offset,
                        );
                        render::draw_paragraph(
                            renderer,
                            &child.paragraph,
                            para_origin,
                            self.text_color,
                            text_bounds,
                        );
                    }
                }
                Block::List { ordered, .. } => {
                    for (item_idx, child) in entry.child_paragraphs().iter().enumerate() {
                        let content_bounds = Rectangle::new(
                            Point::new(
                                block_origin.x + render::LIST_MARKER_WIDTH,
                                block_origin.y + child.local_y_offset,
                            ),
                            Size::new(
                                (text_bounds.width - render::LIST_MARKER_WIDTH).max(0.0),
                                child.height,
                            ),
                        );
                        render::draw_list_marker(
                            renderer,
                            content_bounds,
                            *ordered,
                            item_idx,
                            self.font,
                            self.text_color,
                            text_bounds,
                        );
                        render::draw_paragraph(
                            renderer,
                            &child.paragraph,
                            content_bounds.position(),
                            self.text_color,
                            text_bounds,
                        );
                    }
                }
                Block::Paragraph { .. } | Block::Heading { .. } => {
                    if let Some(paragraph) = entry.paragraph() {
                        render::draw_paragraph(
                            renderer,
                            paragraph,
                            block_origin,
                            self.text_color,
                            text_bounds,
                        );
                    }
                }
            }
        }

        // Draw selection highlights.
        if !self.state.selection.is_collapsed() {
            let sel_ranges = cursor::selection_block_ranges(self.state.selection);

            for (block_idx, kind) in &sel_ranges {
                let Some(entry) = cache.get(*block_idx) else {
                    continue;
                };

                let block_y = text_bounds.y + entry.y_offset();
                let content_x_off = self
                    .state
                    .document
                    .block(*block_idx)
                    .map(block_content_x_offset)
                    .unwrap_or(0.0);
                let para_origin_x = text_bounds.x + content_x_off;

                // For blocks with a paragraph, compute per-line selection rects.
                // For blocks without (e.g. HorizontalRule), fall back to full-block.
                let sel_rects: Vec<SelectionRect> = match (kind, entry.paragraph()) {
                    (BlockSelectionKind::Full, _) => {
                        vec![SelectionRect {
                            x: text_bounds.x,
                            y: block_y,
                            width: text_bounds.width,
                            height: entry.height(),
                        }]
                    }
                    (
                        BlockSelectionKind::Single {
                            start_offset,
                            end_offset,
                        },
                        Some(paragraph),
                    ) => compute_selection_rects(
                        paragraph,
                        *start_offset,
                        *end_offset,
                        para_origin_x,
                        block_y,
                        text_bounds.width,
                    ),
                    (BlockSelectionKind::First { start_offset }, Some(paragraph)) => {
                        compute_selection_rects(
                            paragraph,
                            *start_offset,
                            usize::MAX,
                            para_origin_x,
                            block_y,
                            text_bounds.width,
                        )
                    }
                    (BlockSelectionKind::Last { end_offset }, Some(paragraph)) => {
                        compute_selection_rects(
                            paragraph,
                            0,
                            *end_offset,
                            para_origin_x,
                            block_y,
                            text_bounds.width,
                        )
                    }
                    // No paragraph — fall back to full-block highlight.
                    (_, None) => {
                        vec![SelectionRect {
                            x: text_bounds.x,
                            y: block_y,
                            width: text_bounds.width,
                            height: entry.height(),
                        }]
                    }
                };

                for sel_rect in &sel_rects {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: Rectangle::new(
                                Point::new(sel_rect.x, sel_rect.y),
                                Size::new(sel_rect.width, sel_rect.height),
                            ),
                            ..Default::default()
                        },
                        self.selection_color,
                    );
                }
            }
        }

        // Draw cursor.
        if let Some(focus) = &widget_state.focus
            && focus.is_cursor_visible()
            && self.state.selection.is_collapsed()
        {
            let pos = self.state.selection.focus;
            if let Some(entry) = cache.get(pos.block_index) {
                // Compute the paragraph origin for this block type (accounting
                // for list/blockquote indentation).
                let content_x_off = self
                    .state
                    .document
                    .block(pos.block_index)
                    .map(block_content_x_offset)
                    .unwrap_or(0.0);

                let para_origin_x = text_bounds.x + content_x_off;
                let para_origin_y = text_bounds.y + entry.y_offset();

                let (cursor_x, cursor_y, cursor_height) =
                    if let Some(paragraph) = entry.paragraph() {
                        let gp = grapheme_pixel_position(paragraph, pos.offset);
                        let lh = paragraph_line_height_px(paragraph);
                        (para_origin_x + gp.x, para_origin_y + gp.y, lh)
                    } else {
                        // No paragraph (e.g. HorizontalRule) — fall back to
                        // block origin with a default line height.
                        let lh = render::FONT_SIZE_BODY * render::LINE_HEIGHT_MULTIPLIER;
                        (para_origin_x, para_origin_y, lh)
                    };

                let cursor_rect = Rectangle::new(
                    Point::new(cursor_x, cursor_y),
                    Size::new(CURSOR_WIDTH, cursor_height),
                );

                if let Some(clipped) = text_bounds.intersection(&cursor_rect) {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: clipped,
                            ..Default::default()
                        },
                        self.cursor_color,
                    );
                }
            }
        }
    }

    fn mouse_interaction(
        &self,
        _tree: &widget::Tree,
        layout: iced::advanced::Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let is_disabled = self.on_action.is_none();

        if cursor.is_over(layout.bounds()) {
            if is_disabled {
                mouse::Interaction::NotAllowed
            } else {
                mouse::Interaction::Text
            }
        } else {
            mouse::Interaction::default()
        }
    }
}

// ── Paste style helper ───────────────────────────────────

/// Determine the inline style that `InsertText` would inherit at the given
/// position. Mirrors the logic in `operations::insert_text_into_runs`: the
/// inserted text lands in whichever run contains the offset, so the
/// inherited style is that run's style.
fn run_style_at_for_paste(doc: &Document, pos: DocPosition) -> InlineStyle {
    let Some(block) = doc.block(pos.block_index) else {
        return InlineStyle::empty();
    };
    let Some(runs) = block.runs() else {
        return InlineStyle::empty();
    };
    if runs.is_empty() {
        return InlineStyle::empty();
    }

    let mut char_pos = 0;
    for run in runs {
        let run_len = run.char_len();
        if pos.offset >= char_pos && pos.offset <= char_pos + run_len {
            return run.style;
        }
        char_pos += run_len;
    }

    // Past the end: use last run's style.
    runs.last().map_or(InlineStyle::empty(), |r| r.style)
}

// ── Block content x-offset helper ────────────────────────

/// Returns the horizontal content offset for a block type. List and blockquote
/// blocks indent their paragraph content.
fn block_content_x_offset(block: &Block) -> f32 {
    match block {
        Block::List { .. } => render::LIST_MARKER_WIDTH,
        Block::BlockQuote { .. } => render::BLOCKQUOTE_INDENT,
        _ => 0.0,
    }
}

// ── Paragraph line mapping helpers ──────────────────────

/// Information about a visual line within a paragraph.
struct LineInfo {
    /// The visual line index.
    line: usize,
    /// The character offset at the start of this line (relative to block start).
    start_offset: usize,
}

/// Compute the absolute line height in pixels for a paragraph.
fn paragraph_line_height_px(paragraph: &IcedParagraph) -> f32 {
    let font_size: f32 = paragraph.size().0;
    font_size * render::LINE_HEIGHT_MULTIPLIER
}

/// Estimate the number of visual lines in a paragraph.
fn paragraph_line_count(paragraph: &IcedParagraph) -> usize {
    let line_height_px = paragraph_line_height_px(paragraph);
    if line_height_px <= 0.0 {
        return 1;
    }
    let total_height = paragraph.min_bounds().height;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let count = (total_height / line_height_px).ceil() as usize;
    count.max(1)
}

/// Build a list of `(line_index, start_char_offset)` pairs for all visual lines
/// in a paragraph. Uses `hit_test` at the left edge of each line to discover
/// line-start offsets.
fn build_line_starts(paragraph: &IcedParagraph) -> Vec<(usize, usize)> {
    let line_count = paragraph_line_count(paragraph);
    let line_height_px = paragraph_line_height_px(paragraph);

    let mut line_starts: Vec<(usize, usize)> = Vec::with_capacity(line_count);
    line_starts.push((0, 0));

    for line_idx in 1..line_count {
        let probe_y = line_idx as f32 * line_height_px;
        let probe = Point::new(0.0, probe_y);
        if let Some(hit) = paragraph.hit_test(probe) {
            let offset = hit.cursor();
            // Only add if this line starts at a different offset than the previous.
            if line_starts.last().is_some_and(|&(_, prev)| prev != offset) {
                line_starts.push((line_idx, offset));
            }
        }
    }

    line_starts
}

/// Find which visual line a character offset falls on within a paragraph.
/// Returns the line index and the character offset at the start of that line.
fn find_line_for_offset(
    paragraph: &IcedParagraph,
    char_offset: usize,
) -> LineInfo {
    let line_starts = build_line_starts(paragraph);

    // Find the last line whose start_offset <= char_offset.
    let mut best = LineInfo {
        line: 0,
        start_offset: 0,
    };
    for &(line_idx, start) in &line_starts {
        if start <= char_offset {
            best = LineInfo {
                line: line_idx,
                start_offset: start,
            };
        } else {
            break;
        }
    }

    best
}

/// Compute the pixel position of a character offset within a paragraph.
/// Returns a point relative to the paragraph origin, or `(0, 0)` as fallback.
fn grapheme_pixel_position(
    paragraph: &IcedParagraph,
    char_offset: usize,
) -> Point {
    let line_info = find_line_for_offset(paragraph, char_offset);
    let within_line = char_offset.saturating_sub(line_info.start_offset);

    paragraph
        .grapheme_position(line_info.line, within_line)
        .unwrap_or(Point::ORIGIN)
}

// ── Selection rectangle computation ─────────────────────

/// Compute per-line selection rectangles for a partial selection within a block.
///
/// `start_offset` and `end_offset` are character offsets within the block.
/// `para_origin_x` / `para_origin_y` are the absolute pixel positions of the
/// paragraph origin. `available_width` is the full text area width.
///
/// Returns a list of `SelectionRect`s, one per visual line that intersects the
/// selection range.
fn compute_selection_rects(
    paragraph: &IcedParagraph,
    start_offset: usize,
    end_offset: usize,
    para_origin_x: f32,
    para_origin_y: f32,
    available_width: f32,
) -> Vec<SelectionRect> {
    let line_height_px = paragraph_line_height_px(paragraph);
    let line_starts = build_line_starts(paragraph);
    let line_count = line_starts.len();
    let mut rects = Vec::new();

    for (i, &(line_idx, line_start)) in line_starts.iter().enumerate() {
        // Determine the character range for this line.
        let line_end = if i + 1 < line_count {
            line_starts[i + 1].1
        } else {
            usize::MAX // last line extends to end of block
        };

        // Skip lines that don't overlap the selection.
        if line_start >= end_offset || line_end <= start_offset {
            continue;
        }

        let line_y = para_origin_y + line_idx as f32 * line_height_px;

        // Determine x-coordinates for this line's selection portion.
        let sel_start_in_line = start_offset.max(line_start);
        let sel_end_in_line = end_offset.min(line_end);

        let x_start = if sel_start_in_line <= line_start {
            // Selection starts at or before this line — use left edge.
            para_origin_x
        } else {
            let within_line = sel_start_in_line.saturating_sub(line_start);
            let pos = paragraph
                .grapheme_position(line_idx, within_line)
                .unwrap_or(Point::ORIGIN);
            para_origin_x + pos.x
        };

        let x_end = if sel_end_in_line >= line_end && i + 1 < line_count {
            // Selection extends to or past the end of this line — use right edge.
            para_origin_x + available_width
        } else {
            let within_line = sel_end_in_line.saturating_sub(line_start);
            let pos = paragraph
                .grapheme_position(line_idx, within_line)
                .unwrap_or(Point::ORIGIN);
            para_origin_x + pos.x
        };

        let width = (x_end - x_start).max(0.0);
        if width > 0.0 {
            rects.push(SelectionRect {
                x: x_start,
                y: line_y,
                width,
                height: line_height_px,
            });
        }
    }

    rects
}

// ── Hit testing helper ───────────────────────────────────

/// Convert a pixel position (relative to the content origin, after padding) to a
/// `DocPosition` by hit-testing the paragraph cache.
///
/// Finds which block the point falls in via `ParagraphCache::block_at_y`, then
/// calls `Paragraph::hit_test` on that block's cached paragraph to get the
/// character offset within the block.
fn hit_test_content_point(
    content_pos: Point,
    cache: &ParagraphCache<IcedParagraph>,
    document: &Document,
) -> DocPosition {
    // Find the block at the click y-coordinate.
    let Some(block_index) = cache.block_at_y(content_pos.y) else {
        return DocPosition::zero();
    };

    let Some(entry) = cache.get(block_index) else {
        return DocPosition::new(block_index, 0);
    };

    let content_x_offset = document
        .block(block_index)
        .map(block_content_x_offset)
        .unwrap_or(0.0);

    // For container blocks (List, BlockQuote) with child paragraphs,
    // find which child the click falls in and hit-test that child.
    let children = entry.child_paragraphs();
    if !children.is_empty() {
        let local_y = content_pos.y - entry.y_offset();

        // Find the child whose y-range contains the click. Fall back to
        // the last child if the click is below all children.
        let child = children
            .iter()
            .rev()
            .find(|c| local_y >= c.local_y_offset)
            .unwrap_or(&children[0]);

        let local_point = Point::new(
            content_pos.x - content_x_offset,
            local_y - child.local_y_offset,
        );

        let char_offset = child
            .paragraph
            .hit_test(local_point)
            .map(iced::advanced::text::Hit::cursor)
            .unwrap_or(0);

        return DocPosition::new(block_index, char_offset);
    }

    // For inline blocks, hit-test the single paragraph.
    let Some(paragraph) = entry.paragraph() else {
        // No paragraph (e.g. HorizontalRule) — place cursor at start of block.
        return DocPosition::new(block_index, 0);
    };

    // Translate into paragraph-local coordinates.
    let local_point = Point::new(
        content_pos.x - content_x_offset,
        content_pos.y - entry.y_offset(),
    );

    let char_offset = paragraph
        .hit_test(local_point)
        .map(iced::advanced::text::Hit::cursor)
        .unwrap_or(0);

    DocPosition::new(block_index, char_offset)
}

// ── Into<Element> ───────────────────────────────────────

impl<'a, Message: 'a> From<RichTextEditor<'a, Message>>
    for Element<'a, Message, iced::Theme, iced::Renderer>
{
    fn from(editor: RichTextEditor<'a, Message>) -> Self {
        Self::new(editor)
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
        let doc = Document::from_blocks(vec![
            Block::paragraph("hello"),
            Block::paragraph("world"),
        ]);
        let state = EditorState::from_document(doc);
        assert_eq!(state.document.block_count(), 2);
        assert_eq!(state.document.block(0).map(Block::flattened_text).as_deref(), Some("hello"));
        assert_eq!(state.document.block(1).map(Block::flattened_text).as_deref(), Some("world"));
    }

    #[test]
    fn from_html_parses() {
        let state = EditorState::from_html("<p>hello</p>");
        assert_eq!(state.document.block(0).map(Block::flattened_text).as_deref(), Some("hello"));
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
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello world"),
        ]));
        state.selection = DocSelection::range(
            DocPosition::new(0, 0),
            DocPosition::new(0, 5),
        );
        assert_eq!(state.selection_text(), "hello");
    }

    #[test]
    fn selection_text_cross_block() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
            Block::paragraph("world"),
        ]));
        state.selection = DocSelection::range(
            DocPosition::new(0, 3),
            DocPosition::new(1, 2),
        );
        let text = state.selection_text();
        assert!(text.contains("lo"));
        assert!(text.contains("wo"));
    }

    // ── EditorState::apply_action — insert ───────────────

    #[test]
    fn apply_action_insert_text() {
        let mut state = EditorState::new();
        state.apply_action(EditAction::InsertText("hello".into()));
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
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
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("hel"),
        );
        assert_eq!(state.selection.focus, DocPosition::new(0, 3));
    }

    // ── EditorState::apply_action — delete ───────────────

    #[test]
    fn apply_action_delete_backward() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
        ]));
        state.selection = DocSelection::caret(DocPosition::new(0, 5));
        state.apply_action(EditAction::DeleteBackward);
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("hell"),
        );
        assert_eq!(state.selection.focus, DocPosition::new(0, 4));
    }

    #[test]
    fn apply_action_delete_forward() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
        ]));
        state.selection = DocSelection::caret(DocPosition::new(0, 0));
        state.apply_action(EditAction::DeleteForward);
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("ello"),
        );
    }

    #[test]
    fn apply_action_delete_selection() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello world"),
        ]));
        state.selection = DocSelection::range(
            DocPosition::new(0, 5),
            DocPosition::new(0, 11),
        );
        state.apply_action(EditAction::DeleteSelection);
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("hello"),
        );
    }

    // ── EditorState::apply_action — split block ──────────

    #[test]
    fn apply_action_split_block() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello world"),
        ]));
        state.selection = DocSelection::caret(DocPosition::new(0, 5));
        state.apply_action(EditAction::SplitBlock);
        assert_eq!(state.document.block_count(), 2);
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("hello"),
        );
        assert_eq!(
            state.document.block(1).map(Block::flattened_text).as_deref(),
            Some(" world"),
        );
        // Cursor should be at start of new block.
        assert_eq!(state.selection.focus, DocPosition::new(1, 0));
    }

    // ── EditorState::apply_action — toggle inline style ──

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
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello world"),
        ]));
        state.selection = DocSelection::range(
            DocPosition::new(0, 0),
            DocPosition::new(0, 5),
        );
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
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("hello"),
        );

        state.undo();
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
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
            state.document.block(0).map(Block::flattened_text).as_deref(),
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
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("abc"),
        );

        state.undo(); // remove "c"
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("ab"),
        );

        state.undo(); // remove "b"
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("a"),
        );

        state.redo(); // re-add "b"
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("ab"),
        );
    }

    // ── EditorState::apply_move ──────────────────────────

    #[test]
    fn move_left() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
        ]));
        state.selection = DocSelection::caret(DocPosition::new(0, 3));
        state.perform(Action::Move(MoveAction::Left));
        assert_eq!(state.selection.focus, DocPosition::new(0, 2));
        assert!(state.selection.is_collapsed());
    }

    #[test]
    fn move_right() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
        ]));
        state.selection = DocSelection::caret(DocPosition::new(0, 2));
        state.perform(Action::Move(MoveAction::Right));
        assert_eq!(state.selection.focus, DocPosition::new(0, 3));
    }

    #[test]
    fn move_left_collapses_selection_to_start() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello world"),
        ]));
        state.selection = DocSelection::range(
            DocPosition::new(0, 2),
            DocPosition::new(0, 7),
        );
        state.perform(Action::Move(MoveAction::Left));
        assert!(state.selection.is_collapsed());
        assert_eq!(state.selection.focus, DocPosition::new(0, 2));
    }

    #[test]
    fn move_right_collapses_selection_to_end() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello world"),
        ]));
        state.selection = DocSelection::range(
            DocPosition::new(0, 2),
            DocPosition::new(0, 7),
        );
        state.perform(Action::Move(MoveAction::Right));
        assert!(state.selection.is_collapsed());
        assert_eq!(state.selection.focus, DocPosition::new(0, 7));
    }

    #[test]
    fn select_extends_selection() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
        ]));
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
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello world"),
        ]));
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
            Block::paragraph("xy"),                    // len 2
            Block::paragraph("12345678901234567890"),  // len 20
        ]));
        state.selection = DocSelection::caret(DocPosition::new(0, 10));

        // Move down to block 1 — offset clamped to 2
        state.perform(Action::Move(MoveAction::Down));
        assert_eq!(state.selection.focus, DocPosition::new(1, 2));

        // Move down to block 2 — offset should recover to 10 (the saved target)
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

        // Move up to block 1 — clamped to 2
        state.perform(Action::Move(MoveAction::Up));
        assert_eq!(state.selection.focus, DocPosition::new(1, 2));

        // Move up to block 0 — offset recovers to 12
        state.perform(Action::Move(MoveAction::Up));
        assert_eq!(state.selection.focus, DocPosition::new(0, 12));
    }

    #[test]
    fn horizontal_move_clears_target_column() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("abcdefghij"), // len 10
            Block::paragraph("xy"),          // len 2
            Block::paragraph("abcdefghij"), // len 10
        ]));
        state.selection = DocSelection::caret(DocPosition::new(0, 8));

        // Move down — saves target_column = 8, lands at (1, 2)
        state.perform(Action::Move(MoveAction::Down));
        assert_eq!(state.selection.focus, DocPosition::new(1, 2));

        // Horizontal move clears target_column
        state.perform(Action::Move(MoveAction::Left));
        assert_eq!(state.selection.focus, DocPosition::new(1, 1));

        // Move down again — target_column is now 1 (the current offset)
        state.perform(Action::Move(MoveAction::Down));
        assert_eq!(state.selection.focus, DocPosition::new(2, 1));
    }

    #[test]
    fn vertical_move_at_boundary_clamps_correctly() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("abc"),
        ]));
        state.selection = DocSelection::caret(DocPosition::new(0, 2));

        // Move up at top boundary — goes to (0, 0)
        state.perform(Action::Move(MoveAction::Up));
        assert_eq!(state.selection.focus, DocPosition::new(0, 0));

        // Move down at bottom boundary — goes to end position
        state.selection = DocSelection::caret(DocPosition::new(0, 2));
        state.perform(Action::Move(MoveAction::Down));
        assert_eq!(state.selection.focus, DocPosition::new(0, 3));
    }

    // ── EditorState::set_selection ───────────────────────

    #[test]
    fn set_selection() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
        ]));
        let sel = DocSelection::range(
            DocPosition::new(0, 1),
            DocPosition::new(0, 4),
        );
        state.set_selection(sel);
        assert_eq!(state.selection, sel);
    }

    // ── EditorState::perform — focus / blur ──────────────

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
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello world"),
        ]));
        state.selection = DocSelection::range(
            DocPosition::new(0, 5),
            DocPosition::new(0, 11),
        );
        state.apply_action(EditAction::InsertText("!".into()));
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
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
            state.document.block(0).map(Block::flattened_text).as_deref(),
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
            state.document.block(0).map(Block::flattened_text).as_deref(),
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

        // Type 'x' — should be bold.
        state.apply_action(EditAction::InsertText("x".into()));
        assert!(state.pending_style.is_empty(), "pending style should be cleared after insert");

        let runs = state.document.block(0).and_then(Block::runs).expect("runs");
        // The 'x' run should be bold.
        let bold_runs: Vec<_> = runs.iter().filter(|r| !r.is_empty()).collect();
        assert!(
            bold_runs.iter().all(|r| r.style.contains(InlineStyle::BOLD)),
            "all non-empty runs should be bold, got: {bold_runs:?}",
        );
    }

    // ── Clipboard: internal copy/paste ───────────────────

    #[test]
    fn copy_captures_internal_clipboard() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello world"),
        ]));
        state.selection = DocSelection::range(
            DocPosition::new(0, 0),
            DocPosition::new(0, 5),
        );
        state.perform(Action::Copy);
        assert!(state.internal_clipboard.is_some());
        let ic = state.internal_clipboard.as_ref().expect("clipboard");
        assert_eq!(ic.plain_text, "hello");
        assert!(ic.slice.open_start);
        assert!(ic.slice.open_end);
    }

    #[test]
    fn copy_paste_plain_text_within_single_block() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello world"),
        ]));
        // Copy "world"
        state.selection = DocSelection::range(
            DocPosition::new(0, 6),
            DocPosition::new(0, 11),
        );
        state.perform(Action::Copy);

        // Move cursor to start and paste
        state.selection = DocSelection::caret(DocPosition::new(0, 0));
        state.perform(Action::Paste("world".into()));

        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("worldhello world"),
        );
    }

    #[test]
    fn copy_paste_preserves_bold_formatting() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::Paragraph {
                runs: vec![
                    StyledRun::plain("hello "),
                    StyledRun::styled("bold", InlineStyle::BOLD),
                    StyledRun::plain(" text"),
                ],
            },
        ]));
        // Copy "bold" (offsets 6..10)
        state.selection = DocSelection::range(
            DocPosition::new(0, 6),
            DocPosition::new(0, 10),
        );
        state.perform(Action::Copy);

        // Move cursor to end and paste
        state.selection = DocSelection::caret(DocPosition::new(0, 15));
        state.perform(Action::Paste("bold".into()));

        // The pasted "bold" at the end should be bold.
        let runs = state.document.block(0).and_then(Block::runs).expect("runs");
        let total_text: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(total_text, "hello bold textbold");

        // Find the last "bold" — it should have BOLD style.
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
        state.selection = DocSelection::range(
            DocPosition::new(0, 0),
            DocPosition::new(1, 6),
        );
        state.perform(Action::Copy);

        // Paste at end of "third"
        state.selection = DocSelection::caret(DocPosition::new(2, 5));
        state.perform(Action::Paste("first\nsecond".into()));

        // Should now have: "first", "second", "thirdfirst", "second"
        assert_eq!(state.document.block_count(), 4);
        assert_eq!(
            state.document.block(2).map(Block::flattened_text).as_deref(),
            Some("thirdfirst"),
        );
        assert_eq!(
            state.document.block(3).map(Block::flattened_text).as_deref(),
            Some("second"),
        );
    }

    #[test]
    fn cut_captures_and_deletes() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello world"),
        ]));
        state.selection = DocSelection::range(
            DocPosition::new(0, 5),
            DocPosition::new(0, 11),
        );
        state.perform(Action::Cut);

        // Text should be deleted.
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("hello"),
        );

        // Internal clipboard should have the cut content.
        assert!(state.internal_clipboard.is_some());
        let ic = state.internal_clipboard.as_ref().expect("clipboard");
        assert_eq!(ic.plain_text, " world");
    }

    #[test]
    fn paste_external_text_when_clipboard_differs() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
        ]));
        // Copy "hello"
        state.selection = DocSelection::range(
            DocPosition::new(0, 0),
            DocPosition::new(0, 5),
        );
        state.perform(Action::Copy);

        // Now paste something different from the system clipboard
        // (simulating the user copying from another app).
        state.selection = DocSelection::caret(DocPosition::new(0, 5));
        state.perform(Action::Paste("external".into()));

        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("helloexternal"),
        );
    }

    #[test]
    fn paste_replaces_selection() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello world"),
        ]));
        // Copy "hello"
        state.selection = DocSelection::range(
            DocPosition::new(0, 0),
            DocPosition::new(0, 5),
        );
        state.perform(Action::Copy);

        // Select " world" and paste "hello" over it.
        state.selection = DocSelection::range(
            DocPosition::new(0, 5),
            DocPosition::new(0, 11),
        );
        state.perform(Action::Paste("hello".into()));

        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
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
        state.selection = DocSelection::range(
            DocPosition::new(0, 0),
            DocPosition::new(1, 3),
        );
        state.perform(Action::Copy);

        // Paste in the middle of "target text" at offset 6
        state.selection = DocSelection::caret(DocPosition::new(2, 6));
        state.perform(Action::Paste("aaa\nbbb".into()));

        // Expected: "target" + "aaa" content merged, then "bbb" + " text" merged
        // Block 0: "aaa", Block 1: "bbb", Block 2: "targetaaa", Block 3: "bbb text"
        assert!(state.document.block_count() >= 4);
        assert_eq!(
            state.document.block(2).map(Block::flattened_text).as_deref(),
            Some("targetaaa"),
        );
        assert_eq!(
            state.document.block(3).map(Block::flattened_text).as_deref(),
            Some("bbb text"),
        );
    }

    #[test]
    fn collapsed_copy_does_not_capture() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
        ]));
        state.selection = DocSelection::caret(DocPosition::new(0, 3));
        state.perform(Action::Copy);
        assert!(state.internal_clipboard.is_none());
    }
}
