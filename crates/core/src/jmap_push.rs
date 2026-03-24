//! JMAP push notification setup.
//!
//! Moved to core so the app crate doesn't need direct JMAP provider dependencies.
//! The push manager spawns its own tokio task and runs independently after setup.

use crate::db::DbState;

/// Start JMAP push for a single account.
///
/// Constructs the JMAP client, creates a push channel, starts the push
/// manager (which runs in its own tokio task), and waits for the first
/// state change. Returns the account_id to trigger a sync.
pub async fn start_jmap_push_for_account(
    db: &DbState,
    account_id: &str,
    email: &str,
    encryption_key: [u8; 32],
) -> Result<String, String> {
    let client =
        ratatoskr_jmap::client::JmapClient::from_account(db, account_id, &encryption_key).await?;

    let (tx, mut rx) = ratatoskr_jmap::push::create_push_channel();
    let _manager = ratatoskr_jmap::push::start_push(&client, account_id, db, tx).await?;

    // Wait for the first state change, then return to trigger a sync.
    // The push manager continues running in its own tokio task.
    log::info!("[JMAP Push] Listening for changes on {email}");
    if let Some(change) = rx.recv().await {
        log::info!(
            "[JMAP Push] State change for {email}: {} data types changed",
            change.changed.len()
        );
    }

    Ok(account_id.to_string())
}
