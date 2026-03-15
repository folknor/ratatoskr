use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use crate::categorization::AiCategorizationCandidate;
use crate::db::queries_extra::load_recent_rule_categorized_threads;
use crate::db::DbState;
use crate::filters::FilterableMessage;
use crate::sync::types::NotificationCandidate;

/// Check settings and return threads that need AI categorization.
pub async fn get_ai_categorization_candidates(
    db: &DbState,
    account_id: &str,
) -> Result<Vec<AiCategorizationCandidate>, String> {
    let account_id = account_id.to_string();
    db.with_conn(move |conn| {
        let auto_categorize = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'ai_auto_categorize'",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok();
        if auto_categorize.as_deref() == Some("false") {
            return Ok(Vec::new());
        }

        load_recent_rule_categorized_threads(conn, &account_id, 20).map(|threads| {
            threads
                .into_iter()
                .map(|thread| AiCategorizationCandidate {
                    id: thread.id,
                    subject: thread.subject,
                    snippet: thread.snippet,
                    from_address: thread.from_address,
                })
                .collect()
        })
    })
    .await
}

/// Evaluate which new messages should trigger desktop notifications.
///
/// Returns an empty list when `is_delta` is false (initial sync) or
/// when notifications are disabled in settings.
pub async fn evaluate_notifications(
    db: &DbState,
    account_id: &str,
    messages: &[FilterableMessage],
    is_delta: bool,
) -> Result<Vec<NotificationCandidate>, String> {
    if !is_delta || messages.is_empty() {
        return Ok(Vec::new());
    }

    let account_id = account_id.to_string();
    let messages = messages.to_vec();
    let thread_ids: Vec<String> = messages.iter().map(|msg| msg.thread_id.clone()).collect();
    db.with_conn(move |conn| evaluate_notifications_sync(conn, &account_id, &messages, &thread_ids))
        .await
}

fn evaluate_notifications_sync(
    conn: &Connection,
    account_id: &str,
    messages: &[FilterableMessage],
    thread_ids: &[String],
) -> Result<Vec<NotificationCandidate>, String> {
    let notifications_enabled: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'notifications_enabled'",
            [],
            |row| row.get(0),
        )
        .ok();
    if notifications_enabled.as_deref() == Some("false") {
        return Ok(Vec::new());
    }

    let smart_notifications = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'smart_notifications'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .unwrap_or_else(|| "true".to_string())
        == "true";
    let notify_categories = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'notify_categories'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .unwrap_or_else(|| "Primary".to_string());
    let allowed_categories: HashSet<String> = notify_categories
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let vip_senders = load_vip_senders(conn, account_id)?;
    let muted_thread_ids = load_muted_thread_ids(conn, account_id, thread_ids)?;
    let category_by_thread = load_thread_categories(conn, account_id, thread_ids)?;

    let mut candidates = Vec::new();
    for msg in messages {
        if muted_thread_ids.contains(&msg.thread_id) {
            continue;
        }
        let from_normalized = msg
            .from_address
            .as_ref()
            .map(|email| email.trim().to_lowercase());
        let should_notify = if !smart_notifications {
            true
        } else if let Some(from_addr) = from_normalized.as_ref() {
            if vip_senders.contains(from_addr) {
                true
            } else {
                category_allowed(&category_by_thread, &msg.thread_id, &allowed_categories)
            }
        } else {
            category_allowed(&category_by_thread, &msg.thread_id, &allowed_categories)
        };

        if should_notify {
            candidates.push(NotificationCandidate {
                thread_id: msg.thread_id.clone(),
                from_name: msg.from_name.clone(),
                from_address: msg.from_address.clone(),
                subject: msg.subject.clone(),
            });
        }
    }

    Ok(candidates)
}

fn category_allowed(
    category_by_thread: &HashMap<String, String>,
    thread_id: &str,
    allowed: &HashSet<String>,
) -> bool {
    let category = category_by_thread
        .get(thread_id)
        .map(String::as_str)
        .unwrap_or("Primary");
    allowed.contains(category)
}

fn load_vip_senders(conn: &Connection, account_id: &str) -> Result<HashSet<String>, String> {
    let mut stmt = conn
        .prepare("SELECT email_address FROM notification_vips WHERE account_id = ?1")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![account_id], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|email| email.trim().to_lowercase())
        .collect())
}

fn load_muted_thread_ids(
    conn: &Connection,
    account_id: &str,
    thread_ids: &[String],
) -> Result<HashSet<String>, String> {
    if thread_ids.is_empty() {
        return Ok(HashSet::new());
    }
    let placeholders: String = thread_ids
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 2))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT id FROM threads WHERE account_id = ?1 AND is_muted = 1 AND id IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(account_id.to_string())];
    for id in thread_ids {
        param_values.push(Box::new(id.clone()));
    }
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(AsRef::as_ref).collect();
    let rows = stmt
        .query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().collect())
}

fn load_thread_categories(
    conn: &Connection,
    account_id: &str,
    thread_ids: &[String],
) -> Result<HashMap<String, String>, String> {
    let mut category_by_thread = HashMap::new();
    for chunk in thread_ids.chunks(100) {
        if chunk.is_empty() {
            continue;
        }
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
        param_values.push(Box::new(account_id.to_string()));
        for id in chunk {
            param_values.push(Box::new(id.clone()));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(AsRef::as_ref).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        category_by_thread.extend(rows);
    }
    Ok(category_by_thread)
}
