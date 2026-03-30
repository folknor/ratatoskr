//! Generic undo stack.
//!
//! The palette crate owns the stack structure. The payload type is
//! defined by the consumer (e.g., the app crate defines `MailUndoPayload`).
//! Each entry carries a description string and one or more payloads
//! (C4: one user action = one undo step, even for mixed-direction toggles).

use std::collections::VecDeque;

/// A single undo entry: description + compensation payloads.
///
/// `payloads` is a `Vec<T>` because one user action (e.g., toggle star
/// on 5 threads with mixed prior state) may produce multiple compensation
/// items that should all execute on a single Ctrl+Z.
#[derive(Debug, Clone)]
pub struct UndoEntry<T> {
    /// Human-readable description (e.g., "Archived", "Star toggled").
    /// Set at push time (C3), not derived from the payload.
    pub description: String,
    /// Compensation payloads to execute on undo.
    pub payloads: Vec<T>,
}

/// Bounded FIFO stack of undo entries.
///
/// When the stack is full, the oldest entry is evicted.
pub struct UndoStack<T> {
    entries: VecDeque<UndoEntry<T>>,
    capacity: usize,
}

impl<T> UndoStack<T> {
    /// Create a new undo stack with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Push an entry onto the stack, evicting the oldest if at capacity.
    pub fn push(&mut self, description: String, payloads: Vec<T>) {
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(UndoEntry {
            description,
            payloads,
        });
    }

    /// Pop the most recent entry (for undo).
    pub fn pop(&mut self) -> Option<UndoEntry<T>> {
        self.entries.pop_back()
    }

    /// Whether there are any entries to undo.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of entries in the stack.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Clear all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Peek at the most recent entry without removing it.
    pub fn peek(&self) -> Option<&UndoEntry<T>> {
        self.entries.back()
    }
}

impl<T> Default for UndoStack<T> {
    fn default() -> Self {
        Self::new(20)
    }
}
