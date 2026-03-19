use rusqlite::{Connection, params};

use super::from_row::FromRow;
use super::sql_fragments::LATEST_MESSAGE_SUBQUERY;
use super::types::ThreadInfoRow;

/// Read a single value from the `settings` table, returning `Ok(None)` when
/// the key does not exist.
pub fn get_setting(conn: &Connection, key: String) -> Result<Option<String>, String> {
    let result = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>("value"),
        )
        .ok();
    Ok(result)
}

/// Persist a refreshed access token to the `accounts` table.
///
/// The caller is responsible for encrypting the token before calling this.
pub fn persist_refreshed_token(
    conn: &Connection,
    account_id: &str,
    encrypted_access_token: &str,
    expires_at: i64,
) -> Result<(), String> {
    conn.execute(
        "UPDATE accounts SET access_token = ?1, token_expires_at = ?2, \
         updated_at = unixepoch() WHERE id = ?3",
        params![encrypted_access_token, expires_at, account_id],
    )
    .map_err(|e| format!("Failed to persist refreshed token: {e}"))?;
    Ok(())
}

/// Color fields for a category upsert. If `None`, the column is set to NULL on
/// insert and left unchanged on conflict.
pub struct CategoryColors<'a> {
    pub preset: Option<&'a str>,
    pub bg: Option<&'a str>,
    pub fg: Option<&'a str>,
}

/// Whether to update `sort_order` on conflict.
pub enum CategorySortOnConflict {
    /// Keep the existing sort_order on conflict.
    Keep,
    /// Overwrite the sort_order on conflict (uses the same `sort_order` value
    /// passed for the INSERT).
    Update,
}

/// Upsert a category row. All providers share the same INSERT shape; the ON
/// CONFLICT clause varies in which columns are updated.
///
/// - `update_colors_on_conflict` — if true, color columns are overwritten on
///   conflict (Graph and Gmail). If false, only `provider_id` and `sync_state`
///   are updated (IMAP, JMAP).
pub fn upsert_category(
    conn: &Connection,
    id: &str,
    account_id: &str,
    display_name: &str,
    colors: &CategoryColors<'_>,
    provider_id: &str,
    sort_order: i64,
    update_colors_on_conflict: bool,
    sort_on_conflict: CategorySortOnConflict,
) -> Result<(), String> {
    let sort_clause = match sort_on_conflict {
        CategorySortOnConflict::Keep => "",
        CategorySortOnConflict::Update => ", sort_order = ?8",
    };

    let color_clause = if update_colors_on_conflict {
        "color_preset = ?4, color_bg = ?5, color_fg = ?6, "
    } else {
        ""
    };

    let sql = format!(
        "INSERT INTO categories \
         (id, account_id, display_name, color_preset, color_bg, color_fg, \
          provider_id, sync_state, sort_order) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'synced', ?8) \
         ON CONFLICT(account_id, display_name) DO UPDATE SET \
           {color_clause}provider_id = ?7, sync_state = 'synced'{sort_clause}"
    );

    conn.execute(
        &sql,
        params![
            id,
            account_id,
            display_name,
            colors.preset,
            colors.bg,
            colors.fg,
            provider_id,
            sort_order,
        ],
    )
    .map_err(|e| format!("upsert category: {e}"))?;
    Ok(())
}

pub fn load_recent_rule_categorized_threads(
    conn: &Connection,
    account_id: &str,
    limit: i64,
) -> Result<Vec<ThreadInfoRow>, String> {
    let sql = format!(
        "SELECT t.id, t.subject, t.snippet, m.from_address
         FROM threads t
         INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
         INNER JOIN thread_categories tc ON tc.account_id = t.account_id AND tc.thread_id = t.id
         LEFT JOIN ({LATEST_MESSAGE_SUBQUERY}
         ) m ON m.account_id = t.account_id AND m.thread_id = t.id
         WHERE t.account_id = ?1 AND tl.label_id = 'INBOX' AND tc.is_manual = 0
         ORDER BY t.last_message_at DESC
         LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    stmt.query_map(params![account_id, limit], ThreadInfoRow::from_row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}
