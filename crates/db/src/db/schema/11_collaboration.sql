-- ── Public folders ──────────────────────────────────────────

CREATE TABLE IF NOT EXISTS public_folders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder_id TEXT NOT NULL,
    parent_id TEXT,
    display_name TEXT NOT NULL,
    folder_class TEXT,
    unread_count INTEGER NOT NULL DEFAULT 0,
    total_count INTEGER NOT NULL DEFAULT 0,
    can_create_items INTEGER NOT NULL DEFAULT 0,
    can_modify INTEGER NOT NULL DEFAULT 0,
    can_delete INTEGER NOT NULL DEFAULT 0,
    can_read INTEGER NOT NULL DEFAULT 1,
    UNIQUE(account_id, folder_id)
);
CREATE INDEX IF NOT EXISTS idx_public_folders_parent ON public_folders(account_id, parent_id);

CREATE TABLE IF NOT EXISTS public_folder_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder_id TEXT NOT NULL,
    item_id TEXT NOT NULL,
    change_key TEXT,
    subject TEXT,
    sender_email TEXT,
    sender_name TEXT,
    received_at INTEGER,
    body_preview TEXT,
    is_read INTEGER NOT NULL DEFAULT 0,
    item_class TEXT NOT NULL DEFAULT 'IPM.Note',
    UNIQUE(account_id, item_id)
);
CREATE INDEX IF NOT EXISTS idx_public_folder_items_folder
    ON public_folder_items(account_id, folder_id, received_at DESC);

CREATE TABLE IF NOT EXISTS public_folder_pins (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder_id TEXT NOT NULL,
    sync_enabled INTEGER NOT NULL DEFAULT 1,
    sync_depth_days INTEGER NOT NULL DEFAULT 30,
    last_sync_at INTEGER,
    UNIQUE(account_id, folder_id)
);

CREATE TABLE IF NOT EXISTS public_folder_sync_state (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder_id TEXT NOT NULL,
    last_sync_timestamp INTEGER,
    last_full_scan_at INTEGER,
    PRIMARY KEY (account_id, folder_id)
);

CREATE TABLE IF NOT EXISTS public_folder_content_routing (
    account_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,
    replica_guid TEXT,
    content_mailbox TEXT NOT NULL,
    discovered_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (account_id, folder_id),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
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
