use bifrost_types::{AccountId, ContainerId, ContainerKind, ContainerStyle, ObjectId};
use common::typed_ids::LabelId;
use db::db::queries_extra::{LabelWriteRow, upsert_labels};
use types::{LabelKind, MailProviderKind};

use super::context::ActionContext;
use super::dispatch_target::engine_error_to_action_error;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::pending::enqueue_if_retryable_with_id;
use crate::bifrost::BifrostProviderKind;
use crate::bifrost::resident::ResidentActionAccount;
use db::db::WriteTarget;
use db::db::queries_extra::{PendingLabelIntent, PendingLabelIntentOp};

/// Captured shape of a local-step upsert. The dispatcher needs the parsed
/// label kind (for the provider call), the intent list as actually written
/// (Graph importance expands to two intents), and the generation snapshot
/// the upsert captured (so a later attach / clear keys on the same row).
pub(crate) struct LocalLabelStep {
    label_kind: LabelKind,
    intents: Vec<(String, PendingLabelIntentOp)>,
    generation_seen: i64,
}

/// Local DB mutation for add-label: validate label exists, then write pending
/// user intent. Provider truth is updated only after provider success or sync.
pub(crate) async fn add_label_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> Result<LocalLabelStep, ActionError> {
    label_local_step(
        ctx,
        account_id,
        thread_id,
        label_id,
        PendingLabelIntentOp::Add,
    )
    .await
}

/// Local DB mutation for remove-label: same as `add_label_local` but
/// records a `Remove` intent. Provider truth is updated only after the
/// provider call succeeds or sync converges.
pub(crate) async fn remove_label_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> Result<LocalLabelStep, ActionError> {
    label_local_step(
        ctx,
        account_id,
        thread_id,
        label_id,
        PendingLabelIntentOp::Remove,
    )
    .await
}

async fn label_local_step(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
    op: PendingLabelIntentOp,
) -> Result<LocalLabelStep, ActionError> {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let lid = label_id.as_str().to_string();
    db.with_write_mapped(
        move |conn| {
            let label_kind =
                label_kind_for_account_sync(conn, &aid, &lid).map_err(ActionError::db)?;

            let exists = db::db::queries_extra::action_helpers::label_exists_sync(
                &conn.as_read(),
                &lid,
                &aid,
            )
            .map_err(|e| ActionError::db(format!("label lookup: {e}")))?;
            if !exists {
                ensure_typed_tag_label(conn, &aid, &label_kind)
                    .map_err(ActionError::db)?
                    .ok_or_else(|| ActionError::not_found("label not found for this account"))?;
            }

            if matches!(op, PendingLabelIntentOp::Add)
                && let LabelKind::GraphImportance(level) = label_kind
            {
                let opposite = LabelKind::graph_importance(level.opposite());
                ensure_typed_tag_label(conn, &aid, &opposite).map_err(ActionError::db)?;
            }
            let intents = label_intents(&lid, &label_kind, op);
            let generation_seen = upsert_pending_intents_sync(conn, &aid, &tid, &intents, None)
                .map_err(ActionError::db)?;

            Ok(LocalLabelStep {
                label_kind,
                intents,
                generation_seen,
            })
        },
        ActionError::db,
    )
    .await
}

fn label_intents(
    label_id: &str,
    label_kind: &LabelKind,
    op: PendingLabelIntentOp,
) -> Vec<(String, PendingLabelIntentOp)> {
    match (op, label_kind) {
        (PendingLabelIntentOp::Add, LabelKind::GraphImportance(level)) => vec![
            (
                level.opposite().label_id().to_string(),
                PendingLabelIntentOp::Remove,
            ),
            (label_id.to_string(), PendingLabelIntentOp::Add),
        ],
        _ => vec![(label_id.to_string(), op)],
    }
}

fn upsert_pending_intents_sync(
    conn: &impl WriteTarget,
    account_id: &str,
    thread_id: &str,
    intents: &[(String, PendingLabelIntentOp)],
    action_id: Option<&str>,
) -> Result<i64, String> {
    db::db::queries_extra::upsert_pending_thread_label_intents(
        conn,
        account_id,
        thread_id,
        intents
            .iter()
            .map(|(label_id, op)| PendingLabelIntent { label_id, op: *op }),
        action_id,
    )
}

async fn attach_pending_action_id(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    intents: Vec<(String, PendingLabelIntentOp)>,
    generation_seen: i64,
    action_id: String,
) {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    if let Err(e) = db
        .with_write(move |conn| {
            db::db::queries_extra::attach_action_id_to_pending_thread_label_intents(
                conn,
                &aid,
                &tid,
                intents
                    .iter()
                    .map(|(label_id, op)| PendingLabelIntent { label_id, op: *op }),
                generation_seen,
                &action_id,
            )
        })
        .await
    {
        log::warn!("[actions] attach pending label intent action id failed: {e}");
    }
}

async fn clear_pending_intents_immediate(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    intents: Vec<(String, PendingLabelIntentOp)>,
    generation_seen: i64,
) {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    if let Err(e) = db
        .with_write(move |conn| {
            db::db::queries_extra::delete_pending_thread_label_intents_for_labels(
                conn,
                &aid,
                &tid,
                intents
                    .iter()
                    .map(|(label_id, op)| PendingLabelIntent { label_id, op: *op }),
                generation_seen,
            )
        })
        .await
    {
        log::warn!("[actions] clear pending label intent on permanent fail: {e}");
    }
}

async fn confirm_provider_intents(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    intents: Vec<(String, PendingLabelIntentOp)>,
) -> Result<(), ActionError> {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    db.with_write_mapped(
        move |conn| {
            let tx = conn
                .transaction()
                .map_err(|e| ActionError::db(format!("begin confirm tx: {e}")))?;
            db::db::queries_extra::confirmed_provider_label_intents(
                &tx,
                &aid,
                &tid,
                intents
                    .iter()
                    .map(|(label_id, op)| PendingLabelIntent { label_id, op: *op }),
            )
            .map_err(ActionError::db)?;
            tx.commit()
                .map_err(|e| ActionError::db(format!("commit confirm tx: {e}")))
        },
        ActionError::db,
    )
    .await
}

fn label_kind_for_account_sync(
    conn: &impl WriteTarget,
    account_id: &str,
    label_id: &str,
) -> Result<LabelKind, String> {
    let provider = conn
        .query_row(
            "SELECT provider FROM accounts WHERE id = ?1",
            rusqlite::params![account_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(|e| format!("get account provider: {e}"))?;
    let provider = MailProviderKind::parse(&provider)?;
    LabelKind::parse(label_id, provider)
}

fn ensure_typed_tag_label(
    conn: &impl WriteTarget,
    account_id: &str,
    label: &LabelKind,
) -> Result<Option<()>, String> {
    let Some((name, sort_order, is_undeletable)) = label_write_metadata(label) else {
        return Ok(None);
    };
    let label_id = label.storage_id();

    // ON CONFLICT with OR semantics on is_undeletable repairs a pre-existing
    // row that was synced before the invariant landed. This matches
    // the OR rule in `upsert_labels` so the invariant from redesign.md
    // "is_undeletable" holds regardless of writer.
    conn.execute(
        "INSERT INTO labels \
         (id, account_id, name, sort_order, is_undeletable) \
         VALUES (?1, ?2, ?3, COALESCE(?4, 0), ?5) \
         ON CONFLICT(account_id, id) DO UPDATE SET \
           is_undeletable = (labels.is_undeletable OR excluded.is_undeletable)",
        rusqlite::params![label_id, account_id, name, sort_order, is_undeletable],
    )
    .map(|_| Some(()))
    .map_err(|e| format!("ensure typed tag label: {e}"))
}

fn label_write_metadata(label: &LabelKind) -> Option<(String, Option<i64>, bool)> {
    match label {
        LabelKind::GmailUser(_) => None,
        LabelKind::GraphCategory(category) => Some((category.as_str().to_string(), None, false)),
        LabelKind::GraphImportance(level) => Some((
            level.display_name().to_string(),
            Some(level.sort_order()),
            true,
        )),
        LabelKind::JmapKeyword(keyword) | LabelKind::ImapKeyword(keyword) => {
            Some((keyword.as_str().to_string(), None, false))
        }
    }
}

fn intent_op(add: bool) -> PendingLabelIntentOp {
    if add {
        PendingLabelIntentOp::Add
    } else {
        PendingLabelIntentOp::Remove
    }
}

/// Apply or remove a single label on a thread through the resident bifrost
/// engine, preserving the optimistic-intent lifecycle: the local step writes
/// the pending thread-label intent, the engine dispatch runs, then on success
/// the intent is confirmed (overlay cleared, truth written), on a retryable
/// failure it is enqueued and the pending action id attached, and on a
/// permanent failure it is cleared immediately (rather than waiting for the
/// 48h stale sweep).
///
/// `action_account == None` is the degraded path (no resident engine handle):
/// the local write lands and a retryable pending row is enqueued, exactly as
/// the legacy provider-construction-failure path behaved.
pub(crate) async fn dispatch_label_via_engine(
    ctx: &ActionContext,
    action_account: Option<&ResidentActionAccount>,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
    add: bool,
) -> ActionOutcome {
    let (op_type, mlog_name) = if add {
        ("addLabel", "add_label")
    } else {
        ("removeLabel", "remove_label")
    };
    let mlog = MutationLog::begin(mlog_name, account_id, thread_id);
    let params_json = serde_json::json!({"labelId": label_id.as_str()}).to_string();

    let local = match label_local_step(ctx, account_id, thread_id, label_id, intent_op(add)).await {
        Ok(local) => local,
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };
    let LocalLabelStep {
        label_kind,
        intents,
        generation_seen,
    } = local;

    let outcome = match action_account {
        None => ActionOutcome::LocalOnly {
            reason: ActionError::remote("resident engine unavailable"),
            retryable: true,
        },
        Some(account) => {
            match super::dispatch_target::resolve_thread_messages(
                ctx,
                account_id,
                thread_id,
                account.provider,
            )
            .await
            {
                Err(e) => ActionOutcome::LocalOnly {
                    retryable: e.is_retryable(),
                    reason: e,
                },
                Ok(ids) => match super::dispatch_target::dispatch_label_engine(
                    account,
                    account_id,
                    &label_kind,
                    add,
                    ids,
                )
                .await
                {
                    Ok(()) => {
                        match confirm_provider_intents(ctx, account_id, thread_id, intents.clone())
                            .await
                        {
                            Ok(()) => ActionOutcome::Success,
                            Err(error) => ActionOutcome::LocalOnly {
                                reason: error,
                                retryable: true,
                            },
                        }
                    }
                    Err(e) => ActionOutcome::LocalOnly {
                        retryable: e.is_retryable(),
                        reason: e,
                    },
                },
            }
        }
    };

    finalize_dispatch_outcome(
        ctx,
        account_id,
        thread_id,
        op_type,
        &params_json,
        intents,
        generation_seen,
        &outcome,
        true,
    )
    .await;
    mlog.emit(&outcome);
    outcome
}

/// Per-member engine dispatch for a composite label-group write. Reuses the
/// group-captured `generation_seen` and the group-resolved message `ids`, and
/// does NOT enqueue a per-member retry row (the composite enqueues one
/// composite-typed row covering the failed members - the contract on
/// `label_group::DispatchKind`). Member intents are still confirmed on success
/// and cleared on a permanent failure.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch_member_label_via_engine(
    ctx: &ActionContext,
    action_account: &ResidentActionAccount,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
    add: bool,
    generation_seen: i64,
    ids: &[ObjectId],
) -> ActionOutcome {
    let (op_type, mlog_name) = if add {
        ("addLabel", "add_label")
    } else {
        ("removeLabel", "remove_label")
    };
    let mlog = MutationLog::begin(mlog_name, account_id, thread_id);
    let params_json = serde_json::json!({"labelId": label_id.as_str()}).to_string();

    let label_kind = match resolve_member_label_kind(ctx, account_id, label_id).await {
        Ok(label_kind) => label_kind,
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };
    let intents = label_intents(label_id.as_str(), &label_kind, intent_op(add));

    let outcome = match super::dispatch_target::dispatch_label_engine(
        action_account,
        account_id,
        &label_kind,
        add,
        ids.to_vec(),
    )
    .await
    {
        Ok(()) => match confirm_provider_intents(ctx, account_id, thread_id, intents.clone()).await
        {
            Ok(()) => ActionOutcome::Success,
            Err(error) => ActionOutcome::LocalOnly {
                reason: error,
                retryable: true,
            },
        },
        Err(e) => ActionOutcome::LocalOnly {
            retryable: e.is_retryable(),
            reason: e,
        },
    };

    finalize_dispatch_outcome(
        ctx,
        account_id,
        thread_id,
        op_type,
        &params_json,
        intents,
        generation_seen,
        &outcome,
        false,
    )
    .await;
    mlog.emit(&outcome);
    outcome
}

async fn resolve_member_label_kind(
    ctx: &ActionContext,
    account_id: &str,
    label_id: &LabelId,
) -> Result<LabelKind, ActionError> {
    let aid = account_id.to_string();
    let lid = label_id.as_str().to_string();
    ctx.write_db
        .with_write(move |conn| label_kind_for_account_sync(conn, &aid, &lid))
        .await
        .map_err(|e| ActionError::db(format!("label kind lookup: {e}")))
}

#[allow(clippy::too_many_arguments)]
async fn finalize_dispatch_outcome(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    operation_type: &str,
    params_json: &str,
    intents: Vec<(String, PendingLabelIntentOp)>,
    generation_seen: i64,
    outcome: &ActionOutcome,
    enqueue_pending: bool,
) {
    match outcome {
        ActionOutcome::LocalOnly {
            retryable: true, ..
        } => {
            if enqueue_pending
                && let Some(action_id) = enqueue_if_retryable_with_id(
                    ctx,
                    outcome,
                    account_id,
                    operation_type,
                    thread_id,
                    params_json,
                )
                .await
            {
                attach_pending_action_id(
                    ctx,
                    account_id,
                    thread_id,
                    intents,
                    generation_seen,
                    action_id,
                )
                .await;
            }
        }
        ActionOutcome::LocalOnly {
            retryable: false, ..
        } => {
            // Permanent failure: the action is over. Tear down the
            // optimistic intent now instead of leaving it for the
            // 48h stale-intent sweep.
            clear_pending_intents_immediate(ctx, account_id, thread_id, intents, generation_seen)
                .await;
        }
        ActionOutcome::Success | ActionOutcome::NoOp | ActionOutcome::Failed { .. } => {}
    }
}

// ── Label CRUD (B6b) ────────────────────────────────────────────────
//
// Provider-first, local best-effort, mirroring the folder CRUD handlers.
// Dispatch `ContainerKind::Label` through the engine's `container_*`
// primitives (with the B6-SQ `style` arg for create/recolor), then upsert
// or delete the local `labels` row. In practice only Gmail accepts a
// label-kind container_create/rename/delete; the folder-shaped providers
// return `Unsupported`, surfaced as a permanent failure.

/// Storage id for a freshly-created user label (glossary Identity
/// prefixing). Gmail user labels carry no prefix; keyword/category
/// providers prefix `kw:` / `cat:` (those create paths are `Unsupported`
/// on the wire today, so this only documents intent).
fn new_label_storage_id(provider: BifrostProviderKind, native: &str) -> String {
    match provider {
        BifrostProviderKind::Gmail => native.to_string(),
        BifrostProviderKind::Graph => format!("cat:{native}"),
        BifrostProviderKind::Jmap | BifrostProviderKind::Imap => format!("kw:{native}"),
    }
}

/// Provider-native id for a stored label storage id (strip the `kw:` /
/// `cat:` prefix; Gmail ids pass through unchanged).
fn native_label_id(storage_id: &str) -> String {
    storage_id
        .strip_prefix("kw:")
        .or_else(|| storage_id.strip_prefix("cat:"))
        .unwrap_or(storage_id)
        .to_string()
}

fn container_style(style: Option<(&str, &str)>) -> Option<ContainerStyle> {
    style.map(|(bg, fg)| ContainerStyle::new(bg, fg))
}

fn no_resident_label() -> ActionOutcome {
    ActionOutcome::LocalOnly {
        reason: ActionError::remote("resident engine unavailable"),
        retryable: true,
    }
}

async fn upsert_local_label(
    ctx: &ActionContext,
    account_id: &str,
    storage_id: &str,
    name: &str,
    style: Option<(&str, &str)>,
) {
    let (server_color_bg, server_color_fg) = match style {
        Some((bg, fg)) => (Some(bg.to_string()), Some(fg.to_string())),
        None => (None, None),
    };
    let row = LabelWriteRow {
        id: storage_id.to_string(),
        account_id: account_id.to_string(),
        name: name.to_string(),
        visible: None,
        sort_order: None,
        server_color_bg,
        server_color_fg,
        user_color_bg: None,
        user_color_fg: None,
        is_undeletable: false,
    };
    if let Err(error) = ctx
        .write_db
        .with_write(move |conn| {
            let tx = conn
                .transaction()
                .map_err(|e| format!("begin label upsert tx: {e}"))?;
            upsert_labels(&tx, &[row])?;
            tx.commit().map_err(|e| format!("commit label upsert: {e}"))
        })
        .await
    {
        log::warn!("label CRUD local upsert failed (provider succeeded): {error}");
    }
}

/// Create a label on the provider, then upsert it locally.
pub(crate) async fn create_label(
    ctx: &ActionContext,
    action_account: Option<&ResidentActionAccount>,
    account_id: &str,
    name: &str,
    style: Option<(&str, &str)>,
) -> (ActionOutcome, Option<String>) {
    let mlog = MutationLog::begin("create_label", account_id, "(pending)");
    let Some(account) = action_account else {
        let outcome = no_resident_label();
        mlog.emit(&outcome);
        return (outcome, None);
    };

    let new_native = match account
        .engine
        .container_create(
            &AccountId(account_id.to_string()),
            ContainerKind::Label,
            name.to_string(),
            None,
            container_style(style),
        )
        .await
    {
        Ok(id) => id.0,
        Err(error) => {
            let outcome = ActionOutcome::Failed {
                error: engine_error_to_action_error(error),
            };
            mlog.emit(&outcome);
            return (outcome, None);
        }
    };

    let storage_id = new_label_storage_id(account.provider, &new_native);
    upsert_local_label(ctx, account_id, &storage_id, name, style).await;
    let outcome = ActionOutcome::Success;
    mlog.emit(&outcome);
    (outcome, Some(storage_id))
}

/// Rename (and optionally recolor) a label on the provider, then update
/// the local row.
pub(crate) async fn rename_label(
    ctx: &ActionContext,
    action_account: Option<&ResidentActionAccount>,
    account_id: &str,
    label_id: &LabelId,
    new_name: &str,
    style: Option<(&str, &str)>,
) -> ActionOutcome {
    let mlog = MutationLog::begin("rename_label", account_id, label_id.as_str());
    let Some(account) = action_account else {
        let outcome = no_resident_label();
        mlog.emit(&outcome);
        return outcome;
    };

    let native = ContainerId(native_label_id(label_id.as_str()));
    if let Err(error) = account
        .engine
        .container_rename(
            &AccountId(account_id.to_string()),
            native,
            new_name.to_string(),
            container_style(style),
        )
        .await
    {
        let outcome = ActionOutcome::Failed {
            error: engine_error_to_action_error(error),
        };
        mlog.emit(&outcome);
        return outcome;
    }

    upsert_local_label(ctx, account_id, label_id.as_str(), new_name, style).await;
    let outcome = ActionOutcome::Success;
    mlog.emit(&outcome);
    outcome
}

/// Recolor a label: a rename to its current name carrying the new style
/// (the engine's recolor path rides `container_rename`'s `style` arg).
pub(crate) async fn recolor_label(
    ctx: &ActionContext,
    action_account: Option<&ResidentActionAccount>,
    account_id: &str,
    label_id: &LabelId,
    name: &str,
    style: (&str, &str),
) -> ActionOutcome {
    rename_label(ctx, action_account, account_id, label_id, name, Some(style)).await
}

/// Delete a label on the provider, then remove the local rows.
pub(crate) async fn delete_label(
    ctx: &ActionContext,
    action_account: Option<&ResidentActionAccount>,
    account_id: &str,
    label_id: &LabelId,
) -> ActionOutcome {
    let mlog = MutationLog::begin("delete_label", account_id, label_id.as_str());
    let Some(account) = action_account else {
        let outcome = no_resident_label();
        mlog.emit(&outcome);
        return outcome;
    };

    let native = ContainerId(native_label_id(label_id.as_str()));
    if let Err(error) = account
        .engine
        .container_delete(&AccountId(account_id.to_string()), native)
        .await
    {
        let outcome = ActionOutcome::Failed {
            error: engine_error_to_action_error(error),
        };
        mlog.emit(&outcome);
        return outcome;
    }

    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let lid = label_id.as_str().to_string();
    if let Err(error) = db
        .with_write(move |conn| {
            let tx = conn
                .transaction()
                .map_err(|e| format!("begin label delete tx: {e}"))?;
            tx.execute(
                "DELETE FROM thread_labels WHERE account_id = ?1 AND label_id = ?2",
                rusqlite::params![aid, lid],
            )
            .map_err(|e| format!("delete thread_labels: {e}"))?;
            tx.execute(
                "DELETE FROM labels WHERE account_id = ?1 AND id = ?2",
                rusqlite::params![aid, lid],
            )
            .map_err(|e| format!("delete label: {e}"))?;
            tx.commit().map_err(|e| format!("commit label delete: {e}"))
        })
        .await
    {
        log::warn!("delete_label local delete failed (provider succeeded): {error}");
    }

    let outcome = ActionOutcome::Success;
    mlog.emit(&outcome);
    outcome
}
