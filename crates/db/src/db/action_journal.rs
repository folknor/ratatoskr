//! Action-journal write helpers.
//!
//! The journal tables `action_jobs` and `action_job_ops` are defined in
//! `schema/12_actions.sql`. This module exposes the narrow `pub(crate)`
//! helpers that `service-state::WriteDbState` re-exposes through
//! `ActionDbWrite` (Phase 2 task 7) and that the action handler +
//! worker (Phase 2 task 9) and Phase 1.5's boot recovery
//! (`recover_on_boot_db_only`) consume.

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

/// Return the `job_id`s of every `kind = 'send'` job whose status is
/// not yet terminal (i.e. queued / leased / executing). Backs the
/// boot-time send-vault orphan cleanup pass: the on-disk
/// `<app_data>/send_vault/` is reconciled against this set so any
/// vault directory whose parent job no longer exists (because the
/// handler crashed mid-transfer, or the worker finalized + unlinked
/// without a clean shutdown) is removed.
///
/// Phase 2 task 5. The query is a small SELECT; the boot recovery
/// pass that calls it runs once per Service incarnation.
pub fn live_send_job_ids(conn: &Connection) -> Result<Vec<[u8; 16]>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT job_id FROM action_jobs \
             WHERE kind = 'send' AND status NOT IN ('completed', 'failed')",
        )
        .map_err(|e| format!("live_send_job_ids prepare: {e}"))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, Vec<u8>>("job_id"))
        .map_err(|e| format!("live_send_job_ids query: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        let bytes = row.map_err(|e| format!("live_send_job_ids row: {e}"))?;
        let arr: [u8; 16] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| format!("live_send_job_ids: job_id len {} != 16", bytes.len()))?;
        out.push(arr);
    }
    Ok(out)
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

// ---------------------------------------------------------------------------
// Handler-side insert (Phase 2 task 9)
// ---------------------------------------------------------------------------

/// One operation row to insert as part of an `insert_mail_plan` call.
#[derive(Debug, Clone)]
pub struct PlanOpInsert {
    pub operation_id: u32,
    pub ordinal: u32,
    pub thread_id: String,
    /// Serialized `WireMailOperation` payload.
    pub operation_blob: Vec<u8>,
}

/// Atomically insert a `mail_plan` job + its ops in a single
/// transaction. Returns the journal-side timestamp (UNIX seconds) so
/// the caller can echo it in logs / reconciliation responses.
///
/// The handler calls this BEFORE returning `ActionPlanAck` to the UI.
/// The transaction commit IS the durability boundary that backs the
/// "journaled = true" promise: on a Service crash after this returns,
/// the worker's recovery sweep finds the rows and replays them.
pub fn insert_mail_plan(
    conn: &Connection,
    plan_id: &[u8; 16],
    account_id: &str,
    quiet: bool,
    ops: &[PlanOpInsert],
) -> Result<i64, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("insert_mail_plan begin: {e}"))?;
    let now: i64 = tx
        .query_row("SELECT unixepoch()", [], |row| row.get(0))
        .map_err(|e| format!("insert_mail_plan now: {e}"))?;
    tx.execute(
        "INSERT INTO action_jobs (\
             job_id, kind, account_id, status, quiet, payload, \
             created_at, updated_at\
         ) VALUES (?1, 'mail_plan', ?2, 'queued', ?3, X'', ?4, ?4)",
        params![plan_id.as_slice(), account_id, quiet as i64, now],
    )
    .map_err(|e| format!("insert_mail_plan jobs: {e}"))?;
    for op in ops {
        tx.execute(
            "INSERT INTO action_job_ops (\
                 job_id, operation_id, ordinal, thread_id, operation, status\
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'pending')",
            params![
                plan_id.as_slice(),
                op.operation_id,
                op.ordinal,
                op.thread_id,
                op.operation_blob.as_slice(),
            ],
        )
        .map_err(|e| format!("insert_mail_plan ops: {e}"))?;
    }
    tx.commit()
        .map_err(|e| format!("insert_mail_plan commit: {e}"))?;
    Ok(now)
}

/// Insert a quiet single-row job with no `action_job_ops` rows.
///
/// Used by Phase 2 task 15 (`mark_chat_read`) and similar
/// non-mail-thread jobs where the per-job state lives entirely in the
/// payload BLOB. The handler writes the row inside the request future
/// (so the durability boundary is the same as `insert_mail_plan`),
/// then signals the worker. The worker picks the row up via
/// `lease_next_ready_quiet_job`, runs the `kind`-specific work, and
/// finalizes via `finalize_job`.
///
/// `kind` MUST be a value the schema CHECK constraint accepts
/// (currently `mail_plan` / `send` / `mark_chat_read`); the row gets
/// `quiet = 1`.
pub fn insert_quiet_job(
    conn: &Connection,
    job_id: &[u8; 16],
    kind: &str,
    account_id: &str,
    payload: &[u8],
) -> Result<i64, String> {
    let now: i64 = conn
        .query_row("SELECT unixepoch()", [], |row| row.get(0))
        .map_err(|e| format!("insert_quiet_job now: {e}"))?;
    conn.execute(
        "INSERT INTO action_jobs (\
             job_id, kind, account_id, status, quiet, payload, \
             created_at, updated_at\
         ) VALUES (?1, ?2, ?3, 'queued', 1, ?4, ?5, ?5)",
        params![job_id.as_slice(), kind, account_id, payload, now],
    )
    .map_err(|e| format!("insert_quiet_job: {e}"))?;
    Ok(now)
}

/// A leased quiet job (no `action_job_ops` rows) ready for execution.
#[derive(Debug, Clone)]
pub struct LeasedQuietJob {
    pub job_id: [u8; 16],
    pub kind: String,
    pub account_id: String,
    pub payload: Vec<u8>,
}

/// Atomically pick the next ready quiet job of the given kind and
/// transition it from `queued` to `executing` with the worker
/// incarnation as owner. Used by the Phase 2 task 15 / 17 / 13
/// quiet-job paths.
///
/// SQLite's `UPDATE ... RETURNING` (3.35+) gives single-round-trip
/// atomicity. The `action_jobs_status_account` index covers the
/// inner SELECT.
pub fn lease_next_ready_quiet_job(
    conn: &Connection,
    kind: &str,
    worker_owner: &[u8; 16],
    lease_duration_ms: i64,
) -> Result<Option<LeasedQuietJob>, String> {
    type LeaseRow = (Vec<u8>, String, String, Vec<u8>);
    let row: Option<LeaseRow> = conn
        .query_row(
            "UPDATE action_jobs SET \
                 status = 'executing', \
                 lease_owner = ?1, \
                 lease_expires_at = unixepoch('subsec') * 1000 + ?2, \
                 updated_at = unixepoch() \
             WHERE job_id = ( \
                 SELECT job_id FROM action_jobs \
                 WHERE kind = ?3 AND status = 'queued' \
                 ORDER BY created_at \
                 LIMIT 1 \
             ) \
             RETURNING job_id, kind, account_id, payload",
            params![worker_owner.as_slice(), lease_duration_ms, kind],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|e| format!("lease_next_ready_quiet_job: {e}"))?;
    let Some((job_id_bytes, kind, account_id, payload)) = row else {
        return Ok(None);
    };
    let job_id: [u8; 16] = job_id_bytes
        .as_slice()
        .try_into()
        .map_err(|_| "lease_next_ready_quiet_job: job_id is not 16 bytes".to_string())?;
    Ok(Some(LeasedQuietJob {
        job_id,
        kind,
        account_id,
        payload,
    }))
}

// ---------------------------------------------------------------------------
// Worker-side lease (Phase 2 task 9)
// ---------------------------------------------------------------------------

/// A row leased by `lease_next_ready_op`, ready for execution.
#[derive(Debug, Clone)]
pub struct LeasedOp {
    pub plan_id: [u8; 16],
    pub operation_id: u32,
    pub ordinal: u32,
    pub thread_id: String,
    pub operation_blob: Vec<u8>,
    pub account_id: String,
    pub quiet: bool,
}

/// Atomically pick the next ready op across all jobs and transition it
/// from `pending` to `leased` with the worker incarnation as owner.
///
/// Order: oldest job first (`action_jobs.created_at` ASC), then
/// smallest ordinal within the job. SQLite's `UPDATE ... RETURNING`
/// (3.35+) gives us atomicity in one round-trip; the partial index
/// `action_job_ops_ready` covers the inner SELECT.
///
/// Account-fairness is enforced on the worker side via a per-account
/// `tokio::sync::Semaphore` rather than in SQL - the SELECT below is
/// purely "next ready op anywhere," and the worker pool is
/// responsible for not grabbing more than N ops per account in
/// parallel.
///
/// `lease_duration_ms` sets `lease_expires_at` for the recovery sweep
/// (`recover_stale_leases`) - if the worker dies before completing
/// the op, recovery resets it to `pending`.
pub fn lease_next_ready_op(
    conn: &Connection,
    worker_owner: &[u8; 16],
    lease_duration_ms: i64,
) -> Result<Option<LeasedOp>, String> {
    /// `(job_id_bytes, operation_id, ordinal, thread_id, operation_blob)`
    /// destructured into a typed `LeasedOp` immediately after the query.
    type LeaseRow = (Vec<u8>, u32, u32, String, Vec<u8>);
    let row: Option<LeaseRow> = conn
        .query_row(
            "UPDATE action_job_ops SET \
                 status = 'leased', \
                 lease_owner = ?1, \
                 lease_expires_at = unixepoch('subsec') * 1000 + ?2 \
             WHERE (job_id, operation_id) = ( \
                 SELECT ops.job_id, ops.operation_id \
                 FROM action_job_ops ops \
                 JOIN action_jobs jobs USING (job_id) \
                 WHERE ops.status = 'pending' \
                   AND jobs.status IN ('queued', 'leased', 'executing') \
                 ORDER BY jobs.created_at, ops.ordinal \
                 LIMIT 1 \
             ) \
             RETURNING job_id, operation_id, ordinal, thread_id, operation",
            params![worker_owner.as_slice(), lease_duration_ms],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, u32>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|e| format!("lease_next_ready_op: {e}"))?;
    let Some((job_id_bytes, operation_id, ordinal, thread_id, operation_blob)) = row else {
        return Ok(None);
    };
    let plan_id: [u8; 16] = job_id_bytes
        .as_slice()
        .try_into()
        .map_err(|_| "lease_next_ready_op: job_id is not 16 bytes".to_string())?;
    // Pull account_id + quiet from the parent row. Cheap PK lookup.
    let (account_id, quiet) = conn
        .query_row(
            "SELECT account_id, quiet FROM action_jobs WHERE job_id = ?1",
            params![plan_id.as_slice()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? != 0)),
        )
        .map_err(|e| format!("lease_next_ready_op parent lookup: {e}"))?;
    Ok(Some(LeasedOp {
        plan_id,
        operation_id,
        ordinal,
        thread_id,
        operation_blob,
        account_id,
        quiet,
    }))
}

/// Mark a leased op as transitioned out of execution. The worker calls
/// this with the terminal status (`Done` / `Failed` / `Conflict`) and
/// the serialized `OperationResult` blob. Clears the lease so recovery
/// won't reset the row.
pub fn mark_op_terminal(
    conn: &Connection,
    plan_id: &[u8; 16],
    operation_id: u32,
    new_status: OpTerminalStatus,
    outcome_blob: &[u8],
) -> Result<(), String> {
    let status_str = new_status.as_sql();
    conn.execute(
        "UPDATE action_job_ops SET \
             status = ?1, \
             outcome = ?2, \
             lease_owner = NULL, \
             lease_expires_at = NULL \
         WHERE job_id = ?3 AND operation_id = ?4",
        params![status_str, outcome_blob, plan_id.as_slice(), operation_id],
    )
    .map_err(|e| format!("mark_op_terminal: {e}"))?;
    Ok(())
}

/// Terminal status for an op, as written by `mark_op_terminal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpTerminalStatus {
    Done,
    Failed,
    Conflict,
}

impl OpTerminalStatus {
    fn as_sql(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Conflict => "conflict",
        }
    }
}

/// Counts of `action_job_ops` rows by terminal/non-terminal state for
/// a single job. Returned by `count_ops_by_status` so the worker can
/// decide whether the job is finished and what summary to write.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpStatusCounts {
    pub pending: u32,
    pub leased: u32,
    pub executing: u32,
    pub done: u32,
    pub failed: u32,
    pub conflict: u32,
}

impl OpStatusCounts {
    pub fn total(&self) -> u32 {
        self.pending + self.leased + self.executing + self.done + self.failed + self.conflict
    }
    pub fn non_terminal(&self) -> u32 {
        self.pending + self.leased + self.executing
    }
    pub fn terminal(&self) -> u32 {
        self.done + self.failed + self.conflict
    }
}

/// Return per-status counts of ops for a job. Used by the worker after
/// `mark_op_terminal` to decide whether to finalize the job (if
/// `non_terminal() == 0`) and which terminal status to write
/// (`completed` / `partial` / `failed`).
pub fn count_ops_by_status(
    conn: &Connection,
    plan_id: &[u8; 16],
) -> Result<OpStatusCounts, String> {
    let mut counts = OpStatusCounts::default();
    let mut stmt = conn
        .prepare(
            "SELECT status, COUNT(*) FROM action_job_ops \
             WHERE job_id = ?1 GROUP BY status",
        )
        .map_err(|e| format!("count_ops_by_status prepare: {e}"))?;
    let rows = stmt
        .query_map(params![plan_id.as_slice()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
        })
        .map_err(|e| format!("count_ops_by_status query: {e}"))?;
    for row in rows {
        let (status, count) = row.map_err(|e| format!("count_ops_by_status row: {e}"))?;
        match status.as_str() {
            "pending" => counts.pending = count,
            "leased" => counts.leased = count,
            "executing" => counts.executing = count,
            "done" => counts.done = count,
            "failed" => counts.failed = count,
            "conflict" => counts.conflict = count,
            other => log::warn!("count_ops_by_status: unknown op status {other}"),
        }
    }
    Ok(counts)
}

/// Set the terminal status + summary blob on an `action_jobs` row.
/// The worker calls this once when the last op transitions out of
/// non-terminal status, choosing the new status from the per-op
/// counts (`completed` if everything succeeded, `failed` if every op
/// failed, `partial` otherwise).
pub fn finalize_job(
    conn: &Connection,
    plan_id: &[u8; 16],
    new_status: JobTerminalStatus,
    summary_blob: &[u8],
) -> Result<(), String> {
    let status_str = new_status.as_sql();
    conn.execute(
        "UPDATE action_jobs SET \
             status = ?1, \
             summary = ?2, \
             lease_owner = NULL, \
             lease_expires_at = NULL, \
             updated_at = unixepoch() \
         WHERE job_id = ?3",
        params![status_str, summary_blob, plan_id.as_slice()],
    )
    .map_err(|e| format!("finalize_job: {e}"))?;
    Ok(())
}

/// Terminal status for a job, written by `finalize_job`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobTerminalStatus {
    Completed,
    Partial,
    Failed,
}

impl JobTerminalStatus {
    fn as_sql(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Partial => "partial",
            Self::Failed => "failed",
        }
    }
}

/// One row returned by `unemitted_terminal_ops` for replay on UI
/// reconnection.
#[derive(Debug, Clone)]
pub struct ReplayableOp {
    pub plan_id: [u8; 16],
    pub operation_id: u32,
    pub status: OpTerminalStatus,
    pub outcome_blob: Vec<u8>,
    pub quiet: bool,
}

/// Return all ops that have a terminal outcome (`outcome IS NOT NULL`)
/// belonging to non-terminal jobs. These are the ops the Service must
/// re-emit to the UI on reconnection - the UI's per-plan
/// `applied_outcomes` set dedupes any duplicates against what it
/// already saw.
///
/// Quiet jobs (e.g. mark-chat-read) suppress per-op outcome emission;
/// `quiet` is returned so the caller can skip them.
pub fn unemitted_terminal_ops(conn: &Connection) -> Result<Vec<ReplayableOp>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT ops.job_id, ops.operation_id, ops.status, ops.outcome, jobs.quiet \
             FROM action_job_ops ops \
             JOIN action_jobs jobs USING (job_id) \
             WHERE ops.outcome IS NOT NULL \
               AND jobs.status IN ('queued', 'leased', 'executing') \
             ORDER BY jobs.created_at, ops.ordinal",
        )
        .map_err(|e| format!("unemitted_terminal_ops prepare: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, i64>(4)? != 0,
            ))
        })
        .map_err(|e| format!("unemitted_terminal_ops query: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        let (job_id_bytes, operation_id, status_str, outcome_blob, quiet) =
            row.map_err(|e| format!("unemitted_terminal_ops row: {e}"))?;
        let plan_id: [u8; 16] = job_id_bytes
            .as_slice()
            .try_into()
            .map_err(|_| "unemitted_terminal_ops: job_id is not 16 bytes".to_string())?;
        let status = match status_str.as_str() {
            "done" => OpTerminalStatus::Done,
            "failed" => OpTerminalStatus::Failed,
            "conflict" => OpTerminalStatus::Conflict,
            other => return Err(format!("unemitted_terminal_ops unknown op status: {other}")),
        };
        out.push(ReplayableOp {
            plan_id,
            operation_id,
            status,
            outcome_blob,
            quiet,
        });
    }
    Ok(out)
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
    fn insert_mail_plan_writes_jobs_and_ops_atomically() {
        let conn = fresh_db();
        let plan_id = [0xA1; 16];
        let ops = vec![
            PlanOpInsert {
                operation_id: 0,
                ordinal: 0,
                thread_id: "thr-1".into(),
                operation_blob: b"op-0-blob".to_vec(),
            },
            PlanOpInsert {
                operation_id: 1,
                ordinal: 1,
                thread_id: "thr-2".into(),
                operation_blob: b"op-1-blob".to_vec(),
            },
        ];
        let now = insert_mail_plan(&conn, &plan_id, "acc-1", false, &ops).expect("insert");
        assert!(now > 0);

        // jobs row exists with status='queued'
        let snapshot = query_job_status(&conn, &plan_id)
            .expect("status")
            .expect("present");
        assert_eq!(snapshot.status, JobStatus::Queued);

        // both ops inserted with status='pending'
        let counts = count_ops_by_status(&conn, &plan_id).expect("counts");
        assert_eq!(counts.pending, 2);
        assert_eq!(counts.terminal(), 0);
    }

    #[test]
    fn lease_next_ready_op_picks_oldest_pending() {
        let conn = fresh_db();
        let plan_a = [0xAA; 16];
        let plan_b = [0xBB; 16];
        // plan_b is created later, so plan_a should be picked first.
        insert_mail_plan(
            &conn,
            &plan_a,
            "acc-1",
            false,
            &[PlanOpInsert {
                operation_id: 0,
                ordinal: 0,
                thread_id: "thr-a".into(),
                operation_blob: b"a".to_vec(),
            }],
        )
        .expect("insert a");
        std::thread::sleep(std::time::Duration::from_millis(1100));
        insert_mail_plan(
            &conn,
            &plan_b,
            "acc-1",
            false,
            &[PlanOpInsert {
                operation_id: 0,
                ordinal: 0,
                thread_id: "thr-b".into(),
                operation_blob: b"b".to_vec(),
            }],
        )
        .expect("insert b");

        let owner = [0xFF; 16];
        let leased = lease_next_ready_op(&conn, &owner, 60_000)
            .expect("lease")
            .expect("some");
        assert_eq!(leased.plan_id, plan_a, "older job leased first");
        assert_eq!(leased.thread_id, "thr-a");
        assert_eq!(leased.account_id, "acc-1");
        assert!(!leased.quiet);
    }

    #[test]
    fn lease_next_ready_op_returns_none_when_no_pending() {
        let conn = fresh_db();
        let owner = [0u8; 16];
        let result = lease_next_ready_op(&conn, &owner, 60_000).expect("lease");
        assert!(result.is_none());
    }

    #[test]
    fn lease_next_ready_op_does_not_release_a_leased_op() {
        let conn = fresh_db();
        let plan_id = [0xCC; 16];
        insert_mail_plan(
            &conn,
            &plan_id,
            "acc-1",
            false,
            &[PlanOpInsert {
                operation_id: 0,
                ordinal: 0,
                thread_id: "thr".into(),
                operation_blob: b"x".to_vec(),
            }],
        )
        .expect("insert");
        let owner = [0xFF; 16];
        let first = lease_next_ready_op(&conn, &owner, 60_000)
            .expect("first lease")
            .expect("some");
        assert_eq!(first.operation_id, 0);
        // Second lease finds no pending (the one we just leased moved to
        // 'leased', not 'pending').
        let second = lease_next_ready_op(&conn, &owner, 60_000).expect("second lease");
        assert!(second.is_none());
    }

    #[test]
    fn mark_op_terminal_clears_lease_and_sets_outcome() {
        let conn = fresh_db();
        let plan_id = [0xDD; 16];
        insert_mail_plan(
            &conn,
            &plan_id,
            "acc-1",
            false,
            &[PlanOpInsert {
                operation_id: 7,
                ordinal: 0,
                thread_id: "thr".into(),
                operation_blob: b"x".to_vec(),
            }],
        )
        .expect("insert");
        let owner = [0xFF; 16];
        let leased = lease_next_ready_op(&conn, &owner, 60_000)
            .expect("lease")
            .expect("some");

        mark_op_terminal(
            &conn,
            &leased.plan_id,
            leased.operation_id,
            OpTerminalStatus::Done,
            b"outcome-blob",
        )
        .expect("mark done");

        let counts = count_ops_by_status(&conn, &plan_id).expect("counts");
        assert_eq!(counts.done, 1);
        assert_eq!(counts.non_terminal(), 0);

        // lease fields cleared
        let (lease_owner, lease_expires_at): (Option<Vec<u8>>, Option<i64>) = conn
            .query_row(
                "SELECT lease_owner, lease_expires_at FROM action_job_ops \
                 WHERE job_id = ?1 AND operation_id = 7",
                params![plan_id.as_slice()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("query");
        assert!(lease_owner.is_none());
        assert!(lease_expires_at.is_none());
    }

    #[test]
    fn finalize_job_sets_status_and_summary() {
        let conn = fresh_db();
        let plan_id = [0xEE; 16];
        insert_mail_plan(
            &conn,
            &plan_id,
            "acc-1",
            false,
            &[PlanOpInsert {
                operation_id: 0,
                ordinal: 0,
                thread_id: "thr".into(),
                operation_blob: b"x".to_vec(),
            }],
        )
        .expect("insert");

        finalize_job(
            &conn,
            &plan_id,
            JobTerminalStatus::Completed,
            b"summary-blob",
        )
        .expect("finalize");

        let snapshot = query_job_status(&conn, &plan_id)
            .expect("query")
            .expect("present");
        assert_eq!(snapshot.status, JobStatus::Completed);
        assert_eq!(snapshot.summary.as_deref(), Some(b"summary-blob".as_ref()));
    }

    #[test]
    fn count_ops_by_status_aggregates_correctly() {
        let conn = fresh_db();
        let plan_id = [0x12; 16];
        insert_mail_plan(
            &conn,
            &plan_id,
            "acc-1",
            false,
            &[
                PlanOpInsert {
                    operation_id: 0,
                    ordinal: 0,
                    thread_id: "t".into(),
                    operation_blob: b"x".to_vec(),
                },
                PlanOpInsert {
                    operation_id: 1,
                    ordinal: 1,
                    thread_id: "t".into(),
                    operation_blob: b"x".to_vec(),
                },
                PlanOpInsert {
                    operation_id: 2,
                    ordinal: 2,
                    thread_id: "t".into(),
                    operation_blob: b"x".to_vec(),
                },
            ],
        )
        .expect("insert");
        // Mark op 0 done, op 1 failed, leave op 2 pending.
        mark_op_terminal(&conn, &plan_id, 0, OpTerminalStatus::Done, b"o0").expect("done");
        mark_op_terminal(&conn, &plan_id, 1, OpTerminalStatus::Failed, b"o1").expect("failed");
        let counts = count_ops_by_status(&conn, &plan_id).expect("counts");
        assert_eq!(counts.done, 1);
        assert_eq!(counts.failed, 1);
        assert_eq!(counts.pending, 1);
        assert_eq!(counts.total(), 3);
        assert_eq!(counts.terminal(), 2);
        assert_eq!(counts.non_terminal(), 1);
    }

    #[test]
    fn unemitted_terminal_ops_returns_only_non_terminal_jobs() {
        let conn = fresh_db();
        let plan_active = [0x01; 16];
        let plan_done = [0x02; 16];

        insert_mail_plan(
            &conn,
            &plan_active,
            "acc-1",
            false,
            &[PlanOpInsert {
                operation_id: 0,
                ordinal: 0,
                thread_id: "t".into(),
                operation_blob: b"a".to_vec(),
            }],
        )
        .expect("insert active");
        insert_mail_plan(
            &conn,
            &plan_done,
            "acc-1",
            false,
            &[PlanOpInsert {
                operation_id: 0,
                ordinal: 0,
                thread_id: "t".into(),
                operation_blob: b"b".to_vec(),
            }],
        )
        .expect("insert done");

        // Both ops have outcomes, but only plan_active is non-terminal.
        mark_op_terminal(&conn, &plan_active, 0, OpTerminalStatus::Done, b"o-active")
            .expect("active done");
        mark_op_terminal(&conn, &plan_done, 0, OpTerminalStatus::Done, b"o-done").expect("done");
        finalize_job(&conn, &plan_done, JobTerminalStatus::Completed, b"sum")
            .expect("finalize done");

        let replayable = unemitted_terminal_ops(&conn).expect("query");
        assert_eq!(replayable.len(), 1);
        assert_eq!(replayable[0].plan_id, plan_active);
        assert_eq!(replayable[0].outcome_blob, b"o-active");
        assert!(matches!(replayable[0].status, OpTerminalStatus::Done));
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
