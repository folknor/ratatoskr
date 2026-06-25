use common::ops::ProviderOps;
use db::db::ReadDbState;
use provider_sync::ProviderSyncOps;
use service_state::WriteDbState;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use types::MailProviderKind;

use super::outcome::RemoteFailureKind;

/// Create a provider ops instance for the given account.
///
/// Reads the provider type from the database, decrypts credentials with
/// the encryption key, and constructs the appropriate provider client.
///
/// Returns `Box<dyn ProviderSyncOps>` (Phase 6d-B). The trait inherits
/// `ProviderOps` so a single trait object covers both the action and
/// sync surfaces - action callers continue to call `provider.archive(...)`
/// etc. directly via supertrait method resolution; the sync dispatcher
/// calls `provider.sync_initial(...)` / `sync_delta(...)`. The action
/// service is the single point of provider resolution; the app crate
/// must not construct providers directly.
pub async fn create_provider(
    db: &ReadDbState,
    write_db: &WriteDbState,
    account_id: &str,
    encryption_key: [u8; 32],
) -> Result<Box<dyn ProviderOps>, String> {
    let aid = account_id.to_string();
    let raw_provider = db
        .with_read(move |conn| db::db::queries_extra::get_account_provider_raw_sync(conn, &aid))
        .await?;
    match raw_provider.as_str() {
        "harness-offline" => return Ok(Box::new(HarnessOfflineProvider::immediate())),
        "harness-slow-sync" => return Ok(Box::new(HarnessOfflineProvider::slow_sync())),
        _ => {}
    }

    match MailProviderKind::parse(&raw_provider)? {
        MailProviderKind::Gmail => {
            let client = gmail::client::GmailClient::from_account(
                db,
                write_db.writer_pool(),
                account_id,
                encryption_key,
            )
            .await?;
            Ok(Box::new(gmail::ops::GmailOps::new(client)))
        }
        MailProviderKind::Graph => {
            let client = graph::client::GraphClient::from_account(
                db,
                write_db.writer_pool(),
                account_id,
                encryption_key,
            )
            .await?;
            Ok(Box::new(graph::ops::GraphOps::new(client)))
        }
        MailProviderKind::Jmap => {
            let client = jmap::client::JmapClient::from_account(
                db,
                write_db.writer_pool(),
                account_id,
                &encryption_key,
            )
            .await?;
            Ok(Box::new(jmap::ops::JmapOps::new(client)))
        }
        MailProviderKind::Imap => Ok(Box::new(imap::ops::ImapOps::new(
            encryption_key,
            write_db.writer_pool(),
        ))),
    }
}

pub async fn create_sync_provider(
    db: &ReadDbState,
    write_db: &WriteDbState,
    account_id: &str,
    encryption_key: [u8; 32],
) -> Result<Box<dyn ProviderSyncOps>, String> {
    let aid = account_id.to_string();
    let raw_provider = db
        .with_read(move |conn| db::db::queries_extra::get_account_provider_raw_sync(conn, &aid))
        .await?;
    match raw_provider.as_str() {
        "harness-offline" => return Ok(Box::new(HarnessOfflineProvider::immediate())),
        "harness-slow-sync" => return Ok(Box::new(HarnessOfflineProvider::slow_sync())),
        _ => {}
    }

    match MailProviderKind::parse(&raw_provider)? {
        MailProviderKind::Gmail => {
            let client = gmail::client::GmailClient::from_account(
                db,
                write_db.writer_pool(),
                account_id,
                encryption_key,
            )
            .await?;
            Ok(Box::new(gmail::ops::GmailOps::new(client)))
        }
        MailProviderKind::Graph => {
            let client = graph::client::GraphClient::from_account(
                db,
                write_db.writer_pool(),
                account_id,
                encryption_key,
            )
            .await?;
            Ok(Box::new(graph::ops::GraphOps::new(client)))
        }
        MailProviderKind::Jmap => Err("JMAP sync is handled by the bifrost runner".to_string()),
        MailProviderKind::Imap => Ok(Box::new(imap::ops::ImapOps::new(
            encryption_key,
            write_db.writer_pool(),
        ))),
    }
}

/// Classify a provider-construction error for retry policy.
pub(crate) fn classify_provider_error(error: &str) -> RemoteFailureKind {
    let lower = error.to_lowercase();
    if lower.contains("unknown provider")
        || lower.contains("no rows returned")
        || lower.contains("queryreturnednorows")
        || lower.contains("not found")
        || lower.contains("missing account")
        || lower.contains("malformed stored secret")
        || lower.contains("decrypt credential")
    {
        RemoteFailureKind::Permanent
    } else if lower.contains("timeout")
        || lower.contains("connection refused")
        || lower.contains("dns")
        || lower.contains("network")
    {
        RemoteFailureKind::Transient
    } else {
        RemoteFailureKind::Unknown
    }
}

#[derive(Clone, Copy)]
enum HarnessOfflineMode {
    Immediate,
    SlowSync,
}

struct HarnessOfflineProvider {
    mode: HarnessOfflineMode,
}

impl HarnessOfflineProvider {
    fn immediate() -> Self {
        Self {
            mode: HarnessOfflineMode::Immediate,
        }
    }

    fn slow_sync() -> Self {
        Self {
            mode: HarnessOfflineMode::SlowSync,
        }
    }

    fn offline() -> common::error::ProviderError {
        common::error::ProviderError::Network("harness offline".into())
    }

    async fn sync_result(
        &self,
        ctx: &provider_sync::SyncProviderCtx<'_>,
    ) -> Result<common::types::SyncResult, common::error::ProviderError> {
        match self.mode {
            HarnessOfflineMode::Immediate => Err(Self::offline()),
            HarnessOfflineMode::SlowSync => {
                ctx.cancellation_token.cancelled().await;
                Err(Self::offline())
            }
        }
    }
}

type HarnessAttachmentKey = (String, String, String);

fn harness_attachments()
-> &'static Mutex<HashMap<HarnessAttachmentKey, common::types::FetchedAttachment>> {
    static MAP: OnceLock<Mutex<HashMap<HarnessAttachmentKey, common::types::FetchedAttachment>>> =
        OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn register_harness_attachment(
    account_id: &str,
    message_id: &str,
    attachment_id: &str,
    bytes: Vec<u8>,
) {
    let key = (
        account_id.to_string(),
        message_id.to_string(),
        attachment_id.to_string(),
    );
    let size = bytes.len() as u64;
    let mut guard = harness_attachments()
        .lock()
        .expect("harness attachment map poisoned");
    guard.insert(key, common::types::FetchedAttachment { bytes, size });
}

#[async_trait::async_trait]
impl common::ops::ProviderOps for HarnessOfflineProvider {
    async fn archive(
        &self,
        _ctx: &common::types::ActionProviderCtx<'_>,
        _thread_id: &str,
    ) -> Result<(), common::error::ProviderError> {
        Err(Self::offline())
    }

    async fn trash(
        &self,
        _ctx: &common::types::ActionProviderCtx<'_>,
        _thread_id: &str,
    ) -> Result<(), common::error::ProviderError> {
        Err(Self::offline())
    }

    async fn permanent_delete(
        &self,
        _ctx: &common::types::ActionProviderCtx<'_>,
        _thread_id: &str,
    ) -> Result<(), common::error::ProviderError> {
        Err(Self::offline())
    }

    async fn mark_read(
        &self,
        _ctx: &common::types::ActionProviderCtx<'_>,
        _thread_id: &str,
        _read: bool,
    ) -> Result<(), common::error::ProviderError> {
        Err(Self::offline())
    }

    async fn star(
        &self,
        _ctx: &common::types::ActionProviderCtx<'_>,
        _thread_id: &str,
        _starred: bool,
    ) -> Result<(), common::error::ProviderError> {
        Err(Self::offline())
    }

    async fn spam(
        &self,
        _ctx: &common::types::ActionProviderCtx<'_>,
        _thread_id: &str,
        _is_spam: bool,
    ) -> Result<(), common::error::ProviderError> {
        Err(Self::offline())
    }

    async fn move_to_folder(
        &self,
        _ctx: &common::types::ActionProviderCtx<'_>,
        _thread_id: &str,
        _folder_id: &common::typed_ids::FolderId,
    ) -> Result<(), common::error::ProviderError> {
        Err(Self::offline())
    }

    async fn add_label(
        &self,
        _ctx: &common::types::ActionProviderCtx<'_>,
        _thread_id: &str,
        _label: &common::types::LabelKind,
    ) -> Result<(), common::error::ProviderError> {
        Err(Self::offline())
    }

    async fn remove_label(
        &self,
        _ctx: &common::types::ActionProviderCtx<'_>,
        _thread_id: &str,
        _label: &common::types::LabelKind,
    ) -> Result<(), common::error::ProviderError> {
        Err(Self::offline())
    }

    async fn send_email(
        &self,
        _ctx: &common::types::ProviderCtx<'_>,
        _raw_base64url: &str,
        _thread_id: Option<&str>,
    ) -> Result<String, common::error::ProviderError> {
        Err(Self::offline())
    }

    async fn create_draft(
        &self,
        _ctx: &common::types::ProviderCtx<'_>,
        _raw_base64url: &str,
        _thread_id: Option<&str>,
    ) -> Result<String, common::error::ProviderError> {
        Err(Self::offline())
    }

    async fn update_draft(
        &self,
        _ctx: &common::types::ProviderCtx<'_>,
        _draft_id: &str,
        _raw_base64url: &str,
        _thread_id: Option<&str>,
    ) -> Result<String, common::error::ProviderError> {
        Err(Self::offline())
    }

    async fn delete_draft(
        &self,
        _ctx: &common::types::ProviderCtx<'_>,
        _draft_id: &str,
    ) -> Result<(), common::error::ProviderError> {
        Err(Self::offline())
    }

    async fn fetch_attachment(
        &self,
        ctx: &common::types::ProviderCtx<'_>,
        message_id: &str,
        attachment_id: &str,
    ) -> Result<common::types::FetchedAttachment, common::error::ProviderError> {
        let key = (
            ctx.account_id.to_string(),
            message_id.to_string(),
            attachment_id.to_string(),
        );
        if let Some(data) = harness_attachments()
            .lock()
            .expect("harness attachment map poisoned")
            .get(&key)
            .cloned()
        {
            return Ok(data);
        }
        Err(Self::offline())
    }

    async fn list_folders(
        &self,
        _ctx: &common::types::ProviderCtx<'_>,
    ) -> Result<Vec<common::types::ProviderFolderEntry>, common::error::ProviderError> {
        Err(Self::offline())
    }

    async fn create_folder(
        &self,
        _ctx: &common::types::ProviderCtx<'_>,
        _name: &str,
        _parent_id: Option<&common::typed_ids::FolderId>,
        _text_color: Option<&str>,
        _bg_color: Option<&str>,
    ) -> Result<common::types::ProviderFolderMutation, common::error::ProviderError> {
        Err(Self::offline())
    }

    async fn rename_folder(
        &self,
        _ctx: &common::types::ProviderCtx<'_>,
        _folder_id: &common::typed_ids::FolderId,
        _new_name: &str,
        _text_color: Option<&str>,
        _bg_color: Option<&str>,
    ) -> Result<common::types::ProviderFolderMutation, common::error::ProviderError> {
        Err(Self::offline())
    }

    async fn delete_folder(
        &self,
        _ctx: &common::types::ProviderCtx<'_>,
        _folder_id: &common::typed_ids::FolderId,
    ) -> Result<(), common::error::ProviderError> {
        Err(Self::offline())
    }

    async fn test_connection(
        &self,
        _ctx: &common::types::ProviderCtx<'_>,
    ) -> Result<common::types::ProviderTestResult, common::error::ProviderError> {
        Err(Self::offline())
    }

    async fn get_profile(
        &self,
        _ctx: &common::types::ProviderCtx<'_>,
    ) -> Result<common::types::ProviderProfile, common::error::ProviderError> {
        Err(Self::offline())
    }
}

#[async_trait::async_trait]
impl provider_sync::ProviderSyncOps for HarnessOfflineProvider {
    async fn sync_initial(
        &self,
        ctx: &provider_sync::SyncProviderCtx<'_>,
        _days_back: i64,
    ) -> Result<common::types::SyncResult, common::error::ProviderError> {
        self.sync_result(ctx).await
    }

    async fn sync_delta(
        &self,
        ctx: &provider_sync::SyncProviderCtx<'_>,
        _days_back: Option<i64>,
    ) -> Result<common::types::SyncResult, common::error::ProviderError> {
        self.sync_result(ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_creation_errors_are_classified_for_retry() {
        assert_eq!(
            classify_provider_error("Unknown provider: bogus"),
            RemoteFailureKind::Permanent,
        );
        assert_eq!(
            classify_provider_error("network error: connection refused"),
            RemoteFailureKind::Transient,
        );
        assert_eq!(
            classify_provider_error("decrypt credential: Decryption failed"),
            RemoteFailureKind::Permanent,
        );
        assert_eq!(
            classify_provider_error("provider returned an odd response"),
            RemoteFailureKind::Unknown,
        );
    }
}
