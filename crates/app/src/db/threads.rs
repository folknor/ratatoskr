use rusqlite::params;

use super::connection::Db;
use super::types::{*, row_to_thread};

impl Db {
    pub async fn get_threads(
        &self,
        account_id: String,
        label_id: Option<String>,
        limit: i64,
    ) -> Result<Vec<Thread>, String> {
        self.with_conn(move |conn| {
            let (sql, do_label) = if label_id.is_some() {
                (
                    "SELECT t.*, m.from_name, m.from_address FROM threads t
                     INNER JOIN thread_labels tl
                       ON tl.account_id = t.account_id AND tl.thread_id = t.id
                     LEFT JOIN messages m
                       ON m.account_id = t.account_id AND m.thread_id = t.id
                       AND m.date = (SELECT MAX(m2.date) FROM messages m2
                                     WHERE m2.account_id = t.account_id
                                       AND m2.thread_id = t.id)
                     WHERE t.account_id = ?1 AND tl.label_id = ?2
                     GROUP BY t.account_id, t.id
                     ORDER BY t.is_pinned DESC, t.last_message_at DESC
                     LIMIT ?3",
                    true,
                )
            } else {
                (
                    "SELECT t.*, m.from_name, m.from_address FROM threads t
                     LEFT JOIN messages m
                       ON m.account_id = t.account_id AND m.thread_id = t.id
                       AND m.date = (SELECT MAX(m2.date) FROM messages m2
                                     WHERE m2.account_id = t.account_id
                                       AND m2.thread_id = t.id)
                     WHERE t.account_id = ?1
                     ORDER BY t.is_pinned DESC, t.last_message_at DESC
                     LIMIT ?2",
                    false,
                )
            };

            let mut stmt =
                conn.prepare(sql).map_err(|e| e.to_string())?;

            let rows = if do_label {
                stmt.query_map(
                    params![account_id, label_id.unwrap_or_default(), limit],
                    row_to_thread,
                )
            } else {
                stmt.query_map(
                    params![account_id, limit],
                    row_to_thread,
                )
            };

            rows.map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
    }

    pub async fn get_thread_messages(
        &self,
        account_id: String,
        thread_id: String,
    ) -> Result<Vec<ThreadMessage>, String> {
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, thread_id, account_id, from_name, from_address,
                        to_addresses, date, subject, snippet, is_read, is_starred
                 FROM messages
                 WHERE account_id = ?1 AND thread_id = ?2
                 ORDER BY date DESC"
            ).map_err(|e| e.to_string())?;

            stmt.query_map(params![account_id, thread_id], |row| {
                Ok(ThreadMessage {
                    id: row.get("id")?,
                    thread_id: row.get("thread_id")?,
                    account_id: row.get("account_id")?,
                    from_name: row.get("from_name")?,
                    from_address: row.get("from_address")?,
                    to_addresses: row.get("to_addresses")?,
                    date: row.get("date")?,
                    subject: row.get("subject")?,
                    snippet: row.get("snippet")?,
                    is_read: row.get::<_, i64>("is_read")? != 0,
                    is_starred: row.get::<_, i64>("is_starred")? != 0,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }

    pub async fn get_thread_attachments(
        &self,
        account_id: String,
        thread_id: String,
    ) -> Result<Vec<ThreadAttachment>, String> {
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT a.id, a.filename, a.mime_type, a.size,
                        m.from_name, m.date
                 FROM attachments a
                 JOIN messages m ON a.message_id = m.id AND a.account_id = m.account_id
                 WHERE a.account_id = ?1 AND m.thread_id = ?2
                   AND a.is_inline = 0
                   AND a.filename IS NOT NULL AND a.filename != ''
                 ORDER BY m.date DESC"
            ).map_err(|e| e.to_string())?;

            stmt.query_map(params![account_id, thread_id], |row| {
                Ok(ThreadAttachment {
                    id: row.get("id")?,
                    filename: row.get("filename")?,
                    mime_type: row.get("mime_type")?,
                    size: row.get("size")?,
                    from_name: row.get("from_name")?,
                    date: row.get("date")?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }

    /// Load the body (text + HTML) for a single message.
    pub async fn load_message_body(
        &self,
        account_id: String,
        message_id: String,
    ) -> Result<(Option<String>, Option<String>), String> {
        self.with_conn(move |conn| {
            let snippet: Option<String> = conn
                .query_row(
                    "SELECT snippet FROM messages
                     WHERE account_id = ?1 AND id = ?2",
                    params![account_id, message_id],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;

            Ok((snippet, None))
        })
        .await
    }

    /// Load attachments for a single message.
    pub async fn load_message_attachments(
        &self,
        account_id: String,
        message_id: String,
    ) -> Result<Vec<MessageViewAttachment>, String> {
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, filename, mime_type, size
                     FROM attachments
                     WHERE account_id = ?1 AND message_id = ?2
                       AND is_inline = 0
                       AND filename IS NOT NULL AND filename != ''
                     ORDER BY filename ASC",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map(params![account_id, message_id], |row| {
                Ok(MessageViewAttachment {
                    id: row.get("id")?,
                    filename: row.get("filename")?,
                    mime_type: row.get("mime_type")?,
                    size: row.get("size")?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }
}
