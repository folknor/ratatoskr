use super::DbState;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

// ── Types ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingOperation {
    pub id: String,
    pub account_id: String,
    pub operation_type: String,
    pub resource_id: String,
    pub params: String,
    pub status: String,
    pub retry_count: i64,
    pub max_retries: i64,
    pub next_retry_at: Option<i64>,
    pub created_at: i64,
    pub error_message: Option<String>,
}

fn row_to_pending_op(row: &rusqlite::Row<'_>) -> rusqlite::Result<PendingOperation> {
    Ok(PendingOperation {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        operation_type: row.get("operation_type")?,
        resource_id: row.get("resource_id")?,
        params: row.get("params")?,
        status: row.get("status")?,
        retry_count: row.get("retry_count")?,
        max_retries: row.get("max_retries")?,
        next_retry_at: row.get("next_retry_at")?,
        created_at: row.get("created_at")?,
        error_message: row.get("error_message")?,
    })
}

// ── Backoff schedule (seconds) ───────────────────────────────

const BACKOFF_SCHEDULE: &[i64] = &[60, 300, 900, 3600];

// ── Commands ─────────────────────────────────────────────────

pub async fn db_pending_ops_enqueue(
    db: &DbState,
    id: String,
    account_id: String,
    operation_type: String,
    resource_id: String,
    params_json: String,
) -> Result<(), String> {
    db
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO pending_operations (id, account_id, operation_type, resource_id, params, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'pending')",
                params![id, account_id, operation_type, resource_id, params_json],
            )
            .map_err(|e| format!("enqueue pending op: {e}"))?;
            Ok(())
        })
        .await
}

pub async fn db_pending_ops_get(
    db: &DbState,
    account_id: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<PendingOperation>, String> {
    db.with_conn(move |conn| {
        let lim = limit.unwrap_or(50);
        let now = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_secs(),
        )
        .map_err(|_| "current time exceeds i64 range".to_string())?;

        if let Some(ref aid) = account_id {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM pending_operations
                         WHERE account_id = ?1 AND status = 'pending'
                           AND (next_retry_at IS NULL OR next_retry_at <= ?2)
                         ORDER BY created_at ASC LIMIT ?3",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![aid, now, lim], row_to_pending_op)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM pending_operations
                         WHERE status = 'pending'
                           AND (next_retry_at IS NULL OR next_retry_at <= ?1)
                         ORDER BY created_at ASC LIMIT ?2",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![now, lim], row_to_pending_op)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        }
    })
    .await
}

pub async fn db_pending_ops_update_status(
    db: &DbState,
    id: String,
    status: String,
    error_message: Option<String>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE pending_operations SET status = ?1, error_message = ?2 WHERE id = ?3",
            params![status, error_message, id],
        )
        .map_err(|e| format!("update op status: {e}"))?;
        Ok(())
    })
    .await
}

pub async fn db_pending_ops_delete(db: &DbState, id: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute("DELETE FROM pending_operations WHERE id = ?1", params![id])
            .map_err(|e| format!("delete op: {e}"))?;
        Ok(())
    })
    .await
}

/// Cancel pending ops for a specific resource and operation type.
/// Used by undo to prevent retried actions from re-executing after undo.
/// Catches both 'pending' and 'executing' status — but cannot stop an
/// already in-flight provider call.
pub async fn db_pending_ops_cancel_for_resource(
    db: &DbState,
    account_id: String,
    resource_id: String,
    operation_type: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM pending_operations \
             WHERE account_id = ?1 AND resource_id = ?2 AND operation_type = ?3 \
               AND status IN ('pending', 'executing')",
            params![account_id, resource_id, operation_type],
        )
        .map_err(|e| format!("cancel pending op: {e}"))?;
        Ok(())
    })
    .await
}

pub async fn db_pending_ops_increment_retry(db: &DbState, id: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        let (retry_count, max_retries): (i64, i64) = conn
            .query_row(
                "SELECT retry_count, max_retries FROM pending_operations WHERE id = ?1",
                params![id],
                |row| Ok((row.get("retry_count")?, row.get("max_retries")?)),
            )
            .map_err(|e| format!("get retry info: {e}"))?;

        let new_count = retry_count + 1;
        if new_count >= max_retries {
            conn.execute(
                "UPDATE pending_operations SET status = 'failed', retry_count = ?1 WHERE id = ?2",
                params![new_count, id],
            )
            .map_err(|e| format!("mark failed: {e}"))?;
            return Ok(());
        }

        let retry_idx = usize::try_from(new_count - 1)
            .map_err(|_| format!("invalid retry count for pending op {id}: {new_count}"))?;
        let backoff_idx = std::cmp::min(retry_idx, BACKOFF_SCHEDULE.len() - 1);
        let delay_sec = BACKOFF_SCHEDULE[backoff_idx];
        let now = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_secs(),
        )
        .map_err(|_| "current time exceeds i64 range".to_string())?;
        let next_retry_at = now + delay_sec;

        conn.execute(
            "UPDATE pending_operations SET retry_count = ?1, next_retry_at = ?2 WHERE id = ?3",
            params![new_count, next_retry_at, id],
        )
        .map_err(|e| format!("increment retry: {e}"))?;
        Ok(())
    })
    .await
}

pub async fn db_pending_ops_count(db: &DbState, account_id: Option<String>) -> Result<i64, String> {
    db
        .with_conn(move |conn| {
            if let Some(ref aid) = account_id {
                conn.query_row(
                    "SELECT COUNT(*) AS cnt FROM pending_operations WHERE account_id = ?1 AND status = 'pending'",
                    params![aid],
                    |row| row.get("cnt"),
                )
                .map_err(|e| e.to_string())
            } else {
                conn.query_row(
                    "SELECT COUNT(*) AS cnt FROM pending_operations WHERE status = 'pending'",
                    [],
                    |row| row.get("cnt"),
                )
                .map_err(|e| e.to_string())
            }
        })
        .await
}

pub async fn db_pending_ops_failed_count(
    db: &DbState,
    account_id: Option<String>,
) -> Result<i64, String> {
    db
        .with_conn(move |conn| {
            if let Some(ref aid) = account_id {
                conn.query_row(
                    "SELECT COUNT(*) AS cnt FROM pending_operations WHERE account_id = ?1 AND status = 'failed'",
                    params![aid],
                    |row| row.get("cnt"),
                )
                .map_err(|e| e.to_string())
            } else {
                conn.query_row(
                    "SELECT COUNT(*) AS cnt FROM pending_operations WHERE status = 'failed'",
                    [],
                    |row| row.get("cnt"),
                )
                .map_err(|e| e.to_string())
            }
        })
        .await
}

pub async fn db_pending_ops_for_resource(
    db: &DbState,
    account_id: String,
    resource_id: String,
) -> Result<Vec<PendingOperation>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT * FROM pending_operations
                     WHERE account_id = ?1 AND resource_id = ?2 AND status = 'pending'
                     ORDER BY created_at ASC",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id, resource_id], row_to_pending_op)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_pending_ops_compact(
    db: &DbState,
    account_id: Option<String>,
) -> Result<i64, String> {
    db.with_conn(move |conn| compact_queue(conn, account_id.as_deref()))
        .await
}

pub async fn db_pending_ops_clear_failed(
    db: &DbState,
    account_id: Option<String>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        if let Some(ref aid) = account_id {
            conn.execute(
                "DELETE FROM pending_operations WHERE account_id = ?1 AND status = 'failed'",
                params![aid],
            )
            .map_err(|e| format!("clear failed: {e}"))?;
        } else {
            conn.execute("DELETE FROM pending_operations WHERE status = 'failed'", [])
                .map_err(|e| format!("clear failed: {e}"))?;
        }
        Ok(())
    })
    .await
}

/// Reset any operations stuck in 'executing' back to 'pending'.
/// Called at startup to recover from crash/forced quit.
pub async fn db_pending_ops_recover_executing(db: &DbState) -> Result<i64, String> {
    db.with_conn(move |conn| {
        let count = conn
            .execute(
                "UPDATE pending_operations SET status = 'pending' WHERE status = 'executing'",
                [],
            )
            .map_err(|e| format!("recover executing ops: {e}"))?;
        let count = i64::try_from(count).map_err(|_| "row count exceeds i64 range".to_string())?;
        if count > 0 {
            log::warn!("[pending_ops] Recovered {count} stranded executing operations");
        }
        Ok(count)
    })
    .await
}

pub async fn db_pending_ops_retry_failed(
    db: &DbState,
    account_id: Option<String>,
) -> Result<(), String> {
    db
        .with_conn(move |conn| {
            if let Some(ref aid) = account_id {
                conn.execute(
                    "UPDATE pending_operations SET status = 'pending', retry_count = 0, next_retry_at = NULL, error_message = NULL
                     WHERE account_id = ?1 AND status = 'failed'",
                    params![aid],
                )
                .map_err(|e| format!("retry failed: {e}"))?;
            } else {
                conn.execute(
                    "UPDATE pending_operations SET status = 'pending', retry_count = 0, next_retry_at = NULL, error_message = NULL
                     WHERE status = 'failed'",
                    [],
                )
                .map_err(|e| format!("retry failed: {e}"))?;
            }
            Ok(())
        })
        .await
}

// ── Queue compaction logic ───────────────────────────────────
//
// Groups pending ops by resource, then:
// 1. Cancels toggle pairs (star true+false, markRead true+false)
// 2. Cancels matching addLabel/removeLabel for same label
// 3. Collapses sequential moveToFolder ops (keeps only the latest)

#[allow(clippy::too_many_lines)]
fn compact_queue(conn: &Connection, account_id: Option<&str>) -> Result<i64, String> {
    // Fetch all pending ops, optionally filtered by account
    let ops: Vec<PendingOperation> = if let Some(aid) = account_id {
        let mut stmt = conn
            .prepare(
                "SELECT * FROM pending_operations WHERE status = 'pending' AND account_id = ?1
                 ORDER BY created_at ASC",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![aid], row_to_pending_op)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT * FROM pending_operations WHERE status = 'pending'
                 ORDER BY created_at ASC",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map([], row_to_pending_op)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };

    // Group by account_id:resource_id
    let mut by_resource: std::collections::HashMap<String, Vec<&PendingOperation>> =
        std::collections::HashMap::new();
    for op in &ops {
        let key = format!("{}:{}", op.account_id, op.resource_id);
        by_resource.entry(key).or_default().push(op);
    }

    let mut to_delete: Vec<&str> = Vec::new();

    for resource_ops in by_resource.values() {
        // 1. Cancel toggle pairs: star(true)+star(false), markRead(true)+markRead(false)
        for toggle_type in &["star", "markRead"] {
            let toggle_ops: Vec<&&PendingOperation> = resource_ops
                .iter()
                .filter(|o| o.operation_type == *toggle_type)
                .collect();

            let mut i = 0;
            while i + 1 < toggle_ops.len() {
                let params_a: serde_json::Value =
                    serde_json::from_str(&toggle_ops[i].params).unwrap_or_default();
                let params_b: serde_json::Value =
                    serde_json::from_str(&toggle_ops[i + 1].params).unwrap_or_default();

                let opposite = match *toggle_type {
                    "star" => params_a.get("starred") != params_b.get("starred"),
                    "markRead" => params_a.get("read") != params_b.get("read"),
                    _ => false,
                };

                if opposite {
                    to_delete.push(&toggle_ops[i].id);
                    to_delete.push(&toggle_ops[i + 1].id);
                    i += 2;
                } else {
                    i += 1;
                }
            }
        }

        // 2. Cancel matching addLabel/removeLabel for same label on same resource
        let add_label_ops: Vec<&&PendingOperation> = resource_ops
            .iter()
            .filter(|o| o.operation_type == "addLabel")
            .collect();
        let mut remove_label_ops: Vec<&&PendingOperation> = resource_ops
            .iter()
            .filter(|o| o.operation_type == "removeLabel")
            .collect();

        for add_op in &add_label_ops {
            let add_params: serde_json::Value =
                serde_json::from_str(&add_op.params).unwrap_or_default();
            let add_label_id = add_params.get("labelId");

            if let Some(match_idx) = remove_label_ops.iter().position(|r| {
                let r_params: serde_json::Value =
                    serde_json::from_str(&r.params).unwrap_or_default();
                r_params.get("labelId") == add_label_id
            }) {
                to_delete.push(&add_op.id);
                to_delete.push(&remove_label_ops[match_idx].id);
                remove_label_ops.remove(match_idx);
            }
        }

        // 3. Collapse sequential moveToFolder — keep only the latest
        let move_ops: Vec<&&PendingOperation> = resource_ops
            .iter()
            .filter(|o| o.operation_type == "moveToFolder")
            .collect();
        if move_ops.len() > 1 {
            for op in &move_ops[..move_ops.len() - 1] {
                to_delete.push(&op.id);
            }
        }
    }

    // Delete compacted ops
    let deleted =
        i64::try_from(to_delete.len()).map_err(|_| "too many operations to delete".to_string())?;
    if !to_delete.is_empty() {
        // Batch delete in chunks to stay within SQLite parameter limits
        for chunk in to_delete.chunks(100) {
            let placeholders: String = chunk
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 1))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!("DELETE FROM pending_operations WHERE id IN ({placeholders})");
            let param_values: Vec<Box<dyn rusqlite::types::ToSql>> = chunk
                .iter()
                .map(|id| Box::new(id.to_string()) as Box<dyn rusqlite::types::ToSql>)
                .collect();
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(AsRef::as_ref).collect();
            conn.execute(&sql, param_refs.as_slice())
                .map_err(|e| format!("compact delete: {e}"))?;
        }
    }

    Ok(deleted)
}
