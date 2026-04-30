use crate::ui::undoable::UndoableText;

/// An account card in the settings list.
#[derive(Debug, Clone)]
pub struct ManagedAccount {
    pub id: String,
    pub email: String,
    pub provider: String,
    pub account_name: Option<String>,
    pub account_color: Option<String>,
    pub display_name: Option<String>,
    pub last_sync_at: Option<i64>,
    pub health: AccountHealth,
}

/// Account health status for the settings card indicator.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AccountHealth {
    #[default]
    Healthy,
    Warning,
    Error,
    Disabled,
}

/// Compute account health from token expiry and sync state.
pub fn compute_health(
    last_sync_at: Option<i64>,
    token_expires_at: Option<i64>,
    is_active: bool,
) -> AccountHealth {
    if !is_active {
        return AccountHealth::Disabled;
    }
    let now = chrono::Utc::now().timestamp();
    if let Some(expires) = token_expires_at {
        let no_recent_sync = last_sync_at.is_none_or(|ls| now - ls > 3600);
        if expires < now && no_recent_sync {
            return AccountHealth::Error;
        }
        if expires < now + 86400 {
            return AccountHealth::Warning;
        }
    }
    AccountHealth::Healthy
}

/// The slide-in editor state for a single account.
#[derive(Debug, Clone)]
pub struct AccountEditor {
    pub account_id: String,
    pub account_email: String,
    pub account_name: UndoableText,
    pub display_name: UndoableText,
    pub account_color_index: Option<usize>,
    pub caldav_url: UndoableText,
    pub caldav_username: UndoableText,
    pub caldav_password: UndoableText,
    pub show_delete_confirmation: bool,
    pub dirty: bool,
}

/// State for an active account card drag operation.
#[derive(Debug, Clone)]
pub struct AccountDragState {
    pub dragging_index: usize,
    /// Y coordinate when the grip was pressed (list-relative, set on first move).
    pub start_y: f32,
    /// Whether the mouse has moved far enough to count as a real drag.
    pub is_dragging: bool,
}
