use super::context::ActionContext;
use super::outcome::{ActionError, ActionOutcome};
use super::provider::create_provider;
use crate::progress::NoopProgressReporter;
use crate::send::{build_mime_message_base64url, mark_draft_failed, mark_draft_sent, SendRequest};
use ratatoskr_provider_utils::types::ProviderCtx;

/// Send an email: build MIME, persist draft, dispatch to provider.
///
/// On success, the provider-assigned sent message ID is stored in
/// `local_drafts.remote_draft_id` via `mark_draft_sent()` — the caller
/// does not need it. Returns plain `ActionOutcome::Success`.
///
/// On any failure (MIME build, DB, or provider), returns `Failed` and
/// marks the draft as `'failed'` if it was persisted. `LocalOnly` is
/// not used for send — the desired outcome is delivery, not local
/// persistence.
pub async fn send_email(ctx: &ActionContext, request: SendRequest) -> ActionOutcome {
    // 1. Build MIME + persist draft in one spawn_blocking call.
    //    MIME build is CPU-bound (large attachments); draft persist is DB I/O.
    //    Both are sync — combine them to avoid two spawn_blocking round-trips.
    //
    //    Both `db_save_local_draft` and `mark_draft_sending` are async helpers
    //    that take `&DbState`. Inside spawn_blocking we already hold the Mutex
    //    lock, so we inline the equivalent SQL rather than calling the async
    //    helpers. The validation logic is identical.
    let db = ctx.db.clone();
    let draft_id = request.draft_id.clone();
    let account_id = request.account_id.clone();
    let thread_id = request.thread_id.clone();
    // Clone for use after the spawn_blocking closure moves the originals.
    let draft_id_outer = draft_id.clone();
    let account_id_outer = account_id.clone();
    let thread_id_outer = thread_id.clone();
    let local_result = tokio::task::spawn_blocking(move || {
        let mime_base64url = build_mime_message_base64url(&request)
            .map_err(|e| ActionError::build(format!("{e}")))?;

        let conn = db.conn();
        let conn = conn.lock().map_err(|e| ActionError::db(format!("db lock: {e}")))?;

        // Persist draft as 'pending'.
        // Field mapping: SendRequest → local_drafts columns
        //   request.from        → from_email
        //   request.to          → to_addresses (joined)
        //   request.cc          → cc_addresses (joined)
        //   request.bcc         → bcc_addresses (joined)
        //   request.in_reply_to → reply_to_message_id
        //   mime_base64url       → attachments
        // INSERT with ON CONFLICT so retries (same draft_id after failure)
        // update the existing row instead of creating a new one.
        conn.execute(
            "INSERT INTO local_drafts \
             (id, account_id, to_addresses, cc_addresses, bcc_addresses, \
              subject, body_html, reply_to_message_id, thread_id, \
              from_email, attachments, updated_at, sync_status) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, \
                     unixepoch(), 'pending') \
             ON CONFLICT(id) DO UPDATE SET \
               to_addresses = ?3, cc_addresses = ?4, bcc_addresses = ?5, \
               subject = ?6, body_html = ?7, reply_to_message_id = ?8, \
               thread_id = ?9, from_email = ?10, attachments = ?11, \
               updated_at = unixepoch(), sync_status = 'pending'",
            rusqlite::params![
                draft_id,
                account_id,
                request.to.join(", "),
                request.cc.join(", "),
                request.bcc.join(", "),
                request.subject,
                request.body_html,
                request.in_reply_to,
                thread_id,
                request.from,
                mime_base64url,
            ],
        )
        .map_err(|e| ActionError::db(format!("draft persist: {e}")))?;

        // Transition to 'sending' — same state-machine validation as
        // mark_draft_sending(): rejects already-sent/sending drafts.
        let rows = conn
            .execute(
                "UPDATE local_drafts SET sync_status = 'sending' \
                 WHERE id = ?1 AND sync_status IN ('pending', 'synced', 'failed')",
                rusqlite::params![draft_id],
            )
            .map_err(|e| ActionError::db(format!("mark sending: {e}")))?;
        if rows == 0 {
            return Err(ActionError::invalid_state(format!(
                "Draft {draft_id} not found or already sending/sent"
            )));
        }

        Ok(mime_base64url)
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r);

    let mime_base64url = match local_result {
        Ok(mime) => mime,
        Err(e) => return ActionOutcome::Failed { error: e },
    };

    // 2. Provider dispatch
    let provider = match create_provider(&ctx.db, &account_id_outer, ctx.encryption_key).await {
        Ok(p) => p,
        Err(e) => {
            log::warn!("Send failed (provider create): {e}");
            let _ = mark_draft_failed(&ctx.db, draft_id_outer).await;
            return ActionOutcome::Failed {
                error: ActionError::remote(e),
            };
        }
    };

    let provider_ctx = ProviderCtx {
        account_id: &account_id_outer,
        db: &ctx.db,
        body_store: &ctx.body_store,
        inline_images: &ctx.inline_images,
        search: &ctx.search,
        progress: &NoopProgressReporter,
    };

    // Mentions are empty until @-autocomplete is built
    match provider
        .send_email(
            &provider_ctx,
            &mime_base64url,
            thread_id_outer.as_deref(),
            &[],
        )
        .await
    {
        Ok(sent_message_id) => {
            let _ = mark_draft_sent(&ctx.db, draft_id_outer, sent_message_id).await;
            ActionOutcome::Success
        }
        Err(e) => {
            let msg = e.to_string();
            log::warn!("Send failed for {account_id_outer}: {msg}");
            let _ = mark_draft_failed(&ctx.db, draft_id_outer).await;
            ActionOutcome::Failed {
                error: ActionError::remote(msg),
            }
        }
    }
}

/// Delete a local draft. If it has a `remote_draft_id`, also deletes
/// the server-side draft (best-effort).
///
/// Forward-looking: no call site in Phase 2.3 (no auto-save yet).
/// Becomes useful when auto-save or outbox UI land.
pub async fn delete_draft(
    ctx: &ActionContext,
    account_id: &str,
    draft_id: &str,
) -> ActionOutcome {
    // 1. Look up remote_draft_id and delete locally in one spawn_blocking call
    let db = ctx.db.clone();
    let did = draft_id.to_string();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| ActionError::db(format!("db lock: {e}")))?;

        let remote_id: Option<String> = match conn.query_row(
            "SELECT remote_draft_id FROM local_drafts WHERE id = ?1",
            rusqlite::params![did],
            |row| row.get(0),
        ) {
            Ok(id) => id,
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(ActionError::db(format!("draft lookup: {e}"))),
        };

        conn.execute(
            "DELETE FROM local_drafts WHERE id = ?1",
            rusqlite::params![did],
        )
        .map_err(|e| ActionError::db(format!("draft delete: {e}")))?;

        Ok(remote_id)
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r);

    let remote_id = match local_result {
        Ok(id) => id,
        Err(e) => return ActionOutcome::Failed { error: e },
    };

    // 2. Provider delete (best-effort, only if remote_draft_id exists)
    if let Some(remote_draft_id) = remote_id {
        if let Ok(provider) = create_provider(&ctx.db, account_id, ctx.encryption_key).await {
            let provider_ctx = ProviderCtx {
                account_id,
                db: &ctx.db,
                body_store: &ctx.body_store,
                inline_images: &ctx.inline_images,
                search: &ctx.search,
                progress: &NoopProgressReporter,
            };
            if let Err(e) = provider
                .delete_draft(&provider_ctx, &remote_draft_id)
                .await
            {
                log::warn!("Remote draft delete failed for {account_id}/{draft_id}: {e}");
                // Don't return Failed — the local delete succeeded and that's
                // what matters. The orphaned server draft will be cleaned up by sync.
            }
        }
    }

    ActionOutcome::Success
}
