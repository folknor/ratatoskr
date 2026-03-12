pub use ratatoskr_core::progress::*;

/// Tauri-backed implementation — forwards to `AppHandle::emit()`.
pub struct TauriProgressReporter {
    app_handle: tauri::AppHandle,
}

impl TauriProgressReporter {
    pub fn from_ref(app_handle: &tauri::AppHandle) -> Self {
        Self {
            app_handle: app_handle.clone(),
        }
    }
}

impl ProgressReporter for TauriProgressReporter {
    fn emit_json(&self, event_name: &str, json: serde_json::Value) {
        use tauri::Emitter;
        if let Err(e) = self.app_handle.emit(event_name, json) {
            log::warn!("Failed to emit {event_name}: {e}");
        }
    }
}
