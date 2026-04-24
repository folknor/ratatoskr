//! Session state persistence for pop-out windows.
//!
//! Saves the full window session (main window + pop-outs) to `session.json`
//! on app close, and restores it on launch. Best-effort - if a message was
//! deleted, the window opens with an error banner.

use crate::window_state::WindowState;
use serde::{Deserialize, Serialize};

/// Full session state, saved on app close.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    /// Main window geometry and layout state.
    pub main_window: WindowState,

    /// Open message view pop-out windows.
    #[serde(default)]
    pub message_views: Vec<MessageViewSessionEntry>,
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

impl SessionState {
    /// Load from disk. Falls back gracefully:
    /// 1. Try `session.json`.
    /// 2. Fall back to `window.json` (migrating from old format).
    /// 3. Use defaults.
    pub fn load(data_dir: &std::path::Path) -> Self {
        // Try session.json first
        let session_path = data_dir.join("session.json");
        if let Ok(bytes) = std::fs::read(&session_path) {
            if let Ok(session) = serde_json::from_slice::<Self>(&bytes) {
                return session;
            }
        }

        // Fall back to window.json (old format migration)
        let window = WindowState::load(data_dir);
        Self {
            main_window: window,
            message_views: Vec::new(),
        }
    }
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            main_window: WindowState::default(),
            message_views: Vec::new(),
        }
    }
}
