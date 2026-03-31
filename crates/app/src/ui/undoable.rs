//! Undo/redo infrastructure for plain text inputs and pill-based list inputs.
//!
//! `UndoableText` tracks state snapshots for short text fields (search bar,
//! subject line, contact notes, etc.). `UndoableList<T>` does the same for
//! ordered collections of items (To/Cc/Bcc recipients, label tags).
//!
//! Both use a simple snapshot approach: each undo entry stores the full
//! previous state. This is appropriate because the values are small (short
//! strings, small lists). The `dissimilar` crate is available for future
//! smart-grouping of consecutive single-character edits.

use std::collections::VecDeque;

/// Default maximum number of undo entries before the oldest is evicted.
const DEFAULT_MAX_ENTRIES: usize = 50;

// ── UndoableText ────────────────────────────────────────

/// Undo/redo state for a plain text input field.
///
/// Each call to [`set_text`](Self::set_text) snapshots the previous value
/// onto the undo stack and clears the redo stack. The stacks are capped
/// at `max_entries`; the oldest entry is dropped when the cap is exceeded.
#[derive(Debug, Clone)]
pub struct UndoableText {
    current: String,
    undo_stack: VecDeque<String>,
    redo_stack: Vec<String>,
    max_entries: usize,
}

impl Default for UndoableText {
    fn default() -> Self {
        Self::new()
    }
}

impl UndoableText {
    /// Create a new empty `UndoableText` with the default max entries (50).
    #[must_use]
    pub fn new() -> Self {
        Self {
            current: String::new(),
            undo_stack: VecDeque::new(),
            redo_stack: Vec::new(),
            max_entries: DEFAULT_MAX_ENTRIES,
        }
    }

    /// Create a new `UndoableText` with initial text and the default max entries.
    #[must_use]
    pub fn with_initial(text: &str) -> Self {
        Self {
            current: text.to_owned(),
            ..Self::new()
        }
    }

    /// Create a new empty `UndoableText` with a custom max entries cap.
    #[must_use]
    pub fn with_max_entries(max_entries: usize) -> Self {
        Self {
            max_entries,
            ..Self::new()
        }
    }

    /// Get the current text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.current
    }

    /// Record a text change. Pushes the previous text onto the undo stack,
    /// sets `current` to `new_text`, and clears the redo stack.
    ///
    /// If `new_text` is identical to the current text, this is a no-op.
    pub fn set_text(&mut self, new_text: String) {
        if new_text == self.current {
            return;
        }
        self.redo_stack.clear();
        self.undo_stack
            .push_back(std::mem::replace(&mut self.current, new_text));
        if self.undo_stack.len() > self.max_entries {
            self.undo_stack.pop_front();
        }
    }

    /// Undo the last change. Returns the new current text, or `None` if
    /// there is nothing to undo.
    pub fn undo(&mut self) -> Option<&str> {
        let previous = self.undo_stack.pop_back()?;
        self.redo_stack
            .push(std::mem::replace(&mut self.current, previous));
        Some(&self.current)
    }

    /// Redo a previously undone change. Returns the new current text, or
    /// `None` if there is nothing to redo.
    pub fn redo(&mut self) -> Option<&str> {
        let next = self.redo_stack.pop()?;
        self.undo_stack
            .push_back(std::mem::replace(&mut self.current, next));
        Some(&self.current)
    }

    /// Whether there is at least one entry on the undo stack.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Whether there is at least one entry on the redo stack.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Clear both undo and redo stacks, keeping the current text.
    pub fn clear_history(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// Reset to `new_text`, clearing all history. Use this for
    /// programmatic resets (e.g. clearing the search bar, loading a
    /// pinned search) where the previous undo stack is meaningless.
    pub fn reset(&mut self, new_text: String) {
        self.current = new_text;
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// Returns `true` if the current text is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.current.is_empty()
    }
}

// ── UndoableList ────────────────────────────────────────

/// Undo/redo state for an ordered list of items (e.g. recipients, tags).
///
/// Mutating operations ([`push`](Self::push), [`remove`](Self::remove))
/// snapshot the previous item list onto the undo stack and clear redo.
#[derive(Debug, Clone)]
pub struct UndoableList<T: Clone> {
    items: Vec<T>,
    undo_stack: VecDeque<Vec<T>>,
    redo_stack: Vec<Vec<T>>,
    max_entries: usize,
}

impl<T: Clone> Default for UndoableList<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone> UndoableList<T> {
    /// Create a new empty `UndoableList` with the default max entries (50).
    #[must_use]
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            undo_stack: VecDeque::new(),
            redo_stack: Vec::new(),
            max_entries: DEFAULT_MAX_ENTRIES,
        }
    }

    /// Get the current items.
    #[must_use]
    pub fn items(&self) -> &[T] {
        &self.items
    }

    /// Append an item. Snapshots the previous state to the undo stack.
    pub fn push(&mut self, item: T) {
        self.snapshot();
        self.items.push(item);
    }

    /// Remove the item at `index`. Returns the removed item, or `None` if
    /// the index is out of bounds. Snapshots the previous state to the
    /// undo stack on success.
    pub fn remove(&mut self, index: usize) -> Option<T> {
        if index >= self.items.len() {
            return None;
        }
        self.snapshot();
        Some(self.items.remove(index))
    }

    /// Undo the last change. Returns the new item slice, or `None` if
    /// there is nothing to undo.
    pub fn undo(&mut self) -> Option<&[T]> {
        let previous = self.undo_stack.pop_back()?;
        self.redo_stack
            .push(std::mem::replace(&mut self.items, previous));
        Some(&self.items)
    }

    /// Redo a previously undone change. Returns the new item slice, or
    /// `None` if there is nothing to redo.
    pub fn redo(&mut self) -> Option<&[T]> {
        let next = self.redo_stack.pop()?;
        self.undo_stack
            .push_back(std::mem::replace(&mut self.items, next));
        Some(&self.items)
    }

    /// Whether there is at least one entry on the undo stack.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Whether there is at least one entry on the redo stack.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Push the current items onto the undo stack and clear redo.
    fn snapshot(&mut self) {
        self.redo_stack.clear();
        self.undo_stack.push_back(self.items.clone());
        if self.undo_stack.len() > self.max_entries {
            self.undo_stack.pop_front();
        }
    }
}

// ── Tests ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── UndoableText ────────────────────────────────────

    #[test]
    fn text_initial_state() {
        let ut = UndoableText::new();
        assert_eq!(ut.text(), "");
        assert!(!ut.can_undo());
        assert!(!ut.can_redo());
    }

    #[test]
    fn text_set_and_undo() {
        let mut ut = UndoableText::new();
        ut.set_text("hello".into());
        assert_eq!(ut.text(), "hello");
        assert!(ut.can_undo());

        let result = ut.undo();
        assert_eq!(result, Some(""));
        assert_eq!(ut.text(), "");
        assert!(!ut.can_undo());
    }

    #[test]
    fn text_undo_then_redo() {
        let mut ut = UndoableText::new();
        ut.set_text("a".into());
        ut.set_text("ab".into());

        let _ = ut.undo();
        assert_eq!(ut.text(), "a");
        assert!(ut.can_redo());

        let result = ut.redo();
        assert_eq!(result, Some("ab"));
        assert_eq!(ut.text(), "ab");
        assert!(!ut.can_redo());
    }

    #[test]
    fn text_new_edit_clears_redo() {
        let mut ut = UndoableText::new();
        ut.set_text("a".into());
        ut.set_text("ab".into());
        let _ = ut.undo(); // back to "a"
        assert!(ut.can_redo());

        ut.set_text("ac".into()); // new edit from "a"
        assert!(!ut.can_redo());
        assert_eq!(ut.text(), "ac");
    }

    #[test]
    fn text_undo_empty_returns_none() {
        let mut ut = UndoableText::new();
        assert!(ut.undo().is_none());
    }

    #[test]
    fn text_redo_empty_returns_none() {
        let mut ut = UndoableText::new();
        assert!(ut.redo().is_none());
    }

    #[test]
    fn text_set_same_value_is_noop() {
        let mut ut = UndoableText::new();
        ut.set_text("x".into());
        ut.set_text("x".into()); // same value
        // Only one undo entry should exist.
        assert!(ut.can_undo());
        let _ = ut.undo();
        assert!(!ut.can_undo());
    }

    #[test]
    fn text_max_entries_eviction() {
        let mut ut = UndoableText::with_max_entries(3);
        ut.set_text("a".into());
        ut.set_text("b".into());
        ut.set_text("c".into());
        ut.set_text("d".into());
        // Stack should have at most 3 entries: "a", "b", "c"
        // but the oldest ("") was evicted, so we have "a", "b", "c".
        // Undoing 3 times should work, 4th should fail.
        assert!(ut.undo().is_some()); // d -> c
        assert!(ut.undo().is_some()); // c -> b
        assert!(ut.undo().is_some()); // b -> a
        assert!(ut.undo().is_none()); // no more
        assert_eq!(ut.text(), "a");
    }

    #[test]
    fn text_multiple_sequential_undos() {
        let mut ut = UndoableText::new();
        ut.set_text("one".into());
        ut.set_text("two".into());
        ut.set_text("three".into());

        assert_eq!(ut.undo(), Some("two"));
        assert_eq!(ut.undo(), Some("one"));
        assert_eq!(ut.undo(), Some(""));
        assert_eq!(ut.undo(), None);
    }

    #[test]
    fn text_clear_history() {
        let mut ut = UndoableText::new();
        ut.set_text("a".into());
        ut.set_text("b".into());
        let _ = ut.undo();

        ut.clear_history();
        assert!(!ut.can_undo());
        assert!(!ut.can_redo());
        assert_eq!(ut.text(), "a"); // current text preserved
    }

    #[test]
    fn text_interleaved_undo_redo() {
        let mut ut = UndoableText::new();
        ut.set_text("a".into());
        ut.set_text("b".into());
        ut.set_text("c".into());

        assert_eq!(ut.undo(), Some("b"));
        assert_eq!(ut.redo(), Some("c"));
        assert_eq!(ut.undo(), Some("b"));
        assert_eq!(ut.undo(), Some("a"));
        assert_eq!(ut.redo(), Some("b"));
    }

    // ── UndoableList ────────────────────────────────────

    #[test]
    fn list_initial_state() {
        let ul: UndoableList<String> = UndoableList::new();
        assert!(ul.items().is_empty());
        assert!(!ul.can_undo());
        assert!(!ul.can_redo());
    }

    #[test]
    fn list_push_and_undo() {
        let mut ul = UndoableList::new();
        ul.push("alice".to_string());
        ul.push("bob".to_string());
        assert_eq!(ul.items().len(), 2);

        let result = ul.undo();
        assert_eq!(result.map(<[String]>::len), Some(1));
        assert_eq!(ul.items(), &["alice".to_string()]);
    }

    #[test]
    fn list_remove_and_undo() {
        let mut ul = UndoableList::new();
        ul.push("a".to_string());
        ul.push("b".to_string());
        ul.push("c".to_string());

        let removed = ul.remove(1);
        assert_eq!(removed.as_deref(), Some("b"));
        assert_eq!(ul.items().len(), 2);

        let result = ul.undo();
        assert_eq!(result.map(<[String]>::len), Some(3));
        assert_eq!(ul.items()[1], "b");
    }

    #[test]
    fn list_remove_out_of_bounds() {
        let mut ul: UndoableList<i32> = UndoableList::new();
        ul.push(1);
        assert!(ul.remove(5).is_none());
        // Should not have created an undo entry for the failed remove.
        // We have one entry from push, none from the failed remove.
        let _ = ul.undo(); // undo the push
        assert!(!ul.can_undo());
    }

    #[test]
    fn list_undo_then_redo() {
        let mut ul = UndoableList::new();
        ul.push(10);
        ul.push(20);

        let _ = ul.undo(); // back to [10]
        assert!(ul.can_redo());

        let result = ul.redo();
        assert_eq!(result, Some([10, 20].as_slice()));
        assert!(!ul.can_redo());
    }

    #[test]
    fn list_new_edit_clears_redo() {
        let mut ul = UndoableList::new();
        ul.push(1);
        ul.push(2);
        let _ = ul.undo(); // back to [1]
        assert!(ul.can_redo());

        ul.push(3); // new edit from [1]
        assert!(!ul.can_redo());
        assert_eq!(ul.items(), &[1, 3]);
    }

    #[test]
    fn list_undo_empty_returns_none() {
        let mut ul: UndoableList<i32> = UndoableList::new();
        assert!(ul.undo().is_none());
    }

    #[test]
    fn list_redo_empty_returns_none() {
        let mut ul: UndoableList<i32> = UndoableList::new();
        assert!(ul.redo().is_none());
    }

    #[test]
    fn list_multiple_undo_redo_cycle() {
        let mut ul = UndoableList::new();
        ul.push("a".to_string());
        ul.push("b".to_string());
        ul.push("c".to_string());

        // Undo all three pushes.
        assert!(ul.undo().is_some()); // [a, b]
        assert!(ul.undo().is_some()); // [a]
        assert!(ul.undo().is_some()); // []
        assert!(ul.undo().is_none());
        assert!(ul.items().is_empty());

        // Redo all three.
        assert!(ul.redo().is_some()); // [a]
        assert!(ul.redo().is_some()); // [a, b]
        assert!(ul.redo().is_some()); // [a, b, c]
        assert!(ul.redo().is_none());
        assert_eq!(ul.items().len(), 3);
    }
}
