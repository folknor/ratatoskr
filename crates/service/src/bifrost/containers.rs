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
use bifrost_types::{
    AccountId, Container, ContainerContentClass, ContainerId, ContainerKind, ContainerNamespace,
    CursorScope, FolderRole, ProtocolKind,
};
use common::folder_roles::is_message_state_label_id;
use common::types::{FolderKind, MailProviderKind, NamespaceAttribution, SystemFolderId};
use db::db::queries_extra::{FolderWriteRow, LabelWriteRow, insert_folders_batch, upsert_labels};
use db::db::{WriteConn, WriteTxn};
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
) -> Result<ContainerIndex, String> {
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

/// Attach-time container snapshot used everywhere a bifrost native scope id
/// crosses into Ratatoskr. It is deliberately the sole native-id lookup
/// surface: unknown folder scopes return `None`, never Personal.
#[derive(Debug, Clone, Default)]
pub struct ContainerIndex {
    folder_map: HashMap<String, FolderKind>,
    namespaces: HashMap<String, NamespaceAttribution>,
    storage_ids: HashMap<String, String>,
    roles: HashMap<(NamespaceAttribution, FolderRole), String>,
    mail_containers: HashMap<String, bool>,
}

impl ContainerIndex {
    pub fn folder_map(&self) -> &HashMap<String, FolderKind> {
        &self.folder_map
    }

    pub fn storage_id_for_native(&self, native: &str) -> Option<&str> {
        self.storage_ids.get(native).map(String::as_str)
    }

    /// Resolve a system destination without crossing a mailbox namespace.
    /// The returned value is the bifrost-native container id, suitable for a
    /// provider mutation, rather than Ratatoskr's namespaced storage id.
    pub fn role_target(&self, namespace: &NamespaceAttribution, role: FolderRole) -> Option<&str> {
        self.roles
            .get(&(namespace.clone(), role))
            .map(String::as_str)
    }

    pub fn attribution_for_scope(&self, scope: &CursorScope) -> Option<NamespaceAttribution> {
        match scope {
            CursorScope::Account | CursorScope::Type(_) => Some(NamespaceAttribution::Personal),
            CursorScope::Folder(folder) => self.namespaces.get(&folder.0).cloned(),
            CursorScope::FolderType { folder, .. } => self.namespaces.get(&folder.0).cloned(),
            _ => None,
        }
    }

    pub fn is_mail_container(&self, native: &str) -> bool {
        self.mail_containers.get(native).copied().unwrap_or(true)
    }

    /// Graph delegated and public-folder scopes are poll-only. IMAP can IDLE
    /// a shared folder, so its eligibility is decided by the resident using
    /// this attribution rather than by guessing from a storage id.
    pub fn is_personal(&self, native: &str) -> bool {
        matches!(
            self.namespaces.get(native),
            Some(NamespaceAttribution::Personal)
        )
    }

    pub fn is_shared(&self, native: &str) -> bool {
        matches!(
            self.namespaces.get(native),
            Some(NamespaceAttribution::Shared { .. })
        )
    }

    pub fn namespace_for_native(&self, native: &str) -> Option<&NamespaceAttribution> {
        self.namespaces.get(native)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.folder_map.len()
    }

    #[cfg(test)]
    fn get(&self, native: &str) -> Option<&FolderKind> {
        self.folder_map.get(native)
    }

    #[cfg(test)]
    fn contains_key(&self, native: &str) -> bool {
        self.folder_map.contains_key(native)
    }
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
    // A public folder's native id is an opaque EWS folder id, not a provider
    // folder id, so it never goes through the role or provider dispatch below.
    // A public folder also plays no mailbox role.
    if matches!(container.namespace, ContainerNamespace::Public) {
        return FolderKind::public(container.native_id.as_str());
    }
    let base = base_folder_kind(container)?;
    match container.namespace {
        ContainerNamespace::Personal => Ok(base),
        // The namespace wrap MUST apply to role-bearing containers too. A
        // shared mailbox's Inbox is `shared:alice:INBOX`, not `INBOX`: the
        // canonical ids are account-local, so an unwrapped shared Inbox would
        // collide with the personal one on `folders`' primary key and make
        // every role destination resolve to the personal folder.
        ContainerNamespace::Shared => FolderKind::shared(
            container
                .owner
                .as_ref()
                .map(|owner| owner.0.as_str())
                .ok_or_else(|| format!("shared container {} has no owner", container.native_id))?,
            base,
        ),
        // `Public` returned above; `ContainerNamespace` is `#[non_exhaustive]`
        // so a namespace bifrost adds later must fail loudly rather than be
        // silently persisted as personal mail.
        _ => Err(format!(
            "container {} has unknown namespace",
            container.native_id
        )),
    }
}

/// The namespace-independent identity: the canonical id for a role-bearing
/// system folder, the prefixed provider-native id otherwise.
fn base_folder_kind(container: &Container) -> Result<FolderKind, String> {
    if let Some(role) = container.role
        && let Some(system) = role_to_system_folder_id(role)
    {
        return Ok(FolderKind::System(system));
    }
    // `owner_local_id` is the id WITHIN the owner's namespace; for a personal
    // container it is absent and the native id already is that id. The
    // foreign-encoded `native_id` (which embeds a control character) must
    // never reach a storage id - B12 obstacle C.
    let native = container
        .owner_local_id
        .as_deref()
        .unwrap_or(container.native_id.as_str());
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
) -> Result<ContainerIndex, String> {
    let (folder_rows, label_rows, index) = build_container_rows(account_id, containers)?;
    let tx = conn
        .transaction()
        .map_err(|error| format!("begin containers tx: {error}"))?;
    insert_folders_batch(&tx, &folder_rows)?;
    upsert_labels(&tx, &label_rows)?;
    reconcile_namespace_registry(&tx, account_id, containers, &index)?;
    tx.commit()
        .map_err(|error| format!("commit containers: {error}"))?;
    Ok(index)
}

/// Pure partition of `containers` into `folders` rows, `labels` rows, and
/// the native-id-keyed folder map. Split out of `persist_containers` so the
/// `containers_persist_equals_legacy` golden can assert the produced rows
/// without a database. The DB write side is the unchanged
/// `insert_folders_batch` / `upsert_labels` seam.
#[allow(clippy::type_complexity)]
pub(super) fn build_container_rows(
    account_id: &str,
    containers: &[Container],
) -> Result<(Vec<FolderWriteRow>, Vec<LabelWriteRow>, ContainerIndex), String> {
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
    let mut index = ContainerIndex::default();

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
                // Per-folder JMAP ACL rights + subscription state. Only JMAP
                // surfaces these (from `Mailbox.myRights` / `isSubscribed`);
                // other providers leave `Container::rights` / `is_subscribed`
                // `None`, so the columns stay `None` for them - matching the
                // legacy behaviour where only the JMAP `sync_mailboxes` pass
                // wrote them. Consumed by `navigation::rights_from_folder` ->
                // `MailboxRightsInfo`, which gates shared-mailbox submit.
                let rights = container.rights.as_ref();
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
                    namespace_type: match container.namespace {
                        ContainerNamespace::Personal => None,
                        ContainerNamespace::Shared => Some("shared".to_string()),
                        ContainerNamespace::Public => Some("public".to_string()),
                        _ => None,
                    },
                    owner_id: container.owner.as_ref().map(|owner| owner.0.clone()),
                    content_class: container.content_class.map(content_class_name),
                    parent_id,
                    right_read: rights.and_then(|r| r.may_read_items).map(i64::from),
                    right_add: rights.and_then(|r| r.may_add_items).map(i64::from),
                    right_remove: rights.and_then(|r| r.may_remove_items).map(i64::from),
                    right_set_seen: rights.and_then(|r| r.may_set_seen).map(i64::from),
                    right_set_keywords: rights.and_then(|r| r.may_set_keywords).map(i64::from),
                    right_create_child: rights.and_then(|r| r.may_create_child).map(i64::from),
                    right_rename: rights.and_then(|r| r.may_rename).map(i64::from),
                    right_delete: rights.and_then(|r| r.may_delete).map(i64::from),
                    right_submit: rights.and_then(|r| r.may_submit).map(i64::from),
                    is_subscribed: container.is_subscribed.map(i64::from),
                    is_undeletable: container.role.is_some() || container.system,
                });
                let attribution = namespace_for_container(container)?;
                if let Some(role) = container.role {
                    index
                        .roles
                        .insert((attribution.clone(), role), container.native_id.clone());
                }
                index
                    .storage_ids
                    .insert(container.native_id.clone(), storage_id);
                index
                    .namespaces
                    .insert(container.native_id.clone(), attribution);
                index.mail_containers.insert(
                    container.native_id.clone(),
                    !matches!(
                        container.content_class,
                        Some(
                            ContainerContentClass::Calendar
                                | ContainerContentClass::Contacts
                                | ContainerContentClass::Other
                        )
                    ),
                );
                index.folder_map.insert(container.native_id.clone(), kind);
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

    Ok((folder_rows, label_rows, index))
}

/// Keep the sidebar registries derived from the same snapshot that provides
/// event attribution. This is intentionally provider-agnostic: Graph,
/// IMAP, and JMAP all surface foreign containers here.
fn reconcile_namespace_registry(
    conn: &WriteTxn<'_>,
    account_id: &str,
    containers: &[Container],
    index: &ContainerIndex,
) -> Result<(), String> {
    let mut owners = std::collections::HashSet::new();
    for container in containers {
        let Some(NamespaceAttribution::Shared { owner }) =
            index.namespace_for_native(&container.native_id)
        else {
            continue;
        };
        if !owners.insert(owner.clone()) {
            continue;
        }
        conn.execute(
            "INSERT INTO shared_mailboxes \
             (account_id, mailbox_id, display_name, is_visible, revoked_at) \
             VALUES (?1, ?2, ?3, 1, NULL) \
             ON CONFLICT(account_id, mailbox_id) DO UPDATE SET \
             display_name = COALESCE(excluded.display_name, shared_mailboxes.display_name), \
             revoked_at = NULL",
            rusqlite::params![account_id, owner, container.name],
        )
        .map_err(|error| format!("upsert shared mailbox registry: {error}"))?;
    }

    if owners.is_empty() {
        conn.execute(
            "UPDATE shared_mailboxes SET revoked_at = unixepoch() \
             WHERE account_id = ?1 AND revoked_at IS NULL",
            rusqlite::params![account_id],
        )
        .map_err(|error| format!("revoke shared mailbox registry: {error}"))?;
    } else {
        let placeholders = std::iter::repeat_n("?", owners.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "UPDATE shared_mailboxes SET revoked_at = unixepoch() \
             WHERE account_id = ?1 AND revoked_at IS NULL \
             AND mailbox_id NOT IN ({placeholders})"
        );
        let mut params: Vec<&dyn rusqlite::types::ToSql> = vec![&account_id];
        for owner in &owners {
            params.push(owner);
        }
        conn.execute(&sql, params.as_slice())
            .map_err(|error| format!("revoke missing shared mailboxes: {error}"))?;
    }

    for container in containers {
        let Some(NamespaceAttribution::Public { folder_storage_id }) =
            index.namespace_for_native(&container.native_id)
        else {
            continue;
        };
        conn.execute(
            "INSERT INTO public_folder_pins (account_id, folder_id, is_sync_enabled, is_visible) \
             VALUES (?1, ?2, 0, 1) \
             ON CONFLICT(account_id, folder_id) DO NOTHING",
            rusqlite::params![account_id, folder_storage_id],
        )
        .map_err(|error| format!("upsert public-folder registry: {error}"))?;
    }
    Ok(())
}

fn namespace_for_container(container: &Container) -> Result<NamespaceAttribution, String> {
    match container.namespace {
        ContainerNamespace::Personal => Ok(NamespaceAttribution::Personal),
        ContainerNamespace::Shared => Ok(NamespaceAttribution::Shared {
            owner: container
                .owner
                .as_ref()
                .map(|owner| owner.0.clone())
                .ok_or_else(|| format!("shared container {} has no owner", container.native_id))?,
        }),
        ContainerNamespace::Public => Ok(NamespaceAttribution::Public {
            folder_storage_id: FolderKind::public(container.native_id.as_str())?.storage_id(),
        }),
        _ => Err(format!(
            "container {} has unknown namespace",
            container.native_id
        )),
    }
}

fn content_class_name(class: ContainerContentClass) -> String {
    match class {
        ContainerContentClass::Mail => "Mail",
        ContainerContentClass::Calendar => "Calendar",
        ContainerContentClass::Contacts => "Contacts",
        _ => "Other",
    }
    .to_string()
}

#[cfg(test)]
mod tests;
