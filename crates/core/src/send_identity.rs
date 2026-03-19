use rusqlite::{Connection, params};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A row from the `send_identities` table.
#[derive(Debug, Clone)]
pub struct SendIdentity {
    pub id: i64,
    pub account_id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub mailbox_id: Option<String>,
    pub send_mode: String,
    pub is_primary: bool,
}

/// Context used by [`select_from_address`] to pick the best identity.
#[derive(Debug, Clone, Default)]
pub struct FromSelectionContext {
    /// The To/Cc addresses of the original message (for replies).
    /// These are the addresses the original was *sent to* — one of them
    /// should become the From if the user owns it.
    pub reply_to_addresses: Vec<String>,
    /// Set when composing from within a shared/delegate mailbox.
    pub shared_mailbox_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

/// Return all send identities for the given account, ordered so that the
/// primary identity comes first.
pub fn get_send_identities(
    conn: &Connection,
    account_id: &str,
) -> rusqlite::Result<Vec<SendIdentity>> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, email, display_name, mailbox_id, send_mode, is_primary
         FROM send_identities
         WHERE account_id = ?1
         ORDER BY is_primary DESC, id ASC",
    )?;

    let rows = stmt.query_map(params![account_id], |row| {
        Ok(SendIdentity {
            id: row.get("id")?,
            account_id: row.get("account_id")?,
            email: row.get("email")?,
            display_name: row.get("display_name")?,
            mailbox_id: row.get("mailbox_id")?,
            send_mode: row.get("send_mode")?,
            is_primary: row.get::<_, i64>("is_primary")? != 0,
        })
    })?;

    rows.collect()
}

// ---------------------------------------------------------------------------
// Auto-From selection
// ---------------------------------------------------------------------------

/// Pick the best `SendIdentity` for composing/replying.
///
/// Priority:
/// 1. If `shared_mailbox_id` is set, find the identity whose `mailbox_id`
///    matches.
/// 2. If replying, match `reply_to_addresses` against known identities
///    (case-insensitive) — the address the original was sent *to* becomes
///    the From.
/// 3. Fall back to the account's primary identity (`is_primary = 1`).
pub fn select_from_address(
    conn: &Connection,
    account_id: &str,
    context: &FromSelectionContext,
) -> rusqlite::Result<Option<SendIdentity>> {
    let identities = get_send_identities(conn, account_id)?;

    if identities.is_empty() {
        return Ok(None);
    }

    // 1. Shared mailbox match
    if let Some(ref mb_id) = context.shared_mailbox_id {
        if let Some(hit) = identities
            .iter()
            .find(|i| i.mailbox_id.as_deref() == Some(mb_id.as_str()))
        {
            return Ok(Some(hit.clone()));
        }
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

    // 3. Primary identity (first in the list since we ORDER BY is_primary DESC)
    Ok(identities.into_iter().find(|i| i.is_primary))
}
