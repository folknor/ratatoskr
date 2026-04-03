use rusqlite::Connection;

use crate::db::FromRow;
use crate::db::sql_fragments::LATEST_MESSAGE_SUBQUERY;
use crate::db::types::{AccountScope, DbThread, FolderAccountUnreadCount, FolderUnreadCount};

/// Build the WHERE clause fragment and collect parameter values for an `AccountScope`.
///
/// Returns `(sql_fragment, params)` where `sql_fragment` is either:
/// - `"t.account_id = ?N"` for `Single`
/// - `"t.account_id IN (?N, ?N+1, ...)"` for `Multiple`
/// - `"1=1"` for `All`
///
/// `base_idx` is the starting `?N` placeholder index (1-based).
fn account_scope_clause(
    scope: &AccountScope,
    base_idx: usize,
) -> (String, Vec<Box<dyn rusqlite::types::ToSql>>) {
    match scope {
        AccountScope::Single(id) => {
            let clause = format!("t.account_id = ?{base_idx}");
            let params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(id.clone())];
            (clause, params)
        }
        AccountScope::Multiple(ids) => {
            if ids.is_empty() {
                return ("0=1".to_owned(), Vec::new());
            }
            let placeholders: Vec<String> = ids
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", base_idx + i))
                .collect();
            let clause = format!("t.account_id IN ({})", placeholders.join(", "));
            let params: Vec<Box<dyn rusqlite::types::ToSql>> =
                ids.iter().map(|id| Box::new(id.clone()) as _).collect();
            (clause, params)
        }
        AccountScope::All => ("1=1".to_owned(), Vec::new()),
    }
}

/// Like `get_threads` but accepts an `AccountScope` to query across accounts.
#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn get_threads_scoped(
    conn: &Connection,
    scope: &AccountScope,
    label_id: Option<&str>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DbThread>, String> {
    log::debug!(
        "Scoped query: scope={scope:?}, label={label_id:?}, limit={limit:?}, offset={offset:?}"
    );
    let lim = limit.unwrap_or(50);
    let off = offset.unwrap_or(0);

    let (scope_clause, scope_params) = account_scope_clause(scope, 1);
    let next_idx = scope_params.len() + 1;

    let result = if let Some(lid) = label_id {
        let sql = format!(
            "SELECT t.*, m.from_name, m.from_address FROM threads t
             INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
             LEFT JOIN ({LATEST_MESSAGE_SUBQUERY}
             ) m ON m.account_id = t.account_id AND m.thread_id = t.id
             WHERE {scope_clause} AND tl.label_id = ?{next_idx}
               AND t.shared_mailbox_id IS NULL AND t.is_chat_thread = 0
             GROUP BY t.account_id, t.id
             ORDER BY t.is_pinned DESC, t.last_message_at DESC
             LIMIT ?{limit_idx} OFFSET ?{offset_idx}",
            limit_idx = next_idx + 1,
            offset_idx = next_idx + 2,
        );
        execute_thread_query_with_label(conn, &sql, scope_params, lid, lim, off)
    } else {
        let sql = format!(
            "SELECT t.*, m.from_name, m.from_address FROM threads t
             LEFT JOIN ({LATEST_MESSAGE_SUBQUERY}
             ) m ON m.account_id = t.account_id AND m.thread_id = t.id
             WHERE {scope_clause} AND t.shared_mailbox_id IS NULL AND t.is_chat_thread = 0
             ORDER BY t.is_pinned DESC, t.last_message_at DESC
             LIMIT ?{limit_idx} OFFSET ?{offset_idx}",
            limit_idx = next_idx,
            offset_idx = next_idx + 1,
        );
        execute_thread_query_no_label(conn, &sql, scope_params, lim, off)
    };
    if let Err(ref e) = result {
        log::error!("Scoped query failed: scope={scope:?}, label={label_id:?}, error={e}");
    }
    result
}

fn execute_thread_query_with_label(
    conn: &Connection,
    sql: &str,
    scope_params: Vec<Box<dyn rusqlite::types::ToSql>>,
    label_id: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<DbThread>, String> {
    let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = scope_params;
    all_params.push(Box::new(label_id.to_owned()));
    all_params.push(Box::new(limit));
    all_params.push(Box::new(offset));

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        all_params.iter().map(AsRef::as_ref).collect();

    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    stmt.query_map(param_refs.as_slice(), DbThread::from_row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn execute_thread_query_no_label(
    conn: &Connection,
    sql: &str,
    scope_params: Vec<Box<dyn rusqlite::types::ToSql>>,
    limit: i64,
    offset: i64,
) -> Result<Vec<DbThread>, String> {
    let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = scope_params;
    all_params.push(Box::new(limit));
    all_params.push(Box::new(offset));

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        all_params.iter().map(AsRef::as_ref).collect();

    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    stmt.query_map(param_refs.as_slice(), DbThread::from_row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Like `get_thread_count` but accepts an `AccountScope`.
#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn get_thread_count_scoped(
    conn: &Connection,
    scope: &AccountScope,
    label_id: Option<&str>,
) -> Result<i64, String> {
    let (scope_clause, scope_params) = account_scope_clause(scope, 1);
    let next_idx = scope_params.len() + 1;

    if let Some(lid) = label_id {
        let sql = format!(
            "SELECT COUNT(DISTINCT t.account_id || '/' || t.id) AS cnt FROM threads t
             INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
             WHERE {scope_clause} AND tl.label_id = ?{next_idx}
               AND t.shared_mailbox_id IS NULL AND t.is_chat_thread = 0"
        );
        execute_count_query(conn, &sql, scope_params, Some(lid))
    } else {
        let sql = format!(
            "SELECT COUNT(*) AS cnt FROM threads t WHERE {scope_clause}
               AND t.shared_mailbox_id IS NULL AND t.is_chat_thread = 0"
        );
        execute_count_query(conn, &sql, scope_params, None)
    }
}

/// Like `get_unread_count` but accepts an `AccountScope`.
#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn get_unread_count_scoped(conn: &Connection, scope: &AccountScope) -> Result<i64, String> {
    let (scope_clause, scope_params) = account_scope_clause(scope, 1);

    let sql = format!(
        "SELECT COUNT(*) AS cnt FROM threads t
         INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
         WHERE {scope_clause} AND tl.label_id = 'INBOX' AND t.is_read = 0
           AND t.shared_mailbox_id IS NULL AND t.is_chat_thread = 0"
    );

    execute_count_query(conn, &sql, scope_params, None)
}

fn execute_count_query(
    conn: &Connection,
    sql: &str,
    scope_params: Vec<Box<dyn rusqlite::types::ToSql>>,
    extra_param: Option<&str>,
) -> Result<i64, String> {
    let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = scope_params;
    if let Some(val) = extra_param {
        all_params.push(Box::new(val.to_owned()));
    }

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        all_params.iter().map(AsRef::as_ref).collect();

    conn.query_row(sql, param_refs.as_slice(), |row| row.get::<_, i64>("cnt"))
        .map_err(|e| e.to_string())
}

/// Label-based folder IDs whose unread count comes from `thread_labels`.
const LABEL_FOLDER_IDS: &[&str] = &["INBOX", "SENT", "DRAFT", "TRASH", "SPAM"];

/// Return unread counts for each universal folder, aggregated across `scope`.
///
/// Starred uses `threads.is_starred`, Snoozed uses `threads.is_snoozed`,
/// and all other folders use the `thread_labels` join.
#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn get_unread_counts_by_folder(
    conn: &Connection,
    scope: &AccountScope,
) -> Result<Vec<FolderUnreadCount>, String> {
    let mut results = get_label_folder_unread_counts(conn, scope)?;
    results.push(get_flag_folder_unread_count(conn, scope, "STARRED")?);
    results.push(get_flag_folder_unread_count(conn, scope, "SNOOZED")?);
    Ok(results)
}

/// Return unread counts for each universal folder, grouped by account.
#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn get_unread_counts_by_folder_and_account(
    conn: &Connection,
    scope: &AccountScope,
) -> Result<Vec<FolderAccountUnreadCount>, String> {
    let mut results = get_label_folder_unread_counts_by_account(conn, scope)?;
    let starred = get_flag_folder_unread_by_account(conn, scope, "STARRED")?;
    let snoozed = get_flag_folder_unread_by_account(conn, scope, "SNOOZED")?;
    results.extend(starred);
    results.extend(snoozed);
    Ok(results)
}

/// Unread counts for label-based folders (INBOX, SENT, DRAFT, TRASH, SPAM).
fn get_label_folder_unread_counts(
    conn: &Connection,
    scope: &AccountScope,
) -> Result<Vec<FolderUnreadCount>, String> {
    let (scope_clause, scope_params) = account_scope_clause(scope, 1);
    let placeholders: Vec<String> = LABEL_FOLDER_IDS
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", scope_params.len() + 1 + i))
        .collect();

    let sql = format!(
        "SELECT tl.label_id AS folder_id, COUNT(*) AS unread_count FROM threads t
         INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
         WHERE {scope_clause} AND t.is_read = 0
           AND t.shared_mailbox_id IS NULL AND t.is_chat_thread = 0
           AND tl.label_id IN ({})
         GROUP BY tl.label_id",
        placeholders.join(", ")
    );

    let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = scope_params;
    for id in LABEL_FOLDER_IDS {
        all_params.push(Box::new((*id).to_owned()));
    }
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        all_params.iter().map(AsRef::as_ref).collect();

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    stmt.query_map(param_refs.as_slice(), FolderUnreadCount::from_row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Unread count for a flag-based virtual folder (STARRED or SNOOZED).
fn get_flag_folder_unread_count(
    conn: &Connection,
    scope: &AccountScope,
    folder_id: &str,
) -> Result<FolderUnreadCount, String> {
    let (scope_clause, scope_params) = account_scope_clause(scope, 1);
    let flag_col = flag_column(folder_id);

    let sql = format!(
        "SELECT COUNT(*) AS cnt FROM threads t
         WHERE {scope_clause} AND t.is_read = 0 AND t.{flag_col} = 1
           AND t.shared_mailbox_id IS NULL AND t.is_chat_thread = 0"
    );

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        scope_params.iter().map(AsRef::as_ref).collect();

    let count = conn
        .query_row(&sql, param_refs.as_slice(), |row| row.get::<_, i64>("cnt"))
        .map_err(|e| e.to_string())?;

    Ok(FolderUnreadCount {
        folder_id: folder_id.to_owned(),
        unread_count: count,
    })
}

/// Label-based folder unread counts grouped by account.
fn get_label_folder_unread_counts_by_account(
    conn: &Connection,
    scope: &AccountScope,
) -> Result<Vec<FolderAccountUnreadCount>, String> {
    let (scope_clause, scope_params) = account_scope_clause(scope, 1);
    let placeholders: Vec<String> = LABEL_FOLDER_IDS
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", scope_params.len() + 1 + i))
        .collect();

    let sql = format!(
        "SELECT tl.label_id AS folder_id, t.account_id, COUNT(*) AS unread_count FROM threads t
         INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
         WHERE {scope_clause} AND t.is_read = 0
           AND t.shared_mailbox_id IS NULL AND t.is_chat_thread = 0
           AND tl.label_id IN ({})
         GROUP BY tl.label_id, t.account_id",
        placeholders.join(", ")
    );

    let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = scope_params;
    for id in LABEL_FOLDER_IDS {
        all_params.push(Box::new((*id).to_owned()));
    }
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        all_params.iter().map(AsRef::as_ref).collect();

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    stmt.query_map(param_refs.as_slice(), FolderAccountUnreadCount::from_row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Flag-based virtual folder unread counts grouped by account.
fn get_flag_folder_unread_by_account(
    conn: &Connection,
    scope: &AccountScope,
    folder_id: &str,
) -> Result<Vec<FolderAccountUnreadCount>, String> {
    let (scope_clause, scope_params) = account_scope_clause(scope, 1);
    let flag_col = flag_column(folder_id);

    let sql = format!(
        "SELECT t.account_id, COUNT(*) AS unread_count FROM threads t
         WHERE {scope_clause} AND t.is_read = 0 AND t.{flag_col} = 1
           AND t.shared_mailbox_id IS NULL AND t.is_chat_thread = 0
         GROUP BY t.account_id"
    );

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        scope_params.iter().map(AsRef::as_ref).collect();

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    stmt.query_map(param_refs.as_slice(), |row| {
        Ok(FolderAccountUnreadCount {
            folder_id: folder_id.to_owned(),
            account_id: row.get("account_id")?,
            unread_count: row.get("unread_count")?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

/// Threads where `is_starred = 1`, scoped by `AccountScope`.
#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn get_starred_threads(
    conn: &Connection,
    scope: &AccountScope,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DbThread>, String> {
    get_flag_threads(conn, scope, "is_starred", limit, offset)
}

/// Threads where `is_snoozed = 1`, scoped by `AccountScope`.
#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn get_snoozed_threads(
    conn: &Connection,
    scope: &AccountScope,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DbThread>, String> {
    get_flag_threads(conn, scope, "is_snoozed", limit, offset)
}

/// Draft threads (via `thread_labels` DRAFT label), scoped by `AccountScope`.
///
/// **Note**: This only returns server-synced drafts that have threads. Local-only
/// drafts (in the `local_drafts` table) are not included because they have a
/// different schema and no `DbThread` representation. Use
/// `get_draft_count_with_local()` for a count that includes both.
#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn get_draft_threads(
    conn: &Connection,
    scope: &AccountScope,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DbThread>, String> {
    let lim = limit.unwrap_or(50);
    let off = offset.unwrap_or(0);

    let (scope_clause, scope_params) = account_scope_clause(scope, 1);
    let next_idx = scope_params.len() + 1;

    let sql = format!(
        "SELECT t.*, m.from_name, m.from_address FROM threads t
         INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
         LEFT JOIN ({LATEST_MESSAGE_SUBQUERY}
         ) m ON m.account_id = t.account_id AND m.thread_id = t.id
         WHERE {scope_clause} AND tl.label_id = ?{next_idx}
           AND t.shared_mailbox_id IS NULL AND t.is_chat_thread = 0
         GROUP BY t.account_id, t.id
         ORDER BY t.is_pinned DESC, t.last_message_at DESC
         LIMIT ?{limit_idx} OFFSET ?{offset_idx}",
        limit_idx = next_idx + 1,
        offset_idx = next_idx + 2,
    );
    execute_thread_query_with_label(conn, &sql, scope_params, "DRAFT", lim, off)
}

/// Shared implementation for flag-based thread queries (`is_starred`, `is_snoozed`).
fn get_flag_threads(
    conn: &Connection,
    scope: &AccountScope,
    flag_col: &str,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DbThread>, String> {
    let lim = limit.unwrap_or(50);
    let off = offset.unwrap_or(0);

    let (scope_clause, scope_params) = account_scope_clause(scope, 1);
    let next_idx = scope_params.len() + 1;

    let sql = format!(
        "SELECT t.*, m.from_name, m.from_address FROM threads t
         LEFT JOIN ({LATEST_MESSAGE_SUBQUERY}
         ) m ON m.account_id = t.account_id AND m.thread_id = t.id
         WHERE {scope_clause} AND t.{flag_col} = 1
           AND t.shared_mailbox_id IS NULL AND t.is_chat_thread = 0
         ORDER BY t.is_pinned DESC, t.last_message_at DESC
         LIMIT ?{next_idx} OFFSET ?{offset_idx}",
        offset_idx = next_idx + 1,
    );
    execute_thread_query_no_label(conn, &sql, scope_params, lim, off)
}

/// Map a virtual folder ID to the corresponding boolean column on `threads`.
fn flag_column(folder_id: &str) -> &'static str {
    match folder_id {
        "STARRED" => "is_starred",
        "SNOOZED" => "is_snoozed",
        _ => "is_starred", // fallback, shouldn't be reached
    }
}

/// Count of all drafts (server-synced thread drafts + local-only drafts),
/// scoped by `AccountScope`.
///
/// This is the correct count for the sidebar's "Drafts" folder, per the
/// documented requirement that draft counts include local-only drafts
/// (docs/sidebar/problem-statement.md).
pub fn get_draft_count_with_local(conn: &Connection, scope: &AccountScope) -> Result<i64, String> {
    // Count server-synced drafts (threads with DRAFT label)
    let synced = get_thread_count_scoped(conn, scope, Some("DRAFT"))?;

    // Count local-only drafts (pending or new, not yet synced to a thread)
    let local = count_local_drafts(conn, scope)?;

    Ok(synced + local)
}

/// Local draft summary for display in the thread list.
///
/// These fields mirror the subset of `DbThread` that the app layer uses
/// to build a unified drafts list. Local-only drafts don't have real threads,
/// so most fields default to sensible values.
#[derive(Debug, Clone)]
pub struct LocalDraftSummary {
    pub id: String,
    pub account_id: String,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub updated_at: i64,
    pub from_email: Option<String>,
}

/// Fetch local-only drafts (not yet synced) as summaries, scoped by `AccountScope`.
///
/// Returns drafts from `local_drafts` where `sync_status != 'synced'`,
/// ordered by `updated_at DESC`. These are combined with server-synced
/// draft threads by the app layer to produce a unified drafts view.
pub fn get_local_draft_summaries(
    conn: &Connection,
    scope: &AccountScope,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<LocalDraftSummary>, String> {
    let lim = limit.unwrap_or(50);
    let off = offset.unwrap_or(0);

    let (scope_clause, scope_params) = account_scope_clause(scope, 1);
    let clause = scope_clause.replace("t.account_id", "account_id");
    let next_idx = scope_params.len() + 1;

    let sql = format!(
        "SELECT id, account_id, subject, body_html, updated_at, from_email
         FROM local_drafts
         WHERE {clause} AND sync_status != 'synced'
         ORDER BY updated_at DESC
         LIMIT ?{next_idx} OFFSET ?{offset_idx}",
        offset_idx = next_idx + 1,
    );

    let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = scope_params;
    all_params.push(Box::new(lim));
    all_params.push(Box::new(off));

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        all_params.iter().map(AsRef::as_ref).collect();

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    stmt.query_map(param_refs.as_slice(), |row| {
        let body_html: Option<String> = row.get("body_html")?;
        let snippet = body_html.map(|html| {
            // Strip HTML tags for a simple snippet
            let text = html
                .replace("<br>", " ")
                .replace("<br/>", " ")
                .replace("<br />", " ")
                .replace("&nbsp;", " ");
            let mut result = String::new();
            let mut in_tag = false;
            for ch in text.chars() {
                if ch == '<' {
                    in_tag = true;
                } else if ch == '>' {
                    in_tag = false;
                } else if !in_tag {
                    result.push(ch);
                }
            }
            let trimmed: String = result.split_whitespace().collect::<Vec<_>>().join(" ");
            if trimmed.len() > 200 {
                format!("{}...", &trimmed[..197])
            } else {
                trimmed
            }
        });
        Ok(LocalDraftSummary {
            id: row.get("id")?,
            account_id: row.get("account_id")?,
            subject: row.get("subject")?,
            snippet,
            updated_at: row.get("updated_at")?,
            from_email: row.get("from_email")?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

/// Threads belonging to a specific shared mailbox.
///
/// Uses a CTE to pre-filter thread IDs by `shared_mailbox_id`, then scopes
/// the latest-message subquery to only those threads (avoiding a full scan
/// of the messages table).
#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn get_threads_for_shared_mailbox(
    conn: &Connection,
    account_id: &str,
    mailbox_id: &str,
    label_id: Option<&str>,
    limit: Option<i64>,
) -> Result<Vec<DbThread>, String> {
    let lim = limit.unwrap_or(200);

    let sql = if let Some(lid) = label_id {
        format!(
            "WITH mb_threads AS (
               SELECT id FROM threads
               WHERE account_id = ?1 AND shared_mailbox_id = ?2
             )
             SELECT t.*, m.from_name, m.from_address FROM threads t
             INNER JOIN thread_labels tl
               ON tl.account_id = t.account_id AND tl.thread_id = t.id
             LEFT JOIN (
               SELECT id, account_id, thread_id, from_name, from_address FROM (
                 SELECT id, account_id, thread_id, from_name, from_address,
                        ROW_NUMBER() OVER (
                          PARTITION BY account_id, thread_id
                          ORDER BY date DESC, id DESC
                        ) AS rn
                 FROM messages
                 WHERE account_id = ?1 AND thread_id IN (SELECT id FROM mb_threads)
               ) WHERE rn = 1
             ) m ON m.account_id = t.account_id AND m.thread_id = t.id
             WHERE t.account_id = ?1 AND t.shared_mailbox_id = ?2
               AND t.is_chat_thread = 0
               AND tl.label_id = ?3
             GROUP BY t.account_id, t.id
             ORDER BY t.is_pinned DESC, t.last_message_at DESC
             LIMIT ?4"
        )
    } else {
        format!(
            "WITH mb_threads AS (
               SELECT id FROM threads
               WHERE account_id = ?1 AND shared_mailbox_id = ?2
             )
             SELECT t.*, m.from_name, m.from_address FROM threads t
             LEFT JOIN (
               SELECT id, account_id, thread_id, from_name, from_address FROM (
                 SELECT id, account_id, thread_id, from_name, from_address,
                        ROW_NUMBER() OVER (
                          PARTITION BY account_id, thread_id
                          ORDER BY date DESC, id DESC
                        ) AS rn
                 FROM messages
                 WHERE account_id = ?1 AND thread_id IN (SELECT id FROM mb_threads)
               ) WHERE rn = 1
             ) m ON m.account_id = t.account_id AND m.thread_id = t.id
             WHERE t.account_id = ?1 AND t.shared_mailbox_id = ?2
               AND t.is_chat_thread = 0
             ORDER BY t.is_pinned DESC, t.last_message_at DESC
             LIMIT ?3"
        )
    };

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;

    if label_id.is_some() {
        stmt.query_map(
            rusqlite::params![account_id, mailbox_id, label_id, lim],
            DbThread::from_row,
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
    } else {
        stmt.query_map(
            rusqlite::params![account_id, mailbox_id, lim],
            DbThread::from_row,
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
    }
}

/// Count local drafts that don't yet have a server-synced thread.
fn count_local_drafts(conn: &Connection, scope: &AccountScope) -> Result<i64, String> {
    let (scope_clause, scope_params) = account_scope_clause(scope, 1);
    // Rewrite "t.account_id" references to just "account_id" for local_drafts table
    let clause = scope_clause.replace("t.account_id", "account_id");

    let sql = format!(
        "SELECT COUNT(*) AS cnt FROM local_drafts WHERE {clause} AND sync_status != 'synced'"
    );

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        scope_params.iter().map(AsRef::as_ref).collect();

    conn.query_row(&sql, param_refs.as_slice(), |row| row.get::<_, i64>("cnt"))
        .map_err(|e| e.to_string())
}

/// A single item from a public folder.
#[derive(Debug, Clone)]
pub struct PublicFolderItem {
    pub item_id: String,
    pub account_id: String,
    pub folder_id: String,
    pub subject: Option<String>,
    pub sender_name: Option<String>,
    pub sender_email: Option<String>,
    pub received_at: Option<i64>,
    pub body_preview: Option<String>,
    pub is_read: bool,
    pub item_class: String,
}

/// Items from a pinned public folder, ordered by received date.
#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn get_public_folder_items(
    conn: &Connection,
    account_id: &str,
    folder_id: &str,
    limit: Option<i64>,
) -> Result<Vec<PublicFolderItem>, String> {
    let lim = limit.unwrap_or(200);
    let mut stmt = conn
        .prepare(
            "SELECT item_id, account_id, folder_id, subject, sender_name,
                    sender_email, received_at, body_preview, is_read, item_class
             FROM public_folder_items
             WHERE account_id = ?1 AND folder_id = ?2
             ORDER BY received_at DESC
             LIMIT ?3",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map(rusqlite::params![account_id, folder_id, lim], |row| {
        Ok(PublicFolderItem {
            item_id: row.get("item_id")?,
            account_id: row.get("account_id")?,
            folder_id: row.get("folder_id")?,
            subject: row.get("subject")?,
            sender_name: row.get("sender_name")?,
            sender_email: row.get("sender_email")?,
            received_at: row.get("received_at")?,
            body_preview: row.get("body_preview")?,
            is_read: row.get::<_, i64>("is_read")? != 0,
            item_class: row.get("item_class")?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}
