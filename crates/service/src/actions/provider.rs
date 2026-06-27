use common::ops::ProviderOps;
use db::db::ReadDbState;
use service_state::WriteDbState;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use types::MailProviderKind;

/// Create a provider ops instance for the given account.
///
/// Reads the provider type from the database, decrypts credentials with
/// the encryption key, and constructs the appropriate provider client.
///
/// The action service is the single point of provider resolution; the app
/// crate must not construct providers directly.
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
        "harness-slow-sync" => return Ok(Box::new(HarnessOfflineProvider)),
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

struct HarnessOfflineProvider;

impl HarnessOfflineProvider {
    fn immediate() -> Self {
        Self
    }

    fn offline() -> common::error::ProviderError {
        common::error::ProviderError::Network("harness offline".into())
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
