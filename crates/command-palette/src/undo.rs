//! Undo token types and undo stack.
//!
//! The undo system allows reversible email actions (archive, trash, move,
//! star, pin, mute, mark-read, add/remove label) to be undone via Ctrl+Z.
//!
//! **Ownership boundary:** The `UndoToken` type and `UndoStack` live in
//! the command-palette crate (shared types). The app layer:
//! 1. Receives tokens from command execution
//! 2. Pushes them onto the stack
//! 3. Pops and executes compensation when Undo is invoked

use std::collections::VecDeque;

/// A token capturing the state needed to reverse a single action.
///
/// Each variant carries enough data to construct the compensation
/// action (e.g., move back to the original folder, re-apply the
/// removed label). The app layer interprets these and calls the
/// appropriate provider APIs.
#[derive(Debug, Clone)]
pub enum UndoToken {
    /// Undo an archive: move thread(s) back to inbox.
    Archive {
        account_id: String,
        thread_ids: Vec<String>,
    },
    /// Undo a trash: move thread(s) back to the original folder.
    Trash {
        account_id: String,
        thread_ids: Vec<String>,
        original_folder_id: Option<String>,
    },
    /// Undo a move: move thread(s) back to the source folder.
    MoveToFolder {
        account_id: String,
        thread_ids: Vec<String>,
        source_folder_id: String,
    },
    /// Undo a read/unread toggle.
    ToggleRead {
        account_id: String,
        thread_ids: Vec<String>,
        /// The read state before the action (true = was read).
        was_read: bool,
    },
    /// Undo a star/unstar toggle.
    ToggleStar {
        account_id: String,
        thread_ids: Vec<String>,
        was_starred: bool,
    },
    /// Undo a pin/unpin toggle.
    TogglePin {
        account_id: String,
        thread_ids: Vec<String>,
        was_pinned: bool,
    },
    /// Undo a mute/unmute toggle.
    ToggleMute {
        account_id: String,
        thread_ids: Vec<String>,
        was_muted: bool,
    },
    /// Undo a spam toggle: move back from spam.
    ToggleSpam {
        account_id: String,
        thread_ids: Vec<String>,
        was_spam: bool,
    },
    /// Undo add label: remove the label that was added.
    AddLabel {
        account_id: String,
        thread_ids: Vec<String>,
        label_id: String,
    },
    /// Undo remove label: re-add the label that was removed.
    RemoveLabel {
        account_id: String,
        thread_ids: Vec<String>,
        label_id: String,
    },
    /// Undo a snooze: unsnooze the thread.
    Snooze {
        account_id: String,
        thread_ids: Vec<String>,
    },
}

impl UndoToken {
    /// Human-readable description for the status bar confirmation.
    pub fn description(&self) -> String {
        match self {
            Self::Archive { thread_ids, .. } => {
                format!("Archived {} thread(s)", thread_ids.len())
            }
            Self::Trash { thread_ids, .. } => {
                format!("Trashed {} thread(s)", thread_ids.len())
            }
            Self::MoveToFolder { thread_ids, .. } => {
                format!("Moved {} thread(s)", thread_ids.len())
            }
            Self::ToggleRead { was_read, .. } => {
                if *was_read {
                    "Marked as unread".to_string()
                } else {
                    "Marked as read".to_string()
                }
            }
            Self::ToggleStar { was_starred, .. } => {
                if *was_starred {
                    "Unstarred".to_string()
                } else {
                    "Starred".to_string()
                }
            }
            Self::TogglePin { was_pinned, .. } => {
                if *was_pinned {
                    "Unpinned".to_string()
                } else {
                    "Pinned".to_string()
                }
            }
            Self::ToggleMute { was_muted, .. } => {
                if *was_muted {
                    "Unmuted".to_string()
                } else {
                    "Muted".to_string()
                }
            }
            Self::ToggleSpam { was_spam, .. } => {
                if *was_spam {
                    "Removed from spam".to_string()
                } else {
                    "Marked as spam".to_string()
                }
            }
            Self::AddLabel { .. } => "Added label".to_string(),
            Self::RemoveLabel { .. } => "Removed label".to_string(),
            Self::Snooze { .. } => "Snoozed".to_string(),
        }
    }
}

/// Bounded FIFO stack of undo tokens.
///
/// When the stack is full, the oldest token is evicted.
pub struct UndoStack {
    tokens: VecDeque<UndoToken>,
    capacity: usize,
}

impl UndoStack {
    /// Create a new undo stack with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            tokens: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Push a token onto the stack, evicting the oldest if at capacity.
    pub fn push(&mut self, token: UndoToken) {
        if self.tokens.len() >= self.capacity {
            self.tokens.pop_front();
        }
        self.tokens.push_back(token);
    }

    /// Pop the most recent token (for undo).
    pub fn pop(&mut self) -> Option<UndoToken> {
        self.tokens.pop_back()
    }

    /// Whether there are any tokens to undo.
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// Number of tokens in the stack.
    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    /// Clear all tokens.
    pub fn clear(&mut self) {
        self.tokens.clear();
    }

    /// Peek at the most recent token without removing it.
    pub fn peek(&self) -> Option<&UndoToken> {
        self.tokens.back()
    }
}

impl Default for UndoStack {
    fn default() -> Self {
        Self::new(20)
    }
}
