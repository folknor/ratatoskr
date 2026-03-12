use super::super::DbState;
use super::super::types::{AttachmentSender, AttachmentWithContext, LabelSortOrderItem};
use rusqlite::params;

pub async fn db_upsert_label_coalesce(
    db: &DbState,
    id: String,
    account_id: String,
    name: String,
    label_type: String,
    color_bg: Option<String>,
    color_fg: Option<String>,
    imap_folder_path: Option<String>,
    imap_special_use: Option<String>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO labels (id, account_id, name, type, color_bg, color_fg, imap_folder_path, imap_special_use)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(account_id, id) DO UPDATE SET
                   name = ?3, type = ?4, color_bg = ?5, color_fg = ?6,
                   imap_folder_path = COALESCE(?7, imap_folder_path),
                   imap_special_use = COALESCE(?8, imap_special_use)",
            params![id, account_id, name, label_type, color_bg, color_fg, imap_folder_path, imap_special_use],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_delete_labels_for_account(db: &DbState, account_id: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM labels WHERE account_id = ?1",
            params![account_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_update_label_sort_order(
    db: &DbState,
    account_id: String,
    label_orders: Vec<LabelSortOrderItem>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for item in &label_orders {
            tx.execute(
                "UPDATE labels SET sort_order = ?1 WHERE account_id = ?2 AND id = ?3",
                params![item.sort_order, account_id, item.id],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_upsert_attachment(
    db: &DbState,
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
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO attachments (id, message_id, account_id, filename, mime_type, size, gmail_attachment_id, content_id, is_inline)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(id) DO UPDATE SET
                   filename = ?4, mime_type = ?5, size = ?6,
                   gmail_attachment_id = ?7, content_id = ?8, is_inline = ?9",
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
    db: &DbState,
    account_id: String,
    limit: i64,
    offset: i64,
) -> Result<Vec<AttachmentWithContext>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT a.id, a.message_id, a.account_id, a.filename, a.mime_type, a.size,
                            a.gmail_attachment_id, a.content_id, a.is_inline, a.local_path,
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
                gmail_attachment_id: row.get("gmail_attachment_id")?,
                content_id: row.get("content_id")?,
                is_inline: row.get("is_inline")?,
                local_path: row.get("local_path")?,
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
    db: &DbState,
    account_id: String,
) -> Result<Vec<AttachmentSender>, String> {
    db.with_conn(move |conn| {
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
