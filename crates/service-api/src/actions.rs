//! Action-pipeline DTOs shared across the app/service boundary.
//!
//! Owned here (the IPC + leaf-API crate) so the app does not need to
//! depend on `service` or `action-types` just to name `MailOperation`,
//! `SendIntent`, `ActionOutcome`, etc. The Service side re-exports
//! these through `service::actions::*` and `action-types::*` for code
//! that historically named them there.

pub use crate::action::SendIntent;
pub use types::{FolderId, LabelGroupId, LabelId};

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
        label_id: LabelId,
    },
    RemoveLabel {
        label_id: LabelId,
    },
    ApplyLabelGroup {
        group_id: LabelGroupId,
    },
    RemoveLabelGroup {
        group_id: LabelGroupId,
    },
    Snooze {
        until: i64,
    },
    /// Inverse of `Snooze` (Phase 2 task 14): restore a snoozed thread
    /// to the inbox and clear the snooze timestamp. Local-only - no
    /// provider has a universal snooze API, so the undo path is purely
    /// a DB mutation.
    Unsnooze,
}

/// Structured result of an email action.
#[derive(Debug, Clone)]
pub enum ActionOutcome {
    /// Local and remote both succeeded (or local-only-by-design succeeded).
    Success,
    /// The action was a no-op - state didn't change (e.g., archiving a thread
    /// already not in inbox). Provider dispatch and undo token skipped.
    NoOp,
    /// Local succeeded, remote dispatch failed or was skipped.
    /// The action took effect locally but may revert on next sync.
    ///
    /// `retryable` is a policy flag set per action class at the call site
    /// (email actions = true, contacts/calendar = false). The pending-ops
    /// worker (Phase 3.4) should also check `reason.is_retryable()` before
    /// actually enqueuing - a Permanent error shouldn't be retried even if
    /// the action class says "generally retry this action."
    LocalOnly {
        reason: ActionError,
        retryable: bool,
    },
    /// The action failed entirely (local not applied).
    Failed { error: ActionError },
}

impl ActionOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success)
    }

    pub fn is_local_only(&self) -> bool {
        matches!(self, Self::LocalOnly { .. })
    }

    pub fn is_noop(&self) -> bool {
        matches!(self, Self::NoOp)
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

// ── Error types ──────────────────────────────────────────

/// Structured error from an action service operation.
///
/// Provides machine-readable categorization and user-facing messages.
/// `user_message()` is an intermediate step - messages still incorporate
/// internal wording from provider errors. The structure enables future
/// refinement per-variant without changing the API.
#[derive(Debug, Clone)]
pub enum ActionError {
    /// Local database error (lock, query, constraint).
    Db(String),
    /// Remote provider operation failed.
    Remote {
        kind: RemoteFailureKind,
        message: String,
    },
    /// Resource not found (label, event, contact, draft, calendar).
    NotFound(String),
    /// State machine violation (e.g., draft already sending).
    InvalidState(String),
    /// Payload construction failed (MIME build, JSON serialization).
    Build(String),
}

/// Distinguishes retryable from permanent remote failures.
/// Used by Phase 3.4 to decide whether to enqueue a pending op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteFailureKind {
    /// Network error, timeout, 5xx - worth retrying.
    Transient,
    /// 4xx, permission denied, invalid request - won't succeed on retry.
    Permanent,
    /// Provider write-back not yet implemented (stub).
    NotImplemented,
    /// Unknown completion - provider error couldn't be classified.
    Unknown,
}

impl ActionError {
    /// Wrap a DB/lock/query error.
    pub fn db(msg: impl Into<String>) -> Self {
        Self::Db(msg.into())
    }

    /// Wrap a provider operation error. Defaults to `Unknown` kind
    /// since most provider errors are opaque strings.
    pub fn remote(msg: impl Into<String>) -> Self {
        Self::Remote {
            kind: RemoteFailureKind::Unknown,
            message: msg.into(),
        }
    }

    /// Wrap a provider error with explicit kind.
    pub fn remote_with_kind(kind: RemoteFailureKind, msg: impl Into<String>) -> Self {
        Self::Remote {
            kind,
            message: msg.into(),
        }
    }

    /// Provider write-back not yet implemented.
    pub fn not_implemented(msg: impl Into<String>) -> Self {
        Self::Remote {
            kind: RemoteFailureKind::NotImplemented,
            message: msg.into(),
        }
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::NotFound(msg.into())
    }

    pub fn invalid_state(msg: impl Into<String>) -> Self {
        Self::InvalidState(msg.into())
    }

    pub fn build(msg: impl Into<String>) -> Self {
        Self::Build(msg.into())
    }

    /// Whether this error is worth retrying (Transient or Unknown remote).
    /// Used by action functions to set `retryable` on `LocalOnly`.
    /// Permanent, NotImplemented, Db, NotFound, InvalidState, and Build
    /// errors are never retryable.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Remote {
                kind: RemoteFailureKind::Transient | RemoteFailureKind::Unknown,
                ..
            }
        )
    }

    /// User-facing summary for toast/status display.
    ///
    /// This is an intermediate step - messages still incorporate internal
    /// wording. The structure enables future refinement per-variant.
    pub fn user_message(&self) -> String {
        match self {
            Self::Db(msg) => format!("Database error: {msg}"),
            Self::Remote { kind, message } => match kind {
                RemoteFailureKind::Transient => format!("Network error: {message}"),
                RemoteFailureKind::Permanent => format!("Server rejected: {message}"),
                RemoteFailureKind::NotImplemented => {
                    format!("Not yet supported: {message}")
                }
                RemoteFailureKind::Unknown => format!("Sync error: {message}"),
            },
            Self::NotFound(what) => format!("Not found: {what}"),
            Self::InvalidState(msg) => msg.clone(),
            Self::Build(msg) => format!("Build error: {msg}"),
        }
    }
}

impl std::fmt::Display for ActionError {
    /// Display preserves the internal detail (variant + message) for logging.
    /// Use `user_message()` for UI-facing text.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Db(msg) => write!(f, "Db: {msg}"),
            Self::Remote { kind, message } => write!(f, "Remote({kind:?}): {message}"),
            Self::NotFound(what) => write!(f, "NotFound: {what}"),
            Self::InvalidState(msg) => write!(f, "InvalidState: {msg}"),
            Self::Build(msg) => write!(f, "Build: {msg}"),
        }
    }
}

impl std::error::Error for ActionError {}

// ── Send DTOs ────────────────────────────────────────────

/// Attachment payload for outgoing messages.
#[derive(Debug, Clone)]
pub struct SendAttachment {
    /// Original filename (e.g. `report.pdf`).
    pub filename: String,
    /// MIME type (e.g. `application/pdf`).
    pub mime_type: String,
    /// Raw file bytes. `Bytes` (not `Vec<u8>`) so the mapping into
    /// bifrost's `AttachmentInline` is an O(1) ref-count bump sharing one
    /// heap buffer, rather than a full re-copy of a (potentially 50 MB+)
    /// payload. See `crate::send::to_bifrost_send_request`.
    pub data: bytes::Bytes,
    /// Optional Content-ID for inline images (without angle brackets).
    pub content_id: Option<String>,
}

/// Everything needed to send a single email.
///
/// The UI (compose window) populates this from the local draft and
/// finalized HTML/plain-text bodies.
#[derive(Debug, Clone)]
pub struct SendRequest {
    /// Local draft ID (from `local_drafts` table).
    pub draft_id: String,
    /// Account this message is sent from.
    pub account_id: String,
    /// RFC 5322 `From` address (e.g. `"Alice <alice@example.com>"`).
    pub from: String,
    /// RFC 5322 `To` addresses.
    pub to: Vec<String>,
    /// RFC 5322 `Cc` addresses.
    pub cc: Vec<String>,
    /// RFC 5322 `Bcc` addresses.
    pub bcc: Vec<String>,
    /// Subject line.
    pub subject: Option<String>,
    /// HTML body (finalized via `finalize_compose_html`).
    pub body_html: String,
    /// Plain-text body (finalized via `finalize_compose_plain_text`).
    pub body_text: String,
    /// File attachments.
    pub attachments: Vec<SendAttachment>,
    /// `In-Reply-To` header value (Message-ID of the message being replied to).
    pub in_reply_to: Option<String>,
    /// `References` header value (space-separated Message-IDs).
    pub references: Option<String>,
    /// Provider thread ID (for threading on send).
    pub thread_id: Option<String>,
    /// Local DB message ID for the message being replied to or forwarded.
    pub source_message_id: Option<String>,
    /// Whether this send is new mail, a reply, or a forward.
    pub intent: SendIntent,
}
