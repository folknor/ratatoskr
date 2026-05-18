use common::typed_ids::FolderId;

use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::provider::create_provider_with_writer;
use db::progress::NoopProgressReporter;
use common::types::{ProviderCtx, ProviderFolderMutation};

/// Build a `ProviderCtx` from an `ActionContext` and account ID.
///
/// Shared across all folder operations. Also usable by other action
/// functions - existing ones can be refactored to use this in a cleanup pass.
fn build_provider_ctx<'a>(ctx: &'a ActionContext, account_id: &'a str) -> ProviderCtx<'a> {
    ProviderCtx {
        account_id,
        db: &ctx.db,
        progress: &NoopProgressReporter,
    }
}

/// Create a folder on the provider, then insert it into the local `folders` table.
///
/// Provider-first: the provider assigns the folder ID, path, and metadata.
/// The local DB is updated best-effort - if it fails, the action still returns
/// `Success` because the provider state is canonical and sync will reconcile.
///
/// **Limitation:** `Success` after a local DB failure means the caller cannot
/// rely on local state being current. The sidebar won't reflect the new folder
/// until the next sync. Phase 3 (structured outcomes) should introduce a
/// distinct outcome for "provider succeeded, local stale" so the caller can
/// trigger an immediate sync or nav refresh.
///
/// **IMAP:** Returns `Failed` - IMAP does not support folder creation via
/// the current `ProviderOps` implementation. UI must gate this for IMAP accounts.
///
/// Returns `(ActionOutcome, Option<ProviderFolderMutation>)` so the caller
/// has the provider-assigned metadata (e.g., to navigate to the new folder).
pub async fn create_folder(
    ctx: &ActionContext,
    account_id: &str,
    name: &str,
    parent_id: Option<&FolderId>,
    text_color: Option<&str>,
    bg_color: Option<&str>,
) -> (ActionOutcome, Option<ProviderFolderMutation>) {
    let mut mlog = MutationLog::begin("create_folder", account_id, "(pending)");

    // 1. Provider dispatch first - we need the provider-assigned ID
    let provider = match create_provider_with_writer(&ctx.db, &ctx.write_db, account_id, ctx.encryption_key).await {
        Ok(p) => p,
        Err(e) => {
            let outcome = ActionOutcome::Failed {
                error: ActionError::remote(e),
            };
            mlog.emit(&outcome);
            return (outcome, None);
        }
    };

    let provider_ctx = build_provider_ctx(ctx, account_id);

    let mutation = match provider
        .create_folder(
            &provider_ctx,
            name,
            parent_id,
            text_color,
            bg_color,
        )
        .await
    {
        Ok(m) => m,
        Err(e) => {
            let msg = e.to_string();
            let outcome = ActionOutcome::Failed {
                error: ActionError::remote(msg),
            };
            mlog.emit(&outcome);
            return (outcome, None);
        }
    };

    mlog.set_local_id(&mutation.id);
    mlog.set_remote_id(&mutation.id);

    // 2. Local DB - insert the new folder into folders (best-effort)
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let m = mutation.clone();
    let parent_id_for_db = parent_id.map(|id| id.as_str().to_string());
    let local_result = db
        .with_write_mapped(move |conn| {
            db::db::queries_extra::action_helpers::upsert_folder_from_mutation_sync(
                conn,
                &m.id,
                &aid,
                &m.name,
                Some(m.path.as_str()),
                m.special_use.as_deref(),
                parent_id_for_db.as_deref(),
            )
            .map_err(ActionError::db)?;
            Ok(())
        }, ActionError::db)
        .await;

    if let Err(e) = local_result {
        // Provider succeeded but local DB failed - unusual but possible.
        // The folder exists on the server; next sync will pick it up.
        log::warn!("create_folder local insert failed (provider succeeded): {e}");
    }

    let outcome = ActionOutcome::Success;
    mlog.emit(&outcome);
    (outcome, Some(mutation))
}

/// Rename a folder on the provider, then update the local `folders` row.
///
/// Provider-first, same pattern and limitations as `create_folder`.
/// All provider-returned metadata is persisted locally (name, type,
/// path and special_use). IMAP: returns `Failed` (not supported).
pub async fn rename_folder(
    ctx: &ActionContext,
    account_id: &str,
    folder_id: &FolderId,
    new_name: &str,
    text_color: Option<&str>,
    bg_color: Option<&str>,
) -> (ActionOutcome, Option<ProviderFolderMutation>) {
    let mlog = MutationLog::begin("rename_folder", account_id, folder_id.as_str());

    let provider = match create_provider_with_writer(&ctx.db, &ctx.write_db, account_id, ctx.encryption_key).await {
        Ok(p) => p,
        Err(e) => {
            let outcome = ActionOutcome::Failed {
                error: ActionError::remote(e),
            };
            mlog.emit(&outcome);
            return (outcome, None);
        }
    };

    let provider_ctx = build_provider_ctx(ctx, account_id);

    let mutation = match provider
        .rename_folder(
            &provider_ctx,
            folder_id,
            new_name,
            text_color,
            bg_color,
        )
        .await
    {
        Ok(m) => m,
        Err(e) => {
            let msg = e.to_string();
            let outcome = ActionOutcome::Failed {
                error: ActionError::remote(msg),
            };
            mlog.emit(&outcome);
            return (outcome, None);
        }
    };

    // Local DB update (best-effort)
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let fid = folder_id.as_str().to_string();
    let m = mutation.clone();
    let local_result = db
        .with_write_mapped(move |conn| {
            db::db::queries_extra::action_helpers::upsert_folder_from_mutation_sync(
                conn,
                &fid,
                &aid,
                &m.name,
                Some(m.path.as_str()),
                m.special_use.as_deref(),
                None, // parent folder is not changed in rename
            )
            .map_err(ActionError::db)?;
            Ok(())
        }, ActionError::db)
        .await;

    if let Err(e) = local_result {
        log::warn!("rename_folder local update failed (provider succeeded): {e}");
    }

    let outcome = ActionOutcome::Success;
    mlog.emit(&outcome);
    (outcome, Some(mutation))
}

/// Delete a folder on the provider, then remove it from the local DB.
///
/// Provider-first, same limitations as `create_folder` re: local DB failure.
/// `thread_folders` rows for this folder are explicitly cleaned up.
/// IMAP: returns `Failed` (not supported).
pub async fn delete_folder(
    ctx: &ActionContext,
    account_id: &str,
    folder_id: &FolderId,
) -> ActionOutcome {
    let mlog = MutationLog::begin("delete_folder", account_id, folder_id.as_str());

    let provider = match create_provider_with_writer(&ctx.db, &ctx.write_db, account_id, ctx.encryption_key).await {
        Ok(p) => p,
        Err(e) => {
            let outcome = ActionOutcome::Failed {
                error: ActionError::remote(e),
            };
            mlog.emit(&outcome);
            return outcome;
        }
    };

    let provider_ctx = build_provider_ctx(ctx, account_id);

    if let Err(e) = provider.delete_folder(&provider_ctx, folder_id).await {
        let msg = e.to_string();
        let outcome = ActionOutcome::Failed {
            error: ActionError::remote(msg),
        };
        mlog.emit(&outcome);
        return outcome;
    }

    // Provider succeeded - remove local rows (best-effort)
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let fid = folder_id.as_str().to_string();
    let local_result = db
        .with_write_mapped(move |conn| {
            db::db::queries_extra::action_helpers::delete_folder_sync(conn, &aid, &fid)
                .map_err(ActionError::db)?;
            Ok(())
        }, ActionError::db)
        .await;

    if let Err(e) = local_result {
        log::warn!("delete_folder local delete failed (provider succeeded): {e}");
    }

    let outcome = ActionOutcome::Success;
    mlog.emit(&outcome);
    outcome
}
