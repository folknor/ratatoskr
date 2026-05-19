use crate::db::ReadConn;

#[derive(Debug, Clone)]
pub struct DbChatContactSummary {
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_path: Option<String>,
    pub latest_message_preview: Option<String>,
    pub latest_message_at: Option<i64>,
    pub unread_count: i64,
    pub sort_order: i64,
}

#[derive(Debug, Clone)]
pub struct DbChatMessage {
    pub message_id: String,
    pub account_id: String,
    pub thread_id: String,
    pub from_address: String,
    pub from_name: Option<String>,
    pub date: i64,
    pub subject: Option<String>,
    pub is_read: bool,
    pub message_id_header: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DbChatInlineImage {
    pub message_id: String,
    pub account_id: String,
    pub content_hash: String,
    pub mime_type: String,
}

pub fn get_chat_inline_images_sync(
    conn: &ReadConn<'_>,
    message_ids: &[String],
) -> Result<Vec<DbChatInlineImage>, String> {
    if message_ids.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders: Vec<String> = (0..message_ids.len()).map(|i| format!("?{}", i + 1)).collect();
    let placeholders_csv = placeholders.join(", ");
    let sql = format!(
        "SELECT message_id, account_id, content_hash, mime_type
         FROM attachments
         WHERE message_id IN ({placeholders_csv})
           AND is_inline = 1
           AND mime_type LIKE 'image/%'
           AND content_hash IS NOT NULL"
    );

    let params: Vec<&dyn rusqlite::types::ToSql> = message_ids
        .iter()
        .map(|s| s as &dyn rusqlite::types::ToSql)
        .collect();

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    stmt.query_map(params.as_slice(), |row| {
        Ok(DbChatInlineImage {
            message_id: row.get("message_id")?,
            account_id: row.get("account_id")?,
            content_hash: row
                .get::<_, crate::blob_hash::BlobHash>("content_hash")?
                .to_hex(),
            mime_type: row.get("mime_type")?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

pub fn get_chat_contacts_sync(conn: &ReadConn<'_>) -> Result<Vec<DbChatContactSummary>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT cc.email, cc.display_name, cc.latest_message_at,
                    cc.latest_message_preview, cc.unread_count, cc.sort_order,
                    (SELECT file_path FROM contact_photo_cache
                     WHERE email = cc.email
                     ORDER BY last_accessed_at DESC LIMIT 1) AS file_path
             FROM chat_contacts cc
             ORDER BY cc.sort_order ASC",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map([], |row| {
        Ok(DbChatContactSummary {
            email: row.get("email")?,
            display_name: row.get("display_name")?,
            latest_message_at: row.get("latest_message_at")?,
            latest_message_preview: row.get("latest_message_preview")?,
            unread_count: row.get::<_, i64>("unread_count")?,
            sort_order: row.get::<_, i64>("sort_order")?,
            avatar_path: row.get("file_path")?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

pub fn get_user_signature_texts_sync(conn: &ReadConn<'_>) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT body_text FROM signatures
             WHERE body_text IS NOT NULL AND TRIM(body_text) <> ''",
        )
        .map_err(|e| e.to_string())?;
    stmt.query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub fn get_chat_timeline_sync(
    conn: &ReadConn<'_>,
    email: &str,
    limit: usize,
    before: Option<(i64, String)>,
) -> Result<Vec<DbChatMessage>, String> {
    let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
    let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
        if let Some((before_ts, before_id)) = before {
            (
                "SELECT m.id, m.account_id, m.thread_id, m.from_address, m.from_name,
                    m.date, m.is_read, m.subject, m.message_id_header
                 FROM messages m
                 INNER JOIN threads t ON t.id = m.thread_id AND t.account_id = m.account_id
                 INNER JOIN thread_participants tp
                   ON tp.account_id = m.account_id AND tp.thread_id = m.thread_id
                 WHERE t.is_chat_thread = 1 AND tp.email = ?1
                   AND (m.date < ?2 OR (m.date = ?2 AND m.id < ?3))
                 ORDER BY m.date DESC, m.id DESC
                 LIMIT ?4"
                    .to_string(),
                vec![
                    Box::new(email.to_string()),
                    Box::new(before_ts),
                    Box::new(before_id),
                    Box::new(limit_i64),
                ],
            )
        } else {
            (
                "SELECT m.id, m.account_id, m.thread_id, m.from_address, m.from_name,
                    m.date, m.is_read, m.subject, m.message_id_header
                 FROM messages m
                 INNER JOIN threads t ON t.id = m.thread_id AND t.account_id = m.account_id
                 INNER JOIN thread_participants tp
                   ON tp.account_id = m.account_id AND tp.thread_id = m.thread_id
                 WHERE t.is_chat_thread = 1 AND tp.email = ?1
                 ORDER BY m.date DESC, m.id DESC
                 LIMIT ?2"
                    .to_string(),
                vec![Box::new(email.to_string()), Box::new(limit_i64)],
            )
        };

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(AsRef::as_ref).collect();

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    stmt.query_map(param_refs.as_slice(), |row| {
        let from_address = row
            .get::<_, Option<String>>("from_address")?
            .unwrap_or_default();

        Ok(DbChatMessage {
            message_id: row.get("id")?,
            account_id: row.get("account_id")?,
            thread_id: row.get("thread_id")?,
            from_address,
            from_name: row.get("from_name")?,
            date: row.get("date")?,
            subject: row.get("subject")?,
            is_read: row.get::<_, i64>("is_read")? != 0,
            message_id_header: row.get("message_id_header")?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}
