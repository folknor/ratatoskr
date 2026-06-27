//! Provider-agnostic folder/label list sync (B6a).
//!
//! The single seam that replaces the four per-provider folder-map passes
//! (`prepare_{jmap_mailboxes,graph_folders,gmail_labels}` + the IMAP
//! `sync_imap_folder_map`). One call to `SyncEngine::containers_list`
//! returns a provider-agnostic `Vec<Container>`; `persist_containers`
//! partitions it into `folders` / `labels` rows through the EXISTING
//! `insert_folders_batch` / `upsert_labels` helpers and returns the
//! in-memory `HashMap<String, FolderKind>` folder map (keyed by native
//! id) that the action dispatch and push scopes consume.
//!
//! The system-role normalisation that each legacy provider impl did by
//! hand collapses into the single `FolderRole -> canonical id` table in
//! `role_to_system_folder_id`; non-system ids keep their glossary
//! prefixes via `FolderKind`'s per-provider constructors. See
//! `reference/glossary/folders-labels.md` (the binding contract) and
//! `docs/bifrost-migration.md` (the B6 landing).

use std::collections::HashMap;

use bifrost_sync::SyncEngine;
use bifrost_types::{AccountId, Container, ContainerId, ContainerKind, FolderRole, ProtocolKind};
use common::folder_roles::is_message_state_label_id;
use common::types::{FolderKind, MailProviderKind, SystemFolderId};
use db::db::WriteConn;
use db::db::queries_extra::{FolderWriteRow, LabelWriteRow, insert_folders_batch, upsert_labels};
use service_state::WriteDbState;

/// Read the account's containers off the live engine slot and persist them.
///
/// Runs on every attach and every `refresh_folder_map`. The engine slot
/// MUST already be attached (`attach_account` is reordered so
/// `engine.attach` precedes this call - Obstacle A'), or the
/// `live_account` resolve inside `containers_list` bails
/// `AccountNotAttached`.
pub(crate) async fn sync_containers(
    engine: &SyncEngine,
    account_id: &str,
    write_db: &WriteDbState,
) -> Result<HashMap<String, FolderKind>, String> {
    let account = AccountId(account_id.to_string());
    let containers = engine
        .containers_list(&account)
        .await
        .map_err(|error| format!("containers_list: {error:?}"))?;
    let aid = account_id.to_string();
    write_db
        .with_write(move |conn| persist_containers(conn, &aid, &containers))
        .await
}

/// Map a bifrost `FolderRole` to ratatoskr's canonical system-folder id.
///
/// The ONE place per-provider system-role normalisation lives after B6.
/// Pinned by `container_role_maps_to_canonical_id` against the glossary
/// Identity table. `FolderRole` is `#[non_exhaustive]` in bifrost-types,
/// so a wildcard arm is unavoidable; the unit test pins every known role
/// so a glossary drift is a test failure.
fn role_to_system_folder_id(role: FolderRole) -> Option<SystemFolderId> {
    match role {
        FolderRole::Inbox => Some(SystemFolderId::Inbox),
        FolderRole::Sent => Some(SystemFolderId::Sent),
        FolderRole::Drafts => Some(SystemFolderId::Draft),
        FolderRole::Archive => Some(SystemFolderId::Archive),
        FolderRole::Trash => Some(SystemFolderId::Trash),
        FolderRole::Spam => Some(SystemFolderId::Spam),
        _ => None,
    }
}

/// What a container persists as: a `folders` row (carrying its resolved
/// `FolderKind`), a `labels` row (carrying its storage id), or nothing
/// (message-state ids like `STARRED` / `UNREAD`, filtered exactly as the
/// legacy Gmail pass did before the system/user split).
enum Classified {
    Folder(FolderKind),
    Label(String),
    Skip,
}

/// Classify a container into a folder row, a label row, or a skip.
///
/// A container is a FOLDER when it is folder-shaped on the wire
/// (`kind == Folder`, covering Graph/JMAP/IMAP user folders that carry no
/// role), OR carries a canonical `role`, OR is a native system container
/// (`system`, covering Gmail `CATEGORY_*` / `IMPORTANT` / `CHAT` system
/// labels that `role` alone cannot capture). Message-state
/// ids are filtered first so a Gmail `STARRED` system label never lands as
/// a folder.
fn classify(container: &Container) -> Result<Classified, String> {
    if is_message_state_label_id(&container.native_id) {
        return Ok(Classified::Skip);
    }
    let is_folder = matches!(container.kind, ContainerKind::Folder)
        || container.role.is_some()
        || container.system;
    if is_folder {
        Ok(Classified::Folder(folder_kind_for(container)?))
    } else {
        Ok(Classified::Label(label_storage_id_for(container)?))
    }
}

/// Resolve a folder container to its `FolderKind` (canonical for
/// role-bearing system folders, prefixed-native otherwise). Keys the
/// prefixed fallback off `Container::native_id` by name - the documented
/// protocol id.
fn folder_kind_for(container: &Container) -> Result<FolderKind, String> {
    if let Some(role) = container.role
        && let Some(system) = role_to_system_folder_id(role)
    {
        return Ok(FolderKind::System(system));
    }
    let native = container.native_id.as_str();
    match container.provenance.provider {
        ProtocolKind::Gmail => FolderKind::parse(native, MailProviderKind::Gmail),
        ProtocolKind::Graph => FolderKind::graph_user(native),
        ProtocolKind::Jmap => FolderKind::jmap_user(native),
        ProtocolKind::Imap => FolderKind::imap_user(native),
        other => Err(format!(
            "container {native}: unsupported folder provider {other:?}"
        )),
    }
}

/// Resolve a label container to its storage id. Only Gmail user labels
/// reach this branch in practice (folder-shaped protocols return folders
/// only from `containers_list`); a non-Gmail label is a surprise and is
/// surfaced as an error rather than silently mis-prefixed.
fn label_storage_id_for(container: &Container) -> Result<String, String> {
    match container.provenance.provider {
        // Gmail user labels carry no storage prefix (glossary Identity).
        ProtocolKind::Gmail => Ok(container.native_id.clone()),
        other => Err(format!(
            "container {}: unexpected non-Gmail label from {other:?}",
            container.native_id
        )),
    }
}

/// The persisted id (folder storage id or label storage id) a container
/// resolves to, used to key the parent-resolution map. `None` for skips.
fn persisted_id(container: &Container) -> Result<Option<String>, String> {
    Ok(match classify(container)? {
        Classified::Folder(kind) => Some(kind.storage_id()),
        Classified::Label(id) => Some(id),
        Classified::Skip => None,
    })
}

/// The IMAP special-use string for a canonical system folder id, so an
/// IMAP folder row keeps its `imap_special_use` column across the cut
/// (bifrost folds special-use into `role`, dropping the raw string).
fn imap_special_use_for_storage_id(storage_id: &str) -> Option<String> {
    common::folder_roles::SYSTEM_FOLDER_ROLES
        .iter()
        .find(|entry| entry.label_id == storage_id)
        .and_then(|entry| entry.imap_special_use)
        .map(str::to_string)
}

/// Partition `containers` into `folders` / `labels` rows and write them
/// through the frozen `insert_folders_batch` / `upsert_labels` seam.
/// Returns the native-id-keyed folder map.
fn persist_containers(
    conn: &WriteConn<'_>,
    account_id: &str,
    containers: &[Container],
) -> Result<HashMap<String, FolderKind>, String> {
    let (folder_rows, label_rows, folder_map) = build_container_rows(account_id, containers)?;
    let tx = conn
        .transaction()
        .map_err(|error| format!("begin containers tx: {error}"))?;
    insert_folders_batch(&tx, &folder_rows)?;
    upsert_labels(&tx, &label_rows)?;
    tx.commit()
        .map_err(|error| format!("commit containers: {error}"))?;
    Ok(folder_map)
}

/// Pure partition of `containers` into `folders` rows, `labels` rows, and
/// the native-id-keyed folder map. Split out of `persist_containers` so the
/// `containers_persist_equals_legacy` golden can assert the produced rows
/// without a database. The DB write side is the unchanged
/// `insert_folders_batch` / `upsert_labels` seam.
#[allow(clippy::type_complexity)]
fn build_container_rows(
    account_id: &str,
    containers: &[Container],
) -> Result<
    (
        Vec<FolderWriteRow>,
        Vec<LabelWriteRow>,
        HashMap<String, FolderKind>,
    ),
    String,
> {
    // First pass: map each container's own engine id to its persisted id
    // so the second pass can resolve `parent` (a `ContainerId`, not a
    // `Container`, so `folder_kind_for` cannot run on it directly - B6
    // spec 4.1 two-pass id map).
    let mut id_to_persisted: HashMap<ContainerId, String> = HashMap::new();
    for container in containers {
        if let Some(persisted) = persisted_id(container)? {
            id_to_persisted.insert(container.id.clone(), persisted);
        }
    }

    let mut folder_rows: Vec<FolderWriteRow> = Vec::new();
    let mut label_rows: Vec<LabelWriteRow> = Vec::new();
    let mut folder_map: HashMap<String, FolderKind> = HashMap::new();

    for container in containers {
        match classify(container)? {
            Classified::Skip => {}
            Classified::Folder(kind) => {
                let storage_id = kind.storage_id();
                let is_imap = matches!(container.provenance.provider, ProtocolKind::Imap);
                let parent_id = container
                    .parent
                    .as_ref()
                    .and_then(|parent| id_to_persisted.get(parent).cloned());
                folder_rows.push(FolderWriteRow {
                    id: storage_id.clone(),
                    account_id: account_id.to_string(),
                    name: container.name.clone(),
                    visible: None,
                    sort_order: None,
                    imap_folder_path: is_imap.then(|| container.native_id.clone()),
                    imap_special_use: if is_imap {
                        imap_special_use_for_storage_id(&storage_id)
                    } else {
                        None
                    },
                    namespace_type: None,
                    parent_id,
                    // KNOWN GAP (feature-preserving mandate): the
                    // legacy JMAP `sync_mailboxes` pass populated the
                    // `right_*` ACL columns and `is_subscribed` from
                    // `Mailbox.myRights` / `isSubscribed`; the other three
                    // providers always wrote them `None`. Bifrost's frozen
                    // `Container` shape carries neither, so the JMAP rights
                    // (consumed by `navigation::rights_from_folder` ->
                    // `MailboxRightsInfo`, gating shared-mailbox submit) and
                    // subscription state are dropped here. Restoring them
                    // needs a `Container` field in a future bifrost SQ; it
                    // cannot be reconstructed consumer-side.
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
                    is_undeletable: container.role.is_some() || container.system,
                });
                folder_map.insert(container.native_id.clone(), kind);
            }
            Classified::Label(storage_id) => {
                let (server_color_bg, server_color_fg) = match container.style.as_ref() {
                    Some(style) => (Some(style.color_bg.clone()), Some(style.color_fg.clone())),
                    None => (None, None),
                };
                label_rows.push(LabelWriteRow {
                    id: storage_id,
                    account_id: account_id.to_string(),
                    name: container.name.clone(),
                    visible: None,
                    sort_order: None,
                    server_color_bg,
                    server_color_fg,
                    // `user_color_*` is a purely-local override the sync
                    // never writes; `upsert_labels` COALESCE-preserves any
                    // existing value on conflict.
                    user_color_bg: None,
                    user_color_fg: None,
                    // Provider containers are deletable; `importance:*`
                    // synth rows (never in `containers_list`) keep their
                    // flag via `upsert_labels`' OR-on-conflict semantics.
                    is_undeletable: false,
                });
            }
        }
    }

    Ok((folder_rows, label_rows, folder_map))
}

#[cfg(test)]
mod tests;
