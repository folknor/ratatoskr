-- ── Core ────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    display_name TEXT,
    avatar_url TEXT,
    access_token TEXT,
    refresh_token TEXT,
    token_expires_at INTEGER,
    history_id TEXT,
    last_sync_at INTEGER,
    is_active INTEGER DEFAULT 1,
    created_at INTEGER DEFAULT (unixepoch()),
    updated_at INTEGER DEFAULT (unixepoch()),
    provider TEXT DEFAULT 'gmail_api',
    imap_host TEXT,
    imap_port INTEGER,
    imap_security TEXT,
    smtp_host TEXT,
    smtp_port INTEGER,
    smtp_security TEXT,
    auth_method TEXT DEFAULT 'oauth2',
    imap_password TEXT,
    oauth_provider TEXT,
    oauth_client_id TEXT,
    oauth_client_secret TEXT,
    imap_username TEXT,
    caldav_url TEXT,
    caldav_username TEXT,
    caldav_password TEXT,
    caldav_principal_url TEXT,
    caldav_home_url TEXT,
    calendar_provider TEXT,
    accept_invalid_certs INTEGER DEFAULT 0,
    jmap_url TEXT,
    oauth_token_url TEXT,
    initial_sync_completed INTEGER NOT NULL DEFAULT 0,
    account_color TEXT,
    account_name TEXT,
    sort_order INTEGER NOT NULL DEFAULT 0,
    smtp_username TEXT,
    smtp_password TEXT,
    supports_keywords INTEGER,
    -- Phase 8-5 (account deletion is_deleting gate): set to 1 by
    -- `account.delete` immediately after the cancel-and-await flow
    -- starts so subsequent SyncTick / start_account requests skip the
    -- account. The row deletion finalizes the flow; before that, the
    -- `is_deleting=1` flag prevents a SyncTick firing between cancel-
    -- ack and row-delete from re-kicking a sync against the
    -- disappearing account. Defense-in-depth: the gate exists at
    -- both the UI SyncTick filter and the Service-side
    -- SyncRuntime::start_account guard.
    is_deleting INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

INSERT OR IGNORE INTO settings (key, value) VALUES
    ('theme', 'System'),
    ('sidebar_collapsed', 'false'),
    ('reading_pane_position', 'Right'),
    ('sync_period_days', '365'),
    ('notifications_enabled', 'true'),
    ('undo_send_delay_seconds', '5'),
    ('default_font', 'system'),
    ('font_size', 'Default'),
    ('block_remote_images', 'true'),
    ('ai_enabled', 'true'),
    ('ai_auto_categorize', 'true'),
    ('ai_auto_summarize', 'true'),
    ('contact_sidebar_visible', 'true'),
    ('attachment_cache_max_mb', '500'),
    ('calendar_enabled', 'false'),
    ('smart_notifications', 'true'),
    ('notify_categories', 'Primary'),
    ('auto_archive_after_unsubscribe', 'true'),
    ('phishing_detection_enabled', 'true'),
    ('phishing_sensitivity', 'Default'),
    ('ai_auto_draft_enabled', 'true'),
    ('ai_writing_style_enabled', 'true'),
    ('default_read_receipt_policy', 'never'),
    ('calendar_default_view', 'month');
