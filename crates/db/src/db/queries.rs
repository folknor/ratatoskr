use rusqlite::{Connection, params};

use super::from_row::{query_as, query_one, FromRow};
use super::sql_fragments::{
    LATEST_MESSAGE_SUBQUERY, SEEN_ADDRESS_SCORE_EXPR, validate_thread_bool_column,
};
use super::types::{
    CategoryCount, DbAttachment, DbContact, DbLabel, DbThread, ThreadCategoryRow, ThreadInfoRow,
};
use super::DbState;

/// Read a single value from the `settings` table, returning `Ok(None)` when
/// the key does not exist.
pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>, String> {
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

/// Get all labels for an account, ordered by sort_order then name.
pub fn get_labels(conn: &Connection, account_id: &str) -> Result<Vec<DbLabel>, String> {
    query_as::<DbLabel>(
        conn,
        "SELECT * FROM labels WHERE account_id = ?1 ORDER BY sort_order ASC, name ASC",
        &[&account_id],
    )
}

// ---------------------------------------------------------------------------
// Thread queries
// ---------------------------------------------------------------------------

#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn get_threads(
    conn: &Connection,
    account_id: &str,
    label_id: Option<&str>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DbThread>, String> {
    let lim = limit.unwrap_or(50);
    let off = offset.unwrap_or(0);

    if let Some(lid) = label_id {
        let sql = format!(
            "SELECT t.*, m.from_name, m.from_address FROM threads t
             INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
             LEFT JOIN ({LATEST_MESSAGE_SUBQUERY}
             ) m ON m.account_id = t.account_id AND m.thread_id = t.id
             WHERE t.account_id = ?1 AND tl.label_id = ?2
               AND t.is_chat_thread = 0
             GROUP BY t.account_id, t.id
             ORDER BY t.is_pinned DESC, t.last_message_at DESC
             LIMIT ?3 OFFSET ?4"
        );
        query_as::<DbThread>(conn, &sql, &[&account_id, &lid, &lim, &off])
    } else {
        let sql = format!(
            "SELECT t.*, m.from_name, m.from_address FROM threads t
             LEFT JOIN ({LATEST_MESSAGE_SUBQUERY}
             ) m ON m.account_id = t.account_id AND m.thread_id = t.id
             WHERE t.account_id = ?1 AND t.is_chat_thread = 0
             ORDER BY t.is_pinned DESC, t.last_message_at DESC
             LIMIT ?2 OFFSET ?3"
        );
        query_as::<DbThread>(conn, &sql, &[&account_id, &lim, &off])
    }
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn get_threads_for_bundle(
    conn: &Connection,
    account_id: &str,
    category: &str,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DbThread>, String> {
    let lim = limit.unwrap_or(50);
    let off = offset.unwrap_or(0);

    if category == "Primary" {
        let sql = format!(
            "SELECT t.*, m.from_name, m.from_address FROM threads t
             INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
             LEFT JOIN thread_bundles tc ON tc.account_id = t.account_id AND tc.thread_id = t.id
             LEFT JOIN ({LATEST_MESSAGE_SUBQUERY}
             ) m ON m.account_id = t.account_id AND m.thread_id = t.id
             WHERE t.account_id = ?1 AND tl.label_id = 'INBOX'
               AND (tc.bundle IS NULL OR tc.bundle = 'Primary')
               AND t.is_chat_thread = 0
             GROUP BY t.account_id, t.id
             ORDER BY t.is_pinned DESC, t.last_message_at DESC
             LIMIT ?2 OFFSET ?3"
        );
        query_as::<DbThread>(conn, &sql, &[&account_id, &lim, &off])
    } else {
        let sql = format!(
            "SELECT t.*, m.from_name, m.from_address FROM threads t
             INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
             INNER JOIN thread_bundles tc ON tc.account_id = t.account_id AND tc.thread_id = t.id
             LEFT JOIN ({LATEST_MESSAGE_SUBQUERY}
             ) m ON m.account_id = t.account_id AND m.thread_id = t.id
             WHERE t.account_id = ?1 AND tl.label_id = 'INBOX' AND tc.bundle = ?2
               AND t.is_chat_thread = 0
             GROUP BY t.account_id, t.id
             ORDER BY t.is_pinned DESC, t.last_message_at DESC
             LIMIT ?3 OFFSET ?4"
        );
        query_as::<DbThread>(conn, &sql, &[&account_id, &category, &lim, &off])
    }
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn get_thread_by_id(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
) -> Result<Option<DbThread>, String> {
    let sql = format!(
        "SELECT t.*, m.from_name, m.from_address FROM threads t
         LEFT JOIN ({LATEST_MESSAGE_SUBQUERY}
         ) m ON m.account_id = t.account_id AND m.thread_id = t.id
         WHERE t.account_id = ?1 AND t.id = ?2
         LIMIT 1"
    );
    query_one::<DbThread>(conn, &sql, &[&account_id, &thread_id])
}

pub fn get_thread_label_ids(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT label_id FROM thread_labels WHERE account_id = ?1 AND thread_id = ?2")
        .map_err(|e| e.to_string())?;

    stmt.query_map(params![account_id, thread_id], |row| {
        row.get::<_, String>("label_id")
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Thread mutation
// ---------------------------------------------------------------------------

fn set_thread_bool_field(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
    column: &str,
    value: bool,
) -> Result<usize, String> {
    let column = validate_thread_bool_column(column)?;
    let sql = format!(
        "UPDATE threads SET {column} = ?3 WHERE account_id = ?1 AND id = ?2 AND {column} != ?3"
    );
    conn.execute(&sql, params![account_id, thread_id, value])
        .map_err(|e| e.to_string())
}

pub fn set_thread_read(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
    is_read: bool,
) -> Result<usize, String> {
    set_thread_bool_field(conn, account_id, thread_id, "is_read", is_read)
}

pub fn set_thread_starred(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
    is_starred: bool,
) -> Result<usize, String> {
    set_thread_bool_field(conn, account_id, thread_id, "is_starred", is_starred)
}

pub fn set_thread_pinned(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
    is_pinned: bool,
) -> Result<usize, String> {
    set_thread_bool_field(conn, account_id, thread_id, "is_pinned", is_pinned)
}

pub fn set_thread_muted(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
    is_muted: bool,
) -> Result<usize, String> {
    set_thread_bool_field(conn, account_id, thread_id, "is_muted", is_muted)
}

pub fn delete_thread(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
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
    account_id: &str,
    thread_id: &str,
    label_id: &str,
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
    account_id: &str,
    thread_id: &str,
    label_id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM thread_labels WHERE account_id = ?1 AND thread_id = ?2 AND label_id = ?3",
        params![account_id, thread_id, label_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Label queries
// ---------------------------------------------------------------------------

pub fn upsert_label(conn: &Connection, label: &DbLabel) -> Result<(), String> {
    conn.execute(
        "INSERT INTO labels (account_id, id, name, type, color_bg, color_fg, visible, sort_order, imap_folder_path, imap_special_use)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(account_id, id) DO UPDATE SET
           name = excluded.name,
           type = excluded.type,
           color_bg = excluded.color_bg,
           color_fg = excluded.color_fg,
           visible = excluded.visible,
           sort_order = excluded.sort_order,
           imap_folder_path = excluded.imap_folder_path,
           imap_special_use = excluded.imap_special_use",
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

pub fn delete_label(conn: &Connection, account_id: &str, label_id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM labels WHERE account_id = ?1 AND id = ?2",
        params![account_id, label_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
        params![key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Bundle/category queries
// ---------------------------------------------------------------------------

#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn get_bundle_unread_counts(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<CategoryCount>, String> {
    query_as::<CategoryCount>(
        conn,
        "SELECT tc.bundle, COUNT(*) as count
         FROM threads t
         INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
         LEFT JOIN thread_bundles tc ON tc.account_id = t.account_id AND tc.thread_id = t.id
         WHERE t.account_id = ?1 AND tl.label_id = 'INBOX' AND t.is_read = 0
         GROUP BY tc.bundle",
        &[&account_id],
    )
}

pub fn get_categories_for_threads(
    conn: &Connection,
    account_id: &str,
    thread_ids: &[String],
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
            "SELECT thread_id, bundle FROM thread_bundles WHERE account_id = ?1 AND thread_id IN ({placeholders})"
        );

        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        param_values.push(Box::new(account_id.to_owned()));
        for thread_id in chunk {
            param_values.push(Box::new(thread_id.clone()));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(AsRef::as_ref).collect();

        let rows = stmt
            .query_map(param_refs.as_slice(), ThreadCategoryRow::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        all_results.extend(rows);
    }

    Ok(all_results)
}

// ---------------------------------------------------------------------------
// Contact/attachment lookups
// ---------------------------------------------------------------------------

pub fn get_attachments_for_message(
    conn: &Connection,
    account_id: &str,
    message_id: &str,
) -> Result<Vec<DbAttachment>, String> {
    query_as::<DbAttachment>(
        conn,
        "SELECT * FROM attachments WHERE account_id = ?1 AND message_id = ?2 ORDER BY filename ASC",
        &[&account_id, &message_id],
    )
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn search_contacts(
    conn: &Connection,
    query: &str,
    limit: i64,
) -> Result<Vec<DbContact>, String> {
    match search_contacts_fts(conn, query, limit) {
        Ok(results) => Ok(results),
        Err(_) => search_contacts_like(conn, query, limit),
    }
}

fn search_contacts_fts(
    conn: &Connection,
    query: &str,
    limit: i64,
) -> Result<Vec<DbContact>, String> {
    let fts_query = super::sql_fragments::build_fts_query(query);
    if fts_query.is_empty() {
        return Ok(Vec::new());
    }
    let like_pattern = if query.len() <= 2 {
        format!("{query}%")
    } else {
        format!("%{query}%")
    };

    let sql = format!(
        "SELECT c.id, c.email, c.display_name, c.avatar_url, c.frequency,
                c.last_contacted_at, c.notes, 1 AS source_rank
         FROM contacts c
         INNER JOIN contacts_fts ON contacts_fts.rowid = c.rowid
         WHERE contacts_fts MATCH ?1

         UNION ALL

         SELECT '' AS id, sa.email, sa.display_name, NULL AS avatar_url,
           {SEEN_ADDRESS_SCORE_EXPR} AS frequency,
           NULL AS last_contacted_at, NULL AS notes, 2 AS source_rank
         FROM seen_addresses sa
         WHERE (sa.email LIKE ?2 OR sa.display_name LIKE ?2)
           AND sa.email NOT IN (
             SELECT c2.email FROM contacts c2
             INNER JOIN contacts_fts fts2 ON fts2.rowid = c2.rowid
             WHERE contacts_fts MATCH ?1
           )

         ORDER BY source_rank ASC, frequency DESC, display_name ASC
         LIMIT ?3"
    );
    query_as::<DbContact>(conn, &sql, &[&fts_query, &like_pattern, &limit])
}

fn search_contacts_like(
    conn: &Connection,
    query: &str,
    limit: i64,
) -> Result<Vec<DbContact>, String> {
    let pattern = if query.len() <= 2 {
        format!("{query}%")
    } else {
        format!("%{query}%")
    };

    let sql = format!(
        "SELECT id, email, display_name, avatar_url, frequency,
                last_contacted_at, notes, 1 AS source_rank
         FROM contacts
         WHERE email LIKE ?1 OR display_name LIKE ?1

         UNION ALL

         SELECT '' AS id, sa.email, sa.display_name, NULL AS avatar_url,
           {SEEN_ADDRESS_SCORE_EXPR} AS frequency,
           NULL AS last_contacted_at, NULL AS notes, 2 AS source_rank
         FROM seen_addresses sa
         WHERE (sa.email LIKE ?1 OR sa.display_name LIKE ?1)
           AND sa.email NOT IN (
             SELECT email FROM contacts
             WHERE email LIKE ?1 OR display_name LIKE ?1
           )

         ORDER BY source_rank ASC, frequency DESC, display_name ASC
         LIMIT ?2"
    );
    query_as::<DbContact>(conn, &sql, &[&pattern, &limit])
}

pub fn get_contact_by_email(conn: &Connection, email: &str) -> Result<Option<DbContact>, String> {
    let normalized = email.to_lowercase();
    query_one::<DbContact>(
        conn,
        "SELECT * FROM contacts WHERE email = ?1 LIMIT 1",
        &[&normalized],
    )
}

// ---------------------------------------------------------------------------
// Thread count / unread
// ---------------------------------------------------------------------------

pub fn get_thread_count(
    conn: &Connection,
    account_id: &str,
    label_id: Option<&str>,
) -> Result<i64, String> {
    if let Some(label_id) = label_id {
        conn.query_row(
            "SELECT COUNT(DISTINCT t.id) AS cnt FROM threads t
             INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
             WHERE t.account_id = ?1 AND tl.label_id = ?2
               AND t.is_chat_thread = 0",
            params![account_id, label_id],
            |row| row.get::<_, i64>("cnt"),
        )
        .map_err(|e| e.to_string())
    } else {
        conn.query_row(
            "SELECT COUNT(*) AS cnt FROM threads WHERE account_id = ?1
               AND is_chat_thread = 0",
            params![account_id],
            |row| row.get::<_, i64>("cnt"),
        )
        .map_err(|e| e.to_string())
    }
}

pub async fn get_provider_type(db: &DbState, account_id: &str) -> Result<String, String> {
    let aid = account_id.to_string();
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare("SELECT provider FROM accounts WHERE id = ?1")
            .map_err(|e| format!("prepare: {e}"))?;
        stmt.query_row([&aid], |row| row.get::<_, String>(0))
            .map_err(|e| format!("No account found for {aid}: {e}"))
    })
    .await
}

pub fn get_unread_count(conn: &Connection, account_id: &str) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) AS cnt FROM threads t
         INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
         WHERE t.account_id = ?1 AND tl.label_id = 'INBOX' AND t.is_read = 0
           AND t.is_chat_thread = 0",
        params![account_id],
        |row| row.get::<_, i64>("cnt"),
    )
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Bundle/rule queries
// ---------------------------------------------------------------------------

pub fn load_recent_rule_bundled_threads(
    conn: &Connection,
    account_id: &str,
    limit: i64,
) -> Result<Vec<ThreadInfoRow>, String> {
    let sql = format!(
        "SELECT t.id, t.subject, t.snippet, m.from_address
         FROM threads t
         INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
         INNER JOIN thread_bundles tc ON tc.account_id = t.account_id AND tc.thread_id = t.id
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
