//! Sync dispatch and provider-adjacent app helpers.
//!
//! All provider operations go through core. Phase 4 removed the UI-side
//! JMAP push subscription wiring; push events arrive as
//! `Notification::PushEvent` from the Service-side `PushRuntime`.

use std::sync::Arc;

use iced::Task;

use crate::{Message, ReadyApp};

// ── Sync dispatch ───────────────────────────────────────────────────

impl ReadyApp {
    /// Dispatch a delta sync for a specific account by issuing
    /// `sync.start_account` over IPC. The Service spawns the runner;
    /// the returned task awaits the matching `sync.completed`
    /// notification (correlated by `SyncRunId` inside
    /// `ServiceClient::start_sync`) and resolves to a
    /// `Message::SyncComplete`.
    ///
    /// The "already in flight" guard now lives Service-side: a
    /// duplicate `sync.start_account` for the same account returns
    /// `already_in_flight: true` with the live `run_id`, and both
    /// callers' broadcast subscribers resolve from the same
    /// `SyncCompleted` notification.
    pub(crate) fn dispatch_sync_delta(&mut self, account_id: String) -> Task<Message> {
        let Some(client) = self.service_client.as_ref().cloned() else {
            log::debug!(
                "dispatch_sync_delta({account_id}): no ServiceClient yet; skipping",
            );
            return Task::none();
        };
        let aid_for_msg = account_id.clone();
        Task::perform(
            async move {
                match client.start_sync(account_id).await {
                    Ok(service_api::SyncResult::Completed)
                    | Ok(service_api::SyncResult::Cancelled) => Ok(()),
                    Ok(service_api::SyncResult::Failed(e)) => Err(e),
                    Err(e) => Err(e.to_string()),
                }
            },
            move |result| Message::SyncComplete(aid_for_msg.clone(), result),
        )
    }

    /// Dispatch delta sync for all active accounts.
    pub(crate) fn sync_all_accounts(&mut self) -> Task<Message> {
        let account_ids: Vec<String> =
            self.sidebar.accounts.iter().map(|a| a.id.clone()).collect();
        let tasks: Vec<Task<Message>> = account_ids
            .into_iter()
            .map(|id| self.dispatch_sync_delta(id))
            .collect();

        if tasks.is_empty() {
            return Task::none();
        }
        Task::batch(tasks)
    }

    /// Nudge the Service to drain the `pending_operations` retry queue.
    ///
    /// Phase 2 task 18: the periodic drainer runs Service-side; the UI
    /// is just the trigger so the existing tick policy (focus / online
    /// state gating) stays UI-owned. Fires a fire-and-forget
    /// `pending_ops.kick` notification per Phase 2 plan scope item 11.
    /// Notification class is `Drop`: if the Service's notification pool
    /// is at capacity, the kick is dropped server-side and the UI's
    /// next `SyncTick` retries.
    pub(crate) fn process_pending_ops(&self) -> Task<Message> {
        let Some(client) = self.service_client.as_ref().cloned() else {
            return Task::none();
        };
        Task::perform(
            async move {
                if let Err(error) = client
                    .send_notification(service_api::ClientNotification::PendingOpsKick)
                    .await
                {
                    log::debug!("pending_ops.kick send failed: {error}");
                }
            },
            |()| Message::Noop,
        )
    }

    /// Refresh GAL (Global Address List) caches for accounts that support it.
    /// Only fetches if the cache is stale (>24h). Runs silently in the background.
    pub(crate) fn refresh_gal_caches(&self) -> Task<Message> {
        let encryption_key = self.encryption_key;

        let account_ids: Vec<String> = self
            .sidebar
            .accounts
            .iter()
            .filter(|a| matches!(a.provider.as_str(), "graph" | "gmail_api"))
            .map(|a| a.id.clone())
            .collect();

        if account_ids.is_empty() {
            return Task::none();
        }

        let db = Arc::clone(&self.db);
        Task::perform(
            async move {
                let core_db = db.write_db_state();
                for account_id in &account_ids {
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(60),
                        rtsk::contacts::gal::refresh_gal_for_account(
                            &core_db,
                            account_id,
                            encryption_key,
                        ),
                    )
                    .await
                    {
                        Ok(Ok(n)) if n > 0 => {
                            log::info!("[GAL] Cached {n} entries for {account_id}");
                        }
                        Ok(Ok(_)) => {}
                        Ok(Err(e)) => {
                            log::warn!("[GAL] Refresh failed for {account_id}: {e}");
                        }
                        Err(_) => {
                            log::warn!("[GAL] Refresh timed out for {account_id}");
                        }
                    }
                }
            },
            |()| Message::Noop,
        )
    }

}
