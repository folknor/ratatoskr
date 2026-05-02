-- ── Notifications & reminders ───────────────────────────────

CREATE TABLE IF NOT EXISTS follow_up_reminders (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    thread_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    remind_at INTEGER NOT NULL,
    status TEXT DEFAULT 'pending',
    created_at INTEGER DEFAULT (unixepoch()),
    FOREIGN KEY (account_id, thread_id) REFERENCES threads(account_id, id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_followup_status ON follow_up_reminders(status, remind_at);
CREATE INDEX IF NOT EXISTS idx_followup_thread ON follow_up_reminders(account_id, thread_id);

CREATE TABLE IF NOT EXISTS notification_vips (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    email_address TEXT NOT NULL,
    display_name TEXT,
    created_at INTEGER DEFAULT (unixepoch()),
    UNIQUE(account_id, email_address)
);
CREATE INDEX IF NOT EXISTS idx_notification_vips ON notification_vips(account_id, email_address);

-- ── Subscriptions & bundling ────────────────────────────────

CREATE TABLE IF NOT EXISTS unsubscribe_actions (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    thread_id TEXT NOT NULL,
    from_address TEXT NOT NULL,
    from_name TEXT,
    method TEXT NOT NULL,
    unsubscribe_url TEXT NOT NULL,
    status TEXT DEFAULT 'subscribed',
    unsubscribed_at INTEGER,
    created_at INTEGER DEFAULT (unixepoch()),
    UNIQUE(account_id, from_address)
);
CREATE INDEX IF NOT EXISTS idx_unsub_account ON unsubscribe_actions(account_id, status);

CREATE TABLE IF NOT EXISTS bundle_rules (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    bundle TEXT NOT NULL,
    is_bundled INTEGER DEFAULT 1,
    delivery_enabled INTEGER DEFAULT 0,
    delivery_schedule TEXT,
    last_delivered_at INTEGER,
    created_at INTEGER DEFAULT (unixepoch()),
    UNIQUE(account_id, bundle)
);
CREATE INDEX IF NOT EXISTS idx_bundle_rules_account ON bundle_rules(account_id);

CREATE TABLE IF NOT EXISTS bundled_threads (
    account_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    bundle TEXT NOT NULL,
    held_until INTEGER,
    PRIMARY KEY (account_id, thread_id),
    FOREIGN KEY (account_id, thread_id) REFERENCES threads(account_id, id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_bundled_held ON bundled_threads(held_until);

-- ── Auto-responses ──────────────────────────────────────────

CREATE TABLE IF NOT EXISTS auto_responses (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    enabled INTEGER NOT NULL DEFAULT 0,
    start_date TEXT,
    end_date TEXT,
    internal_message_html TEXT,
    external_message_html TEXT,
    external_audience TEXT NOT NULL DEFAULT 'all',
    last_synced_at INTEGER DEFAULT (unixepoch()),
    PRIMARY KEY (account_id)
);
