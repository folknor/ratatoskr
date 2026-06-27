use super::context::ActionContext;
use super::dispatch_target::{
    dispatch_send_intent_mark, engine_error_to_action_error, resolve_message_object_id,
};
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome, RemoteFailureKind};
use crate::send::{
    SendIntent, SendRequest, delete_local_draft, mark_draft_failed, mark_send_intent_local,
    to_bifrost_send_request,
};
use crate::sync::SyncRuntime;
use bifrost_types::{AccountId, DraftHandle, ObjectId};
use std::time::SystemTime;

/// Send an email: build MIME, persist draft, dispatch to provider.
///
/// On success, the `local_drafts` row is deleted - the sent message
/// will arrive through provider sync as a thread in the Sent folder,
/// so the local row has no further purpose. Returns plain
/// `ActionOutcome::Success`.
///
/// On any failure (MIME build, DB, or provider), returns `Failed` and
/// marks the draft as `'failed'` if it was persisted. `LocalOnly` is
/// not used for send - the desired outcome is delivery, not local
/// persistence.
pub async fn send_email(
    ctx: &ActionContext,
    sync_runtime: Option<&SyncRuntime>,
    request: SendRequest,
) -> ActionOutcome {
    send_email_inner(ctx, sync_runtime, request, None).await
}

pub async fn send_scheduled(
    ctx: &ActionContext,
    sync_runtime: Option<&SyncRuntime>,
    request: SendRequest,
    scheduled_at: SystemTime,
) -> ActionOutcome {
    send_email_inner(ctx, sync_runtime, request, Some(scheduled_at)).await
}

async fn send_email_inner(
    ctx: &ActionContext,
    sync_runtime: Option<&SyncRuntime>,
    request: SendRequest,
    scheduled_at: Option<SystemTime>,
) -> ActionOutcome {
    let mut mlog = MutationLog::begin("send_email", &request.account_id, &request.draft_id);

    // 1. Persist draft and transition to sending in one write.
    //
    //    Both `db_save_local_draft` and `mark_draft_sending` are async helpers
    //    that take `&ReadDbState`. Inside spawn_blocking we already hold the Mutex
    //    lock, so we inline the equivalent SQL rather than calling the async
    //    helpers. The validation logic is identical.
    let db = ctx.write_db.clone();
    let draft_id = request.draft_id.clone();
    let account_id = request.account_id.clone();
    let thread_id = request.thread_id.clone();
    let source_message_id = request.source_message_id.clone();
    let send_intent = request.intent;
    // Clone for use after the spawn_blocking closure moves the originals.
    let draft_id_outer = draft_id.clone();
    let account_id_outer = account_id.clone();
    let source_message_id_outer = source_message_id.clone();
    // Compute the persisted draft columns up front as owned locals so the
    // write closure captures only these (not `request`), leaving `request`
    // live for the engine dispatch and scheduled-send bookkeeping below.
    let to_joined = request.to.join(", ");
    let cc_joined = request.cc.join(", ");
    let bcc_joined = request.bcc.join(", ");
    let subject = request.subject.clone();
    let body_html = request.body_html.clone();
    let in_reply_to = request.in_reply_to.clone();
    let from = request.from.clone();
    let attachment_metadata = match serde_json::to_string(
        &request
            .attachments
            .iter()
            .map(|att| {
                serde_json::json!({
                    "filename": att.filename,
                    "mime_type": att.mime_type,
                    "content_id": att.content_id,
                    "bytes": att.data.len()
                })
            })
            .collect::<Vec<_>>(),
    ) {
        Ok(metadata) => metadata,
        Err(e) => {
            let outcome = ActionOutcome::Failed {
                error: ActionError::build(format!("serialize attachments: {e}")),
            };
            mlog.emit(&outcome);
            return outcome;
        }
    };

    let local_result = db
        .with_write_mapped(
            move |conn| {
                db::db::queries_extra::draft_lifecycle::persist_draft_pending_sync(
                    conn,
                    &draft_id,
                    &account_id,
                    &to_joined,
                    &cc_joined,
                    &bcc_joined,
                    subject.as_deref(),
                    &body_html,
                    in_reply_to.as_deref(),
                    thread_id.as_deref(),
                    &from,
                    &attachment_metadata,
                )
                .map_err(ActionError::db)?;

                let transitioned = db::db::queries_extra::draft_lifecycle::mark_draft_sending_sync(
                    conn, &draft_id,
                )
                .map_err(ActionError::db)?;
                if !transitioned {
                    return Err(ActionError::invalid_state(format!(
                        "Draft {draft_id} not found or already sending/sent"
                    )));
                }

                Ok(())
            },
            ActionError::db,
        )
        .await;

    match local_result {
        Ok(()) => {}
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };

    let Some(sync_runtime) = sync_runtime else {
        let _ = mark_draft_failed(&ctx.write_db, draft_id_outer).await;
        let outcome = ActionOutcome::Failed {
            error: ActionError::remote_with_kind(
                RemoteFailureKind::Transient,
                "resident sync engine unavailable for send",
            ),
        };
        mlog.emit(&outcome);
        return outcome;
    };
    let action_account = match sync_runtime
        .resident_action_account(&account_id_outer)
        .await
        .map_err(ActionError::remote)
    {
        Ok(account) => account,
        Err(error) => {
            let _ = mark_draft_failed(&ctx.write_db, draft_id_outer).await;
            let outcome = ActionOutcome::Failed { error };
            mlog.emit(&outcome);
            return outcome;
        }
    };

    if scheduled_at.is_some() {
        let account = AccountId(account_id_outer.clone());
        let gate = match action_account.engine.account_capabilities(&account) {
            Ok(capabilities) => scheduled_send_gate(capabilities.pim_methods.scheduled_send),
            Err(error) => Err(engine_error_to_action_error(error)),
        };
        if let Err(error) = gate {
            let _ = mark_draft_failed(&ctx.write_db, draft_id_outer).await;
            let outcome = ActionOutcome::Failed { error };
            mlog.emit(&outcome);
            return outcome;
        }
    }

    let bifrost_request = match to_bifrost_send_request(&request, scheduled_at) {
        Ok(request) => request,
        Err(error) => {
            let _ = mark_draft_failed(&ctx.write_db, draft_id_outer).await;
            let outcome = ActionOutcome::Failed { error };
            mlog.emit(&outcome);
            return outcome;
        }
    };

    let account = AccountId(account_id_outer.clone());
    let outcome = match action_account
        .engine
        .send_message(&account, bifrost_request)
        .await
    {
        Ok(sent_message_id) => {
            mlog.set_remote_id(&sent_message_id.0);
            if let Some(scheduled_at) = scheduled_at
                && let Err(e) = mark_scheduled_delegated(
                    ctx,
                    &draft_id_outer,
                    &account_id_outer,
                    &request,
                    scheduled_at,
                    &sent_message_id.0,
                )
                .await
            {
                log::warn!("Scheduled-send local delegation record failed: {e}");
            }
            if send_intent != SendIntent::New {
                if let Some(source_message_id) = source_message_id_outer.as_deref() {
                    match resolve_message_object_id(
                        ctx,
                        &account_id_outer,
                        source_message_id,
                        action_account.provider,
                    )
                    .await
                    {
                        Ok(object_id) => {
                            if let Err(e) = dispatch_send_intent_mark(
                                &action_account,
                                &account_id_outer,
                                send_intent,
                                object_id,
                            )
                            .await
                            {
                                log::warn!("Provider send-intent writeback failed: {e}");
                            }
                        }
                        Err(e) => log::warn!("Provider send-intent id resolution failed: {e}"),
                    }
                }
                if let Err(e) = mark_send_intent_local(
                    &ctx.write_db,
                    account_id_outer.clone(),
                    source_message_id_outer,
                    send_intent,
                )
                .await
                {
                    log::warn!("Local send-intent writeback failed: {e}");
                }
            }
            let _ = delete_local_draft(&ctx.write_db, draft_id_outer).await;
            ActionOutcome::Success
        }
        Err(e) => {
            let _ = mark_draft_failed(&ctx.write_db, draft_id_outer).await;
            if scheduled_at.is_some() {
                let _ = mark_scheduled_failed(ctx, &request.draft_id, &e.to_string()).await;
            }
            ActionOutcome::Failed {
                error: engine_error_to_action_error(e),
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
pub async fn delete_draft(
    ctx: &ActionContext,
    sync_runtime: Option<&SyncRuntime>,
    account_id: &str,
    draft_id: &str,
) -> ActionOutcome {
    let mlog = MutationLog::begin("delete_draft", account_id, draft_id);

    // 1. Look up remote_draft_id and delete locally in one spawn_blocking call
    let db = ctx.write_db.clone();
    let did = draft_id.to_string();
    let local_result = db
        .with_write_mapped(
            move |conn| {
                let remote_id =
                    db::db::queries_extra::draft_lifecycle::get_remote_draft_id_sync(conn, &did)
                        .map_err(ActionError::db)?;

                db::db::queries_extra::draft_lifecycle::delete_draft_sync(conn, &did)
                    .map_err(ActionError::db)?;

                Ok(remote_id)
            },
            ActionError::db,
        )
        .await;

    let remote_id = match local_result {
        Ok(id) => id,
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };

    // 2. Provider delete (best-effort, only if remote_draft_id exists)
    if let Some(remote_draft_id) = remote_id
        && let Some(sync_runtime) = sync_runtime
    {
        let remote = async {
            let action_account = sync_runtime
                .resident_action_account(account_id)
                .await
                .map_err(ActionError::remote)?;
            action_account
                .engine
                .draft_discard(
                    &AccountId(account_id.to_string()),
                    DraftHandle(remote_draft_id.clone()),
                )
                .await
                .map_err(engine_error_to_action_error)
        }
        .await;
        // Best-effort: don't fail if remote delete fails.
        // The orphaned server draft will be cleaned up by sync.
        if let Err(e) = remote {
            log::warn!("Remote draft delete failed for {account_id}/{draft_id}: {e}");
        }
    }

    let outcome = ActionOutcome::Success;
    mlog.emit(&outcome);
    outcome
}

async fn mark_scheduled_delegated(
    ctx: &ActionContext,
    id: &str,
    account_id: &str,
    request: &SendRequest,
    scheduled_at: SystemTime,
    remote_message_id: &str,
) -> Result<(), String> {
    let id = id.to_string();
    let account_id = account_id.to_string();
    let to = request.to.join(", ");
    let cc = request.cc.join(", ");
    let bcc = request.bcc.join(", ");
    let subject = request.subject.clone();
    let body_html = request.body_html.clone();
    let reply_to_message_id = request.in_reply_to.clone();
    let thread_id = request.thread_id.clone();
    let from_email = request.from.clone();
    let attachment_paths = serde_json::to_string(
        &request
            .attachments
            .iter()
            .map(|att| {
                serde_json::json!({
                    "filename": att.filename,
                    "mime_type": att.mime_type,
                    "content_id": att.content_id,
                    "bytes": att.data.len()
                })
            })
            .collect::<Vec<_>>(),
    )
    .map_err(|e| format!("serialize scheduled attachments: {e}"))?;
    let scheduled_at = i64::try_from(
        scheduled_at
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|e| format!("scheduled time before epoch: {e}"))?
            .as_secs(),
    )
    .map_err(|e| format!("scheduled time out of range: {e}"))?;
    let remote_message_id = remote_message_id.to_string();
    ctx.write_db
        .with_write(move |conn| {
            conn.execute(
                "INSERT INTO scheduled_emails \
                 (id, account_id, to_addresses, cc_addresses, bcc_addresses, subject, \
                  body_html, reply_to_message_id, thread_id, scheduled_at, attachment_paths, \
                  status, delegation, remote_message_id, from_email) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, \
                         'delegated', 'server', ?12, ?13) \
                 ON CONFLICT(id) DO UPDATE SET \
                   account_id = ?2, to_addresses = ?3, cc_addresses = ?4, bcc_addresses = ?5, \
                   subject = ?6, body_html = ?7, reply_to_message_id = ?8, thread_id = ?9, \
                   scheduled_at = ?10, attachment_paths = ?11, status = 'delegated', \
                   delegation = 'server', remote_message_id = ?12, from_email = ?13, \
                   error_message = NULL",
                rusqlite::params![
                    id,
                    account_id,
                    to,
                    cc,
                    bcc,
                    subject,
                    body_html,
                    reply_to_message_id,
                    thread_id,
                    scheduled_at,
                    attachment_paths,
                    remote_message_id,
                    from_email,
                ],
            )
            .map_err(|e| format!("mark scheduled delegated: {e}"))?;
            Ok(())
        })
        .await
}

async fn mark_scheduled_failed(ctx: &ActionContext, id: &str, error: &str) -> Result<(), String> {
    let id = id.to_string();
    let error = error.to_string();
    ctx.write_db
        .with_write(move |conn| {
            conn.execute(
                "UPDATE scheduled_emails \
                 SET status = 'failed', error_message = ?1, retry_count = retry_count + 1 \
                 WHERE id = ?2",
                rusqlite::params![error, id],
            )
            .map_err(|e| format!("mark scheduled failed: {e}"))?;
            Ok(())
        })
        .await
}

pub async fn cancel_scheduled_send(
    ctx: &ActionContext,
    sync_runtime: Option<&SyncRuntime>,
    account_id: &str,
    remote_message_id: &str,
) -> ActionOutcome {
    scheduled_send_lifecycle(ctx, sync_runtime, account_id, remote_message_id, None).await
}

pub async fn reschedule_send(
    ctx: &ActionContext,
    sync_runtime: Option<&SyncRuntime>,
    account_id: &str,
    remote_message_id: &str,
    scheduled_at: SystemTime,
) -> ActionOutcome {
    scheduled_send_lifecycle(
        ctx,
        sync_runtime,
        account_id,
        remote_message_id,
        Some(scheduled_at),
    )
    .await
}

async fn scheduled_send_lifecycle(
    ctx: &ActionContext,
    sync_runtime: Option<&SyncRuntime>,
    account_id: &str,
    remote_message_id: &str,
    scheduled_at: Option<SystemTime>,
) -> ActionOutcome {
    let Some(sync_runtime) = sync_runtime else {
        return ActionOutcome::Failed {
            error: ActionError::remote_with_kind(
                RemoteFailureKind::Transient,
                "resident sync engine unavailable for scheduled send",
            ),
        };
    };
    let action_account = match sync_runtime.resident_action_account(account_id).await {
        Ok(account) => account,
        Err(error) => {
            return ActionOutcome::Failed {
                error: ActionError::remote(error),
            };
        }
    };
    let account = AccountId(account_id.to_string());
    let gate = match action_account.engine.account_capabilities(&account) {
        Ok(capabilities) => scheduled_send_gate(capabilities.pim_methods.scheduled_send),
        Err(error) => Err(engine_error_to_action_error(error)),
    };
    if let Err(error) = gate {
        return ActionOutcome::Failed { error };
    }

    let remote = ObjectId(remote_message_id.to_string());
    let result = if let Some(scheduled_at) = scheduled_at {
        action_account
            .engine
            .reschedule_send(&account, remote, scheduled_at)
            .await
            .map(|new_id| Some(new_id.0))
    } else {
        action_account
            .engine
            .cancel_scheduled_send(&account, remote)
            .await
            .map(|()| None)
    };
    match result {
        Ok(new_remote_id) => {
            if let Err(error) =
                update_scheduled_status(ctx, remote_message_id, scheduled_at, new_remote_id).await
            {
                log::warn!("Scheduled-send local status update failed: {error}");
            }
            ActionOutcome::Success
        }
        Err(error) => ActionOutcome::Failed {
            error: engine_error_to_action_error(error),
        },
    }
}

/// Capability gate for scheduled send (A4). The provider's
/// `pim_methods.scheduled_send` flag is the single source of truth: a
/// `false` flag yields a user-safe `Unsupported`-class `ActionError`
/// before any wire round-trip. IMAP is statically `false` (FUTURERELEASE
/// is a per-connection EHLO truth ratatoskr treats as absent); Gmail
/// advertises as PIM-rich yet its REST API has no delayed-send lever, so
/// it too reports `false` - read the flag, never a hardcoded provider
/// allowlist.
fn scheduled_send_gate(scheduled_send_supported: bool) -> Result<(), ActionError> {
    if scheduled_send_supported {
        Ok(())
    } else {
        Err(ActionError::remote_with_kind(
            RemoteFailureKind::Permanent,
            "scheduled send is not supported for this account",
        ))
    }
}

async fn update_scheduled_status(
    ctx: &ActionContext,
    remote_message_id: &str,
    scheduled_at: Option<SystemTime>,
    new_remote_id: Option<String>,
) -> Result<(), String> {
    let remote_message_id = remote_message_id.to_string();
    let new_remote_id = new_remote_id.unwrap_or_else(|| remote_message_id.clone());
    let scheduled_at_unix = scheduled_at
        .map(|at| {
            at.duration_since(SystemTime::UNIX_EPOCH)
                .map_err(|e| format!("scheduled time before epoch: {e}"))
                .and_then(|duration| {
                    i64::try_from(duration.as_secs())
                        .map_err(|e| format!("scheduled time out of range: {e}"))
                })
        })
        .transpose()?;
    ctx.write_db
        .with_write(move |conn| {
            if let Some(scheduled_at) = scheduled_at_unix {
                conn.execute(
                    "UPDATE scheduled_emails \
                     SET scheduled_at = ?1, remote_message_id = ?2, status = 'delegated' \
                     WHERE remote_message_id = ?3",
                    rusqlite::params![scheduled_at, new_remote_id, remote_message_id],
                )
            } else {
                conn.execute(
                    "UPDATE scheduled_emails SET status = 'canceled' WHERE remote_message_id = ?1",
                    rusqlite::params![remote_message_id],
                )
            }
            .map_err(|e| format!("update scheduled status: {e}"))?;
            Ok(())
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduled_send_capability_gate() {
        // A scheduled-send-incapable provider (IMAP, Gmail-in-practice)
        // is rejected at the gate with a permanent, user-safe error and
        // NO wire round-trip is attempted.
        let rejected = scheduled_send_gate(false);
        let error = rejected.expect_err("incapable provider must be rejected");
        assert!(
            matches!(
                error,
                ActionError::Remote {
                    kind: RemoteFailureKind::Permanent,
                    ..
                }
            ),
            "expected permanent remote error, got {error:?}"
        );

        // A capable provider (Graph/JMAP) passes the gate; the request
        // then stamps `SendRequest::scheduled` (covered by the
        // `to_bifrost_send_request` schedule round-trip test in
        // `crate::send`).
        assert!(scheduled_send_gate(true).is_ok());
    }
}
