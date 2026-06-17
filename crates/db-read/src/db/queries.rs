use crate::{ReadConn, ReadDbState};

use super::from_row::{FromRow, query_as, query_one};
use super::sql_fragments::{LATEST_MESSAGE_SUBQUERY, SEEN_ADDRESS_SCORE_EXPR};
use super::types::{
    CategoryCount, DbAttachment, DbContact, DbFolder, DbLabel, DbThread, ThreadCategoryRow,
    ThreadInfoRow,
};

pub fn get_setting(conn: &ReadConn<'_>, key: &str) -> Result<Option<String>, String> {
    match conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        rusqlite::params![key],
        |row| row.get::<_, String>("value"),
    ) {
        Ok(value) => Ok(Some(value)),
        Err(crate::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

pub fn get_folders(conn: &ReadConn<'_>, account_id: &str) -> Result<Vec<DbFolder>, String> {
    query_as::<DbFolder>(
        conn,
        "SELECT * FROM folders
         WHERE account_id = ?1
         ORDER BY sort_order ASC, name ASC",
        &[&account_id],
    )
}

pub fn get_labels(conn: &ReadConn<'_>, account_id: &str) -> Result<Vec<DbLabel>, String> {
    query_as::<DbLabel>(
        conn,
        "SELECT * FROM labels
         WHERE account_id = ?1
         ORDER BY sort_order ASC, name ASC",
        &[&account_id],
    )
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn get_threads(
    conn: &ReadConn<'_>,
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
             INNER JOIN thread_folders tf ON tf.account_id = t.account_id AND tf.thread_id = t.id
             LEFT JOIN ({LATEST_MESSAGE_SUBQUERY}
             ) m ON m.account_id = t.account_id AND m.thread_id = t.id
             WHERE t.account_id = ?1 AND tf.folder_id = ?2
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
    conn: &ReadConn<'_>,
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
             INNER JOIN thread_folders tf ON tf.account_id = t.account_id AND tf.thread_id = t.id
             LEFT JOIN thread_bundles tc ON tc.account_id = t.account_id AND tc.thread_id = t.id
             LEFT JOIN ({LATEST_MESSAGE_SUBQUERY}
             ) m ON m.account_id = t.account_id AND m.thread_id = t.id
             WHERE t.account_id = ?1 AND tf.folder_id = 'INBOX'
               AND (tc.bundle IS NULL OR tc.bundle = 'Primary')
               AND t.is_chat_thread = 0
             GROUP BY t.account_id, t.id
             ORDER BY t.is_pinned DESC, t.last_message_at DESC
             LIMIT ?2 OFFSET ?3"
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        stmt.query_map(rusqlite::params![account_id, lim, off], DbThread::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    } else {
        let sql = format!(
            "SELECT t.*, m.from_name, m.from_address FROM threads t
             INNER JOIN thread_folders tf ON tf.account_id = t.account_id AND tf.thread_id = t.id
             INNER JOIN thread_bundles tc ON tc.account_id = t.account_id AND tc.thread_id = t.id
             LEFT JOIN ({LATEST_MESSAGE_SUBQUERY}
             ) m ON m.account_id = t.account_id AND m.thread_id = t.id
             WHERE t.account_id = ?1 AND tf.folder_id = 'INBOX' AND tc.bundle = ?2
               AND t.is_chat_thread = 0
             GROUP BY t.account_id, t.id
             ORDER BY t.is_pinned DESC, t.last_message_at DESC
             LIMIT ?3 OFFSET ?4"
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        stmt.query_map(
            rusqlite::params![account_id, category, lim, off],
            DbThread::from_row,
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
    }
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn get_thread_by_id(
    conn: &ReadConn<'_>,
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
    conn: &ReadConn<'_>,
    account_id: &str,
    thread_id: &str,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT label_id FROM thread_labels WHERE account_id = ?1 AND thread_id = ?2")
        .map_err(|e| e.to_string())?;

    stmt.query_map(rusqlite::params![account_id, thread_id], |row| {
        row.get::<_, String>("label_id")
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

pub fn get_thread_folder_ids(
    conn: &ReadConn<'_>,
    account_id: &str,
    thread_id: &str,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT folder_id FROM thread_folders WHERE account_id = ?1 AND thread_id = ?2")
        .map_err(|e| e.to_string())?;

    stmt.query_map(rusqlite::params![account_id, thread_id], |row| {
        row.get::<_, String>("folder_id")
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn get_bundle_unread_counts(
    conn: &ReadConn<'_>,
    account_id: &str,
) -> Result<Vec<CategoryCount>, String> {
    query_as::<CategoryCount>(
        conn,
        "SELECT tc.bundle, COUNT(*) as count
         FROM threads t
         INNER JOIN thread_folders tf ON tf.account_id = t.account_id AND tf.thread_id = t.id
         LEFT JOIN thread_bundles tc ON tc.account_id = t.account_id AND tc.thread_id = t.id
         WHERE t.account_id = ?1 AND tf.folder_id = 'INBOX' AND t.is_read = 0
         GROUP BY tc.bundle",
        &[&account_id],
    )
}

pub fn get_categories_for_threads(
    conn: &ReadConn<'_>,
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

pub fn get_attachments_for_message(
    conn: &ReadConn<'_>,
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
    conn: &ReadConn<'_>,
    query: &str,
    limit: i64,
) -> Result<Vec<DbContact>, String> {
    match search_contacts_fts(conn, query, limit) {
        Ok(results) => Ok(results),
        Err(_) => search_contacts_like(conn, query, limit),
    }
}

fn search_contacts_fts(
    conn: &ReadConn<'_>,
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
                c.last_contacted_at, c.notes, c.email2, c.phone, c.company,
                c.account_id, c.server_id, c.source
         FROM contacts c
         INNER JOIN contacts_fts ON contacts_fts.rowid = c.rowid
         WHERE contacts_fts MATCH ?1

         UNION ALL

         SELECT '' AS id, sa.email, sa.display_name, NULL AS avatar_url,
           {SEEN_ADDRESS_SCORE_EXPR} AS frequency,
           NULL AS last_contacted_at, NULL AS notes, NULL AS email2, NULL AS phone,
           NULL AS company, NULL AS account_id, NULL AS server_id, 'seen' AS source
         FROM seen_addresses sa
         WHERE (sa.email LIKE ?2 OR sa.display_name LIKE ?2)
           AND sa.email NOT IN (
             SELECT c2.email FROM contacts c2
             INNER JOIN contacts_fts fts2 ON fts2.rowid = c2.rowid
             WHERE contacts_fts MATCH ?1
           )

         ORDER BY frequency DESC, display_name ASC
         LIMIT ?3"
    );
    query_as::<DbContact>(conn, &sql, &[&fts_query, &like_pattern, &limit])
}

fn search_contacts_like(
    conn: &ReadConn<'_>,
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
                last_contacted_at, notes, email2, phone, company,
                account_id, server_id, source
         FROM contacts
         WHERE email LIKE ?1 OR display_name LIKE ?1

         UNION ALL

         SELECT '' AS id, sa.email, sa.display_name, NULL AS avatar_url,
           {SEEN_ADDRESS_SCORE_EXPR} AS frequency,
           NULL AS last_contacted_at, NULL AS notes, NULL AS email2, NULL AS phone,
           NULL AS company, NULL AS account_id, NULL AS server_id, 'seen' AS source
         FROM seen_addresses sa
         WHERE (sa.email LIKE ?1 OR sa.display_name LIKE ?1)
           AND sa.email NOT IN (
             SELECT email FROM contacts
             WHERE email LIKE ?1 OR display_name LIKE ?1
           )

         ORDER BY frequency DESC, display_name ASC
         LIMIT ?2"
    );
    query_as::<DbContact>(conn, &sql, &[&pattern, &limit])
}

pub fn get_contact_by_email(conn: &ReadConn<'_>, email: &str) -> Result<Option<DbContact>, String> {
    let normalized = email.to_lowercase();
    query_one::<DbContact>(
        conn,
        "SELECT * FROM contacts WHERE email = ?1 LIMIT 1",
        &[&normalized],
    )
}

pub fn get_thread_count(
    conn: &ReadConn<'_>,
    account_id: &str,
    label_id: Option<&str>,
) -> Result<i64, String> {
    if let Some(label_id) = label_id {
        conn.query_row(
            "SELECT COUNT(DISTINCT t.id) AS cnt FROM threads t
             INNER JOIN thread_folders tf ON tf.account_id = t.account_id AND tf.thread_id = t.id
             WHERE t.account_id = ?1 AND tf.folder_id = ?2
               AND t.is_chat_thread = 0",
            rusqlite::params![account_id, label_id],
            |row| row.get::<_, i64>("cnt"),
        )
        .map_err(|e| e.to_string())
    } else {
        conn.query_row(
            "SELECT COUNT(*) AS cnt FROM threads WHERE account_id = ?1
               AND is_chat_thread = 0",
            rusqlite::params![account_id],
            |row| row.get::<_, i64>("cnt"),
        )
        .map_err(|e| e.to_string())
    }
}

pub async fn get_provider_type(db: &ReadDbState, account_id: &str) -> Result<String, String> {
    let aid = account_id.to_string();
    db.with_read(move |conn| {
        let mut stmt = conn
            .prepare("SELECT provider FROM accounts WHERE id = ?1")
            .map_err(|e| format!("prepare: {e}"))?;
        stmt.query_row([&aid], |row| row.get::<_, String>(0))
            .map_err(|e| format!("No account found for {aid}: {e}"))
    })
    .await
}

pub fn get_unread_count(conn: &ReadConn<'_>, account_id: &str) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) AS cnt FROM threads t
         INNER JOIN thread_folders tf ON tf.account_id = t.account_id AND tf.thread_id = t.id
         WHERE t.account_id = ?1 AND tf.folder_id = 'INBOX' AND t.is_read = 0
           AND t.is_chat_thread = 0",
        rusqlite::params![account_id],
        |row| row.get::<_, i64>("cnt"),
    )
    .map_err(|e| e.to_string())
}

pub fn load_recent_rule_bundled_threads(
    conn: &ReadConn<'_>,
    account_id: &str,
    limit: i64,
) -> Result<Vec<ThreadInfoRow>, String> {
    let sql = format!(
        "SELECT t.id, t.subject, t.snippet, m.from_address
         FROM threads t
         INNER JOIN thread_folders tf ON tf.account_id = t.account_id AND tf.thread_id = t.id
         INNER JOIN thread_bundles tc ON tc.account_id = t.account_id AND tc.thread_id = t.id
         LEFT JOIN ({LATEST_MESSAGE_SUBQUERY}
         ) m ON m.account_id = t.account_id AND m.thread_id = t.id
         WHERE t.account_id = ?1 AND tf.folder_id = 'INBOX' AND tc.is_manual = 0
         ORDER BY t.last_message_at DESC
         LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    stmt.query_map(
        rusqlite::params![account_id, limit],
        ThreadInfoRow::from_row,
    )
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}
