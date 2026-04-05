use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::provider::create_provider;
use crate::progress::NoopProgressReporter;
use crate::send::{SendRequest, build_mime_message_base64url, mark_draft_failed, mark_draft_sent};
use common::types::ProviderCtx;

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
    let mut mlog = MutationLog::begin("send_email", &request.account_id, &request.draft_id);

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
        let conn = conn
            .lock()
            .map_err(|e| ActionError::db(format!("db lock: {e}")))?;

        crate::db::queries_extra::draft_lifecycle::persist_draft_pending_sync(
            &conn,
            &draft_id,
            &account_id,
            &request.to.join(", "),
            &request.cc.join(", "),
            &request.bcc.join(", "),
            request.subject.as_deref(),
            &request.body_html,
            request.in_reply_to.as_deref(),
            thread_id.as_deref(),
            &request.from,
            &mime_base64url,
        )
        .map_err(ActionError::db)?;

        let transitioned =
            crate::db::queries_extra::draft_lifecycle::mark_draft_sending_sync(&conn, &draft_id)
                .map_err(ActionError::db)?;
        if !transitioned {
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
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };

    // 2. Provider dispatch
    let provider = match create_provider(&ctx.db, &account_id_outer, ctx.encryption_key).await {
        Ok(p) => p,
        Err(e) => {
            let _ = mark_draft_failed(&ctx.db, draft_id_outer).await;
            let outcome = ActionOutcome::Failed {
                error: ActionError::remote(e),
            };
            mlog.emit(&outcome);
            return outcome;
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

    let outcome = match provider
        .send_email(&provider_ctx, &mime_base64url, thread_id_outer.as_deref())
        .await
    {
        Ok(sent_message_id) => {
            mlog.set_remote_id(&sent_message_id);
            let _ = mark_draft_sent(&ctx.db, draft_id_outer, sent_message_id).await;
            ActionOutcome::Success
        }
        Err(e) => {
            let msg = e.to_string();
            let _ = mark_draft_failed(&ctx.db, draft_id_outer).await;
            ActionOutcome::Failed {
                error: ActionError::remote(msg),
            }
        }
    };
    mlog.emit(&outcome);
    outcome
}

/// Delete a local draft. If it has a `remote_draft_id`, also deletes
/// the server-side draft (best-effort).
///
/// Forward-looking: no call site in Phase 2.3 (no auto-save yet).
/// Becomes useful when auto-save or outbox UI land.
pub async fn delete_draft(ctx: &ActionContext, account_id: &str, draft_id: &str) -> ActionOutcome {
    let mlog = MutationLog::begin("delete_draft", account_id, draft_id);

    // 1. Look up remote_draft_id and delete locally in one spawn_blocking call
    let db = ctx.db.clone();
    let did = draft_id.to_string();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn
            .lock()
            .map_err(|e| ActionError::db(format!("db lock: {e}")))?;

        let remote_id =
            crate::db::queries_extra::draft_lifecycle::get_remote_draft_id_sync(&conn, &did)
                .map_err(ActionError::db)?;

        crate::db::queries_extra::draft_lifecycle::delete_draft_sync(&conn, &did)
            .map_err(ActionError::db)?;

        Ok(remote_id)
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r);

    let remote_id = match local_result {
        Ok(id) => id,
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
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
            // Best-effort: don't fail if remote delete fails.
            // The orphaned server draft will be cleaned up by sync.
            if let Err(e) = provider.delete_draft(&provider_ctx, &remote_draft_id).await {
                log::warn!("Remote draft delete failed for {account_id}/{draft_id}: {e}");
            }
        }
    }

    let outcome = ActionOutcome::Success;
    mlog.emit(&outcome);
    outcome
}
