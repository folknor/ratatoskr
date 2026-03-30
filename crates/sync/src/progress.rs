use db::progress::{self, ProgressReporter};

pub fn emit_sync_progress(
    reporter: &dyn ProgressReporter,
    event_name: &str,
    account_id: &str,
    phase: &str,
    current: u64,
    total: u64,
    folder: Option<&str>,
) {
    progress::emit_event(
        reporter,
        event_name,
        &serde_json::json!({
            "accountId": account_id,
            "phase": phase,
            "current": current,
            "total": total,
            "folder": folder,
        }),
    );
}
