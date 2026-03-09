// tauri::command macro generates code that trips let_underscore_must_use
#![allow(clippy::let_underscore_must_use)]

use rusqlite::{params, Connection, Row};
use tauri::State;

use super::DbState;
use super::queries::row_to_contact;
use super::types::{
    BundleSummary, ContactAttachmentRow, ContactStats, DbBundleRule, DbContact, DbFilterRule,
    DbFollowUpReminder, DbQuickStep, DbSmartFolder, DbSmartLabelRule, RecentThread,
    SameDomainContact, SortOrderItem,
};

// ── Dynamic update helper ───────────────────────────────────

fn dynamic_update(
    conn: &Connection,
    table: &str,
    id_col: &str,
    id_val: &str,
    sets: Vec<(&str, Box<dyn rusqlite::types::ToSql>)>,
) -> Result<(), String> {
    if sets.is_empty() {
        return Ok(());
    }
    let mut placeholders = Vec::new();
    let mut param_vals: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    for (i, (col, val)) in sets.into_iter().enumerate() {
        placeholders.push(format!("{col} = ?{}", i + 1));
        param_vals.push(val);
    }
    let id_idx = param_vals.len() + 1;
    param_vals.push(Box::new(id_val.to_owned()));
    let sql = format!(
        "UPDATE {table} SET {} WHERE {id_col} = ?{id_idx}",
        placeholders.join(", ")
    );
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        param_vals.iter().map(AsRef::as_ref).collect();
    conn.execute(&sql, param_refs.as_slice())
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ── Row mappers ─────────────────────────────────────────────

fn row_to_filter(row: &Row<'_>) -> rusqlite::Result<DbFilterRule> {
    Ok(DbFilterRule {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        name: row.get("name")?,
        is_enabled: row.get::<_, i64>("is_enabled")? != 0,
        criteria_json: row.get("criteria_json")?,
        actions_json: row.get("actions_json")?,
        sort_order: row.get("sort_order")?,
        created_at: row.get("created_at")?,
    })
}

fn row_to_smart_folder(row: &Row<'_>) -> rusqlite::Result<DbSmartFolder> {
    Ok(DbSmartFolder {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        name: row.get("name")?,
        query: row.get("query")?,
        icon: row.get("icon")?,
        color: row.get("color")?,
        sort_order: row.get("sort_order")?,
        is_default: row.get::<_, i64>("is_default")? != 0,
        created_at: row.get("created_at")?,
    })
}

fn row_to_smart_label_rule(row: &Row<'_>) -> rusqlite::Result<DbSmartLabelRule> {
    Ok(DbSmartLabelRule {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        label_id: row.get("label_id")?,
        ai_description: row.get("ai_description")?,
        criteria_json: row.get("criteria_json")?,
        is_enabled: row.get::<_, i64>("is_enabled")? != 0,
        sort_order: row.get("sort_order")?,
        created_at: row.get("created_at")?,
    })
}

fn row_to_follow_up(row: &Row<'_>) -> rusqlite::Result<DbFollowUpReminder> {
    Ok(DbFollowUpReminder {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        thread_id: row.get("thread_id")?,
        message_id: row.get("message_id")?,
        remind_at: row.get("remind_at")?,
        status: row.get("status")?,
        created_at: row.get("created_at")?,
    })
}

fn row_to_quick_step(row: &Row<'_>) -> rusqlite::Result<DbQuickStep> {
    Ok(DbQuickStep {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        name: row.get("name")?,
        description: row.get("description")?,
        shortcut: row.get("shortcut")?,
        actions_json: row.get("actions_json")?,
        icon: row.get("icon")?,
        is_enabled: row.get::<_, i64>("is_enabled")? != 0,
        continue_on_error: row.get::<_, i64>("continue_on_error")? != 0,
        sort_order: row.get("sort_order")?,
        created_at: row.get("created_at")?,
    })
}

// ═══════════════════════════════════════════════════════════════
// CONTACTS — remaining queries beyond search/getByEmail
// ═══════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn db_get_all_contacts(
    state: State<'_, DbState>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DbContact>, String> {
    state
        .with_conn(move |conn| {
            let lim = limit.unwrap_or(500);
            let off = offset.unwrap_or(0);
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM contacts ORDER BY frequency DESC, display_name ASC LIMIT ?1 OFFSET ?2",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![lim, off], row_to_contact)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_upsert_contact(
    state: State<'_, DbState>,
    id: String,
    email: String,
    display_name: Option<String>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let normalized = email.to_lowercase();
            conn.execute(
                "INSERT INTO contacts (id, email, display_name, last_contacted_at)
                 VALUES (?1, ?2, ?3, unixepoch())
                 ON CONFLICT(email) DO UPDATE SET
                   display_name = COALESCE(?3, display_name),
                   frequency = frequency + 1,
                   last_contacted_at = unixepoch(),
                   updated_at = unixepoch()",
                params![id, normalized, display_name],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_update_contact(
    state: State<'_, DbState>,
    id: String,
    display_name: Option<String>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE contacts SET display_name = ?1, updated_at = unixepoch() WHERE id = ?2",
                params![display_name, id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_update_contact_notes(
    state: State<'_, DbState>,
    email: String,
    notes: Option<String>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let normalized = email.to_lowercase();
            conn.execute(
                "UPDATE contacts SET notes = ?1, updated_at = unixepoch() WHERE email = ?2",
                params![notes, normalized],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_delete_contact(
    state: State<'_, DbState>,
    id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute("DELETE FROM contacts WHERE id = ?1", params![id])
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_get_contact_stats(
    state: State<'_, DbState>,
    email: String,
) -> Result<ContactStats, String> {
    state
        .with_conn(move |conn| {
            let normalized = email.to_lowercase();
            conn.query_row(
                "SELECT COUNT(*) as cnt, MIN(date) as first_date, MAX(date) as last_date
                 FROM messages WHERE from_address = ?1",
                params![normalized],
                |row| {
                    Ok(ContactStats {
                        email_count: row.get(0)?,
                        first_email: row.get(1)?,
                        last_email: row.get(2)?,
                    })
                },
            )
            .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_get_contacts_from_same_domain(
    state: State<'_, DbState>,
    email: String,
    limit: Option<i64>,
) -> Result<Vec<SameDomainContact>, String> {
    state
        .with_conn(move |conn| {
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
            stmt.query_map(params![domain, normalized, lim], |row| {
                Ok(SameDomainContact {
                    email: row.get(0)?,
                    display_name: row.get(1)?,
                    avatar_url: row.get(2)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_get_latest_auth_result(
    state: State<'_, DbState>,
    email: String,
) -> Result<Option<String>, String> {
    state
        .with_conn(move |conn| {
            let normalized = email.to_lowercase();
            let result = conn
                .query_row(
                    "SELECT auth_results FROM messages
                     WHERE from_address = ?1 AND auth_results IS NOT NULL
                     ORDER BY date DESC LIMIT 1",
                    params![normalized],
                    |row| row.get::<_, String>(0),
                )
                .ok();
            Ok(result)
        })
        .await
}

#[tauri::command]
pub async fn db_get_recent_threads_with_contact(
    state: State<'_, DbState>,
    email: String,
    limit: Option<i64>,
) -> Result<Vec<RecentThread>, String> {
    state
        .with_conn(move |conn| {
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
            stmt.query_map(params![normalized, lim], |row| {
                Ok(RecentThread {
                    thread_id: row.get(0)?,
                    subject: row.get(1)?,
                    last_message_at: row.get(2)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_get_attachments_from_contact(
    state: State<'_, DbState>,
    email: String,
    limit: Option<i64>,
) -> Result<Vec<ContactAttachmentRow>, String> {
    state
        .with_conn(move |conn| {
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
            stmt.query_map(params![normalized, lim], |row| {
                Ok(ContactAttachmentRow {
                    filename: row.get(0)?,
                    mime_type: row.get(1)?,
                    size: row.get(2)?,
                    date: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
}

// ═══════════════════════════════════════════════════════════════
// FILTERS
// ═══════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn db_get_filters_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbFilterRule>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM filter_rules WHERE account_id = ?1 ORDER BY sort_order, created_at",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id], row_to_filter)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_insert_filter(
    state: State<'_, DbState>,
    id: String,
    account_id: String,
    name: String,
    criteria_json: String,
    actions_json: String,
    is_enabled: Option<bool>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO filter_rules (id, account_id, name, is_enabled, criteria_json, actions_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, account_id, name, is_enabled.unwrap_or(true), criteria_json, actions_json],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_update_filter(
    state: State<'_, DbState>,
    id: String,
    name: Option<String>,
    criteria_json: Option<String>,
    actions_json: Option<String>,
    is_enabled: Option<bool>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let mut sets: Vec<(&str, Box<dyn rusqlite::types::ToSql>)> = Vec::new();
            if let Some(v) = name {
                sets.push(("name", Box::new(v)));
            }
            if let Some(v) = criteria_json {
                sets.push(("criteria_json", Box::new(v)));
            }
            if let Some(v) = actions_json {
                sets.push(("actions_json", Box::new(v)));
            }
            if let Some(v) = is_enabled {
                sets.push(("is_enabled", Box::new(v)));
            }
            dynamic_update(conn, "filter_rules", "id", &id, sets)
        })
        .await
}

#[tauri::command]
pub async fn db_delete_filter(
    state: State<'_, DbState>,
    id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute("DELETE FROM filter_rules WHERE id = ?1", params![id])
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ═══════════════════════════════════════════════════════════════
// SMART FOLDERS
// ═══════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn db_get_smart_folders(
    state: State<'_, DbState>,
    account_id: Option<String>,
) -> Result<Vec<DbSmartFolder>, String> {
    state
        .with_conn(move |conn| {
            if let Some(ref aid) = account_id {
                let mut stmt = conn
                    .prepare(
                        "SELECT * FROM smart_folders WHERE account_id IS NULL OR account_id = ?1
                         ORDER BY sort_order, created_at",
                    )
                    .map_err(|e| e.to_string())?;
                stmt.query_map(params![aid], row_to_smart_folder)
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())
            } else {
                let mut stmt = conn
                    .prepare(
                        "SELECT * FROM smart_folders WHERE account_id IS NULL
                         ORDER BY sort_order, created_at",
                    )
                    .map_err(|e| e.to_string())?;
                stmt.query_map([], row_to_smart_folder)
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())
            }
        })
        .await
}

#[tauri::command]
pub async fn db_get_smart_folder_by_id(
    state: State<'_, DbState>,
    id: String,
) -> Result<Option<DbSmartFolder>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare("SELECT * FROM smart_folders WHERE id = ?1")
                .map_err(|e| e.to_string())?;
            let mut rows = stmt
                .query_map(params![id], row_to_smart_folder)
                .map_err(|e| e.to_string())?;
            match rows.next() {
                Some(Ok(f)) => Ok(Some(f)),
                Some(Err(e)) => Err(e.to_string()),
                None => Ok(None),
            }
        })
        .await
}

#[tauri::command]
pub async fn db_insert_smart_folder(
    state: State<'_, DbState>,
    id: String,
    name: String,
    query: String,
    account_id: Option<String>,
    icon: Option<String>,
    color: Option<String>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO smart_folders (id, account_id, name, query, icon, color)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, account_id, name, query, icon.unwrap_or_else(|| "search".to_owned()), color],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_update_smart_folder(
    state: State<'_, DbState>,
    id: String,
    name: Option<String>,
    query: Option<String>,
    icon: Option<String>,
    color: Option<String>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let mut sets: Vec<(&str, Box<dyn rusqlite::types::ToSql>)> = Vec::new();
            if let Some(v) = name {
                sets.push(("name", Box::new(v)));
            }
            if let Some(v) = query {
                sets.push(("query", Box::new(v)));
            }
            if let Some(v) = icon {
                sets.push(("icon", Box::new(v)));
            }
            if let Some(v) = color {
                sets.push(("color", Box::new(v)));
            }
            dynamic_update(conn, "smart_folders", "id", &id, sets)
        })
        .await
}

#[tauri::command]
pub async fn db_delete_smart_folder(
    state: State<'_, DbState>,
    id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute("DELETE FROM smart_folders WHERE id = ?1", params![id])
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_update_smart_folder_sort_order(
    state: State<'_, DbState>,
    orders: Vec<SortOrderItem>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            for item in &orders {
                conn.execute(
                    "UPDATE smart_folders SET sort_order = ?1 WHERE id = ?2",
                    params![item.sort_order, item.id],
                )
                .map_err(|e| e.to_string())?;
            }
            Ok(())
        })
        .await
}

// ═══════════════════════════════════════════════════════════════
// SMART LABEL RULES
// ═══════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn db_get_smart_label_rules_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbSmartLabelRule>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM smart_label_rules WHERE account_id = ?1
                     ORDER BY sort_order, created_at",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id], row_to_smart_label_rule)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_insert_smart_label_rule(
    state: State<'_, DbState>,
    id: String,
    account_id: String,
    label_id: String,
    ai_description: String,
    criteria_json: Option<String>,
    is_enabled: Option<bool>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO smart_label_rules (id, account_id, label_id, ai_description, criteria_json, is_enabled)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, account_id, label_id, ai_description, criteria_json, is_enabled.unwrap_or(true)],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_update_smart_label_rule(
    state: State<'_, DbState>,
    id: String,
    label_id: Option<String>,
    ai_description: Option<String>,
    criteria_json: Option<String>,
    is_enabled: Option<bool>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let mut sets: Vec<(&str, Box<dyn rusqlite::types::ToSql>)> = Vec::new();
            if let Some(v) = label_id {
                sets.push(("label_id", Box::new(v)));
            }
            if let Some(v) = ai_description {
                sets.push(("ai_description", Box::new(v)));
            }
            if let Some(v) = criteria_json {
                sets.push(("criteria_json", Box::new(v)));
            }
            if let Some(v) = is_enabled {
                sets.push(("is_enabled", Box::new(v)));
            }
            dynamic_update(conn, "smart_label_rules", "id", &id, sets)
        })
        .await
}

#[tauri::command]
pub async fn db_delete_smart_label_rule(
    state: State<'_, DbState>,
    id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute("DELETE FROM smart_label_rules WHERE id = ?1", params![id])
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ═══════════════════════════════════════════════════════════════
// FOLLOW-UP REMINDERS
// ═══════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn db_insert_follow_up_reminder(
    state: State<'_, DbState>,
    id: String,
    account_id: String,
    thread_id: String,
    message_id: String,
    remind_at: i64,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO follow_up_reminders (id, account_id, thread_id, message_id, remind_at, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'pending')
                 ON CONFLICT(account_id, thread_id) DO UPDATE SET
                   message_id = ?4, remind_at = ?5, status = 'pending'",
                params![id, account_id, thread_id, message_id, remind_at],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_get_follow_up_for_thread(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<Option<DbFollowUpReminder>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM follow_up_reminders
                     WHERE account_id = ?1 AND thread_id = ?2 AND status = 'pending' LIMIT 1",
                )
                .map_err(|e| e.to_string())?;
            let mut rows = stmt
                .query_map(params![account_id, thread_id], row_to_follow_up)
                .map_err(|e| e.to_string())?;
            match rows.next() {
                Some(Ok(r)) => Ok(Some(r)),
                Some(Err(e)) => Err(e.to_string()),
                None => Ok(None),
            }
        })
        .await
}

#[tauri::command]
pub async fn db_cancel_follow_up_for_thread(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE follow_up_reminders SET status = 'cancelled'
                 WHERE account_id = ?1 AND thread_id = ?2 AND status = 'pending'",
                params![account_id, thread_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_get_active_follow_up_thread_ids(
    state: State<'_, DbState>,
    account_id: String,
    thread_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    if thread_ids.is_empty() {
        return Ok(Vec::new());
    }
    state
        .with_conn(move |conn| {
            let mut results = Vec::new();
            for chunk in thread_ids.chunks(100) {
                let placeholders: String = chunk
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", i + 2))
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "SELECT thread_id FROM follow_up_reminders
                     WHERE account_id = ?1 AND status = 'pending' AND thread_id IN ({placeholders})"
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
                    .query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?;
                results.extend(rows);
            }
            Ok(results)
        })
        .await
}

// ═══════════════════════════════════════════════════════════════
// QUICK STEPS
// ═══════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn db_get_quick_steps_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbQuickStep>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM quick_steps WHERE account_id = ?1 ORDER BY sort_order, created_at",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id], row_to_quick_step)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_get_enabled_quick_steps_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbQuickStep>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM quick_steps WHERE account_id = ?1 AND is_enabled = 1
                     ORDER BY sort_order, created_at",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id], row_to_quick_step)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_insert_quick_step(
    state: State<'_, DbState>,
    step: DbQuickStep,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO quick_steps (id, account_id, name, description, shortcut, actions_json, icon, is_enabled, continue_on_error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    step.id, step.account_id, step.name, step.description, step.shortcut,
                    step.actions_json, step.icon, step.is_enabled, step.continue_on_error
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_update_quick_step(
    state: State<'_, DbState>,
    step: DbQuickStep,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE quick_steps SET name = ?2, description = ?3, shortcut = ?4,
                 actions_json = ?5, icon = ?6, is_enabled = ?7, continue_on_error = ?8
                 WHERE id = ?1",
                params![
                    step.id, step.name, step.description, step.shortcut,
                    step.actions_json, step.icon, step.is_enabled, step.continue_on_error
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_delete_quick_step(
    state: State<'_, DbState>,
    id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute("DELETE FROM quick_steps WHERE id = ?1", params![id])
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ═══════════════════════════════════════════════════════════════
// IMAGE ALLOWLIST
// ═══════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn db_add_to_allowlist(
    state: State<'_, DbState>,
    id: String,
    account_id: String,
    sender_address: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT OR IGNORE INTO image_allowlist (id, account_id, sender_address) VALUES (?1, ?2, ?3)",
                params![id, account_id, sender_address],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_get_allowlisted_senders(
    state: State<'_, DbState>,
    account_id: String,
    sender_addresses: Vec<String>,
) -> Result<Vec<String>, String> {
    if sender_addresses.is_empty() {
        return Ok(Vec::new());
    }
    state
        .with_conn(move |conn| {
            let mut results = Vec::new();
            for chunk in sender_addresses.chunks(100) {
                let placeholders: String = chunk
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", i + 2))
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "SELECT sender_address FROM image_allowlist
                     WHERE account_id = ?1 AND sender_address IN ({placeholders})"
                );
                let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
                let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
                param_values.push(Box::new(account_id.clone()));
                for addr in chunk {
                    param_values.push(Box::new(addr.clone()));
                }
                let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                    param_values.iter().map(AsRef::as_ref).collect();
                let rows = stmt
                    .query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?;
                results.extend(rows);
            }
            Ok(results)
        })
        .await
}

// ═══════════════════════════════════════════════════════════════
// NOTIFICATION VIPS
// ═══════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn db_add_vip_sender(
    state: State<'_, DbState>,
    id: String,
    account_id: String,
    email: String,
    display_name: Option<String>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT OR IGNORE INTO notification_vips (id, account_id, email_address, display_name)
                 VALUES (?1, ?2, ?3, ?4)",
                params![id, account_id, email, display_name],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_remove_vip_sender(
    state: State<'_, DbState>,
    account_id: String,
    email: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "DELETE FROM notification_vips WHERE account_id = ?1 AND email_address = ?2",
                params![account_id, email],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_is_vip_sender(
    state: State<'_, DbState>,
    account_id: String,
    email: String,
) -> Result<bool, String> {
    state
        .with_conn(move |conn| {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM notification_vips WHERE account_id = ?1 AND email_address = ?2",
                    params![account_id, email],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            Ok(count > 0)
        })
        .await
}

// ═══════════════════════════════════════════════════════════════
// THREAD CATEGORIES — set
// ═══════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn db_set_thread_category(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    category: String,
    is_manual: bool,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO thread_categories (account_id, thread_id, category, is_manual)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(account_id, thread_id) DO UPDATE SET category = ?3, is_manual = ?4",
                params![account_id, thread_id, category, is_manual as i64],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ═══════════════════════════════════════════════════════════════
// BUNDLE RULES
// ═══════════════════════════════════════════════════════════════

fn row_to_bundle_rule(row: &Row<'_>) -> rusqlite::Result<DbBundleRule> {
    Ok(DbBundleRule {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        category: row.get("category")?,
        is_bundled: row.get("is_bundled")?,
        delivery_enabled: row.get("delivery_enabled")?,
        delivery_schedule: row.get("delivery_schedule")?,
        last_delivered_at: row.get("last_delivered_at")?,
        created_at: row.get("created_at")?,
    })
}

#[tauri::command]
pub async fn db_get_bundle_rules(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbBundleRule>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare("SELECT * FROM bundle_rules WHERE account_id = ?1")
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id], row_to_bundle_rule)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_get_bundle_summaries(
    state: State<'_, DbState>,
    account_id: String,
    categories: Vec<String>,
) -> Result<Vec<BundleSummary>, String> {
    if categories.is_empty() {
        return Ok(Vec::new());
    }
    state
        .with_conn(move |conn| {
            let placeholders: String = categories
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 2))
                .collect::<Vec<_>>()
                .join(", ");

            // Query 1: counts per category
            let count_sql = format!(
                "SELECT tc.category, COUNT(DISTINCT t.id) as count
                 FROM threads t
                 JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id AND tl.label_id = 'INBOX'
                 JOIN thread_categories tc ON tc.account_id = t.account_id AND tc.thread_id = t.id AND tc.category IN ({placeholders})
                 WHERE t.account_id = ?1
                 GROUP BY tc.category"
            );
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            param_values.push(Box::new(account_id.clone()));
            for cat in &categories {
                param_values.push(Box::new(cat.clone()));
            }
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(AsRef::as_ref).collect();

            let mut stmt = conn.prepare(&count_sql).map_err(|e| e.to_string())?;
            let count_rows: Vec<(String, i64)> = stmt
                .query_map(param_refs.as_slice(), |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;

            // Query 2: latest subject/sender per category
            let latest_sql = format!(
                "SELECT tc.category, t.subject, m.from_name
                 FROM threads t
                 JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id AND tl.label_id = 'INBOX'
                 JOIN thread_categories tc ON tc.account_id = t.account_id AND tc.thread_id = t.id AND tc.category IN ({placeholders})
                 JOIN messages m ON m.account_id = t.account_id AND m.thread_id = t.id
                 WHERE t.account_id = ?1
                 GROUP BY tc.category
                 HAVING t.last_message_at = MAX(t.last_message_at)"
            );
            let mut stmt2 = conn.prepare(&latest_sql).map_err(|e| e.to_string())?;
            let latest_rows: Vec<(String, Option<String>, Option<String>)> = stmt2
                .query_map(param_refs.as_slice(), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;

            // Build result with defaults for every requested category
            let mut results = Vec::with_capacity(categories.len());
            for cat in &categories {
                let count = count_rows
                    .iter()
                    .find(|(c, _)| c == cat)
                    .map(|(_, n)| *n)
                    .unwrap_or(0);
                let (latest_subject, latest_sender) = latest_rows
                    .iter()
                    .find(|(c, _, _)| c == cat)
                    .map(|(_, s, f)| (s.clone(), f.clone()))
                    .unwrap_or((None, None));
                results.push(BundleSummary {
                    category: cat.clone(),
                    count,
                    latest_subject,
                    latest_sender,
                });
            }
            Ok(results)
        })
        .await
}

#[tauri::command]
pub async fn db_get_held_thread_ids(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<String>, String> {
    state
        .with_conn(move |conn| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_secs() as i64;
            let mut stmt = conn
                .prepare(
                    "SELECT thread_id FROM bundled_threads WHERE account_id = ?1 AND held_until > ?2",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id, now], |row| row.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}
