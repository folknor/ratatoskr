use std::collections::HashMap;

use super::{Connection, Row, params};

use super::DbState;
use super::types::DbMessage;
use crate::body_store::{BodyStoreState, MessageBody};
use crate::provider::crypto::{decrypt_value, is_encrypted};

// Re-export everything from db::queries so existing callers keep working.
pub use db::db::queries::{
    add_thread_label, delete_label, delete_thread, get_attachments_for_message,
    get_bundle_unread_counts, get_categories_for_threads, get_contact_by_email, get_labels,
    get_provider_type, get_setting, get_thread_by_id, get_thread_count, get_thread_label_ids,
    get_threads, get_threads_for_bundle, get_unread_count, remove_thread_label, search_contacts,
    set_setting, set_thread_muted, set_thread_pinned, set_thread_read, set_thread_starred,
    upsert_label,
};
// Re-export FTS5/LIKE helpers.
pub use db::db::sql_fragments::{build_fts_query, make_like_pattern};

pub(crate) fn row_to_message(row: &Row<'_>) -> std::result::Result<DbMessage, super::SqlError> {
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
            Ok((row.get::<_, String>("key")?, row.get::<_, String>("value")?))
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

#[cfg_attr(feature = "hotpath", hotpath::measure)]
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

pub fn get_secure_setting(
    conn: &Connection,
    encryption_key: &[u8; 32],
    key: &str,
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
    use crate::db::Connection;

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
            params![key, value],
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

// Remaining functions (set_thread_bool_field through get_unread_count)
// moved to db::queries in Phase E step 4 and re-exported above.
