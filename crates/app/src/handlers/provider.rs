//! Sync dispatch and provider-adjacent app helpers.
//!
//! All provider operations go through core. Phase 4 removed the UI-side
//! JMAP push subscription wiring; push events arrive as
//! `Notification::PushEvent` from the Service-side `PushRuntime`.

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

    /// Phase 5 task 10: kick the Service-side GAL refresh.
    ///
    /// Replaces the deleted UI-side `refresh_gal_caches`. The Service
    /// handler iterates all accounts and calls `refresh_gal_for_account`
    /// (which self-gates via the 24 h cache check), under a global
    /// Tokio Mutex so the `NOTIFY_CAP=4` concurrent dispatcher can't
    /// double-fire stale-account fetches. Notification class is `Drop`
    /// - missed kicks self-heal on the next `SyncTick`.
    pub(crate) fn kick_gal_refresh(&self) -> Task<Message> {
        let Some(client) = self.service_client.as_ref().cloned() else {
            return Task::none();
        };
        Task::perform(
            async move {
                if let Err(error) = client
                    .send_notification(service_api::ClientNotification::GalKick)
                    .await
                {
                    log::debug!("gal.kick send failed: {error}");
                }
            },
            |()| Message::Noop,
        )
    }

    /// Phase 5 task 10: kick the Service-side calendar sync.
    ///
    /// Replaces the deleted UI-side `sync_calendars`. The Service
    /// handler enumerates accounts whose `last_calendar_sync` is more
    /// than 1 h stale and starts a `CalendarRuntime` runner for each.
    /// Notification class is `Drop` - missed kicks self-heal on the
    /// next `SyncTick`.
    pub(crate) fn kick_calendar_sync(&self) -> Task<Message> {
        let Some(client) = self.service_client.as_ref().cloned() else {
            return Task::none();
        };
        Task::perform(
            async move {
                if let Err(error) = client
                    .send_notification(service_api::ClientNotification::CalendarKick)
                    .await
                {
                    log::debug!("calendar.kick send failed: {error}");
                }
            },
            |()| Message::Noop,
        )
    }

}
