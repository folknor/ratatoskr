//! Orchestration layer for per-shared-mailbox sync.
//!
//! Each shared/delegated mailbox syncs independently with its own delta tokens,
//! retry state, and error tracking. The existing `graph_initial_sync()` and
//! `graph_delta_sync()` work transparently with shared mailbox clients because
//! all API paths use `client.api_path_prefix()` and delta token routing is
//! handled by the wrapper functions in `sync.rs`.

use common::types::SyncResult;
use db::db::ReadDbState;
use db::progress::ProgressReporter;
use service_state::{
    BodyStoreWriteState, InlineImageStoreWriteState, SearchWriteHandle, WriteDbState,
};
use sync::state as sync_state;
use tokio_util::sync::CancellationToken;

use super::client::GraphClient;
use super::sync::{graph_delta_sync, graph_initial_sync};

/// Default number of days to look back during initial sync of a shared mailbox.
const SHARED_MAILBOX_INITIAL_SYNC_DAYS: i64 = 30;

/// Run sync for a single shared mailbox.
///
/// Creates a scoped `GraphClient` and runs initial or delta sync depending on
/// whether delta tokens already exist for this mailbox. Updates the sync state
/// in `shared_mailbox_sync_state` on success or failure.
#[allow(clippy::too_many_arguments)]
pub async fn sync_shared_mailbox(
    primary_client: &GraphClient,
    mailbox_id: &str,
    db: &WriteDbState,
    read_db: &ReadDbState,
    body_store: &BodyStoreWriteState,
    inline_images: &InlineImageStoreWriteState,
    search: &SearchWriteHandle,
    progress: &dyn ProgressReporter,
    cancellation_token: &CancellationToken,
    account_id: &str,
) -> Result<SyncResult, String> {
    let scoped_client = primary_client.for_shared_mailbox(mailbox_id.to_string());

    // Check if we have any delta tokens for this mailbox - if not, run initial sync.
    let tokens =
        sync_state::load_shared_mailbox_delta_tokens(read_db, account_id, mailbox_id).await?;

    let now = chrono::Utc::now().timestamp();

    if tokens.is_empty() {
        log::info!("Shared mailbox {mailbox_id}: no delta tokens found, running initial sync");
        match graph_initial_sync(
            &scoped_client,
            account_id,
            db,
            read_db,
            body_store,
            inline_images,
            search,
            progress,
            cancellation_token,
            SHARED_MAILBOX_INITIAL_SYNC_DAYS,
        )
        .await
        {
            Ok(()) => {
                sync_state::update_shared_mailbox_sync_status(
                    read_db, account_id, mailbox_id, now, None,
                )
                .await?;
                Ok(SyncResult::default())
            }
            Err(e) => {
                log::warn!("Shared mailbox {mailbox_id} initial sync failed: {e}");
                sync_state::update_shared_mailbox_sync_status(
                    read_db,
                    account_id,
                    mailbox_id,
                    now,
                    Some(&e),
                )
                .await?;
                Err(e)
            }
        }
    } else {
        log::info!(
            "Shared mailbox {mailbox_id}: {} delta tokens found, running delta sync",
            tokens.len()
        );
        match graph_delta_sync(
            &scoped_client,
            account_id,
            db,
            read_db,
            body_store,
            inline_images,
            search,
            progress,
            cancellation_token,
        )
        .await
        {
            Ok(sync_result) => {
                sync_state::update_shared_mailbox_sync_status(
                    read_db, account_id, mailbox_id, now, None,
                )
                .await?;
                Ok(sync_result)
            }
            Err(e) => {
                log::warn!("Shared mailbox {mailbox_id} delta sync failed: {e}");
                sync_state::update_shared_mailbox_sync_status(
                    read_db,
                    account_id,
                    mailbox_id,
                    now,
                    Some(&e),
                )
                .await?;
                Err(e)
            }
        }
    }
}

/// Sync all enabled shared mailboxes for an account.
///
/// Each mailbox syncs independently - one failure does not block others.
/// Returns a list of `(mailbox_id, result)` pairs for the caller to log/report.
#[allow(clippy::too_many_arguments)]
pub async fn sync_all_shared_mailboxes(
    primary_client: &GraphClient,
    db: &WriteDbState,
    read_db: &ReadDbState,
    body_store: &BodyStoreWriteState,
    inline_images: &InlineImageStoreWriteState,
    search: &SearchWriteHandle,
    progress: &dyn ProgressReporter,
    cancellation_token: &CancellationToken,
    account_id: &str,
) -> Vec<(String, Result<SyncResult, String>)> {
    let enabled = match sync_state::get_enabled_shared_mailboxes(read_db, account_id).await {
        Ok(list) => list,
        Err(e) => {
            log::warn!("Failed to load enabled shared mailboxes: {e}");
            return Vec::new();
        }
    };

    if enabled.is_empty() {
        return Vec::new();
    }

    log::info!(
        "Syncing {} enabled shared mailbox(es) for account {account_id}",
        enabled.len()
    );

    let mut results = Vec::with_capacity(enabled.len());

    for entry in &enabled {
        let display = entry.display_name.as_deref().unwrap_or(&entry.mailbox_id);
        log::info!("Starting sync for shared mailbox: {display}");

        let result = sync_shared_mailbox(
            primary_client,
            &entry.mailbox_id,
            db,
            read_db,
            body_store,
            inline_images,
            search,
            progress,
            cancellation_token,
            account_id,
        )
        .await;

        match &result {
            Ok(sr) => {
                log::info!(
                    "Shared mailbox {display}: sync complete ({} new inbox, {} affected threads)",
                    sr.new_inbox_message_ids.len(),
                    sr.affected_thread_ids.len()
                );
            }
            Err(e) => {
                log::warn!("Shared mailbox {display}: sync failed: {e}");
            }
        }

        results.push((entry.mailbox_id.clone(), result));
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use sync::state::SharedMailboxSyncEntry;

    #[test]
    fn shared_mailbox_initial_sync_days_is_reasonable() {
        const _: () = assert!(SHARED_MAILBOX_INITIAL_SYNC_DAYS >= 7);
        const _: () = assert!(SHARED_MAILBOX_INITIAL_SYNC_DAYS <= 90);
    }

    #[test]
    fn shared_mailbox_sync_entry_fields() {
        let entry = SharedMailboxSyncEntry {
            mailbox_id: "team@contoso.com".to_string(),
            display_name: Some("Team Mailbox".to_string()),
            last_synced_at: Some(1_700_000_000),
            sync_error: None,
        };
        assert_eq!(entry.mailbox_id, "team@contoso.com");
        assert_eq!(entry.display_name.as_deref(), Some("Team Mailbox"));
        assert_eq!(entry.last_synced_at, Some(1_700_000_000));
        assert!(entry.sync_error.is_none());
    }

    #[test]
    fn shared_mailbox_sync_entry_with_error() {
        let entry = SharedMailboxSyncEntry {
            mailbox_id: "shared@example.com".to_string(),
            display_name: None,
            last_synced_at: Some(1_700_000_000),
            sync_error: Some("401 Unauthorized".to_string()),
        };
        assert!(entry.sync_error.is_some());
        assert!(entry.display_name.is_none());
    }

}
