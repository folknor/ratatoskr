//! `action.mark_chat_read` handler.
//!
//! Phase 2 plan scope item 18c: the chat read-on-view side effect
//! relocates as a quiet journal job. The handler runs the local DB
//! mutation (atomically resolving the affected threads), journals the
//! list as `kind = 'mark_chat_read'` (quiet = 1) for deterministic
//! replay, and signals the worker. The worker then dispatches provider
//! `mark_read` per affected thread and finalizes the job; quiet jobs
//! suppress per-operation `OperationOutcome` notifications and emit
//! only a final `ActionCompleted`.
//!
//! Crash semantics:
//! - Crash between local-commit and journal-commit: the local DB has
//!   `is_read = 1` but no remote dispatch fires. The next sync
//!   reconciles. Acceptable - matches the existing UI-side flow's
//!   behavior under the same race.
//! - Crash between journal-commit and worker-completion: the
//!   respawned worker picks up the queued `mark_chat_read` row and
//!   runs the remote dispatch.
//!
//! 10 s timeout (handler is just local DB write + journal + signal).
//! Provider mark-read is on the worker, with no IPC timeout.

use crate::boot::BootSharedState;
use db::db::action_journal::insert_quiet_job;
use db::db::queries_extra::chat::mark_chat_read_local_sync;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use service_api::{MarkChatReadAck, PlanId, ServiceError};
use std::sync::Arc;

/// Serialized payload for a `mark_chat_read` job. Stored in
/// `action_jobs.payload` so the worker has everything it needs to
/// dispatch provider mark-read against the resolved threads even
/// after a Service respawn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct JournaledChatRead {
    pub chat_email: String,
    /// `(account_id, thread_id)` pairs returned by
    /// `mark_chat_read_local`. Captured at handler time so worker
    /// behaviour is deterministic across respawns.
    pub affected: Vec<(String, String)>,
}

pub(super) async fn handle(
    state: &Arc<BootSharedState>,
    chat_email: String,
) -> Result<Value, ServiceError> {
    let conn = state
        .db_conn()
        .ok_or_else(|| ServiceError::Internal("boot context not populated".into()))?;

    // 1. Local DB write (single transaction): flip messages.is_read,
    //    mirror on threads.is_read, reset chat_contacts.unread_count.
    //    Returns affected (account_id, thread_id) pairs.
    let conn_for_local = Arc::clone(&conn);
    let email_for_local = chat_email.to_lowercase();
    let affected: Vec<(String, String)> = tokio::task::spawn_blocking(move || {
        let conn = conn_for_local
            .lock()
            .map_err(|error| format!("db lock poisoned: {error}"))?;
        mark_chat_read_local_sync(&conn, &email_for_local)
    })
    .await
    .map_err(|error| ServiceError::Internal(format!("spawn_blocking: {error}")))?
    .map_err(ServiceError::Internal)?;

    // 2. Journal a quiet job. The payload carries the affected list so
    //    the worker can run remote dispatch deterministically across
    //    respawns. account_id is the journaled account scope; we use
    //    the first affected thread's account if any (mark_chat_read
    //    spans accounts naturally - the journal column accepts any
    //    valid account id and the worker walks the payload's pairs
    //    directly). For an empty affected list (chat had no unread
    //    messages), we still journal so the UI's ack is meaningful;
    //    the worker no-ops and emits ActionCompleted immediately.
    let job_id = uuid::Uuid::now_v7();
    let job_id_bytes = *job_id.as_bytes();
    let payload = JournaledChatRead {
        chat_email: chat_email.clone(),
        affected,
    };
    let payload_blob = serde_json::to_vec(&payload)
        .map_err(|error| ServiceError::Internal(format!("serialize JournaledChatRead: {error}")))?;
    let account_id_for_journal = payload
        .affected
        .first()
        .map_or_else(|| "<chat>".to_string(), |(aid, _)| aid.clone());

    let conn_for_journal = Arc::clone(&conn);
    tokio::task::spawn_blocking(move || {
        let conn = conn_for_journal
            .lock()
            .map_err(|error| format!("db lock poisoned: {error}"))?;
        insert_quiet_job(
            &conn,
            &job_id_bytes,
            "mark_chat_read",
            &account_id_for_journal,
            &payload_blob,
        )
        .map(|_| ())
    })
    .await
    .map_err(|error| ServiceError::Internal(format!("spawn_blocking: {error}")))?
    .map_err(ServiceError::Internal)?;

    // 3. Wake the worker so it picks up this job in its next pass.
    state.notify_action_worker();

    let ack = MarkChatReadAck {
        job_id: PlanId(job_id),
        journaled: true,
    };
    serde_json::to_value(&ack).map_err(|error| ServiceError::Internal(error.to_string()))
}
