-- ── Sync state ──────────────────────────────────────────────

-- Bifrost opaque cursor store (B2). Replaces the per-protocol cursor
-- tables.
-- bifrost owns the protocol-minted envelope bytes serialized by its
-- encode_envelope codec; ratatoskr owns the SQLite storage and lookup keys.
-- `checkpoint_blob` is the self-describing envelope (scope, protocol, BOTH
-- envelope versions, server_state bytes, advanced_through, partition,
-- progress_marker, BackfillProgress) and is the single source of truth for
-- the round-trip. The other columns are query keys minted from the typed
-- Checkpoint at write time, never authoritative: `scope_key` (serialized
-- CursorScope) for scope lookup, `kind` ('change' | 'backfill') to
-- discriminate, `partition_key` (Partition.0; empty blob for change cursors)
-- as the backfill PK dimension, `items_done` so get_backfill picks the
-- latest partition via ORDER BY without decoding every blob.
CREATE TABLE IF NOT EXISTS sync_cursors (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,                       -- 'change' | 'backfill'
    scope_key TEXT NOT NULL,                  -- serialized CursorScope
    partition_key BLOB NOT NULL DEFAULT X'',  -- Partition.0; empty for change
    items_done INTEGER NOT NULL DEFAULT 0,    -- BackfillProgress.items_done
    checkpoint_blob BLOB NOT NULL,            -- encode_envelope(&Checkpoint)
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    PRIMARY KEY (account_id, kind, scope_key, partition_key)
);

CREATE TABLE IF NOT EXISTS seen_ingest_markers (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    scope_key TEXT NOT NULL,
    checkpoint_blob BLOB NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    PRIMARY KEY (account_id, scope_key, checkpoint_blob)
);

CREATE TABLE IF NOT EXISTS shared_mailboxes (
    account_id TEXT NOT NULL,
    mailbox_id TEXT NOT NULL,
    display_name TEXT,
    is_sync_enabled INTEGER NOT NULL DEFAULT 0,
    email_address TEXT,
    is_visible INTEGER NOT NULL DEFAULT 1,
    discovered_at INTEGER NOT NULL DEFAULT (unixepoch()),
    revoked_at INTEGER,
    PRIMARY KEY (account_id, mailbox_id),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);

-- ── Offline queue ───────────────────────────────────────────

CREATE TABLE IF NOT EXISTS pending_operations (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    operation_type TEXT NOT NULL,
    resource_id TEXT NOT NULL,
    params TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    retry_count INTEGER DEFAULT 0,
    max_retries INTEGER DEFAULT 10,
    next_retry_at INTEGER,
    created_at INTEGER DEFAULT (unixepoch()),
    error_message TEXT
);
CREATE INDEX IF NOT EXISTS idx_pending_ops_status ON pending_operations(status, next_retry_at);
CREATE INDEX IF NOT EXISTS idx_pending_ops_resource ON pending_operations(account_id, resource_id);

-- ── Cross-store invariant pass cursors (Phase 8-2) ──────────
-- One row per content store. Updated to unixepoch() during the
-- graceful shutdown drain, just before the clean_shutdown sentinel
-- write. The startup invariant pass scans only rows whose store-side
-- timestamp (bodies.inserted_at, inline_images.created_at,
-- attachment_extracted_text.extracted_at) exceeds the cursor, bounding
-- the scan on a 200 GB mailbox after a non-graceful exit.
--
-- Defense-in-depth, not load-bearing: the per-account initial-sync reset
-- + next initial-style sync handles correctness regardless of what the
-- cursor-bounded scan misses.
CREATE TABLE IF NOT EXISTS clean_shutdown_cursors (
    store_name TEXT PRIMARY KEY,
    last_clean_shutdown_at INTEGER NOT NULL DEFAULT 0
);
