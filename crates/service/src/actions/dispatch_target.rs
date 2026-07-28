use std::collections::{HashMap, HashSet};

use bifrost_sync::Error as EngineError;
use bifrost_sync::IdempotencyVendor;
use bifrost_types::{
    AccountId, ContainerId, ContainerKind, FlagOp, FolderId as BifrostFolderId, FolderRole,
    Importance, Label as BifrostLabel, LabelId as BifrostLabelId, MailboxId, MembershipScope,
    MutationTarget, ObjectId, ProtocolKind, Provenance,
};
use common::types::{FolderKind, NamespaceAttribution};
use types::{ImportanceLevel, LabelKind};

use super::context::ActionContext;
use super::operation::MailOperation;
use super::outcome::{ActionError, ActionOutcome, RemoteFailureKind};
use crate::bifrost::BifrostProviderKind;
use crate::bifrost::containers::ContainerIndex;
use crate::bifrost::resident::ResidentActionAccount;

const FLAG_SEEN: &str = "\\Seen";
const FLAG_FLAGGED: &str = "\\Flagged";
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
/// The starred/flagged counterpart of `JMAP_KEYWORD_SEEN`, for the same
/// verbatim-keyword reason (RFC 8621 4.1.1 names the flag `$flagged`).
const JMAP_KEYWORD_FLAGGED: &str = "$flagged";

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

/// The starred-state flag string to hand `bulk_set_flags` for `provider`.
///
/// Exactly the `seen_flag` shape, and the reason star can ride the same bulk
/// primitive the other volume ops use instead of the per-message `set_starred`
/// convenience: every provider's bulk flag path already translates the starred
/// flag into its own native star field, so the capability dispatch bifrost's
/// `set_starred` performs (`StarredFlagShape`) is reproduced by the flag string
/// alone.
///
/// - Gmail: `translate_flag_op` maps `\Flagged` onto the `STARRED` label
///   membership (the same thing `StarredFlagShape::LabelMembership` selects).
/// - Graph: `patch_for_flags` maps `\Flagged` onto `flag.flagStatus` (the same
///   field `StarredFlagShape::Category` reaches via its `$flagged` special case).
/// - IMAP: `Flag::from_imap_str` maps `\Flagged` onto the `\Flagged` system
///   flag. Note it does NOT recognise `$flagged` - only the single-object
///   `set_keyword` path does that translation - so the bulk path must be handed
///   the backslash form.
/// - JMAP: keywords are written verbatim, so it must be handed `$flagged`.
///
/// The consumer's `hydrate::normalized_flags` performs the mirror-image
/// `\Flagged -> $flagged` normalisation on the read side for exactly the three
/// non-JMAP providers, so write and read vocabularies match per provider.
fn starred_flag(provider: BifrostProviderKind) -> &'static str {
    match provider {
        BifrostProviderKind::Jmap => JMAP_KEYWORD_FLAGGED,
        BifrostProviderKind::Gmail | BifrostProviderKind::Graph | BifrostProviderKind::Imap => {
            FLAG_FLAGGED
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum RemoteBatchKey {
    Star {
        to: bool,
    },
    Read {
        to: bool,
    },
    Archive,
    Trash,
    Spam {
        to: bool,
    },
    /// The source is part of the KEY, not dropped as it once was. On a
    /// label-model provider the source decides the wire patch (Gmail's
    /// `bulk_move_from` folds the source detach into the same `batchModify`
    /// as the destination), so two moves to the same destination out of
    /// DIFFERENT sources are two different wire ops and must not coalesce
    /// into one. Moves issued from a single folder view share a source and
    /// still coalesce, which is the case that matters for volume.
    MoveToFolder {
        dest: String,
        source: Option<String>,
    },
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
            MailOperation::MoveToFolder { dest, source } => Some(Self::MoveToFolder {
                dest: dest.as_str().to_string(),
                source: source.as_ref().map(|source| source.as_str().to_string()),
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

pub(crate) async fn resolve_message_object_id(
    ctx: &ActionContext,
    account_id: &str,
    message_id: &str,
    provider: BifrostProviderKind,
) -> Result<ObjectId, ActionError> {
    let aid = account_id.to_string();
    let mid = message_id.to_string();
    let row = ctx
        .db
        .with_read(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, imap_folder, imap_uid, imap_uidvalidity \
                     FROM messages WHERE account_id = ?1 AND id = ?2",
                )
                .map_err(|error| format!("prepare message object id: {error}"))?;
            let mut rows = stmt
                .query_map(rusqlite::params![aid, mid], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                    ))
                })
                .map_err(|error| format!("query message object id: {error}"))?;
            match rows.next() {
                Some(row) => {
                    Ok(Some(row.map_err(|error| {
                        format!("read message object id: {error}")
                    })?))
                }
                None => Ok(None),
            }
        })
        .await
        .map_err(ActionError::db)?;

    let Some((message_id, imap_folder, imap_uid, imap_uidvalidity)) = row else {
        return Err(ActionError::not_found(format!(
            "message {message_id} not found"
        )));
    };
    if provider == BifrostProviderKind::Imap {
        let folder = imap_folder.ok_or_else(|| ActionError::db("IMAP message missing folder"))?;
        let uid = imap_uid.ok_or_else(|| ActionError::db("IMAP message missing UID"))?;
        let uidvalidity =
            imap_uidvalidity.ok_or_else(|| ActionError::db("IMAP message missing uidvalidity"))?;
        Ok(ObjectId(format!(
            "imap1:{}:{}:{}:{}",
            folder.len(),
            folder,
            uidvalidity,
            uid
        )))
    } else {
        Ok(ObjectId(message_id))
    }
}

pub(crate) async fn dispatch_send_intent_mark(
    action_account: &ResidentActionAccount,
    account_id: &str,
    intent: service_api::actions::SendIntent,
    object_id: ObjectId,
) -> Result<(), ActionError> {
    let account = AccountId(account_id.to_string());
    match intent {
        service_api::actions::SendIntent::New => Ok(()),
        service_api::actions::SendIntent::Reply => action_account
            .engine
            .mark_replied(&account, object_id)
            .await
            .map_err(engine_error_to_action_error),
        service_api::actions::SendIntent::Forward => action_account
            .engine
            .mark_forwarded(&account, object_id)
            .await
            .map_err(engine_error_to_action_error),
    }
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
    namespace: &NamespaceAttribution,
) -> Result<(), ActionError> {
    let account = AccountId(account_id.to_string());
    match op {
        MailOperation::SetStarred { to } => {
            dispatch_flags(
                action_account,
                &account,
                ids,
                starred_flag(action_account.provider),
                *to,
            )
            .await
        }
        MailOperation::SetRead { to } => {
            dispatch_flags(
                action_account,
                &account,
                ids,
                seen_flag(action_account.provider),
                *to,
            )
            .await
        }
        MailOperation::Archive => {
            dispatch_container_op(
                action_account,
                &account,
                ContainerMoveOp::Archive,
                ids,
                namespace,
            )
            .await
        }
        MailOperation::Trash => {
            dispatch_container_op(
                action_account,
                &account,
                ContainerMoveOp::Trash,
                ids,
                namespace,
            )
            .await
        }
        MailOperation::SetSpam { to } => {
            dispatch_container_op(
                action_account,
                &account,
                ContainerMoveOp::Spam { to: *to },
                ids,
                namespace,
            )
            .await
        }
        MailOperation::MoveToFolder { dest, source } => {
            dispatch_container_op(
                action_account,
                &account,
                ContainerMoveOp::MoveToFolder {
                    dest: dest.as_str(),
                    source: source.as_ref().map(common::typed_ids::FolderId::as_str),
                },
                ids,
                namespace,
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

/// Coalesced engine dispatch for a multi-thread batch: same-account, same-op
/// `ObjectId`s accumulate and dispatch through the bulk surface
/// (`bulk_move` / `bulk_set_flags` / `bulk_destroy`) so the provider's native
/// batch wire op applies (spec 4.5). Star coalesces here too, through
/// `bulk_set_flags` with the provider's own starred flag string - see
/// `starred_flag` for why that reproduces the `StarredFlagShape` capability
/// dispatch without the per-id `set_starred` convenience, which issued one
/// single-object call PER MESSAGE while archive / move / delete each ran a
/// single bulk campaign.
///
/// "One campaign" is not "one request": every provider chunks the id set
/// (Gmail by its `batchModify` limit, Graph by `max_items`, JMAP by
/// `maxObjectsInSet`, IMAP by folder grouping). The win is the provider's
/// native batch verb plus one pass through the engine's idempotency /
/// read-back / recovery pipeline, not a single HTTP call.
pub(crate) async fn dispatch_bulk_mutation(
    action_account: &ResidentActionAccount,
    account_id: &str,
    key: &RemoteBatchKey,
    ids: Vec<ObjectId>,
    namespace: &NamespaceAttribution,
) -> Result<(), ActionError> {
    let account = AccountId(account_id.to_string());
    match key {
        RemoteBatchKey::Star { to } => {
            dispatch_flags(
                action_account,
                &account,
                ids,
                starred_flag(action_account.provider),
                *to,
            )
            .await
        }
        RemoteBatchKey::Read { to } => {
            dispatch_flags(
                action_account,
                &account,
                ids,
                seen_flag(action_account.provider),
                *to,
            )
            .await
        }
        RemoteBatchKey::Archive => {
            dispatch_container_op(
                action_account,
                &account,
                ContainerMoveOp::Archive,
                ids,
                namespace,
            )
            .await
        }
        RemoteBatchKey::Trash => {
            dispatch_container_op(
                action_account,
                &account,
                ContainerMoveOp::Trash,
                ids,
                namespace,
            )
            .await
        }
        RemoteBatchKey::Spam { to } => {
            dispatch_container_op(
                action_account,
                &account,
                ContainerMoveOp::Spam { to: *to },
                ids,
                namespace,
            )
            .await
        }
        RemoteBatchKey::MoveToFolder { dest, source } => {
            dispatch_container_op(
                action_account,
                &account,
                ContainerMoveOp::MoveToFolder {
                    dest: dest.as_str(),
                    source: source.as_deref(),
                },
                ids,
                namespace,
            )
            .await
        }
        RemoteBatchKey::PermanentDelete => {
            dispatch_permanent_delete(action_account, &account, ids).await
        }
    }
}

/// Flag-state dispatch (read state, star) over the resolved message id set.
///
/// ALWAYS rides `bulk_set_flags`, including a one-id set. This is the single
/// remote path for flag state across single- and multi-thread dispatch: the
/// engine's idempotency / read-back / recovery pipeline applies uniformly, and
/// the provider issues its native flag verb over the id set (a one-element set
/// is a one-element batch). The earlier singleton special-case that routed a
/// one-id set through the single-object `set_read` convenience is deliberately
/// gone: that convenience drives a separate per-provider primitive (e.g. Graph
/// does an etag-resolving read-modify-write before the flag write) whose
/// failure modes differ from the bulk surface, which silently degraded a
/// single-message thread's read writeback to `local_only` with no wire op. The
/// same reasoning retires the per-id `set_starred` loop star used to run.
async fn dispatch_flags(
    action_account: &ResidentActionAccount,
    account: &AccountId,
    ids: Vec<ObjectId>,
    flag: &str,
    to: bool,
) -> Result<(), ActionError> {
    let vendor =
        IdempotencyVendor::fresh(bifrost_sync::mutation::idempotency::default_salt_factory());
    let op = flag_op(flag, to);
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
        style: None,
        system: false,
    })
}

/// The container-move family: the four ops whose remote leg is "put these
/// messages somewhere else". `dispatch_mutation` (single thread) and
/// `dispatch_bulk_mutation` (coalesced batch) both reduce to this so the two
/// paths cannot drift in destination resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContainerMoveOp<'a> {
    Archive,
    Trash,
    Spam {
        to: bool,
    },
    MoveToFolder {
        dest: &'a str,
        /// Storage id of the container the thread is leaving, when the caller
        /// knows it. Advisory (spec 2.2.3) for the folder-model providers,
        /// whose `bulk_move` removes the source itself; load-bearing on Gmail,
        /// where the move patch is destination-only.
        source: Option<&'a str>,
    },
}

impl ContainerMoveOp<'_> {
    /// Text for the terminal not-found when nothing resolves the destination,
    /// even after a container refresh.
    fn destination_description(self) -> String {
        match self {
            Self::Archive => "archive destination".to_string(),
            Self::Trash => "trash destination".to_string(),
            Self::Spam { to: true } => "spam destination".to_string(),
            Self::Spam { to: false } => "un-spam destination".to_string(),
            Self::MoveToFolder { dest, .. } => format!("container {dest}"),
        }
    }
}

/// The `FolderRole` a container-move op targets, or `None` for `MoveToFolder`,
/// whose destination is an explicit storage id rather than a role.
///
/// Spamming moves INBOX -> SPAM and un-spamming moves SPAM -> INBOX, so the
/// un-spam destination is the Inbox role, not the Spam one.
fn destination_role(op: ContainerMoveOp<'_>) -> Option<FolderRole> {
    match op {
        ContainerMoveOp::Archive => Some(FolderRole::Archive),
        ContainerMoveOp::Trash => Some(FolderRole::Trash),
        ContainerMoveOp::Spam { to: true } => Some(FolderRole::Spam),
        ContainerMoveOp::Spam { to: false } => Some(FolderRole::Inbox),
        ContainerMoveOp::MoveToFolder { .. } => None,
    }
}

/// The container a container-move op LEAVES, expressed as a role, when the op
/// itself names one. `None` means "no role-shaped source": either the op does
/// not imply one, or (for `MoveToFolder`) the source is an explicit storage id.
///
/// Only Gmail's `bulk_move_from` consumes the source on the wire, and the two
/// `None` arms below are deliberate rather than unfinished:
///
/// - `Trash` and mark-as-spam: bifrost's Gmail patch already removes INBOX for
///   any non-INBOX destination, and a USER label surviving a trash is native
///   Gmail behaviour, not drift - `trash_local` / `spam_local` only remove
///   INBOX locally, so local and remote agree.
/// - `Archive`: the whole op IS the INBOX detach.
///
/// Un-spam is the one role-shaped source that matters: `spam_local` removes
/// SPAM locally, and the destination-only Gmail patch for a destination of
/// INBOX removes nothing at all, so without this the thread comes back from
/// the next sync still labelled SPAM.
fn source_role(op: ContainerMoveOp<'_>) -> Option<FolderRole> {
    match op {
        ContainerMoveOp::Spam { to: false } => Some(FolderRole::Spam),
        ContainerMoveOp::Archive
        | ContainerMoveOp::Trash
        | ContainerMoveOp::Spam { to: true }
        | ContainerMoveOp::MoveToFolder { .. } => None,
    }
}

/// What a container-move op reduces to once its native destination is resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ContainerMovePlan {
    /// Bulk-move every id into `destination`, carrying `source` for the
    /// provider to fold in as it sees fit: bifrost's `bulk_move_from` default
    /// ignores it on the folder-model providers (their `bulk_move` removes
    /// each message's own source natively), and Gmail folds it into the same
    /// `batchModify` as the destination - which is what makes a Gmail move
    /// out of a user label ONE request instead of one plus a
    /// `remove_from_container` per message.
    MoveTo {
        destination: String,
        source: Option<String>,
    },
    /// Detach every id from the account's INBOX container. Gmail's archive
    /// shape, and the only shape that legitimately has no destination folder.
    DetachInbox(String),
    /// Gmail archive on an account whose INBOX container did not resolve:
    /// there is nothing to detach, so there is nothing to send.
    NoOp,
}

/// Gmail models archive as the ABSENCE of INBOX, so bifrost's Gmail
/// `containers_list` synthesises an `archive` container purely to give the role
/// table an Archive entry.
const GMAIL_SYNTHETIC_ARCHIVE_ID: &str = "archive";

/// True when this op on Gmail is the "drop INBOX, add nothing" shape.
///
/// OPERATION-AWARE ON PURPOSE. An earlier revision keyed this on
/// `native_destination.is_none()` alone, which is wrong twice over, because
/// Trash and Spam resolve their destinations through the SAME role table: a
/// snapshot that has not yet seen the account's Trash role would (a) silently
/// turn a trash into an archive, and (b) by answering `Some`, suppress the
/// caller's refresh-and-retry, so the snapshot never got re-fetched either.
/// Only Archive - and a `MoveToFolder` naming Gmail's synthetic archive id -
/// may take this shape; every other op with an unresolved destination must
/// stay `None` so the caller refreshes and then fails terminally.
fn is_gmail_archive_shape(op: ContainerMoveOp<'_>, native_destination: Option<&str>) -> bool {
    let is_synthetic = |dest: &str| dest.eq_ignore_ascii_case(GMAIL_SYNTHETIC_ARCHIVE_ID);
    match op {
        ContainerMoveOp::Archive => native_destination.is_none_or(is_synthetic),
        ContainerMoveOp::MoveToFolder { .. } => native_destination.is_some_and(is_synthetic),
        ContainerMoveOp::Trash | ContainerMoveOp::Spam { .. } => false,
    }
}

/// Reduce a resolved native destination to the plan the engine dispatch runs.
///
/// `None` means the op has no destination this account can satisfy. It is
/// reachable for any role-resolved op whose role is missing from the snapshot,
/// and it must NEVER degrade into detaching from INBOX: bifrost lowers IMAP
/// `remove_from_container` to `\Deleted` + `UID EXPUNGE`, so an INBOX-shaped
/// fallback destroys an inbox message on archive (immediately where UIDPLUS or
/// IMAP4rev2 allow `UID EXPUNGE`, and otherwise by leaving the message
/// `\Deleted` and exposed to the next expunge from any client), and for a
/// message living in any other folder it filters to an empty id set and fails
/// `Unsupported` - degrading the whole op to a retryable LocalOnly that can
/// never succeed. The caller turns `None` into a terminal not-found after one
/// container refresh.
///
/// The Gmail archive shape is the one legitimate destination-less plan: Gmail's
/// `archive` container id is SYNTHETIC, not a real label id, and bifrost's bulk
/// `move_patch` lowers a `MembershipScope::Label` destination straight into
/// `addLabelIds`, so routing Gmail archive through `bulk_move` would ask Gmail
/// to apply a label that does not exist. (Only bifrost's single-object
/// `add_to_container` special-cases the synthetic id, and the bulk surface is
/// the only one the action pipeline uses.)
fn container_move_plan(
    provider: BifrostProviderKind,
    op: ContainerMoveOp<'_>,
    native_destination: Option<&str>,
    native_inbox: Option<&str>,
    native_source: Option<&str>,
) -> Option<ContainerMovePlan> {
    if provider == BifrostProviderKind::Gmail && is_gmail_archive_shape(op, native_destination) {
        return Some(match native_inbox {
            Some(inbox) => ContainerMovePlan::DetachInbox(inbox.to_string()),
            None => ContainerMovePlan::NoOp,
        });
    }
    let destination = native_destination?;
    // The source flows through UNFILTERED. The exclusions the dispatch used
    // to apply (Gmail-only, skip INBOX, skip source == destination) are
    // bifrost's to make now: Gmail's `move_patch` drops a redundant or
    // synthetic source itself, and the folder-model providers' default
    // `bulk_move_from` ignores the source entirely because their `bulk_move`
    // removes each message's own source natively.
    Some(ContainerMovePlan::MoveTo {
        destination: destination.to_string(),
        source: native_source.map(str::to_string),
    })
}

/// Resolve one container-move op against a container snapshot. Pure, so the
/// caller owns the decision to re-fetch the snapshot on a `None`.
///
/// An unresolvable SOURCE is not a miss: the source is advisory, so it simply
/// yields no detach. Only an unresolvable DESTINATION returns `None`.
fn plan_for_container_op(
    provider: BifrostProviderKind,
    op: ContainerMoveOp<'_>,
    containers: &ContainerIndex,
    namespace: &NamespaceAttribution,
) -> Option<ContainerMovePlan> {
    let inbox = containers.role_target(namespace, FolderRole::Inbox);
    let destination: Option<String> = match destination_role(op) {
        Some(role) => containers.role_target(namespace, role).map(str::to_string),
        None => {
            let ContainerMoveOp::MoveToFolder { dest, .. } = op else {
                unreachable!("only MoveToFolder has no destination role")
            };
            Some(native_folder_for_storage_id_opt(
                containers.folder_map(),
                dest,
            )?)
        }
    };
    let source: Option<String> = match op {
        ContainerMoveOp::MoveToFolder { source, .. } => source
            .and_then(|storage| native_folder_for_storage_id_opt(containers.folder_map(), storage)),
        _ => source_role(op)
            .and_then(|role| containers.role_target(namespace, role))
            .map(str::to_string),
    };
    container_move_plan(
        provider,
        op,
        destination.as_deref(),
        inbox,
        source.as_deref(),
    )
}

/// Object-level container move composed for archive / trash / spam / move.
///
/// Resolves the op's destination against the resident slot's container
/// snapshot, re-fetching once on a miss (spec 4.1, finding 5) so a folder
/// created since attach - or an Archive folder the account grew later - resolves
/// instead of stranding the already-completed local write on a terminal
/// not-found.
async fn dispatch_container_op(
    action_account: &ResidentActionAccount,
    account: &AccountId,
    op: ContainerMoveOp<'_>,
    ids: Vec<ObjectId>,
    namespace: &NamespaceAttribution,
) -> Result<(), ActionError> {
    let plan = match plan_for_container_op(
        action_account.provider,
        op,
        action_account.containers.as_ref(),
        namespace,
    ) {
        Some(plan) => plan,
        None => {
            let fresh = action_account.refresh_containers().await.map_err(|error| {
                ActionError::remote_with_kind(
                    RemoteFailureKind::Transient,
                    format!("refresh container map: {error}"),
                )
            })?;
            plan_for_container_op(action_account.provider, op, &fresh, namespace).ok_or_else(
                || ActionError::not_found(format!("{} not found", op.destination_description())),
            )?
        }
    };
    run_container_move_plan(action_account, account, ids, plan).await
}

/// Execute a resolved `ContainerMovePlan`.
///
/// A move ALWAYS rides the bulk surface, including a one-id set: it maps the
/// native destination (and source, when the plan carries one) to the
/// provider's `MembershipScope` shapes and dispatches ONE `bulk_move_from`
/// campaign over the id set - the engine still chunks it to the provider's
/// batch ceiling, keeping the provider's native batch verb AND the engine's
/// idempotency / read-back guard. The folder-model providers ignore the
/// source (their `bulk_move` removes each message's own source natively);
/// Gmail folds it into the same `batchModify` as the destination, so a Gmail
/// move out of a user label is one request where it used to be one plus a
/// per-id `remove_from_container` detach. Note the engine's read-back guard
/// reconciles membership of the DESTINATION only - absence from the source is
/// asserted by our own gates, not re-verified on the wire.
///
/// The earlier singleton special-case that routed a one-id set through a
/// single-object `add_to_container` + `remove_from_container` compose is
/// deliberately gone: those single-object primitives drive a separate
/// per-provider path (e.g. Graph's etag-resolving read-modify-write) whose
/// failure modes differ from the bulk surface, which silently degraded a
/// single-message thread's move writeback to `local_only` with no wire op.
async fn run_container_move_plan(
    action_account: &ResidentActionAccount,
    account: &AccountId,
    ids: Vec<ObjectId>,
    plan: ContainerMovePlan,
) -> Result<(), ActionError> {
    match plan {
        ContainerMovePlan::MoveTo {
            destination,
            source,
        } => {
            let scope = membership_scope_for(action_account.provider, &destination);
            let source_scope =
                source.map(|source| membership_scope_for(action_account.provider, &source));
            let vendor = IdempotencyVendor::fresh(
                bifrost_sync::mutation::idempotency::default_salt_factory(),
            );
            action_account
                .engine
                .bulk_move_from(
                    account,
                    ids,
                    scope,
                    source_scope,
                    &vendor,
                    protocol_for_provider(action_account.provider),
                )
                .await
                .map(|_| ())
                .map_err(engine_error_to_action_error)
        }
        ContainerMovePlan::DetachInbox(inbox) => {
            detach_from_container(action_account, account, ids, ContainerId(inbox)).await
        }
        ContainerMovePlan::NoOp => Ok(()),
    }
}

/// Remove every id from one container.
///
/// Per-id by necessity: bifrost's `remove_from_container` takes a single
/// `MutationTarget`, and there is no bulk container-detach on the engine
/// surface. Only ever reached for the Gmail ARCHIVE shape (`DetachInbox`),
/// where the operation is a label patch with no destination to ride
/// `bulk_move_from`.
async fn detach_from_container(
    action_account: &ResidentActionAccount,
    account: &AccountId,
    ids: Vec<ObjectId>,
    container: ContainerId,
) -> Result<(), ActionError> {
    for id in ids {
        action_account
            .engine
            .remove_from_container(account, MutationTarget::Message(id), container.clone())
            .await
            .map_err(engine_error_to_action_error)?;
    }
    Ok(())
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

    /// The engine primitive each `SendIntent` arm of `dispatch_send_intent_mark`
    /// targets. Kept as its own exhaustive match (no wildcard) so adding a
    /// `SendIntent` variant is a compile error in BOTH the classifier and the
    /// real reply/forward write-back dispatch - the same exhaustiveness guard
    /// `dispatch_mutation_mapping_is_exhaustive` pins for the mutation path.
    #[derive(Debug, PartialEq, Eq)]
    enum SendIntentClass {
        NoOp,
        MarkReplied,
        MarkForwarded,
    }

    fn send_intent_class(intent: service_api::actions::SendIntent) -> SendIntentClass {
        use service_api::actions::SendIntent;
        match intent {
            SendIntent::New => SendIntentClass::NoOp,
            SendIntent::Reply => SendIntentClass::MarkReplied,
            SendIntent::Forward => SendIntentClass::MarkForwarded,
        }
    }

    /// Star and read state must speak each provider's own flag vocabulary,
    /// because star now rides `bulk_set_flags` (one bulk campaign for an
    /// N-thread star, still chunked to the provider's batch ceiling) instead of
    /// the per-message `set_starred` convenience that did the
    /// `StarredFlagShape` capability dispatch for us. The flag string IS the
    /// capability dispatch now, so it is pinned per provider - and pinned
    /// against the read side, which is what makes a star survive a round trip.
    #[test]
    fn flag_vocabulary_matches_the_consumer_read_side() {
        for provider in [
            BifrostProviderKind::Gmail,
            BifrostProviderKind::Graph,
            BifrostProviderKind::Imap,
        ] {
            assert_eq!(
                starred_flag(provider),
                "\\Flagged",
                "{provider:?} bulk flag path canonicalizes the backslash form; \
                 `$flagged` would fall through as an unknown custom keyword"
            );
            assert_eq!(seen_flag(provider), "\\Seen");
        }
        assert_eq!(
            starred_flag(BifrostProviderKind::Jmap),
            "$flagged",
            "JMAP writes keywords verbatim, and the consumer reads `$flagged`"
        );
        assert_eq!(seen_flag(BifrostProviderKind::Jmap), "$seen");
    }

    /// `hydrate::normalized_flags` maps `\Flagged` -> `$flagged` for exactly the
    /// three non-JMAP providers, and `is_starred` is `flags.contains("$flagged")`
    /// for all four. Pinning the write vocabulary against that read rule here
    /// keeps the two halves from drifting independently.
    #[test]
    fn starred_write_flag_normalizes_to_the_read_key() {
        const READ_KEY: &str = "$flagged";
        for provider in [
            BifrostProviderKind::Gmail,
            BifrostProviderKind::Graph,
            BifrostProviderKind::Imap,
            BifrostProviderKind::Jmap,
        ] {
            let written = starred_flag(provider);
            let normalizes = written == READ_KEY
                || (provider != BifrostProviderKind::Jmap
                    && written.eq_ignore_ascii_case("\\flagged"));
            assert!(
                normalizes,
                "{provider:?} writes {written} which never normalizes to {READ_KEY}"
            );
        }
    }

    #[test]
    fn set_starred_and_set_read_both_add_or_remove_one_flag() {
        let FlagOp::Add(added) = flag_op("\\Flagged", true) else {
            panic!("starring must ADD the flag");
        };
        assert_eq!(added.len(), 1);
        assert!(added.contains("\\Flagged"));
        let FlagOp::Remove(removed) = flag_op("$flagged", false) else {
            panic!("un-starring must REMOVE the flag");
        };
        assert!(removed.contains("$flagged"));
    }

    fn move_to(dest: &str) -> ContainerMoveOp<'_> {
        ContainerMoveOp::MoveToFolder { dest, source: None }
    }

    fn move_from<'a>(dest: &'a str, source: &'a str) -> ContainerMoveOp<'a> {
        ContainerMoveOp::MoveToFolder {
            dest,
            source: Some(source),
        }
    }

    fn moves_to(destination: &str) -> Option<ContainerMovePlan> {
        Some(ContainerMovePlan::MoveTo {
            destination: destination.to_string(),
            source: None,
        })
    }

    /// Gmail archive must compose as an INBOX detach, never as a move onto the
    /// synthetic `archive` container id. That id exists only so bifrost's role
    /// table has an Archive entry; it is not a real Gmail label, and the bulk
    /// move path lowers the destination straight into `addLabelIds`.
    #[test]
    fn gmail_archive_detaches_inbox_rather_than_labelling_a_synthetic_id() {
        let detach_inbox = Some(ContainerMovePlan::DetachInbox("INBOX".to_string()));
        for (op, destination) in [
            (ContainerMoveOp::Archive, Some("archive")),
            // Same answer whether the Archive role resolved or not, and
            // case-insensitively.
            (ContainerMoveOp::Archive, None),
            (ContainerMoveOp::Archive, Some("ARCHIVE")),
            // A MoveToFolder that names the synthetic id is the same shape.
            (move_to("archive"), Some("archive")),
        ] {
            assert_eq!(
                container_move_plan(
                    BifrostProviderKind::Gmail,
                    op,
                    destination,
                    Some("INBOX"),
                    None
                ),
                detach_inbox,
                "{op:?} with destination {destination:?} must detach INBOX"
            );
        }
        // No INBOX container resolved: nothing to detach, so nothing to send.
        assert_eq!(
            container_move_plan(
                BifrostProviderKind::Gmail,
                ContainerMoveOp::Archive,
                Some("archive"),
                None,
                None
            ),
            Some(ContainerMovePlan::NoOp)
        );
    }

    /// REGRESSION PIN. The archive shape must be decided by the OPERATION, not
    /// by "this Gmail op has no destination". Trash and Spam resolve their
    /// destinations through the same role table, so a snapshot that has not yet
    /// seen the account's Trash role would otherwise (a) silently archive
    /// instead of trashing, and (b) by answering `Some`, suppress the caller's
    /// refresh-and-retry so the role never got re-resolved either.
    #[test]
    fn gmail_unresolved_non_archive_role_is_a_miss_not_an_archive() {
        for op in [
            ContainerMoveOp::Trash,
            ContainerMoveOp::Spam { to: true },
            ContainerMoveOp::Spam { to: false },
            move_to("Work"),
        ] {
            assert_eq!(
                container_move_plan(BifrostProviderKind::Gmail, op, None, Some("INBOX"), None),
                None,
                "{op:?} with an unresolved destination must force a container refresh, \
                 never degrade into a Gmail archive"
            );
        }
    }

    /// The correctness gap this pins: a non-Gmail account with no Archive-role
    /// folder must NOT fall back to "detach from INBOX". On IMAP that lowers to
    /// `\Deleted` + `UID EXPUNGE` - destroying an inbox message on archive where
    /// UIDPLUS / IMAP4rev2 allow the expunge, and otherwise leaving it `\Deleted`
    /// and exposed to the next expunge - and for a message in any other folder it
    /// fails outright on a batch-wide source that never matched its real folder.
    #[test]
    fn non_gmail_archive_without_an_archive_folder_is_unresolved_not_an_inbox_detach() {
        for provider in [
            BifrostProviderKind::Imap,
            BifrostProviderKind::Jmap,
            BifrostProviderKind::Graph,
        ] {
            assert_eq!(
                container_move_plan(
                    provider,
                    ContainerMoveOp::Archive,
                    None,
                    Some("INBOX"),
                    None
                ),
                None,
                "{provider:?} must surface an unresolved destination, not an INBOX detach"
            );
        }
    }

    /// Everything with a real destination is a bulk move, including a
    /// non-Gmail folder that happens to be NAMED `archive` - the synthetic-id
    /// special case is Gmail-only.
    #[test]
    fn resolved_destinations_always_bulk_move() {
        for provider in [
            BifrostProviderKind::Gmail,
            BifrostProviderKind::Graph,
            BifrostProviderKind::Imap,
            BifrostProviderKind::Jmap,
        ] {
            assert_eq!(
                container_move_plan(
                    provider,
                    ContainerMoveOp::Trash,
                    Some("Trash"),
                    Some("INBOX"),
                    None
                ),
                moves_to("Trash")
            );
        }
        assert_eq!(
            container_move_plan(
                BifrostProviderKind::Imap,
                ContainerMoveOp::Archive,
                Some("archive"),
                Some("INBOX"),
                None
            ),
            moves_to("archive")
        );
    }

    /// Gmail's move patch is destination-only (add the destination, remove
    /// INBOX), and a Gmail message id carries no source label, so the source
    /// container must ride the plan into `bulk_move_from` or it survives the
    /// move and the next sync restores the row the local write removed.
    #[test]
    fn gmail_move_carries_the_source_it_leaves() {
        assert_eq!(
            container_move_plan(
                BifrostProviderKind::Gmail,
                move_from("TRASH", "SPAM"),
                Some("TRASH"),
                Some("INBOX"),
                Some("SPAM")
            ),
            Some(ContainerMovePlan::MoveTo {
                destination: "TRASH".to_string(),
                source: Some("SPAM".to_string()),
            }),
            "trashing out of SPAM must clear SPAM"
        );
        // Un-spam: the destination IS Inbox, so the patch removes nothing at
        // all and SPAM would otherwise stick.
        assert_eq!(
            container_move_plan(
                BifrostProviderKind::Gmail,
                ContainerMoveOp::Spam { to: false },
                Some("INBOX"),
                Some("INBOX"),
                Some("SPAM")
            ),
            Some(ContainerMovePlan::MoveTo {
                destination: "INBOX".to_string(),
                source: Some("SPAM".to_string()),
            })
        );
    }

    /// The plan passes the source through UNFILTERED for every provider - the
    /// exclusions the dispatch used to apply are bifrost's now. Gmail's
    /// `move_patch` drops a redundant source (INBOX on a non-INBOX
    /// destination, source == destination, the synthetic archive id) itself,
    /// and the folder-model providers' default `bulk_move_from` ignores the
    /// source entirely, so no wire op is added anywhere by carrying it.
    #[test]
    fn move_plan_passes_the_source_through_for_bifrost_to_filter() {
        for provider in [
            BifrostProviderKind::Gmail,
            BifrostProviderKind::Graph,
            BifrostProviderKind::Imap,
            BifrostProviderKind::Jmap,
        ] {
            assert_eq!(
                container_move_plan(
                    provider,
                    move_from("TRASH", "INBOX"),
                    Some("TRASH"),
                    Some("INBOX"),
                    Some("INBOX")
                ),
                Some(ContainerMovePlan::MoveTo {
                    destination: "TRASH".to_string(),
                    source: Some("INBOX".to_string()),
                }),
                "{provider:?} plan carries the source verbatim"
            );
        }
    }

    /// Un-spamming moves SPAM -> INBOX, so its destination role is Inbox. Kept
    /// as its own pinned mapping because reading it off the op is the one place
    /// a polarity flip would silently send un-spam back to SPAM.
    #[test]
    fn container_ops_target_the_expected_role() {
        assert_eq!(
            destination_role(ContainerMoveOp::Archive),
            Some(FolderRole::Archive)
        );
        assert_eq!(
            destination_role(ContainerMoveOp::Trash),
            Some(FolderRole::Trash)
        );
        assert_eq!(
            destination_role(ContainerMoveOp::Spam { to: true }),
            Some(FolderRole::Spam)
        );
        assert_eq!(
            destination_role(ContainerMoveOp::Spam { to: false }),
            Some(FolderRole::Inbox)
        );
        assert_eq!(
            destination_role(move_to("f1")),
            None,
            "an explicit move destination is a storage id, not a role"
        );
    }

    /// Un-spam is the ONLY role-shaped source. Trash and mark-as-spam leave
    /// user labels attached, which is native Gmail behaviour and matches what
    /// `trash_local` / `spam_local` do locally (they only drop INBOX), so
    /// inventing a source for them would delete a label the user still expects.
    #[test]
    fn only_unspam_carries_a_role_shaped_source() {
        assert_eq!(
            source_role(ContainerMoveOp::Spam { to: false }),
            Some(FolderRole::Spam)
        );
        for op in [
            ContainerMoveOp::Archive,
            ContainerMoveOp::Trash,
            ContainerMoveOp::Spam { to: true },
            move_to("f1"),
        ] {
            assert_eq!(source_role(op), None, "{op:?} must not invent a source");
        }
    }

    /// The coalescing key has to carry the source, because on Gmail the source
    /// decides the wire patch. Two moves to the same destination out of
    /// different sources are different wire ops and must land in different
    /// batches; moves out of the same folder view still coalesce.
    #[test]
    fn move_batch_key_separates_distinct_sources() {
        use common::typed_ids::FolderId;
        let key = |source: Option<&str>| {
            RemoteBatchKey::from_operation(&MailOperation::MoveToFolder {
                dest: FolderId::from("TRASH"),
                source: source.map(FolderId::from),
            })
        };
        assert_ne!(key(Some("SPAM")), key(Some("INBOX")));
        assert_ne!(key(Some("SPAM")), key(None));
        assert_eq!(key(Some("SPAM")), key(Some("SPAM")));
    }

    #[test]
    fn send_intent_maps_to_engine_mark() {
        use service_api::actions::SendIntent;
        let cases = [
            (SendIntent::New, SendIntentClass::NoOp),
            (SendIntent::Reply, SendIntentClass::MarkReplied),
            (SendIntent::Forward, SendIntentClass::MarkForwarded),
        ];
        for (intent, expected) in cases {
            assert_eq!(
                send_intent_class(intent),
                expected,
                "unexpected engine mark for {intent:?}"
            );
        }
    }
}
