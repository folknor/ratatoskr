use std::collections::HashSet;

use rusqlite::{OptionalExtension, params};

use super::super::{ReadConn, WriterPool, WriteTarget, WriteTransactionTarget};
use super::super::types::{DbContactGroup, DbContactGroupMember};
use super::contacts::ExpandedGroupContact;
use crate::db::from_row::FromRow;

pub async fn db_create_contact_group(db: &WriterPool, id: String, name: String) -> Result<(), String> {
    log::debug!("Creating contact group: id={id}, name={name}");
    db.with_write(move |conn| {
        conn.execute(
            "INSERT INTO contact_groups (id, name) VALUES (?1, ?2)",
            params![id, name],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_update_contact_group(db: &WriterPool, id: String, name: String) -> Result<(), String> {
    log::debug!("Updating contact group: id={id}, name={name}");
    db.with_write(move |conn| {
        conn.execute(
            "UPDATE contact_groups SET name = ?1, updated_at = unixepoch() WHERE id = ?2",
            params![name, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_delete_contact_group(db: &WriterPool, id: String) -> Result<(), String> {
    log::debug!("Deleting contact group: id={id}");
    db.with_write(move |conn| {
        let tx = conn
            .transaction()
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

pub async fn db_get_all_contact_groups(db: &WriterPool) -> Result<Vec<DbContactGroup>, String> {
    db.with_write(move |conn| {
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

pub async fn db_get_contact_group(db: &WriterPool, id: String) -> Result<DbContactGroup, String> {
    db.with_write(move |conn| {
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
    db: &WriterPool,
    group_id: String,
) -> Result<Vec<DbContactGroupMember>, String> {
    db.with_write(move |conn| {
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
    db: &WriterPool,
    group_id: String,
    member_type: String,
    member_value: String,
) -> Result<(), String> {
    db.with_write(move |conn| {
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
    db: &WriterPool,
    group_id: String,
    member_type: String,
    member_value: String,
) -> Result<(), String> {
    db.with_write(move |conn| {
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
    db: &WriterPool,
    query: String,
    limit: i64,
) -> Result<Vec<DbContactGroup>, String> {
    log::debug!("Searching contact groups: query={query}, limit={limit}");
    db.with_write(move |conn| {
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

pub async fn db_find_contact_group_id_by_name(
    db: &WriterPool,
    name: String,
) -> Result<Option<String>, String> {
    db.with_write(move |conn| {
        conn.query_row(
            "SELECT id FROM contact_groups WHERE name = ?1 LIMIT 1",
            params![name],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    })
    .await
}

/// Return (local_id, server_id) pairs for all contact groups owned by an
/// account with a given source label.
pub fn list_contact_groups_for_account_by_source(
    conn: &ReadConn<'_>,
    account_id: &str,
    source: &str,
) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, server_id FROM contact_groups \
             WHERE account_id = ?1 AND source = ?2",
        )
        .map_err(|e| format!("list_contact_groups_for_account_by_source prepare: {e}"))?;

    stmt.query_map(params![account_id, source], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })
    .map_err(|e| format!("list_contact_groups_for_account_by_source query: {e}"))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| format!("list_contact_groups_for_account_by_source collect: {e}"))
}

pub async fn db_expand_contact_group(
    db: &WriterPool,
    group_id: String,
) -> Result<Vec<String>, String> {
    db.with_write(move |conn| {
        let mut visited = HashSet::new();
        let mut emails = HashSet::new();
        expand_recursive(conn, &group_id, &mut visited, &mut emails)?;
        let mut result: Vec<String> = emails.into_iter().collect();
        result.sort();
        Ok(result)
    })
    .await
}

pub async fn db_expand_contact_group_with_names(
    db: &WriterPool,
    group_id: String,
) -> Result<Vec<ExpandedGroupContact>, String> {
    db.with_write(move |conn| expand_group_with_names_sync(conn, &group_id)).await
}

/// A group matched against a set of pasted emails.
#[derive(Debug, Clone)]
pub struct MatchedGroup {
    pub id: String,
    pub name: String,
    pub member_count: i64,
}

/// Find a contact group whose recursively-expanded member email set is
/// exactly equal to `emails` (case-insensitive). Returns the first match
/// found; if multiple groups have the same member set the choice is
/// arbitrary but stable per scan order.
pub async fn db_find_group_matching_emails(
    db: &WriterPool,
    emails: Vec<String>,
) -> Result<Option<MatchedGroup>, String> {
    let target: HashSet<String> = emails.into_iter().map(|e| e.to_lowercase()).collect();
    if target.is_empty() {
        return Ok(None);
    }
    db.with_write(move |conn| {
        let mut stmt = conn
            .prepare("SELECT id, name FROM contact_groups")
            .map_err(|e| e.to_string())?;
        let groups: Vec<(String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);

        for (gid, gname) in groups {
            let mut visited = HashSet::new();
            let mut group_emails = HashSet::new();
            expand_recursive(conn, &gid, &mut visited, &mut group_emails)?;
            if group_emails == target {
                #[allow(clippy::cast_possible_wrap)]
                let count = group_emails.len() as i64;
                return Ok(Some(MatchedGroup {
                    id: gid,
                    name: gname,
                    member_count: count,
                }));
            }
        }
        Ok(None)
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
    conn: &crate::db::ReadConn<'_>,
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
    conn: &crate::db::ReadConn<'_>,
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
///
/// The whole sequence (UPSERT contact_groups + DELETE members +
/// INSERT N members) runs inside one `unchecked_transaction` so a
/// crash mid-write cannot leave a half-populated member list. Phase
/// 6a tightened this from the prior per-statement autocommit shape -
/// the Service is the new write boundary, and the IPC ack must imply
/// "all rows committed or none."
pub fn save_group_sync(
    conn: &impl WriteTransactionTarget,
    entry: &GroupSettingsEntry,
    member_emails: &[String],
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp();
    let tx = conn
        .transaction()
        .map_err(|e| format!("save_group begin tx: {e}"))?;
    tx.execute(
        "INSERT INTO contact_groups (id, name, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?3)
         ON CONFLICT(id) DO UPDATE SET
             name = excluded.name,
             updated_at = excluded.updated_at",
        params![entry.id, entry.name, now],
    )
    .map_err(|e| e.to_string())?;

    // Replace all members
    tx.execute(
        "DELETE FROM contact_group_members WHERE group_id = ?1",
        params![entry.id],
    )
    .map_err(|e| e.to_string())?;

    {
        let mut stmt = tx
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
    }
    tx.commit()
        .map_err(|e| format!("save_group commit: {e}"))?;
    Ok(())
}

/// Delete a group and clean up inbound refs (synchronous).
pub fn delete_group_sync(
    conn: &impl WriteTransactionTarget,
    group_id: &str,
) -> Result<(), String> {
    let tx = conn
        .transaction()
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

pub fn expand_group_with_names_sync(
    conn: &impl WriteTarget,
    group_id: &str,
) -> Result<Vec<ExpandedGroupContact>, String> {
    fn recurse(
        conn: &impl WriteTarget,
        gid: &str,
        visited: &mut HashSet<String>,
        result: &mut Vec<ExpandedGroupContact>,
        seen_emails: &mut HashSet<String>,
    ) -> Result<(), String> {
        if !visited.insert(gid.to_string()) {
            return Ok(());
        }

        let mut stmt = conn
            .prepare(
                "SELECT member_type, member_value
                 FROM contact_group_members
                 WHERE group_id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows: Vec<(String, String)> = stmt
            .query_map(params![gid], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        for (member_type, member_value) in rows {
            if member_type == "group" {
                recurse(conn, &member_value, visited, result, seen_emails)?;
            } else {
                let email_lower = member_value.to_lowercase();
                if seen_emails.insert(email_lower) {
                    let display_name: Option<String> = conn
                        .query_row(
                            "SELECT display_name FROM contacts
                             WHERE LOWER(email) = LOWER(?1) LIMIT 1",
                            params![member_value],
                            |row| row.get(0),
                        )
                        .ok()
                        .flatten();
                    result.push(ExpandedGroupContact {
                        email: member_value,
                        display_name,
                    });
                }
            }
        }

        Ok(())
    }

    let mut visited = HashSet::new();
    let mut result = Vec::new();
    let mut seen_emails = HashSet::new();
    recurse(conn, group_id, &mut visited, &mut result, &mut seen_emails)?;
    result.sort_by(|a, b| a.email.cmp(&b.email));
    Ok(result)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn expand_recursive(
    conn: &impl WriteTarget,
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

#[cfg(test)]
mod sync_group_tests {
    use super::*;
    use crate::db::migrations;
    use rusqlite::Connection;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("pragmas");
        migrations::run_all(&conn).expect("migrations");
        conn
    }

    fn write(conn: &Connection) -> crate::db::WriteConn<'_> {
        crate::db::WriteConn::from_raw(conn)
    }

    fn group_count(conn: &Connection, group_id: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM contact_groups WHERE id = ?1",
            params![group_id],
            |row| row.get(0),
        )
        .expect("group count")
    }

    fn member_emails(conn: &Connection, group_id: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT member_value FROM contact_group_members \
                 WHERE group_id = ?1 AND member_type = 'email' \
                 ORDER BY member_value",
            )
            .expect("prepare");
        let rows: Vec<String> = stmt
            .query_map([group_id], |row| row.get::<_, String>(0))
            .expect("query")
            .filter_map(Result::ok)
            .collect();
        rows
    }

    fn group_name(conn: &Connection, group_id: &str) -> String {
        conn.query_row(
            "SELECT name FROM contact_groups WHERE id = ?1",
            params![group_id],
            |row| row.get(0),
        )
        .expect("group name")
    }

    #[test]
    fn save_group_inserts_new_row_and_members() {
        let conn = setup_db();
        let entry = GroupSettingsEntry {
            id: "grp-1".into(),
            name: "Friends".into(),
            member_count: 2,
            created_at: 0,
            updated_at: 0,
        };
        save_group_sync(
            &write(&conn),
            &entry,
            &["alice@example.com".into(), "bob@example.com".into()],
        )
        .expect("save");
        assert_eq!(group_count(&conn, "grp-1"), 1);
        assert_eq!(
            member_emails(&conn, "grp-1"),
            vec!["alice@example.com", "bob@example.com"]
        );
    }

    #[test]
    fn save_group_updates_existing_name_and_replaces_members() {
        let conn = setup_db();
        let entry = GroupSettingsEntry {
            id: "grp-1".into(),
            name: "Friends".into(),
            member_count: 1,
            created_at: 0,
            updated_at: 0,
        };
        save_group_sync(
            &write(&conn), &entry, &["alice@example.com".into()]).expect("first save");

        let entry2 = GroupSettingsEntry {
            id: "grp-1".into(),
            name: "Best Friends".into(),
            member_count: 2,
            created_at: 0,
            updated_at: 0,
        };
        save_group_sync(
            &write(&conn),
            &entry2,
            &["bob@example.com".into(), "carol@example.com".into()],
        )
        .expect("second save");

        assert_eq!(group_name(&conn, "grp-1"), "Best Friends");
        // Old member (alice) is gone; new members are present.
        assert_eq!(
            member_emails(&conn, "grp-1"),
            vec!["bob@example.com", "carol@example.com"]
        );
    }

    #[test]
    fn save_group_with_empty_member_list_clears_existing_members() {
        let conn = setup_db();
        let entry = GroupSettingsEntry {
            id: "grp-1".into(),
            name: "Friends".into(),
            member_count: 1,
            created_at: 0,
            updated_at: 0,
        };
        save_group_sync(
            &write(&conn), &entry, &["alice@example.com".into()]).expect("seed");

        let entry2 = GroupSettingsEntry {
            id: "grp-1".into(),
            name: "Empty".into(),
            member_count: 0,
            created_at: 0,
            updated_at: 0,
        };
        save_group_sync(
            &write(&conn), &entry2, &[]).expect("clear");

        assert!(member_emails(&conn, "grp-1").is_empty());
        // Group row itself remains.
        assert_eq!(group_count(&conn, "grp-1"), 1);
    }

    #[test]
    fn delete_group_removes_row_and_members_and_inbound_refs() {
        let conn = setup_db();
        // Inner group with members.
        let inner = GroupSettingsEntry {
            id: "grp-inner".into(),
            name: "Inner".into(),
            member_count: 1,
            created_at: 0,
            updated_at: 0,
        };
        save_group_sync(
            &write(&conn), &inner, &["alice@example.com".into()]).expect("inner");
        // Outer group that references the inner one as a member.
        let outer = GroupSettingsEntry {
            id: "grp-outer".into(),
            name: "Outer".into(),
            member_count: 1,
            created_at: 0,
            updated_at: 0,
        };
        save_group_sync(
            &write(&conn), &outer, &[]).expect("outer");
        conn.execute(
            "INSERT INTO contact_group_members \
             (group_id, member_type, member_value) \
             VALUES (?1, 'group', ?2)",
            params!["grp-outer", "grp-inner"],
        )
        .expect("seed nested ref");

        delete_group_sync(&write(&conn), "grp-inner").expect("delete inner");

        assert_eq!(group_count(&conn, "grp-inner"), 0, "group row gone");
        let inbound: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM contact_group_members \
                 WHERE member_type = 'group' AND member_value = ?1",
                params!["grp-inner"],
                |row| row.get(0),
            )
            .expect("inbound count");
        assert_eq!(inbound, 0, "outer's nested ref to inner is cleaned up");
    }

    #[test]
    fn delete_group_is_idempotent() {
        let conn = setup_db();
        // No group with this id exists; delete must succeed.
        delete_group_sync(&write(&conn), "grp-missing").expect("delete missing");
        // Second call also succeeds.
        delete_group_sync(&write(&conn), "grp-missing").expect("delete again");
    }
}
