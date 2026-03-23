/// Structured result of an email action.
///
/// Phase 1: minimal shape. Phase 3 expands with structured error types,
/// user-facing result categories, and undo context.
///
/// Note: `String` error fields are temporary. If Phase 2 migration across
/// ~10 actions causes friction with stringly-typed errors, introduce a
/// structured error enum early.
#[derive(Debug, Clone)]
pub enum ActionOutcome {
    /// Local and remote both succeeded.
    Success,
    /// Local succeeded, remote dispatch failed.
    /// The action took effect locally but may revert on next sync.
    LocalOnly { remote_error: String },
    /// The action failed entirely (local not applied).
    Failed { error: String },
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
