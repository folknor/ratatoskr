//! Session state persistence for pop-out windows.
//!
//! Saves the full window session (main window + pop-outs) to `session.json`
//! on app close, and restores it on launch. Best-effort - if a message was
//! deleted, the window opens with an error banner.

use crate::window_state::WindowState;
use serde::{Deserialize, Serialize};

/// Full session state, saved on app close.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionState {
    /// Main window geometry and layout state.
    pub main_window: WindowState,

    /// Open message view pop-out windows.
    #[serde(default)]
    pub message_views: Vec<MessageViewSessionEntry>,

    /// Open compose pop-out windows. Each carries the `draft_id` of its
    /// `local_drafts` row, which is the canonical store for compose state -
    /// the entry only needs to remember which draft to reopen and where.
    #[serde(default)]
    pub compose_windows: Vec<ComposeSessionEntry>,

    /// Calendar pop-out window, if one was open. The calendar's view and
    /// date persist via the main DB, so the entry only needs geometry.
    #[serde(default)]
    pub calendar_window: Option<CalendarSessionEntry>,
}

/// Minimal data needed to restore a message view window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageViewSessionEntry {
    pub message_id: String,
    pub thread_id: String,
    pub account_id: String,

    // Window geometry
    pub width: f32,
    pub height: f32,
    pub x: Option<f32>,
    pub y: Option<f32>,
}

/// Minimal data needed to restore a compose pop-out. The compose body,
/// recipients, subject, signature, and reply context all live on the
/// `local_drafts` row - the entry just records which draft to reopen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeSessionEntry {
    pub draft_id: String,

    // Window geometry
    pub width: f32,
    pub height: f32,
    pub x: Option<f32>,
    pub y: Option<f32>,
}

/// Geometry needed to restore the calendar pop-out window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarSessionEntry {
    pub width: f32,
    pub height: f32,
    pub x: Option<f32>,
    pub y: Option<f32>,
}

impl SessionState {
    /// Load from disk. Falls back gracefully:
    /// 1. Try `session.json`.
    /// 2. Fall back to `window.json` (migrating from old format).
    /// 3. Use defaults.
    pub fn load(data_dir: &std::path::Path) -> Self {
        // Try session.json first
        let session_path = data_dir.join("session.json");
        if let Ok(bytes) = std::fs::read(&session_path)
            && let Ok(session) = serde_json::from_slice::<Self>(&bytes)
        {
            return session;
        }

        // Fall back to window.json (old format migration)
        let window = WindowState::load(data_dir);
        Self {
            main_window: window,
            message_views: Vec::new(),
            compose_windows: Vec::new(),
            calendar_window: None,
        }
    }
}
