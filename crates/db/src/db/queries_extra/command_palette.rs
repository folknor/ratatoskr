//! Raw label/account queries for command palette population.
//!
//! Returns simple row structs. OptionItem mapping stays in core (avoids
//! db depending on cmdk).

use rusqlite::{Connection, params};

/// A label row: id + name.
pub struct LabelRow {
    pub id: String,
    pub name: String,
}

/// A label row with account context for cross-account display.
pub struct CrossAccountLabelRow {
    pub account_id: String,
    pub account_name: String,
    pub label_id: String,
    pub label_name: String,
    pub label_kind: String,
}

/// User-visible labels for an account, excluding system labels.
pub fn get_user_labels_for_account_sync(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<LabelRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name FROM labels
             WHERE account_id = ?1 AND type != 'system' AND visible = 1
             ORDER BY sort_order ASC, name ASC",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map(params![account_id], |row| {
        Ok(LabelRow {
            id: row.get("id")?,
            name: row.get("name")?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

/// Labels currently applied to a specific thread.
pub fn get_thread_labels_sync(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
) -> Result<Vec<LabelRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT l.id, l.name FROM labels l
             INNER JOIN thread_labels tl
               ON tl.account_id = l.account_id AND tl.label_id = l.id
             WHERE tl.account_id = ?1 AND tl.thread_id = ?2
               AND l.type != 'system' AND l.visible = 1
             ORDER BY l.sort_order ASC, l.name ASC",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map(params![account_id, thread_id], |row| {
        Ok(LabelRow {
            id: row.get("id")?,
            name: row.get("name")?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

/// All user labels across all active accounts with account context.
pub fn get_all_labels_cross_account_sync(
    conn: &Connection,
) -> Result<Vec<CrossAccountLabelRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT a.id AS account_id,
                    COALESCE(a.display_name, a.email) AS account_name,
                    l.id AS label_id,
                    l.name AS label_name,
                    l.label_kind
             FROM labels l
             INNER JOIN accounts a ON a.id = l.account_id
             WHERE l.type != 'system' AND l.visible = 1 AND a.is_active = 1
             ORDER BY a.email ASC, l.sort_order ASC, l.name ASC",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map([], |row| {
        Ok(CrossAccountLabelRow {
            account_id: row.get("account_id")?,
            account_name: row.get("account_name")?,
            label_id: row.get("label_id")?,
            label_name: row.get("label_name")?,
            label_kind: row.get("label_kind")?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

/// Check whether an account uses folder-based semantics.
pub fn is_folder_based_provider_sync(
    conn: &Connection,
    account_id: &str,
) -> Result<bool, String> {
    let provider: String = conn
        .query_row(
            "SELECT provider FROM accounts WHERE id = ?1",
            params![account_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(provider != "gmail_api")
}
