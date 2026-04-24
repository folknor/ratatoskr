//! Undo/redo stack with `UndoGroup` batching and cursor bookmark mapping.
//!
//! Consecutive character insertions group into one undo entry (split on pause,
//! format change, or cursor jump).

use crate::document::DocSelection;
use crate::operations::{EditOp, PosMap};

/// A group of operations forming one undoable user action.
#[derive(Debug, Clone)]
pub struct UndoGroup {
    /// The operations in this group (in application order).
    pub ops: Vec<EditOp>,
    /// The cursor/selection state before this group was applied.
    pub cursor_before: DocSelection,
    /// The cursor/selection state after this group was applied.
    pub cursor_after: DocSelection,
}

/// The undo/redo stack.
#[derive(Debug, Clone)]
pub struct UndoStack {
    undo: Vec<UndoGroup>,
    redo: Vec<UndoGroup>,
    max_entries: usize,
    /// When `true`, the next `push` must start a new group even if it would
    /// otherwise merge with the previous one.
    force_new_group: bool,
}

impl UndoStack {
    /// Create a new undo stack with the given max entry count.
    pub fn new(max_entries: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            max_entries,
            force_new_group: false,
        }
    }

    /// Push a batch of operations. Decides whether to merge with the last
    /// group or start a new one, based on operation type and adjacency.
    /// Clears the redo stack.
    pub fn push(
        &mut self,
        ops: Vec<EditOp>,
        cursor_before: DocSelection,
        cursor_after: DocSelection,
    ) {
        if ops.is_empty() {
            return;
        }

        // Any new edit invalidates the redo history.
        self.redo.clear();

        // Try to merge with the last group if allowed.
        if !self.force_new_group
            && let Some(last) = self.undo.last_mut()
            && can_merge(last, &ops, &cursor_before)
        {
            last.ops.extend(ops);
            last.cursor_after = cursor_after;
            return;
        }

        self.force_new_group = false;

        self.undo.push(UndoGroup {
            ops,
            cursor_before,
            cursor_after,
        });

        // Evict oldest if we exceed max_entries.
        if self.undo.len() > self.max_entries {
            let excess = self.undo.len() - self.max_entries;
            self.undo.drain(..excess);
        }
    }

    /// Pop the most recent undo group. The caller is responsible for applying
    /// inverse operations and restoring `cursor_before`. The group is moved
    /// to the redo stack.
    pub fn undo(&mut self) -> Option<UndoGroup> {
        let group = self.undo.pop()?;
        self.redo.push(group.clone());
        Some(group)
    }

    /// Pop the most recent redo group. The caller is responsible for
    /// re-applying the operations and restoring `cursor_after`. The group
    /// is moved back to the undo stack.
    pub fn redo(&mut self) -> Option<UndoGroup> {
        let group = self.redo.pop()?;
        self.undo.push(group.clone());
        Some(group)
    }

    /// Force the next `push` to start a new group even if it would otherwise
    /// merge with the previous one.
    pub fn break_group(&mut self) {
        self.force_new_group = true;
    }

    /// Map all stored cursor bookmarks through a `PosMap`.
    ///
    /// This should be called by the editor after each operation so that cursor
    /// positions in earlier undo groups remain valid. Currently a no-op because
    /// `PosMap::map` is still a stub, but the infrastructure is in place.
    pub fn map_cursors(&mut self, pos_map: &PosMap) {
        for group in &mut self.undo {
            map_selection(&mut group.cursor_before, pos_map);
            map_selection(&mut group.cursor_after, pos_map);
        }
        for group in &mut self.redo {
            map_selection(&mut group.cursor_before, pos_map);
            map_selection(&mut group.cursor_after, pos_map);
        }
    }

    /// Whether the undo stack is empty.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether the redo stack is empty.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Number of groups on the undo stack.
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    /// Number of groups on the redo stack.
    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    /// Clear both stacks entirely.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.force_new_group = false;
    }
}

impl Default for UndoStack {
    fn default() -> Self {
        Self::new(100)
    }
}

// ── Merging logic ───────────────────────────────────────

/// Determine whether `new_ops` should merge into `last_group` rather than
/// starting a new undo group.
///
/// Merging happens when:
/// 1. Both the last group and the new ops consist entirely of `InsertText`.
/// 2. The new insertion position is adjacent to where the last group's
///    insertions ended (same block, offset = previous end).
fn can_merge(last_group: &UndoGroup, new_ops: &[EditOp], cursor_before: &DocSelection) -> bool {
    // Both sides must be all InsertText.
    if !all_insert_text(&last_group.ops) || !all_insert_text(new_ops) {
        return false;
    }

    // The cursor_before of the new ops should match the cursor_after of the
    // last group - this confirms the user hasn't jumped the cursor.
    if *cursor_before != last_group.cursor_after {
        return false;
    }

    // Check adjacency: the first new insertion should start where the last
    // group's final insertion ended.
    let Some(last_end) = last_insert_end(&last_group.ops) else {
        return false;
    };
    let Some(new_start) = first_insert_position(new_ops) else {
        return false;
    };

    last_end.block_index == new_start.block_index && last_end.offset == new_start.offset
}

/// Check whether every op in the slice is `InsertText`.
fn all_insert_text(ops: &[EditOp]) -> bool {
    ops.iter().all(|op| matches!(op, EditOp::InsertText { .. }))
}

/// Get the position just after the last `InsertText` in a slice of ops.
fn last_insert_end(ops: &[EditOp]) -> Option<crate::document::DocPosition> {
    for op in ops.iter().rev() {
        if let EditOp::InsertText { position, text } = op {
            return Some(crate::document::DocPosition::new(
                position.block_index,
                position.offset + text.chars().count(),
            ));
        }
    }
    None
}

/// Get the position of the first `InsertText` in a slice of ops.
fn first_insert_position(ops: &[EditOp]) -> Option<crate::document::DocPosition> {
    for op in ops {
        if let EditOp::InsertText { position, .. } = op {
            return Some(*position);
        }
    }
    None
}

/// Map a `DocSelection` through a `PosMap`. Delegates to `PosMap::map` on
/// each position (currently a stub/identity).
fn map_selection(sel: &mut DocSelection, pos_map: &PosMap) {
    sel.anchor = pos_map.map(sel.anchor);
    sel.focus = pos_map.map(sel.focus);
}

// ── Tests ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{DocPosition, DocSelection};

    fn caret(block: usize, offset: usize) -> DocSelection {
        DocSelection::caret(DocPosition::new(block, offset))
    }

    fn insert_op(block: usize, offset: usize, text: &str) -> EditOp {
        EditOp::InsertText {
            position: DocPosition::new(block, offset),
            text: text.to_owned(),
        }
    }

    fn delete_op(
        start_block: usize,
        start_offset: usize,
        end_block: usize,
        end_offset: usize,
    ) -> EditOp {
        EditOp::DeleteRange {
            start: DocPosition::new(start_block, start_offset),
            end: DocPosition::new(end_block, end_offset),
            deleted: crate::operations::DeletedContent {
                blocks: vec![crate::document::Block::paragraph("x")],
            },
        }
    }

    // ── Basic push / undo / redo ────────────────────────

    #[test]
    fn push_and_undo() {
        let mut stack = UndoStack::default();
        stack.push(vec![insert_op(0, 0, "a")], caret(0, 0), caret(0, 1));

        assert!(stack.can_undo());
        assert!(!stack.can_redo());

        let group = stack.undo().expect("should have undo group");
        assert_eq!(group.ops.len(), 1);
        assert_eq!(group.cursor_before, caret(0, 0));
        assert_eq!(group.cursor_after, caret(0, 1));

        assert!(!stack.can_undo());
        assert!(stack.can_redo());
    }

    #[test]
    fn undo_then_redo() {
        let mut stack = UndoStack::default();
        stack.push(vec![insert_op(0, 0, "a")], caret(0, 0), caret(0, 1));

        let undone = stack.undo().expect("undo");
        assert_eq!(undone.cursor_after, caret(0, 1));

        let redone = stack.redo().expect("redo");
        assert_eq!(redone.cursor_after, caret(0, 1));

        // After redo, the group is back on undo stack.
        assert!(stack.can_undo());
        assert!(!stack.can_redo());
    }

    #[test]
    fn undo_empty_stack_returns_none() {
        let mut stack = UndoStack::default();
        assert!(stack.undo().is_none());
    }

    #[test]
    fn redo_empty_stack_returns_none() {
        let mut stack = UndoStack::default();
        assert!(stack.redo().is_none());
    }

    // ── Redo cleared on new push ────────────────────────

    #[test]
    fn push_clears_redo() {
        let mut stack = UndoStack::default();
        stack.push(vec![insert_op(0, 0, "a")], caret(0, 0), caret(0, 1));
        let _ = stack.undo();
        assert!(stack.can_redo());

        // New push should clear redo.
        stack.push(vec![insert_op(0, 0, "b")], caret(0, 0), caret(0, 1));
        assert!(!stack.can_redo());
    }

    // ── Grouping: consecutive InsertText merges ─────────

    #[test]
    fn consecutive_inserts_merge() {
        let mut stack = UndoStack::default();
        stack.push(vec![insert_op(0, 0, "a")], caret(0, 0), caret(0, 1));
        stack.push(vec![insert_op(0, 1, "b")], caret(0, 1), caret(0, 2));
        stack.push(vec![insert_op(0, 2, "c")], caret(0, 2), caret(0, 3));

        // All three should have merged into a single group.
        assert_eq!(stack.undo_len(), 1);

        let group = stack.undo().expect("undo");
        assert_eq!(group.ops.len(), 3);
        assert_eq!(group.cursor_before, caret(0, 0));
        assert_eq!(group.cursor_after, caret(0, 3));
    }

    #[test]
    fn non_adjacent_inserts_do_not_merge() {
        let mut stack = UndoStack::default();
        stack.push(vec![insert_op(0, 0, "a")], caret(0, 0), caret(0, 1));
        // Jump: cursor moved to a different position.
        stack.push(vec![insert_op(0, 5, "b")], caret(0, 5), caret(0, 6));

        assert_eq!(stack.undo_len(), 2);
    }

    #[test]
    fn insert_then_delete_do_not_merge() {
        let mut stack = UndoStack::default();
        stack.push(vec![insert_op(0, 0, "a")], caret(0, 0), caret(0, 1));
        stack.push(vec![delete_op(0, 0, 0, 1)], caret(0, 1), caret(0, 0));

        assert_eq!(stack.undo_len(), 2);
    }

    #[test]
    fn inserts_across_blocks_do_not_merge() {
        let mut stack = UndoStack::default();
        stack.push(vec![insert_op(0, 0, "a")], caret(0, 0), caret(0, 1));
        stack.push(vec![insert_op(1, 0, "b")], caret(1, 0), caret(1, 1));

        assert_eq!(stack.undo_len(), 2);
    }

    // ── Group break ─────────────────────────────────────

    #[test]
    fn break_group_prevents_merge() {
        let mut stack = UndoStack::default();
        stack.push(vec![insert_op(0, 0, "a")], caret(0, 0), caret(0, 1));
        stack.break_group();
        stack.push(vec![insert_op(0, 1, "b")], caret(0, 1), caret(0, 2));

        // Should be two separate groups despite adjacency.
        assert_eq!(stack.undo_len(), 2);
    }

    #[test]
    fn break_group_resets_after_one_push() {
        let mut stack = UndoStack::default();
        stack.push(vec![insert_op(0, 0, "a")], caret(0, 0), caret(0, 1));
        stack.break_group();
        stack.push(vec![insert_op(0, 1, "b")], caret(0, 1), caret(0, 2));
        // This third push should merge with the second (break was consumed).
        stack.push(vec![insert_op(0, 2, "c")], caret(0, 2), caret(0, 3));

        assert_eq!(stack.undo_len(), 2);
    }

    // ── Max entries eviction ────────────────────────────

    #[test]
    fn max_entries_evicts_oldest() {
        let mut stack = UndoStack::new(3);

        for i in 0..5 {
            // Use break_group to ensure each is its own group.
            stack.break_group();
            stack.push(vec![insert_op(0, i, "x")], caret(0, i), caret(0, i + 1));
        }

        assert_eq!(stack.undo_len(), 3);

        // The oldest two should have been evicted. The remaining groups
        // should be the last three pushes (i=2,3,4).
        let g = stack.undo().expect("undo");
        assert_eq!(g.cursor_before, caret(0, 4));
    }

    // ── Empty ops push is a no-op ───────────────────────

    #[test]
    fn push_empty_ops_is_noop() {
        let mut stack = UndoStack::default();
        stack.push(vec![], caret(0, 0), caret(0, 0));
        assert!(!stack.can_undo());
    }

    // ── Multiple undo/redo cycles ───────────────────────

    #[test]
    fn multiple_undo_redo_cycles() {
        let mut stack = UndoStack::default();

        stack.push(vec![insert_op(0, 0, "a")], caret(0, 0), caret(0, 1));
        stack.break_group();
        stack.push(vec![insert_op(0, 1, "b")], caret(0, 1), caret(0, 2));
        stack.break_group();
        stack.push(vec![insert_op(0, 2, "c")], caret(0, 2), caret(0, 3));

        assert_eq!(stack.undo_len(), 3);
        assert_eq!(stack.redo_len(), 0);

        // Undo all three.
        let g3 = stack.undo().expect("undo 3");
        assert_eq!(g3.cursor_after, caret(0, 3));
        let g2 = stack.undo().expect("undo 2");
        assert_eq!(g2.cursor_after, caret(0, 2));
        let g1 = stack.undo().expect("undo 1");
        assert_eq!(g1.cursor_after, caret(0, 1));

        assert_eq!(stack.undo_len(), 0);
        assert_eq!(stack.redo_len(), 3);

        // Redo all three.
        let r1 = stack.redo().expect("redo 1");
        assert_eq!(r1.cursor_after, caret(0, 1));
        let r2 = stack.redo().expect("redo 2");
        assert_eq!(r2.cursor_after, caret(0, 2));
        let r3 = stack.redo().expect("redo 3");
        assert_eq!(r3.cursor_after, caret(0, 3));

        assert_eq!(stack.undo_len(), 3);
        assert_eq!(stack.redo_len(), 0);
    }

    // ── Undo after partial redo, then new edit ──────────

    #[test]
    fn undo_partial_redo_then_new_push_clears_redo() {
        let mut stack = UndoStack::default();

        stack.push(vec![insert_op(0, 0, "a")], caret(0, 0), caret(0, 1));
        stack.break_group();
        stack.push(vec![insert_op(0, 1, "b")], caret(0, 1), caret(0, 2));
        stack.break_group();
        stack.push(vec![insert_op(0, 2, "c")], caret(0, 2), caret(0, 3));

        // Undo the last two.
        let _ = stack.undo();
        let _ = stack.undo();
        assert_eq!(stack.undo_len(), 1);
        assert_eq!(stack.redo_len(), 2);

        // Redo one.
        let _ = stack.redo();
        assert_eq!(stack.undo_len(), 2);
        assert_eq!(stack.redo_len(), 1);

        // New push should clear remaining redo.
        stack.push(vec![insert_op(0, 2, "d")], caret(0, 2), caret(0, 3));
        assert_eq!(stack.redo_len(), 0);
    }

    // ── Format ops never merge ──────────────────────────

    #[test]
    fn format_ops_never_merge_with_inserts() {
        let mut stack = UndoStack::default();
        stack.push(vec![insert_op(0, 0, "a")], caret(0, 0), caret(0, 1));
        stack.push(
            vec![EditOp::ToggleInlineStyle {
                start: DocPosition::new(0, 0),
                end: DocPosition::new(0, 1),
                style_bit: crate::document::InlineStyle::BOLD,
            }],
            caret(0, 1),
            caret(0, 1),
        );

        assert_eq!(stack.undo_len(), 2);
    }

    #[test]
    fn set_block_type_does_not_merge() {
        let mut stack = UndoStack::default();
        stack.push(vec![insert_op(0, 0, "a")], caret(0, 0), caret(0, 1));
        stack.push(
            vec![EditOp::SetBlockType {
                block_index: 0,
                old: crate::document::BlockKind::Paragraph,
                new: crate::document::BlockKind::Heading(crate::document::HeadingLevel::H1),
            }],
            caret(0, 1),
            caret(0, 1),
        );

        assert_eq!(stack.undo_len(), 2);
    }

    // ── Clear ───────────────────────────────────────────

    #[test]
    fn clear_empties_both_stacks() {
        let mut stack = UndoStack::default();
        stack.push(vec![insert_op(0, 0, "a")], caret(0, 0), caret(0, 1));
        let _ = stack.undo();
        assert!(stack.can_redo());

        stack.clear();
        assert!(!stack.can_undo());
        assert!(!stack.can_redo());
    }

    // ── map_cursors infrastructure ──────────────────────

    #[test]
    fn map_cursors_shifts_positions_through_insert() {
        use crate::operations::PosMapEntry;

        let mut stack = UndoStack::default();
        stack.push(vec![insert_op(0, 0, "a")], caret(0, 8), caret(0, 10));

        // Simulate an insert of 3 chars at offset 2 in block 0.
        let pos_map = PosMap {
            block_index: 0,
            entries: vec![PosMapEntry {
                old_offset: 2,
                old_len: 0,
                new_len: 3,
            }],
            structural: None,
        };

        stack.map_cursors(&pos_map);

        let group = stack.undo().expect("should have undo group");
        // Positions after the insert point (offset 2) should shift by +3.
        assert_eq!(group.cursor_before, caret(0, 11));
        assert_eq!(group.cursor_after, caret(0, 13));
    }

    // ── Multi-char insert text merging ──────────────────

    #[test]
    fn multi_char_insert_merges_correctly() {
        let mut stack = UndoStack::default();
        stack.push(vec![insert_op(0, 0, "hello")], caret(0, 0), caret(0, 5));
        stack.push(vec![insert_op(0, 5, " world")], caret(0, 5), caret(0, 11));

        assert_eq!(stack.undo_len(), 1);
        let group = stack.undo().expect("undo");
        assert_eq!(group.ops.len(), 2);
        assert_eq!(group.cursor_before, caret(0, 0));
        assert_eq!(group.cursor_after, caret(0, 11));
    }

    // ── Selection (non-collapsed) cursor_before prevents merge ──

    #[test]
    fn selection_cursor_before_prevents_merge() {
        let mut stack = UndoStack::default();
        stack.push(vec![insert_op(0, 0, "a")], caret(0, 0), caret(0, 1));

        // The new push has a non-collapsed selection as cursor_before, which
        // doesn't match the previous cursor_after (collapsed caret).
        let sel = DocSelection::range(DocPosition::new(0, 0), DocPosition::new(0, 1));
        stack.push(vec![insert_op(0, 1, "b")], sel, caret(0, 2));

        assert_eq!(stack.undo_len(), 2);
    }
}
