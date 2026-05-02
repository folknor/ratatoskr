-- ── Email security ──────────────────────────────────────────

CREATE TABLE IF NOT EXISTS link_scan_results (
    message_id TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    result_json TEXT NOT NULL,
    scanned_at INTEGER DEFAULT (unixepoch()),
    PRIMARY KEY (account_id, message_id)
);

CREATE TABLE IF NOT EXISTS phishing_allowlist (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    sender_address TEXT NOT NULL,
    created_at INTEGER DEFAULT (unixepoch()),
    UNIQUE(account_id, sender_address)
);

CREATE TABLE IF NOT EXISTS bimi_cache (
    domain TEXT PRIMARY KEY,
    has_bimi INTEGER NOT NULL DEFAULT 0,
    logo_uri TEXT,
    authority_uri TEXT,
    fetched_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_bimi_cache_expires ON bimi_cache(expires_at);

CREATE TABLE IF NOT EXISTS message_reactions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    reactor_email TEXT NOT NULL,
    reactor_name TEXT,
    reaction_type TEXT NOT NULL,
    reacted_at INTEGER,
    source TEXT NOT NULL,
    UNIQUE(message_id, account_id, reactor_email, reaction_type)
);
CREATE INDEX IF NOT EXISTS idx_message_reactions_message ON message_reactions(message_id, account_id);

CREATE TABLE IF NOT EXISTS read_receipt_policy (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    scope TEXT NOT NULL,
    policy TEXT NOT NULL DEFAULT 'never',
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    UNIQUE(account_id, scope)
);

-- ── Filter rules, templates, image allowlist ────────────────

CREATE TABLE IF NOT EXISTS filter_rules (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    is_enabled INTEGER DEFAULT 1,
    criteria_json TEXT NOT NULL,
    actions_json TEXT NOT NULL,
    sort_order INTEGER DEFAULT 0,
    created_at INTEGER DEFAULT (unixepoch())
);
CREATE INDEX IF NOT EXISTS idx_filter_rules_account ON filter_rules(account_id);

CREATE TABLE IF NOT EXISTS templates (
    id TEXT PRIMARY KEY,
    account_id TEXT,
    name TEXT NOT NULL,
    subject TEXT,
    body_html TEXT NOT NULL,
    shortcut TEXT,
    sort_order INTEGER DEFAULT 0,
    created_at INTEGER DEFAULT (unixepoch()),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_templates_account ON templates(account_id);

CREATE TABLE IF NOT EXISTS image_allowlist (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    sender_address TEXT NOT NULL,
    created_at INTEGER DEFAULT (unixepoch()),
    UNIQUE(account_id, sender_address)
);
CREATE INDEX IF NOT EXISTS idx_image_allowlist_sender ON image_allowlist(account_id, sender_address);
