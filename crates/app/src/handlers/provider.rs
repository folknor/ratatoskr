//! Provider client construction and sync dispatch.
//!
//! This module provides the bridge between the app layer and the provider
//! crates (Gmail, Graph, JMAP, IMAP). It constructs provider clients from
//! account credentials and builds `ProviderCtx` for calling provider
//! operations.

use std::sync::Arc;

use iced::Task;
use ratatoskr_provider_utils::ops::ProviderOps;
use ratatoskr_provider_utils::types::ProviderCtx;

use crate::db::Db;
use crate::{App, Message};

/// Create a provider ops instance for the given account.
///
/// Reads credentials from the database, decrypts them with the encryption
/// key, and constructs the appropriate provider client.
pub(crate) async fn create_provider(
    db: &Arc<Db>,
    account_id: &str,
    encryption_key: [u8; 32],
) -> Result<Box<dyn ProviderOps>, String> {
    let aid = account_id.to_string();
    let provider = db
        .with_conn(move |conn| {
            conn.query_row(
                "SELECT provider FROM accounts WHERE id = ?1",
                rusqlite::params![aid],
                |row| row.get::<_, String>(0),
            )
            .map_err(|e| e.to_string())
        })
        .await?;

    let core_db = ratatoskr_core::db::DbState::from_arc(db.write_conn_arc());

    match provider.as_str() {
        "gmail_api" => {
            let client =
                ratatoskr_gmail::client::GmailClient::from_account(
                    &core_db,
                    account_id,
                    encryption_key,
                )
                .await?;
            Ok(Box::new(ratatoskr_gmail::ops::GmailOps::new(client)))
        }
        "graph" => {
            let client =
                ratatoskr_graph::client::GraphClient::from_account(
                    &core_db,
                    account_id,
                    encryption_key,
                )
                .await?;
            Ok(Box::new(ratatoskr_graph::ops::GraphOps::new(client)))
        }
        "jmap" => {
            let client =
                ratatoskr_jmap::client::JmapClient::from_account(
                    &core_db,
                    account_id,
                    &encryption_key,
                )
                .await?;
            Ok(Box::new(ratatoskr_jmap::ops::JmapOps::new(client)))
        }
        "imap" => Ok(Box::new(ratatoskr_imap::ops::ImapOps::new(
            encryption_key,
        ))),
        other => Err(format!("Unknown provider: {other}")),
    }
}

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

        Task::perform(
            async move {
                let provider = create_provider(&db, &account_id, encryption_key).await?;
                let core_db = ratatoskr_core::db::DbState::from_arc(db.write_conn_arc());
                let ctx = ProviderCtx {
                    account_id: &account_id,
                    db: &core_db,
                    body_store: &body_store,
                    inline_images: &inline_images,
                    search: &*search_state,
                    progress: reporter.as_ref(),
                };
                provider.sync_delta(&ctx, None).await.map_err(|e| e.to_string())
            },
            |result| {
                if let Err(e) = result {
                    log::error!("Sync delta failed: {e}");
                }
                Message::Noop
            },
        )
    }
}
