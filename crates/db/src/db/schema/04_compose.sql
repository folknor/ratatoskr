-- ── Signatures & send identities ────────────────────────────

CREATE TABLE IF NOT EXISTS signatures (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    body_html TEXT NOT NULL,
    is_default INTEGER DEFAULT 0,
    sort_order INTEGER DEFAULT 0,
    created_at INTEGER DEFAULT (unixepoch()),
    server_id TEXT,
    body_text TEXT,
    is_reply_default INTEGER NOT NULL DEFAULT 0,
    source TEXT NOT NULL DEFAULT 'local',
    last_synced_at INTEGER,
    server_html_hash TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_signatures_server ON signatures(account_id, server_id);

CREATE TABLE IF NOT EXISTS send_as_aliases (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    display_name TEXT,
    reply_to_address TEXT,
    signature_id TEXT,
    is_primary INTEGER DEFAULT 0,
    is_default INTEGER DEFAULT 0,
    treat_as_alias INTEGER DEFAULT 1,
    verification_status TEXT DEFAULT 'accepted',
    created_at INTEGER DEFAULT (unixepoch()),
    UNIQUE(account_id, email)
);
CREATE INDEX IF NOT EXISTS idx_send_as_account ON send_as_aliases(account_id);

CREATE TABLE IF NOT EXISTS send_identities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    display_name TEXT,
    mailbox_id TEXT,
    send_mode TEXT NOT NULL DEFAULT 'send_as',
    save_to_personal_sent INTEGER NOT NULL DEFAULT 1,
    is_primary INTEGER NOT NULL DEFAULT 0,
    UNIQUE(account_id, email)
);
CREATE INDEX IF NOT EXISTS idx_send_identities_account ON send_identities(account_id);

-- ── Drafts & scheduled email ────────────────────────────────

CREATE TABLE IF NOT EXISTS local_drafts (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    to_addresses TEXT,
    cc_addresses TEXT,
    bcc_addresses TEXT,
    subject TEXT,
    body_html TEXT,
    reply_to_message_id TEXT,
    thread_id TEXT,
    from_email TEXT,
    signature_id TEXT,
    remote_draft_id TEXT,
    attachments TEXT,
    created_at INTEGER DEFAULT (unixepoch()),
    updated_at INTEGER DEFAULT (unixepoch()),
    sync_status TEXT DEFAULT 'pending',
    signature_separator_index INTEGER
);

CREATE TABLE IF NOT EXISTS scheduled_emails (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    to_addresses TEXT NOT NULL,
    cc_addresses TEXT,
    bcc_addresses TEXT,
    subject TEXT,
    body_html TEXT NOT NULL,
    reply_to_message_id TEXT,
    thread_id TEXT,
    scheduled_at INTEGER NOT NULL,
    signature_id TEXT,
    attachment_paths TEXT,
    status TEXT DEFAULT 'pending',
    created_at INTEGER DEFAULT (unixepoch()),
    delegation TEXT NOT NULL DEFAULT 'local',
    remote_message_id TEXT,
    remote_status TEXT,
    timezone TEXT,
    from_email TEXT,
    error_message TEXT,
    retry_count INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_scheduled_status ON scheduled_emails(status, scheduled_at);
