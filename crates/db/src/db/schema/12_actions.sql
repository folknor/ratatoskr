-- ── Action job journal ──────────────────────────────────────
--
-- Phase 2 sibling-job model (per docs/service/phase-2-plan.md scope
-- item 18a). One `action_jobs` row per Service-side journaled action
-- (mail-plan execution, compose-send, mark-chat-read), discriminated
-- by `kind`. Multi-op mail plans also have one `action_job_ops` row
-- per operation; send / mark-chat-read jobs live in `action_jobs`
-- only and stash their per-job state in the `payload` BLOB.
--
-- The journal is the durability boundary that backs the
-- `action.execute_plan` ack contract: the request handler validates
-- the plan, INSERTs into both tables in one transaction, and only
-- then returns `ActionPlanAck { plan_id, journaled: true }`. From
-- that point a Service crash does NOT lose the plan - the worker
-- picks it up after respawn and journal-driven replay drives the
-- per-operation outcomes.
--
-- Note on `pending_operations`: the existing per-op transient-retry
-- queue (see `10_sync.sql`) is orthogonal to the journal. A plan op
-- that fails with a retryable RemoteFailure marks its journal row
-- `failed` AND enqueues the single op into `pending_operations` for
-- the periodic drainer to retry. Action-worker recovery drains the
-- journal at boot; pending-ops periodic drains transient retries on
-- tick. Neither subsumes the other.

CREATE TABLE IF NOT EXISTS action_jobs (
    -- 16-byte UUIDv7. Time-ordered so the partial index that drives
    -- worker scheduling has good locality; UI-generated so the UI
    -- can correlate `OperationOutcome` notifications back to the
    -- originating intent's UI metadata without a Service round-trip.
    job_id BLOB PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('mail_plan', 'send', 'mark_chat_read', 'calendar_plan')),
    account_id TEXT NOT NULL,
    status TEXT NOT NULL
        CHECK (status IN ('queued', 'leased', 'executing', 'completed', 'partial', 'failed')),
    quiet INTEGER NOT NULL DEFAULT 0
        CHECK (quiet IN (0, 1)),
    -- Job-kind-specific serialized payload. Empty for `mail_plan`
    -- (per-op state lives in action_job_ops). For `send`: serialized
    -- JournaledSend { send_id, message, attachments[vault_path,
    -- content_hash, size, mime, filename] }. For `mark_chat_read`:
    -- serialized JournaledChatRead { chat_email, resolved_thread_ids }.
    payload BLOB NOT NULL,
    -- Serialized PlanSummary / SendSummary / etc., populated when the
    -- job reaches a terminal status.
    summary BLOB,
    -- Worker-instance UUID currently leasing this job (NULL when idle).
    -- Boot recovery resets `leased` / `executing` rows whose lease
    -- doesn't match the live Service incarnation back to `queued`.
    lease_owner BLOB,
    -- UNIX millis. Worker renews on long jobs; recovery reclaims
    -- expired leases.
    lease_expires_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS action_job_ops (
    job_id BLOB NOT NULL,
    -- Per-plan ordinal id; UI generates `(plan_id, operation_id)`
    -- pairs and uses them to correlate `OperationOutcome` arrivals.
    operation_id INTEGER NOT NULL,
    -- Order of execution within the plan. Mostly informational; the
    -- worker is account-fair so strict ordinal order across accounts
    -- is not preserved (a fast op for account A and a slow op for
    -- account B can interleave).
    ordinal INTEGER NOT NULL,
    thread_id TEXT NOT NULL,
    -- Serialized WireMailOperation.
    operation BLOB NOT NULL,
    status TEXT NOT NULL
        CHECK (status IN ('pending', 'leased', 'executing', 'done', 'failed', 'conflict')),
    -- Serialized OperationResult; presence is the durable
    -- "result available" bit. Recovery replays any op with
    -- outcome IS NOT NULL on a non-quiet job whose parent job is
    -- not yet completed.
    outcome BLOB,
    lease_owner BLOB,
    lease_expires_at INTEGER,
    PRIMARY KEY (job_id, operation_id),
    UNIQUE (job_id, ordinal),
    FOREIGN KEY (job_id) REFERENCES action_jobs(job_id) ON DELETE CASCADE
);

-- Worker scheduler index: pick the next ready op for any active
-- job, ordered by ordinal within a plan. Partial so it stays small
-- as completed ops accumulate.
CREATE INDEX IF NOT EXISTS action_job_ops_ready
    ON action_job_ops(job_id, ordinal)
    WHERE status = 'pending';

-- Account-fair scheduling + boot recovery: walk active jobs by
-- (status, account_id, created_at) so the worker can balance across
-- accounts and recovery can scan only non-terminal jobs.
CREATE INDEX IF NOT EXISTS action_jobs_status_account
    ON action_jobs(status, account_id, created_at);

-- Lease-expiry sweeps. Partial so it stays small (most rows have
-- NULL `lease_expires_at`).
CREATE INDEX IF NOT EXISTS action_jobs_lease_expiry
    ON action_jobs(lease_expires_at)
    WHERE lease_expires_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS action_job_ops_lease_expiry
    ON action_job_ops(lease_expires_at)
    WHERE lease_expires_at IS NOT NULL;
