/// Structured result of an email action.
#[derive(Debug, Clone)]
pub enum ActionOutcome {
    /// Local and remote both succeeded (or local-only-by-design succeeded).
    Success,
    /// Local succeeded, remote dispatch failed or was skipped.
    /// The action took effect locally but may revert on next sync.
    ///
    /// `retryable` indicates whether this failure is a candidate for
    /// automatic retry via the pending-ops queue (Phase 3.4). Classified
    /// per action class at the call site:
    /// - `true`: email actions (archive, trash, spam, move, star, mark_read,
    ///   label) with Transient or Unknown remote errors.
    /// - `false`: calendar create, contact save/delete (low priority),
    ///   NotImplemented stubs, Permanent errors.
    LocalOnly { reason: ActionError, retryable: bool },
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

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

// ── Error types ──────────────────────────────────────────

/// Structured error from an action service operation.
///
/// Provides machine-readable categorization and user-facing messages.
/// `user_message()` is an intermediate step — messages still incorporate
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
    /// Network error, timeout, 5xx — worth retrying.
    Transient,
    /// 4xx, permission denied, invalid request — won't succeed on retry.
    Permanent,
    /// Provider write-back not yet implemented (stub).
    NotImplemented,
    /// Unknown completion — provider error couldn't be classified.
    Unknown,
}

// ── Convenience constructors ─────────────────────────────

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
    /// This is an intermediate step — messages still incorporate internal
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
