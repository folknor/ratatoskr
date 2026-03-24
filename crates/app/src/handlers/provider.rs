//! Sync dispatch and provider-adjacent app helpers.
//!
//! All provider operations go through core. This module provides thin
//! Task::perform wrappers for sync and JMAP push.

use std::sync::Arc;

use iced::Task;

use crate::db::Db;
use crate::{App, Message};

// ── JMAP push subscription ─────────────────────────────────────────

use std::sync::Mutex;

use iced::advanced::graphics::futures::subscription;
use iced::advanced::subscription::Hasher;
use iced::futures::stream::BoxStream;
use iced::futures::StreamExt;
use iced::Subscription;

/// Shared handle for the JMAP push notification receiver.
pub type JmapPushReceiver = Arc<Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<String>>>>;

/// Create a JMAP push notification channel.
///
/// Returns `(sender, shared_receiver)`. Clone the sender for each
/// account's push setup. Wrap the receiver in a subscription to
/// receive account-ID notifications that trigger syncs.
pub fn create_jmap_push_channel() -> (
    tokio::sync::mpsc::UnboundedSender<String>,
    JmapPushReceiver,
) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    (tx, Arc::new(Mutex::new(Some(rx))))
}

struct JmapPushRecipe {
    receiver: JmapPushReceiver,
}

impl subscription::Recipe for JmapPushRecipe {
    type Output = String;

    fn hash(&self, state: &mut Hasher) {
        use std::hash::Hash;
        struct Marker;
        std::any::TypeId::of::<Marker>().hash(state);
    }

    fn stream(
        self: Box<Self>,
        _input: subscription::EventStream,
    ) -> BoxStream<'static, String> {
        let taken = self
            .receiver
            .lock()
            .ok()
            .and_then(|mut guard| guard.take());

        match taken {
            Some(rx) => {
                iced::futures::stream::unfold(rx, |mut rx| async {
                    let account_id = rx.recv().await?;
                    Some((account_id, rx))
                })
                .boxed()
            }
            None => iced::futures::stream::empty().boxed(),
        }
    }
}

/// Build an iced `Subscription` that yields account IDs from JMAP
/// push state-change notifications.
pub fn jmap_push_subscription(
    receiver: &JmapPushReceiver,
) -> Subscription<String> {
    subscription::from_recipe(JmapPushRecipe {
        receiver: Arc::clone(receiver),
    })
}

// ── Sync dispatch & push setup ──────────────────────────────────────

impl App {
    /// Dispatch a delta sync for a specific account as a background task.
    pub(crate) fn dispatch_sync_delta(
        &self,
        account_id: String,
    ) -> Task<Message> {
        let Some(encryption_key) = self.encryption_key else {
            log::error!("Cannot sync: no encryption key");
            return Task::none();
        };

        let db = Arc::clone(&self.db);
        let body_store = match self.body_store.clone() {
            Some(bs) => bs,
            None => {
                log::error!("Cannot sync: no body store");
                return Task::none();
            }
        };
        let search_state = match self.search_state.clone() {
            Some(ss) => ss,
            None => {
                log::error!("Cannot sync: no search state");
                return Task::none();
            }
        };
        let inline_images = match self.inline_image_store.clone() {
            Some(iis) => iis,
            None => {
                log::error!("Cannot sync: no inline image store");
                return Task::none();
            }
        };
        let reporter = Arc::clone(&self.sync_reporter);

        let aid = account_id.clone();
        Task::perform(
            async move {
                let core_db = ratatoskr_core::db::DbState::from_arc(db.write_conn_arc());
                ratatoskr_core::sync_dispatch::sync_delta_for_account(
                    &core_db,
                    &account_id,
                    encryption_key,
                    &body_store,
                    &inline_images,
                    &*search_state,
                    reporter.as_ref(),
                )
                .await
            },
            move |result| Message::SyncComplete(aid, result),
        )
    }

    /// Dispatch delta sync for all active accounts.
    pub(crate) fn sync_all_accounts(&self) -> Task<Message> {
        let tasks: Vec<Task<Message>> = self
            .sidebar
            .accounts
            .iter()
            .map(|a| self.dispatch_sync_delta(a.id.clone()))
            .collect();

        if tasks.is_empty() {
            return Task::none();
        }
        Task::batch(tasks)
    }

    /// Process pending operations from the retry queue.
    /// Called on SyncTick alongside delta sync.
    pub(crate) fn process_pending_ops(&self) -> Task<Message> {
        let Some(ref action_ctx) = self.action_ctx else {
            return Task::none();
        };
        let ctx = action_ctx.clone();
        Task::perform(
            async move {
                ratatoskr_core::actions::pending::process_pending_ops(&ctx).await;
            },
            |()| Message::Noop,
        )
    }

    /// Start JMAP push notification managers for all JMAP accounts.
    /// Call after accounts are loaded and encryption key is available.
    pub(crate) fn start_jmap_push(&self) -> Task<Message> {
        let Some(encryption_key) = self.encryption_key else {
            return Task::none();
        };

        let jmap_accounts: Vec<(String, String)> = self
            .sidebar
            .accounts
            .iter()
            .filter(|a| a.provider == "jmap")
            .map(|a| (a.id.clone(), a.email.clone()))
            .collect();

        if jmap_accounts.is_empty() {
            return Task::none();
        }

        let db = Arc::clone(&self.db);
        let notify_tx = self.jmap_push_tx.clone();
        let mut tasks = Vec::new();

        for (account_id, email) in jmap_accounts {
            let db = Arc::clone(&db);
            let notify_tx = notify_tx.clone();

            tasks.push(Task::perform(
                async move {
                    let core_db = ratatoskr_core::db::DbState::from_arc(db.write_conn_arc());
                    ratatoskr_core::jmap_push::start_jmap_push_for_account(
                        &core_db,
                        &account_id,
                        &email,
                        encryption_key,
                        notify_tx,
                    )
                    .await
                },
                |result| {
                    if let Err(e) = result {
                        log::warn!("[JMAP Push] Failed to start: {e}");
                    }
                    Message::Noop
                },
            ));
        }

        Task::batch(tasks)
    }
}
