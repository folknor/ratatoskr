use rusqlite::Connection;

use crate::db::queries::row_to_thread;
use crate::db::types::{AccountScope, DbThread};

use super::parser::ParsedQuery;

// ── Public entry points ─────────────────────────────────────

/// Query threads matching a parsed smart folder query within the given account scope.
pub fn query_threads(
    conn: &Connection,
    parsed: &ParsedQuery,
    scope: &AccountScope,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DbThread>, String> {
    let lim = limit.unwrap_or(50);
    let off = offset.unwrap_or(0);
    let mut ctx = QueryContext::new();

    build_message_clauses(&mut ctx, parsed);
    build_scope_clause(&mut ctx, scope);
    build_thread_flag_clauses(&mut ctx, parsed);
    build_label_clause(&mut ctx, parsed);

    let where_str = ctx.where_string();
    let thread_flag_str = ctx.thread_flag_where_string();

    let sql = build_thread_select_sql(&where_str, &thread_flag_str, ctx.next_idx);
    ctx.params.push(Box::new(lim));
    ctx.params.push(Box::new(off));

    execute_thread_query(conn, &sql, &ctx.params)
}

/// Count distinct threads matching a parsed query (used for unread counts).
pub fn count_matching(
    conn: &Connection,
    parsed: &ParsedQuery,
    scope: &AccountScope,
) -> Result<i64, String> {
    let mut ctx = QueryContext::new();

    build_message_clauses(&mut ctx, parsed);
    build_scope_clause(&mut ctx, scope);
    build_thread_flag_clauses(&mut ctx, parsed);
    build_label_clause(&mut ctx, parsed);

    let where_str = ctx.where_string();
    let thread_flag_str = ctx.thread_flag_where_string();

    let sql = build_count_sql(&where_str, &thread_flag_str);

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        ctx.params.iter().map(AsRef::as_ref).collect();

    conn.query_row(&sql, param_refs.as_slice(), |row| row.get::<_, i64>(0))
        .map_err(|e| e.to_string())
}

// ── Query context ───────────────────────────────────────────

/// Accumulates WHERE clauses and parameters during query building.
struct QueryContext {
    /// Clauses that filter on the messages table (alias `m`).
    msg_clauses: Vec<String>,
    /// Clauses that filter on the threads table (alias `t`) — for boolean flags.
    thread_flag_clauses: Vec<String>,
    /// Positional parameters.
    params: Vec<Box<dyn rusqlite::types::ToSql>>,
    /// Next parameter index (1-based).
    next_idx: usize,
}

impl QueryContext {
    fn new() -> Self {
        Self {
            msg_clauses: Vec::new(),
            thread_flag_clauses: Vec::new(),
            params: Vec::new(),
            next_idx: 1,
        }
    }

    fn push_param(&mut self, val: Box<dyn rusqlite::types::ToSql>) -> usize {
        let idx = self.next_idx;
        self.params.push(val);
        self.next_idx += 1;
        idx
    }

    fn where_string(&self) -> String {
        if self.msg_clauses.is_empty() {
            String::new()
        } else {
            format!(" AND {}", self.msg_clauses.join(" AND "))
        }
    }

    fn thread_flag_where_string(&self) -> String {
        if self.thread_flag_clauses.is_empty() {
            String::new()
        } else {
            format!(" AND {}", self.thread_flag_clauses.join(" AND "))
        }
    }
}

// ── Clause builders ─────────────────────────────────────────

/// Add WHERE clauses for message-level filters (free text, from, to, subject, dates, read state).
fn build_message_clauses(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    build_free_text_clause(ctx, parsed);
    build_from_clause(ctx, parsed);
    build_to_clause(ctx, parsed);
    build_subject_clause(ctx, parsed);
    build_attachment_clause(ctx, parsed);
    build_read_clauses(ctx, parsed);
    build_date_clauses(ctx, parsed);
}

fn build_free_text_clause(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    if parsed.free_text.is_empty() {
        return;
    }
    let idx = ctx.push_param(Box::new(parsed.free_text.clone()));
    ctx.msg_clauses.push(format!(
        "(m.subject LIKE '%' || ?{idx} || '%' \
         OR m.from_name LIKE '%' || ?{idx} || '%' \
         OR m.from_address LIKE '%' || ?{idx} || '%' \
         OR m.snippet LIKE '%' || ?{idx} || '%')"
    ));
}

fn build_from_clause(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    if let Some(ref from) = parsed.from {
        let idx = ctx.push_param(Box::new(from.clone()));
        ctx.msg_clauses.push(format!(
            "(m.from_address LIKE '%' || ?{idx} || '%' \
             OR m.from_name LIKE '%' || ?{idx} || '%')"
        ));
    }
}

fn build_to_clause(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    if let Some(ref to) = parsed.to {
        let idx = ctx.push_param(Box::new(to.clone()));
        ctx.msg_clauses.push(format!(
            "m.to_addresses LIKE '%' || ?{idx} || '%'"
        ));
    }
}

fn build_subject_clause(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    if let Some(ref subject) = parsed.subject {
        let idx = ctx.push_param(Box::new(subject.clone()));
        ctx.msg_clauses.push(format!(
            "m.subject LIKE '%' || ?{idx} || '%'"
        ));
    }
}

fn build_attachment_clause(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    if parsed.has_attachment == Some(true) {
        ctx.msg_clauses.push(
            "EXISTS (SELECT 1 FROM attachments a \
             WHERE a.account_id = m.account_id AND a.message_id = m.id)"
                .to_owned(),
        );
    }
}

fn build_read_clauses(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    if parsed.is_unread == Some(true) {
        ctx.msg_clauses.push("m.is_read = 0".to_owned());
    }
    if parsed.is_read == Some(true) {
        ctx.msg_clauses.push("m.is_read = 1".to_owned());
    }
    if parsed.is_starred == Some(true) {
        ctx.msg_clauses.push("m.is_starred = 1".to_owned());
    }
}

fn build_date_clauses(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    if let Some(before) = parsed.before {
        let idx = ctx.push_param(Box::new(before));
        ctx.msg_clauses.push(format!("m.date < ?{idx}"));
    }
    if let Some(after) = parsed.after {
        let idx = ctx.push_param(Box::new(after));
        ctx.msg_clauses.push(format!("m.date > ?{idx}"));
    }
}

/// Add account scope clause (operates on `m.account_id`).
fn build_scope_clause(ctx: &mut QueryContext, scope: &AccountScope) {
    match scope {
        AccountScope::Single(id) => {
            let idx = ctx.push_param(Box::new(id.clone()));
            ctx.msg_clauses.push(format!("m.account_id = ?{idx}"));
        }
        AccountScope::Multiple(ids) => {
            if ids.is_empty() {
                ctx.msg_clauses.push("0=1".to_owned());
                return;
            }
            let placeholders: Vec<String> = ids
                .iter()
                .map(|id| {
                    let idx = ctx.push_param(Box::new(id.clone()));
                    format!("?{idx}")
                })
                .collect();
            ctx.msg_clauses
                .push(format!("m.account_id IN ({})", placeholders.join(", ")));
        }
        AccountScope::All => { /* no filter */ }
    }
}

/// Add thread-level flag clauses (snoozed, pinned, muted, important).
/// These filter on the `threads` table rather than `messages`.
fn build_thread_flag_clauses(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    if parsed.is_snoozed == Some(true) {
        ctx.thread_flag_clauses.push("t.is_snoozed = 1".to_owned());
    }
    if parsed.is_pinned == Some(true) {
        ctx.thread_flag_clauses.push("t.is_pinned = 1".to_owned());
    }
    if parsed.is_muted == Some(true) {
        ctx.thread_flag_clauses.push("t.is_muted = 1".to_owned());
    }
    if parsed.is_important == Some(true) {
        ctx.thread_flag_clauses
            .push("t.is_important = 1".to_owned());
    }
}

/// Add label clause via EXISTS subquery on thread_labels.
fn build_label_clause(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    if let Some(ref label) = parsed.label {
        let idx = ctx.push_param(Box::new(label.clone()));
        ctx.msg_clauses.push(format!(
            "EXISTS (SELECT 1 FROM thread_labels tl \
             JOIN labels l ON l.account_id = tl.account_id AND l.id = tl.label_id \
             WHERE tl.account_id = m.account_id AND tl.thread_id = m.thread_id \
             AND LOWER(l.name) = LOWER(?{idx}))"
        ));
    }
}

// ── SQL templates ───────────────────────────────────────────

/// Build the main SELECT that returns `DbThread` rows from a message-based search,
/// joined back to threads for the full thread shape.
fn build_thread_select_sql(
    msg_where: &str,
    thread_flag_where: &str,
    next_idx: usize,
) -> String {
    let limit_idx = next_idx;
    let offset_idx = next_idx + 1;

    format!(
        "SELECT t.*, latest_m.from_name, latest_m.from_address \
         FROM threads t \
         INNER JOIN ( \
           SELECT DISTINCT m.account_id, m.thread_id \
           FROM messages m \
           WHERE 1=1{msg_where} \
         ) matched ON matched.account_id = t.account_id AND matched.thread_id = t.id \
         LEFT JOIN ( \
           SELECT id, account_id, thread_id, from_name, from_address FROM ( \
             SELECT id, account_id, thread_id, from_name, from_address, \
                    ROW_NUMBER() OVER ( \
                      PARTITION BY account_id, thread_id \
                      ORDER BY date DESC, id DESC \
                    ) AS rn \
             FROM messages \
           ) WHERE rn = 1 \
         ) latest_m ON latest_m.account_id = t.account_id AND latest_m.thread_id = t.id \
         WHERE 1=1{thread_flag_where} \
         ORDER BY t.is_pinned DESC, t.last_message_at DESC \
         LIMIT ?{limit_idx} OFFSET ?{offset_idx}"
    )
}

/// Build a COUNT query for matching threads.
fn build_count_sql(msg_where: &str, thread_flag_where: &str) -> String {
    format!(
        "SELECT COUNT(*) FROM threads t \
         INNER JOIN ( \
           SELECT DISTINCT m.account_id, m.thread_id \
           FROM messages m \
           WHERE 1=1{msg_where} \
         ) matched ON matched.account_id = t.account_id AND matched.thread_id = t.id \
         WHERE 1=1{thread_flag_where}"
    )
}

// ── Execution ───────────────────────────────────────────────

fn execute_thread_query(
    conn: &Connection,
    sql: &str,
    params: &[Box<dyn rusqlite::types::ToSql>],
) -> Result<Vec<DbThread>, String> {
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        params.iter().map(AsRef::as_ref).collect();

    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    stmt.query_map(param_refs.as_slice(), row_to_thread)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}
