//! `action.send` handler.
//!
//! Phase 2 task 13: compose-send relocates as a quiet journal job
//! (`kind = 'send'`). Handler validates the wire request, transfers
//! each staged attachment into the Service-owned send vault
//! (atomic rename with SHA-256 verify), journals the send into
//! `action_jobs`, and signals the worker. SMTP submit happens on the
//! worker, not the handler.
//!
//! Crash semantics:
//! - Pre-transfer crash (Service died before reading the request):
//!   staging dir is still UI-owned, UI cleans up on respawn.
//! - Mid-transfer crash (some attachments already renamed into vault,
//!   journal not yet committed): boot's orphan-vault sweep removes
//!   the partially-populated vault dir because no `action_jobs` row
//!   references it.
//! - Post-journal-commit crash: vault dir is preserved by the boot
//!   sweep (live job), worker resumes via journal replay after the
//!   respawn.
//!
//! 30 s handler timeout (per `service-api`'s `RequestParams::timeout`)
//! covers SHA-256 verify of typical attachment payloads. Cross-
//! filesystem rename is out of scope (see `send_vault` doc); same-FS
//! is asserted by the assumption that `<app_data>/staging/` and
//! `<app_data>/send_vault/` live on the same volume.

use std::sync::Arc;

use db::db::action_journal::insert_quiet_job;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use service_api::{PlanId, SendAck, SendWireRequest, ServiceError};

use crate::boot::BootSharedState;
use crate::send_vault;

/// Serialized payload for a `kind = 'send'` job. Stored in
/// `action_jobs.payload` so the worker has everything it needs to
/// build the MIME message and submit via SMTP after a Service
/// respawn. Vault paths are absolute so the worker can read directly
/// without recomputing the layout (the handler ran the rename, the
/// journal records the result).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct JournaledSend {
    pub send_id: PlanId,
    pub account_id: String,
    pub message: JournaledMessage,
    pub attachments: Vec<JournaledAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct JournaledMessage {
    pub draft_id: String,
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: Option<String>,
    pub body_html: String,
    pub body_text: String,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    pub thread_id: Option<String>,
    #[serde(default)]
    pub source_message_id: Option<String>,
    #[serde(default)]
    pub intent: service_api::SendIntent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct JournaledAttachment {
    pub vault_path: std::path::PathBuf,
    pub filename: String,
    pub mime: String,
    pub content_id: Option<String>,
    pub size: u64,
    pub content_hash: [u8; 32],
}

pub(super) async fn handle(
    state: &Arc<BootSharedState>,
    request: SendWireRequest,
) -> Result<Value, ServiceError> {
    let db = state.write_db_state()?;
    let app_data_dir = state.app_data_dir().to_path_buf();
    let send_id = request.send_id;

    // Move all the request-derived state into a spawn_blocking so the
    // SHA-256 verify + filesystem rename don't tie up the runtime.
    // Returns the journaled-payload-shaped data; we serialize + insert
    // in a follow-up spawn_blocking against the DB lock.
    let app_data_for_blocking = app_data_dir.clone();
    let payload: JournaledSend = tokio::task::spawn_blocking(move || {
        transfer_attachments(&app_data_for_blocking, request)
    })
    .await
    .map_err(|error| ServiceError::Internal(format!("spawn_blocking: {error}")))??;

    let payload_blob = serde_json::to_vec(&payload).map_err(|error| {
        ServiceError::Internal(format!("serialize JournaledSend: {error}"))
    })?;
    let job_id_bytes = *payload.send_id.0.as_bytes();
    let account_id_for_journal = payload.account_id.clone();

    // Journal the send. On success, the bytes-ownership boundary is
    // crossed: a Service crash from this point will replay via the
    // worker's journal drain. On failure, the partial vault dir is
    // unlinked so the orphan-cleanup pass at next boot has nothing to
    // collect.
    let journal_result = db
        .with_write(move |conn| {
            insert_quiet_job(
                conn,
                &job_id_bytes,
                "send",
                &account_id_for_journal,
                &payload_blob,
            )
            .map(|_| ())
        })
        .await
        .map_err(ServiceError::Internal);

    if let Err(error) = journal_result {
        send_vault::cleanup_vault_dir(&app_data_dir, &send_id);
        return Err(error);
    }

    state.notify_action_worker();

    let ack = SendAck {
        send_id,
        journaled: true,
    };
    serde_json::to_value(&ack).map_err(|error| ServiceError::Internal(error.to_string()))
}

/// Validate the request and rename every staged attachment into the
/// vault. Returns a fully-populated `JournaledSend` ready for
/// serialization. On any failure, removes the partially-populated
/// vault dir before returning so the caller can surface the error
/// without leaving filesystem residue.
fn transfer_attachments(
    app_data_dir: &std::path::Path,
    request: SendWireRequest,
) -> Result<JournaledSend, ServiceError> {
    let send_id = request.send_id;
    let vault_dir = send_vault::vault_dir(app_data_dir, &send_id);

    if let Err(error) = std::fs::create_dir_all(&vault_dir) {
        return Err(ServiceError::Internal(format!(
            "create vault dir {}: {error}",
            vault_dir.display()
        )));
    }

    let mut journaled_attachments = Vec::with_capacity(request.attachments.len());
    for (index, att) in request.attachments.into_iter().enumerate() {
        let (relative_path, content_hash) = match att.source {
            service_api::SendAttachmentSource::StagingFile {
                relative_path,
                content_hash,
            } => (relative_path, content_hash),
        };
        let vault_path = match send_vault::verify_and_transfer(
            app_data_dir,
            &send_id,
            &relative_path,
            &content_hash,
            index,
        ) {
            Ok(p) => p,
            Err(error) => {
                send_vault::cleanup_vault_dir(app_data_dir, &send_id);
                return Err(map_vault_error(error));
            }
        };
        journaled_attachments.push(JournaledAttachment {
            vault_path,
            filename: att.filename,
            mime: att.mime,
            content_id: att.content_id,
            size: att.size,
            content_hash,
        });
    }

    Ok(JournaledSend {
        send_id,
        account_id: request.from_account_id,
        message: JournaledMessage {
            draft_id: request.message.draft_id,
            from: request.message.from,
            to: request.message.to,
            cc: request.message.cc,
            bcc: request.message.bcc,
            subject: request.message.subject,
            body_html: request.message.body_html,
            body_text: request.message.body_text,
            in_reply_to: request.message.in_reply_to,
            references: request.message.references,
            thread_id: request.message.thread_id,
            source_message_id: request.message.source_message_id,
            intent: request.message.intent,
        },
        attachments: journaled_attachments,
    })
}

/// Translate a `send_vault::VaultError` into a `ServiceError`. Hash
/// mismatch and traversal violations are `InvalidParams` (UI bug or
/// hostile crafted request); IO errors are `Internal` (the Service
/// couldn't read or rename a file we expected to exist).
fn map_vault_error(error: send_vault::VaultError) -> ServiceError {
    use send_vault::VaultError;
    match error {
        VaultError::InvalidPath(detail) => ServiceError::InvalidParams {
            method: "action.send".into(),
            message: format!("staging path rejected: {detail}"),
        },
        VaultError::HashMismatch(path) => ServiceError::InvalidParams {
            method: "action.send".into(),
            message: format!(
                "staging file hash mismatch: {}",
                path.display()
            ),
        },
        VaultError::StagingSymlink(path) => ServiceError::InvalidParams {
            method: "action.send".into(),
            message: format!("staging file is a symlink: {}", path.display()),
        },
        VaultError::StagingIo(path, error) => ServiceError::Internal(format!(
            "staging IO error {}: {error}",
            path.display()
        )),
        VaultError::VaultIo(path, error) => ServiceError::Internal(format!(
            "vault IO error {}: {error}",
            path.display()
        )),
    }
}
