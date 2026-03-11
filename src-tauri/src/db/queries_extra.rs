// tauri::command macro generates code that trips let_underscore_must_use
#![allow(clippy::let_underscore_must_use)]

use rusqlite::{Connection, Row, params};
use tauri::State;

use super::DbState;
use super::queries::{row_to_contact, row_to_message};
use super::types::{
    AttachmentSender, AttachmentWithContext, BackfillRow, BundleSummary, BundleSummarySingle,
    CachedAttachmentRow, ContactAttachmentRow, ContactStats, DbAccount, DbAllowlistEntry,
    DbBundleRule, DbCalendar, DbCalendarEvent, DbContact, DbFilterRule, DbFolderSyncState,
    DbFollowUpReminder, DbLocalDraft, DbNotificationVip, DbPhishingAllowlistEntry, DbQuickStep,
    DbScheduledEmail, DbSendAsAlias, DbSignature, DbSmartFolder, DbSmartLabelRule, DbTask,
    DbTaskTag, DbTemplate, DbWritingStyleProfile, ImapMessageRow, LabelSortOrderItem, RecentThread,
    SameDomainContact, SnoozedThread, SortOrderItem, SpecialFolderRow, SubscriptionEntry,
    ThreadCategoryWithManual, ThreadInfoRow, TriggeredFollowUp, UncachedAttachment,
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
pub async fn db_delete_contact(state: State<'_, DbState>, id: String) -> Result<(), String> {
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
pub async fn db_delete_filter(state: State<'_, DbState>, id: String) -> Result<(), String> {
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
                params![
                    id,
                    account_id,
                    name,
                    query,
                    icon.unwrap_or_else(|| "search".to_owned()),
                    color
                ],
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
pub async fn db_delete_smart_folder(state: State<'_, DbState>, id: String) -> Result<(), String> {
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

/// Batch-check all pending follow-up reminders in a single transaction.
///
/// For each reminder that is due (status = 'pending', remind_at <= now):
///   - If a reply exists in the thread (from someone other than the account owner,
///     dated after the tracked message) → set status = 'cancelled'
///   - If no reply → set status = 'triggered' and include in result
///
/// Returns the list of triggered reminders with thread subjects for notification dispatch.
#[tauri::command]
pub async fn db_check_follow_up_reminders(
    state: State<'_, DbState>,
) -> Result<Vec<TriggeredFollowUp>, String> {
    state
        .with_conn(move |conn| {
            let now = i64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_secs(),
            )
            .map_err(|_| "current time exceeds i64 range".to_string())?;

            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;

            // 1. Fetch all pending, due reminders
            let reminders: Vec<DbFollowUpReminder> = {
                let mut stmt = tx
                    .prepare(
                        "SELECT * FROM follow_up_reminders WHERE status = 'pending' AND remind_at <= ?1",
                    )
                    .map_err(|e| e.to_string())?;
                let rows = stmt.query_map(params![now], row_to_follow_up)
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string());
                drop(stmt);
                rows?
            };

            if reminders.is_empty() {
                tx.commit().map_err(|e| e.to_string())?;
                return Ok(Vec::new());
            }

            let mut triggered = Vec::new();

            for reminder in &reminders {
                // 2. Check if a reply exists: any message in the thread from someone
                //    other than the account owner, dated after the tracked message
                let reply_count: i64 = tx
                    .query_row(
                        "SELECT COUNT(*) FROM messages m
                         WHERE m.account_id = ?1 AND m.thread_id = ?2
                           AND m.date > (SELECT date FROM messages WHERE id = ?3 AND account_id = ?1)
                           AND m.from_address != (SELECT email FROM accounts WHERE id = ?1)",
                        params![reminder.account_id, reminder.thread_id, reminder.message_id],
                        |row| row.get(0),
                    )
                    .map_err(|e| e.to_string())?;

                if reply_count > 0 {
                    // 3. Reply exists → cancel
                    tx.execute(
                        "UPDATE follow_up_reminders SET status = 'cancelled' WHERE id = ?1",
                        params![reminder.id],
                    )
                    .map_err(|e| e.to_string())?;
                } else {
                    // 4. No reply → trigger
                    tx.execute(
                        "UPDATE follow_up_reminders SET status = 'triggered' WHERE id = ?1",
                        params![reminder.id],
                    )
                    .map_err(|e| e.to_string())?;

                    // 5. Get thread subject for notification
                    let subject: String = tx
                        .query_row(
                            "SELECT COALESCE(subject, '') FROM threads WHERE account_id = ?1 AND id = ?2",
                            params![reminder.account_id, reminder.thread_id],
                            |row| row.get(0),
                        )
                        .unwrap_or_default();

                    triggered.push(TriggeredFollowUp {
                        id: reminder.id.clone(),
                        account_id: reminder.account_id.clone(),
                        thread_id: reminder.thread_id.clone(),
                        subject,
                    });
                }
            }

            tx.commit().map_err(|e| e.to_string())?;
            Ok(triggered)
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
                    step.id,
                    step.name,
                    step.description,
                    step.shortcut,
                    step.actions_json,
                    step.icon,
                    step.is_enabled,
                    step.continue_on_error
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_delete_quick_step(state: State<'_, DbState>, id: String) -> Result<(), String> {
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
            let now = i64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_secs(),
            )
            .map_err(|_| "current time exceeds i64 range".to_string())?;
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

// ── Attachment pre-cache queries ─────────────────────────────

#[tauri::command]
pub async fn db_attachment_cache_total_size(state: State<'_, DbState>) -> Result<i64, String> {
    state
        .with_conn(move |conn| {
            conn.query_row(
                "SELECT COALESCE(SUM(cache_size), 0) FROM attachments WHERE cached_at IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_uncached_recent_attachments(
    state: State<'_, DbState>,
    max_size: i64,
    cutoff_epoch: i64,
    limit: i64,
) -> Result<Vec<UncachedAttachment>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT a.id, a.message_id, a.account_id, a.size, a.gmail_attachment_id, a.imap_part_id
                     FROM attachments a
                     INNER JOIN messages m ON m.account_id = a.account_id AND m.id = a.message_id
                     WHERE a.cached_at IS NULL
                       AND a.is_inline = 0
                       AND a.size IS NOT NULL AND a.size <= ?1
                       AND m.date >= ?2
                     ORDER BY m.date DESC
                     LIMIT ?3",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![max_size, cutoff_epoch, limit], |row| {
                Ok(UncachedAttachment {
                    id: row.get("id")?,
                    message_id: row.get("message_id")?,
                    account_id: row.get("account_id")?,
                    size: row.get("size")?,
                    gmail_attachment_id: row.get("gmail_attachment_id")?,
                    imap_part_id: row.get("imap_part_id")?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
}

// ── AI Cache ───────────────────────────────────────────────

#[tauri::command]
pub async fn db_get_ai_cache(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    cache_type: String,
) -> Result<Option<String>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT content FROM ai_cache WHERE account_id = ?1 AND thread_id = ?2 AND type = ?3",
                )
                .map_err(|e| e.to_string())?;
            let mut rows = stmt
                .query_map(params![account_id, thread_id, cache_type], |row| {
                    row.get::<_, String>(0)
                })
                .map_err(|e| e.to_string())?;
            match rows.next() {
                Some(Ok(content)) => Ok(Some(content)),
                Some(Err(e)) => Err(e.to_string()),
                None => Ok(None),
            }
        })
        .await
}

#[tauri::command]
pub async fn db_set_ai_cache(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    cache_type: String,
    content: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO ai_cache (id, account_id, thread_id, type, content)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(account_id, thread_id, type) DO UPDATE SET
                   content = ?5, created_at = unixepoch()",
                params![id, account_id, thread_id, cache_type, content],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_delete_ai_cache(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    cache_type: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "DELETE FROM ai_cache WHERE account_id = ?1 AND thread_id = ?2 AND type = ?3",
                params![account_id, thread_id, cache_type],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ── Link Scan Results ──────────────────────────────────────

#[tauri::command]
pub async fn db_get_cached_scan_result(
    state: State<'_, DbState>,
    account_id: String,
    message_id: String,
) -> Result<Option<String>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT result_json FROM link_scan_results WHERE account_id = ?1 AND message_id = ?2 LIMIT 1",
                )
                .map_err(|e| e.to_string())?;
            let mut rows = stmt
                .query_map(params![account_id, message_id], |row| {
                    row.get::<_, String>(0)
                })
                .map_err(|e| e.to_string())?;
            match rows.next() {
                Some(Ok(val)) => Ok(Some(val)),
                Some(Err(e)) => Err(e.to_string()),
                None => Ok(None),
            }
        })
        .await
}

#[tauri::command]
pub async fn db_cache_scan_result(
    state: State<'_, DbState>,
    account_id: String,
    message_id: String,
    result_json: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO link_scan_results (account_id, message_id, result_json) VALUES (?1, ?2, ?3)",
                params![account_id, message_id, result_json],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_delete_scan_results(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "DELETE FROM link_scan_results WHERE account_id = ?1",
                params![account_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ── Writing Style Profiles ─────────────────────────────────

#[tauri::command]
pub async fn db_get_writing_style_profile(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Option<DbWritingStyleProfile>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, profile_text, sample_count, created_at, updated_at
                     FROM writing_style_profiles WHERE account_id = ?1",
                )
                .map_err(|e| e.to_string())?;
            let mut rows = stmt
                .query_map(params![account_id], |row| {
                    Ok(DbWritingStyleProfile {
                        id: row.get("id")?,
                        account_id: row.get("account_id")?,
                        profile_text: row.get("profile_text")?,
                        sample_count: row.get("sample_count")?,
                        created_at: row.get("created_at")?,
                        updated_at: row.get("updated_at")?,
                    })
                })
                .map_err(|e| e.to_string())?;
            match rows.next() {
                Some(Ok(profile)) => Ok(Some(profile)),
                Some(Err(e)) => Err(e.to_string()),
                None => Ok(None),
            }
        })
        .await
}

#[tauri::command]
pub async fn db_upsert_writing_style_profile(
    state: State<'_, DbState>,
    account_id: String,
    profile_text: String,
    sample_count: i64,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO writing_style_profiles (id, account_id, profile_text, sample_count)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(account_id) DO UPDATE SET
                   profile_text = ?3, sample_count = ?4, updated_at = unixepoch()",
                params![id, account_id, profile_text, sample_count],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_delete_writing_style_profile(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "DELETE FROM writing_style_profiles WHERE account_id = ?1",
                params![account_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ── Folder Sync State ──────────────────────────────────────

#[tauri::command]
pub async fn db_get_folder_sync_state(
    state: State<'_, DbState>,
    account_id: String,
    folder_path: String,
) -> Result<Option<DbFolderSyncState>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT account_id, folder_path, uidvalidity, last_uid, modseq, last_sync_at
                     FROM folder_sync_state WHERE account_id = ?1 AND folder_path = ?2",
                )
                .map_err(|e| e.to_string())?;
            let mut rows = stmt
                .query_map(params![account_id, folder_path], |row| {
                    Ok(DbFolderSyncState {
                        account_id: row.get("account_id")?,
                        folder_path: row.get("folder_path")?,
                        uidvalidity: row.get("uidvalidity")?,
                        last_uid: row.get("last_uid")?,
                        modseq: row.get("modseq")?,
                        last_sync_at: row.get("last_sync_at")?,
                    })
                })
                .map_err(|e| e.to_string())?;
            match rows.next() {
                Some(Ok(s)) => Ok(Some(s)),
                Some(Err(e)) => Err(e.to_string()),
                None => Ok(None),
            }
        })
        .await
}

#[tauri::command]
pub async fn db_upsert_folder_sync_state(
    state: State<'_, DbState>,
    account_id: String,
    folder_path: String,
    uidvalidity: Option<i64>,
    last_uid: i64,
    modseq: Option<i64>,
    last_sync_at: Option<i64>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO folder_sync_state (account_id, folder_path, uidvalidity, last_uid, modseq, last_sync_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(account_id, folder_path) DO UPDATE SET
                   uidvalidity = ?3, last_uid = ?4, modseq = ?5, last_sync_at = ?6",
                params![account_id, folder_path, uidvalidity, last_uid, modseq, last_sync_at],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_delete_folder_sync_state(
    state: State<'_, DbState>,
    account_id: String,
    folder_path: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "DELETE FROM folder_sync_state WHERE account_id = ?1 AND folder_path = ?2",
                params![account_id, folder_path],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_clear_all_folder_sync_states(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "DELETE FROM folder_sync_state WHERE account_id = ?1",
                params![account_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_get_all_folder_sync_states(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbFolderSyncState>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT account_id, folder_path, uidvalidity, last_uid, modseq, last_sync_at
                     FROM folder_sync_state WHERE account_id = ?1 ORDER BY folder_path ASC",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id], |row| {
                Ok(DbFolderSyncState {
                    account_id: row.get("account_id")?,
                    folder_path: row.get("folder_path")?,
                    uidvalidity: row.get("uidvalidity")?,
                    last_uid: row.get("last_uid")?,
                    modseq: row.get("modseq")?,
                    last_sync_at: row.get("last_sync_at")?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
}

// ═══════════════════════════════════════════════════════════════
// NOTIFICATION VIPS — additional queries
// ═══════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn db_get_vip_senders(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<String>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare("SELECT email_address FROM notification_vips WHERE account_id = ?1")
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id], |row| row.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_get_all_vip_senders(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbNotificationVip>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, email_address, display_name, created_at
                     FROM notification_vips WHERE account_id = ?1
                     ORDER BY display_name, email_address",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id], |row| {
                Ok(DbNotificationVip {
                    id: row.get("id")?,
                    account_id: row.get("account_id")?,
                    email_address: row.get("email_address")?,
                    display_name: row.get("display_name")?,
                    created_at: row.get("created_at")?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
}

// ═══════════════════════════════════════════════════════════════
// IMAGE ALLOWLIST — additional queries
// ═══════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn db_is_allowlisted(
    state: State<'_, DbState>,
    account_id: String,
    sender_address: String,
) -> Result<bool, String> {
    let sender_address = sender_address.to_lowercase();
    state
        .with_conn(move |conn| {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM image_allowlist WHERE account_id = ?1 AND sender_address = ?2",
                    params![account_id, sender_address],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            Ok(count > 0)
        })
        .await
}

#[tauri::command]
pub async fn db_remove_from_allowlist(
    state: State<'_, DbState>,
    account_id: String,
    sender_address: String,
) -> Result<(), String> {
    let sender_address = sender_address.to_lowercase();
    state
        .with_conn(move |conn| {
            conn.execute(
                "DELETE FROM image_allowlist WHERE account_id = ?1 AND sender_address = ?2",
                params![account_id, sender_address],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_get_allowlist_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbAllowlistEntry>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, sender_address, created_at
                     FROM image_allowlist WHERE account_id = ?1
                     ORDER BY sender_address",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id], |row| {
                Ok(DbAllowlistEntry {
                    id: row.get("id")?,
                    account_id: row.get("account_id")?,
                    sender_address: row.get("sender_address")?,
                    created_at: row.get("created_at")?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
}

// ═══════════════════════════════════════════════════════════════
// PHISHING ALLOWLIST
// ═══════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn db_is_phishing_allowlisted(
    state: State<'_, DbState>,
    account_id: String,
    sender_address: String,
) -> Result<bool, String> {
    let sender_address = sender_address.to_lowercase();
    state
        .with_conn(move |conn| {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM phishing_allowlist WHERE account_id = ?1 AND sender_address = ?2",
                    params![account_id, sender_address],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            Ok(count > 0)
        })
        .await
}

#[tauri::command]
pub async fn db_add_to_phishing_allowlist(
    state: State<'_, DbState>,
    account_id: String,
    sender_address: String,
) -> Result<(), String> {
    let sender_address = sender_address.to_lowercase();
    let id = uuid::Uuid::new_v4().to_string();
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT OR IGNORE INTO phishing_allowlist (id, account_id, sender_address) VALUES (?1, ?2, ?3)",
                params![id, account_id, sender_address],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_remove_from_phishing_allowlist(
    state: State<'_, DbState>,
    account_id: String,
    sender_address: String,
) -> Result<(), String> {
    let sender_address = sender_address.to_lowercase();
    state
        .with_conn(move |conn| {
            conn.execute(
                "DELETE FROM phishing_allowlist WHERE account_id = ?1 AND sender_address = ?2",
                params![account_id, sender_address],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_get_phishing_allowlist(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbPhishingAllowlistEntry>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, sender_address, created_at
                     FROM phishing_allowlist WHERE account_id = ?1
                     ORDER BY sender_address",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id], |row| {
                Ok(DbPhishingAllowlistEntry {
                    id: row.get("id")?,
                    sender_address: row.get("sender_address")?,
                    created_at: row.get("created_at")?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
}

// ═══════════════════════════════════════════════════════════════
// TEMPLATES
// ═══════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn db_get_templates_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbTemplate>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, name, subject, body_html, shortcut, sort_order, created_at
                     FROM templates WHERE account_id = ?1 OR account_id IS NULL
                     ORDER BY sort_order, created_at",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id], |row| {
                Ok(DbTemplate {
                    id: row.get("id")?,
                    account_id: row.get("account_id")?,
                    name: row.get("name")?,
                    subject: row.get("subject")?,
                    body_html: row.get("body_html")?,
                    shortcut: row.get("shortcut")?,
                    sort_order: row.get("sort_order")?,
                    created_at: row.get("created_at")?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_insert_template(
    state: State<'_, DbState>,
    account_id: Option<String>,
    name: String,
    subject: Option<String>,
    body_html: String,
    shortcut: Option<String>,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let ret_id = id.clone();
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO templates (id, account_id, name, subject, body_html, shortcut)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, account_id, name, subject, body_html, shortcut],
            )
            .map_err(|e| e.to_string())?;
            Ok(ret_id)
        })
        .await
}

#[tauri::command]
pub async fn db_update_template(
    state: State<'_, DbState>,
    id: String,
    name: Option<String>,
    subject: Option<String>,
    subject_set: bool,
    body_html: Option<String>,
    shortcut: Option<String>,
    shortcut_set: bool,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let mut sets: Vec<(&str, Box<dyn rusqlite::types::ToSql>)> = Vec::new();
            if let Some(v) = name {
                sets.push(("name", Box::new(v)));
            }
            if subject_set {
                sets.push(("subject", Box::new(subject)));
            }
            if let Some(v) = body_html {
                sets.push(("body_html", Box::new(v)));
            }
            if shortcut_set {
                sets.push(("shortcut", Box::new(shortcut)));
            }
            dynamic_update(conn, "templates", "id", &id, sets)
        })
        .await
}

#[tauri::command]
pub async fn db_delete_template(state: State<'_, DbState>, id: String) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute("DELETE FROM templates WHERE id = ?1", params![id])
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ═══════════════════════════════════════════════════════════════
// SIGNATURES
// ═══════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn db_get_signatures_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbSignature>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, name, body_html, is_default, sort_order
                     FROM signatures WHERE account_id = ?1
                     ORDER BY sort_order, created_at",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id], |row| {
                Ok(DbSignature {
                    id: row.get("id")?,
                    account_id: row.get("account_id")?,
                    name: row.get("name")?,
                    body_html: row.get("body_html")?,
                    is_default: row.get("is_default")?,
                    sort_order: row.get("sort_order")?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_get_default_signature(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Option<DbSignature>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, name, body_html, is_default, sort_order
                     FROM signatures WHERE account_id = ?1 AND is_default = 1 LIMIT 1",
                )
                .map_err(|e| e.to_string())?;
            let rows: Vec<DbSignature> = stmt
                .query_map(params![account_id], |row| {
                    Ok(DbSignature {
                        id: row.get("id")?,
                        account_id: row.get("account_id")?,
                        name: row.get("name")?,
                        body_html: row.get("body_html")?,
                        is_default: row.get("is_default")?,
                        sort_order: row.get("sort_order")?,
                    })
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            Ok(rows.into_iter().next())
        })
        .await
}

#[tauri::command]
pub async fn db_insert_signature(
    state: State<'_, DbState>,
    account_id: String,
    name: String,
    body_html: String,
    is_default: bool,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let ret_id = id.clone();
    let is_default_int = i64::from(is_default);
    state
        .with_conn(move |conn| {
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            if is_default {
                tx.execute(
                    "UPDATE signatures SET is_default = 0 WHERE account_id = ?1",
                    params![account_id],
                )
                .map_err(|e| e.to_string())?;
            }
            tx.execute(
                "INSERT INTO signatures (id, account_id, name, body_html, is_default)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, account_id, name, body_html, is_default_int],
            )
            .map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            Ok(ret_id)
        })
        .await
}

#[tauri::command]
pub async fn db_update_signature(
    state: State<'_, DbState>,
    id: String,
    name: Option<String>,
    body_html: Option<String>,
    is_default: Option<bool>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            if is_default == Some(true) {
                let account_id: Option<String> = tx
                    .query_row(
                        "SELECT account_id FROM signatures WHERE id = ?1",
                        params![id],
                        |row| row.get(0),
                    )
                    .ok();
                if let Some(aid) = account_id {
                    tx.execute(
                        "UPDATE signatures SET is_default = 0 WHERE account_id = ?1",
                        params![aid],
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
            let mut sets: Vec<(&str, Box<dyn rusqlite::types::ToSql>)> = Vec::new();
            if let Some(v) = name {
                sets.push(("name", Box::new(v)));
            }
            if let Some(v) = body_html {
                sets.push(("body_html", Box::new(v)));
            }
            if let Some(v) = is_default {
                sets.push(("is_default", Box::new(i64::from(v))));
            }
            dynamic_update(&tx, "signatures", "id", &id, sets)?;
            tx.commit().map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_delete_signature(state: State<'_, DbState>, id: String) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute("DELETE FROM signatures WHERE id = ?1", params![id])
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ── Send-As Alias queries ──────────────────────────────────

fn row_to_send_as_alias(row: &Row<'_>) -> rusqlite::Result<DbSendAsAlias> {
    Ok(DbSendAsAlias {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        email: row.get("email")?,
        display_name: row.get("display_name")?,
        reply_to_address: row.get("reply_to_address")?,
        signature_id: row.get("signature_id")?,
        is_primary: row.get("is_primary")?,
        is_default: row.get("is_default")?,
        treat_as_alias: row.get("treat_as_alias")?,
        verification_status: row.get("verification_status")?,
        created_at: row.get("created_at")?,
    })
}

#[tauri::command]
pub async fn db_get_aliases_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbSendAsAlias>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM send_as_aliases WHERE account_id = ?1 ORDER BY is_primary DESC, email",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id], row_to_send_as_alias)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_upsert_alias(
    state: State<'_, DbState>,
    account_id: String,
    email: String,
    display_name: Option<String>,
    reply_to_address: Option<String>,
    signature_id: Option<String>,
    is_primary: bool,
    is_default: bool,
    treat_as_alias: bool,
    verification_status: String,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let id_clone = id.clone();
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO send_as_aliases (id, account_id, email, display_name, reply_to_address, signature_id, is_primary, is_default, treat_as_alias, verification_status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(account_id, email) DO UPDATE SET
                   display_name = excluded.display_name,
                   reply_to_address = excluded.reply_to_address,
                   signature_id = excluded.signature_id,
                   is_primary = excluded.is_primary,
                   treat_as_alias = excluded.treat_as_alias,
                   verification_status = excluded.verification_status",
                params![
                    id_clone,
                    account_id,
                    email,
                    display_name,
                    reply_to_address,
                    signature_id,
                    i64::from(is_primary),
                    i64::from(is_default),
                    i64::from(treat_as_alias),
                    verification_status,
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await?;
    Ok(id)
}

#[tauri::command]
pub async fn db_get_default_alias(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Option<DbSendAsAlias>, String> {
    state
        .with_conn(move |conn| {
            // Try explicitly set default first
            let result = conn
                .query_row(
                    "SELECT * FROM send_as_aliases WHERE account_id = ?1 AND is_default = 1 LIMIT 1",
                    params![account_id],
                    row_to_send_as_alias,
                )
                .ok();
            if result.is_some() {
                return Ok(result);
            }
            // Fall back to primary
            Ok(conn
                .query_row(
                    "SELECT * FROM send_as_aliases WHERE account_id = ?1 AND is_primary = 1 LIMIT 1",
                    params![account_id],
                    row_to_send_as_alias,
                )
                .ok())
        })
        .await
}

#[tauri::command]
pub async fn db_set_default_alias(
    state: State<'_, DbState>,
    account_id: String,
    alias_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            tx.execute(
                "UPDATE send_as_aliases SET is_default = 0 WHERE account_id = ?1",
                params![account_id],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "UPDATE send_as_aliases SET is_default = 1 WHERE id = ?1 AND account_id = ?2",
                params![alias_id, account_id],
            )
            .map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_delete_alias(state: State<'_, DbState>, id: String) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute("DELETE FROM send_as_aliases WHERE id = ?1", params![id])
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ── Local Draft queries ────────────────────────────────────

fn row_to_local_draft(row: &Row<'_>) -> rusqlite::Result<DbLocalDraft> {
    Ok(DbLocalDraft {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        to_addresses: row.get("to_addresses")?,
        cc_addresses: row.get("cc_addresses")?,
        bcc_addresses: row.get("bcc_addresses")?,
        subject: row.get("subject")?,
        body_html: row.get("body_html")?,
        reply_to_message_id: row.get("reply_to_message_id")?,
        thread_id: row.get("thread_id")?,
        from_email: row.get("from_email")?,
        signature_id: row.get("signature_id")?,
        remote_draft_id: row.get("remote_draft_id")?,
        attachments: row.get("attachments")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        sync_status: row.get("sync_status")?,
    })
}

#[tauri::command]
pub async fn db_save_local_draft(
    state: State<'_, DbState>,
    id: String,
    account_id: String,
    to_addresses: Option<String>,
    cc_addresses: Option<String>,
    bcc_addresses: Option<String>,
    subject: Option<String>,
    body_html: Option<String>,
    reply_to_message_id: Option<String>,
    thread_id: Option<String>,
    from_email: Option<String>,
    signature_id: Option<String>,
    remote_draft_id: Option<String>,
    attachments: Option<String>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO local_drafts (id, account_id, to_addresses, cc_addresses, bcc_addresses, subject, body_html, reply_to_message_id, thread_id, from_email, signature_id, remote_draft_id, attachments, updated_at, sync_status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, unixepoch(), 'pending')
                 ON CONFLICT(id) DO UPDATE SET
                   to_addresses = ?3, cc_addresses = ?4, bcc_addresses = ?5,
                   subject = ?6, body_html = ?7, reply_to_message_id = ?8,
                   thread_id = ?9, from_email = ?10, signature_id = ?11,
                   remote_draft_id = ?12, attachments = ?13,
                   updated_at = unixepoch(), sync_status = 'pending'",
                params![
                    id, account_id, to_addresses, cc_addresses, bcc_addresses,
                    subject, body_html, reply_to_message_id, thread_id,
                    from_email, signature_id, remote_draft_id, attachments,
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_get_local_draft(
    state: State<'_, DbState>,
    id: String,
) -> Result<Option<DbLocalDraft>, String> {
    state
        .with_conn(move |conn| {
            Ok(conn
                .query_row(
                    "SELECT * FROM local_drafts WHERE id = ?1",
                    params![id],
                    row_to_local_draft,
                )
                .ok())
        })
        .await
}

#[tauri::command]
pub async fn db_get_unsynced_drafts(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbLocalDraft>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM local_drafts WHERE account_id = ?1 AND sync_status = 'pending' ORDER BY updated_at ASC",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id], row_to_local_draft)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_mark_draft_synced(
    state: State<'_, DbState>,
    id: String,
    remote_draft_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE local_drafts SET sync_status = 'synced', remote_draft_id = ?1 WHERE id = ?2",
                params![remote_draft_id, id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_delete_local_draft(state: State<'_, DbState>, id: String) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute("DELETE FROM local_drafts WHERE id = ?1", params![id])
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ── Scheduled Email queries ────────────────────────────────

fn row_to_scheduled_email(row: &Row<'_>) -> rusqlite::Result<DbScheduledEmail> {
    Ok(DbScheduledEmail {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        to_addresses: row.get("to_addresses")?,
        cc_addresses: row.get("cc_addresses")?,
        bcc_addresses: row.get("bcc_addresses")?,
        subject: row.get("subject")?,
        body_html: row.get("body_html")?,
        reply_to_message_id: row.get("reply_to_message_id")?,
        thread_id: row.get("thread_id")?,
        scheduled_at: row.get("scheduled_at")?,
        signature_id: row.get("signature_id")?,
        attachment_paths: row.get("attachment_paths")?,
        status: row.get("status")?,
        created_at: row.get("created_at")?,
    })
}

#[tauri::command]
pub async fn db_get_pending_scheduled_emails(
    state: State<'_, DbState>,
    now: i64,
) -> Result<Vec<DbScheduledEmail>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM scheduled_emails WHERE status = 'pending' AND scheduled_at <= ?1 ORDER BY scheduled_at ASC",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![now], row_to_scheduled_email)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_get_scheduled_emails_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbScheduledEmail>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM scheduled_emails WHERE account_id = ?1 AND status = 'pending' ORDER BY scheduled_at ASC",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id], row_to_scheduled_email)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_insert_scheduled_email(
    state: State<'_, DbState>,
    account_id: String,
    to_addresses: String,
    cc_addresses: Option<String>,
    bcc_addresses: Option<String>,
    subject: Option<String>,
    body_html: String,
    reply_to_message_id: Option<String>,
    thread_id: Option<String>,
    scheduled_at: i64,
    signature_id: Option<String>,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let id_clone = id.clone();
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO scheduled_emails (id, account_id, to_addresses, cc_addresses, bcc_addresses, subject, body_html, reply_to_message_id, thread_id, scheduled_at, signature_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    id_clone, account_id, to_addresses, cc_addresses, bcc_addresses,
                    subject, body_html, reply_to_message_id, thread_id,
                    scheduled_at, signature_id,
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await?;
    Ok(id)
}

#[tauri::command]
pub async fn db_update_scheduled_email_status(
    state: State<'_, DbState>,
    id: String,
    status: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE scheduled_emails SET status = ?1 WHERE id = ?2",
                params![status, id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_delete_scheduled_email(
    state: State<'_, DbState>,
    id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute("DELETE FROM scheduled_emails WHERE id = ?1", params![id])
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ── Label extra queries ────────────────────────────────────

#[tauri::command]
pub async fn db_upsert_label_coalesce(
    state: State<'_, DbState>,
    id: String,
    account_id: String,
    name: String,
    label_type: String,
    color_bg: Option<String>,
    color_fg: Option<String>,
    imap_folder_path: Option<String>,
    imap_special_use: Option<String>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
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

#[tauri::command]
pub async fn db_delete_labels_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "DELETE FROM labels WHERE account_id = ?1",
                params![account_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_update_label_sort_order(
    state: State<'_, DbState>,
    account_id: String,
    label_orders: Vec<LabelSortOrderItem>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
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

// ── Attachment extra queries ───────────────────────────────

#[tauri::command]
pub async fn db_upsert_attachment(
    state: State<'_, DbState>,
    id: String,
    message_id: String,
    account_id: String,
    filename: Option<String>,
    mime_type: Option<String>,
    size: Option<i64>,
    gmail_attachment_id: Option<String>,
    content_id: Option<String>,
    is_inline: bool,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO attachments (id, message_id, account_id, filename, mime_type, size, gmail_attachment_id, content_id, is_inline)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(id) DO UPDATE SET
                   filename = ?4, mime_type = ?5, size = ?6,
                   gmail_attachment_id = ?7, content_id = ?8, is_inline = ?9",
                params![
                    id, message_id, account_id, filename, mime_type, size,
                    gmail_attachment_id, content_id, i64::from(is_inline),
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_get_attachments_for_account(
    state: State<'_, DbState>,
    account_id: String,
    limit: i64,
    offset: i64,
) -> Result<Vec<AttachmentWithContext>, String> {
    state
        .with_conn(move |conn| {
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

#[tauri::command]
pub async fn db_get_attachment_senders(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<AttachmentSender>, String> {
    state
        .with_conn(move |conn| {
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

// ═══════════════════════════════════════════════════════════════
// CONTACTS — update avatar
// ═══════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn db_update_contact_avatar(
    state: State<'_, DbState>,
    email: String,
    avatar_url: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let normalized = email.to_lowercase();
            conn.execute(
                "UPDATE contacts SET avatar_url = ?1, updated_at = unixepoch() WHERE email = ?2",
                params![avatar_url, normalized],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ═══════════════════════════════════════════════════════════════
// ACCOUNTS
// ═══════════════════════════════════════════════════════════════

fn row_to_account(row: &Row<'_>) -> rusqlite::Result<DbAccount> {
    Ok(DbAccount {
        id: row.get("id")?,
        email: row.get("email")?,
        display_name: row.get("display_name")?,
        avatar_url: row.get("avatar_url")?,
        access_token: row.get("access_token")?,
        refresh_token: row.get("refresh_token")?,
        token_expires_at: row.get("token_expires_at")?,
        history_id: row.get("history_id")?,
        last_sync_at: row.get("last_sync_at")?,
        is_active: row.get("is_active")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        provider: row.get("provider")?,
        imap_host: row.get("imap_host")?,
        imap_port: row.get("imap_port")?,
        imap_security: row.get("imap_security")?,
        smtp_host: row.get("smtp_host")?,
        smtp_port: row.get("smtp_port")?,
        smtp_security: row.get("smtp_security")?,
        auth_method: row.get("auth_method")?,
        imap_password: row.get("imap_password")?,
        oauth_provider: row.get("oauth_provider")?,
        oauth_client_id: row.get("oauth_client_id")?,
        oauth_client_secret: row.get("oauth_client_secret")?,
        imap_username: row.get("imap_username")?,
        caldav_url: row.get("caldav_url")?,
        caldav_username: row.get("caldav_username")?,
        caldav_password: row.get("caldav_password")?,
        caldav_principal_url: row.get("caldav_principal_url")?,
        caldav_home_url: row.get("caldav_home_url")?,
        calendar_provider: row.get("calendar_provider")?,
        accept_invalid_certs: row.get("accept_invalid_certs")?,
        jmap_url: row.get("jmap_url")?,
    })
}

#[tauri::command]
pub async fn db_get_all_accounts(state: State<'_, DbState>) -> Result<Vec<DbAccount>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare("SELECT * FROM accounts ORDER BY created_at ASC")
                .map_err(|e| e.to_string())?;
            stmt.query_map([], row_to_account)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_get_account(
    state: State<'_, DbState>,
    id: String,
) -> Result<Option<DbAccount>, String> {
    state
        .with_conn(move |conn| {
            let result = conn
                .query_row(
                    "SELECT * FROM accounts WHERE id = ?1",
                    params![id],
                    row_to_account,
                )
                .ok();
            Ok(result)
        })
        .await
}

#[tauri::command]
pub async fn db_get_account_by_email(
    state: State<'_, DbState>,
    email: String,
) -> Result<Option<DbAccount>, String> {
    state
        .with_conn(move |conn| {
            let result = conn
                .query_row(
                    "SELECT * FROM accounts WHERE email = ?1",
                    params![email],
                    row_to_account,
                )
                .ok();
            Ok(result)
        })
        .await
}

/// Generic account insert — all fields optional except id, email, provider,
/// auth_method. Encrypted tokens are passed pre-encrypted from TS.
#[tauri::command]
pub async fn db_insert_account(
    state: State<'_, DbState>,
    id: String,
    email: String,
    display_name: Option<String>,
    avatar_url: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
    token_expires_at: Option<i64>,
    provider: String,
    auth_method: String,
    imap_host: Option<String>,
    imap_port: Option<i64>,
    imap_security: Option<String>,
    smtp_host: Option<String>,
    smtp_port: Option<i64>,
    smtp_security: Option<String>,
    imap_password: Option<String>,
    oauth_provider: Option<String>,
    oauth_client_id: Option<String>,
    oauth_client_secret: Option<String>,
    imap_username: Option<String>,
    accept_invalid_certs: Option<i64>,
    caldav_url: Option<String>,
    caldav_username: Option<String>,
    caldav_password: Option<String>,
    caldav_principal_url: Option<String>,
    caldav_home_url: Option<String>,
    calendar_provider: Option<String>,
    jmap_url: Option<String>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO accounts (id, email, display_name, avatar_url, access_token, \
                 refresh_token, token_expires_at, provider, auth_method, imap_host, imap_port, \
                 imap_security, smtp_host, smtp_port, smtp_security, imap_password, \
                 oauth_provider, oauth_client_id, oauth_client_secret, imap_username, \
                 accept_invalid_certs, caldav_url, caldav_username, caldav_password, \
                 caldav_principal_url, caldav_home_url, calendar_provider, jmap_url) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, \
                 ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28)",
                params![
                    id,
                    email,
                    display_name,
                    avatar_url,
                    access_token,
                    refresh_token,
                    token_expires_at,
                    provider,
                    auth_method,
                    imap_host,
                    imap_port,
                    imap_security,
                    smtp_host,
                    smtp_port,
                    smtp_security,
                    imap_password,
                    oauth_provider,
                    oauth_client_id,
                    oauth_client_secret,
                    imap_username,
                    accept_invalid_certs.unwrap_or(0),
                    caldav_url,
                    caldav_username,
                    caldav_password,
                    caldav_principal_url,
                    caldav_home_url,
                    calendar_provider,
                    jmap_url,
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_update_account_tokens(
    state: State<'_, DbState>,
    id: String,
    access_token: String,
    token_expires_at: i64,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE accounts SET access_token = ?1, token_expires_at = ?2, \
                 updated_at = unixepoch() WHERE id = ?3",
                params![access_token, token_expires_at, id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_update_account_all_tokens(
    state: State<'_, DbState>,
    id: String,
    access_token: String,
    refresh_token: String,
    token_expires_at: i64,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE accounts SET access_token = ?1, refresh_token = ?2, \
                 token_expires_at = ?3, updated_at = unixepoch() WHERE id = ?4",
                params![access_token, refresh_token, token_expires_at, id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_update_account_sync_state(
    state: State<'_, DbState>,
    id: String,
    history_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE accounts SET history_id = ?1, last_sync_at = unixepoch(), \
                 updated_at = unixepoch() WHERE id = ?2",
                params![history_id, id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_clear_account_history_id(
    state: State<'_, DbState>,
    id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE accounts SET history_id = NULL, updated_at = unixepoch() WHERE id = ?1",
                params![id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_delete_account(state: State<'_, DbState>, id: String) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute("DELETE FROM accounts WHERE id = ?1", params![id])
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_update_account_caldav(
    state: State<'_, DbState>,
    id: String,
    caldav_url: String,
    caldav_username: String,
    caldav_password: String,
    caldav_principal_url: Option<String>,
    caldav_home_url: Option<String>,
    calendar_provider: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE accounts SET caldav_url = ?1, caldav_username = ?2, caldav_password = ?3, \
                 caldav_principal_url = ?4, caldav_home_url = ?5, calendar_provider = ?6, \
                 updated_at = unixepoch() WHERE id = ?7",
                params![
                    caldav_url,
                    caldav_username,
                    caldav_password,
                    caldav_principal_url,
                    caldav_home_url,
                    calendar_provider,
                    id
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ═══════════════════════════════════════════════════════════════
// THREADS — extra mutations
// ═══════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn db_upsert_thread(
    state: State<'_, DbState>,
    id: String,
    account_id: String,
    subject: Option<String>,
    snippet: Option<String>,
    last_message_at: Option<i64>,
    message_count: i64,
    is_read: bool,
    is_starred: bool,
    is_important: bool,
    has_attachments: bool,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO threads (id, account_id, subject, snippet, last_message_at, message_count, is_read, is_starred, is_important, has_attachments)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(account_id, id) DO UPDATE SET
                   subject = ?3, snippet = ?4, last_message_at = ?5, message_count = ?6,
                   is_read = ?7, is_starred = ?8, is_important = ?9, has_attachments = ?10",
                params![
                    id,
                    account_id,
                    subject,
                    snippet,
                    last_message_at,
                    message_count,
                    is_read,
                    is_starred,
                    is_important,
                    has_attachments
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_set_thread_labels(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    label_ids: Vec<String>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            tx.execute(
                "DELETE FROM thread_labels WHERE account_id = ?1 AND thread_id = ?2",
                params![account_id, thread_id],
            )
            .map_err(|e| e.to_string())?;
            for label_id in &label_ids {
                tx.execute(
                    "INSERT OR IGNORE INTO thread_labels (account_id, thread_id, label_id) VALUES (?1, ?2, ?3)",
                    params![account_id, thread_id, label_id],
                )
                .map_err(|e| e.to_string())?;
            }
            tx.commit().map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_delete_all_threads_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "DELETE FROM threads WHERE account_id = ?1",
                params![account_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_get_muted_thread_ids(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<String>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare("SELECT id FROM threads WHERE account_id = ?1 AND is_muted = 1")
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id], |row| row.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_get_unread_inbox_count(state: State<'_, DbState>) -> Result<i64, String> {
    state
        .with_conn(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM threads t
                 INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
                 WHERE tl.label_id = 'INBOX' AND t.is_read = 0",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| e.to_string())
        })
        .await
}

// ═══════════════════════════════════════════════════════════════
// MESSAGES — extra queries/mutations
// ═══════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn db_get_messages_by_ids(
    state: State<'_, DbState>,
    account_id: String,
    message_ids: Vec<String>,
) -> Result<Vec<super::types::DbMessage>, String> {
    if message_ids.is_empty() {
        return Ok(Vec::new());
    }
    state
        .with_conn(move |conn| {
            let mut all_results = Vec::new();
            for chunk in message_ids.chunks(500) {
                let placeholders: String = chunk
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", i + 2))
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "SELECT * FROM messages WHERE account_id = ?1 AND id IN ({placeholders})"
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
                    .query_map(param_refs.as_slice(), row_to_message)
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?;
                all_results.extend(rows);
            }
            Ok(all_results)
        })
        .await
}

#[tauri::command]
pub async fn db_upsert_message(
    state: State<'_, DbState>,
    id: String,
    account_id: String,
    thread_id: String,
    from_address: Option<String>,
    from_name: Option<String>,
    to_addresses: Option<String>,
    cc_addresses: Option<String>,
    bcc_addresses: Option<String>,
    reply_to: Option<String>,
    subject: Option<String>,
    snippet: Option<String>,
    date: i64,
    is_read: bool,
    is_starred: bool,
    body_cached: bool,
    raw_size: Option<i64>,
    internal_date: Option<i64>,
    list_unsubscribe: Option<String>,
    list_unsubscribe_post: Option<String>,
    auth_results: Option<String>,
    message_id_header: Option<String>,
    references_header: Option<String>,
    in_reply_to_header: Option<String>,
    imap_uid: Option<i64>,
    imap_folder: Option<String>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO messages (id, account_id, thread_id, from_address, from_name, to_addresses, cc_addresses, bcc_addresses, reply_to, subject, snippet, date, is_read, is_starred, body_cached, raw_size, internal_date, list_unsubscribe, list_unsubscribe_post, auth_results, message_id_header, references_header, in_reply_to_header, imap_uid, imap_folder)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)
                 ON CONFLICT(account_id, id) DO UPDATE SET
                   from_address = ?4, from_name = ?5, to_addresses = ?6, cc_addresses = ?7,
                   bcc_addresses = ?8, reply_to = ?9, subject = ?10, snippet = ?11,
                   date = ?12, is_read = ?13, is_starred = ?14,
                   body_cached = CASE WHEN ?15 = 1 THEN 1 ELSE body_cached END,
                   raw_size = ?16, internal_date = ?17, list_unsubscribe = ?18, list_unsubscribe_post = ?19,
                   auth_results = ?20, message_id_header = COALESCE(?21, message_id_header),
                   references_header = COALESCE(?22, references_header),
                   in_reply_to_header = COALESCE(?23, in_reply_to_header),
                   imap_uid = COALESCE(?24, imap_uid), imap_folder = COALESCE(?25, imap_folder)",
                params![
                    id,
                    account_id,
                    thread_id,
                    from_address,
                    from_name,
                    to_addresses,
                    cc_addresses,
                    bcc_addresses,
                    reply_to,
                    subject,
                    snippet,
                    date,
                    is_read,
                    is_starred,
                    body_cached,
                    raw_size,
                    internal_date,
                    list_unsubscribe,
                    list_unsubscribe_post,
                    auth_results,
                    message_id_header,
                    references_header,
                    in_reply_to_header,
                    imap_uid,
                    imap_folder,
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_delete_message(
    state: State<'_, DbState>,
    account_id: String,
    message_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "DELETE FROM messages WHERE account_id = ?1 AND id = ?2",
                params![account_id, message_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_update_message_thread_ids(
    state: State<'_, DbState>,
    account_id: String,
    message_ids: Vec<String>,
    thread_id: String,
) -> Result<(), String> {
    if message_ids.is_empty() {
        return Ok(());
    }
    state
        .with_conn(move |conn| {
            for chunk in message_ids.chunks(500) {
                let placeholders: String = chunk
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", i + 3))
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "UPDATE messages SET thread_id = ?1 WHERE account_id = ?2 AND id IN ({placeholders})"
                );
                let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
                param_values.push(Box::new(thread_id.clone()));
                param_values.push(Box::new(account_id.clone()));
                for id in chunk {
                    param_values.push(Box::new(id.clone()));
                }
                let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                    param_values.iter().map(AsRef::as_ref).collect();
                conn.execute(&sql, param_refs.as_slice())
                    .map_err(|e| e.to_string())?;
            }
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_delete_all_messages_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "DELETE FROM messages WHERE account_id = ?1",
                params![account_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_get_recent_sent_messages(
    state: State<'_, DbState>,
    account_id: String,
    account_email: String,
    limit: Option<i64>,
) -> Result<Vec<super::types::DbMessage>, String> {
    state
        .with_conn(move |conn| {
            let lim = limit.unwrap_or(15);
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM messages
                     WHERE account_id = ?1 AND LOWER(from_address) = LOWER(?2)
                       AND body_cached = 1
                     ORDER BY date DESC LIMIT ?3",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id, account_email, lim], row_to_message)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}

// ═══════════════════════════════════════════════════════════════
// TASKS
// ═══════════════════════════════════════════════════════════════

fn row_to_task(row: &Row<'_>) -> rusqlite::Result<DbTask> {
    Ok(DbTask {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        title: row.get("title")?,
        description: row.get("description")?,
        priority: row.get("priority")?,
        is_completed: row.get("is_completed")?,
        completed_at: row.get("completed_at")?,
        due_date: row.get("due_date")?,
        parent_id: row.get("parent_id")?,
        thread_id: row.get("thread_id")?,
        thread_account_id: row.get("thread_account_id")?,
        sort_order: row.get("sort_order")?,
        recurrence_rule: row.get("recurrence_rule")?,
        next_recurrence_at: row.get("next_recurrence_at")?,
        tags_json: row.get("tags_json")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn row_to_task_tag(row: &Row<'_>) -> rusqlite::Result<DbTaskTag> {
    Ok(DbTaskTag {
        tag: row.get("tag")?,
        account_id: row.get("account_id")?,
        color: row.get("color")?,
        sort_order: row.get("sort_order")?,
        created_at: row.get("created_at")?,
    })
}

#[tauri::command]
pub async fn db_get_tasks_for_account(
    state: State<'_, DbState>,
    account_id: Option<String>,
    include_completed: Option<bool>,
) -> Result<Vec<DbTask>, String> {
    state
        .with_conn(move |conn| {
            if include_completed.unwrap_or(false) {
                let mut stmt = conn
                    .prepare(
                        "SELECT * FROM tasks WHERE (account_id = ?1 OR account_id IS NULL) AND parent_id IS NULL
                         ORDER BY is_completed ASC, sort_order ASC, created_at DESC",
                    )
                    .map_err(|e| e.to_string())?;
                stmt.query_map(params![account_id], row_to_task)
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())
            } else {
                let mut stmt = conn
                    .prepare(
                        "SELECT * FROM tasks WHERE (account_id = ?1 OR account_id IS NULL) AND parent_id IS NULL AND is_completed = 0
                         ORDER BY sort_order ASC, created_at DESC",
                    )
                    .map_err(|e| e.to_string())?;
                stmt.query_map(params![account_id], row_to_task)
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())
            }
        })
        .await
}

#[tauri::command]
pub async fn db_get_task_by_id(
    state: State<'_, DbState>,
    id: String,
) -> Result<Option<DbTask>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare("SELECT * FROM tasks WHERE id = ?1")
                .map_err(|e| e.to_string())?;
            let mut rows = stmt
                .query_map(params![id], row_to_task)
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
pub async fn db_get_tasks_for_thread(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<Vec<DbTask>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM tasks WHERE thread_account_id = ?1 AND thread_id = ?2
                     ORDER BY is_completed ASC, sort_order ASC, created_at DESC",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id, thread_id], row_to_task)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_get_subtasks(
    state: State<'_, DbState>,
    parent_id: String,
) -> Result<Vec<DbTask>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM tasks WHERE parent_id = ?1 ORDER BY sort_order ASC, created_at ASC",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![parent_id], row_to_task)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_insert_task(
    state: State<'_, DbState>,
    id: String,
    account_id: Option<String>,
    title: String,
    description: Option<String>,
    priority: Option<String>,
    due_date: Option<i64>,
    parent_id: Option<String>,
    thread_id: Option<String>,
    thread_account_id: Option<String>,
    sort_order: Option<i64>,
    recurrence_rule: Option<String>,
    tags_json: Option<String>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO tasks (id, account_id, title, description, priority, due_date, parent_id, thread_id, thread_account_id, sort_order, recurrence_rule, tags_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    id,
                    account_id,
                    title,
                    description,
                    priority.unwrap_or_else(|| "none".to_owned()),
                    due_date,
                    parent_id,
                    thread_id,
                    thread_account_id,
                    sort_order.unwrap_or(0),
                    recurrence_rule,
                    tags_json.unwrap_or_else(|| "[]".to_owned()),
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_update_task(
    state: State<'_, DbState>,
    id: String,
    title: Option<String>,
    description: Option<String>,
    priority: Option<String>,
    due_date: Option<i64>,
    sort_order: Option<i64>,
    recurrence_rule: Option<String>,
    next_recurrence_at: Option<i64>,
    tags_json: Option<String>,
    // Sentinel flags to distinguish "set to null" from "not provided"
    clear_description: Option<bool>,
    clear_due_date: Option<bool>,
    clear_recurrence_rule: Option<bool>,
    clear_next_recurrence_at: Option<bool>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let mut sets: Vec<(&str, Box<dyn rusqlite::types::ToSql>)> = Vec::new();
            sets.push(("updated_at", Box::new(chrono::Utc::now().timestamp())));

            if let Some(v) = title {
                sets.push(("title", Box::new(v)));
            }
            if clear_description.unwrap_or(false) {
                sets.push(("description", Box::new(Option::<String>::None)));
            } else if let Some(v) = description {
                sets.push(("description", Box::new(Some(v))));
            }
            if let Some(v) = priority {
                sets.push(("priority", Box::new(v)));
            }
            if clear_due_date.unwrap_or(false) {
                sets.push(("due_date", Box::new(Option::<i64>::None)));
            } else if let Some(v) = due_date {
                sets.push(("due_date", Box::new(Some(v))));
            }
            if let Some(v) = sort_order {
                sets.push(("sort_order", Box::new(v)));
            }
            if clear_recurrence_rule.unwrap_or(false) {
                sets.push(("recurrence_rule", Box::new(Option::<String>::None)));
            } else if let Some(v) = recurrence_rule {
                sets.push(("recurrence_rule", Box::new(Some(v))));
            }
            if clear_next_recurrence_at.unwrap_or(false) {
                sets.push(("next_recurrence_at", Box::new(Option::<i64>::None)));
            } else if let Some(v) = next_recurrence_at {
                sets.push(("next_recurrence_at", Box::new(Some(v))));
            }
            if let Some(v) = tags_json {
                sets.push(("tags_json", Box::new(v)));
            }

            dynamic_update(conn, "tasks", "id", &id, sets)
        })
        .await
}

#[tauri::command]
pub async fn db_delete_task(state: State<'_, DbState>, id: String) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute("DELETE FROM tasks WHERE id = ?1", params![id])
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_complete_task(state: State<'_, DbState>, id: String) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let now = chrono::Utc::now().timestamp();
            conn.execute(
                "UPDATE tasks SET is_completed = 1, completed_at = ?2, updated_at = ?2 WHERE id = ?1",
                params![id, now],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_uncomplete_task(state: State<'_, DbState>, id: String) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let now = chrono::Utc::now().timestamp();
            conn.execute(
                "UPDATE tasks SET is_completed = 0, completed_at = NULL, updated_at = ?2 WHERE id = ?1",
                params![id, now],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_reorder_tasks(
    state: State<'_, DbState>,
    task_ids: Vec<String>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            let now = chrono::Utc::now().timestamp();
            for (i, task_id) in task_ids.iter().enumerate() {
                tx.execute(
                    "UPDATE tasks SET sort_order = ?1, updated_at = ?3 WHERE id = ?2",
                    params![i as i64, task_id, now],
                )
                .map_err(|e| e.to_string())?;
            }
            tx.commit().map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_get_incomplete_task_count(
    state: State<'_, DbState>,
    account_id: Option<String>,
) -> Result<i64, String> {
    state
        .with_conn(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM tasks WHERE (account_id = ?1 OR account_id IS NULL) AND is_completed = 0",
                params![account_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_get_task_tags(
    state: State<'_, DbState>,
    account_id: Option<String>,
) -> Result<Vec<DbTaskTag>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM task_tags WHERE account_id = ?1 OR account_id IS NULL ORDER BY sort_order ASC",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id], row_to_task_tag)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_upsert_task_tag(
    state: State<'_, DbState>,
    tag: String,
    account_id: Option<String>,
    color: Option<String>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO task_tags (tag, account_id, color)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(tag, account_id) DO UPDATE SET color = ?3",
                params![tag, account_id, color],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_delete_task_tag(
    state: State<'_, DbState>,
    tag: String,
    account_id: Option<String>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "DELETE FROM task_tags WHERE tag = ?1 AND account_id = ?2",
                params![tag, account_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ═══════════════════════════════════════════════════════════════
// BUNDLE RULES — remaining queries
// ═══════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn db_get_bundle_rule(
    state: State<'_, DbState>,
    account_id: String,
    category: String,
) -> Result<Option<DbBundleRule>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare("SELECT * FROM bundle_rules WHERE account_id = ?1 AND category = ?2")
                .map_err(|e| e.to_string())?;
            let mut rows = stmt
                .query_map(params![account_id, category], row_to_bundle_rule)
                .map_err(|e| e.to_string())?;
            match rows.next() {
                Some(Ok(rule)) => Ok(Some(rule)),
                Some(Err(e)) => Err(e.to_string()),
                None => Ok(None),
            }
        })
        .await
}

#[tauri::command]
pub async fn db_set_bundle_rule(
    state: State<'_, DbState>,
    account_id: String,
    category: String,
    is_bundled: bool,
    delivery_enabled: bool,
    schedule: Option<String>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO bundle_rules (id, account_id, category, is_bundled, delivery_enabled, delivery_schedule)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(account_id, category) DO UPDATE SET
                   is_bundled = ?4, delivery_enabled = ?5, delivery_schedule = ?6",
                params![
                    id, account_id, category,
                    is_bundled as i64, delivery_enabled as i64, schedule
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_hold_thread(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    category: String,
    held_until: Option<i64>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO bundled_threads (account_id, thread_id, category, held_until)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(account_id, thread_id) DO UPDATE SET
                   category = ?3, held_until = ?4",
                params![account_id, thread_id, category, held_until],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_is_thread_held(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    now: i64,
) -> Result<bool, String> {
    state
        .with_conn(move |conn| {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM bundled_threads WHERE account_id = ?1 AND thread_id = ?2 AND held_until > ?3",
                    params![account_id, thread_id, now],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            Ok(count > 0)
        })
        .await
}

#[tauri::command]
pub async fn db_release_held_threads(
    state: State<'_, DbState>,
    account_id: String,
    category: String,
) -> Result<i64, String> {
    state
        .with_conn(move |conn| {
            let affected = conn
                .execute(
                    "DELETE FROM bundled_threads WHERE account_id = ?1 AND category = ?2 AND held_until IS NOT NULL",
                    params![account_id, category],
                )
                .map_err(|e| e.to_string())?;
            i64::try_from(affected).map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_update_last_delivered(
    state: State<'_, DbState>,
    account_id: String,
    category: String,
    now: i64,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE bundle_rules SET last_delivered_at = ?1 WHERE account_id = ?2 AND category = ?3",
                params![now, account_id, category],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_get_bundle_summary(
    state: State<'_, DbState>,
    account_id: String,
    category: String,
) -> Result<BundleSummarySingle, String> {
    state
        .with_conn(move |conn| {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(DISTINCT t.id)
                     FROM threads t
                     JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id AND tl.label_id = 'INBOX'
                     JOIN thread_categories tc ON tc.account_id = t.account_id AND tc.thread_id = t.id AND tc.category = ?2
                     WHERE t.account_id = ?1",
                    params![account_id, category],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;

            let latest = conn
                .query_row(
                    "SELECT t.subject, m.from_name
                     FROM threads t
                     JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id AND tl.label_id = 'INBOX'
                     JOIN thread_categories tc ON tc.account_id = t.account_id AND tc.thread_id = t.id AND tc.category = ?2
                     JOIN messages m ON m.account_id = t.account_id AND m.thread_id = t.id
                     WHERE t.account_id = ?1
                     ORDER BY t.last_message_at DESC LIMIT 1",
                    params![account_id, category],
                    |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, Option<String>>(1)?,
                        ))
                    },
                )
                .ok();

            let (latest_subject, latest_sender) = latest.unwrap_or((None, None));

            Ok(BundleSummarySingle {
                count,
                latest_subject,
                latest_sender,
            })
        })
        .await
}

// ═══════════════════════════════════════════════════════════════
// THREAD CATEGORIES — remaining queries
// ═══════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn db_get_thread_category(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<Option<String>, String> {
    state
        .with_conn(move |conn| {
            let result = conn.query_row(
                "SELECT category FROM thread_categories WHERE account_id = ?1 AND thread_id = ?2",
                params![account_id, thread_id],
                |row| row.get::<_, String>(0),
            );
            match result {
                Ok(cat) => Ok(Some(cat)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.to_string()),
            }
        })
        .await
}

#[tauri::command]
pub async fn db_get_thread_category_with_manual(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<Option<ThreadCategoryWithManual>, String> {
    state
        .with_conn(move |conn| {
            let result = conn.query_row(
                "SELECT category, is_manual FROM thread_categories WHERE account_id = ?1 AND thread_id = ?2",
                params![account_id, thread_id],
                |row| {
                    Ok(ThreadCategoryWithManual {
                        category: row.get(0)?,
                        is_manual: row.get::<_, i64>(1)? != 0,
                    })
                },
            );
            match result {
                Ok(tc) => Ok(Some(tc)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.to_string()),
            }
        })
        .await
}

#[tauri::command]
pub async fn db_get_recent_rule_categorized_thread_ids(
    state: State<'_, DbState>,
    account_id: String,
    limit: Option<i64>,
) -> Result<Vec<ThreadInfoRow>, String> {
    state
        .with_conn(move |conn| {
            let lim = limit.unwrap_or(20);
            let mut stmt = conn
                .prepare(
                    "SELECT t.id, t.subject, t.snippet, m.from_address
                     FROM threads t
                     INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
                     INNER JOIN thread_categories tc ON tc.account_id = t.account_id AND tc.thread_id = t.id
                     LEFT JOIN messages m ON m.account_id = t.account_id AND m.thread_id = t.id
                       AND m.date = (SELECT MAX(m2.date) FROM messages m2 WHERE m2.account_id = t.account_id AND m2.thread_id = t.id)
                     WHERE t.account_id = ?1 AND tl.label_id = 'INBOX' AND tc.is_manual = 0
                     ORDER BY t.last_message_at DESC
                     LIMIT ?2",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id, lim], |row| {
                Ok(ThreadInfoRow {
                    id: row.get(0)?,
                    subject: row.get(1)?,
                    snippet: row.get(2)?,
                    from_address: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_set_thread_categories_batch(
    state: State<'_, DbState>,
    account_id: String,
    categories: Vec<(String, String)>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            for (thread_id, category) in &categories {
                tx.execute(
                    "INSERT INTO thread_categories (account_id, thread_id, category, is_manual)
                     VALUES (?1, ?2, ?3, 0)
                     ON CONFLICT(account_id, thread_id) DO UPDATE SET
                       category = ?3
                     WHERE is_manual = 0",
                    params![account_id, thread_id, category],
                )
                .map_err(|e| e.to_string())?;
            }
            tx.commit().map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_get_uncategorized_inbox_thread_ids(
    state: State<'_, DbState>,
    account_id: String,
    limit: Option<i64>,
) -> Result<Vec<ThreadInfoRow>, String> {
    state
        .with_conn(move |conn| {
            let lim = limit.unwrap_or(20);
            let mut stmt = conn
                .prepare(
                    "SELECT t.id, t.subject, t.snippet, m.from_address
                     FROM threads t
                     INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
                     LEFT JOIN messages m ON m.account_id = t.account_id AND m.thread_id = t.id
                       AND m.date = (SELECT MAX(m2.date) FROM messages m2 WHERE m2.account_id = t.account_id AND m2.thread_id = t.id)
                     LEFT JOIN thread_categories tc ON tc.account_id = t.account_id AND tc.thread_id = t.id
                     WHERE t.account_id = ?1 AND tl.label_id = 'INBOX' AND tc.thread_id IS NULL
                     ORDER BY t.last_message_at DESC
                     LIMIT ?2",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id, lim], |row| {
                Ok(ThreadInfoRow {
                    id: row.get(0)?,
                    subject: row.get(1)?,
                    snippet: row.get(2)?,
                    from_address: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
}

// ═══════════════════════════════════════════════════════════════
// CALENDARS
// ═══════════════════════════════════════════════════════════════

fn row_to_calendar(row: &Row<'_>) -> rusqlite::Result<DbCalendar> {
    Ok(DbCalendar {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        provider: row.get("provider")?,
        remote_id: row.get("remote_id")?,
        display_name: row.get("display_name")?,
        color: row.get("color")?,
        is_primary: row.get("is_primary")?,
        is_visible: row.get("is_visible")?,
        sync_token: row.get("sync_token")?,
        ctag: row.get("ctag")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

#[tauri::command]
pub async fn db_upsert_calendar(
    state: State<'_, DbState>,
    account_id: String,
    provider: String,
    remote_id: String,
    display_name: Option<String>,
    color: Option<String>,
    is_primary: bool,
) -> Result<String, String> {
    state
        .with_conn(move |conn| {
            let id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO calendars (id, account_id, provider, remote_id, display_name, color, is_primary)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(account_id, remote_id) DO UPDATE SET
                   display_name = ?5, color = ?6, is_primary = ?7, updated_at = unixepoch()",
                params![
                    id, account_id, provider, remote_id, display_name, color,
                    is_primary as i64
                ],
            )
            .map_err(|e| e.to_string())?;
            let actual_id: String = conn
                .query_row(
                    "SELECT id FROM calendars WHERE account_id = ?1 AND remote_id = ?2",
                    params![account_id, remote_id],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            Ok(actual_id)
        })
        .await
}

#[tauri::command]
pub async fn db_get_calendars_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbCalendar>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM calendars WHERE account_id = ?1 \
                     ORDER BY is_primary DESC, display_name ASC",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id], row_to_calendar)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_get_visible_calendars(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbCalendar>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM calendars WHERE account_id = ?1 AND is_visible = 1 \
                     ORDER BY is_primary DESC, display_name ASC",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id], row_to_calendar)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_set_calendar_visibility(
    state: State<'_, DbState>,
    calendar_id: String,
    visible: bool,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE calendars SET is_visible = ?1, updated_at = unixepoch() WHERE id = ?2",
                params![visible as i64, calendar_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_update_calendar_sync_token(
    state: State<'_, DbState>,
    calendar_id: String,
    sync_token: Option<String>,
    ctag: Option<String>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE calendars SET sync_token = ?1, ctag = ?2, updated_at = unixepoch() WHERE id = ?3",
                params![sync_token, ctag, calendar_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_delete_calendars_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "DELETE FROM calendars WHERE account_id = ?1",
                params![account_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_get_calendar_by_id(
    state: State<'_, DbState>,
    calendar_id: String,
) -> Result<Option<DbCalendar>, String> {
    state
        .with_conn(move |conn| {
            let result = conn.query_row(
                "SELECT * FROM calendars WHERE id = ?1",
                params![calendar_id],
                row_to_calendar,
            );
            match result {
                Ok(cal) => Ok(Some(cal)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.to_string()),
            }
        })
        .await
}

// ═══════════════════════════════════════════════════════════════
// CALENDAR EVENTS
// ═══════════════════════════════════════════════════════════════

fn row_to_calendar_event(row: &Row<'_>) -> rusqlite::Result<DbCalendarEvent> {
    Ok(DbCalendarEvent {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        google_event_id: row.get("google_event_id")?,
        summary: row.get("summary")?,
        description: row.get("description")?,
        location: row.get("location")?,
        start_time: row.get("start_time")?,
        end_time: row.get("end_time")?,
        is_all_day: row.get("is_all_day")?,
        status: row.get("status")?,
        organizer_email: row.get("organizer_email")?,
        attendees_json: row.get("attendees_json")?,
        html_link: row.get("html_link")?,
        updated_at: row.get("updated_at")?,
        calendar_id: row.get("calendar_id")?,
        remote_event_id: row.get("remote_event_id")?,
        etag: row.get("etag")?,
        ical_data: row.get("ical_data")?,
        uid: row.get("uid")?,
    })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn db_upsert_calendar_event(
    state: State<'_, DbState>,
    account_id: String,
    google_event_id: String,
    summary: Option<String>,
    description: Option<String>,
    location: Option<String>,
    start_time: i64,
    end_time: i64,
    is_all_day: bool,
    status: String,
    organizer_email: Option<String>,
    attendees_json: Option<String>,
    html_link: Option<String>,
    calendar_id: Option<String>,
    remote_event_id: Option<String>,
    etag: Option<String>,
    ical_data: Option<String>,
    uid: Option<String>,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO calendar_events (id, account_id, google_event_id, summary, description, location, start_time, end_time, is_all_day, status, organizer_email, attendees_json, html_link, calendar_id, remote_event_id, etag, ical_data, uid)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
                 ON CONFLICT(account_id, google_event_id) DO UPDATE SET
                   summary = ?4, description = ?5, location = ?6, start_time = ?7, end_time = ?8,
                   is_all_day = ?9, status = ?10, organizer_email = ?11, attendees_json = ?12,
                   html_link = ?13, calendar_id = ?14, remote_event_id = ?15, etag = ?16,
                   ical_data = ?17, uid = ?18, updated_at = unixepoch()",
                params![
                    id, account_id, google_event_id, summary, description, location,
                    start_time, end_time, is_all_day as i64, status, organizer_email,
                    attendees_json, html_link, calendar_id, remote_event_id, etag,
                    ical_data, uid,
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_get_calendar_events_in_range(
    state: State<'_, DbState>,
    account_id: String,
    start_time: i64,
    end_time: i64,
) -> Result<Vec<DbCalendarEvent>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM calendar_events \
                     WHERE account_id = ?1 AND start_time < ?3 AND end_time > ?2 \
                     ORDER BY start_time ASC",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(
                params![account_id, start_time, end_time],
                row_to_calendar_event,
            )
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_get_calendar_events_in_range_multi(
    state: State<'_, DbState>,
    account_id: String,
    calendar_ids: Vec<String>,
    start_time: i64,
    end_time: i64,
) -> Result<Vec<DbCalendarEvent>, String> {
    if calendar_ids.is_empty() {
        return db_get_calendar_events_in_range(state, account_id, start_time, end_time).await;
    }
    state
        .with_conn(move |conn| {
            let placeholders: String = calendar_ids
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 4))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT * FROM calendar_events \
                 WHERE account_id = ?1 AND start_time < ?3 AND end_time > ?2 \
                   AND (calendar_id IN ({placeholders}) OR calendar_id IS NULL) \
                 ORDER BY start_time ASC"
            );
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            param_values.push(Box::new(account_id));
            param_values.push(Box::new(start_time));
            param_values.push(Box::new(end_time));
            for cid in &calendar_ids {
                param_values.push(Box::new(cid.clone()));
            }
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(AsRef::as_ref).collect();
            let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
            stmt.query_map(param_refs.as_slice(), row_to_calendar_event)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_delete_events_for_calendar(
    state: State<'_, DbState>,
    calendar_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "DELETE FROM calendar_events WHERE calendar_id = ?1",
                params![calendar_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_get_event_by_remote_id(
    state: State<'_, DbState>,
    calendar_id: String,
    remote_event_id: String,
) -> Result<Option<DbCalendarEvent>, String> {
    state
        .with_conn(move |conn| {
            let result = conn.query_row(
                "SELECT * FROM calendar_events WHERE calendar_id = ?1 AND remote_event_id = ?2",
                params![calendar_id, remote_event_id],
                row_to_calendar_event,
            );
            match result {
                Ok(evt) => Ok(Some(evt)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.to_string()),
            }
        })
        .await
}

#[tauri::command]
pub async fn db_delete_event_by_remote_id(
    state: State<'_, DbState>,
    calendar_id: String,
    remote_event_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "DELETE FROM calendar_events WHERE calendar_id = ?1 AND remote_event_id = ?2",
                params![calendar_id, remote_event_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_delete_calendar_event(
    state: State<'_, DbState>,
    event_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "DELETE FROM calendar_events WHERE id = ?1",
                params![event_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ═══════════════════════════════════════════════════════════════
// NON-DB SERVICE QUERIES (snooze, unsubscribe, imap, cache, backfill, composer)
// ═══════════════════════════════════════════════════════════════

// ── Snooze Manager ─────────────────────────────────────────

#[tauri::command]
pub async fn db_get_snoozed_threads_due(
    state: State<'_, DbState>,
    now: i64,
) -> Result<Vec<SnoozedThread>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id FROM threads WHERE is_snoozed = 1 AND snooze_until <= ?1",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![now], |row| {
                Ok(SnoozedThread {
                    id: row.get(0)?,
                    account_id: row.get(1)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
}

// ── Unsubscribe Manager ────────────────────────────────────

#[tauri::command]
pub async fn db_record_unsubscribe_action(
    state: State<'_, DbState>,
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
    state
        .with_conn(move |conn| {
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

#[tauri::command]
pub async fn db_get_subscriptions(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<SubscriptionEntry>, String> {
    state
        .with_conn(move |conn| {
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
            stmt.query_map(params![account_id], |row| {
                Ok(SubscriptionEntry {
                    from_address: row.get("from_address")?,
                    from_name: row.get("from_name")?,
                    latest_unsubscribe_header: row.get("latest_unsubscribe_header")?,
                    latest_unsubscribe_post: row.get("latest_unsubscribe_post")?,
                    message_count: row.get("message_count")?,
                    latest_date: row.get("latest_date")?,
                    status: row.get("status")?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_get_unsubscribe_status(
    state: State<'_, DbState>,
    account_id: String,
    from_address: String,
) -> Result<Option<String>, String> {
    let from_address = from_address.to_lowercase();
    state
        .with_conn(move |conn| {
            Ok(conn
                .query_row(
                    "SELECT status FROM unsubscribe_actions WHERE account_id = ?1 AND from_address = ?2",
                    params![account_id, from_address],
                    |row| row.get(0),
                )
                .ok())
        })
        .await
}

// ── IMAP Message Helper ────────────────────────────────────

#[tauri::command]
pub async fn db_get_imap_uids_for_messages(
    state: State<'_, DbState>,
    account_id: String,
    message_ids: Vec<String>,
) -> Result<Vec<ImapMessageRow>, String> {
    if message_ids.is_empty() {
        return Ok(vec![]);
    }
    state
        .with_conn(move |conn| {
            let placeholders: Vec<String> = (0..message_ids.len()).map(|i| format!("?{}", i + 2)).collect();
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
            let param_refs: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
            let rows = stmt
                .query_map(param_refs.as_slice(), |row| {
                    Ok(ImapMessageRow {
                        id: row.get(0)?,
                        imap_uid: row.get(1)?,
                        imap_folder: row.get(2)?,
                    })
                })
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_find_special_folder(
    state: State<'_, DbState>,
    account_id: String,
    special_use: String,
    fallback_label_id: Option<String>,
) -> Result<Option<String>, String> {
    state
        .with_conn(move |conn| {
            // Primary: look up by imap_special_use attribute
            let result: Option<SpecialFolderRow> = conn
                .query_row(
                    "SELECT imap_folder_path, name FROM labels WHERE account_id = ?1 AND imap_special_use = ?2 LIMIT 1",
                    params![account_id, special_use],
                    |row| Ok(SpecialFolderRow {
                        imap_folder_path: row.get(0)?,
                        name: row.get(1)?,
                    }),
                )
                .ok();
            if let Some(row) = result {
                return Ok(Some(row.imap_folder_path.unwrap_or(row.name)));
            }
            // Fallback: look up by well-known label ID
            if let Some(label_id) = fallback_label_id {
                let fallback: Option<SpecialFolderRow> = conn
                    .query_row(
                        "SELECT imap_folder_path, name FROM labels WHERE account_id = ?1 AND id = ?2 AND imap_folder_path IS NOT NULL LIMIT 1",
                        params![account_id, label_id],
                        |row| Ok(SpecialFolderRow {
                            imap_folder_path: row.get(0)?,
                            name: row.get(1)?,
                        }),
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

#[tauri::command]
pub async fn db_update_message_imap_folder(
    state: State<'_, DbState>,
    account_id: String,
    message_ids: Vec<String>,
    new_folder: String,
) -> Result<(), String> {
    if message_ids.is_empty() {
        return Ok(());
    }
    state
        .with_conn(move |conn| {
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
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(|p| p.as_ref()).collect();
            conn.execute(&sql, param_refs.as_slice())
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ── Attachment Cache Manager ───────────────────────────────

#[tauri::command]
pub async fn db_update_attachment_cached(
    state: State<'_, DbState>,
    attachment_id: String,
    local_path: String,
    cache_size: i64,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE attachments SET local_path = ?1, cached_at = unixepoch(), cache_size = ?2 WHERE id = ?3",
                params![local_path, cache_size, attachment_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_get_attachment_cache_size(state: State<'_, DbState>) -> Result<i64, String> {
    state
        .with_conn(move |conn| {
            let total: i64 = conn
                .query_row(
                    "SELECT COALESCE(SUM(cache_size), 0) FROM attachments WHERE cached_at IS NOT NULL",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            Ok(total)
        })
        .await
}

#[tauri::command]
pub async fn db_get_oldest_cached_attachments(
    state: State<'_, DbState>,
    limit: i64,
) -> Result<Vec<CachedAttachmentRow>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, local_path, cache_size, content_hash FROM attachments WHERE cached_at IS NOT NULL ORDER BY cached_at ASC LIMIT ?1",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![limit], |row| {
                Ok(CachedAttachmentRow {
                    id: row.get(0)?,
                    local_path: row.get(1)?,
                    cache_size: row.get(2)?,
                    content_hash: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
}

#[tauri::command]
pub async fn db_clear_attachment_cache_entry(
    state: State<'_, DbState>,
    attachment_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE attachments SET local_path = NULL, cached_at = NULL, cache_size = NULL WHERE id = ?1",
                params![attachment_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn db_clear_all_attachment_cache(state: State<'_, DbState>) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE attachments SET local_path = NULL, cached_at = NULL, cache_size = NULL WHERE cached_at IS NOT NULL",
                [],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

/// Count cached attachments sharing a content hash (for safe eviction).
#[tauri::command]
pub async fn db_count_cached_by_hash(
    state: State<'_, DbState>,
    content_hash: String,
) -> Result<i64, String> {
    state
        .with_conn(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM attachments WHERE content_hash = ?1 AND cached_at IS NOT NULL",
                params![content_hash],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())
        })
        .await
}

// ── Smart Label Backfill ───────────────────────────────────

#[tauri::command]
pub async fn db_get_inbox_threads_for_backfill(
    state: State<'_, DbState>,
    account_id: String,
    batch_size: i64,
    offset: i64,
) -> Result<Vec<BackfillRow>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT t.id AS thread_id, t.subject, t.snippet,
                            m.from_address, m.from_name,
                            m.to_addresses, t.has_attachments, m.id
                     FROM threads t
                     INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
                     LEFT JOIN messages m ON m.account_id = t.account_id AND m.thread_id = t.id
                       AND m.date = (SELECT MAX(m2.date) FROM messages m2 WHERE m2.account_id = t.account_id AND m2.thread_id = t.id)
                     WHERE t.account_id = ?1 AND tl.label_id = 'INBOX'
                     ORDER BY t.last_message_at DESC
                     LIMIT ?2 OFFSET ?3",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id, batch_size, offset], |row| {
                Ok(BackfillRow {
                    thread_id: row.get("thread_id")?,
                    subject: row.get("subject")?,
                    snippet: row.get("snippet")?,
                    from_address: row.get("from_address")?,
                    from_name: row.get("from_name")?,
                    to_addresses: row.get("to_addresses")?,
                    has_attachments: row.get("has_attachments")?,
                    id: row.get("id")?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
}

// ── Composer: scheduled email attachment update ─────────────

#[tauri::command]
pub async fn db_update_scheduled_email_attachments(
    state: State<'_, DbState>,
    account_id: String,
    attachment_data: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let id: Option<String> = conn
                .query_row(
                    "SELECT id FROM scheduled_emails WHERE account_id = ?1 ORDER BY created_at DESC LIMIT 1",
                    params![account_id],
                    |row| row.get(0),
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

// ── Smart folder raw SQL queries ───────────────────────────

#[tauri::command]
pub async fn db_query_raw_select(
    state: State<'_, DbState>,
    sql: String,
    params: Vec<serde_json::Value>,
) -> Result<Vec<serde_json::Map<String, serde_json::Value>>, String> {
    state
        .with_conn(move |conn| {
            let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
            let col_count = stmt.column_count();
            let col_names: Vec<String> = (0..col_count)
                .map(|i| stmt.column_name(i).map(|s| s.to_string()))
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
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(|p| p.as_ref()).collect();

            let rows = stmt
                .query_map(param_refs.as_slice(), |row| {
                    let mut map = serde_json::Map::new();
                    for (i, name) in col_names.iter().enumerate() {
                        let val: rusqlite::types::Value = row.get(i)?;
                        let json_val = match val {
                            rusqlite::types::Value::Null => serde_json::Value::Null,
                            rusqlite::types::Value::Integer(n) => {
                                serde_json::Value::Number(n.into())
                            }
                            rusqlite::types::Value::Real(f) => {
                                serde_json::json!(f)
                            }
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
