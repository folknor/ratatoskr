-- ── Smart folders & quick steps ─────────────────────────────

CREATE TABLE IF NOT EXISTS smart_folders (
    id TEXT PRIMARY KEY,
    account_id TEXT,
    name TEXT NOT NULL,
    query TEXT NOT NULL,
    icon TEXT DEFAULT 'Search',
    color TEXT,
    sort_order INTEGER DEFAULT 0,
    is_default INTEGER DEFAULT 0,
    created_at INTEGER DEFAULT (unixepoch()),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_smart_folders_account ON smart_folders(account_id);

INSERT OR IGNORE INTO smart_folders (id, account_id, name, query, icon, sort_order, is_default) VALUES
    ('sf-unread', NULL, 'Unread', 'is:unread', 'MailOpen', 0, 1),
    ('sf-attachments', NULL, 'Has Attachments', 'has:attachment', 'Paperclip', 1, 1),
    ('sf-starred-recent', NULL, 'Starred This Week', 'is:starred after:__LAST_7_DAYS__', 'Star', 2, 1);

CREATE TABLE IF NOT EXISTS quick_steps (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT,
    shortcut TEXT,
    actions_json TEXT NOT NULL,
    icon TEXT,
    is_enabled INTEGER DEFAULT 1,
    continue_on_error INTEGER DEFAULT 0,
    sort_order INTEGER DEFAULT 0,
    created_at INTEGER DEFAULT (unixepoch())
);
CREATE INDEX IF NOT EXISTS idx_quick_steps_account ON quick_steps(account_id);

-- ── Pinned searches ─────────────────────────────────────────

CREATE TABLE IF NOT EXISTS pinned_searches (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    query TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    scope_account_id TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_pinned_searches_query
    ON pinned_searches(query);

CREATE TABLE IF NOT EXISTS pinned_search_threads (
    pinned_search_id INTEGER NOT NULL
        REFERENCES pinned_searches(id) ON DELETE CASCADE,
    thread_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    PRIMARY KEY (pinned_search_id, thread_id, account_id)
);

-- ── Smart features ──────────────────────────────────────────

CREATE TABLE IF NOT EXISTS ai_cache (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    thread_id TEXT NOT NULL,
    type TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at INTEGER DEFAULT (unixepoch()),
    UNIQUE(account_id, thread_id, type)
);
CREATE INDEX IF NOT EXISTS idx_ai_cache_lookup ON ai_cache(account_id, thread_id, type);

CREATE TABLE IF NOT EXISTS smart_label_rules (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    label_id TEXT NOT NULL,
    ai_description TEXT NOT NULL,
    criteria_json TEXT,
    is_enabled INTEGER DEFAULT 1,
    sort_order INTEGER DEFAULT 0,
    created_at INTEGER DEFAULT (unixepoch())
);
CREATE INDEX IF NOT EXISTS idx_smart_label_rules_account ON smart_label_rules(account_id);

CREATE TABLE IF NOT EXISTS writing_style_profiles (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    profile_text TEXT NOT NULL,
    sample_count INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER DEFAULT (unixepoch()),
    updated_at INTEGER DEFAULT (unixepoch()),
    UNIQUE(account_id)
);
