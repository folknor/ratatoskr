//! JMAP push notification setup.
//!
//! Moved to core so the app crate doesn't need direct JMAP provider dependencies.
//! Each account gets a long-lived bridge task that forwards state changes as
//! account-ID notifications through a caller-provided channel.

use crate::db::DbState;

/// Start JMAP push for a single account.
///
/// Constructs the JMAP client, starts the push manager, and spawns a
/// bridge task that forwards state-change notifications as `account_id`
/// strings through `notify_tx`. The bridge task keeps the push manager
/// alive — when `notify_tx` is closed (receiver dropped), the bridge
/// exits and the push connection shuts down cleanly.
pub async fn start_jmap_push_for_account(
    db: &DbState,
    account_id: &str,
    email: &str,
    encryption_key: [u8; 32],
    notify_tx: tokio::sync::mpsc::UnboundedSender<String>,
) -> Result<(), String> {
    let client =
        ratatoskr_jmap::client::JmapClient::from_account(db, account_id, &encryption_key).await?;

    let (tx, mut rx) = ratatoskr_jmap::push::create_push_channel();
    let manager = ratatoskr_jmap::push::start_push(&client, account_id, db, tx).await?;

    // Spawn a bridge task that keeps the manager alive and forwards
    // state changes as account-ID notifications to the app layer.
    let aid = account_id.to_string();
    let email = email.to_string();
    tokio::spawn(async move {
        // Moving the manager into this task keeps shutdown_tx alive.
        // When this task exits, the manager drops, shutdown_tx closes,
        // and the push connection loop terminates cleanly.
        let _manager = manager;
        log::info!("[JMAP Push] Listening for changes on {email}");
        while let Some(change) = rx.recv().await {
            log::info!(
                "[JMAP Push] State change for {email}: {} data types changed",
                change.changed.len()
            );
            if notify_tx.send(aid.clone()).is_err() {
                log::info!("[JMAP Push] Notify channel closed for {email}");
                break;
            }
        }
        log::info!("[JMAP Push] Push ended for {email}");
    });

    Ok(())
}
