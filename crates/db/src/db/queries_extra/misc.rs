use super::super::DbState;
use super::super::types::{
    BackfillRow, CachedAttachmentRow, ImapMessageRow, SnoozedThread, SpecialFolderRow,
    SubscriptionEntry,
};
use crate::db::from_row::FromRow;
use rusqlite::params;

/// Backfill variant of the latest-message subquery that also selects `to_addresses`.
/// The standard `LATEST_MESSAGE_SUBQUERY` does not include this column.
const LATEST_MESSAGE_BACKFILL_SUBQUERY: &str = "\
SELECT id, account_id, thread_id, from_address, from_name, to_addresses FROM (
  SELECT id, account_id, thread_id, from_address, from_name, to_addresses,
         ROW_NUMBER() OVER (
           PARTITION BY account_id, thread_id
           ORDER BY date DESC, id DESC
         ) AS rn
  FROM messages
) WHERE rn = 1";

pub async fn db_get_snoozed_threads_due(
    db: &DbState,
    now: i64,
) -> Result<Vec<SnoozedThread>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, account_id FROM threads WHERE is_snoozed = 1 AND snooze_until <= ?1",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![now], SnoozedThread::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub fn get_calendar_default_view_sync(conn: &crate::db::Connection) -> Result<Option<String>, String> {
    use rusqlite::OptionalExtension;

    conn.query_row(
        "SELECT value FROM settings WHERE key = 'calendar_default_view'",
        [],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

#[allow(clippy::too_many_arguments)]
pub async fn db_record_unsubscribe_action(
    db: &DbState,
    id: String,
    account_id: String,
    thread_id: String,
    from_address: String,
    from_name: Option<String>,
    method: String,
    unsubscribe_url: String,
    status: String,
    now: i64,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO unsubscribe_actions (id, account_id, thread_id, from_address, from_name, method, unsubscribe_url, status, unsubscribed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(account_id, from_address) DO UPDATE SET
                   status = ?8, unsubscribed_at = ?9, method = ?6, thread_id = ?3",
            params![id, account_id, thread_id, from_address, from_name, method, unsubscribe_url, status, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_subscriptions(
    db: &DbState,
    account_id: String,
) -> Result<Vec<SubscriptionEntry>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT
                       m.from_address,
                       MAX(m.from_name) as from_name,
                       MAX(m.list_unsubscribe) as latest_unsubscribe_header,
                       MAX(m.list_unsubscribe_post) as latest_unsubscribe_post,
                       COUNT(*) as message_count,
                       MAX(m.date) as latest_date,
                       ua.status
                     FROM messages m
                     LEFT JOIN unsubscribe_actions ua ON ua.account_id = m.account_id AND ua.from_address = LOWER(m.from_address)
                     WHERE m.account_id = ?1 AND m.list_unsubscribe IS NOT NULL
                     GROUP BY LOWER(m.from_address)
                     ORDER BY MAX(m.date) DESC",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id], SubscriptionEntry::from_row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_unsubscribe_status(
    db: &DbState,
    account_id: String,
    from_address: String,
) -> Result<Option<String>, String> {
    let from_address = from_address.to_lowercase();
    db.with_conn(move |conn| {
        Ok(conn
            .query_row(
                "SELECT status FROM unsubscribe_actions WHERE account_id = ?1 AND from_address = ?2",
                params![account_id, from_address],
                |row| row.get("status"),
            )
            .ok())
    })
    .await
}

pub async fn db_get_imap_uids_for_messages(
    db: &DbState,
    account_id: String,
    message_ids: Vec<String>,
) -> Result<Vec<ImapMessageRow>, String> {
    if message_ids.is_empty() {
        return Ok(Vec::new());
    }
    db.with_conn(move |conn| {
        let placeholders: Vec<String> = (0..message_ids.len())
            .map(|i| format!("?{}", i + 2))
            .collect();
        let sql = format!(
            "SELECT id, imap_uid, imap_folder FROM messages WHERE account_id = ?1 AND id IN ({})",
            placeholders.join(", ")
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        param_values.push(Box::new(account_id));
        for id in &message_ids {
            param_values.push(Box::new(id.clone()));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = param_values
            .iter()
            .map(std::convert::AsRef::as_ref)
            .collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), ImapMessageRow::from_row)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_find_special_folder(
    db: &DbState,
    account_id: String,
    special_use: String,
    fallback_label_id: Option<String>,
) -> Result<Option<String>, String> {
    db.with_conn(move |conn| {
        let result: Option<SpecialFolderRow> = conn
            .query_row(
                "SELECT imap_folder_path, name FROM labels WHERE account_id = ?1 AND imap_special_use = ?2 LIMIT 1",
                params![account_id, special_use],
                SpecialFolderRow::from_row,
            )
            .ok();
        if let Some(row) = result {
            return Ok(Some(row.imap_folder_path.unwrap_or(row.name)));
        }
        if let Some(label_id) = fallback_label_id {
            let fallback: Option<SpecialFolderRow> = conn
                .query_row(
                    "SELECT imap_folder_path, name FROM labels WHERE account_id = ?1 AND id = ?2 AND imap_folder_path IS NOT NULL LIMIT 1",
                    params![account_id, label_id],
                    SpecialFolderRow::from_row,
                )
                .ok();
            if let Some(row) = fallback {
                return Ok(Some(row.imap_folder_path.unwrap_or(row.name)));
            }
        }
        Ok(None)
    })
    .await
}

pub async fn db_update_message_imap_folder(
    db: &DbState,
    account_id: String,
    message_ids: Vec<String>,
    new_folder: String,
) -> Result<(), String> {
    if message_ids.is_empty() {
        return Ok(());
    }
    db.with_conn(move |conn| {
        let placeholders: Vec<String> = (0..message_ids.len())
            .map(|i| format!("?{}", i + 3))
            .collect();
        let sql = format!(
            "UPDATE messages SET imap_folder = ?1 WHERE account_id = ?2 AND id IN ({})",
            placeholders.join(", ")
        );
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        param_values.push(Box::new(new_folder));
        param_values.push(Box::new(account_id));
        for id in &message_ids {
            param_values.push(Box::new(id.clone()));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = param_values
            .iter()
            .map(std::convert::AsRef::as_ref)
            .collect();
        conn.execute(&sql, param_refs.as_slice())
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_update_attachment_cached(
    db: &DbState,
    attachment_id: String,
    local_path: String,
    cache_size: i64,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE attachments SET local_path = ?1, cached_at = unixepoch(), cache_size = ?2 WHERE id = ?3",
            params![local_path, cache_size, attachment_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_attachment_cache_size(db: &DbState) -> Result<i64, String> {
    db.with_conn(move |conn| {
        let total: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cache_size), 0) AS total FROM attachments WHERE cached_at IS NOT NULL",
                [],
                |row| row.get("total"),
            )
            .map_err(|e| e.to_string())?;
        Ok(total)
    })
    .await
}

pub async fn db_get_oldest_cached_attachments(
    db: &DbState,
    limit: i64,
) -> Result<Vec<CachedAttachmentRow>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, local_path, cache_size, content_hash FROM attachments WHERE cached_at IS NOT NULL ORDER BY cached_at ASC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![limit], CachedAttachmentRow::from_row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_clear_attachment_cache_entry(
    db: &DbState,
    attachment_id: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE attachments SET local_path = NULL, cached_at = NULL, cache_size = NULL WHERE id = ?1",
            params![attachment_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_clear_all_attachment_cache(db: &DbState) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE attachments SET local_path = NULL, cached_at = NULL, cache_size = NULL WHERE cached_at IS NOT NULL",
            [],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_count_cached_by_hash(db: &DbState, content_hash: String) -> Result<i64, String> {
    db.with_conn(move |conn| {
        conn.query_row(
            "SELECT COUNT(*) AS cnt FROM attachments WHERE content_hash = ?1 AND cached_at IS NOT NULL",
            params![content_hash],
            |row| row.get("cnt"),
        )
        .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_inbox_threads_for_backfill(
    db: &DbState,
    account_id: String,
    batch_size: i64,
    offset: i64,
) -> Result<Vec<BackfillRow>, String> {
    db.with_conn(move |conn| {
        let sql = format!(
            "SELECT t.id AS thread_id, t.subject, t.snippet,
                        m.from_address, m.from_name,
                        m.to_addresses, t.has_attachments, m.id
                 FROM threads t
                 INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
                 LEFT JOIN ({LATEST_MESSAGE_BACKFILL_SUBQUERY}
                 ) m ON m.account_id = t.account_id AND m.thread_id = t.id
                 WHERE t.account_id = ?1 AND tl.label_id = 'INBOX'
                 ORDER BY t.last_message_at DESC
                 LIMIT ?2 OFFSET ?3"
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        stmt.query_map(
            params![account_id, batch_size, offset],
            BackfillRow::from_row,
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_update_scheduled_email_attachments(
    db: &DbState,
    account_id: String,
    attachment_data: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let id: Option<String> = conn
            .query_row(
                "SELECT id FROM scheduled_emails WHERE account_id = ?1 ORDER BY created_at DESC LIMIT 1",
                params![account_id],
                |row| row.get("id"),
            )
            .ok();
        if let Some(id) = id {
            conn.execute(
                "UPDATE scheduled_emails SET attachment_paths = ?1 WHERE id = ?2",
                params![attachment_data, id],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .await
}

pub async fn db_query_raw_select(
    db: &DbState,
    sql: String,
    params: Vec<serde_json::Value>,
) -> Result<Vec<serde_json::Map<String, serde_json::Value>>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let col_count = stmt.column_count();
        let col_names: Vec<String> = (0..col_count)
            .map(|i| stmt.column_name(i).map(ToString::to_string))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        let param_values: Vec<Box<dyn rusqlite::types::ToSql>> = params
            .iter()
            .map(|v| -> Box<dyn rusqlite::types::ToSql> {
                match v {
                    serde_json::Value::Null => Box::new(Option::<String>::None),
                    serde_json::Value::Bool(b) => Box::new(*b),
                    serde_json::Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            Box::new(i)
                        } else if let Some(f) = n.as_f64() {
                            Box::new(f)
                        } else {
                            Box::new(n.to_string())
                        }
                    }
                    serde_json::Value::String(s) => Box::new(s.clone()),
                    other => Box::new(other.to_string()),
                }
            })
            .collect();
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = param_values
            .iter()
            .map(std::convert::AsRef::as_ref)
            .collect();

        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                let mut map = serde_json::Map::new();
                for name in &col_names {
                    let val: rusqlite::types::Value = row.get(name.as_str())?;
                    let json_val = match val {
                        rusqlite::types::Value::Null => serde_json::Value::Null,
                        rusqlite::types::Value::Integer(n) => serde_json::Value::Number(n.into()),
                        rusqlite::types::Value::Real(f) => serde_json::json!(f),
                        rusqlite::types::Value::Text(s) => serde_json::Value::String(s),
                        rusqlite::types::Value::Blob(b) => {
                            serde_json::Value::String(base64_encode(&b))
                        }
                    };
                    map.insert(name.clone(), json_val);
                }
                Ok(map)
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

fn base64_encode(data: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(data.len() * 4 / 3 + 4);
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = if chunk.len() > 1 {
            chunk[1] as usize
        } else {
            0
        };
        let b2 = if chunk.len() > 2 {
            chunk[2] as usize
        } else {
            0
        };
        let _ = write!(s, "{}", CHARS[(b0 >> 2) & 0x3F] as char);
        let _ = write!(s, "{}", CHARS[((b0 << 4) | (b1 >> 4)) & 0x3F] as char);
        if chunk.len() > 1 {
            let _ = write!(s, "{}", CHARS[((b1 << 2) | (b2 >> 6)) & 0x3F] as char);
        } else {
            let _ = write!(s, "=");
        }
        if chunk.len() > 2 {
            let _ = write!(s, "{}", CHARS[b2 & 0x3F] as char);
        } else {
            let _ = write!(s, "=");
        }
    }
    s
}
