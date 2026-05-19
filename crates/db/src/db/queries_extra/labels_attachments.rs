use super::super::WriterPool;
use super::super::types::{AttachmentSender, AttachmentWithContext};
use rusqlite::params;

pub async fn db_get_attachments_for_account(
    db: &WriterPool,
    account_id: String,
    limit: i64,
    offset: i64,
) -> Result<Vec<AttachmentWithContext>, String> {
    db.with_write(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT a.id, a.message_id, a.account_id, a.filename, a.mime_type, a.size,
                            a.remote_attachment_id, a.content_id, a.is_inline,
                            a.content_hash,
                            m.from_address, m.from_name, m.date, m.subject, m.thread_id
                     FROM attachments a
                     JOIN messages m ON a.message_id = m.id AND a.account_id = m.account_id
                     WHERE a.account_id = ?1 AND a.filename IS NOT NULL AND a.filename != ''
                     ORDER BY m.date DESC
                     LIMIT ?2 OFFSET ?3",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id, limit, offset], |row| {
            Ok(AttachmentWithContext {
                id: row.get("id")?,
                message_id: row.get("message_id")?,
                account_id: row.get("account_id")?,
                filename: row.get("filename")?,
                mime_type: row.get("mime_type")?,
                size: row.get("size")?,
                remote_attachment_id: row.get("remote_attachment_id")?,
                content_id: row.get("content_id")?,
                is_inline: row.get("is_inline")?,
                content_hash: row.get("content_hash")?,
                from_address: row.get("from_address")?,
                from_name: row.get("from_name")?,
                date: row.get("date")?,
                subject: row.get("subject")?,
                thread_id: row.get("thread_id")?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_attachment_senders(
    db: &WriterPool,
    account_id: String,
) -> Result<Vec<AttachmentSender>, String> {
    db.with_write(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT m.from_address, m.from_name, COUNT(*) as count
                     FROM attachments a
                     JOIN messages m ON a.message_id = m.id AND a.account_id = m.account_id
                     WHERE a.account_id = ?1 AND a.filename IS NOT NULL AND a.filename != ''
                       AND m.from_address IS NOT NULL
                     GROUP BY m.from_address
                     ORDER BY count DESC",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id], |row| {
            Ok(AttachmentSender {
                from_address: row.get("from_address")?,
                from_name: row.get("from_name")?,
                count: row.get("count")?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
    })
    .await
}
