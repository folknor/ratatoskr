//! Key binding → action mapping and cursor movement helpers.
//!
//! This module maps iced keyboard events to semantic editor actions ([`KeyAction`]),
//! and provides cursor movement functions that operate on the document model.
//!
//! The key binding table follows standard desktop text editor conventions:
//! - Arrow keys for movement (with Shift for selection, Ctrl for word/document jumps)
//! - Ctrl+B/I/U for format toggles
//! - Ctrl+C/X/V for clipboard
//! - Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y for undo/redo

use crate::document::{DocPosition, Document, InlineStyle};
use crate::rules::EditAction;

use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};

// ── Key action types ────────────────────────────────────

/// A keyboard action that the editor should handle.
/// Maps 1:1 from keyboard events to semantic editor actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    /// A text editing action (resolved by rules.rs).
    Edit(EditAction),
    /// Move cursor (no edit).
    Move(MoveAction),
    /// Select (extend selection while moving).
    Select(MoveAction),
    /// Select all text.
    SelectAll,
    /// Copy selection to clipboard.
    Copy,
    /// Cut selection to clipboard.
    Cut,
    /// Paste from clipboard.
    Paste,
    /// Undo.
    Undo,
    /// Redo.
    Redo,
    /// No action (key not handled).
    None,
}

/// Cursor movement directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveAction {
    /// Move left one character.
    Left,
    /// Move right one character.
    Right,
    /// Move up one line.
    Up,
    /// Move down one line.
    Down,
    /// Move to start of line.
    Home,
    /// Move to end of line.
    End,
    /// Move left one word (Ctrl+Left).
    WordLeft,
    /// Move right one word (Ctrl+Right).
    WordRight,
    /// Move to start of document (Ctrl+Home).
    DocumentStart,
    /// Move to end of document (Ctrl+End).
    DocumentEnd,
}

// ── Key event mapping ───────────────────────────────────

/// Map a keyboard event to a [`KeyAction`].
///
/// This is the central key binding dispatch. It handles:
/// - Format shortcuts (Ctrl+B/I/U)
/// - Navigation (arrows, Home/End, Ctrl+arrows for word movement)
/// - Editing (Enter, Backspace, Delete)
/// - Clipboard (Ctrl+C/X/V)
/// - Undo/Redo (Ctrl+Z, Ctrl+Shift+Z or Ctrl+Y)
/// - Text input (characters)
pub fn map_key_event(key: &Key, modifiers: Modifiers, text: Option<&str>) -> KeyAction {
    let cmd = modifiers.command();
    let shift = modifiers.shift();

    // First: check command (Ctrl/Cmd) shortcuts on character keys.
    if cmd && let Some(action) = map_command_shortcut(key, shift) {
        return action;
    }

    // Second: named keys (arrows, Home/End, Backspace, Delete, Enter).
    if let Key::Named(named) = key
        && let Some(action) = map_named_key(*named, cmd, shift)
    {
        return action;
    }

    // Third: text input (non-control characters) when no command modifier is held.
    if !cmd
        && let Some(text) = text
        && let Some(ch) = text.chars().find(|c| !c.is_control())
    {
        return KeyAction::Edit(EditAction::InsertText(ch.to_string()));
    }

    KeyAction::None
}

/// Map a command (Ctrl/Cmd) + key combination to an action.
fn map_command_shortcut(key: &Key, shift: bool) -> Option<KeyAction> {
    let ch = match key.as_ref() {
        Key::Character(c) => c,
        _ => return Option::None,
    };

    match ch {
        "b" => Some(KeyAction::Edit(EditAction::ToggleInlineStyle(
            InlineStyle::BOLD,
        ))),
        "i" => Some(KeyAction::Edit(EditAction::ToggleInlineStyle(
            InlineStyle::ITALIC,
        ))),
        "u" => Some(KeyAction::Edit(EditAction::ToggleInlineStyle(
            InlineStyle::UNDERLINE,
        ))),
        "c" => Some(KeyAction::Copy),
        "x" => Some(KeyAction::Cut),
        "v" => Some(KeyAction::Paste),
        "z" if shift => Some(KeyAction::Redo),
        "z" => Some(KeyAction::Undo),
        "y" => Some(KeyAction::Redo),
        "a" => Some(KeyAction::SelectAll),
        _ => Option::None,
    }
}

/// Map a named key (arrows, Home/End, etc.) to an action.
fn map_named_key(named: Named, cmd: bool, shift: bool) -> Option<KeyAction> {
    match named {
        // Arrow keys
        Named::ArrowLeft => Some(arrow_action(
            MoveAction::Left,
            MoveAction::WordLeft,
            cmd,
            shift,
        )),
        Named::ArrowRight => Some(arrow_action(
            MoveAction::Right,
            MoveAction::WordRight,
            cmd,
            shift,
        )),
        Named::ArrowUp if !cmd => Some(if shift {
            KeyAction::Select(MoveAction::Up)
        } else {
            KeyAction::Move(MoveAction::Up)
        }),
        Named::ArrowDown if !cmd => Some(if shift {
            KeyAction::Select(MoveAction::Down)
        } else {
            KeyAction::Move(MoveAction::Down)
        }),

        // Home/End
        Named::Home => Some(arrow_action(
            MoveAction::Home,
            MoveAction::DocumentStart,
            cmd,
            shift,
        )),
        Named::End => Some(arrow_action(
            MoveAction::End,
            MoveAction::DocumentEnd,
            cmd,
            shift,
        )),

        // Editing keys
        Named::Backspace => Some(KeyAction::Edit(EditAction::DeleteBackward)),
        Named::Delete => Some(KeyAction::Edit(EditAction::DeleteForward)),
        Named::Enter => Some(KeyAction::Edit(EditAction::SplitBlock)),

        _ => Option::None,
    }
}

/// Resolve an arrow/home/end key into Move or Select, with optional Ctrl widening.
fn arrow_action(base: MoveAction, widened: MoveAction, cmd: bool, shift: bool) -> KeyAction {
    let motion = if cmd { widened } else { base };
    if shift {
        KeyAction::Select(motion)
    } else {
        KeyAction::Move(motion)
    }
}

// ── Character classification for word movement ──────────

/// Character class for word boundary detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharClass {
    /// Alphanumeric or underscore (word characters).
    Word,
    /// Whitespace.
    Whitespace,
    /// Punctuation and everything else.
    Other,
}

fn classify(ch: char) -> CharClass {
    if ch.is_alphanumeric() || ch == '_' {
        CharClass::Word
    } else if ch.is_whitespace() {
        CharClass::Whitespace
    } else {
        CharClass::Other
    }
}

// ── Cursor movement helpers ─────────────────────────────

/// Move cursor left by one character in the document.
/// At start of block, moves to end of previous block.
pub fn move_left(doc: &Document, pos: DocPosition) -> DocPosition {
    if pos.offset > 0 {
        return DocPosition::new(pos.block_index, pos.offset - 1);
    }

    // At start of block: move to end of previous block.
    if pos.block_index > 0 {
        let prev_len = doc
            .block(pos.block_index - 1)
            .map_or(0, super::super::document::Block::char_len);
        return DocPosition::new(pos.block_index - 1, prev_len);
    }

    // Already at document start.
    pos
}

/// Move cursor right by one character.
/// At end of block, moves to start of next block.
pub fn move_right(doc: &Document, pos: DocPosition) -> DocPosition {
    let block_len = doc
        .block(pos.block_index)
        .map_or(0, super::super::document::Block::char_len);

    if pos.offset < block_len {
        return DocPosition::new(pos.block_index, pos.offset + 1);
    }

    // At end of block: move to start of next block.
    if pos.block_index + 1 < doc.block_count() {
        return DocPosition::new(pos.block_index + 1, 0);
    }

    // Already at document end.
    pos
}

/// Move cursor left by one word.
/// A "word" is a contiguous sequence of characters with the same class.
/// Skips whitespace first, then moves through the word.
pub fn word_left(doc: &Document, pos: DocPosition) -> DocPosition {
    let text = block_text(doc, pos.block_index);
    let chars: Vec<char> = text.chars().collect();

    if pos.offset == 0 {
        // At start of block: move to end of previous block, then word-left there.
        if pos.block_index > 0 {
            let prev_len = doc
                .block(pos.block_index - 1)
                .map_or(0, super::super::document::Block::char_len);
            return DocPosition::new(pos.block_index - 1, prev_len);
        }
        return pos;
    }

    let mut idx = pos.offset;

    // Skip whitespace going left.
    while idx > 0 && classify(chars[idx - 1]) == CharClass::Whitespace {
        idx -= 1;
    }

    if idx == 0 {
        return DocPosition::new(pos.block_index, 0);
    }

    // Now skip characters of the same class.
    let target_class = classify(chars[idx - 1]);
    while idx > 0 && classify(chars[idx - 1]) == target_class {
        idx -= 1;
    }

    DocPosition::new(pos.block_index, idx)
}

/// Move cursor right by one word.
/// Skips the current word, then any whitespace after it.
pub fn word_right(doc: &Document, pos: DocPosition) -> DocPosition {
    let text = block_text(doc, pos.block_index);
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();

    if pos.offset >= len {
        // At end of block: move to start of next block.
        if pos.block_index + 1 < doc.block_count() {
            return DocPosition::new(pos.block_index + 1, 0);
        }
        return pos;
    }

    let mut idx = pos.offset;

    // Skip characters of the current class.
    let start_class = classify(chars[idx]);
    while idx < len && classify(chars[idx]) == start_class {
        idx += 1;
    }

    // Skip whitespace after the word.
    while idx < len && classify(chars[idx]) == CharClass::Whitespace {
        idx += 1;
    }

    DocPosition::new(pos.block_index, idx)
}

/// Move to start of current block.
pub fn home(pos: DocPosition) -> DocPosition {
    DocPosition::new(pos.block_index, 0)
}

/// Move to end of current block.
pub fn end(doc: &Document, pos: DocPosition) -> DocPosition {
    let block_len = doc
        .block(pos.block_index)
        .map_or(0, super::super::document::Block::char_len);
    DocPosition::new(pos.block_index, block_len)
}

/// Move to start of document.
pub fn document_start() -> DocPosition {
    DocPosition::zero()
}

/// Move to end of document.
pub fn document_end(doc: &Document) -> DocPosition {
    doc.end_position()
}

/// Get the flattened text of a block, returning an empty string if the index is invalid.
fn block_text(doc: &Document, block_index: usize) -> String {
    doc.block(block_index)
        .map_or_else(String::new, super::super::document::Block::flattened_text)
}

// ── Word / block selection helpers (double/triple click) ─

/// Find the word boundaries around a character offset within a block.
///
/// Returns `(start, end)` char offsets. A "word" is a contiguous run of
/// characters sharing the same `CharClass`. If the cursor is between two
/// classes, the word to the left of the cursor is selected (left-affinity).
pub fn word_at(doc: &Document, pos: DocPosition) -> (DocPosition, DocPosition) {
    use super::super::document::Block;

    let text = block_text(doc, pos.block_index);
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();

    if len == 0 {
        let p = DocPosition::new(pos.block_index, 0);
        return (p, p);
    }

    // Determine the character class at (or just before) the cursor.
    let anchor_idx = if pos.offset > 0 && pos.offset <= len {
        pos.offset - 1
    } else if pos.offset < len {
        pos.offset
    } else {
        len - 1
    };
    let target_class = classify(chars[anchor_idx]);

    // Expand left.
    let mut start = anchor_idx;
    while start > 0 && classify(chars[start - 1]) == target_class {
        start -= 1;
    }

    // Expand right.
    let mut end = anchor_idx + 1;
    while end < len && classify(chars[end]) == target_class {
        end += 1;
    }

    let block_len = doc.block(pos.block_index).map_or(0, Block::char_len);
    let end = end.min(block_len);

    (
        DocPosition::new(pos.block_index, start),
        DocPosition::new(pos.block_index, end),
    )
}

/// Select an entire block (triple-click).
///
/// Returns anchor at block start and focus at block end.
pub fn select_block(doc: &Document, pos: DocPosition) -> (DocPosition, DocPosition) {
    use super::super::document::Block;

    let block_len = doc.block(pos.block_index).map_or(0, Block::char_len);
    (
        DocPosition::new(pos.block_index, 0),
        DocPosition::new(pos.block_index, block_len),
    )
}

// ── Tests ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Block, DocPosition, Document};

    // ── Key mapping tests ────────────────────────────────

    mod key_mapping {
        use super::*;

        fn no_mod() -> Modifiers {
            Modifiers::empty()
        }

        fn shift() -> Modifiers {
            Modifiers::SHIFT
        }

        fn ctrl() -> Modifiers {
            Modifiers::COMMAND
        }

        fn ctrl_shift() -> Modifiers {
            Modifiers::COMMAND | Modifiers::SHIFT
        }

        // Arrow keys

        #[test]
        fn arrow_left_no_modifier() {
            let action = map_key_event(&Key::Named(Named::ArrowLeft), no_mod(), Option::None);
            assert_eq!(action, KeyAction::Move(MoveAction::Left));
        }

        #[test]
        fn arrow_left_shift() {
            let action = map_key_event(&Key::Named(Named::ArrowLeft), shift(), Option::None);
            assert_eq!(action, KeyAction::Select(MoveAction::Left));
        }

        #[test]
        fn arrow_left_ctrl() {
            let action = map_key_event(&Key::Named(Named::ArrowLeft), ctrl(), Option::None);
            assert_eq!(action, KeyAction::Move(MoveAction::WordLeft));
        }

        #[test]
        fn arrow_left_ctrl_shift() {
            let action = map_key_event(&Key::Named(Named::ArrowLeft), ctrl_shift(), Option::None);
            assert_eq!(action, KeyAction::Select(MoveAction::WordLeft));
        }

        #[test]
        fn arrow_right_no_modifier() {
            let action = map_key_event(&Key::Named(Named::ArrowRight), no_mod(), Option::None);
            assert_eq!(action, KeyAction::Move(MoveAction::Right));
        }

        #[test]
        fn arrow_right_shift() {
            let action = map_key_event(&Key::Named(Named::ArrowRight), shift(), Option::None);
            assert_eq!(action, KeyAction::Select(MoveAction::Right));
        }

        #[test]
        fn arrow_right_ctrl() {
            let action = map_key_event(&Key::Named(Named::ArrowRight), ctrl(), Option::None);
            assert_eq!(action, KeyAction::Move(MoveAction::WordRight));
        }

        #[test]
        fn arrow_right_ctrl_shift() {
            let action = map_key_event(&Key::Named(Named::ArrowRight), ctrl_shift(), Option::None);
            assert_eq!(action, KeyAction::Select(MoveAction::WordRight));
        }

        #[test]
        fn arrow_up_no_modifier() {
            let action = map_key_event(&Key::Named(Named::ArrowUp), no_mod(), Option::None);
            assert_eq!(action, KeyAction::Move(MoveAction::Up));
        }

        #[test]
        fn arrow_up_shift() {
            let action = map_key_event(&Key::Named(Named::ArrowUp), shift(), Option::None);
            assert_eq!(action, KeyAction::Select(MoveAction::Up));
        }

        #[test]
        fn arrow_down_no_modifier() {
            let action = map_key_event(&Key::Named(Named::ArrowDown), no_mod(), Option::None);
            assert_eq!(action, KeyAction::Move(MoveAction::Down));
        }

        #[test]
        fn arrow_down_shift() {
            let action = map_key_event(&Key::Named(Named::ArrowDown), shift(), Option::None);
            assert_eq!(action, KeyAction::Select(MoveAction::Down));
        }

        // Home/End

        #[test]
        fn home_no_modifier() {
            let action = map_key_event(&Key::Named(Named::Home), no_mod(), Option::None);
            assert_eq!(action, KeyAction::Move(MoveAction::Home));
        }

        #[test]
        fn home_shift() {
            let action = map_key_event(&Key::Named(Named::Home), shift(), Option::None);
            assert_eq!(action, KeyAction::Select(MoveAction::Home));
        }

        #[test]
        fn home_ctrl() {
            let action = map_key_event(&Key::Named(Named::Home), ctrl(), Option::None);
            assert_eq!(action, KeyAction::Move(MoveAction::DocumentStart));
        }

        #[test]
        fn home_ctrl_shift() {
            let action = map_key_event(&Key::Named(Named::Home), ctrl_shift(), Option::None);
            assert_eq!(action, KeyAction::Select(MoveAction::DocumentStart));
        }

        #[test]
        fn end_no_modifier() {
            let action = map_key_event(&Key::Named(Named::End), no_mod(), Option::None);
            assert_eq!(action, KeyAction::Move(MoveAction::End));
        }

        #[test]
        fn end_shift() {
            let action = map_key_event(&Key::Named(Named::End), shift(), Option::None);
            assert_eq!(action, KeyAction::Select(MoveAction::End));
        }

        #[test]
        fn end_ctrl() {
            let action = map_key_event(&Key::Named(Named::End), ctrl(), Option::None);
            assert_eq!(action, KeyAction::Move(MoveAction::DocumentEnd));
        }

        #[test]
        fn end_ctrl_shift() {
            let action = map_key_event(&Key::Named(Named::End), ctrl_shift(), Option::None);
            assert_eq!(action, KeyAction::Select(MoveAction::DocumentEnd));
        }

        // Editing keys

        #[test]
        fn backspace() {
            let action = map_key_event(&Key::Named(Named::Backspace), no_mod(), Option::None);
            assert_eq!(action, KeyAction::Edit(EditAction::DeleteBackward));
        }

        #[test]
        fn backspace_with_shift() {
            let action = map_key_event(&Key::Named(Named::Backspace), shift(), Option::None);
            assert_eq!(action, KeyAction::Edit(EditAction::DeleteBackward));
        }

        #[test]
        fn delete() {
            let action = map_key_event(&Key::Named(Named::Delete), no_mod(), Option::None);
            assert_eq!(action, KeyAction::Edit(EditAction::DeleteForward));
        }

        #[test]
        fn enter() {
            let action = map_key_event(&Key::Named(Named::Enter), no_mod(), Option::None);
            assert_eq!(action, KeyAction::Edit(EditAction::SplitBlock));
        }

        // Format shortcuts

        #[test]
        fn ctrl_b_toggles_bold() {
            let action = map_key_event(&Key::Character("b".into()), ctrl(), Option::None);
            assert_eq!(
                action,
                KeyAction::Edit(EditAction::ToggleInlineStyle(InlineStyle::BOLD))
            );
        }

        #[test]
        fn ctrl_i_toggles_italic() {
            let action = map_key_event(&Key::Character("i".into()), ctrl(), Option::None);
            assert_eq!(
                action,
                KeyAction::Edit(EditAction::ToggleInlineStyle(InlineStyle::ITALIC))
            );
        }

        #[test]
        fn ctrl_u_toggles_underline() {
            let action = map_key_event(&Key::Character("u".into()), ctrl(), Option::None);
            assert_eq!(
                action,
                KeyAction::Edit(EditAction::ToggleInlineStyle(InlineStyle::UNDERLINE))
            );
        }

        // Clipboard

        #[test]
        fn ctrl_c_copies() {
            let action = map_key_event(&Key::Character("c".into()), ctrl(), Option::None);
            assert_eq!(action, KeyAction::Copy);
        }

        #[test]
        fn ctrl_x_cuts() {
            let action = map_key_event(&Key::Character("x".into()), ctrl(), Option::None);
            assert_eq!(action, KeyAction::Cut);
        }

        #[test]
        fn ctrl_v_pastes() {
            let action = map_key_event(&Key::Character("v".into()), ctrl(), Option::None);
            assert_eq!(action, KeyAction::Paste);
        }

        // Undo/Redo

        #[test]
        fn ctrl_z_undoes() {
            let action = map_key_event(&Key::Character("z".into()), ctrl(), Option::None);
            assert_eq!(action, KeyAction::Undo);
        }

        #[test]
        fn ctrl_shift_z_redoes() {
            let action = map_key_event(&Key::Character("z".into()), ctrl_shift(), Option::None);
            assert_eq!(action, KeyAction::Redo);
        }

        #[test]
        fn ctrl_y_redoes() {
            let action = map_key_event(&Key::Character("y".into()), ctrl(), Option::None);
            assert_eq!(action, KeyAction::Redo);
        }

        // Select all

        #[test]
        fn ctrl_a_selects_all() {
            let action = map_key_event(&Key::Character("a".into()), ctrl(), Option::None);
            assert_eq!(action, KeyAction::SelectAll);
        }

        // Text input

        #[test]
        fn character_inserts_text() {
            let action = map_key_event(&Key::Character("h".into()), no_mod(), Some("h"));
            assert_eq!(action, KeyAction::Edit(EditAction::InsertText("h".into())));
        }

        #[test]
        fn shift_character_inserts_text() {
            let action = map_key_event(&Key::Character("H".into()), shift(), Some("H"));
            assert_eq!(action, KeyAction::Edit(EditAction::InsertText("H".into())));
        }

        #[test]
        fn space_inserts_text() {
            let action = map_key_event(&Key::Character(" ".into()), no_mod(), Some(" "));
            assert_eq!(action, KeyAction::Edit(EditAction::InsertText(" ".into())));
        }

        #[test]
        fn unicode_character_inserts_text() {
            let action = map_key_event(
                &Key::Character("\u{00e9}".into()),
                no_mod(),
                Some("\u{00e9}"),
            );
            assert_eq!(
                action,
                KeyAction::Edit(EditAction::InsertText("\u{00e9}".into()))
            );
        }

        #[test]
        fn control_character_text_is_ignored() {
            // Tab produces a control character in `text`, but we don't handle it.
            let action = map_key_event(&Key::Named(Named::Tab), no_mod(), Some("\t"));
            assert_eq!(action, KeyAction::None);
        }

        #[test]
        fn unrecognized_key_returns_none() {
            let action = map_key_event(&Key::Named(Named::F1), no_mod(), Option::None);
            assert_eq!(action, KeyAction::None);
        }

        #[test]
        fn ctrl_with_unknown_char_returns_none() {
            let action = map_key_event(&Key::Character("q".into()), ctrl(), Option::None);
            assert_eq!(action, KeyAction::None);
        }
    }

    // ── Cursor movement tests ────────────────────────────

    mod cursor_movement {
        use super::*;

        fn single_block_doc(text: &str) -> Document {
            Document::from_blocks(vec![Block::paragraph(text)])
        }

        fn multi_block_doc() -> Document {
            Document::from_blocks(vec![
                Block::paragraph("hello"),
                Block::paragraph("world"),
                Block::paragraph("foo"),
            ])
        }

        // move_left

        #[test]
        fn move_left_within_block() {
            let doc = single_block_doc("hello");
            let result = move_left(&doc, DocPosition::new(0, 3));
            assert_eq!(result, DocPosition::new(0, 2));
        }

        #[test]
        fn move_left_at_block_start_to_previous() {
            let doc = multi_block_doc();
            let result = move_left(&doc, DocPosition::new(1, 0));
            assert_eq!(result, DocPosition::new(0, 5));
        }

        #[test]
        fn move_left_at_document_start() {
            let doc = single_block_doc("hello");
            let result = move_left(&doc, DocPosition::new(0, 0));
            assert_eq!(result, DocPosition::new(0, 0));
        }

        // move_right

        #[test]
        fn move_right_within_block() {
            let doc = single_block_doc("hello");
            let result = move_right(&doc, DocPosition::new(0, 2));
            assert_eq!(result, DocPosition::new(0, 3));
        }

        #[test]
        fn move_right_at_block_end_to_next() {
            let doc = multi_block_doc();
            let result = move_right(&doc, DocPosition::new(0, 5));
            assert_eq!(result, DocPosition::new(1, 0));
        }

        #[test]
        fn move_right_at_document_end() {
            let doc = single_block_doc("hello");
            let result = move_right(&doc, DocPosition::new(0, 5));
            assert_eq!(result, DocPosition::new(0, 5));
        }

        // word_left

        #[test]
        fn word_left_skips_word() {
            let doc = single_block_doc("hello world");
            let result = word_left(&doc, DocPosition::new(0, 11));
            assert_eq!(result, DocPosition::new(0, 6));
        }

        #[test]
        fn word_left_skips_whitespace_then_word() {
            let doc = single_block_doc("hello world");
            let result = word_left(&doc, DocPosition::new(0, 6));
            assert_eq!(result, DocPosition::new(0, 0));
        }

        #[test]
        fn word_left_from_mid_word() {
            let doc = single_block_doc("hello world");
            let result = word_left(&doc, DocPosition::new(0, 8));
            assert_eq!(result, DocPosition::new(0, 6));
        }

        #[test]
        fn word_left_with_punctuation() {
            let doc = single_block_doc("hello, world");
            // From 'w' at offset 7, skip to punctuation boundary.
            let result = word_left(&doc, DocPosition::new(0, 7));
            // Should skip space (whitespace), then ',' (other), landing at 5.
            assert_eq!(result, DocPosition::new(0, 5));
        }

        #[test]
        fn word_left_at_block_start_moves_to_previous() {
            let doc = multi_block_doc();
            let result = word_left(&doc, DocPosition::new(1, 0));
            assert_eq!(result, DocPosition::new(0, 5));
        }

        #[test]
        fn word_left_at_document_start() {
            let doc = single_block_doc("hello");
            let result = word_left(&doc, DocPosition::new(0, 0));
            assert_eq!(result, DocPosition::new(0, 0));
        }

        // word_right

        #[test]
        fn word_right_skips_word() {
            let doc = single_block_doc("hello world");
            let result = word_right(&doc, DocPosition::new(0, 0));
            assert_eq!(result, DocPosition::new(0, 6));
        }

        #[test]
        fn word_right_from_space() {
            let doc = single_block_doc("hello world");
            // At offset 5 (the space), class is Whitespace. Skip whitespace to 6,
            // then skip word "world" wouldn't happen because we only skip the
            // starting class. So we land at start of "world".
            let result = word_right(&doc, DocPosition::new(0, 5));
            assert_eq!(result, DocPosition::new(0, 6));
        }

        #[test]
        fn word_right_from_mid_word() {
            let doc = single_block_doc("hello world");
            let result = word_right(&doc, DocPosition::new(0, 2));
            assert_eq!(result, DocPosition::new(0, 6));
        }

        #[test]
        fn word_right_with_punctuation() {
            let doc = single_block_doc("hello, world");
            // From 'o' at offset 4, skip word chars "o" then ",".
            let result = word_right(&doc, DocPosition::new(0, 4));
            // "o" is word, skip to end at 5, then "," is other, skip space to "w" at 7.
            assert_eq!(result, DocPosition::new(0, 5));
        }

        #[test]
        fn word_right_at_block_end_moves_to_next() {
            let doc = multi_block_doc();
            let result = word_right(&doc, DocPosition::new(0, 5));
            assert_eq!(result, DocPosition::new(1, 0));
        }

        #[test]
        fn word_right_at_document_end() {
            let doc = single_block_doc("hello");
            let result = word_right(&doc, DocPosition::new(0, 5));
            assert_eq!(result, DocPosition::new(0, 5));
        }

        // home / end

        #[test]
        fn home_moves_to_block_start() {
            let result = home(DocPosition::new(2, 7));
            assert_eq!(result, DocPosition::new(2, 0));
        }

        #[test]
        fn home_at_start_is_noop() {
            let result = home(DocPosition::new(0, 0));
            assert_eq!(result, DocPosition::new(0, 0));
        }

        #[test]
        fn end_moves_to_block_end() {
            let doc = single_block_doc("hello");
            let result = end(&doc, DocPosition::new(0, 2));
            assert_eq!(result, DocPosition::new(0, 5));
        }

        #[test]
        fn end_at_end_is_noop() {
            let doc = single_block_doc("hello");
            let result = end(&doc, DocPosition::new(0, 5));
            assert_eq!(result, DocPosition::new(0, 5));
        }

        // document_start / document_end

        #[test]
        fn document_start_returns_zero() {
            let result = document_start();
            assert_eq!(result, DocPosition::zero());
        }

        #[test]
        fn document_end_returns_last_position() {
            let doc = multi_block_doc();
            let result = document_end(&doc);
            assert_eq!(result, DocPosition::new(2, 3)); // "foo" has 3 chars
        }

        #[test]
        fn document_end_empty_doc() {
            let doc = Document::new();
            let result = document_end(&doc);
            assert_eq!(result, DocPosition::new(0, 0));
        }

        // Edge cases

        #[test]
        fn move_left_right_round_trip_within_block() {
            let doc = single_block_doc("hello");
            let pos = DocPosition::new(0, 3);
            let left = move_left(&doc, pos);
            let back = move_right(&doc, left);
            assert_eq!(back, pos);
        }

        #[test]
        fn move_left_right_round_trip_across_blocks() {
            let doc = multi_block_doc();
            let pos = DocPosition::new(1, 0);
            let left = move_left(&doc, pos);
            assert_eq!(left, DocPosition::new(0, 5));
            let back = move_right(&doc, left);
            assert_eq!(back, pos);
        }

        #[test]
        fn word_movement_on_empty_block() {
            let doc = Document::from_blocks(vec![Block::empty_paragraph()]);
            let pos = DocPosition::new(0, 0);
            assert_eq!(word_left(&doc, pos), pos);
            assert_eq!(word_right(&doc, pos), pos);
        }

        #[test]
        fn word_left_multiple_spaces() {
            let doc = single_block_doc("hello   world");
            let result = word_left(&doc, DocPosition::new(0, 8));
            // From 'w' at 8, skip no whitespace before 'w', then skip word chars
            // Actually at offset 8 we're at 'w'. Chars: h(0)e(1)l(2)l(3)o(4) (5) (6) (7)w(8)
            // word_left: idx=8, chars[7]=' ' is whitespace, skip to idx=5,
            // chars[4]='o' is word, skip to idx=0
            assert_eq!(result, DocPosition::new(0, 0));
        }

        #[test]
        fn word_right_multiple_spaces() {
            let doc = single_block_doc("hello   world");
            let result = word_right(&doc, DocPosition::new(0, 0));
            // From 'h', skip word "hello" to 5, skip spaces "   " to 8.
            assert_eq!(result, DocPosition::new(0, 8));
        }

        #[test]
        fn word_movement_with_underscores() {
            let doc = single_block_doc("foo_bar baz");
            // "foo_bar" is all word chars (underscore counts as word).
            let result = word_right(&doc, DocPosition::new(0, 0));
            assert_eq!(result, DocPosition::new(0, 8));
        }
    }

    // ── word_at tests ────────────────────────────────────

    mod word_at_tests {
        use super::*;

        #[test]
        fn word_at_middle_of_word() {
            let doc = single_block_doc("hello world");
            let (start, end) = word_at(&doc, DocPosition::new(0, 3));
            assert_eq!(start, DocPosition::new(0, 0));
            assert_eq!(end, DocPosition::new(0, 5));
        }

        #[test]
        fn word_at_start_of_word() {
            let doc = single_block_doc("hello world");
            let (start, end) = word_at(&doc, DocPosition::new(0, 0));
            assert_eq!(start, DocPosition::new(0, 0));
            assert_eq!(end, DocPosition::new(0, 5));
        }

        #[test]
        fn word_at_end_of_word() {
            let doc = single_block_doc("hello world");
            // Offset 5 is on the space; left-affinity selects "hello"
            let (start, end) = word_at(&doc, DocPosition::new(0, 5));
            assert_eq!(start, DocPosition::new(0, 0));
            assert_eq!(end, DocPosition::new(0, 5));
        }

        #[test]
        fn word_at_second_word() {
            let doc = single_block_doc("hello world");
            let (start, end) = word_at(&doc, DocPosition::new(0, 8));
            assert_eq!(start, DocPosition::new(0, 6));
            assert_eq!(end, DocPosition::new(0, 11));
        }

        #[test]
        fn word_at_empty_block() {
            let doc = Document::from_blocks(vec![Block::empty_paragraph()]);
            let (start, end) = word_at(&doc, DocPosition::new(0, 0));
            assert_eq!(start, DocPosition::new(0, 0));
            assert_eq!(end, DocPosition::new(0, 0));
        }

        #[test]
        fn word_at_on_whitespace() {
            let doc = single_block_doc("hello   world");
            // Offset 6 is on a space (left-affinity: chars[5] = ' ')
            let (start, end) = word_at(&doc, DocPosition::new(0, 6));
            assert_eq!(start, DocPosition::new(0, 5));
            assert_eq!(end, DocPosition::new(0, 8));
        }

        #[test]
        fn word_at_punctuation() {
            let doc = single_block_doc("hello, world");
            // Offset 5 is comma; left-affinity: chars[4]='o' is word
            let (start, end) = word_at(&doc, DocPosition::new(0, 5));
            assert_eq!(start, DocPosition::new(0, 0));
            assert_eq!(end, DocPosition::new(0, 5));
        }
    }

    // ── select_block tests ───────────────────────────────

    mod select_block_tests {
        use super::*;

        #[test]
        fn select_block_selects_entire_block() {
            let doc = single_block_doc("hello world");
            let (start, end) = select_block(&doc, DocPosition::new(0, 3));
            assert_eq!(start, DocPosition::new(0, 0));
            assert_eq!(end, DocPosition::new(0, 11));
        }

        #[test]
        fn select_block_empty() {
            let doc = Document::from_blocks(vec![Block::empty_paragraph()]);
            let (start, end) = select_block(&doc, DocPosition::new(0, 0));
            assert_eq!(start, DocPosition::new(0, 0));
            assert_eq!(end, DocPosition::new(0, 0));
        }

        #[test]
        fn select_block_second_block() {
            let doc = multi_block_doc();
            let (start, end) = select_block(&doc, DocPosition::new(1, 2));
            assert_eq!(start, DocPosition::new(1, 0));
            assert_eq!(end, DocPosition::new(1, 5)); // "world" = 5 chars
        }
    }
}
