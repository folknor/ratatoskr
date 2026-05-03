//! Pure mail operation - the canonical "what to do" type.
//!
//! Owned by core. Contains only execution semantics - no UI metadata,
//! no undo provenance, no completion behavior. The app layer resolves
//! user intent into a `MailOperation` before dispatching to core.

use common::typed_ids::{FolderId, TagId};

/// A fully resolved, unambiguous mail operation.
///
/// Every variant is a concrete instruction that can be executed without
/// any additional UI context. Toggle directions are resolved, folder
/// IDs are captured, label IDs are typed.
///
/// `PartialEq` + `Eq` enable the batch executor to group identical
/// operations for future provider-level batching (IMAP STORE, Graph
/// $batch, JMAP Email/set).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailOperation {
    Archive,
    Trash,
    PermanentDelete,
    SetSpam {
        to: bool,
    },
    SetStarred {
        to: bool,
    },
    SetRead {
        to: bool,
    },
    SetPinned {
        to: bool,
    },
    SetMuted {
        to: bool,
    },
    MoveToFolder {
        dest: FolderId,
        source: Option<FolderId>,
    },
    AddLabel {
        label_id: TagId,
    },
    RemoveLabel {
        label_id: TagId,
    },
    Snooze {
        until: i64,
    },
}
