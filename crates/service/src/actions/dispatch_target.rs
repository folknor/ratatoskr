use std::collections::{HashMap, HashSet};

use bifrost_sync::Error as EngineError;
use bifrost_sync::IdempotencyVendor;
use bifrost_types::{
    AccountId, ContainerId, ContainerKind, FlagOp, FolderId as BifrostFolderId, Importance,
    Label as BifrostLabel, LabelId as BifrostLabelId, MailboxId, MembershipScope, MutationTarget,
    ObjectId, ProtocolKind, Provenance,
};
use common::types::FolderKind;
use types::{ImportanceLevel, LabelKind};

use super::context::ActionContext;
use super::operation::MailOperation;
use super::outcome::{ActionError, ActionOutcome, RemoteFailureKind};
use crate::bifrost::BifrostProviderKind;
use crate::bifrost::resident::ResidentActionAccount;

const FLAG_SEEN: &str = "\\Seen";
/// JMAP names the read-state keyword `$seen` (RFC 8621 4.1.1), not the
/// IMAP-style `\Seen` engine flag the other providers use. Bifrost's JMAP
/// `bulk_set_flags` writes the supplied flag string VERBATIM into the
/// `keywords/<flag>` `Email/set` patch (unlike Gmail/Graph/IMAP, whose
/// mutation paths canonicalize `\Seen` to their native read state), and its
/// read side maps keywords back to `Message::flags` verbatim. The consumer
/// derives `is_read` from `flags.contains("$seen")` and only normalizes
/// `\Seen` -> `$seen` for the non-JMAP providers (`hydrate::normalized_flags`),
/// so a `\Seen` keyword never survives a JMAP round-trip and the thread
/// reads back unread. Speak the JMAP-native keyword so the write and read
/// vocabularies match, mirroring that read-side asymmetry.
const JMAP_KEYWORD_SEEN: &str = "$seen";

/// The read-state flag string to hand `bulk_set_flags` for `provider`. JMAP
/// takes the native `$seen` keyword; every other provider takes the IMAP-style
/// `\Seen` engine flag (which bifrost canonicalizes per provider).
fn seen_flag(provider: BifrostProviderKind) -> &'static str {
    match provider {
        BifrostProviderKind::Jmap => JMAP_KEYWORD_SEEN,
        BifrostProviderKind::Gmail | BifrostProviderKind::Graph | BifrostProviderKind::Imap => {
            FLAG_SEEN
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum RemoteBatchKey {
    Star { to: bool },
    Read { to: bool },
    Archive,
    Trash,
    Spam { to: bool },
    MoveToFolder { dest: String },
    PermanentDelete,
}

impl RemoteBatchKey {
    pub(crate) fn from_operation(op: &MailOperation) -> Option<Self> {
        match op {
            MailOperation::SetStarred { to } => Some(Self::Star { to: *to }),
            MailOperation::SetRead { to } => Some(Self::Read { to: *to }),
            MailOperation::Archive => Some(Self::Archive),
            MailOperation::Trash => Some(Self::Trash),
            MailOperation::SetSpam { to } => Some(Self::Spam { to: *to }),
            MailOperation::MoveToFolder { dest, .. } => Some(Self::MoveToFolder {
                dest: dest.as_str().to_string(),
            }),
            MailOperation::PermanentDelete => Some(Self::PermanentDelete),
            _ => None,
        }
    }
}

/// Resolve a ratatoskr `thread_id` to the set of provider message `ObjectId`s
/// the action dispatch mutates (the consumer-side thread->message expansion,
/// spec 2.2.2). For IMAP the object id is reconstructed from the persisted
/// `(imap_folder, imap_uid, imap_uidvalidity)` triple; every other provider
/// round-trips its global message id.
///
/// An empty expansion is RETRYABLE, not terminal: it almost always means the
/// thread's messages are not yet hydrated for a just-acted optimistic write,
/// so it drains via the pending-ops budget rather than stranding the completed
/// local write with no path to push (spec 4.1).
pub(crate) async fn resolve_thread_messages(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    provider: BifrostProviderKind,
) -> Result<Vec<ObjectId>, ActionError> {
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let rows = ctx
        .db
        .with_read(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, imap_folder, imap_uid, imap_uidvalidity \
                     FROM messages WHERE account_id = ?1 AND thread_id = ?2 \
                     ORDER BY date ASC, id ASC",
                )
                .map_err(|error| format!("prepare thread messages: {error}"))?;
            let rows = stmt
                .query_map(rusqlite::params![aid, tid], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                    ))
                })
                .map_err(|error| format!("query thread messages: {error}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("read thread messages: {error}"))?;
            Ok(rows)
        })
        .await
        .map_err(ActionError::db)?;

    if rows.is_empty() {
        return Err(ActionError::remote_with_kind(
            RemoteFailureKind::Transient,
            "thread messages not hydrated",
        ));
    }

    let mut out = Vec::with_capacity(rows.len());
    for (message_id, imap_folder, imap_uid, imap_uidvalidity) in rows {
        let object_id = if provider == BifrostProviderKind::Imap {
            let folder =
                imap_folder.ok_or_else(|| ActionError::db("IMAP message missing folder"))?;
            let uid = imap_uid.ok_or_else(|| ActionError::db("IMAP message missing UID"))?;
            let uidvalidity = imap_uidvalidity
                .ok_or_else(|| ActionError::db("IMAP message missing uidvalidity"))?;
            ObjectId(format!(
                "imap1:{}:{}:{}:{}",
                folder.len(),
                folder,
                uidvalidity,
                uid
            ))
        } else {
            ObjectId(message_id)
        };
        out.push(object_id);
    }
    Ok(out)
}

/// Single-thread engine dispatch: every provider arm maps a `MailOperation` to
/// the bifrost `SyncEngine` mutation passthrough. The label arms are
/// `unreachable!` because the action pipeline routes label / label-group ops
/// through the `label` / `label_group` modules so the optimistic-intent
/// lifecycle (confirm / clear / attach) is preserved (spec 4.3); they never
/// reach here.
pub(crate) async fn dispatch_mutation(
    action_account: &ResidentActionAccount,
    account_id: &str,
    op: &MailOperation,
    ids: Vec<ObjectId>,
) -> Result<(), ActionError> {
    let account = AccountId(account_id.to_string());
    match op {
        MailOperation::SetStarred { to } => {
            set_starred_each(action_account, &account, ids, *to).await
        }
        MailOperation::SetRead { to } => dispatch_read(action_account, &account, ids, *to).await,
        MailOperation::Archive => {
            let dest = native_folder_for_storage_id_opt(&action_account.folder_map, "archive");
            let source = native_folder_for_storage_id_opt(&action_account.folder_map, "INBOX");
            dispatch_container_move(
                action_account,
                &account,
                ids,
                dest.as_deref(),
                source.as_deref(),
            )
            .await
        }
        MailOperation::Trash => {
            let dest = resolve_move_destination(action_account, "TRASH").await?;
            let source = native_folder_for_storage_id_opt(&action_account.folder_map, "INBOX");
            dispatch_container_move(
                action_account,
                &account,
                ids,
                Some(&dest),
                source.as_deref(),
            )
            .await
        }
        MailOperation::SetSpam { to } => {
            // Spamming moves INBOX -> SPAM; un-spamming moves SPAM -> INBOX. The
            // source role is the inverse of the destination so the single-object
            // compose removes the message from the container it actually leaves.
            let (dest_role, source_role) = if *to {
                ("SPAM", "INBOX")
            } else {
                ("INBOX", "SPAM")
            };
            let dest = resolve_move_destination(action_account, dest_role).await?;
            let source = native_folder_for_storage_id_opt(&action_account.folder_map, source_role);
            dispatch_container_move(
                action_account,
                &account,
                ids,
                Some(&dest),
                source.as_deref(),
            )
            .await
        }
        MailOperation::MoveToFolder { dest, source } => {
            let native = resolve_move_destination(action_account, dest.as_str()).await?;
            let native_source = move_source_native(action_account, source.as_ref());
            dispatch_container_move(
                action_account,
                &account,
                ids,
                Some(&native),
                native_source.as_deref(),
            )
            .await
        }
        MailOperation::PermanentDelete => {
            dispatch_permanent_delete(action_account, &account, ids).await
        }
        MailOperation::AddLabel { .. }
        | MailOperation::RemoveLabel { .. }
        | MailOperation::ApplyLabelGroup { .. }
        | MailOperation::RemoveLabelGroup { .. } => {
            unreachable!("label ops dispatch through the label / label_group modules")
        }
        MailOperation::SetPinned { .. }
        | MailOperation::SetMuted { .. }
        | MailOperation::Snooze { .. }
        | MailOperation::Unsnooze => Ok(()),
    }
}

/// Native folder id for a `MoveToFolder` source. The op-level `source` is an
/// advisory hint (spec 2.2.3): when present, resolve it through the folder
/// map; otherwise default to INBOX, the dominant move-out-of-inbox case. Only
/// the single-object compose consumes this - `bulk_move` derives the source
/// per message from each `ObjectId`'s own container context.
fn move_source_native(
    action_account: &ResidentActionAccount,
    source: Option<&common::typed_ids::FolderId>,
) -> Option<String> {
    match source {
        Some(source) => {
            native_folder_for_storage_id_opt(&action_account.folder_map, source.as_str())
        }
        None => native_folder_for_storage_id_opt(&action_account.folder_map, "INBOX"),
    }
}

/// Coalesced engine dispatch for a multi-thread batch: same-account, same-op
/// `ObjectId`s accumulate and dispatch through the bulk surface
/// (`bulk_move` / `bulk_set_flags` / `bulk_destroy`) so the provider's native
/// batch wire op applies (spec 4.5). Star is the exception: it routes per-id
/// through `set_starred` so the capability dispatch (`StarredFlagShape`) picks
/// the right wire field instead of a hardcoded `\Flagged` (finding 2).
pub(crate) async fn dispatch_bulk_mutation(
    action_account: &ResidentActionAccount,
    account_id: &str,
    key: &RemoteBatchKey,
    ids: Vec<ObjectId>,
) -> Result<(), ActionError> {
    let account = AccountId(account_id.to_string());
    match key {
        RemoteBatchKey::Star { to } => set_starred_each(action_account, &account, ids, *to).await,
        RemoteBatchKey::Read { to } => dispatch_read(action_account, &account, ids, *to).await,
        RemoteBatchKey::Archive => {
            let dest = native_folder_for_storage_id_opt(&action_account.folder_map, "archive");
            let source = native_folder_for_storage_id_opt(&action_account.folder_map, "INBOX");
            dispatch_container_move(
                action_account,
                &account,
                ids,
                dest.as_deref(),
                source.as_deref(),
            )
            .await
        }
        RemoteBatchKey::Trash => {
            let dest = resolve_move_destination(action_account, "TRASH").await?;
            let source = native_folder_for_storage_id_opt(&action_account.folder_map, "INBOX");
            dispatch_container_move(
                action_account,
                &account,
                ids,
                Some(&dest),
                source.as_deref(),
            )
            .await
        }
        RemoteBatchKey::Spam { to } => {
            let (dest_role, source_role) = if *to {
                ("SPAM", "INBOX")
            } else {
                ("INBOX", "SPAM")
            };
            let dest = resolve_move_destination(action_account, dest_role).await?;
            let source = native_folder_for_storage_id_opt(&action_account.folder_map, source_role);
            dispatch_container_move(
                action_account,
                &account,
                ids,
                Some(&dest),
                source.as_deref(),
            )
            .await
        }
        RemoteBatchKey::MoveToFolder { dest } => {
            let native = resolve_move_destination(action_account, dest).await?;
            // The coalescing key drops the per-op advisory source; the single
            // compose falls back to INBOX (bulk_move derives source per id).
            let source = native_folder_for_storage_id_opt(&action_account.folder_map, "INBOX");
            dispatch_container_move(
                action_account,
                &account,
                ids,
                Some(&native),
                source.as_deref(),
            )
            .await
        }
        RemoteBatchKey::PermanentDelete => {
            dispatch_permanent_delete(action_account, &account, ids).await
        }
    }
}

/// Read-state dispatch over the resolved message id set.
///
/// ALWAYS rides `bulk_set_flags`, including a one-id set. This is the single
/// remote path for read-state across single- and multi-thread dispatch: the
/// engine's idempotency / read-back / recovery pipeline applies uniformly, and
/// the provider issues its native flag verb over the id set (a one-element set
/// is a one-element batch). The earlier singleton special-case that routed a
/// one-id set through the single-object `set_read` convenience is deliberately
/// gone: that convenience drives a separate per-provider primitive (e.g. Graph
/// does an etag-resolving read-modify-write before the flag write) whose
/// failure modes differ from the bulk surface, which silently degraded a
/// single-message thread's read writeback to `local_only` with no wire op.
async fn dispatch_read(
    action_account: &ResidentActionAccount,
    account: &AccountId,
    ids: Vec<ObjectId>,
    to: bool,
) -> Result<(), ActionError> {
    let vendor =
        IdempotencyVendor::fresh(bifrost_sync::mutation::idempotency::default_salt_factory());
    let op = flag_op(seen_flag(action_account.provider), to);
    action_account
        .engine
        .bulk_set_flags(
            account,
            ids,
            op,
            &vendor,
            protocol_for_provider(action_account.provider),
        )
        .await
        .map(|_| ())
        .map_err(engine_error_to_action_error)
}

/// Hard-delete dispatch. Always routes through `bulk_destroy`: the engine
/// exposes no single-object hard-delete primitive (`delete_thread` is
/// `ThreadId`-typed, which B4 deliberately avoids - spec 2.2.3), and the
/// destroy read-back / absence guard lives on the bulk surface (spec 4.5). A
/// one-id destroy is a one-element `bulk_destroy`; the provider then issues its
/// own individual or batched delete verb per its surface.
async fn dispatch_permanent_delete(
    action_account: &ResidentActionAccount,
    account: &AccountId,
    ids: Vec<ObjectId>,
) -> Result<(), ActionError> {
    let vendor =
        IdempotencyVendor::fresh(bifrost_sync::mutation::idempotency::default_salt_factory());
    action_account
        .engine
        .bulk_destroy(
            account,
            ids,
            &vendor,
            protocol_for_provider(action_account.provider),
        )
        .await
        .map(|_| ())
        .map_err(engine_error_to_action_error)
}

async fn set_starred_each(
    action_account: &ResidentActionAccount,
    account: &AccountId,
    ids: Vec<ObjectId>,
    to: bool,
) -> Result<(), ActionError> {
    for id in ids {
        action_account
            .engine
            .set_starred(account, MutationTarget::Message(id), to)
            .await
            .map_err(engine_error_to_action_error)?;
    }
    Ok(())
}

/// Leaf engine dispatch for a single resolved label across the thread's
/// message ids. `GraphImportance` routes to the exclusive `set_importance`
/// primitive (spec 2.2.4); every other label kind dispatches `apply_label` /
/// `remove_label`, which bifrost fans out by `Label::provenance`. Returns the
/// raw `ActionError`; the intent lifecycle (confirm / clear / attach) is owned
/// by the caller in the `label` / `label_group` modules.
pub(crate) async fn dispatch_label_engine(
    action_account: &ResidentActionAccount,
    account_id: &str,
    label_kind: &LabelKind,
    add: bool,
    ids: Vec<ObjectId>,
) -> Result<(), ActionError> {
    let account = AccountId(account_id.to_string());
    if let LabelKind::GraphImportance(level) = label_kind {
        let importance = match (add, level) {
            (true, ImportanceLevel::High) => Importance::High,
            (true, ImportanceLevel::Low) => Importance::Low,
            (false, _) => Importance::Normal,
        };
        for id in ids {
            action_account
                .engine
                .set_importance(&account, MutationTarget::Message(id), importance)
                .await
                .map_err(engine_error_to_action_error)?;
        }
        return Ok(());
    }
    let label = bifrost_label_for_kind(label_kind, action_account.provider)?;
    for id in ids {
        let target = MutationTarget::Message(id);
        let result = if add {
            action_account
                .engine
                .apply_label(&account, target, label.clone())
                .await
        } else {
            action_account
                .engine
                .remove_label(&account, target, label.clone())
                .await
        };
        result.map_err(engine_error_to_action_error)?;
    }
    Ok(())
}

fn bifrost_label_for_kind(
    kind: &LabelKind,
    provider: BifrostProviderKind,
) -> Result<BifrostLabel, ActionError> {
    let protocol = protocol_for_provider(provider);
    let (native, name, container_kind) = match kind {
        LabelKind::GmailUser(id) => (
            id.as_str().to_string(),
            id.as_str().to_string(),
            ContainerKind::Label,
        ),
        LabelKind::GraphCategory(category) => (
            category.as_str().to_string(),
            category.as_str().to_string(),
            ContainerKind::Label,
        ),
        LabelKind::JmapKeyword(keyword) | LabelKind::ImapKeyword(keyword) => (
            keyword.as_str().to_string(),
            keyword.as_str().to_string(),
            ContainerKind::Label,
        ),
        LabelKind::GraphImportance(_) => {
            return Err(ActionError::invalid_state(
                "importance labels use set_importance",
            ));
        }
    };
    Ok(BifrostLabel {
        id: ContainerId(native.clone()),
        provenance: Provenance {
            provider: protocol,
            kind: container_kind,
            native,
        },
        name,
        role: None,
    })
}

/// Object-level container move composed for archive / trash / spam / move.
///
/// ALWAYS rides the bulk surface, including a one-id set. A destination-bearing
/// move maps the native destination to the provider's `MembershipScope` and
/// dispatches `bulk_move` (one wire campaign over the id set, keeping the
/// provider's native batch verb AND the engine's idempotency / read-back
/// guard); `bulk_move` removes each message from its source as part of the move
/// (IMAP MOVE/COPY+EXPUNGE, Gmail removing INBOX, JMAP mailbox replace, Graph
/// folder move), so no separate per-id source removal is needed.
///
/// The destination-less case is Gmail archive ("archive = remove the INBOX
/// label, no destination folder"): each message is removed from the source
/// (INBOX) container directly.
///
/// The earlier singleton special-case that routed a one-id set through a
/// single-object `add_to_container` + `remove_from_container` compose is
/// deliberately gone: those single-object primitives drive a separate
/// per-provider path (e.g. Graph's etag-resolving read-modify-write) whose
/// failure modes differ from the bulk surface, which silently degraded a
/// single-message thread's move writeback to `local_only` with no wire op.
async fn dispatch_container_move(
    action_account: &ResidentActionAccount,
    account: &AccountId,
    ids: Vec<ObjectId>,
    native_destination: Option<&str>,
    native_source: Option<&str>,
) -> Result<(), ActionError> {
    match native_destination {
        Some(native) => {
            let destination = membership_scope_for(action_account.provider, native);
            let vendor = IdempotencyVendor::fresh(
                bifrost_sync::mutation::idempotency::default_salt_factory(),
            );
            action_account
                .engine
                .bulk_move(
                    account,
                    ids,
                    destination,
                    &vendor,
                    protocol_for_provider(action_account.provider),
                )
                .await
                .map(|_| ())
                .map_err(engine_error_to_action_error)
        }
        None => {
            // Gmail archive: no destination folder; archive means drop the
            // INBOX label. Nothing to remove if the account has no INBOX
            // container resolved.
            let Some(inbox) = native_source
                .map(str::to_string)
                .or_else(|| native_folder_for_storage_id_opt(&action_account.folder_map, "INBOX"))
            else {
                return Ok(());
            };
            let inbox_container = ContainerId(inbox);
            for id in ids {
                action_account
                    .engine
                    .remove_from_container(
                        account,
                        MutationTarget::Message(id),
                        inbox_container.clone(),
                    )
                    .await
                    .map_err(engine_error_to_action_error)?;
            }
            Ok(())
        }
    }
}

/// Map a native folder id to the `MembershipScope` shape `bulk_move` expects
/// for the account's provider: Gmail moves are label patches, Graph and IMAP
/// are folder moves, JMAP is a mailbox replace.
fn membership_scope_for(provider: BifrostProviderKind, native: &str) -> MembershipScope {
    match provider {
        BifrostProviderKind::Gmail => MembershipScope::Label(BifrostLabelId(native.to_string())),
        BifrostProviderKind::Graph | BifrostProviderKind::Imap => {
            MembershipScope::Folder(BifrostFolderId(native.to_string()))
        }
        BifrostProviderKind::Jmap => MembershipScope::Mailbox(MailboxId(native.to_string())),
    }
}

/// Resolve a destination storage id to its native folder id, re-fetching the
/// container map on a cache miss before erroring (spec 4.1, finding 5). A
/// folder created since the resident slot attached is absent from the cached
/// snapshot; re-fetching avoids a terminal not-found that would strand the
/// completed local write.
async fn resolve_move_destination(
    action_account: &ResidentActionAccount,
    storage_id: &str,
) -> Result<String, ActionError> {
    if let Some(native) = native_folder_for_storage_id_opt(&action_account.folder_map, storage_id) {
        return Ok(native);
    }
    let fresh = action_account.refresh_folder_map().await.map_err(|error| {
        ActionError::remote_with_kind(
            RemoteFailureKind::Transient,
            format!("refresh container map: {error}"),
        )
    })?;
    native_folder_for_storage_id_opt(&fresh, storage_id)
        .ok_or_else(|| ActionError::not_found(format!("container {storage_id} not found")))
}

fn native_folder_for_storage_id_opt(
    folder_map: &HashMap<String, FolderKind>,
    storage_id: &str,
) -> Option<String> {
    folder_map
        .iter()
        .find_map(|(native, kind)| (kind.storage_id() == storage_id).then(|| native.clone()))
}

fn flag_op(flag: &str, to: bool) -> FlagOp {
    let flags = HashSet::from([flag.to_string()]);
    if to {
        FlagOp::Add(flags)
    } else {
        FlagOp::Remove(flags)
    }
}

fn protocol_for_provider(provider: BifrostProviderKind) -> ProtocolKind {
    match provider {
        BifrostProviderKind::Gmail => ProtocolKind::Gmail,
        BifrostProviderKind::Graph => ProtocolKind::Graph,
        BifrostProviderKind::Imap => ProtocolKind::Imap,
        BifrostProviderKind::Jmap => ProtocolKind::Jmap,
    }
}

pub(crate) fn engine_error_to_action_error(error: EngineError) -> ActionError {
    match error {
        EngineError::Account(error)
        | EngineError::OpenFailed(error)
        | EngineError::EstablishCursorTerminated(error) => {
            crate::bifrost::account_error_to_action_error(&error)
        }
        EngineError::AccountNotAttached(account) => ActionError::remote_with_kind(
            RemoteFailureKind::Transient,
            format!("account {} is not attached", account.0),
        ),
        other => ActionError::remote_with_kind(RemoteFailureKind::Transient, other.to_string()),
    }
}

pub(crate) fn outcome_from_remote_result(result: Result<(), ActionError>) -> ActionOutcome {
    match result {
        Ok(()) => ActionOutcome::Success,
        Err(reason) => ActionOutcome::LocalOnly {
            retryable: reason.is_retryable(),
            reason,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The engine passthrough each provider arm targets. This mirror of
    /// `dispatch_mutation`'s routing is kept as its own exhaustive match (no
    /// wildcard) so adding a `MailOperation` variant is a compile error in
    /// BOTH the classifier and the real dispatch - the exhaustiveness guard
    /// the spec pins for the action pipeline.
    #[derive(Debug, PartialEq, Eq)]
    enum DispatchClass {
        Star { to: bool },
        Read { to: bool },
        ContainerMove,
        PermanentDelete,
        Label,
        LabelGroup,
        LocalOnly,
    }

    fn dispatch_class(op: &MailOperation) -> DispatchClass {
        match op {
            MailOperation::SetStarred { to } => DispatchClass::Star { to: *to },
            MailOperation::SetRead { to } => DispatchClass::Read { to: *to },
            MailOperation::Archive
            | MailOperation::Trash
            | MailOperation::SetSpam { .. }
            | MailOperation::MoveToFolder { .. } => DispatchClass::ContainerMove,
            MailOperation::PermanentDelete => DispatchClass::PermanentDelete,
            MailOperation::AddLabel { .. } | MailOperation::RemoveLabel { .. } => {
                DispatchClass::Label
            }
            MailOperation::ApplyLabelGroup { .. } | MailOperation::RemoveLabelGroup { .. } => {
                DispatchClass::LabelGroup
            }
            MailOperation::SetPinned { .. }
            | MailOperation::SetMuted { .. }
            | MailOperation::Snooze { .. }
            | MailOperation::Unsnooze => DispatchClass::LocalOnly,
        }
    }

    #[test]
    fn dispatch_mutation_mapping_is_exhaustive() {
        use common::typed_ids::{FolderId, LabelGroupId, LabelId};
        let cases = [
            (
                MailOperation::SetStarred { to: true },
                DispatchClass::Star { to: true },
            ),
            (
                MailOperation::SetRead { to: false },
                DispatchClass::Read { to: false },
            ),
            (MailOperation::Archive, DispatchClass::ContainerMove),
            (MailOperation::Trash, DispatchClass::ContainerMove),
            (
                MailOperation::SetSpam { to: true },
                DispatchClass::ContainerMove,
            ),
            (
                MailOperation::MoveToFolder {
                    dest: FolderId::from("f1"),
                    source: None,
                },
                DispatchClass::ContainerMove,
            ),
            (
                MailOperation::PermanentDelete,
                DispatchClass::PermanentDelete,
            ),
            (
                MailOperation::AddLabel {
                    label_id: LabelId::from("l1"),
                },
                DispatchClass::Label,
            ),
            (
                MailOperation::RemoveLabel {
                    label_id: LabelId::from("l1"),
                },
                DispatchClass::Label,
            ),
            (
                MailOperation::ApplyLabelGroup {
                    group_id: LabelGroupId(1),
                },
                DispatchClass::LabelGroup,
            ),
            (
                MailOperation::RemoveLabelGroup {
                    group_id: LabelGroupId(1),
                },
                DispatchClass::LabelGroup,
            ),
            (
                MailOperation::SetPinned { to: true },
                DispatchClass::LocalOnly,
            ),
            (
                MailOperation::SetMuted { to: true },
                DispatchClass::LocalOnly,
            ),
            (MailOperation::Snooze { until: 0 }, DispatchClass::LocalOnly),
            (MailOperation::Unsnooze, DispatchClass::LocalOnly),
        ];
        for (op, expected) in &cases {
            assert_eq!(
                &dispatch_class(op),
                expected,
                "unexpected dispatch class for {op:?}"
            );
        }
    }
}
