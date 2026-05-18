//! Phase 7-7 + 7-6: queries that ExtractRuntime uses to fan a
//! successful extraction out into one `WriterCommand::Index` per
//! message that references the just-indexed `content_hash`, plus the
//! 7-6 post-boot backfill query that drives the kick handler.
//!
//! Functions are synchronous; callers wrap them in the appropriate
//! typed DB state helper for async dispatch.

use rusqlite::params;
use std::collections::HashMap;

use crate::db::{ReadConn, WriteTarget};

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
    pub content_hash:  Option<crate::blob_hash::BlobHash>,
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
    conn: &ReadConn<'_>,
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

/// Phase 7 (M8 fix): SELECT existing row's status from
/// `attachment_extracted_text` for a given content_hash, scoped to the
/// current schema_version. The worker's pre-flight skip in extract.rs
/// uses this to detect already-resolved hashes and short-circuit
/// without re-running the extractor.
///
/// Returns `Some(status)` when a row exists at the current schema
/// version, `None` otherwise. The schema_version filter ensures a
/// pre-PreserveExisting Tantivy schema bump invalidates stale rows
/// without explicit cleanup.
pub fn select_extracted_text_status(
    conn: &ReadConn<'_>,
    content_hash: &crate::blob_hash::BlobHash,
    schema_version: i64,
) -> Result<Option<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT status FROM attachment_extracted_text \
             WHERE content_hash = ?1 AND schema_version = ?2",
        )
        .map_err(|e| format!("prepare select_extracted_text_status: {e}"))?;
    let mut rows = stmt
        .query_map(rusqlite::params![content_hash, schema_version], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|e| format!("query select_extracted_text_status: {e}"))?;
    match rows.next() {
        None => Ok(None),
        Some(Ok(status)) => Ok(Some(status)),
        Some(Err(e)) => Err(format!("row select_extracted_text_status: {e}")),
    }
}

/// Phase 7 (M8 fix): set `attachments.text_indexed_at` for every row
/// referencing the given content_hash whose value is currently NULL.
/// Idempotent: running twice with the same `at` is a no-op for rows
/// already set. Used by the worker's pre-flight skip (so backfill stops
/// re-emitting permanent skips) and by the post-extraction success
/// path (so backfill stops re-emitting newly-resolved rows).
pub fn mark_attachment_text_indexed(
    conn: &impl WriteTarget,
    content_hash: &crate::blob_hash::BlobHash,
    at: i64,
) -> Result<(), String> {
    conn.execute(
        "UPDATE attachments SET text_indexed_at = ?1 \
         WHERE content_hash = ?2 AND text_indexed_at IS NULL",
        rusqlite::params![at, content_hash],
    )
    .map_err(|e| format!("update text_indexed_at: {e}"))?;
    Ok(())
}

/// Phase 7 (M8 fix): upsert one row of `attachment_extracted_text`
/// per content_hash. The schema's PRIMARY KEY is content_hash, so
/// INSERT OR REPLACE is the correct shape - re-running with the same
/// hash overwrites in place.
pub fn upsert_extracted_text_row(
    conn: &impl WriteTarget,
    content_hash: &crate::blob_hash::BlobHash,
    mime_type: &str,
    extracted_text: Option<&str>,
    status: &str,
    extracted_at: i64,
    schema_version: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO attachment_extracted_text \
         (content_hash, mime_type, extracted_text, status, extracted_at, schema_version) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            content_hash,
            mime_type,
            extracted_text,
            status,
            extracted_at,
            schema_version
        ],
    )
    .map_err(|e| format!("upsert attachment_extracted_text: {e}"))?;
    Ok(())
}

/// Phase 7-9: reset every `attachments.text_indexed_at` to NULL and
/// truncate `attachment_extracted_text`. Run at the start of a Wipe
/// rebuild so the subsequent backfill kick re-extracts everything
/// against the new schema.
pub fn reset_extracted_text_for_rebuild(conn: &impl WriteTarget) -> Result<(), String> {
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
/// attachment rows whose bytes are still live in PackStore
/// (`attachment_blobs.tombstoned_at IS NULL`) but have no
/// extracted-text pointer yet (`text_indexed_at IS NULL`).
/// Uses the partial `idx_attachments_text_indexed_at` index.
///
/// Caller (`handle_backfill_kick`) iterates the result and enqueues
/// each into the installed `ExtractRuntime`. Idempotency is two-fold:
/// the `in_flight_hashes` dedupe inside the runtime, and the
/// status-aware skip inside the worker. A second kick after the first
/// finishes returns 0 rows.
pub fn find_unindexed_cached_attachments(
    conn: &ReadConn<'_>,
    limit: usize,
) -> Result<Vec<UnindexedCachedAttachmentRow>, String> {
    // Attachments roadmap Phase 3: join against `attachment_blobs` so
    // the backfill only enqueues rows whose bytes are still in the
    // pack store (tombstoned_at IS NULL). The flat-cache `cached_at`
    // filter retired here.
    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.message_id, a.account_id, a.content_hash \
             FROM attachments a \
             JOIN attachment_blobs b ON b.content_hash = a.content_hash \
             WHERE b.tombstoned_at IS NULL \
               AND a.text_indexed_at IS NULL \
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
                content_hash:  row.get::<_, Option<crate::blob_hash::BlobHash>>(3)?,
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
    conn: &ReadConn<'_>,
    content_hash: &crate::blob_hash::BlobHash,
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

/// Phase 8-2: drop `attachment_extracted_text` rows added since the
/// given cursor whose `content_hash` is no longer referenced by any
/// `attachments` row. Returns the number of rows deleted.
///
/// Bounded by the `extracted_at > cursor` predicate so on a 200 GB
/// mailbox we only inspect rows written since the last clean shutdown
/// rather than the entire extracted-text store. Defense-in-depth: the
/// content-hash join is by sub-select against `attachments`.
pub fn delete_extracted_text_orphans_since(
    conn: &impl WriteTarget,
    cursor: i64,
) -> Result<u64, String> {
    let n = conn
        .execute(
            "DELETE FROM attachment_extracted_text \
             WHERE extracted_at > ?1 \
               AND content_hash NOT IN ( \
                   SELECT content_hash FROM attachments \
                   WHERE content_hash IS NOT NULL \
               )",
            params![cursor],
        )
        .map_err(|e| format!("delete_extracted_text_orphans_since: {e}"))?;
    Ok(n as u64)
}

/// Fetch the scalar `messages` rows for a batch of `(account_id,
/// message_id)` pairs. Chunks transparently to stay under SQLite's
/// host-parameter cap.
pub fn select_messages_for_index_batch(
    conn: &ReadConn<'_>,
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
    conn: &ReadConn<'_>,
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
        // L10 note: the LEFT JOIN intentionally does not filter on
        // t.schema_version. Today's Wipe-only rebuild path truncates
        // attachment_extracted_text on schema bump, so every row is
        // at the current schema. When Phase 8 lands the true
        // PreserveExisting dual-index path (which keeps the legacy
        // table around during a rebuild), this JOIN must add
        // `AND t.schema_version = ?` keyed on search::INDEX_SCHEMA_VERSION
        // - until then the filter would be redundant. The function
        // signature would need a schema_version parameter; deferred
        // alongside the PreserveExisting work.
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
    use crate::blob_hash::BlobHash;
    use rusqlite::Connection;

    fn h(label: &[u8]) -> BlobHash {
        BlobHash::hash(label)
    }

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
                filename TEXT, mime_type TEXT, content_hash BLOB,\
                text_indexed_at INTEGER);\
             CREATE INDEX idx_attachments_content_hash ON attachments(content_hash);\
             CREATE INDEX idx_attachments_text_indexed_at ON attachments(text_indexed_at)\
                WHERE text_indexed_at IS NULL;\
             CREATE TABLE attachment_blobs (\
                content_hash BLOB PRIMARY KEY, pack_file_id INTEGER NOT NULL,\
                offset INTEGER NOT NULL, length INTEGER NOT NULL,\
                written_at INTEGER NOT NULL, last_read_at INTEGER,\
                tombstoned_at INTEGER);\
             CREATE TABLE attachment_extracted_text (\
                content_hash BLOB PRIMARY KEY, mime_type TEXT,\
                extracted_text TEXT, status TEXT NOT NULL,\
                extracted_at INTEGER NOT NULL, schema_version INTEGER NOT NULL);",
        )
        .expect("schema");
        conn
    }

    fn write(conn: &Connection) -> crate::db::WriteConn<'_> {
        crate::db::WriteConn::from_raw(conn)
    }

    fn read(conn: &Connection) -> ReadConn<'_> {
        ReadConn::from_raw(conn)
    }

    #[test]
    fn find_pairs_returns_distinct() {
        let conn = open_test_db();
        let hash_a = h(b"A");
        let hash_b = h(b"B");
        conn.execute(
            "INSERT INTO attachments (id, message_id, account_id, content_hash) \
             VALUES ('att1', 'msg1', 'acc1', ?1),\
                    ('att2', 'msg1', 'acc1', ?1),\
                    ('att3', 'msg2', 'acc1', ?1),\
                    ('att4', 'msg3', 'acc2', ?2)",
            rusqlite::params![hash_a, hash_b],
        )
        .expect("seed");
        let mut pairs = find_message_ids_referencing_content_hash(&read(&conn), &hash_a).expect("query");
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
        let pairs =
            find_message_ids_referencing_content_hash(&read(&conn), &h(b"nope")).expect("query");
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
        let rows = select_messages_for_index_batch(&read(&conn), &pairs).expect("query");
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
        let hash_a = h(b"A");
        let hash_b = h(b"B");
        conn.execute(
            "INSERT INTO attachments (id, message_id, account_id, filename, mime_type, content_hash) \
             VALUES ('att1', 'msg1', 'acc1', 'a.pdf', 'application/pdf', ?1),\
                    ('att2', 'msg1', 'acc1', 'b.txt', 'text/plain', ?2)",
            rusqlite::params![hash_a, hash_b],
        )
        .expect("seed atts");
        conn.execute(
            "INSERT INTO attachment_extracted_text \
             (content_hash, mime_type, extracted_text, status, extracted_at, schema_version) \
             VALUES (?1, 'application/pdf', 'pdf body', 'indexed', 1, 2),\
                    (?2, 'text/plain', NULL, 'skipped:opaque', 1, 2)",
            rusqlite::params![hash_a, hash_b],
        )
        .expect("seed extracted");
        let pairs = vec![("acc1".into(), "msg1".into())];
        let read = ReadConn::from_raw(&conn);
        let map = select_attachment_fragments_batch(&read, &pairs).expect("query");
        let frags = map.get(&("acc1".into(), "msg1".into())).expect("frags");
        assert_eq!(frags.len(), 2);
        let by_id: HashMap<&str, &AttachmentFragmentRow> =
            frags.iter().map(|f| (f.attachment_id.as_str(), f)).collect();
        assert_eq!(by_id["att1"].extracted_text, "pdf body");
        assert_eq!(by_id["att2"].extracted_text, "");
        assert_eq!(by_id["att2"].filename, "b.txt");
    }

    /// Helper: seed both an `attachments` row and a matching
    /// `attachment_blobs` row so the join in
    /// `find_unindexed_cached_attachments` resolves.
    fn seed_attachment_with_blob(
        conn: &Connection,
        att_id: &str,
        message_id: &str,
        account_id: &str,
        content_hash: Option<&BlobHash>,
        text_indexed_at: Option<i64>,
        tombstoned_at: Option<i64>,
    ) {
        conn.execute(
            "INSERT INTO attachments \
             (id, message_id, account_id, content_hash, text_indexed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![att_id, message_id, account_id, content_hash, text_indexed_at],
        )
        .expect("seed attachment");
        if let Some(hash) = content_hash {
            conn.execute(
                "INSERT OR IGNORE INTO attachment_blobs \
                 (content_hash, pack_file_id, offset, length, written_at, tombstoned_at) \
                 VALUES (?1, 0, 0, 0, 1, ?2)",
                rusqlite::params![hash, tombstoned_at],
            )
            .expect("seed blob");
        }
    }

    #[test]
    fn unindexed_cached_attachments_filters_correctly() {
        let conn = open_test_db();
        let hash_a = h(b"A");
        let hash_b = h(b"B");
        let hash_c = h(b"C");
        seed_attachment_with_blob(
            &conn,
            "live_unindexed",
            "msg1",
            "acc1",
            Some(&hash_a),
            None,
            None,
        );
        seed_attachment_with_blob(
            &conn,
            "live_indexed",
            "msg2",
            "acc1",
            Some(&hash_b),
            Some(200),
            None,
        );
        seed_attachment_with_blob(
            &conn,
            "tombstoned_unindexed",
            "msg3",
            "acc1",
            Some(&hash_c),
            None,
            Some(150),
        );
        seed_attachment_with_blob(
            &conn,
            "no_hash",
            "msg4",
            "acc1",
            None,
            None,
            None,
        );

        let rows = find_unindexed_cached_attachments(&read(&conn), 1000).expect("query");
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].attachment_id, "live_unindexed");
        assert_eq!(rows[0].content_hash, Some(hash_a));
    }

    #[test]
    fn unindexed_cached_attachments_respects_limit() {
        let conn = open_test_db();
        for i in 0..5 {
            let hash = h(format!("hash{i}").as_bytes());
            seed_attachment_with_blob(
                &conn,
                &format!("att{i}"),
                &format!("msg{i}"),
                "acc1",
                Some(&hash),
                None,
                None,
            );
        }
        let rows = find_unindexed_cached_attachments(&read(&conn), 3).expect("query");
        assert_eq!(rows.len(), 3);
    }

    /// Phase 8-2: rows with `extracted_at <= cursor` are NOT scanned;
    /// rows with `extracted_at > cursor` whose content_hash is no
    /// longer in `attachments` ARE deleted; rows with a live
    /// content_hash are preserved.
    #[test]
    fn delete_extracted_text_orphans_since_respects_cursor_and_live_hashes() {
        let conn = open_test_db();
        let hash_live = h(b"live");
        let hash_orphan_old = h(b"orphan-old");
        let hash_orphan_new = h(b"orphan-new");
        // One live attachment referencing the live hash.
        conn.execute(
            "INSERT INTO attachments (id, message_id, account_id, content_hash) \
             VALUES ('att-live', 'm1', 'a1', ?1)",
            rusqlite::params![hash_live],
        )
        .expect("seed attachment");
        // Three extracted_text rows:
        //  - live at extracted_at=200 (referenced, must NOT delete)
        //  - orphan-old at extracted_at=100 (orphan but pre-cursor, must NOT delete)
        //  - orphan-new at extracted_at=300 (orphan and post-cursor, MUST delete)
        for (hash, t) in &[
            (hash_live, 200),
            (hash_orphan_old, 100),
            (hash_orphan_new, 300),
        ] {
            conn.execute(
                "INSERT INTO attachment_extracted_text \
                 (content_hash, mime_type, extracted_text, status, extracted_at, schema_version) \
                 VALUES (?1, 'text/plain', '', 'indexed', ?2, 2)",
                rusqlite::params![hash, t],
            )
            .expect("seed extracted_text");
        }
        let cursor = 150;
        let dropped = delete_extracted_text_orphans_since(&write(&conn), cursor).expect("delete");
        assert_eq!(dropped, 1, "exactly the post-cursor orphan dropped");

        // Verify which rows remain.
        let mut remaining: Vec<BlobHash> = conn
            .prepare("SELECT content_hash FROM attachment_extracted_text")
            .expect("prepare")
            .query_map([], |r| r.get::<_, BlobHash>(0))
            .expect("query")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect");
        remaining.sort_by_key(|h| *h.as_bytes());
        let mut want = vec![hash_live, hash_orphan_old];
        want.sort_by_key(|h| *h.as_bytes());
        assert_eq!(remaining, want);
    }
}
