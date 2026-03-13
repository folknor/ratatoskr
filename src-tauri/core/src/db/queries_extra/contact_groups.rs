use std::collections::HashSet;

use rusqlite::params;

use super::super::DbState;
use super::super::types::{DbContactGroup, DbContactGroupMember};

pub async fn db_create_contact_group(
    db: &DbState,
    id: String,
    name: String,
) -> Result<(), String> {
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

pub async fn db_update_contact_group(
    db: &DbState,
    id: String,
    name: String,
) -> Result<(), String> {
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
}

pub async fn db_get_all_contact_groups(
    db: &DbState,
) -> Result<Vec<DbContactGroup>, String> {
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
        stmt.query_map([], row_to_contact_group)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_contact_group(
    db: &DbState,
    id: String,
) -> Result<DbContactGroup, String> {
    db.with_conn(move |conn| {
        conn.query_row(
            "SELECT g.id, g.name,
                    (SELECT COUNT(*) FROM contact_group_members WHERE group_id = g.id) AS member_count,
                    g.created_at, g.updated_at
             FROM contact_groups g
             WHERE g.id = ?1",
            params![id],
            row_to_contact_group,
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
        stmt.query_map(params![group_id], |row| {
            Ok(DbContactGroupMember {
                member_type: row.get(0)?,
                member_value: row.get(1)?,
            })
        })
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
        stmt.query_map(params![pattern, limit], row_to_contact_group)
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
        .prepare(
            "SELECT member_type, member_value FROM contact_group_members WHERE group_id = ?1",
        )
        .map_err(|e| format!("prepare expand: {e}"))?;

    let members: Vec<(String, String)> = stmt
        .query_map(params![group_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
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

fn row_to_contact_group(row: &rusqlite::Row<'_>) -> rusqlite::Result<DbContactGroup> {
    Ok(DbContactGroup {
        id: row.get(0)?,
        name: row.get(1)?,
        member_count: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}
