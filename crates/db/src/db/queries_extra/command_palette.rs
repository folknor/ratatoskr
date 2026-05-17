//! Folder and label-group queries for command palette population.
//!
//! Returns simple row structs. OptionItem mapping stays in core (avoids
//! db depending on cmdk).

use rusqlite::params;

use crate::db::ReadConn;

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

/// User-visible folders for an account, excluding rows the user cannot
/// delete. `is_undeletable = 0` covers system roles (INBOX, SENT, etc.)
/// AND Gmail's `type: "system"` non-role labels like `CATEGORY_*` and
/// `CHAT`, which live in `folders` but should not appear in user-facing
/// folder pickers.
pub fn get_user_folders_for_account_sync(
    conn: &ReadConn<'_>,
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
pub fn get_label_groups_for_palette_sync(conn: &ReadConn<'_>) -> Result<Vec<LabelGroupRow>, String> {
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
    conn: &ReadConn<'_>,
    account_id: &str,
    thread_id: &str,
) -> Result<Vec<LabelGroupRow>, String> {
    let group_fragment = super::user_visible_label_group_rendered_fragment(
        "t.account_id",
        "t.id",
        "lg.id = lg_outer.id",
    );
    let sql = format!(
        "SELECT lg_outer.id AS group_id, lg_outer.name
         FROM threads t
         INNER JOIN label_groups lg_outer
         WHERE t.account_id = ?1
           AND t.id = ?2
           AND {group_fragment}
         GROUP BY lg_outer.id
         ORDER BY lg_outer.name COLLATE NOCASE ASC"
    );
    let mut stmt = conn
        .prepare(&sql)
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
