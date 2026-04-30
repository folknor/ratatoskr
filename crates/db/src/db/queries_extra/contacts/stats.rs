use crate::db::DbState;
use crate::db::FromRow;
use crate::db::types::{ContactAttachmentRow, ContactStats, RecentThread, SameDomainContact};
use rusqlite::params;

pub async fn db_get_contact_stats(db: &DbState, email: String) -> Result<ContactStats, String> {
    log::debug!("Loading contact stats: email={email}");
    db.with_conn(move |conn| {
        let normalized = email.to_lowercase();
        conn.query_row(
            "SELECT COUNT(*) as cnt, MIN(date) as first_date, MAX(date) as last_date
                 FROM messages WHERE from_address = ?1",
            params![normalized],
            ContactStats::from_row,
        )
        .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_contacts_from_same_domain(
    db: &DbState,
    email: String,
    limit: Option<i64>,
) -> Result<Vec<SameDomainContact>, String> {
    db.with_conn(move |conn| {
        let normalized = email.to_lowercase();
        let domain = normalized
            .split('@')
            .nth(1)
            .map(|d| format!("%@{d}"))
            .unwrap_or_default();
        let lim = limit.unwrap_or(5);
        let mut stmt = conn
            .prepare(
                "SELECT email, display_name, avatar_url FROM contacts
                     WHERE email LIKE ?1 AND email != ?2
                     ORDER BY frequency DESC LIMIT ?3",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(
            params![domain, normalized, lim],
            SameDomainContact::from_row,
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_latest_auth_result(
    db: &DbState,
    email: String,
) -> Result<Option<String>, String> {
    db.with_conn(move |conn| {
        let normalized = email.to_lowercase();
        let result = conn
            .query_row(
                "SELECT auth_results FROM messages
                     WHERE from_address = ?1 AND auth_results IS NOT NULL
                     ORDER BY date DESC LIMIT 1",
                params![normalized],
                |row| row.get::<_, String>("auth_results"),
            )
            .ok();
        Ok(result)
    })
    .await
}

pub async fn db_get_recent_threads_with_contact(
    db: &DbState,
    email: String,
    limit: Option<i64>,
) -> Result<Vec<RecentThread>, String> {
    db.with_conn(move |conn| {
        let normalized = email.to_lowercase();
        let lim = limit.unwrap_or(5);
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT t.id as thread_id, t.subject, t.last_message_at
                     FROM threads t
                     INNER JOIN messages m ON m.account_id = t.account_id AND m.thread_id = t.id
                     WHERE m.from_address = ?1
                     ORDER BY t.last_message_at DESC LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![normalized, lim], RecentThread::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_attachments_from_contact(
    db: &DbState,
    email: String,
    limit: Option<i64>,
) -> Result<Vec<ContactAttachmentRow>, String> {
    db.with_conn(move |conn| {
        let normalized = email.to_lowercase();
        let lim = limit.unwrap_or(5);
        let mut stmt = conn
            .prepare(
                "SELECT a.filename, a.mime_type, a.size, m.date
                     FROM attachments a
                     INNER JOIN messages m ON m.account_id = a.account_id AND m.id = a.message_id
                     WHERE m.from_address = ?1 AND a.is_inline = 0 AND a.filename IS NOT NULL
                     ORDER BY m.date DESC LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![normalized, lim], ContactAttachmentRow::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}
