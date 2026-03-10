// tauri::command macro generates code that trips let_underscore_must_use
#![allow(clippy::let_underscore_must_use)]

use rusqlite::{Row, params};
use tauri::State;

use super::DbState;
use super::types::{
    CategoryCount, DbAttachment, DbContact, DbLabel, DbMessage, DbThread, SettingRow,
    ThreadCategoryRow,
};

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
        body_html: None,
        body_text: None,
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

pub(crate) fn row_to_contact(row: &Row<'_>) -> rusqlite::Result<DbContact> {
    Ok(DbContact {
        id: row.get("id")?,
        email: row.get("email")?,
        display_name: row.get("display_name")?,
        avatar_url: row.get("avatar_url")?,
        frequency: row.get("frequency")?,
        last_contacted_at: row.get("last_contacted_at")?,
        notes: row.get("notes")?,
    })
}

fn row_to_attachment(row: &Row<'_>) -> rusqlite::Result<DbAttachment> {
    Ok(DbAttachment {
        id: row.get("id")?,
        message_id: row.get("message_id")?,
        account_id: row.get("account_id")?,
        filename: row.get("filename")?,
        mime_type: row.get("mime_type")?,
        size: row.get("size")?,
        gmail_attachment_id: row.get("gmail_attachment_id")?,
        content_id: row.get("content_id")?,
        is_inline: row.get::<_, i64>("is_inline")? != 0,
        local_path: row.get("local_path")?,
    })
}

// ── Thread queries ───────────────────────────────────────────

#[tauri::command]
pub async fn db_get_threads(
    state: State<'_, DbState>,
    account_id: String,
    label_id: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DbThread>, String> {
    state
        .with_conn(move |conn| {
            let lim = limit.unwrap_or(50);
            let off = offset.unwrap_or(0);

            if let Some(ref lid) = label_id {
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
                    .map_err(|e| e.to_string())
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
                    .map_err(|e| e.to_string())
            }
        })
        .await
}

#[tauri::command]
pub async fn db_get_threads_for_category(
    state: State<'_, DbState>,
    account_id: String,
    category: String,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DbThread>, String> {
    state
        .with_conn(move |conn| {
            let lim = limit.unwrap_or(50);
            let off = offset.unwrap_or(0);

            if category == "Primary" {
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
                    .map_err(|e| e.to_string())
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
                    .map_err(|e| e.to_string())
            }
        })
        .await
}

#[tauri::command]
pub async fn db_get_thread_by_id(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<Option<DbThread>, String> {
    state
        .with_conn(move |conn| {
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
        })
        .await
}

#[tauri::command]
pub async fn db_get_thread_label_ids(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<Vec<String>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT label_id FROM thread_labels WHERE account_id = ?1 AND thread_id = ?2",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map(params![account_id, thread_id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
}

// ── Message queries ──────────────────────────────────────────

#[tauri::command]
pub async fn db_get_messages_for_thread(
    state: State<'_, DbState>,
    body_store: State<'_, crate::body_store::BodyStoreState>,
    account_id: String,
    thread_id: String,
) -> Result<Vec<DbMessage>, String> {
    // 1. Fetch message metadata from the metadata DB
    let mut messages: Vec<DbMessage> = state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM messages WHERE account_id = ?1 AND thread_id = ?2 ORDER BY date ASC",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map(params![account_id, thread_id], row_to_message)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await?;

    // 2. Hydrate bodies from the body store
    let ids_needing_bodies: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();

    if !ids_needing_bodies.is_empty() {
        let bodies = body_store.get_batch(ids_needing_bodies).await?;
        let body_map: std::collections::HashMap<String, crate::body_store::MessageBody> = bodies
            .into_iter()
            .map(|b| (b.message_id.clone(), b))
            .collect();

        for msg in &mut messages {
            if let Some(body) = body_map.get(&msg.id) {
                msg.body_html = body.body_html.clone();
                msg.body_text = body.body_text.clone();
            }
        }
    }

    Ok(messages)
}

// ── Label queries ────────────────────────────────────────────

#[tauri::command]
pub async fn db_get_labels(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbLabel>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM labels WHERE account_id = ?1 ORDER BY sort_order ASC, name ASC",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map(params![account_id], row_to_label)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}

// ── Settings queries ─────────────────────────────────────────

#[tauri::command]
pub async fn db_get_setting(
    state: State<'_, DbState>,
    key: String,
) -> Result<Option<String>, String> {
    state
        .with_conn(move |conn| {
            let result = conn
                .query_row(
                    "SELECT value FROM settings WHERE key = ?1",
                    params![key],
                    |row| row.get::<_, String>(0),
                )
                .ok();
            Ok(result)
        })
        .await
}

#[tauri::command]
pub async fn db_get_all_settings(state: State<'_, DbState>) -> Result<Vec<SettingRow>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare("SELECT key, value FROM settings")
                .map_err(|e| e.to_string())?;

            stmt.query_map([], |row| {
                Ok(SettingRow {
                    key: row.get(0)?,
                    value: row.get(1)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_set_setting(
    state: State<'_, DbState>,
    key: String,
    value: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
                params![key, value],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ── Thread category queries ──────────────────────────────────

#[tauri::command]
pub async fn db_get_category_unread_counts(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<CategoryCount>, String> {
    state
        .with_conn(move |conn| {
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

            stmt.query_map(params![account_id], |row| {
                Ok(CategoryCount {
                    category: row.get(0)?,
                    count: row.get(1)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_get_categories_for_threads(
    state: State<'_, DbState>,
    account_id: String,
    thread_ids: Vec<String>,
) -> Result<Vec<ThreadCategoryRow>, String> {
    if thread_ids.is_empty() {
        return Ok(Vec::new());
    }

    state
        .with_conn(move |conn| {
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
        })
        .await
}

// ── Thread mutations ─────────────────────────────────────────

#[tauri::command]
pub async fn db_set_thread_read(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    is_read: bool,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE threads SET is_read = ?3 WHERE account_id = ?1 AND id = ?2",
                params![account_id, thread_id, is_read],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_set_thread_starred(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    is_starred: bool,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE threads SET is_starred = ?3 WHERE account_id = ?1 AND id = ?2",
                params![account_id, thread_id, is_starred],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_set_thread_pinned(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    is_pinned: bool,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE threads SET is_pinned = ?3 WHERE account_id = ?1 AND id = ?2",
                params![account_id, thread_id, is_pinned],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_set_thread_muted(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    is_muted: bool,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE threads SET is_muted = ?3 WHERE account_id = ?1 AND id = ?2",
                params![account_id, thread_id, is_muted],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_delete_thread(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "DELETE FROM threads WHERE account_id = ?1 AND id = ?2",
                params![account_id, thread_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ── Thread label mutations ───────────────────────────────────

#[tauri::command]
pub async fn db_add_thread_label(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    label_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT OR IGNORE INTO thread_labels (account_id, thread_id, label_id) VALUES (?1, ?2, ?3)",
                params![account_id, thread_id, label_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_remove_thread_label(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    label_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "DELETE FROM thread_labels WHERE account_id = ?1 AND thread_id = ?2 AND label_id = ?3",
                params![account_id, thread_id, label_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ── Label mutations ──────────────────────────────────────────

#[tauri::command]
pub async fn db_upsert_label(state: State<'_, DbState>, label: DbLabel) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO labels (account_id, id, name, type, color_bg, color_fg, visible, sort_order, imap_folder_path, imap_special_use)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    label.account_id,
                    label.id,
                    label.name,
                    label.label_type,
                    label.color_bg,
                    label.color_fg,
                    label.visible,
                    label.sort_order,
                    label.imap_folder_path,
                    label.imap_special_use
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_delete_label(
    state: State<'_, DbState>,
    account_id: String,
    label_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "DELETE FROM labels WHERE account_id = ?1 AND id = ?2",
                params![account_id, label_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ── Contact queries ─────────────────────────────────────────

#[tauri::command]
pub async fn db_search_contacts(
    state: State<'_, DbState>,
    query: String,
    limit: i64,
) -> Result<Vec<DbContact>, String> {
    state
        .with_conn(move |conn| {
            let pattern = format!("%{query}%");

            let mut stmt = conn
                .prepare(
                    "SELECT * FROM contacts
                 WHERE email LIKE ?1 OR display_name LIKE ?1
                 ORDER BY frequency DESC, display_name ASC
                 LIMIT ?2",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map(params![pattern, limit], row_to_contact)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_get_contact_by_email(
    state: State<'_, DbState>,
    email: String,
) -> Result<Option<DbContact>, String> {
    state
        .with_conn(move |conn| {
            let normalized = email.to_lowercase();

            let mut stmt = conn
                .prepare("SELECT * FROM contacts WHERE email = ?1 LIMIT 1")
                .map_err(|e| e.to_string())?;

            let mut rows = stmt
                .query_map(params![normalized], row_to_contact)
                .map_err(|e| e.to_string())?;

            match rows.next() {
                Some(Ok(c)) => Ok(Some(c)),
                Some(Err(e)) => Err(e.to_string()),
                None => Ok(None),
            }
        })
        .await
}

// ── Attachment queries ──────────────────────────────────────

#[tauri::command]
pub async fn db_get_attachments_for_message(
    state: State<'_, DbState>,
    account_id: String,
    message_id: String,
) -> Result<Vec<DbAttachment>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM attachments WHERE account_id = ?1 AND message_id = ?2 ORDER BY filename ASC",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map(params![account_id, message_id], row_to_attachment)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}

// ── Count queries ───────────────────────────────────────────

#[tauri::command]
pub async fn db_get_thread_count(
    state: State<'_, DbState>,
    account_id: String,
    label_id: Option<String>,
) -> Result<i64, String> {
    state
        .with_conn(move |conn| {
            if let Some(ref lid) = label_id {
                conn.query_row(
                    "SELECT COUNT(DISTINCT t.id) FROM threads t
                 INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
                 WHERE t.account_id = ?1 AND tl.label_id = ?2",
                    params![account_id, lid],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|e| e.to_string())
            } else {
                conn.query_row(
                    "SELECT COUNT(*) FROM threads WHERE account_id = ?1",
                    params![account_id],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|e| e.to_string())
            }
        })
        .await
}

#[tauri::command]
pub async fn db_get_unread_count(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<i64, String> {
    state
        .with_conn(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM threads t
             INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
             WHERE t.account_id = ?1 AND tl.label_id = 'INBOX' AND t.is_read = 0",
                params![account_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| e.to_string())
        })
        .await
}
