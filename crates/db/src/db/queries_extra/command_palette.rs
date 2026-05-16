//! Folder and label-group queries for command palette population.
//!
//! Returns simple row structs. OptionItem mapping stays in core (avoids
//! db depending on cmdk).

use rusqlite::{Connection, params};

/// A label row: id + name.
pub struct LabelRow {
    pub id: String,
    pub name: String,
}

/// A label group row: id + name.
pub struct LabelGroupRow {
    pub id: i64,
    pub name: String,
}

/// User-visible folders for an account, excluding system folders.
pub fn get_user_folders_for_account_sync(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<LabelRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name FROM folders
             WHERE account_id = ?1 AND visible = 1
               AND is_undeletable = 0
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

/// User-visible label groups.
pub fn get_label_groups_for_palette_sync(conn: &Connection) -> Result<Vec<LabelGroupRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name FROM label_groups
             ORDER BY name COLLATE NOCASE ASC",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map([], |row| {
        Ok(LabelGroupRow {
            id: row.get("id")?,
            name: row.get("name")?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

/// Label groups currently rendered for a specific thread.
pub fn get_thread_label_groups_sync(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
) -> Result<Vec<LabelGroupRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT group_id, name
             FROM (
               SELECT lg.id AS group_id, lg.name
               FROM thread_label_groups tlg
               INNER JOIN label_groups lg ON lg.id = tlg.group_id
               WHERE tlg.account_id = ?1 AND tlg.thread_id = ?2
               UNION
               SELECT lg.id AS group_id, lg.name
               FROM thread_labels tl
               INNER JOIN label_group_members lgm
                 ON lgm.account_id = tl.account_id AND lgm.label_id = tl.label_id
               INNER JOIN label_groups lg ON lg.id = lgm.group_id
               WHERE tl.account_id = ?1 AND tl.thread_id = ?2
             )
             ORDER BY name COLLATE NOCASE ASC",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map(params![account_id, thread_id], |row| {
        Ok(LabelGroupRow {
            id: row.get("group_id")?,
            name: row.get("name")?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}
