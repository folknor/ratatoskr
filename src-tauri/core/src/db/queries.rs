use std::collections::HashMap;

use rusqlite::{Connection, Row, params};

use super::DbState;
use super::types::{
    CategoryCount, DbAttachment, DbContact, DbLabel, DbMessage, DbThread, ThreadCategoryRow,
};
use crate::body_store::{BodyStoreState, MessageBody};
use crate::provider::crypto::{decrypt_value, is_encrypted};

pub(crate) fn row_to_thread(row: &Row<'_>) -> rusqlite::Result<DbThread> {
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

pub(crate) fn row_to_message(row: &Row<'_>) -> rusqlite::Result<DbMessage> {
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
        body_cached: row
            .get::<_, Option<i64>>("body_cached")?
            .map(|value| value != 0),
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
        content_hash: row.get("content_hash")?,
    })
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsBootstrapSnapshot {
    pub notifications_enabled: bool,
    pub undo_send_delay_seconds: Option<String>,
    pub block_remote_images: bool,
    pub phishing_detection_enabled: bool,
    pub phishing_sensitivity: Option<String>,
    pub sync_period_days: Option<String>,
    pub ai_provider: Option<String>,
    pub ollama_server_url: Option<String>,
    pub ollama_model: Option<String>,
    pub claude_model: Option<String>,
    pub openai_model: Option<String>,
    pub gemini_model: Option<String>,
    pub copilot_model: Option<String>,
    pub ai_enabled: bool,
    pub ai_auto_categorize: bool,
    pub ai_auto_summarize: bool,
    pub ai_auto_draft_enabled: bool,
    pub ai_writing_style_enabled: bool,
    pub auto_archive_categories: Option<String>,
    pub smart_notifications: bool,
    pub notify_categories: Option<String>,
    pub attachment_cache_max_mb: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSecretsSnapshot {
    pub claude_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub gemini_api_key: Option<String>,
    pub copilot_api_key: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiBootstrapSnapshot {
    pub active_account_id: Option<String>,
    pub language: Option<String>,
    pub global_compose_shortcut: Option<String>,
    pub custom_shortcuts: Option<String>,
    pub search_index_version: Option<String>,
    pub theme: Option<String>,
    pub sidebar_collapsed: bool,
    pub contact_sidebar_visible: bool,
    pub reading_pane_position: Option<String>,
    pub read_filter: Option<String>,
    pub email_list_width: Option<String>,
    pub email_density: Option<String>,
    pub default_reply_mode: Option<String>,
    pub mark_as_read_behavior: Option<String>,
    pub send_and_archive: bool,
    pub font_size: Option<String>,
    pub color_theme: Option<String>,
    pub inbox_view_mode: Option<String>,
    pub show_sync_status: bool,
    pub task_sidebar_visible: bool,
    pub sidebar_nav_config: Option<String>,
}

const SECURE_SETTING_KEYS: &[&str] = &[
    "claude_api_key",
    "openai_api_key",
    "gemini_api_key",
    "copilot_api_key",
];

fn decode_secure_setting_value(raw: String, encryption_key: &[u8; 32]) -> String {
    if is_encrypted(&raw) {
        decrypt_value(encryption_key, &raw).unwrap_or(raw)
    } else {
        raw
    }
}

fn read_setting_map(
    conn: &Connection,
    encryption_key: &[u8; 32],
    secure_keys: &[&str],
) -> Result<HashMap<String, String>, String> {
    let mut stmt = conn
        .prepare("SELECT key, value FROM settings")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(|(key, raw)| {
            let value = if secure_keys.contains(&key.as_str()) {
                decode_secure_setting_value(raw, encryption_key)
            } else {
                raw
            };
            (key, value)
        })
        .collect())
}

pub fn get_threads(
    conn: &Connection,
    account_id: String,
    label_id: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DbThread>, String> {
    let lim = limit.unwrap_or(50);
    let off = offset.unwrap_or(0);

    if let Some(ref lid) = label_id {
        let mut stmt = conn
            .prepare(
                "SELECT t.*, m.from_name, m.from_address FROM threads t
                 INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
                 LEFT JOIN (
                   SELECT id, account_id, thread_id, from_name, from_address FROM (
                     SELECT id, account_id, thread_id, from_name, from_address,
                            ROW_NUMBER() OVER (
                              PARTITION BY account_id, thread_id
                              ORDER BY date DESC, id DESC
                            ) AS rn
                     FROM messages
                   ) WHERE rn = 1
                 ) m ON m.account_id = t.account_id AND m.thread_id = t.id
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
                 LEFT JOIN (
                   SELECT id, account_id, thread_id, from_name, from_address FROM (
                     SELECT id, account_id, thread_id, from_name, from_address,
                            ROW_NUMBER() OVER (
                              PARTITION BY account_id, thread_id
                              ORDER BY date DESC, id DESC
                            ) AS rn
                     FROM messages
                   ) WHERE rn = 1
                 ) m ON m.account_id = t.account_id AND m.thread_id = t.id
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
}

pub fn get_threads_for_category(
    conn: &Connection,
    account_id: String,
    category: String,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DbThread>, String> {
    let lim = limit.unwrap_or(50);
    let off = offset.unwrap_or(0);

    if category == "Primary" {
        let mut stmt = conn
            .prepare(
                "SELECT t.*, m.from_name, m.from_address FROM threads t
                 INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
                 LEFT JOIN thread_categories tc ON tc.account_id = t.account_id AND tc.thread_id = t.id
                 LEFT JOIN (
                   SELECT id, account_id, thread_id, from_name, from_address FROM (
                     SELECT id, account_id, thread_id, from_name, from_address,
                            ROW_NUMBER() OVER (
                              PARTITION BY account_id, thread_id
                              ORDER BY date DESC, id DESC
                            ) AS rn
                     FROM messages
                   ) WHERE rn = 1
                 ) m ON m.account_id = t.account_id AND m.thread_id = t.id
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
                 LEFT JOIN (
                   SELECT id, account_id, thread_id, from_name, from_address FROM (
                     SELECT id, account_id, thread_id, from_name, from_address,
                            ROW_NUMBER() OVER (
                              PARTITION BY account_id, thread_id
                              ORDER BY date DESC, id DESC
                            ) AS rn
                     FROM messages
                   ) WHERE rn = 1
                 ) m ON m.account_id = t.account_id AND m.thread_id = t.id
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
}

pub fn get_thread_by_id(
    conn: &Connection,
    account_id: String,
    thread_id: String,
) -> Result<Option<DbThread>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT t.*, m.from_name, m.from_address FROM threads t
             LEFT JOIN (
               SELECT id, account_id, thread_id, from_name, from_address FROM (
                 SELECT id, account_id, thread_id, from_name, from_address,
                        ROW_NUMBER() OVER (
                          PARTITION BY account_id, thread_id
                          ORDER BY date DESC, id DESC
                        ) AS rn
                 FROM messages
               ) WHERE rn = 1
             ) m ON m.account_id = t.account_id AND m.thread_id = t.id
             WHERE t.account_id = ?1 AND t.id = ?2
             LIMIT 1",
        )
        .map_err(|e| e.to_string())?;

    let mut rows = stmt
        .query_map(params![account_id, thread_id], row_to_thread)
        .map_err(|e| e.to_string())?;

    match rows.next() {
        Some(Ok(thread)) => Ok(Some(thread)),
        Some(Err(error)) => Err(error.to_string()),
        None => Ok(None),
    }
}

pub fn get_thread_label_ids(
    conn: &Connection,
    account_id: String,
    thread_id: String,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT label_id FROM thread_labels WHERE account_id = ?1 AND thread_id = ?2")
        .map_err(|e| e.to_string())?;

    stmt.query_map(params![account_id, thread_id], |row| {
        row.get::<_, String>(0)
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

pub async fn get_messages_for_thread(
    db: &DbState,
    body_store: &BodyStoreState,
    account_id: String,
    thread_id: String,
) -> Result<Vec<DbMessage>, String> {
    let mut messages = db
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

    let ids_needing_bodies: Vec<String> =
        messages.iter().map(|message| message.id.clone()).collect();
    if ids_needing_bodies.is_empty() {
        return Ok(messages);
    }

    let bodies = body_store.get_batch(ids_needing_bodies).await?;
    let body_map: HashMap<String, MessageBody> = bodies
        .into_iter()
        .map(|body| (body.message_id.clone(), body))
        .collect();

    for message in &mut messages {
        if let Some(body) = body_map.get(&message.id) {
            message.body_html = body.body_html.clone();
            message.body_text = body.body_text.clone();
        }
    }

    Ok(messages)
}

pub fn get_labels(conn: &Connection, account_id: String) -> Result<Vec<DbLabel>, String> {
    let mut stmt = conn
        .prepare("SELECT * FROM labels WHERE account_id = ?1 ORDER BY sort_order ASC, name ASC")
        .map_err(|e| e.to_string())?;

    stmt.query_map(params![account_id], row_to_label)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub fn get_setting(conn: &Connection, key: String) -> Result<Option<String>, String> {
    let result = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .ok();
    Ok(result)
}

pub fn get_secure_setting(
    conn: &Connection,
    encryption_key: &[u8; 32],
    key: String,
) -> Result<Option<String>, String> {
    let result = get_setting(conn, key)?;
    Ok(result.map(|raw| decode_secure_setting_value(raw, encryption_key)))
}

pub fn get_settings_bootstrap_snapshot(
    conn: &Connection,
    encryption_key: &[u8; 32],
) -> Result<SettingsBootstrapSnapshot, String> {
    let settings = read_setting_map(conn, encryption_key, &[])?;
    let get = |key: &str| settings.get(key).cloned();
    let get_bool = |key: &str, default: bool| get(key).map_or(default, |value| value != "false");

    Ok(SettingsBootstrapSnapshot {
        notifications_enabled: get_bool("notifications_enabled", true),
        undo_send_delay_seconds: get("undo_send_delay_seconds"),
        block_remote_images: get_bool("block_remote_images", true),
        phishing_detection_enabled: get_bool("phishing_detection_enabled", true),
        phishing_sensitivity: get("phishing_sensitivity"),
        sync_period_days: get("sync_period_days"),
        ai_provider: get("ai_provider"),
        ollama_server_url: get("ollama_server_url"),
        ollama_model: get("ollama_model"),
        claude_model: get("claude_model"),
        openai_model: get("openai_model"),
        gemini_model: get("gemini_model"),
        copilot_model: get("copilot_model"),
        ai_enabled: get_bool("ai_enabled", true),
        ai_auto_categorize: get_bool("ai_auto_categorize", true),
        ai_auto_summarize: get_bool("ai_auto_summarize", true),
        ai_auto_draft_enabled: get_bool("ai_auto_draft_enabled", true),
        ai_writing_style_enabled: get_bool("ai_writing_style_enabled", true),
        auto_archive_categories: get("auto_archive_categories"),
        smart_notifications: get_bool("smart_notifications", true),
        notify_categories: get("notify_categories"),
        attachment_cache_max_mb: get("attachment_cache_max_mb"),
    })
}

pub fn get_settings_secrets_snapshot(
    conn: &Connection,
    encryption_key: &[u8; 32],
) -> Result<SettingsSecretsSnapshot, String> {
    let settings = read_setting_map(conn, encryption_key, SECURE_SETTING_KEYS)?;
    let get = |key: &str| settings.get(key).cloned();

    Ok(SettingsSecretsSnapshot {
        claude_api_key: get("claude_api_key"),
        openai_api_key: get("openai_api_key"),
        gemini_api_key: get("gemini_api_key"),
        copilot_api_key: get("copilot_api_key"),
    })
}

pub fn get_ui_bootstrap_snapshot(
    conn: &Connection,
    encryption_key: &[u8; 32],
) -> Result<UiBootstrapSnapshot, String> {
    let settings = read_setting_map(conn, encryption_key, &[])?;
    let get = |key: &str| settings.get(key).cloned();
    let get_bool = |key: &str, default: bool| get(key).map_or(default, |value| value != "false");

    Ok(UiBootstrapSnapshot {
        active_account_id: get("active_account_id"),
        language: get("language"),
        global_compose_shortcut: get("global_compose_shortcut"),
        custom_shortcuts: get("custom_shortcuts"),
        search_index_version: get("search_index_version"),
        theme: get("theme"),
        sidebar_collapsed: get("sidebar_collapsed").is_some_and(|value| value == "true"),
        contact_sidebar_visible: get_bool("contact_sidebar_visible", true),
        reading_pane_position: get("reading_pane_position"),
        read_filter: get("read_filter"),
        email_list_width: get("email_list_width"),
        email_density: get("email_density"),
        default_reply_mode: get("default_reply_mode"),
        mark_as_read_behavior: get("mark_as_read_behavior"),
        send_and_archive: get("send_and_archive").is_some_and(|value| value == "true"),
        font_size: get("font_size"),
        color_theme: get("color_theme"),
        inbox_view_mode: get("inbox_view_mode"),
        show_sync_status: get_bool("show_sync_status", true),
        task_sidebar_visible: get("task_sidebar_visible").is_some_and(|value| value == "true"),
        sidebar_nav_config: get("sidebar_nav_config"),
    })
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{
        get_settings_bootstrap_snapshot, get_settings_secrets_snapshot, get_ui_bootstrap_snapshot,
    };
    use crate::provider::crypto::encrypt_value;

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute(
            "CREATE TABLE settings (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            )",
            [],
        )
        .expect("create settings table");
        conn
    }

    fn insert_setting(conn: &Connection, key: &str, value: &str) {
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)",
            rusqlite::params![key, value],
        )
        .expect("insert setting");
    }

    #[test]
    fn ui_bootstrap_ignores_secure_settings() {
        let conn = setup_conn();
        let key = [11_u8; 32];
        let encrypted_secret = encrypt_value(&key, "top-secret").expect("encrypt api key");
        insert_setting(&conn, "theme", "dark");
        insert_setting(&conn, "language", "en");
        insert_setting(&conn, "claude_api_key", &encrypted_secret);

        let snapshot = get_ui_bootstrap_snapshot(&conn, &key).expect("ui snapshot");

        assert_eq!(snapshot.theme.as_deref(), Some("dark"));
        assert_eq!(snapshot.language.as_deref(), Some("en"));
    }

    #[test]
    fn settings_bootstrap_excludes_secure_fields() {
        let conn = setup_conn();
        let key = [13_u8; 32];
        insert_setting(&conn, "notifications_enabled", "false");

        let snapshot = get_settings_bootstrap_snapshot(&conn, &key).expect("settings snapshot");

        assert!(!snapshot.notifications_enabled);
    }

    #[test]
    fn secure_snapshot_decrypts_only_secret_fields() {
        let conn = setup_conn();
        let key = [17_u8; 32];
        let encrypted_openai_key =
            encrypt_value(&key, "openai-secret").expect("encrypt openai api key");
        insert_setting(&conn, "openai_api_key", &encrypted_openai_key);

        let snapshot = get_settings_secrets_snapshot(&conn, &key).expect("secure snapshot");

        assert_eq!(snapshot.openai_api_key.as_deref(), Some("openai-secret"));
        assert_eq!(snapshot.gemini_api_key, None);
    }
}

pub fn set_setting(conn: &Connection, key: String, value: String) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
        params![key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_thread_read(
    conn: &Connection,
    account_id: String,
    thread_id: String,
    is_read: bool,
) -> Result<(), String> {
    conn.execute(
        "UPDATE threads SET is_read = ?3 WHERE account_id = ?1 AND id = ?2",
        params![account_id, thread_id, is_read],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_thread_starred(
    conn: &Connection,
    account_id: String,
    thread_id: String,
    is_starred: bool,
) -> Result<(), String> {
    conn.execute(
        "UPDATE threads SET is_starred = ?3 WHERE account_id = ?1 AND id = ?2",
        params![account_id, thread_id, is_starred],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_thread_pinned(
    conn: &Connection,
    account_id: String,
    thread_id: String,
    is_pinned: bool,
) -> Result<(), String> {
    conn.execute(
        "UPDATE threads SET is_pinned = ?3 WHERE account_id = ?1 AND id = ?2",
        params![account_id, thread_id, is_pinned],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_thread_muted(
    conn: &Connection,
    account_id: String,
    thread_id: String,
    is_muted: bool,
) -> Result<(), String> {
    conn.execute(
        "UPDATE threads SET is_muted = ?3 WHERE account_id = ?1 AND id = ?2",
        params![account_id, thread_id, is_muted],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_thread(
    conn: &Connection,
    account_id: String,
    thread_id: String,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM threads WHERE account_id = ?1 AND id = ?2",
        params![account_id, thread_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn add_thread_label(
    conn: &Connection,
    account_id: String,
    thread_id: String,
    label_id: String,
) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO thread_labels (account_id, thread_id, label_id) VALUES (?1, ?2, ?3)",
        params![account_id, thread_id, label_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn remove_thread_label(
    conn: &Connection,
    account_id: String,
    thread_id: String,
    label_id: String,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM thread_labels WHERE account_id = ?1 AND thread_id = ?2 AND label_id = ?3",
        params![account_id, thread_id, label_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn upsert_label(conn: &Connection, label: DbLabel) -> Result<(), String> {
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
}

pub fn delete_label(conn: &Connection, account_id: String, label_id: String) -> Result<(), String> {
    conn.execute(
        "DELETE FROM labels WHERE account_id = ?1 AND id = ?2",
        params![account_id, label_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_category_unread_counts(
    conn: &Connection,
    account_id: String,
) -> Result<Vec<CategoryCount>, String> {
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
}

pub fn get_categories_for_threads(
    conn: &Connection,
    account_id: String,
    thread_ids: Vec<String>,
) -> Result<Vec<ThreadCategoryRow>, String> {
    if thread_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut all_results = Vec::new();
    for chunk in thread_ids.chunks(100) {
        let placeholders: String = chunk
            .iter()
            .enumerate()
            .map(|(index, _)| format!("?{}", index + 2))
            .collect::<Vec<_>>()
            .join(", ");

        let sql = format!(
            "SELECT thread_id, category FROM thread_categories WHERE account_id = ?1 AND thread_id IN ({placeholders})"
        );

        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        param_values.push(Box::new(account_id.clone()));
        for thread_id in chunk {
            param_values.push(Box::new(thread_id.clone()));
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

pub fn get_attachments_for_message(
    conn: &Connection,
    account_id: String,
    message_id: String,
) -> Result<Vec<DbAttachment>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT * FROM attachments WHERE account_id = ?1 AND message_id = ?2 ORDER BY filename ASC",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map(params![account_id, message_id], row_to_attachment)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub fn search_contacts(
    conn: &Connection,
    query: String,
    limit: i64,
) -> Result<Vec<DbContact>, String> {
    let pattern = format!("%{query}%");

    let mut stmt = conn
        .prepare(
            "SELECT id, email, display_name, avatar_url, frequency,
                    last_contacted_at, notes, 1 AS source_rank
             FROM contacts
             WHERE email LIKE ?1 OR display_name LIKE ?1

             UNION ALL

             SELECT '' AS id, sa.email, sa.display_name, NULL AS avatar_url,
               CAST(
                 (sa.times_sent_to * 3.0 + sa.times_sent_cc * 1.5
                  + sa.times_received_from * 1.0 + sa.times_received_cc * 0.5)
                 / (1.0 + CAST((unixepoch() * 1000 - sa.last_seen_at) AS REAL)
                    / 86400000.0 / 90.0)
               AS INTEGER) AS frequency,
               NULL AS last_contacted_at, NULL AS notes, 2 AS source_rank
             FROM seen_addresses sa
             WHERE (sa.email LIKE ?1 OR sa.display_name LIKE ?1)
               AND sa.email NOT IN (
                 SELECT email FROM contacts
                 WHERE email LIKE ?1 OR display_name LIKE ?1
               )

             ORDER BY source_rank ASC, frequency DESC, display_name ASC
             LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map(params![pattern, limit], row_to_contact)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub fn get_contact_by_email(conn: &Connection, email: String) -> Result<Option<DbContact>, String> {
    let normalized = email.to_lowercase();

    let mut stmt = conn
        .prepare("SELECT * FROM contacts WHERE email = ?1 LIMIT 1")
        .map_err(|e| e.to_string())?;

    let mut rows = stmt
        .query_map(params![normalized], row_to_contact)
        .map_err(|e| e.to_string())?;

    match rows.next() {
        Some(Ok(contact)) => Ok(Some(contact)),
        Some(Err(error)) => Err(error.to_string()),
        None => Ok(None),
    }
}

pub fn get_thread_count(
    conn: &Connection,
    account_id: String,
    label_id: Option<String>,
) -> Result<i64, String> {
    if let Some(ref label_id) = label_id {
        conn.query_row(
            "SELECT COUNT(DISTINCT t.id) FROM threads t
             INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
             WHERE t.account_id = ?1 AND tl.label_id = ?2",
            params![account_id, label_id],
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
}

pub fn get_unread_count(conn: &Connection, account_id: String) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM threads t
         INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
         WHERE t.account_id = ?1 AND tl.label_id = 'INBOX' AND t.is_read = 0",
        params![account_id],
        |row| row.get::<_, i64>(0),
    )
    .map_err(|e| e.to_string())
}
