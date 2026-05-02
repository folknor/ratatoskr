-- ── Tasks ───────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    account_id TEXT,
    title TEXT NOT NULL,
    description TEXT,
    priority TEXT DEFAULT 'none',
    is_completed INTEGER DEFAULT 0,
    completed_at INTEGER,
    due_date INTEGER,
    parent_id TEXT,
    thread_id TEXT,
    thread_account_id TEXT,
    sort_order INTEGER DEFAULT 0,
    recurrence_rule TEXT,
    next_recurrence_at INTEGER,
    tags_json TEXT DEFAULT '[]',
    created_at INTEGER DEFAULT (unixepoch()),
    updated_at INTEGER DEFAULT (unixepoch()),
    FOREIGN KEY (parent_id) REFERENCES tasks(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_tasks_account ON tasks(account_id);
CREATE INDEX IF NOT EXISTS idx_tasks_completed_due ON tasks(is_completed, due_date);
CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_id);
CREATE INDEX IF NOT EXISTS idx_tasks_thread ON tasks(thread_account_id, thread_id);
CREATE INDEX IF NOT EXISTS idx_tasks_due ON tasks(due_date);
CREATE INDEX IF NOT EXISTS idx_tasks_sort ON tasks(sort_order);

CREATE TABLE IF NOT EXISTS task_tags (
    tag TEXT NOT NULL,
    account_id TEXT,
    color TEXT,
    sort_order INTEGER DEFAULT 0,
    created_at INTEGER DEFAULT (unixepoch()),
    PRIMARY KEY (tag, account_id)
);
