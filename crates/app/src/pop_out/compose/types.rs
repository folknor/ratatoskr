use std::sync::Arc;

use iced::Point;

use crate::db::ContactMatch;
use crate::ui::token_input::TokenId;

/// Account info for the From dropdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountInfo {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub account_name: Option<String>,
}

impl std::fmt::Display for AccountInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(ref name) = self.display_name {
            write!(f, "{name} <{}>", self.email)
        } else {
            write!(f, "{}", self.email)
        }
    }
}

/// An attachment queued for sending.
#[derive(Debug, Clone)]
pub struct ComposeAttachment {
    /// Original file name.
    pub name: String,
    /// MIME type (guessed from extension).
    pub mime_type: String,
    /// File contents.
    pub data: Arc<Vec<u8>>,
}

impl ComposeAttachment {
    /// Human-readable file size.
    pub fn display_size(&self) -> String {
        let bytes = self.data.len();
        if bytes < 1024 {
            format!("{bytes} B")
        } else if bytes < 1024 * 1024 {
            format!("{:.1} KB", bytes as f64 / 1024.0)
        } else {
            format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
        }
    }
}

/// Which recipient field is currently active for autocomplete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecipientField {
    To,
    Cc,
    Bcc,
}

/// State for the recipient autocomplete dropdown.
pub struct AutocompleteState {
    /// Current query being searched.
    pub query: String,
    /// Results from the most recent autocomplete search.
    pub results: Vec<ContactMatch>,
    /// Currently highlighted index in the results list.
    pub highlighted: Option<usize>,
    /// Generation counter to discard stale results.
    pub search_generation: rtsk::generation::GenerationCounter<rtsk::generation::Autocomplete>,
    /// Which recipient field is currently active.
    pub active_field: RecipientField,
}

impl AutocompleteState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            results: Vec::new(),
            highlighted: None,
            search_generation: rtsk::generation::GenerationCounter::new(),
            active_field: RecipientField::To,
        }
    }
}

/// Active drag state for a token being moved between fields.
#[allow(dead_code)] // token_id/source_field/label populated for inspection during drag overlay rework
pub struct ComposeTokenDrag {
    pub token_id: TokenId,
    pub source_field: RecipientField,
    pub label: String,
    pub current_position: Point,
}

/// A Bcc nudge banner shown when a group token is added to To or Cc.
#[allow(dead_code)] // source_field stays populated for the upcoming undo path
pub struct BccNudgeBanner {
    pub group_name: String,
    pub token_id: TokenId,
    pub source_field: RecipientField,
}

/// Successful save-as-group result. Carries the bits needed to mint the
/// replacement group token.
#[derive(Debug, Clone)]
pub struct GroupSaveSuccess {
    pub group_id: String,
    pub name: String,
    pub member_count: i64,
}

/// A bulk paste banner shown when 10+ addresses are pasted. Tracks which
/// field the paste landed in and which token ids were added so the
/// "Save as group" flow can replace exactly those tokens (not whatever the
/// user has typed since) with a single group token.
pub struct BulkPasteBanner {
    pub count: usize,
    pub field: RecipientField,
    pub token_ids: Vec<TokenId>,
}

/// What the right-click context menu is targeting. A specific token
/// gives the full menu (Cut/Copy/Paste/Delete/Expand-group/Move-to);
/// the field's empty area gives a Paste-only menu, since Cut/Copy/Delete
/// have no target.
pub enum ContextMenuKind {
    Token { token_id: TokenId, is_group: bool },
    Field,
}

/// State for the right-click context menu in a recipient field.
pub struct TokenContextMenuState {
    pub field: RecipientField,
    pub position: Point,
    pub kind: ContextMenuKind,
}
