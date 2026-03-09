// specta::specta attribute generates code that trips let_underscore_must_use
#![allow(clippy::let_underscore_must_use)]

use rusqlite::{params, Row};
use tauri::State;

use super::DbState;
use super::types::{CategoryCount, DbLabel, DbMessage, DbThread, SettingRow, ThreadCategoryRow};

// ── Row mappers ──────────────────────────────────────────────

fn row_to_thread(row: &Row<'_>) -> rusqlite::Result<DbThread> {
    Ok(DbThread {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        subject: row.get("subject")?,
        snippet: row.get("snippet")?,
        last_message_at: row.get("last_message_at")?,
        message_count: row.get("message_count")?,
        is_read: row.get::<_, i64>("is_read")? != 0,
        is_starred: row.get::<_, i64>("is_starred")? != 0,
        is_important: row.get::<_, i64>("is_important")? != 0,
        has_attachments: row.get::<_, i64>("has_attachments")? != 0,
        is_snoozed: row.get::<_, i64>("is_snoozed")? != 0,
        snooze_until: row.get("snooze_until")?,
        is_pinned: row.get::<_, i64>("is_pinned")? != 0,
        is_muted: row.get::<_, i64>("is_muted")? != 0,
        from_name: row.get("from_name")?,
        from_address: row.get("from_address")?,
    })
}

fn row_to_message(row: &Row<'_>) -> rusqlite::Result<DbMessage> {
    Ok(DbMessage {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        thread_id: row.get("thread_id")?,
        from_address: row.get("from_address")?,
        from_name: row.get("from_name")?,
        to_addresses: row.get("to_addresses")?,
        cc_addresses: row.get("cc_addresses")?,
        bcc_addresses: row.get("bcc_addresses")?,
        reply_to: row.get("reply_to")?,
        subject: row.get("subject")?,
        snippet: row.get("snippet")?,
        date: row.get("date")?,
        is_read: row.get::<_, i64>("is_read")? != 0,
        is_starred: row.get::<_, i64>("is_starred")? != 0,
        body_html: row.get("body_html")?,
        body_text: row.get("body_text")?,
        body_cached: row.get::<_, Option<i64>>("body_cached")?.map(|v| v != 0),
        raw_size: row.get("raw_size")?,
        internal_date: row.get("internal_date")?,
        list_unsubscribe: row.get("list_unsubscribe")?,
        list_unsubscribe_post: row.get("list_unsubscribe_post")?,
        auth_results: row.get("auth_results")?,
        message_id_header: row.get("message_id_header")?,
        references_header: row.get("references_header")?,
        in_reply_to_header: row.get("in_reply_to_header")?,
        imap_uid: row.get("imap_uid")?,
        imap_folder: row.get("imap_folder")?,
    })
}

fn row_to_label(row: &Row<'_>) -> rusqlite::Result<DbLabel> {
    Ok(DbLabel {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        name: row.get("name")?,
        label_type: row.get("type")?,
        color_bg: row.get("color_bg")?,
        color_fg: row.get("color_fg")?,
        visible: row.get::<_, i64>("visible")? != 0,
        sort_order: row.get("sort_order")?,
        imap_folder_path: row.get("imap_folder_path")?,
        imap_special_use: row.get("imap_special_use")?,
    })
}

// ── Thread queries ───────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub async fn db_get_threads(
    state: State<'_, DbState>,
    account_id: String,
    label_id: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DbThread>, String> {
    let conn = state.conn().await;
    let lim = limit.unwrap_or(50);
    let off = offset.unwrap_or(0);

    let result = if let Some(ref lid) = label_id {
        let mut stmt = conn
            .prepare(
                "SELECT t.*, m.from_name, m.from_address FROM threads t
                 INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
                 LEFT JOIN messages m ON m.account_id = t.account_id AND m.thread_id = t.id
                   AND m.date = (SELECT MAX(m2.date) FROM messages m2
                                 WHERE m2.account_id = t.account_id AND m2.thread_id = t.id)
                 WHERE t.account_id = ?1 AND tl.label_id = ?2
                 GROUP BY t.account_id, t.id
                 ORDER BY t.is_pinned DESC, t.last_message_at DESC
                 LIMIT ?3 OFFSET ?4",
            )
            .map_err(|e| e.to_string())?;

        stmt.query_map(params![account_id, lid, lim, off], row_to_thread)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT t.*, m.from_name, m.from_address FROM threads t
                 LEFT JOIN messages m ON m.account_id = t.account_id AND m.thread_id = t.id
                   AND m.date = (SELECT MAX(m2.date) FROM messages m2
                                 WHERE m2.account_id = t.account_id AND m2.thread_id = t.id)
                 WHERE t.account_id = ?1
                 ORDER BY t.is_pinned DESC, t.last_message_at DESC
                 LIMIT ?2 OFFSET ?3",
            )
            .map_err(|e| e.to_string())?;

        stmt.query_map(params![account_id, lim, off], row_to_thread)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };

    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub async fn db_get_threads_for_category(
    state: State<'_, DbState>,
    account_id: String,
    category: String,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DbThread>, String> {
    let conn = state.conn().await;
    let lim = limit.unwrap_or(50);
    let off = offset.unwrap_or(0);

    let result = if category == "Primary" {
        let mut stmt = conn
            .prepare(
                "SELECT t.*, m.from_name, m.from_address FROM threads t
                 INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
                 LEFT JOIN thread_categories tc ON tc.account_id = t.account_id AND tc.thread_id = t.id
                 LEFT JOIN messages m ON m.account_id = t.account_id AND m.thread_id = t.id
                   AND m.date = (SELECT MAX(m2.date) FROM messages m2
                                 WHERE m2.account_id = t.account_id AND m2.thread_id = t.id)
                 WHERE t.account_id = ?1 AND tl.label_id = 'INBOX'
                   AND (tc.category IS NULL OR tc.category = 'Primary')
                 GROUP BY t.account_id, t.id
                 ORDER BY t.is_pinned DESC, t.last_message_at DESC
                 LIMIT ?2 OFFSET ?3",
            )
            .map_err(|e| e.to_string())?;

        stmt.query_map(params![account_id, lim, off], row_to_thread)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT t.*, m.from_name, m.from_address FROM threads t
                 INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
                 INNER JOIN thread_categories tc ON tc.account_id = t.account_id AND tc.thread_id = t.id
                 LEFT JOIN messages m ON m.account_id = t.account_id AND m.thread_id = t.id
                   AND m.date = (SELECT MAX(m2.date) FROM messages m2
                                 WHERE m2.account_id = t.account_id AND m2.thread_id = t.id)
                 WHERE t.account_id = ?1 AND tl.label_id = 'INBOX' AND tc.category = ?2
                 GROUP BY t.account_id, t.id
                 ORDER BY t.is_pinned DESC, t.last_message_at DESC
                 LIMIT ?3 OFFSET ?4",
            )
            .map_err(|e| e.to_string())?;

        stmt.query_map(params![account_id, category, lim, off], row_to_thread)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };

    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub async fn db_get_thread_by_id(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<Option<DbThread>, String> {
    let conn = state.conn().await;
    let mut stmt = conn
        .prepare(
            "SELECT t.*, m.from_name, m.from_address FROM threads t
             LEFT JOIN messages m ON m.account_id = t.account_id AND m.thread_id = t.id
               AND m.date = (SELECT MAX(m2.date) FROM messages m2
                             WHERE m2.account_id = t.account_id AND m2.thread_id = t.id)
             WHERE t.account_id = ?1 AND t.id = ?2
             LIMIT 1",
        )
        .map_err(|e| e.to_string())?;

    let mut rows = stmt
        .query_map(params![account_id, thread_id], row_to_thread)
        .map_err(|e| e.to_string())?;

    match rows.next() {
        Some(Ok(t)) => Ok(Some(t)),
        Some(Err(e)) => Err(e.to_string()),
        None => Ok(None),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn db_get_thread_label_ids(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<Vec<String>, String> {
    let conn = state.conn().await;
    let mut stmt = conn
        .prepare("SELECT label_id FROM thread_labels WHERE account_id = ?1 AND thread_id = ?2")
        .map_err(|e| e.to_string())?;

    let ids = stmt
        .query_map(params![account_id, thread_id], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(ids)
}

// ── Message queries ──────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub async fn db_get_messages_for_thread(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<Vec<DbMessage>, String> {
    let conn = state.conn().await;
    let mut stmt = conn
        .prepare(
            "SELECT * FROM messages WHERE account_id = ?1 AND thread_id = ?2 ORDER BY date ASC",
        )
        .map_err(|e| e.to_string())?;

    let msgs = stmt
        .query_map(params![account_id, thread_id], row_to_message)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(msgs)
}

// ── Label queries ────────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub async fn db_get_labels(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbLabel>, String> {
    let conn = state.conn().await;
    let mut stmt = conn
        .prepare("SELECT * FROM labels WHERE account_id = ?1 ORDER BY sort_order ASC, name ASC")
        .map_err(|e| e.to_string())?;

    let labels = stmt
        .query_map(params![account_id], row_to_label)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(labels)
}

// ── Settings queries ─────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub async fn db_get_setting(
    state: State<'_, DbState>,
    key: String,
) -> Result<Option<String>, String> {
    let conn = state.conn().await;
    let result = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .ok();
    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub async fn db_get_all_settings(state: State<'_, DbState>) -> Result<Vec<SettingRow>, String> {
    let conn = state.conn().await;
    let mut stmt = conn
        .prepare("SELECT key, value FROM settings")
        .map_err(|e| e.to_string())?;

    let settings = stmt
        .query_map([], |row| {
            Ok(SettingRow {
                key: row.get(0)?,
                value: row.get(1)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(settings)
}

#[tauri::command]
#[specta::specta]
pub async fn db_set_setting(
    state: State<'_, DbState>,
    key: String,
    value: String,
) -> Result<(), String> {
    let conn = state.conn().await;
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
        params![key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ── Thread category queries ──────────────────────────────────

#[tauri::command]
#[specta::specta]
pub async fn db_get_category_unread_counts(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<CategoryCount>, String> {
    let conn = state.conn().await;
    let mut stmt = conn
        .prepare(
            "SELECT tc.category, COUNT(*) as count
             FROM threads t
             INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
             LEFT JOIN thread_categories tc ON tc.account_id = t.account_id AND tc.thread_id = t.id
             WHERE t.account_id = ?1 AND tl.label_id = 'INBOX' AND t.is_read = 0
             GROUP BY tc.category",
        )
        .map_err(|e| e.to_string())?;

    let counts = stmt
        .query_map(params![account_id], |row| {
            Ok(CategoryCount {
                category: row.get(0)?,
                count: row.get(1)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(counts)
}

#[tauri::command]
#[specta::specta]
pub async fn db_get_categories_for_threads(
    state: State<'_, DbState>,
    account_id: String,
    thread_ids: Vec<String>,
) -> Result<Vec<ThreadCategoryRow>, String> {
    if thread_ids.is_empty() {
        return Ok(Vec::new());
    }

    let conn = state.conn().await;
    let mut all_results = Vec::new();

    // Batch in groups of 100 to stay within SQLite variable limits
    for chunk in thread_ids.chunks(100) {
        let placeholders: String = chunk
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 2))
            .collect::<Vec<_>>()
            .join(", ");

        let sql = format!(
            "SELECT thread_id, category FROM thread_categories WHERE account_id = ?1 AND thread_id IN ({placeholders})"
        );

        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;

        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        param_values.push(Box::new(account_id.clone()));
        for id in chunk {
            param_values.push(Box::new(id.clone()));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(AsRef::as_ref).collect();

        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok(ThreadCategoryRow {
                    thread_id: row.get(0)?,
                    category: row.get(1)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        all_results.extend(rows);
    }

    Ok(all_results)
}
