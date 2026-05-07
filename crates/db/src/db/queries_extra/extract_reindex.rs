//! Phase 7-7 + 7-6: queries that ExtractRuntime uses to fan a
//! successful extraction out into one `WriterCommand::Index` per
//! message that references the just-indexed `content_hash`, plus the
//! 7-6 post-boot backfill query that drives the kick handler.
//!
//! All functions take `&Connection` (sync); callers wrap them in
//! `ReadDbState::with_conn(...)` for async dispatch.

use rusqlite::{Connection, params};
use std::collections::HashMap;

/// `SQLITE_LIMIT_VARIABLE_NUMBER` historically defaulted to 999. Modern
/// builds raise it but we stay portable. Each pair binds two params, so a
/// 256-pair chunk uses 512 placeholders well under the floor.
const MAX_PAIRS_PER_CHUNK: usize = 256;

/// Scalar `messages` row fields needed to build a `SearchDocument`. The
/// caller (in the `service` crate) maps this into `search::SearchDocument`.
#[derive(Debug, Clone)]
pub struct MessageIndexRow {
    pub message_id:   String,
    pub account_id:   String,
    pub thread_id:    String,
    pub subject:      Option<String>,
    pub from_name:    Option<String>,
    pub from_address: Option<String>,
    pub to_addresses: Option<String>,
    pub snippet:      Option<String>,
    pub date:         i64,
    pub is_read:      bool,
    pub is_starred:   bool,
}

/// Per-attachment fields needed to build a `search::AttachmentDocFragment`.
/// `extracted_text` is empty when no `attachment_extracted_text` row exists
/// or its status is not `'indexed'` (skipped/failed rows have NULL text).
#[derive(Debug, Clone)]
pub struct AttachmentFragmentRow {
    pub attachment_id:  String,
    pub message_id:     String,
    pub account_id:     String,
    pub filename:       String,
    pub mime_type:      String,
    pub extracted_text: String,
}

/// Phase 7-6: backfill row. Identifies one cached-but-unindexed
/// attachment for the post-boot kick to enqueue. `content_hash` is
/// `Option` because the `attachments` schema allows NULL there;
/// callers skip rows with no hash (the worker can't extract without
/// one).
#[derive(Debug, Clone)]
pub struct UnindexedCachedAttachmentRow {
    pub attachment_id: String,
    pub message_id:    String,
    pub account_id:    String,
    pub content_hash:  Option<String>,
}

/// Phase 7-9: enumerate every message identity for the index-rebuild
/// task. Returns `(account_id, id)` pairs ordered by `account_id, id`
/// for deterministic chunking. Does not include local_drafts; those
/// are re-emitted by a separate query.
///
/// Memory budget: ~24 bytes per row plus String overhead. A 300k-row
/// mailbox is ~10 MB - acceptable for the rebuild path. If we ever
/// need to scale past that, swap to a paginated query reading from a
/// cursor.
pub fn select_all_message_ids_for_rebuild(
    conn: &Connection,
) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT account_id, id FROM messages \
             ORDER BY account_id, id",
        )
        .map_err(|e| format!("prepare select_all_message_ids_for_rebuild: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("query select_all_message_ids_for_rebuild: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("row select_all_message_ids_for_rebuild: {e}"))?);
    }
    Ok(out)
}

/// Phase 7-9: reset every `attachments.text_indexed_at` to NULL and
/// truncate `attachment_extracted_text`. Run at the start of a Wipe
/// rebuild so the subsequent backfill kick re-extracts everything
/// against the new schema.
pub fn reset_extracted_text_for_rebuild(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "UPDATE attachments SET text_indexed_at = NULL WHERE text_indexed_at IS NOT NULL",
        [],
    )
    .map_err(|e| format!("UPDATE attachments.text_indexed_at: {e}"))?;
    conn.execute("DELETE FROM attachment_extracted_text", [])
        .map_err(|e| format!("DELETE attachment_extracted_text: {e}"))?;
    Ok(())
}

/// Phase 7-6: post-boot backfill query. Returns up to `limit`
/// attachment rows that are cached on disk (`cached_at IS NOT NULL`)
/// but have no extracted-text pointer yet (`text_indexed_at IS NULL`).
/// Uses the partial `idx_attachments_text_indexed_at` index.
///
/// Caller (`handle_backfill_kick`) iterates the result and enqueues
/// each into the installed `ExtractRuntime`. Idempotency is two-fold:
/// the `in_flight_hashes` dedupe inside the runtime, and the
/// status-aware skip inside the worker. A second kick after the first
/// finishes returns 0 rows.
pub fn find_unindexed_cached_attachments(
    conn: &Connection,
    limit: usize,
) -> Result<Vec<UnindexedCachedAttachmentRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, message_id, account_id, content_hash \
             FROM attachments \
             WHERE cached_at IS NOT NULL AND text_indexed_at IS NULL \
             LIMIT ?1",
        )
        .map_err(|e| format!("prepare find_unindexed_cached_attachments: {e}"))?;
    let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
    let rows = stmt
        .query_map(params![limit_i64], |row| {
            Ok(UnindexedCachedAttachmentRow {
                attachment_id: row.get::<_, String>(0)?,
                message_id:    row.get::<_, String>(1)?,
                account_id:    row.get::<_, String>(2)?,
                content_hash:  row.get::<_, Option<String>>(3)?,
            })
        })
        .map_err(|e| format!("query find_unindexed_cached_attachments: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("row find_unindexed_cached_attachments: {e}"))?);
    }
    Ok(out)
}

/// Distinct `(account_id, message_id)` pairs whose `attachments` rows
/// reference the given `content_hash`. Uses `idx_attachments_content_hash`.
pub fn find_message_ids_referencing_content_hash(
    conn: &Connection,
    content_hash: &str,
) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT account_id, message_id FROM attachments \
             WHERE content_hash = ?1",
        )
        .map_err(|e| format!("prepare find_message_ids_referencing_content_hash: {e}"))?;
    let rows = stmt
        .query_map(params![content_hash], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("query find_message_ids_referencing_content_hash: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("row find_message_ids_referencing_content_hash: {e}"))?);
    }
    Ok(out)
}

/// Fetch the scalar `messages` rows for a batch of `(account_id,
/// message_id)` pairs. Chunks transparently to stay under SQLite's
/// host-parameter cap.
pub fn select_messages_for_index_batch(
    conn: &Connection,
    pairs: &[(String, String)],
) -> Result<Vec<MessageIndexRow>, String> {
    if pairs.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(pairs.len());
    for chunk in pairs.chunks(MAX_PAIRS_PER_CHUNK) {
        let placeholders: Vec<String> = (0..chunk.len())
            .map(|i| format!("(?{}, ?{})", i * 2 + 1, i * 2 + 2))
            .collect();
        let sql = format!(
            "SELECT id, account_id, thread_id, subject, from_name, from_address, \
                    to_addresses, snippet, date, is_read, is_starred \
             FROM messages \
             WHERE (account_id, id) IN (VALUES {})",
            placeholders.join(", "),
        );
        let mut params_vec: Vec<&dyn rusqlite::types::ToSql> = Vec::with_capacity(chunk.len() * 2);
        for (acc, mid) in chunk {
            params_vec.push(acc);
            params_vec.push(mid);
        }
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("prepare select_messages_for_index_batch: {e}"))?;
        let rows = stmt
            .query_map(params_vec.as_slice(), |row| {
                Ok(MessageIndexRow {
                    message_id:   row.get::<_, String>(0)?,
                    account_id:   row.get::<_, String>(1)?,
                    thread_id:    row.get::<_, String>(2)?,
                    subject:      row.get::<_, Option<String>>(3)?,
                    from_name:    row.get::<_, Option<String>>(4)?,
                    from_address: row.get::<_, Option<String>>(5)?,
                    to_addresses: row.get::<_, Option<String>>(6)?,
                    snippet:      row.get::<_, Option<String>>(7)?,
                    date:         row.get::<_, i64>(8)?,
                    is_read:      row.get::<_, i64>(9)? != 0,
                    is_starred:   row.get::<_, i64>(10)? != 0,
                })
            })
            .map_err(|e| format!("query select_messages_for_index_batch: {e}"))?;
        for r in rows {
            out.push(r.map_err(|e| format!("row select_messages_for_index_batch: {e}"))?);
        }
    }
    Ok(out)
}

/// Fetch per-attachment fragments for a batch of `(account_id,
/// message_id)` pairs. LEFT JOIN against `attachment_extracted_text` so
/// attachments without an `'indexed'` row still appear with empty
/// `extracted_text` (they remain in the search doc's per-attachment
/// metadata but contribute no full-text content).
///
/// Returned map is keyed by `(account_id, message_id)`. Within each
/// vector, attachments are ordered by `attachments.rowid` ASC for
/// deterministic doc shape across runs.
pub fn select_attachment_fragments_batch(
    conn: &Connection,
    pairs: &[(String, String)],
) -> Result<HashMap<(String, String), Vec<AttachmentFragmentRow>>, String> {
    let mut out: HashMap<(String, String), Vec<AttachmentFragmentRow>> = HashMap::new();
    if pairs.is_empty() {
        return Ok(out);
    }
    for chunk in pairs.chunks(MAX_PAIRS_PER_CHUNK) {
        let placeholders: Vec<String> = (0..chunk.len())
            .map(|i| format!("(?{}, ?{})", i * 2 + 1, i * 2 + 2))
            .collect();
        let sql = format!(
            "SELECT a.id, a.message_id, a.account_id, a.filename, a.mime_type, \
                    t.extracted_text, t.status \
             FROM attachments a \
             LEFT JOIN attachment_extracted_text t ON t.content_hash = a.content_hash \
             WHERE (a.account_id, a.message_id) IN (VALUES {}) \
             ORDER BY a.account_id, a.message_id, a.rowid",
            placeholders.join(", "),
        );
        let mut params_vec: Vec<&dyn rusqlite::types::ToSql> = Vec::with_capacity(chunk.len() * 2);
        for (acc, mid) in chunk {
            params_vec.push(acc);
            params_vec.push(mid);
        }
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("prepare select_attachment_fragments_batch: {e}"))?;
        let rows = stmt
            .query_map(params_vec.as_slice(), |row| {
                let attachment_id  = row.get::<_, String>(0)?;
                let message_id     = row.get::<_, String>(1)?;
                let account_id     = row.get::<_, String>(2)?;
                let filename       = row.get::<_, Option<String>>(3)?.unwrap_or_default();
                let mime_type      = row.get::<_, Option<String>>(4)?.unwrap_or_default();
                let extracted_text = row.get::<_, Option<String>>(5)?;
                let status         = row.get::<_, Option<String>>(6)?;
                // Only carry text into the index when the row was
                // successfully indexed. Skipped/failed rows leave the
                // attachment present in the doc (so filename/mime are
                // searchable) but contribute no full-text segment.
                let text = match (extracted_text, status.as_deref()) {
                    (Some(t), Some("indexed")) => t,
                    _ => String::new(),
                };
                Ok(AttachmentFragmentRow {
                    attachment_id,
                    message_id,
                    account_id,
                    filename,
                    mime_type,
                    extracted_text: text,
                })
            })
            .map_err(|e| format!("query select_attachment_fragments_batch: {e}"))?;
        for r in rows {
            let frag = r.map_err(|e| format!("row select_attachment_fragments_batch: {e}"))?;
            out.entry((frag.account_id.clone(), frag.message_id.clone()))
                .or_default()
                .push(frag);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn open_test_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open_in_memory");
        conn.execute_batch(
            "CREATE TABLE messages (\
                id TEXT NOT NULL, account_id TEXT NOT NULL, thread_id TEXT NOT NULL,\
                from_address TEXT, from_name TEXT, to_addresses TEXT,\
                subject TEXT, snippet TEXT, date INTEGER NOT NULL,\
                is_read INTEGER DEFAULT 0, is_starred INTEGER DEFAULT 0,\
                PRIMARY KEY (account_id, id));\
             CREATE TABLE attachments (\
                id TEXT PRIMARY KEY, message_id TEXT NOT NULL, account_id TEXT NOT NULL,\
                filename TEXT, mime_type TEXT, content_hash TEXT,\
                cached_at INTEGER, text_indexed_at INTEGER);\
             CREATE INDEX idx_attachments_content_hash ON attachments(content_hash);\
             CREATE INDEX idx_attachments_text_indexed_at ON attachments(text_indexed_at)\
                WHERE cached_at IS NOT NULL AND text_indexed_at IS NULL;\
             CREATE TABLE attachment_extracted_text (\
                content_hash TEXT PRIMARY KEY, mime_type TEXT,\
                extracted_text TEXT, status TEXT NOT NULL,\
                extracted_at INTEGER NOT NULL, schema_version INTEGER NOT NULL);",
        )
        .expect("schema");
        conn
    }

    #[test]
    fn find_pairs_returns_distinct() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO attachments (id, message_id, account_id, content_hash) \
             VALUES ('att1', 'msg1', 'acc1', 'hashA'),\
                    ('att2', 'msg1', 'acc1', 'hashA'),\
                    ('att3', 'msg2', 'acc1', 'hashA'),\
                    ('att4', 'msg3', 'acc2', 'hashB')",
            [],
        )
        .expect("seed");
        let mut pairs = find_message_ids_referencing_content_hash(&conn, "hashA").expect("query");
        pairs.sort();
        assert_eq!(
            pairs,
            vec![
                ("acc1".into(), "msg1".into()),
                ("acc1".into(), "msg2".into()),
            ]
        );
    }

    #[test]
    fn empty_hash_returns_empty() {
        let conn = open_test_db();
        let pairs = find_message_ids_referencing_content_hash(&conn, "nope").expect("query");
        assert!(pairs.is_empty());
    }

    #[test]
    fn messages_batch_round_trips_scalar_fields() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO messages (id, account_id, thread_id, subject, from_name,\
                from_address, to_addresses, snippet, date, is_read, is_starred) \
             VALUES ('msg1', 'acc1', 'thr1', 'Hi', 'Alice', 'a@x', 'b@x', 'snip', 100, 1, 0)",
            [],
        )
        .expect("seed");
        let pairs = vec![("acc1".into(), "msg1".into())];
        let rows = select_messages_for_index_batch(&conn, &pairs).expect("query");
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.message_id, "msg1");
        assert_eq!(r.subject.as_deref(), Some("Hi"));
        assert_eq!(r.from_name.as_deref(), Some("Alice"));
        assert!(r.is_read);
        assert!(!r.is_starred);
    }

    #[test]
    fn fragments_batch_carries_text_only_when_indexed() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO attachments (id, message_id, account_id, filename, mime_type, content_hash) \
             VALUES ('att1', 'msg1', 'acc1', 'a.pdf', 'application/pdf', 'hashA'),\
                    ('att2', 'msg1', 'acc1', 'b.txt', 'text/plain', 'hashB')",
            [],
        )
        .expect("seed atts");
        conn.execute(
            "INSERT INTO attachment_extracted_text \
             (content_hash, mime_type, extracted_text, status, extracted_at, schema_version) \
             VALUES ('hashA', 'application/pdf', 'pdf body', 'indexed', 1, 2),\
                    ('hashB', 'text/plain', NULL, 'skipped:opaque', 1, 2)",
            [],
        )
        .expect("seed extracted");
        let pairs = vec![("acc1".into(), "msg1".into())];
        let map = select_attachment_fragments_batch(&conn, &pairs).expect("query");
        let frags = map.get(&("acc1".into(), "msg1".into())).expect("frags");
        assert_eq!(frags.len(), 2);
        let by_id: HashMap<&str, &AttachmentFragmentRow> =
            frags.iter().map(|f| (f.attachment_id.as_str(), f)).collect();
        assert_eq!(by_id["att1"].extracted_text, "pdf body");
        assert_eq!(by_id["att2"].extracted_text, "");
        assert_eq!(by_id["att2"].filename, "b.txt");
    }

    #[test]
    fn unindexed_cached_attachments_filters_correctly() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO attachments \
             (id, message_id, account_id, content_hash, cached_at, text_indexed_at) \
             VALUES \
             ('cached_unindexed', 'msg1', 'acc1', 'hashA', 100, NULL),\
             ('cached_indexed',   'msg2', 'acc1', 'hashB', 100, 200),\
             ('evicted_unindexed','msg3', 'acc1', 'hashC', NULL, NULL),\
             ('cached_no_hash',   'msg4', 'acc1', NULL,    100, NULL)",
            [],
        )
        .expect("seed");

        let mut rows = find_unindexed_cached_attachments(&conn, 1000).expect("query");
        rows.sort_by(|a, b| a.attachment_id.cmp(&b.attachment_id));
        assert_eq!(rows.len(), 2, "{rows:?}");
        // cached + unindexed (with hash) - the canonical backfill row.
        assert_eq!(rows[0].attachment_id, "cached_no_hash");
        assert!(rows[0].content_hash.is_none());
        assert_eq!(rows[1].attachment_id, "cached_unindexed");
        assert_eq!(rows[1].content_hash.as_deref(), Some("hashA"));
    }

    #[test]
    fn unindexed_cached_attachments_respects_limit() {
        let conn = open_test_db();
        for i in 0..5 {
            conn.execute(
                "INSERT INTO attachments \
                 (id, message_id, account_id, content_hash, cached_at, text_indexed_at) \
                 VALUES (?1, ?2, 'acc1', ?3, 100, NULL)",
                rusqlite::params![
                    format!("att{i}"),
                    format!("msg{i}"),
                    format!("hash{i}"),
                ],
            )
            .expect("seed");
        }
        let rows = find_unindexed_cached_attachments(&conn, 3).expect("query");
        assert_eq!(rows.len(), 3);
    }
}
