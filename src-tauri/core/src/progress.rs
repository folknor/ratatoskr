/// Framework-agnostic event emission trait.
///
/// Replaces direct `tauri::AppHandle::emit()` calls so that core logic
/// can run under any UI framework (Tauri, iced, tests, CLI).
///
/// All callers treat emission as best-effort (log on failure, never propagate),
/// so the trait method returns nothing.
pub trait ProgressReporter: Send + Sync {
    /// Emit a named event with a pre-serialized JSON payload.
    fn emit_json(&self, event_name: &str, json: serde_json::Value);
}

/// Convenience: serialize a `Serialize` value and emit it.
pub fn emit_event<T: serde::Serialize>(
    reporter: &dyn ProgressReporter,
    event_name: &str,
    payload: &T,
) {
    match serde_json::to_value(payload) {
        Ok(json) => reporter.emit_json(event_name, json),
        Err(e) => log::warn!("Failed to serialize event {event_name}: {e}"),
    }
}
