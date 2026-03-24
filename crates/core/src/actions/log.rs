//! Structured mutation logging for the action service.
//!
//! Every action function emits exactly one `MutationLog` entry per invocation
//! via `emit()`. Replaces the 30+ ad-hoc `log::warn!` calls with a consistent
//! format that includes duration, identity, and error classification.

use std::time::Instant;

use super::outcome::{ActionError, ActionOutcome};

/// Structured log entry for an action service mutation.
///
/// Each action function creates one of these, fills in the fields, and calls
/// `emit()` at the end. The log level is determined by the outcome:
/// - `Success` → `info`
/// - `LocalOnly` → `warn`
/// - `Failed` → `error`
pub struct MutationLog {
    pub action: &'static str,
    pub account_id: String,
    pub local_id: String,
    pub remote_id: Option<String>,
    pub started: Instant,
}

impl MutationLog {
    /// Create a new log entry. Call `emit()` when the action is complete.
    pub fn begin(action: &'static str, account_id: &str, local_id: &str) -> Self {
        Self {
            action,
            account_id: account_id.to_string(),
            local_id: local_id.to_string(),
            remote_id: None,
            started: Instant::now(),
        }
    }

    /// Set the remote resource ID (if known after provider dispatch).
    pub fn with_remote_id(mut self, id: impl Into<String>) -> Self {
        self.remote_id = Some(id.into());
        self
    }

    /// Emit the structured log entry based on the action outcome.
    pub fn emit(&self, outcome: &ActionOutcome) {
        let duration_ms = self.started.elapsed().as_millis();
        let remote = self.remote_id.as_deref().unwrap_or("-");

        match outcome {
            ActionOutcome::Success => {
                log::info!(
                    "[action] {action} ok | account={account} local={local} remote={remote} | {duration_ms}ms",
                    action = self.action,
                    account = self.account_id,
                    local = self.local_id,
                );
            }
            ActionOutcome::LocalOnly {
                reason, retryable, ..
            } => {
                log::warn!(
                    "[action] {action} local-only (retryable={retryable}) | account={account} local={local} remote={remote} | {kind} | {duration_ms}ms | {reason}",
                    action = self.action,
                    account = self.account_id,
                    local = self.local_id,
                    kind = error_kind(reason),
                );
            }
            ActionOutcome::Failed { error } => {
                log::error!(
                    "[action] {action} failed | account={account} local={local} remote={remote} | {kind} | {duration_ms}ms | {error}",
                    action = self.action,
                    account = self.account_id,
                    local = self.local_id,
                    kind = error_kind(error),
                );
            }
        }
    }
}

/// Extract a short error kind label for structured logging.
fn error_kind(error: &ActionError) -> &'static str {
    match error {
        ActionError::Db(_) => "db",
        ActionError::Remote { kind, .. } => match kind {
            super::outcome::RemoteFailureKind::Transient => "remote/transient",
            super::outcome::RemoteFailureKind::Permanent => "remote/permanent",
            super::outcome::RemoteFailureKind::NotImplemented => "remote/not_implemented",
            super::outcome::RemoteFailureKind::Unknown => "remote/unknown",
        },
        ActionError::NotFound(_) => "not_found",
        ActionError::InvalidState(_) => "invalid_state",
        ActionError::Build(_) => "build",
    }
}
