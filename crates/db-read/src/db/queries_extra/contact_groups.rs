use std::collections::HashSet;

use rusqlite::params;

use super::super::types::{DbContactGroup, DbContactGroupMember};
use super::super::{ReadConn, ReadDbState};
use super::contacts::ExpandedGroupContact;
use crate::db::from_row::FromRow;

pub async fn db_get_all_contact_groups(db: &ReadDbState) -> Result<Vec<DbContactGroup>, String> {
    db.with_read(move |conn| {
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

pub async fn db_get_contact_group(db: &ReadDbState, id: String) -> Result<DbContactGroup, String> {
    db.with_read(move |conn| {
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
    db: &ReadDbState,
    group_id: String,
) -> Result<Vec<DbContactGroupMember>, String> {
    db.with_read(move |conn| {
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

pub async fn db_search_contact_groups(
    db: &ReadDbState,
    query: String,
    limit: i64,
) -> Result<Vec<DbContactGroup>, String> {
    db.with_read(move |conn| {
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
    db: &ReadDbState,
    name: String,
) -> Result<Option<String>, String> {
    db.with_read(move |conn| {
        match conn.query_row(
            "SELECT id FROM contact_groups WHERE name = ?1 LIMIT 1",
            params![name],
            |row| row.get::<_, String>(0),
        ) {
            Ok(id) => Ok(Some(id)),
            Err(crate::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    })
    .await
}

pub async fn db_expand_contact_group(
    db: &ReadDbState,
    group_id: String,
) -> Result<Vec<String>, String> {
    db.with_read(move |conn| {
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
    db: &ReadDbState,
    group_id: String,
) -> Result<Vec<ExpandedGroupContact>, String> {
    db.with_read(move |conn| expand_group_with_names_sync(conn, &group_id))
        .await
}

#[derive(Debug, Clone)]
pub struct MatchedGroup {
    pub id: String,
    pub name: String,
    pub member_count: i64,
}

pub async fn db_find_group_matching_emails(
    db: &ReadDbState,
    emails: Vec<String>,
) -> Result<Option<MatchedGroup>, String> {
    let target: HashSet<String> = emails.into_iter().map(|e| e.to_lowercase()).collect();
    if target.is_empty() {
        return Ok(None);
    }
    db.with_read(move |conn| {
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

#[derive(Debug, Clone)]
pub struct GroupSettingsEntry {
    pub id: String,
    pub name: String,
    pub member_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

pub fn load_groups_for_settings_sync(
    conn: &ReadConn<'_>,
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

pub fn load_group_member_emails_sync(
    conn: &ReadConn<'_>,
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

pub fn expand_group_with_names_sync(
    conn: &ReadConn<'_>,
    group_id: &str,
) -> Result<Vec<ExpandedGroupContact>, String> {
    fn recurse(
        conn: &ReadConn<'_>,
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
                    let display_name = match conn.query_row(
                        "SELECT display_name FROM contacts
                         WHERE LOWER(email) = LOWER(?1) LIMIT 1",
                        params![member_value],
                        |row| row.get::<_, Option<String>>(0),
                    ) {
                        Ok(value) => value,
                        Err(crate::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => {
                            None
                        }
                        Err(e) => return Err(e.to_string()),
                    };
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

fn expand_recursive(
    conn: &ReadConn<'_>,
    group_id: &str,
    visited: &mut HashSet<String>,
    emails: &mut HashSet<String>,
) -> Result<(), String> {
    if !visited.insert(group_id.to_string()) {
        return Ok(());
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

    for (member_type, member_value) in members {
        if member_type == "group" {
            expand_recursive(conn, &member_value, visited, emails)?;
        } else {
            emails.insert(member_value.to_lowercase());
        }
    }
    Ok(())
}
