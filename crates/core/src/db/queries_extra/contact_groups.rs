use std::collections::HashSet;

use rusqlite::params;

use super::super::DbState;
use super::super::types::{DbContactGroup, DbContactGroupMember};
use crate::db::from_row::FromRow;

pub async fn db_create_contact_group(db: &DbState, id: String, name: String) -> Result<(), String> {
    log::debug!("Creating contact group: id={id}, name={name}");
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO contact_groups (id, name) VALUES (?1, ?2)",
            params![id, name],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_update_contact_group(db: &DbState, id: String, name: String) -> Result<(), String> {
    log::debug!("Updating contact group: id={id}, name={name}");
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE contact_groups SET name = ?1, updated_at = unixepoch() WHERE id = ?2",
            params![name, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_delete_contact_group(db: &DbState, id: String) -> Result<(), String> {
    log::debug!("Deleting contact group: id={id}");
    db.with_conn(move |conn| {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("begin tx: {e}"))?;

        // Remove inbound nested-group references from other groups
        tx.execute(
            "DELETE FROM contact_group_members WHERE member_type = 'group' AND member_value = ?1",
            params![id],
        )
        .map_err(|e| format!("delete inbound refs: {e}"))?;

        // Delete the group itself (CASCADE removes owned members)
        tx.execute("DELETE FROM contact_groups WHERE id = ?1", params![id])
            .map_err(|e| format!("delete group: {e}"))?;

        tx.commit().map_err(|e| format!("commit tx: {e}"))?;
        Ok(())
    })
    .await
    .map_err(|e| {
        log::error!("Failed to delete contact group: {e}");
        e
    })
}

pub async fn db_get_all_contact_groups(db: &DbState) -> Result<Vec<DbContactGroup>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT g.id, g.name,
                        (SELECT COUNT(*) FROM contact_group_members WHERE group_id = g.id) AS member_count,
                        g.created_at, g.updated_at
                 FROM contact_groups g
                 ORDER BY g.name ASC",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map([], DbContactGroup::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_contact_group(db: &DbState, id: String) -> Result<DbContactGroup, String> {
    db.with_conn(move |conn| {
        conn.query_row(
            "SELECT g.id, g.name,
                    (SELECT COUNT(*) FROM contact_group_members WHERE group_id = g.id) AS member_count,
                    g.created_at, g.updated_at
             FROM contact_groups g
             WHERE g.id = ?1",
            params![id],
            DbContactGroup::from_row,
        )
        .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_contact_group_members(
    db: &DbState,
    group_id: String,
) -> Result<Vec<DbContactGroupMember>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT member_type, member_value
                 FROM contact_group_members
                 WHERE group_id = ?1
                 ORDER BY member_type ASC, member_value ASC",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![group_id], DbContactGroupMember::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_add_contact_group_member(
    db: &DbState,
    group_id: String,
    member_type: String,
    member_value: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let normalized_value = if member_type == "email" {
            member_value.to_lowercase()
        } else {
            member_value
        };
        conn.execute(
            "INSERT OR IGNORE INTO contact_group_members (group_id, member_type, member_value)
             VALUES (?1, ?2, ?3)",
            params![group_id, member_type, normalized_value],
        )
        .map_err(|e| e.to_string())?;

        // Touch updated_at on the parent group
        conn.execute(
            "UPDATE contact_groups SET updated_at = unixepoch() WHERE id = ?1",
            params![group_id],
        )
        .map_err(|e| e.to_string())?;

        Ok(())
    })
    .await
}

pub async fn db_remove_contact_group_member(
    db: &DbState,
    group_id: String,
    member_type: String,
    member_value: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM contact_group_members
             WHERE group_id = ?1 AND member_type = ?2 AND member_value = ?3",
            params![group_id, member_type, member_value],
        )
        .map_err(|e| e.to_string())?;

        // Touch updated_at on the parent group
        conn.execute(
            "UPDATE contact_groups SET updated_at = unixepoch() WHERE id = ?1",
            params![group_id],
        )
        .map_err(|e| e.to_string())?;

        Ok(())
    })
    .await
}

pub async fn db_search_contact_groups(
    db: &DbState,
    query: String,
    limit: i64,
) -> Result<Vec<DbContactGroup>, String> {
    log::debug!("Searching contact groups: query={query}, limit={limit}");
    db.with_conn(move |conn| {
        let pattern = format!("%{query}%");
        let mut stmt = conn
            .prepare(
                "SELECT g.id, g.name,
                        (SELECT COUNT(*) FROM contact_group_members WHERE group_id = g.id) AS member_count,
                        g.created_at, g.updated_at
                 FROM contact_groups g
                 WHERE g.name LIKE ?1
                 ORDER BY g.name ASC
                 LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![pattern, limit], DbContactGroup::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_expand_contact_group(
    db: &DbState,
    group_id: String,
) -> Result<Vec<String>, String> {
    db.with_conn(move |conn| {
        let mut visited = HashSet::new();
        let mut emails = HashSet::new();
        expand_recursive(conn, &group_id, &mut visited, &mut emails)?;
        let mut result: Vec<String> = emails.into_iter().collect();
        result.sort();
        Ok(result)
    })
    .await
}

// ── Synchronous group helpers (for app-layer settings UI) ──

/// A group entry for the settings UI.
#[derive(Debug, Clone)]
pub struct GroupSettingsEntry {
    pub id: String,
    pub name: String,
    pub member_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Load groups with optional name filter (synchronous).
pub fn load_groups_for_settings_sync(
    conn: &rusqlite::Connection,
    filter: &str,
) -> Result<Vec<GroupSettingsEntry>, String> {
    let trimmed = filter.trim();
    let pattern = format!("%{trimmed}%");

    let sql = if trimmed.is_empty() {
        "SELECT g.id, g.name, g.created_at, g.updated_at,
                (SELECT COUNT(*) FROM contact_group_members m
                 WHERE m.group_id = g.id) AS member_count
         FROM contact_groups g
         ORDER BY g.updated_at DESC
         LIMIT 100"
    } else {
        "SELECT g.id, g.name, g.created_at, g.updated_at,
                (SELECT COUNT(*) FROM contact_group_members m
                 WHERE m.group_id = g.id) AS member_count
         FROM contact_groups g
         WHERE g.name LIKE ?1
         ORDER BY g.updated_at DESC
         LIMIT 100"
    };

    let db_params: &[&dyn rusqlite::types::ToSql] =
        if trimmed.is_empty() { &[] } else { &[&pattern] };

    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(db_params, |row| {
            Ok(GroupSettingsEntry {
                id: row.get("id")?,
                name: row.get("name")?,
                member_count: row.get("member_count")?,
                created_at: row.get("created_at")?,
                updated_at: row.get("updated_at")?,
            })
        })
        .map_err(|e| e.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Load member emails for a group (synchronous).
pub fn load_group_member_emails_sync(
    conn: &rusqlite::Connection,
    group_id: &str,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT member_value FROM contact_group_members
             WHERE group_id = ?1 AND member_type = 'email'
             ORDER BY member_value ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![group_id], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Save (upsert) a group and replace all members (synchronous).
pub fn save_group_sync(
    conn: &rusqlite::Connection,
    entry: &GroupSettingsEntry,
    member_emails: &[String],
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO contact_groups (id, name, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?3)
         ON CONFLICT(id) DO UPDATE SET
             name = excluded.name,
             updated_at = excluded.updated_at",
        params![entry.id, entry.name, now],
    )
    .map_err(|e| e.to_string())?;

    // Replace all members
    conn.execute(
        "DELETE FROM contact_group_members WHERE group_id = ?1",
        params![entry.id],
    )
    .map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare(
            "INSERT INTO contact_group_members
             (group_id, member_type, member_value)
             VALUES (?1, 'email', ?2)",
        )
        .map_err(|e| e.to_string())?;

    for email in member_emails {
        stmt.execute(params![entry.id, email])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Delete a group and clean up inbound refs (synchronous).
pub fn delete_group_sync(conn: &rusqlite::Connection, group_id: &str) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("begin tx: {e}"))?;

    // Remove inbound nested-group references from other groups
    tx.execute(
        "DELETE FROM contact_group_members \
         WHERE member_type = 'group' AND member_value = ?1",
        params![group_id],
    )
    .map_err(|e| format!("delete inbound refs: {e}"))?;

    // Delete the group itself (CASCADE removes owned members)
    tx.execute(
        "DELETE FROM contact_groups WHERE id = ?1",
        params![group_id],
    )
    .map_err(|e| format!("delete group: {e}"))?;

    tx.commit().map_err(|e| format!("commit tx: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn expand_recursive(
    conn: &rusqlite::Connection,
    group_id: &str,
    visited: &mut HashSet<String>,
    emails: &mut HashSet<String>,
) -> Result<(), String> {
    if !visited.insert(group_id.to_string()) {
        return Ok(()); // cycle detection
    }

    let mut stmt = conn
        .prepare("SELECT member_type, member_value FROM contact_group_members WHERE group_id = ?1")
        .map_err(|e| format!("prepare expand: {e}"))?;

    let members: Vec<(String, String)> = stmt
        .query_map(params![group_id], |row| {
            Ok((
                row.get::<_, String>("member_type")?,
                row.get::<_, String>("member_value")?,
            ))
        })
        .map_err(|e| format!("query expand: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect expand: {e}"))?;

    drop(stmt);

    for (mtype, mvalue) in &members {
        match mtype.as_str() {
            "email" => {
                emails.insert(mvalue.to_lowercase());
            }
            "group" => {
                expand_recursive(conn, mvalue, visited, emails)?;
            }
            _ => {}
        }
    }

    Ok(())
}
