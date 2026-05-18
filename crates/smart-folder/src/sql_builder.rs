use db_read::db::ReadConn;
use db_read::db::FromRow;
use db_read::db::sql_fragments::LATEST_MESSAGE_SUBQUERY;
use db_read::db::types::{AccountScope, DbThread};
use types::SystemFolderId;

use super::parser::ParsedQuery;

// ── Public entry points ─────────────────────────────────────

/// Query threads matching a parsed smart folder query within the given account scope.
///
/// When `account:` operators are present in the parsed query, they override the scope parameter.
pub fn query_threads_read(
    conn: &ReadConn<'_>,
    parsed: &ParsedQuery,
    scope: &AccountScope,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DbThread>, String> {
    let lim = limit.unwrap_or(50);
    let off = offset.unwrap_or(0);
    let mut ctx = QueryContext::new();

    build_message_clauses(&mut ctx, parsed);
    build_effective_scope(&mut ctx, parsed, scope);
    build_thread_flag_clauses(&mut ctx, parsed);
    build_label_clause(&mut ctx, parsed);

    let where_str = ctx.where_string();
    let thread_flag_str = ctx.thread_flag_where_string();

    let sql = build_thread_select_sql(&where_str, &thread_flag_str, ctx.next_idx);
    log::debug!(
        "Smart folder SQL built: msg_clauses={}, thread_flag_clauses={}",
        ctx.msg_clauses.len(),
        ctx.thread_flag_clauses.len()
    );
    ctx.params.push(Box::new(lim));
    ctx.params.push(Box::new(off));

    execute_thread_query_read(conn, &sql, &ctx.params)
}

/// Return the (account_id, thread_id) keys of every thread that matches the
/// parsed query and account scope. No LIMIT - the result is exhaustive. Used
/// by the combined search pipeline to feed Tantivy a complete candidate set
/// without dropping older threads that would be cut off by a LIMIT pass.
/// Skips the latest-message join and the ORDER BY since callers only need
/// the keys; hydration happens separately for the top-N Tantivy hits.
pub fn query_thread_keys_read(
    conn: &ReadConn<'_>,
    parsed: &ParsedQuery,
    scope: &AccountScope,
) -> Result<Vec<(String, String)>, String> {
    let mut ctx = QueryContext::new();

    build_message_clauses(&mut ctx, parsed);
    build_effective_scope(&mut ctx, parsed, scope);
    build_thread_flag_clauses(&mut ctx, parsed);
    build_label_clause(&mut ctx, parsed);

    let where_str = ctx.where_string();
    let thread_flag_str = ctx.thread_flag_where_string();

    let sql = format!(
        "SELECT t.account_id, t.id \
         FROM threads t \
         INNER JOIN ( \
           SELECT DISTINCT m.account_id, m.thread_id \
           FROM messages m \
           WHERE 1=1{where_str} \
         ) matched ON matched.account_id = t.account_id AND matched.thread_id = t.id \
         WHERE 1=1{thread_flag_str}"
    );

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        ctx.params.iter().map(AsRef::as_ref).collect();

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    stmt.query_map(param_refs.as_slice(), |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

/// Count distinct threads matching a parsed query (used for unread counts).
pub fn count_matching_read(
    conn: &ReadConn<'_>,
    parsed: &ParsedQuery,
    scope: &AccountScope,
) -> Result<i64, String> {
    let mut ctx = QueryContext::new();

    build_message_clauses(&mut ctx, parsed);
    build_effective_scope(&mut ctx, parsed, scope);
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
    /// Clauses that filter on the threads table (alias `t`) - for boolean flags.
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

// ── In-folder shorthand mappings ─────────────────────────────

/// Shorthands that map to thread boolean flags instead of label joins.
const IN_FLAG_SHORTHANDS: &[(&str, &str)] = &[
    ("starred", "t.is_starred = 1"),
    ("snoozed", "t.is_snoozed = 1"),
];

// ── Clause builders ─────────────────────────────────────────

/// Add WHERE clauses for message-level filters.
fn build_message_clauses(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    build_free_text_clause(ctx, parsed);
    build_from_clause(ctx, parsed);
    build_to_clause(ctx, parsed);
    build_attachment_clause(ctx, parsed);
    build_date_clauses(ctx, parsed);
    build_folder_clause(ctx, parsed);
    build_in_folder_clauses(ctx, parsed);
    build_attachment_type_clause(ctx, parsed);
    build_has_contact_clause(ctx, parsed);
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

/// Build `from:` clause with contact expansion. If `?N` matches a contact's
/// display name, that contact's `email` and `email2` are also considered as
/// possible sender addresses.
fn build_from_clause(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    if parsed.from.is_empty() {
        return;
    }
    let parts: Vec<String> = parsed
        .from
        .iter()
        .map(|from| {
            let idx = ctx.push_param(Box::new(from.clone()));
            format!(
                "(m.from_address LIKE '%' || ?{idx} || '%' \
                 OR m.from_name LIKE '%' || ?{idx} || '%' \
                 OR EXISTS (\
                   SELECT 1 FROM contacts c \
                   WHERE c.display_name LIKE '%' || ?{idx} || '%' \
                     AND (m.from_address = c.email OR m.from_address = c.email2)))"
            )
        })
        .collect();
    ctx.msg_clauses.push(format!("({})", parts.join(" OR ")));
}

/// Build `to:` clause. Matches against `to_addresses`, `cc_addresses`, and
/// `bcc_addresses` directly, and additionally expands via the contacts table:
/// if `?N` matches a contact's display name, that contact's email is also
/// looked for in the three address columns. Mirrors the contact-expansion
/// path in `build_from_clause`.
fn build_to_clause(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    if parsed.to.is_empty() {
        return;
    }
    let parts: Vec<String> = parsed
        .to
        .iter()
        .map(|to| {
            let idx = ctx.push_param(Box::new(to.clone()));
            format!(
                "(m.to_addresses LIKE '%' || ?{idx} || '%' \
                 OR m.cc_addresses LIKE '%' || ?{idx} || '%' \
                 OR m.bcc_addresses LIKE '%' || ?{idx} || '%' \
                 OR EXISTS (\
                   SELECT 1 FROM contacts c \
                   WHERE c.display_name LIKE '%' || ?{idx} || '%' \
                     AND (m.to_addresses LIKE '%' || c.email || '%' \
                       OR m.cc_addresses LIKE '%' || c.email || '%' \
                       OR m.bcc_addresses LIKE '%' || c.email || '%' \
                       OR (c.email2 IS NOT NULL AND (\
                            m.to_addresses LIKE '%' || c.email2 || '%' \
                         OR m.cc_addresses LIKE '%' || c.email2 || '%' \
                         OR m.bcc_addresses LIKE '%' || c.email2 || '%')))))"
            )
        })
        .collect();
    ctx.msg_clauses.push(format!("({})", parts.join(" OR ")));
}

fn build_attachment_clause(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    if parsed.has_attachment {
        ctx.msg_clauses.push(
            "EXISTS (SELECT 1 FROM attachments a \
             WHERE a.account_id = m.account_id AND a.message_id = m.id)"
                .to_owned(),
        );
    }
}

fn build_date_clauses(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    if let Some(before) = parsed.before {
        let idx = ctx.next_idx;
        let (clause, value) = before.to_sql_clause("m.date", idx);
        ctx.push_param(Box::new(value));
        ctx.msg_clauses.push(clause);
    }
    if let Some(after) = parsed.after {
        let idx = ctx.next_idx;
        let (clause, value) = after.to_sql_clause("m.date", idx);
        ctx.push_param(Box::new(value));
        ctx.msg_clauses.push(clause);
    }
}

/// When `account:` operators are present, they override the scope parameter.
/// Otherwise, apply the scope normally.
fn build_effective_scope(ctx: &mut QueryContext, parsed: &ParsedQuery, scope: &AccountScope) {
    if parsed.account.is_empty() {
        build_scope_clause(ctx, scope);
    } else {
        build_account_clause(ctx, parsed);
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

/// Build `account:` clause - match by display_name or email on the accounts table.
/// OR semantics across multiple account values.
fn build_account_clause(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    let parts: Vec<String> = parsed
        .account
        .iter()
        .map(|acct| {
            let idx = ctx.push_param(Box::new(acct.clone()));
            format!(
                "m.account_id IN (\
                   SELECT a.id FROM accounts a \
                   WHERE a.display_name LIKE '%' || ?{idx} || '%' \
                   OR a.email LIKE '%' || ?{idx} || '%')"
            )
        })
        .collect();
    ctx.msg_clauses.push(format!("({})", parts.join(" OR ")));
}

/// Build `folder:` clause - match folder name or IMAP folder path.
fn build_folder_clause(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    if parsed.folder.is_empty() {
        return;
    }
    let parts: Vec<String> = parsed
        .folder
        .iter()
        .map(|folder| {
            let idx = ctx.push_param(Box::new(folder.clone()));
            format!(
                "EXISTS (SELECT 1 FROM thread_folders tf \
                 JOIN folders f ON f.account_id = tf.account_id AND f.id = tf.folder_id \
                 WHERE tf.account_id = m.account_id AND tf.thread_id = m.thread_id \
                 AND (f.name LIKE '%' || ?{idx} || '%' \
                   OR f.imap_folder_path LIKE '%' || ?{idx} || '%'))"
            )
        })
        .collect();
    ctx.msg_clauses.push(format!("({})", parts.join(" OR ")));
}

/// Build `in:` folder-based clauses (inbox, sent, drafts, trash, spam, etc.).
/// Flag-based shorthands (starred, snoozed) are handled in `build_thread_flag_clauses`.
fn build_in_folder_clauses(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    let folder_values: Vec<&str> = parsed
        .in_folder
        .iter()
        .filter_map(|v| SystemFolderId::parse_shorthand(v).map(SystemFolderId::as_str))
        .collect();

    if folder_values.is_empty() {
        return;
    }

    let parts: Vec<String> = folder_values
        .iter()
        .map(|folder_id| {
            let idx = ctx.push_param(Box::new(folder_id.to_string()));
            format!(
                "EXISTS (SELECT 1 FROM thread_folders tf \
                 WHERE tf.account_id = m.account_id \
                 AND tf.thread_id = m.thread_id \
                 AND tf.folder_id = ?{idx})"
            )
        })
        .collect();
    ctx.msg_clauses.push(format!("({})", parts.join(" OR ")));
}

/// Build `has:<type>` / `type:` clause - filter by attachment MIME type.
fn build_attachment_type_clause(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    if parsed.attachment_types.is_empty() {
        return;
    }
    let mime_conditions: Vec<String> = parsed
        .attachment_types
        .iter()
        .map(|mime| build_single_mime_condition(ctx, mime))
        .collect();

    ctx.msg_clauses.push(format!(
        "EXISTS (SELECT 1 FROM attachments a \
         WHERE a.account_id = m.account_id AND a.message_id = m.id \
         AND ({}))",
        mime_conditions.join(" OR ")
    ));
}

/// Build a single MIME condition - glob patterns use LIKE, exact types use `=`.
fn build_single_mime_condition(ctx: &mut QueryContext, mime: &str) -> String {
    if mime.ends_with("/*") {
        // Glob: e.g. "video/*" -> LIKE 'video/%'
        let prefix = &mime[..mime.len() - 1];
        let like_pattern = format!("{prefix}%");
        let idx = ctx.push_param(Box::new(like_pattern));
        format!("a.mime_type LIKE ?{idx}")
    } else {
        let idx = ctx.push_param(Box::new(mime.to_owned()));
        format!("a.mime_type = ?{idx}")
    }
}

/// Build `has:contact` clause - matches when any participant (sender or
/// to/cc/bcc recipient) is a known contact.
fn build_has_contact_clause(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    if !parsed.has_contact {
        return;
    }
    ctx.msg_clauses.push(
        "EXISTS (\
           SELECT 1 FROM contacts c \
           WHERE c.email = m.from_address \
              OR m.to_addresses LIKE '%' || c.email || '%' \
              OR m.cc_addresses LIKE '%' || c.email || '%' \
              OR m.bcc_addresses LIKE '%' || c.email || '%' \
              OR (c.email2 IS NOT NULL AND (\
                   c.email2 = m.from_address \
                OR m.to_addresses LIKE '%' || c.email2 || '%' \
                OR m.cc_addresses LIKE '%' || c.email2 || '%' \
                OR m.bcc_addresses LIKE '%' || c.email2 || '%')))"
            .to_owned(),
    );
}

/// Add thread-level flag clauses (snoozed, pinned, muted, tagged).
/// Also handles `in:starred` and `in:snoozed` shorthands.
fn build_thread_flag_clauses(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    build_thread_state_clauses(ctx, parsed);
    if parsed.is_snoozed == Some(true) {
        ctx.thread_flag_clauses.push("t.is_snoozed = 1".to_owned());
    }
    if parsed.is_pinned == Some(true) {
        ctx.thread_flag_clauses.push("t.is_pinned = 1".to_owned());
    }
    if parsed.is_muted == Some(true) {
        ctx.thread_flag_clauses.push("t.is_muted = 1".to_owned());
    }
    if parsed.is_tagged == Some(true) {
        build_is_tagged_clause(ctx);
    }
    build_in_folder_flag_clauses(ctx, parsed);
}

/// Build thread-aggregate state predicates.
fn build_thread_state_clauses(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    if parsed.is_unread == Some(true) {
        ctx.thread_flag_clauses.push("t.is_read = 0".to_owned());
    }
    if parsed.is_read == Some(true) {
        ctx.thread_flag_clauses.push("t.is_read = 1".to_owned());
    }
    if parsed.is_starred == Some(true) {
        ctx.thread_flag_clauses.push("t.is_starred = 1".to_owned());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LabelGroupRenderGrain {
    Thread,
    Message,
}

impl LabelGroupRenderGrain {
    fn account_column(self) -> &'static str {
        match self {
            Self::Thread => "t.account_id",
            Self::Message => "m.account_id",
        }
    }

    fn thread_column(self) -> &'static str {
        match self {
            Self::Thread => "t.id",
            Self::Message => "m.thread_id",
        }
    }
}

/// SQL fragment: "thread (acct, tid) renders the label group with `<group_predicate>`".
///
/// `grain` selects the only two legal outer-column pairs: thread flags use
/// `t.account_id` / `t.id`, while message clauses use `m.account_id` /
/// `m.thread_id`. `group_predicate` is the constraint on the `lg` alias
/// (e.g. `"1=1"` for "any group", or `"LOWER(lg.name) = LOWER(?N)"` for a
/// named group).
///
/// Label-group rendering derives from the user-visible label set:
/// provider truth in `thread_labels` merged with pending local label
/// intent. The old `thread_label_groups` shortcut is intentionally not
/// consulted; group composites now fan out to per-member label intent.
fn label_group_rendered_fragment(
    grain: LabelGroupRenderGrain,
    group_predicate: &str,
) -> String {
    let account_column = grain.account_column();
    let thread_column = grain.thread_column();

    db_read::db::queries_extra::user_visible_label_group_rendered_fragment(
        account_column,
        thread_column,
        group_predicate,
    )
}

/// Build `is:tagged` clause - thread renders at least one label group.
fn build_is_tagged_clause(ctx: &mut QueryContext) {
    ctx.thread_flag_clauses.push(label_group_rendered_fragment(
        LabelGroupRenderGrain::Thread,
        "1=1",
    ));
}

/// Build `in:starred` / `in:snoozed` as thread flag clauses.
fn build_in_folder_flag_clauses(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    let flag_clauses: Vec<&str> = parsed
        .in_folder
        .iter()
        .filter_map(|v| {
            let lower = v.to_ascii_lowercase();
            IN_FLAG_SHORTHANDS
                .iter()
                .find(|(name, _)| *name == lower)
                .map(|(_, clause)| *clause)
        })
        .collect();

    for clause in flag_clauses {
        ctx.thread_flag_clauses.push(clause.to_owned());
    }
}

/// Add label clause via explicit label groups.
fn build_label_clause(ctx: &mut QueryContext, parsed: &ParsedQuery) {
    if parsed.label.is_empty() {
        return;
    }
    let parts: Vec<String> = parsed
        .label
        .iter()
        .map(|label| {
            let idx = ctx.push_param(Box::new(label.clone()));
            label_group_rendered_fragment(
                LabelGroupRenderGrain::Message,
                &format!("LOWER(lg.name) = LOWER(?{idx})"),
            )
        })
        .collect();
    ctx.msg_clauses.push(format!("({})", parts.join(" OR ")));
}

// ── SQL templates ───────────────────────────────────────────

/// Build the main SELECT that returns `DbThread` rows from a message-based search,
/// joined back to threads for the full thread shape.
fn build_thread_select_sql(msg_where: &str, thread_flag_where: &str, next_idx: usize) -> String {
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
         LEFT JOIN ({LATEST_MESSAGE_SUBQUERY} \
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

fn execute_thread_query_read(
    conn: &ReadConn<'_>,
    sql: &str,
    params: &[Box<dyn rusqlite::types::ToSql>],
) -> Result<Vec<DbThread>, String> {
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(AsRef::as_ref).collect();

    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    stmt.query_map(param_refs.as_slice(), DbThread::from_row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

// ── Tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_query;
    use rusqlite::Connection;

    /// Spin up an in-memory DB with the real migration schema, then seed
    /// minimal data for query testing.
    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("pragmas");
        create_test_schema(&conn);
        seed_test_data(&conn);
        conn
    }

    fn create_test_schema(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE accounts (
                id TEXT PRIMARY KEY,
                email TEXT NOT NULL,
                display_name TEXT,
                provider TEXT NOT NULL,
                auth_method TEXT
             );
             CREATE TABLE threads (
                id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                subject TEXT,
                snippet TEXT,
                last_message_at INTEGER,
                message_count INTEGER NOT NULL DEFAULT 0,
                is_read INTEGER NOT NULL DEFAULT 0,
                is_starred INTEGER NOT NULL DEFAULT 0,
                is_important INTEGER NOT NULL DEFAULT 0,
                has_attachments INTEGER NOT NULL DEFAULT 0,
                is_snoozed INTEGER NOT NULL DEFAULT 0,
                snooze_until INTEGER,
                is_pinned INTEGER NOT NULL DEFAULT 0,
                is_muted INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (account_id, id)
             );
             CREATE TABLE messages (
                id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                thread_id TEXT NOT NULL,
                from_address TEXT,
                from_name TEXT,
                to_addresses TEXT,
                cc_addresses TEXT,
                bcc_addresses TEXT,
                subject TEXT,
                snippet TEXT,
                date INTEGER NOT NULL,
                is_read INTEGER NOT NULL DEFAULT 0,
                is_starred INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (account_id, id)
             );
             CREATE TABLE folders (
                id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                name TEXT NOT NULL,
                imap_folder_path TEXT,
                PRIMARY KEY (account_id, id)
             );
             CREATE TABLE labels (
                id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                name TEXT NOT NULL,
                PRIMARY KEY (account_id, id)
             );
             CREATE TABLE label_groups (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                color_bg TEXT,
                color_fg TEXT
             );
             CREATE TABLE label_group_members (
                group_id INTEGER NOT NULL,
                account_id TEXT NOT NULL,
                label_id TEXT NOT NULL
             );
             CREATE TABLE thread_folders (
                thread_id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                folder_id TEXT NOT NULL
             );
             CREATE TABLE thread_labels (
                thread_id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                label_id TEXT NOT NULL
             );
             CREATE TABLE pending_thread_label_intents (
                account_id TEXT NOT NULL,
                thread_id TEXT NOT NULL,
                label_id TEXT NOT NULL,
                op TEXT NOT NULL,
                generation_seen INTEGER NOT NULL,
                action_id TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (account_id, thread_id, label_id)
             );
             CREATE TABLE attachments (
                id TEXT PRIMARY KEY,
                message_id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                filename TEXT,
                mime_type TEXT,
                size INTEGER
             );
             CREATE TABLE contacts (
                id TEXT PRIMARY KEY,
                email TEXT NOT NULL,
                email2 TEXT,
                display_name TEXT,
                frequency INTEGER NOT NULL DEFAULT 0
             );",
        )
        .expect("test schema");
    }

    fn seed_test_data(conn: &Connection) {
        seed_accounts(conn);
        seed_threads(conn);
        seed_messages(conn);
        seed_folders_labels_and_groups(conn);
        seed_attachments(conn);
        seed_contacts(conn);
    }

    fn seed_accounts(conn: &Connection) {
        conn.execute_batch(
            "INSERT INTO accounts (id, email, display_name, provider, auth_method)
             VALUES ('acc1', 'alice@work.com', 'Work Account', 'gmail_api', 'oauth2');
             INSERT INTO accounts (id, email, display_name, provider, auth_method)
             VALUES ('acc2', 'bob@personal.com', 'Personal', 'imap', 'password');",
        )
        .expect("seed accounts");
    }

    fn seed_threads(conn: &Connection) {
        conn.execute_batch(
            "INSERT INTO threads (id, account_id, subject, snippet, last_message_at,
                message_count, is_read, is_starred, is_important, has_attachments,
                is_snoozed, is_pinned, is_muted)
             VALUES ('t1', 'acc1', 'Hello', 'snippet1', NULL, 1, 0, 1, 0, 1, 0, 0, 0);
             INSERT INTO threads (id, account_id, subject, snippet, last_message_at,
                message_count, is_read, is_starred, is_important, has_attachments,
                is_snoozed, is_pinned, is_muted)
             VALUES ('t2', 'acc2', 'Meeting', 'snippet2', NULL, 1, 1, 0, 0, 0, 0, 0, 0);",
        )
        .expect("seed threads");
    }

    fn seed_messages(conn: &Connection) {
        conn.execute_batch(
            "INSERT INTO messages (id, account_id, thread_id, from_address, from_name,
                to_addresses, subject, snippet, date, is_read, is_starred)
             VALUES ('m1', 'acc1', 't1', 'sender@example.com', 'Sender',
                'alice@work.com', 'Hello', 'snippet1', 1000, 0, 1);
             INSERT INTO messages (id, account_id, thread_id, from_address, from_name,
                to_addresses, subject, snippet, date, is_read, is_starred)
             VALUES ('m2', 'acc2', 't2', 'carol@example.com', 'Carol',
                'bob@personal.com', 'Meeting', 'snippet2', 2000, 1, 0);",
        )
        .expect("seed messages");
    }

    fn seed_split_thread_state_fixture(conn: &Connection) {
        conn.execute_batch(
            "INSERT INTO threads (id, account_id, subject, snippet, last_message_at,
                message_count, is_read, is_starred, is_important, has_attachments,
                is_snoozed, is_pinned, is_muted)
             VALUES ('t3', 'acc1', 'Split State', 'recent activity', 3000, 2, 0, 1, 0, 0, 0, 0, 0);
             INSERT INTO messages (id, account_id, thread_id, from_address, from_name,
                to_addresses, subject, snippet, date, is_read, is_starred)
             VALUES ('m3-old-unread-starred', 'acc1', 't3', 'old@example.com', 'Old Sender',
                'alice@work.com', 'Split State', 'old unread starred', 1000, 0, 1);
             INSERT INTO messages (id, account_id, thread_id, from_address, from_name,
                to_addresses, subject, snippet, date, is_read, is_starred)
             VALUES ('m3-recent-read', 'acc1', 't3', 'recent@example.com', 'Recent Sender',
                'alice@work.com', 'Split State', 'recent read', 3000, 1, 0);",
        )
        .expect("seed split thread state fixture");
    }

    fn seed_folders_labels_and_groups(conn: &Connection) {
        conn.execute_batch(
            "INSERT INTO folders (id, account_id, name)
             VALUES ('INBOX', 'acc1', 'Inbox');
             INSERT INTO folders (id, account_id, name)
             VALUES ('SENT', 'acc1', 'Sent');
             INSERT INTO folders (id, account_id, name)
             VALUES ('folder1', 'acc1', 'Receipts');
             INSERT INTO folders (id, account_id, name, imap_folder_path)
             VALUES ('INBOX', 'acc2', 'Inbox', 'INBOX');
             INSERT INTO labels (id, account_id, name)
             VALUES ('custom1', 'acc1', 'Projects');
             INSERT INTO label_groups (id, name, color_bg, color_fg)
             VALUES (1, 'Projects', '#4285f4', '#ffffff');
             INSERT INTO label_group_members (group_id, account_id, label_id)
             VALUES (1, 'acc1', 'custom1');
             INSERT INTO thread_folders (thread_id, account_id, folder_id)
             VALUES ('t1', 'acc1', 'INBOX');
             INSERT INTO thread_labels (thread_id, account_id, label_id)
             VALUES ('t1', 'acc1', 'custom1');
             INSERT INTO thread_folders (thread_id, account_id, folder_id)
             VALUES ('t1', 'acc1', 'folder1');
             INSERT INTO thread_folders (thread_id, account_id, folder_id)
             VALUES ('t2', 'acc2', 'INBOX');",
        )
        .expect("seed folders labels and groups");
    }

    fn seed_attachments(conn: &Connection) {
        conn.execute_batch(
            "INSERT INTO attachments (id, message_id, account_id, filename, mime_type, size)
             VALUES ('a1', 'm1', 'acc1', 'report.pdf', 'application/pdf', 1024);
             INSERT INTO attachments (id, message_id, account_id, filename, mime_type, size)
             VALUES ('a2', 'm1', 'acc1', 'photo.jpg', 'image/jpeg', 2048);",
        )
        .expect("seed attachments");
    }

    fn seed_contacts(conn: &Connection) {
        conn.execute_batch(
            "INSERT INTO contacts (id, email, display_name, frequency)
             VALUES ('c1', 'sender@example.com', 'Friendly Sender', 5);
             INSERT INTO contacts (id, email, display_name, frequency)
             VALUES ('c2', 'unknown@nowhere.com', 'Unknown', 1);",
        )
        .expect("seed contacts");
    }

    // -- account: operator --

    #[test]
    fn account_filters_by_display_name() {
        let conn = setup_test_db();
        let parsed = parse_query("account:Work");
        let results = query_threads_read(&ReadConn::from_raw(&conn), &parsed, &AccountScope::All, None, None);
        let threads = results.expect("query should succeed");
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].account_id, "acc1");
    }

    #[test]
    fn account_filters_by_email() {
        let conn = setup_test_db();
        let parsed = parse_query("account:personal.com");
        let threads = query_threads_read(&ReadConn::from_raw(&conn), &parsed, &AccountScope::All, None, None).expect("query");
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].account_id, "acc2");
    }

    #[test]
    fn account_overrides_scope() {
        let conn = setup_test_db();
        let parsed = parse_query("account:Work");
        // Scope says acc2, but account: operator should override.
        let scope = AccountScope::Single("acc2".to_owned());
        let threads = query_threads_read(&ReadConn::from_raw(&conn), &parsed, &scope, None, None).expect("query");
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].account_id, "acc1");
    }

    // -- folder: operator --

    #[test]
    fn folder_filters_by_folder_name() {
        let conn = setup_test_db();
        // "Receipts" is a folder on acc1. "Projects" is a tag label and label
        // group, so it must not match `folder:`.
        let parsed = parse_query("folder:Receipts");
        let threads = query_threads_read(&ReadConn::from_raw(&conn), &parsed, &AccountScope::All, None, None).expect("query");
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "t1");

        let projects = parse_query("folder:Projects");
        let no_threads =
            query_threads_read(&ReadConn::from_raw(&conn), &projects, &AccountScope::All, None, None).expect("query");
        assert!(no_threads.is_empty(), "tag labels must not match folder:");
    }

    // -- in: operator --

    #[test]
    fn in_inbox_finds_both_accounts() {
        let conn = setup_test_db();
        let parsed = parse_query("in:inbox");
        let threads = query_threads_read(&ReadConn::from_raw(&conn), &parsed, &AccountScope::All, None, None).expect("query");
        assert_eq!(threads.len(), 2);
    }

    #[test]
    fn in_starred_uses_thread_flag() {
        let conn = setup_test_db();
        let parsed = parse_query("in:starred");
        let threads = query_threads_read(&ReadConn::from_raw(&conn), &parsed, &AccountScope::All, None, None).expect("query");
        assert_eq!(threads.len(), 1);
        assert!(threads[0].is_starred);
    }

    #[test]
    fn is_starred_uses_thread_aggregate_with_message_date_filters() {
        let conn = setup_test_db();
        seed_split_thread_state_fixture(&conn);

        let mut parsed = parse_query("is:starred");
        parsed.after = Some(types::DateBound::after(2500));
        let threads = query_threads_read(&ReadConn::from_raw(&conn), &parsed, &AccountScope::All, None, None).expect("query");

        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "t3");
    }

    // -- is:tagged --

    #[test]
    fn label_group_rendered_fragment_uses_named_grains() {
        let thread_fragment =
            label_group_rendered_fragment(LabelGroupRenderGrain::Thread, "1=1");
        assert!(thread_fragment.contains("tl.account_id = t.account_id"));
        assert!(thread_fragment.contains("tl.thread_id = t.id"));
        assert!(thread_fragment.contains("pli_add.account_id = t.account_id"));

        let message_fragment =
            label_group_rendered_fragment(LabelGroupRenderGrain::Message, "1=1");
        assert!(message_fragment.contains("tl.account_id = m.account_id"));
        assert!(message_fragment.contains("tl.thread_id = m.thread_id"));
        assert!(message_fragment.contains("pli_add.account_id = m.account_id"));
    }

    #[test]
    fn is_tagged_finds_threads_with_labels() {
        let conn = setup_test_db();
        let parsed = parse_query("is:tagged");
        let threads = query_threads_read(&ReadConn::from_raw(&conn), &parsed, &AccountScope::All, None, None).expect("query");
        // Only t1 renders the Projects label group through its member tag.
        // t2's only membership is INBOX, which is a folder and does not count.
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "t1");
    }

    // -- label: operator --

    #[test]
    fn label_filters_by_label_group_name() {
        let conn = setup_test_db();
        let parsed = parse_query("label:Projects");
        let threads = query_threads_read(&ReadConn::from_raw(&conn), &parsed, &AccountScope::All, None, None).expect("query");
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "t1");
    }

    // -- has:contact --

    #[test]
    fn has_contact_filters_by_known_sender() {
        let conn = setup_test_db();
        let parsed = parse_query("has:contact");
        let threads = query_threads_read(&ReadConn::from_raw(&conn), &parsed, &AccountScope::All, None, None).expect("query");
        // Only m1's sender (sender@example.com) is in contacts.
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "t1");
    }

    // -- attachment type filtering --

    #[test]
    fn has_pdf_filters_attachments() {
        let conn = setup_test_db();
        let parsed = parse_query("has:pdf");
        let threads = query_threads_read(&ReadConn::from_raw(&conn), &parsed, &AccountScope::All, None, None).expect("query");
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "t1");
    }

    #[test]
    fn has_image_filters_attachments() {
        let conn = setup_test_db();
        let parsed = parse_query("has:image");
        let threads = query_threads_read(&ReadConn::from_raw(&conn), &parsed, &AccountScope::All, None, None).expect("query");
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "t1");
    }

    #[test]
    fn type_glob_pattern_matches() {
        let conn = setup_test_db();
        let parsed = parse_query("type:image/jpeg");
        let threads = query_threads_read(&ReadConn::from_raw(&conn), &parsed, &AccountScope::All, None, None).expect("query");
        assert_eq!(threads.len(), 1);
    }

    // -- from: with contact expansion --

    #[test]
    fn from_matches_contact_display_name() {
        let conn = setup_test_db();
        let parsed = parse_query("from:\"Friendly Sender\"");
        let threads = query_threads_read(&ReadConn::from_raw(&conn), &parsed, &AccountScope::All, None, None).expect("query");
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "t1");
    }

    // -- count_matching --

    #[test]
    fn count_matching_returns_correct_count() {
        let conn = setup_test_db();
        let parsed = parse_query("in:inbox");
        let count = count_matching_read(&ReadConn::from_raw(&conn), &parsed, &AccountScope::All).expect("count");
        assert_eq!(count, 2);
    }

    #[test]
    fn count_matching_forced_unread_uses_thread_aggregate() {
        let conn = setup_test_db();
        seed_split_thread_state_fixture(&conn);

        let mut parsed = parse_query("is:starred");
        parsed.after = Some(types::DateBound::after(2500));
        parsed.is_unread = Some(true);
        let count = count_matching_read(&ReadConn::from_raw(&conn), &parsed, &AccountScope::All).expect("count");

        assert_eq!(count, 1);
    }
}
