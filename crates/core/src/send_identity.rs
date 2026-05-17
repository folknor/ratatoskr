//! Send identity selection.
//!
//! Queries live in `db::queries_extra::send_identity`. This module
//! keeps the selection algorithm and domain types.

use crate::db::ReadConn;

// Re-export from db (flat re-exports via queries_extra::*).
pub use crate::db::queries_extra::SendIdentity;

pub fn get_all_send_identity_emails(conn: &ReadConn<'_>) -> Result<Vec<String>, String> {
    crate::db::queries_extra::get_all_send_identity_emails_read(conn)
}

/// Context used by [`select_from_address`] to pick the best identity.
#[derive(Debug, Clone, Default)]
pub struct FromSelectionContext {
    pub reply_to_addresses: Vec<String>,
    pub shared_mailbox_id: Option<String>,
}

/// Pick the best `SendIdentity` for composing/replying.
///
/// Priority:
/// 1. Shared mailbox match
/// 2. Reply-address match (case-insensitive)
/// 3. Primary identity
pub fn select_from_address(
    conn: &ReadConn<'_>,
    account_id: &str,
    context: &FromSelectionContext,
) -> Result<Option<SendIdentity>, String> {
    let identities = crate::db::queries_extra::get_send_identities_read(conn, account_id)?;

    if identities.is_empty() {
        return Ok(None);
    }

    // 1. Shared mailbox match
    if let Some(ref mb_id) = context.shared_mailbox_id
        && let Some(hit) = identities
            .iter()
            .find(|i| i.mailbox_id.as_deref() == Some(mb_id.as_str()))
    {
        return Ok(Some(hit.clone()));
    }

    // 2. Reply-address match (case-insensitive)
    if !context.reply_to_addresses.is_empty() {
        let lower: Vec<String> = context
            .reply_to_addresses
            .iter()
            .map(|a| a.to_lowercase())
            .collect();

        if let Some(hit) = identities
            .iter()
            .find(|i| lower.contains(&i.email.to_lowercase()))
        {
            return Ok(Some(hit.clone()));
        }
    }

    // 3. Primary identity
    Ok(identities.into_iter().find(|i| i.is_primary))
}
