-- ── Public folders ──────────────────────────────────────────

CREATE TABLE IF NOT EXISTS public_folder_pins (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder_id TEXT NOT NULL,
    is_sync_enabled INTEGER NOT NULL DEFAULT 0,
    is_visible INTEGER NOT NULL DEFAULT 1,
    sort_order INTEGER NOT NULL DEFAULT 0,
    UNIQUE(account_id, folder_id)
);

-- ── Chats ───────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS chat_contacts (
    email TEXT PRIMARY KEY COLLATE NOCASE,
    designated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    sort_order INTEGER NOT NULL DEFAULT 0,
    display_name TEXT,
    latest_message_at INTEGER,
    latest_message_preview TEXT,
    unread_count INTEGER NOT NULL DEFAULT 0,
    contact_id TEXT
);

CREATE TABLE IF NOT EXISTS thread_participants (
    account_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    email TEXT NOT NULL COLLATE NOCASE,
    PRIMARY KEY (account_id, thread_id, email),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_thread_participants_email ON thread_participants(email, account_id);
