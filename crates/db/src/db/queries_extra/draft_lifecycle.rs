//! Local draft lifecycle: persist, status transitions, and deletion.
//!
//! The `local_drafts` table tracks drafts from creation through sending.
//! Status flow: pending → sending → sent (or failed at any point).

use rusqlite::{Connection, params};

use super::super::from_row::FromRow;

/// Persist a draft as 'pending' (upsert — retries update the existing row).
#[allow(clippy::too_many_arguments)]
pub fn persist_draft_pending_sync(
    conn: &Connection,
    id: &str,
    account_id: &str,
    to_addresses: &str,
    cc_addresses: &str,
    bcc_addresses: &str,
    subject: Option<&str>,
    body_html: &str,
    reply_to_message_id: Option<&str>,
    thread_id: Option<&str>,
    from_email: &str,
    attachments: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO local_drafts \
         (id, account_id, to_addresses, cc_addresses, bcc_addresses, \
          subject, body_html, reply_to_message_id, thread_id, \
          from_email, attachments, updated_at, sync_status) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, \
                 unixepoch(), 'pending') \
         ON CONFLICT(id) DO UPDATE SET \
           to_addresses = ?3, cc_addresses = ?4, bcc_addresses = ?5, \
           subject = ?6, body_html = ?7, reply_to_message_id = ?8, \
           thread_id = ?9, from_email = ?10, attachments = ?11, \
           updated_at = unixepoch(), sync_status = 'pending'",
        params![
            id,
            account_id,
            to_addresses,
            cc_addresses,
            bcc_addresses,
            subject,
            body_html,
            reply_to_message_id,
            thread_id,
            from_email,
            attachments,
        ],
    )
    .map_err(|e| format!("draft persist: {e}"))?;
    Ok(())
}

/// Transition a draft to 'sending'. Returns Ok(()) if the transition
/// succeeded, or Err if the draft was not found or already sending/sent.
pub fn mark_draft_sending_sync(conn: &Connection, draft_id: &str) -> Result<(), String> {
    let rows = conn
        .execute(
            "UPDATE local_drafts SET sync_status = 'sending' \
             WHERE id = ?1 AND sync_status IN ('pending', 'synced', 'failed')",
            params![draft_id],
        )
        .map_err(|e| format!("mark sending: {e}"))?;
    if rows == 0 {
        return Err(format!(
            "Draft {draft_id} not found or already sending/sent"
        ));
    }
    Ok(())
}

/// Transition a draft to 'sent' with the provider-assigned message ID.
pub fn mark_draft_sent_sync(
    conn: &Connection,
    draft_id: &str,
    sent_message_id: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE local_drafts SET sync_status = 'sent', remote_draft_id = ?1 \
         WHERE id = ?2",
        params![sent_message_id, draft_id],
    )
    .map_err(|e| format!("mark sent: {e}"))?;
    Ok(())
}

/// Transition a draft to 'failed'.
pub fn mark_draft_failed_sync(conn: &Connection, draft_id: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE local_drafts SET sync_status = 'failed' WHERE id = ?1",
        params![draft_id],
    )
    .map_err(|e| format!("mark failed: {e}"))?;
    Ok(())
}

/// Look up the remote draft ID for a local draft. Returns None if not found.
pub fn get_remote_draft_id_sync(
    conn: &Connection,
    draft_id: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT remote_draft_id FROM local_drafts WHERE id = ?1",
        params![draft_id],
        |row| row.get(0),
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        _ => Err(format!("draft lookup: {e}")),
    })
}

// ---------------------------------------------------------------------------
// Scheduled email helpers
// ---------------------------------------------------------------------------

/// Get overdue locally-delegated scheduled emails.
pub fn get_overdue_local_scheduled_sync(
    conn: &Connection,
    now_unix: i64,
) -> Result<Vec<super::super::types::DbScheduledEmail>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT * FROM scheduled_emails \
             WHERE status = 'pending' AND delegation = 'local' AND scheduled_at <= ?1 \
             ORDER BY scheduled_at ASC",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map(
        params![now_unix],
        super::super::types::DbScheduledEmail::from_row,
    )
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

/// Mark a scheduled email as delegated to the server.
pub fn mark_scheduled_delegated_sync(
    conn: &Connection,
    email_id: &str,
    remote_message_id: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE scheduled_emails SET status = 'delegated', remote_message_id = ?1 WHERE id = ?2",
        params![remote_message_id, email_id],
    )
    .map_err(|e| format!("mark delegated: {e}"))?;
    Ok(())
}

/// Record a scheduled send failure.
pub fn mark_scheduled_failed_sync(
    conn: &Connection,
    email_id: &str,
    error_message: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE scheduled_emails SET status = 'failed', error_message = ?1, retry_count = retry_count + 1 WHERE id = ?2",
        params![error_message, email_id],
    )
    .map_err(|e| format!("mark failed: {e}"))?;
    Ok(())
}

/// Delete a local draft by ID.
pub fn delete_draft_sync(conn: &Connection, draft_id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM local_drafts WHERE id = ?1",
        params![draft_id],
    )
    .map_err(|e| format!("draft delete: {e}"))?;
    Ok(())
}
