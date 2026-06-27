//! Folder CRUD action handlers (B6b).
//!
//! Provider-first, local best-effort: each handler dispatches through the
//! resident bifrost engine's `container_*` primitives, then upserts/deletes
//! the local `folders` row. The provider-first ordering, the `MutationLog`,
//! and the outcome taxonomy are unchanged from the legacy `ProviderOps`
//! handlers - only the dispatch target moved (from
//! `create_provider(...).create_folder(...)` to
//! `action_account.engine.container_create(...)`).
//!
//! These are the service-side capability; the IPC/app wiring that calls
//! them from a user gesture is the named out-of-scope follow-up (see
//! docs/bifrost-migration.md). They are reachable today only through
//! the harness test-only request.

use std::collections::HashMap;

use bifrost_types::{AccountId, ContainerId, ContainerKind};
use common::typed_ids::FolderId;
use common::types::FolderKind;
use db::db::queries_extra::{FolderWriteRow, insert_folders_batch};

use super::context::ActionContext;
use super::dispatch_target::engine_error_to_action_error;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use crate::bifrost::BifrostProviderKind;
use crate::bifrost::resident::ResidentActionAccount;

/// Resolve a ratatoskr storage `FolderId` to the provider-native
/// `ContainerId` the engine addresses, via the cached folder map. Only
/// folders appear in the map, so a miss means "not a (known) folder".
fn native_container_id(
    folder_map: &HashMap<String, FolderKind>,
    storage_id: &str,
) -> Option<ContainerId> {
    folder_map.iter().find_map(|(native, kind)| {
        (kind.storage_id() == storage_id).then(|| ContainerId(native.clone()))
    })
}

/// Resolve a storage `FolderId` to its provider-native `ContainerId`,
/// refreshing the cached folder map on a miss before giving up.
///
/// A folder created earlier in the same session is persisted to the server
/// and to the local `folders` table, but the resident slot's cached
/// `folder_map` still holds only the attach-time snapshot - so a bare cached
/// lookup misses the freshly-created folder and the handler would strand the
/// mutation on a terminal not-found before reaching the provider. This mirrors
/// `dispatch_target::resolve_move_destination`: cache-hit returns immediately,
/// a miss triggers one `refresh_folder_map` re-fetch and re-lookup, and only a
/// still-absent target yields not-found. A refresh that itself fails maps to a
/// transient remote error so the caller does not classify a flaky re-fetch as a
/// permanent miss.
async fn resolve_container_with_refresh(
    account: &ResidentActionAccount,
    storage_id: &str,
    not_found_msg: impl FnOnce() -> String,
) -> Result<ContainerId, ActionError> {
    if let Some(native) = native_container_id(&account.folder_map, storage_id) {
        return Ok(native);
    }
    let fresh = account
        .refresh_folder_map()
        .await
        .map_err(|error| ActionError::remote(format!("refresh container map: {error}")))?;
    native_container_id(&fresh, storage_id).ok_or_else(|| ActionError::not_found(not_found_msg()))
}

/// Storage id for a freshly-created user folder (role `None`), per the
/// glossary Identity prefixing convention.
fn new_folder_storage_id(provider: BifrostProviderKind, native: &str) -> String {
    match provider {
        BifrostProviderKind::Graph => format!("graph-{native}"),
        BifrostProviderKind::Jmap => format!("jmap-{native}"),
        BifrostProviderKind::Imap => format!("folder-{native}"),
        BifrostProviderKind::Gmail => native.to_string(),
    }
}

/// Best-effort local upsert of a `folders` row after a provider mutation
/// succeeded. A failure here leaves the provider state canonical; the next
/// sync reconciles, so it only logs.
async fn upsert_local_folder(
    ctx: &ActionContext,
    provider: BifrostProviderKind,
    account_id: &str,
    storage_id: &str,
    native: &str,
    name: &str,
    parent_storage: Option<String>,
) {
    let is_imap = matches!(provider, BifrostProviderKind::Imap);
    let row = FolderWriteRow {
        id: storage_id.to_string(),
        account_id: account_id.to_string(),
        name: name.to_string(),
        visible: None,
        sort_order: None,
        imap_folder_path: is_imap.then(|| native.to_string()),
        imap_special_use: None,
        namespace_type: None,
        parent_id: parent_storage,
        right_read: None,
        right_add: None,
        right_remove: None,
        right_set_seen: None,
        right_set_keywords: None,
        right_create_child: None,
        right_rename: None,
        right_delete: None,
        right_submit: None,
        is_subscribed: None,
        is_undeletable: false,
    };
    if let Err(error) = ctx
        .write_db
        .with_write(move |conn| {
            let tx = conn
                .transaction()
                .map_err(|e| format!("begin folder upsert tx: {e}"))?;
            insert_folders_batch(&tx, &[row])?;
            tx.commit()
                .map_err(|e| format!("commit folder upsert: {e}"))
        })
        .await
    {
        log::warn!("create/rename_folder local upsert failed (provider succeeded): {error}");
    }
}

fn no_resident() -> ActionOutcome {
    ActionOutcome::LocalOnly {
        reason: ActionError::remote("resident engine unavailable"),
        retryable: true,
    }
}

/// Create a folder on the provider, then upsert it locally.
pub(crate) async fn create_folder(
    ctx: &ActionContext,
    action_account: Option<&ResidentActionAccount>,
    account_id: &str,
    name: &str,
    parent_id: Option<&FolderId>,
) -> (ActionOutcome, Option<String>) {
    let mlog = MutationLog::begin("create_folder", account_id, "(pending)");
    let Some(account) = action_account else {
        let outcome = no_resident();
        mlog.emit(&outcome);
        return (outcome, None);
    };

    let parent = match parent_id {
        Some(pid) => match resolve_container_with_refresh(account, pid.as_str(), || {
            format!("parent folder {} not found", pid.as_str())
        })
        .await
        {
            Ok(container) => Some(container),
            Err(error) => {
                let outcome = ActionOutcome::Failed { error };
                mlog.emit(&outcome);
                return (outcome, None);
            }
        },
        None => None,
    };

    let new_native = match account
        .engine
        .container_create(
            &AccountId(account_id.to_string()),
            ContainerKind::Folder,
            name.to_string(),
            parent,
            None,
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

    let storage_id = new_folder_storage_id(account.provider, &new_native);
    upsert_local_folder(
        ctx,
        account.provider,
        account_id,
        &storage_id,
        &new_native,
        name,
        parent_id.map(|p| p.as_str().to_string()),
    )
    .await;

    let outcome = ActionOutcome::Success;
    mlog.emit(&outcome);
    (outcome, Some(storage_id))
}

/// Rename a folder on the provider, then update the local row.
pub(crate) async fn rename_folder(
    ctx: &ActionContext,
    action_account: Option<&ResidentActionAccount>,
    account_id: &str,
    folder_id: &FolderId,
    new_name: &str,
) -> ActionOutcome {
    let mlog = MutationLog::begin("rename_folder", account_id, folder_id.as_str());
    let Some(account) = action_account else {
        let outcome = no_resident();
        mlog.emit(&outcome);
        return outcome;
    };

    let native = match resolve_container_with_refresh(account, folder_id.as_str(), || {
        format!("folder {} not found", folder_id.as_str())
    })
    .await
    {
        Ok(native) => native,
        Err(error) => {
            let outcome = ActionOutcome::Failed { error };
            mlog.emit(&outcome);
            return outcome;
        }
    };
    let native_id = native.0.clone();

    if let Err(error) = account
        .engine
        .container_rename(
            &AccountId(account_id.to_string()),
            native,
            new_name.to_string(),
            None,
        )
        .await
    {
        let outcome = ActionOutcome::Failed {
            error: engine_error_to_action_error(error),
        };
        mlog.emit(&outcome);
        return outcome;
    }

    upsert_local_folder(
        ctx,
        account.provider,
        account_id,
        folder_id.as_str(),
        &native_id,
        new_name,
        None,
    )
    .await;

    let outcome = ActionOutcome::Success;
    mlog.emit(&outcome);
    outcome
}

/// Move a folder under a new parent on the provider. Folder-kind only - a
/// label-kind id is rejected before dispatch (it is absent from the folder
/// map, which holds folders only).
pub(crate) async fn move_folder(
    ctx: &ActionContext,
    action_account: Option<&ResidentActionAccount>,
    account_id: &str,
    folder_id: &FolderId,
    new_parent_id: Option<&FolderId>,
) -> ActionOutcome {
    let mlog = MutationLog::begin("move_folder", account_id, folder_id.as_str());
    let Some(account) = action_account else {
        let outcome = no_resident();
        mlog.emit(&outcome);
        return outcome;
    };

    // A label-kind id (or an unknown id) is never in the folder map even after
    // a refresh, so this still rejects label-kind moves before dispatch; the
    // refresh only rescues a folder created since attach.
    let native = match resolve_container_with_refresh(account, folder_id.as_str(), || {
        format!(
            "folder {} not found (move targets folders only)",
            folder_id.as_str()
        )
    })
    .await
    {
        Ok(native) => native,
        Err(error) => {
            let outcome = ActionOutcome::Failed { error };
            mlog.emit(&outcome);
            return outcome;
        }
    };

    let new_parent = match new_parent_id {
        Some(pid) => match resolve_container_with_refresh(account, pid.as_str(), || {
            format!("parent folder {} not found", pid.as_str())
        })
        .await
        {
            Ok(container) => Some(container),
            Err(error) => {
                let outcome = ActionOutcome::Failed { error };
                mlog.emit(&outcome);
                return outcome;
            }
        },
        None => None,
    };

    let outcome = match account
        .engine
        .container_move(&AccountId(account_id.to_string()), native, new_parent)
        .await
    {
        Ok(()) => {
            // Local best-effort write-back of the new parent, consistent with
            // the create/rename/delete handlers. A parentId-only reparent
            // surfaces on the next sync only as a container `updated`, which
            // the change-stream consumer does not reconcile into
            // `folders.parent_id`, so without this write-back the new parent
            // never lands locally. Targeted UPDATE (not a full upsert) so the
            // row's name and other columns are untouched.
            let db = ctx.write_db.clone();
            let aid = account_id.to_string();
            let fid = folder_id.as_str().to_string();
            let new_parent_storage = new_parent_id.map(|p| p.as_str().to_string());
            if let Err(error) = db
                .with_write(move |conn| {
                    db::db::queries_extra::action_helpers::update_folder_parent_sync(
                        conn,
                        &aid,
                        &fid,
                        new_parent_storage.as_deref(),
                    )
                })
                .await
            {
                log::warn!(
                    "move_folder local parent write-back failed (provider succeeded): {error}"
                );
            }
            ActionOutcome::Success
        }
        Err(error) => ActionOutcome::Failed {
            error: engine_error_to_action_error(error),
        },
    };
    mlog.emit(&outcome);
    outcome
}

/// Delete a folder on the provider, then remove the local rows.
pub(crate) async fn delete_folder(
    ctx: &ActionContext,
    action_account: Option<&ResidentActionAccount>,
    account_id: &str,
    folder_id: &FolderId,
) -> ActionOutcome {
    let mlog = MutationLog::begin("delete_folder", account_id, folder_id.as_str());
    let Some(account) = action_account else {
        let outcome = no_resident();
        mlog.emit(&outcome);
        return outcome;
    };

    let native = match resolve_container_with_refresh(account, folder_id.as_str(), || {
        format!("folder {} not found", folder_id.as_str())
    })
    .await
    {
        Ok(native) => native,
        Err(error) => {
            let outcome = ActionOutcome::Failed { error };
            mlog.emit(&outcome);
            return outcome;
        }
    };

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
    let fid = folder_id.as_str().to_string();
    if let Err(error) = db
        .with_write(move |conn| {
            db::db::queries_extra::action_helpers::delete_folder_sync(conn, &aid, &fid)
        })
        .await
    {
        log::warn!("delete_folder local delete failed (provider succeeded): {error}");
    }

    let outcome = ActionOutcome::Success;
    mlog.emit(&outcome);
    outcome
}

#[cfg(test)]
mod crud_dispatch_tests {
    //! `container_crud_dispatch_is_exhaustive`: the engine primitive each
    //! folder/label CRUD op routes to, kept as its own wildcard-free match
    //! so adding a CRUD op is a compile error in the classifier. Mirrors
    //! `dispatch_target::dispatch_mutation_mapping_is_exhaustive`.

    use bifrost_types::ContainerKind;

    #[derive(Debug, PartialEq, Eq)]
    enum ContainerPrimitive {
        Create(ContainerKind),
        Rename(ContainerKind),
        Move(ContainerKind),
        Delete(ContainerKind),
        Recolor(ContainerKind),
    }

    #[derive(Debug, Clone, Copy)]
    enum CrudOp {
        FolderCreate,
        FolderRename,
        FolderMove,
        FolderDelete,
        LabelCreate,
        LabelRename,
        LabelDelete,
        LabelRecolor,
    }

    fn crud_primitive(op: CrudOp) -> ContainerPrimitive {
        match op {
            CrudOp::FolderCreate => ContainerPrimitive::Create(ContainerKind::Folder),
            CrudOp::FolderRename => ContainerPrimitive::Rename(ContainerKind::Folder),
            CrudOp::FolderMove => ContainerPrimitive::Move(ContainerKind::Folder),
            CrudOp::FolderDelete => ContainerPrimitive::Delete(ContainerKind::Folder),
            CrudOp::LabelCreate => ContainerPrimitive::Create(ContainerKind::Label),
            CrudOp::LabelRename => ContainerPrimitive::Rename(ContainerKind::Label),
            CrudOp::LabelDelete => ContainerPrimitive::Delete(ContainerKind::Label),
            CrudOp::LabelRecolor => ContainerPrimitive::Recolor(ContainerKind::Label),
        }
    }

    #[test]
    fn container_crud_dispatch_is_exhaustive() {
        use ContainerKind::{Folder, Label};
        let cases = [
            (CrudOp::FolderCreate, ContainerPrimitive::Create(Folder)),
            (CrudOp::FolderRename, ContainerPrimitive::Rename(Folder)),
            (CrudOp::FolderMove, ContainerPrimitive::Move(Folder)),
            (CrudOp::FolderDelete, ContainerPrimitive::Delete(Folder)),
            (CrudOp::LabelCreate, ContainerPrimitive::Create(Label)),
            (CrudOp::LabelRename, ContainerPrimitive::Rename(Label)),
            (CrudOp::LabelDelete, ContainerPrimitive::Delete(Label)),
            (CrudOp::LabelRecolor, ContainerPrimitive::Recolor(Label)),
        ];
        for (op, expected) in cases {
            assert_eq!(
                crud_primitive(op),
                expected,
                "unexpected primitive for {op:?}"
            );
        }
        // A folder Move is folder-kind; a label-kind Move has no CRUD op
        // (move targets folders only), proving the label-move rejection is
        // structural, not a runtime guard.
        assert!(!matches!(
            crud_primitive(CrudOp::FolderMove),
            ContainerPrimitive::Move(ContainerKind::Label)
        ));
    }
}
