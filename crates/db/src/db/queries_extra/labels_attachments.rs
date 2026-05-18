use super::super::WriterPool;
use super::super::types::{AttachmentSender, AttachmentWithContext};
use rusqlite::params;

// The pre-split unified `labels`-table helpers
// (`db_upsert_label_coalesce`, `db_delete_labels_for_account`,
// `db_update_label_sort_order`) were removed in slice 2 of the
// labels-unification cleanup. They sniffed folder-vs-label at write time
// from a `label_type` string, blind-updated both `labels` and `folders`
// for the same `(account_id, id)`, and deleted from both tables under a
// name that said only labels. Post-split, all writers must call the typed
// helpers - `upsert_labels` / `insert_folders_batch` - so the table
// is structurally determined at the call site.

// TODO(refactor): wrap fields in an UpsertAttachmentParams struct.
#[allow(clippy::too_many_arguments)]
pub async fn db_upsert_attachment(
    db: &WriterPool,
    id: String,
    message_id: String,
    account_id: String,
    filename: Option<String>,
    mime_type: Option<String>,
    size: Option<i64>,
    attachment_id: Option<String>,
    content_id: Option<String>,
    is_inline: bool,
) -> Result<(), String> {
    db.with_write(move |conn| {
        conn.execute(
            "INSERT INTO attachments (id, message_id, account_id, filename, mime_type, size, remote_attachment_id, content_id, is_inline)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(id) DO UPDATE SET
                   filename = ?4, mime_type = ?5, size = ?6,
                   remote_attachment_id = ?7, content_id = ?8, is_inline = ?9",
            params![
                id,
                message_id,
                account_id,
                filename,
                mime_type,
                size,
                attachment_id,
                content_id,
                i64::from(is_inline),
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

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
