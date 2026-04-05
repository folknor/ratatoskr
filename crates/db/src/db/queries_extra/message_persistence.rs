use rusqlite::{Transaction, params};

#[derive(Debug, Clone)]
pub struct MessageInsertRow {
    pub id: String,
    pub account_id: String,
    pub thread_id: String,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub to_addresses: Option<String>,
    pub cc_addresses: Option<String>,
    pub bcc_addresses: Option<String>,
    pub reply_to: Option<String>,
    pub subject: Option<String>,
    pub snippet: String,
    pub date: i64,
    pub is_read: bool,
    pub is_starred: bool,
    pub raw_size: Option<i64>,
    pub internal_date: Option<i64>,
    pub list_unsubscribe: Option<String>,
    pub list_unsubscribe_post: Option<String>,
    pub auth_results: Option<String>,
    pub message_id_header: Option<String>,
    pub references_header: Option<String>,
    pub in_reply_to_header: Option<String>,
    pub body_cached: bool,
    pub mdn_requested: bool,
    pub is_reaction: bool,
    pub imap_uid: Option<i64>,
    pub imap_folder: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AttachmentInsertRow {
    pub id: String,
    pub message_id: String,
    pub account_id: String,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size: Option<i64>,
    pub remote_attachment_id: Option<String>,
    pub content_hash: Option<String>,
    pub content_id: Option<String>,
    pub is_inline: bool,
}

pub fn insert_messages(tx: &Transaction, rows: &[MessageInsertRow]) -> Result<(), String> {
    for row in rows {
        tx.execute(
            "INSERT OR REPLACE INTO messages \
             (id, account_id, thread_id, from_address, from_name, to_addresses, \
              cc_addresses, bcc_addresses, reply_to, subject, snippet, date, \
              is_read, is_starred, raw_size, internal_date, \
              list_unsubscribe, list_unsubscribe_post, auth_results, \
              message_id_header, references_header, in_reply_to_header, body_cached, \
              mdn_requested, is_reaction, imap_uid, imap_folder) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, \
                     ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, \
                     ?24, ?25, ?26, ?27)",
            params![
                row.id,
                row.account_id,
                row.thread_id,
                row.from_address,
                row.from_name,
                row.to_addresses,
                row.cc_addresses,
                row.bcc_addresses,
                row.reply_to,
                row.subject,
                row.snippet,
                row.date,
                row.is_read,
                row.is_starred,
                row.raw_size,
                row.internal_date,
                row.list_unsubscribe,
                row.list_unsubscribe_post,
                row.auth_results,
                row.message_id_header,
                row.references_header,
                row.in_reply_to_header,
                if row.body_cached { 1i64 } else { 0i64 },
                row.mdn_requested,
                if row.is_reaction { 1i64 } else { 0i64 },
                row.imap_uid,
                row.imap_folder,
            ],
        )
        .map_err(|e| format!("upsert message: {e}"))?;
    }

    Ok(())
}

pub fn insert_attachments(tx: &Transaction, rows: &[AttachmentInsertRow]) -> Result<(), String> {
    for row in rows {
        tx.execute(
            "INSERT INTO attachments \
             (id, message_id, account_id, filename, mime_type, size, \
              gmail_attachment_id, content_hash, content_id, is_inline) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
             ON CONFLICT(id) DO UPDATE SET \
               filename = ?4, mime_type = ?5, size = ?6, \
               gmail_attachment_id = ?7, content_hash = ?8, content_id = ?9, is_inline = ?10",
            params![
                row.id,
                row.message_id,
                row.account_id,
                row.filename,
                row.mime_type,
                row.size,
                row.remote_attachment_id,
                row.content_hash,
                row.content_id,
                row.is_inline,
            ],
        )
        .map_err(|e| format!("upsert attachment: {e}"))?;
    }

    Ok(())
}
