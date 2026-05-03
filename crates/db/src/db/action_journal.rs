//! Action-journal write helpers (Phase 2 task 8).
//!
//! The journal tables `action_jobs` and `action_job_ops` are defined in
//! `schema/12_actions.sql`. This module exposes the narrow `pub(crate)`
//! helpers that `service-state::WriteDbState` re-exposes through
//! `ActionDbWrite` (Phase 2 task 7) and that Phase 1.5's boot recovery
//! path (`recover_on_boot_db_only`) consumes.
//!
//! Scope today: boot recovery + status query. The worker-side leasing
//! helpers (`lease_next_ready_op`, `mark_op_complete`,
//! `update_job_status_on_completion`) and the handler-side insert helpers
//! land with task 9 (the action.execute_plan handler + worker) so the
//! shape of those helpers can be designed against a concrete consumer.

use rusqlite::{Connection, OptionalExtension, params};

/// Status of an action job, as reported by `query_job_status`.
///
/// Mirrors the `status` enum in the journal SQL. The IPC reconciliation
/// path (Phase 2 plan scope item 18d's `action.job_status`) maps this
/// to `JobStatusResponse::Journaled { status, summary }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Leased,
    Executing,
    Completed,
    Partial,
    Failed,
}

impl JobStatus {
    fn from_sql(value: &str) -> Result<Self, String> {
        match value {
            "queued" => Ok(Self::Queued),
            "leased" => Ok(Self::Leased),
            "executing" => Ok(Self::Executing),
            "completed" => Ok(Self::Completed),
            "partial" => Ok(Self::Partial),
            "failed" => Ok(Self::Failed),
            other => Err(format!("unknown action_jobs.status value: {other}")),
        }
    }
}

/// Reset every `leased` / `executing` row in both `action_jobs` and
/// `action_job_ops` back to `queued` / `pending`, clearing the lease
/// owner and expiry. Returns `(jobs_reset, ops_reset)`.
///
/// Called from the Service boot sequence (Phase 1.5's
/// `recover_on_boot_db_only`) before the action worker starts. The
/// invariant is "any lease that exists at boot is stale" - the worker
/// instance UUID it points at belongs to a previous Service
/// incarnation that's already gone. The worker will re-lease the
/// reset rows on its first scheduling pass.
///
/// Idempotent: a second call after the first returns `(0, 0)`.
pub fn recover_stale_leases(conn: &Connection) -> Result<(usize, usize), String> {
    let jobs_reset = conn
        .execute(
            "UPDATE action_jobs \
             SET status = 'queued', \
                 lease_owner = NULL, \
                 lease_expires_at = NULL, \
                 updated_at = unixepoch() \
             WHERE status IN ('leased', 'executing')",
            [],
        )
        .map_err(|e| format!("recover_stale_leases jobs: {e}"))?;
    let ops_reset = conn
        .execute(
            "UPDATE action_job_ops \
             SET status = 'pending', \
                 lease_owner = NULL, \
                 lease_expires_at = NULL \
             WHERE status IN ('leased', 'executing')",
            [],
        )
        .map_err(|e| format!("recover_stale_leases ops: {e}"))?;
    Ok((jobs_reset, ops_reset))
}

/// Snapshot of a journaled action job, returned by `query_job_status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobStatusSnapshot {
    pub status: JobStatus,
    /// Serialized PlanSummary / SendSummary / etc., populated on
    /// terminal status. `None` while the job is still in flight.
    pub summary: Option<Vec<u8>>,
}

/// Look up the current status (and summary) of a journaled action job.
///
/// Backs the `action.job_status` IPC method (Phase 2 plan scope item
/// 18d): the UI calls this after a `boot.ready` post-respawn for every
/// `AckUnknown` plan to reconcile to either `Acked` (Journaled) or
/// `RollBack` (NotFound).
///
/// Returns `Ok(None)` if no job with `job_id` exists. The 16-byte
/// `job_id` is a UUIDv7 in raw bytes.
pub fn query_job_status(
    conn: &Connection,
    job_id: &[u8; 16],
) -> Result<Option<JobStatusSnapshot>, String> {
    let row: Option<(String, Option<Vec<u8>>)> = conn
        .query_row(
            "SELECT status, summary FROM action_jobs WHERE job_id = ?1",
            params![job_id.as_slice()],
            |row| Ok((row.get::<_, String>("status")?, row.get::<_, Option<Vec<u8>>>("summary")?)),
        )
        .optional()
        .map_err(|e| format!("query_job_status: {e}"))?;
    let Some((status_str, summary)) = row else {
        return Ok(None);
    };
    Ok(Some(JobStatusSnapshot {
        status: JobStatus::from_sql(&status_str)?,
        summary,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations;

    fn fresh_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory");
        // pragmas the production path applies; foreign_keys=ON is
        // load-bearing here because action_job_ops.job_id references
        // action_jobs.job_id ON DELETE CASCADE.
        conn.execute_batch(
            "PRAGMA foreign_keys = ON; \
             PRAGMA journal_mode = WAL;",
        )
        .expect("apply pragmas");
        migrations::run_all(&conn).expect("apply migrations");
        // Seed a single account so action_jobs FK constraint is satisfied.
        conn.execute(
            "INSERT INTO accounts (id, email, provider) VALUES (?1, ?2, ?3)",
            params!["acc-1", "[email protected]", "gmail_api"],
        )
        .expect("seed account");
        conn
    }

    fn insert_test_job(
        conn: &Connection,
        job_id: &[u8; 16],
        status: &str,
    ) {
        conn.execute(
            "INSERT INTO action_jobs (\
                 job_id, kind, account_id, status, quiet, payload, \
                 created_at, updated_at\
             ) VALUES (?1, 'mail_plan', 'acc-1', ?2, 0, X'', unixepoch(), unixepoch())",
            params![job_id.as_slice(), status],
        )
        .expect("insert action_jobs");
    }

    fn insert_test_op(
        conn: &Connection,
        job_id: &[u8; 16],
        operation_id: u32,
        status: &str,
    ) {
        conn.execute(
            "INSERT INTO action_job_ops (\
                 job_id, operation_id, ordinal, thread_id, operation, status\
             ) VALUES (?1, ?2, ?2, 'thr-1', X'', ?3)",
            params![job_id.as_slice(), operation_id, status],
        )
        .expect("insert action_job_ops");
    }

    #[test]
    fn migration_applies_cleanly() {
        let _conn = fresh_db();
    }

    #[test]
    fn check_constraint_rejects_unknown_kind() {
        let conn = fresh_db();
        let job_id = [0u8; 16];
        let result = conn.execute(
            "INSERT INTO action_jobs (\
                 job_id, kind, account_id, status, quiet, payload, \
                 created_at, updated_at\
             ) VALUES (?1, 'unknown', 'acc-1', 'queued', 0, X'', 0, 0)",
            params![job_id.as_slice()],
        );
        assert!(result.is_err(), "kind CHECK constraint must reject unknowns");
    }

    #[test]
    fn check_constraint_rejects_unknown_status() {
        let conn = fresh_db();
        let job_id = [0u8; 16];
        let result = conn.execute(
            "INSERT INTO action_jobs (\
                 job_id, kind, account_id, status, quiet, payload, \
                 created_at, updated_at\
             ) VALUES (?1, 'mail_plan', 'acc-1', 'wat', 0, X'', 0, 0)",
            params![job_id.as_slice()],
        );
        assert!(
            result.is_err(),
            "action_jobs.status CHECK constraint must reject unknowns",
        );
    }

    #[test]
    fn account_fk_cascades_on_delete() {
        let conn = fresh_db();
        let job_id = [1u8; 16];
        insert_test_job(&conn, &job_id, "queued");
        insert_test_op(&conn, &job_id, 0, "pending");

        conn.execute("DELETE FROM accounts WHERE id = 'acc-1'", [])
            .expect("delete account");

        let job_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM action_jobs", [], |row| row.get(0))
            .expect("count jobs");
        assert_eq!(job_count, 0, "account delete cascades to action_jobs");

        let op_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM action_job_ops", [], |row| row.get(0))
            .expect("count ops");
        assert_eq!(op_count, 0, "action_jobs delete cascades to action_job_ops");
    }

    #[test]
    fn recover_stale_leases_resets_active_jobs_and_ops() {
        let conn = fresh_db();
        let job_a = [0xAA; 16];
        let job_b = [0xBB; 16];
        let job_c = [0xCC; 16];

        insert_test_job(&conn, &job_a, "leased");
        insert_test_job(&conn, &job_b, "executing");
        insert_test_job(&conn, &job_c, "completed");

        insert_test_op(&conn, &job_a, 0, "leased");
        insert_test_op(&conn, &job_a, 1, "executing");
        insert_test_op(&conn, &job_a, 2, "done");
        insert_test_op(&conn, &job_b, 0, "executing");

        let (jobs_reset, ops_reset) = recover_stale_leases(&conn).expect("recover");
        assert_eq!(jobs_reset, 2, "two non-terminal jobs should reset");
        assert_eq!(ops_reset, 3, "three non-terminal ops should reset");

        // `completed` job is unchanged.
        let status: String = conn
            .query_row(
                "SELECT status FROM action_jobs WHERE job_id = ?1",
                params![job_c.as_slice()],
                |row| row.get(0),
            )
            .expect("query completed job");
        assert_eq!(status, "completed");

        // Reset jobs are now `queued`.
        for id in [job_a, job_b] {
            let status: String = conn
                .query_row(
                    "SELECT status FROM action_jobs WHERE job_id = ?1",
                    params![id.as_slice()],
                    |row| row.get(0),
                )
                .expect("query reset job");
            assert_eq!(status, "queued");
        }

        // Reset ops are now `pending`; the `done` op is unchanged.
        let pending_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM action_job_ops WHERE status = 'pending'",
                [],
                |row| row.get(0),
            )
            .expect("count pending");
        assert_eq!(pending_count, 3);
    }

    #[test]
    fn recover_stale_leases_is_idempotent() {
        let conn = fresh_db();
        let job_id = [0x42; 16];
        insert_test_job(&conn, &job_id, "leased");

        let (first, _) = recover_stale_leases(&conn).expect("first recover");
        assert_eq!(first, 1);

        let (second, _) = recover_stale_leases(&conn).expect("second recover");
        assert_eq!(second, 0, "second call has nothing to reset");
    }

    #[test]
    fn query_job_status_returns_none_for_unknown_job() {
        let conn = fresh_db();
        let result = query_job_status(&conn, &[0u8; 16]).expect("query");
        assert!(result.is_none());
    }

    #[test]
    fn query_job_status_returns_status_for_existing_job() {
        let conn = fresh_db();
        let job_id = [0x33; 16];
        insert_test_job(&conn, &job_id, "executing");

        let snapshot = query_job_status(&conn, &job_id)
            .expect("query")
            .expect("present");
        assert_eq!(snapshot.status, JobStatus::Executing);
        assert!(snapshot.summary.is_none());
    }

    #[test]
    fn query_job_status_returns_summary_when_populated() {
        let conn = fresh_db();
        let job_id = [0x44; 16];
        // Insert with a populated summary.
        conn.execute(
            "INSERT INTO action_jobs (\
                 job_id, kind, account_id, status, quiet, payload, summary, \
                 created_at, updated_at\
             ) VALUES (?1, 'mail_plan', 'acc-1', 'completed', 0, X'', ?2, 0, 0)",
            params![job_id.as_slice(), b"summary-blob".as_slice()],
        )
        .expect("insert");

        let snapshot = query_job_status(&conn, &job_id)
            .expect("query")
            .expect("present");
        assert_eq!(snapshot.status, JobStatus::Completed);
        assert_eq!(snapshot.summary.as_deref(), Some(b"summary-blob".as_ref()));
    }

    #[test]
    fn unique_ordinal_per_job_rejects_duplicates() {
        let conn = fresh_db();
        let job_id = [0x55; 16];
        insert_test_job(&conn, &job_id, "queued");
        insert_test_op(&conn, &job_id, 0, "pending");
        // Same ordinal, different operation_id - should violate UNIQUE(job_id, ordinal).
        let result = conn.execute(
            "INSERT INTO action_job_ops (\
                 job_id, operation_id, ordinal, thread_id, operation, status\
             ) VALUES (?1, 1, 0, 'thr-1', X'', 'pending')",
            params![job_id.as_slice()],
        );
        assert!(result.is_err(), "duplicate (job_id, ordinal) must be rejected");
    }
}
