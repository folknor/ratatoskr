//! Provider-aware scheduled send delegation routing and overdue handling.
//!
//! Determines whether a scheduled email should be delegated to the server
//! (Exchange via `PidTagDeferredSendTime`, JMAP via FUTURERELEASE) or handled
//! locally by a client-side timer (Gmail, IMAP).

use serde::{Deserialize, Serialize};

use crate::db::DbState;
use crate::db::types::DbScheduledEmail;

// ── Delegation types ────────────────────────────────────────

/// How a scheduled email should be delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendDelegation {
    /// Client-side timer sends at the scheduled time (Gmail, IMAP).
    Local,
    /// Exchange server holds the message via `PidTagDeferredSendTime`.
    Exchange,
    /// JMAP server holds via FUTURERELEASE / `holduntil` parameter.
    Jmap,
}

impl SendDelegation {
    /// Database string representation matching the `delegation` column.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Exchange => "exchange",
            Self::Jmap => "jmap",
        }
    }

    /// Parse from the database `delegation` column value.
    pub fn from_str(s: &str) -> Self {
        match s {
            "exchange" => Self::Exchange,
            "jmap" => Self::Jmap,
            _ => Self::Local,
        }
    }
}

// ── Status types ────────────────────────────────────────────

/// Lifecycle status of a scheduled email.
///
/// Status flows:
/// - Server delegation: `pending → delegated → sent`
/// - Local timer:       `pending → sending → sent`
/// - Failure:           `… → failed`
/// - Overdue >24h:      `… → needs_review`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledStatus {
    /// Waiting to be picked up by the scheduler.
    Pending,
    /// Handed off to server (Exchange/JMAP) — server will deliver at the
    /// scheduled time.
    Delegated,
    /// Local timer is actively sending the message right now.
    Sending,
    /// Successfully delivered.
    Sent,
    /// Send attempt failed (see `error_message` column).
    Failed,
    /// Overdue by more than 24 hours — requires user review.
    NeedsReview,
    /// User cancelled before send.
    Cancelled,
}

impl ScheduledStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Delegated => "delegated",
            Self::Sending => "sending",
            Self::Sent => "sent",
            Self::Failed => "failed",
            Self::NeedsReview => "needs_review",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "delegated" => Some(Self::Delegated),
            "sending" => Some(Self::Sending),
            "sent" => Some(Self::Sent),
            "failed" => Some(Self::Failed),
            "needs_review" => Some(Self::NeedsReview),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

// ── Overdue check result ────────────────────────────────────

/// Result of checking a single overdue scheduled email.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverdueAction {
    pub email: DbScheduledEmail,
    pub action: OverdueResolution,
}

/// What to do with an overdue scheduled email.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverdueResolution {
    /// Overdue by less than 24 hours — send immediately.
    SendNow,
    /// Overdue by more than 24 hours — flag for user review.
    NeedsReview,
}

// ── Delegation routing ──────────────────────────────────────

/// Maximum age (seconds) before an overdue email requires user review
/// instead of being auto-sent.
const OVERDUE_REVIEW_THRESHOLD_SECS: i64 = 24 * 60 * 60;

/// Determine the delegation strategy for a scheduled send based on the
/// account's provider type.
///
/// Provider type strings match the `provider` column in the `accounts` table:
/// - `"graph"` → Exchange server delegation
/// - `"jmap"` → JMAP server delegation (caller should verify FUTURERELEASE
///   capability via `maxDelayedSend`; this function returns `Jmap` optimistically)
/// - `"gmail"`, `"imap"`, or anything else → local timer
pub fn determine_send_delegation(provider_type: &str) -> SendDelegation {
    match provider_type {
        "graph" => SendDelegation::Exchange,
        "jmap" => SendDelegation::Jmap,
        // Gmail API and IMAP have no server-side scheduled send
        _ => SendDelegation::Local,
    }
}

/// Determine delegation for an account by looking up its provider in the DB.
pub async fn determine_send_delegation_for_account(
    db: &DbState,
    account_id: &str,
) -> Result<SendDelegation, String> {
    let provider = crate::db::queries::get_provider_type(db, account_id).await?;
    Ok(determine_send_delegation(&provider))
}

// ── Overdue handling ────────────────────────────────────────

/// Check for overdue locally-delegated scheduled emails and classify them.
///
/// Only locally-delegated emails (`delegation = 'local'`) with status
/// `'pending'` are considered. Server-delegated emails are handled by the
/// remote server and don't need local overdue checks.
///
/// Returns a list of overdue emails with the recommended action:
/// - `SendNow` if overdue by less than 24 hours
/// - `NeedsReview` if overdue by more than 24 hours
pub async fn check_overdue_scheduled_emails(
    db: &DbState,
    now_unix: i64,
) -> Result<Vec<OverdueAction>, String> {
    db.with_conn(move |conn| {
        let emails =
            crate::db::queries_extra::draft_lifecycle::get_overdue_local_scheduled_sync(
                conn, now_unix,
            )?;

        let mut actions = Vec::new();
        for email in emails {
            let overdue_secs = now_unix - email.scheduled_at;
            let action = if overdue_secs > OVERDUE_REVIEW_THRESHOLD_SECS {
                OverdueResolution::NeedsReview
            } else {
                OverdueResolution::SendNow
            };
            actions.push(OverdueAction { email, action });
        }
        Ok(actions)
    })
    .await
}

/// Apply the overdue resolution by updating the email's status in the DB.
///
/// - `SendNow` → sets status to `"sending"` (caller should then actually send)
/// - `NeedsReview` → sets status to `"needs_review"`
pub async fn apply_overdue_resolution(
    db: &DbState,
    email_id: String,
    resolution: OverdueResolution,
) -> Result<(), String> {
    let new_status = match resolution {
        OverdueResolution::SendNow => ScheduledStatus::Sending.as_str(),
        OverdueResolution::NeedsReview => ScheduledStatus::NeedsReview.as_str(),
    };
    crate::db::queries_extra::db_update_scheduled_email_status(db, email_id, new_status.to_string())
        .await
}

/// Process all overdue scheduled emails: auto-send those within 24h,
/// flag the rest for review.
///
/// Returns the list of emails marked as `SendNow` (status set to `"sending"`)
/// that the caller should actually dispatch.
pub async fn process_overdue_emails(
    db: &DbState,
    now_unix: i64,
) -> Result<Vec<DbScheduledEmail>, String> {
    let actions = check_overdue_scheduled_emails(db, now_unix).await?;
    let mut ready_to_send = Vec::new();

    for action in actions {
        let email_id = action.email.id.clone();
        apply_overdue_resolution(db, email_id, action.action).await?;
        if action.action == OverdueResolution::SendNow {
            ready_to_send.push(action.email);
        }
    }

    Ok(ready_to_send)
}

/// Update a scheduled email's delegation info after server delegation succeeds.
///
/// Sets the status to `"delegated"` and records the remote message ID.
pub async fn mark_delegated(
    db: &DbState,
    email_id: String,
    remote_message_id: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        crate::db::queries_extra::draft_lifecycle::mark_scheduled_delegated_sync(
            conn,
            &email_id,
            &remote_message_id,
        )
    })
    .await
}

/// Record a send failure with error details and increment the retry count.
pub async fn mark_failed(
    db: &DbState,
    email_id: String,
    error_message: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        crate::db::queries_extra::draft_lifecycle::mark_scheduled_failed_sync(
            conn,
            &email_id,
            &error_message,
        )
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_determine_delegation_graph() {
        assert_eq!(determine_send_delegation("graph"), SendDelegation::Exchange);
    }

    #[test]
    fn test_determine_delegation_jmap() {
        assert_eq!(determine_send_delegation("jmap"), SendDelegation::Jmap);
    }

    #[test]
    fn test_determine_delegation_gmail() {
        assert_eq!(determine_send_delegation("gmail"), SendDelegation::Local);
    }

    #[test]
    fn test_determine_delegation_imap() {
        assert_eq!(determine_send_delegation("imap"), SendDelegation::Local);
    }

    #[test]
    fn test_determine_delegation_unknown() {
        assert_eq!(
            determine_send_delegation("something_else"),
            SendDelegation::Local
        );
    }

    #[test]
    fn test_delegation_roundtrip() {
        for d in [
            SendDelegation::Local,
            SendDelegation::Exchange,
            SendDelegation::Jmap,
        ] {
            assert_eq!(SendDelegation::from_str(d.as_str()), d);
        }
    }

    #[test]
    fn test_status_roundtrip() {
        for s in [
            ScheduledStatus::Pending,
            ScheduledStatus::Delegated,
            ScheduledStatus::Sending,
            ScheduledStatus::Sent,
            ScheduledStatus::Failed,
            ScheduledStatus::NeedsReview,
            ScheduledStatus::Cancelled,
        ] {
            assert_eq!(ScheduledStatus::from_str(s.as_str()), Some(s));
        }
    }

    #[test]
    fn test_status_from_str_unknown() {
        assert_eq!(ScheduledStatus::from_str("bogus"), None);
    }
}
