use rusqlite::Connection;

struct Migration {
    version: u32,
    description: &'static str,
    sql: &'static str,
}

// Schema collapsed from 80 incremental migrations on 2026-03-30.
// No releases have been made, so there are no databases in the wild
// that need incremental migration. If you have an existing dev database,
// delete it and re-seed (run_all will detect stale DBs and error).
//
// Future migrations go here as version 2, 3, etc.
static MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        description: "Initial schema (collapsed)",
        sql: r#"

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
    supports_keywords INTEGER
);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

INSERT OR IGNORE INTO settings (key, value) VALUES
    ('theme', 'system'),
    ('sidebar_collapsed', 'false'),
    ('reading_pane_position', 'right'),
    ('sync_period_days', '365'),
    ('notifications_enabled', 'true'),
    ('undo_send_delay_seconds', '5'),
    ('default_font', 'system'),
    ('font_size', 'default'),
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
    ('phishing_sensitivity', 'default'),
    ('ai_auto_draft_enabled', 'true'),
    ('ai_writing_style_enabled', 'true'),
    ('default_read_receipt_policy', 'never'),
    ('calendar_default_view', 'month');

-- ── Labels ──────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS labels (
    id TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    type TEXT NOT NULL,
    color_bg TEXT,
    color_fg TEXT,
    visible INTEGER DEFAULT 1,
    sort_order INTEGER DEFAULT 0,
    imap_folder_path TEXT,
    imap_special_use TEXT,
    namespace_type TEXT,
    parent_label_id TEXT,
    label_kind TEXT NOT NULL DEFAULT 'container',
    right_read INTEGER,
    right_add INTEGER,
    right_remove INTEGER,
    right_set_seen INTEGER,
    right_set_keywords INTEGER,
    right_create_child INTEGER,
    right_rename INTEGER,
    right_delete INTEGER,
    right_submit INTEGER,
    is_subscribed INTEGER,
    PRIMARY KEY (account_id, id)
);
CREATE INDEX IF NOT EXISTS idx_labels_account ON labels(account_id);

CREATE TABLE IF NOT EXISTS label_color_overrides (
    label_name TEXT NOT NULL PRIMARY KEY COLLATE NOCASE,
    color_bg TEXT NOT NULL
);

-- ── Threads ─────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS threads (
    id TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    subject TEXT,
    snippet TEXT,
    last_message_at INTEGER,
    message_count INTEGER DEFAULT 0,
    is_read INTEGER DEFAULT 0,
    is_starred INTEGER DEFAULT 0,
    is_important INTEGER DEFAULT 0,
    has_attachments INTEGER DEFAULT 0,
    is_snoozed INTEGER DEFAULT 0,
    snooze_until INTEGER,
    is_pinned INTEGER DEFAULT 0,
    is_muted INTEGER DEFAULT 0,
    shared_mailbox_id TEXT,
    is_chat_thread INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, id)
);
CREATE INDEX IF NOT EXISTS idx_threads_date ON threads(account_id, last_message_at DESC);
CREATE INDEX IF NOT EXISTS idx_threads_snoozed ON threads(is_snoozed, snooze_until);
CREATE INDEX IF NOT EXISTS idx_threads_pinned ON threads(account_id, is_pinned DESC, last_message_at DESC);
CREATE INDEX IF NOT EXISTS idx_threads_muted ON threads(account_id, is_muted);
CREATE INDEX IF NOT EXISTS idx_threads_shared_mailbox ON threads(account_id, shared_mailbox_id, last_message_at DESC);
CREATE INDEX IF NOT EXISTS idx_threads_chat ON threads(account_id, is_chat_thread) WHERE is_chat_thread = 1;

CREATE TABLE IF NOT EXISTS thread_labels (
    thread_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    label_id TEXT NOT NULL,
    PRIMARY KEY (account_id, thread_id, label_id),
    FOREIGN KEY (account_id, thread_id) REFERENCES threads(account_id, id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_thread_labels_label ON thread_labels(account_id, label_id);

CREATE TABLE IF NOT EXISTS thread_categories (
    account_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    category TEXT NOT NULL,
    is_manual INTEGER DEFAULT 0,
    created_at INTEGER DEFAULT (unixepoch()),
    PRIMARY KEY (account_id, thread_id),
    FOREIGN KEY (account_id, thread_id) REFERENCES threads(account_id, id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_thread_categories_cat ON thread_categories(account_id, category);

-- ── Messages ────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS messages (
    id TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    thread_id TEXT NOT NULL,
    from_address TEXT,
    from_name TEXT,
    to_addresses TEXT,
    cc_addresses TEXT,
    bcc_addresses TEXT,
    reply_to TEXT,
    subject TEXT,
    snippet TEXT,
    date INTEGER NOT NULL,
    is_read INTEGER DEFAULT 0,
    is_starred INTEGER DEFAULT 0,
    body_cached INTEGER DEFAULT 0,
    raw_size INTEGER,
    internal_date INTEGER,
    list_unsubscribe TEXT,
    list_unsubscribe_post TEXT,
    auth_results TEXT,
    message_id_header TEXT,
    references_header TEXT,
    in_reply_to_header TEXT,
    imap_uid INTEGER,
    imap_folder TEXT,
    mdn_requested INTEGER NOT NULL DEFAULT 0,
    is_reaction INTEGER NOT NULL DEFAULT 0,
    mdn_sent INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, id),
    FOREIGN KEY (account_id, thread_id) REFERENCES threads(account_id, id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_messages_thread ON messages(account_id, thread_id, date ASC);
CREATE INDEX IF NOT EXISTS idx_messages_date ON messages(account_id, date DESC);
CREATE INDEX IF NOT EXISTS idx_messages_from ON messages(from_address);
CREATE INDEX IF NOT EXISTS idx_messages_imap_uid ON messages(account_id, imap_folder, imap_uid);
CREATE INDEX IF NOT EXISTS idx_messages_message_id ON messages(message_id_header);

-- ── Attachments ─────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS attachments (
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    filename TEXT,
    mime_type TEXT,
    size INTEGER,
    gmail_attachment_id TEXT,
    content_id TEXT,
    is_inline INTEGER DEFAULT 0,
    local_path TEXT,
    imap_part_id TEXT,
    cached_at INTEGER,
    cache_size INTEGER,
    content_hash TEXT,
    FOREIGN KEY (account_id, message_id) REFERENCES messages(account_id, id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_attachments_message ON attachments(account_id, message_id);
CREATE INDEX IF NOT EXISTS idx_attachments_cid ON attachments(content_id);
CREATE INDEX IF NOT EXISTS idx_attachments_content_hash ON attachments(content_hash);

CREATE TABLE IF NOT EXISTS cloud_attachments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id TEXT,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    direction TEXT NOT NULL,
    provider TEXT NOT NULL,
    cloud_url TEXT,
    file_name TEXT,
    file_size INTEGER,
    mime_type TEXT,
    drive_item_id TEXT,
    upload_session_url TEXT,
    upload_status TEXT NOT NULL DEFAULT 'pending',
    bytes_uploaded INTEGER NOT NULL DEFAULT 0,
    retry_count INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);
CREATE INDEX IF NOT EXISTS idx_cloud_attachments_message ON cloud_attachments(message_id);
CREATE INDEX IF NOT EXISTS idx_cloud_attachments_status ON cloud_attachments(upload_status) WHERE upload_status != 'sent';

-- ── Contacts ────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS contacts (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    display_name TEXT,
    avatar_url TEXT,
    frequency INTEGER DEFAULT 1,
    last_contacted_at INTEGER,
    created_at INTEGER DEFAULT (unixepoch()),
    updated_at INTEGER DEFAULT (unixepoch()),
    first_contacted_at INTEGER,
    notes TEXT,
    source TEXT NOT NULL DEFAULT 'user',
    display_name_overridden INTEGER NOT NULL DEFAULT 0,
    email2 TEXT,
    phone TEXT,
    company TEXT,
    account_id TEXT,
    server_id TEXT
);
CREATE INDEX IF NOT EXISTS idx_contacts_email ON contacts(email);
CREATE INDEX IF NOT EXISTS idx_contacts_frequency ON contacts(frequency DESC);

CREATE VIRTUAL TABLE IF NOT EXISTS contacts_fts USING fts5(
    email, display_name,
    content=contacts, content_rowid=rowid,
    tokenize="unicode61 tokenchars '@._-'", prefix='2,3'
);

CREATE TRIGGER IF NOT EXISTS contacts_fts_insert AFTER INSERT ON contacts BEGIN
    INSERT INTO contacts_fts(rowid, email, display_name)
    VALUES (NEW.rowid, NEW.email, COALESCE(NEW.display_name, ''));
END;
CREATE TRIGGER IF NOT EXISTS contacts_fts_delete AFTER DELETE ON contacts BEGIN
    INSERT INTO contacts_fts(contacts_fts, rowid, email, display_name)
    VALUES ('delete', OLD.rowid, OLD.email, COALESCE(OLD.display_name, ''));
END;
CREATE TRIGGER IF NOT EXISTS contacts_fts_update AFTER UPDATE ON contacts BEGIN
    INSERT INTO contacts_fts(contacts_fts, rowid, email, display_name)
    VALUES ('delete', OLD.rowid, OLD.email, COALESCE(OLD.display_name, ''));
    INSERT INTO contacts_fts(rowid, email, display_name)
    VALUES (NEW.rowid, NEW.email, COALESCE(NEW.display_name, ''));
END;

CREATE TABLE IF NOT EXISTS seen_addresses (
    email TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    display_name TEXT,
    display_name_source TEXT NOT NULL DEFAULT 'observed',
    times_sent_to INTEGER NOT NULL DEFAULT 0,
    times_sent_cc INTEGER NOT NULL DEFAULT 0,
    times_received_from INTEGER NOT NULL DEFAULT 0,
    times_received_cc INTEGER NOT NULL DEFAULT 0,
    first_seen_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    source TEXT NOT NULL DEFAULT 'local_observed',
    PRIMARY KEY (account_id, email)
);
CREATE INDEX IF NOT EXISTS idx_seen_addresses_email ON seen_addresses(email);
CREATE INDEX IF NOT EXISTS idx_seen_addresses_last_seen ON seen_addresses(account_id, last_seen_at DESC);

CREATE VIRTUAL TABLE IF NOT EXISTS seen_addresses_fts USING fts5(
    email, display_name,
    content=seen_addresses, content_rowid=rowid,
    tokenize='unicode61 tokenchars ''@._-''',
    prefix='2,3'
);

CREATE TRIGGER IF NOT EXISTS seen_addresses_fts_insert AFTER INSERT ON seen_addresses BEGIN
    INSERT INTO seen_addresses_fts(rowid, email, display_name)
    VALUES (NEW.rowid, NEW.email, COALESCE(NEW.display_name, ''));
END;
CREATE TRIGGER IF NOT EXISTS seen_addresses_fts_delete AFTER DELETE ON seen_addresses BEGIN
    INSERT INTO seen_addresses_fts(seen_addresses_fts, rowid, email, display_name)
    VALUES ('delete', OLD.rowid, OLD.email, COALESCE(OLD.display_name, ''));
END;
CREATE TRIGGER IF NOT EXISTS seen_addresses_fts_update AFTER UPDATE ON seen_addresses BEGIN
    INSERT INTO seen_addresses_fts(seen_addresses_fts, rowid, email, display_name)
    VALUES ('delete', OLD.rowid, OLD.email, COALESCE(OLD.display_name, ''));
    INSERT INTO seen_addresses_fts(rowid, email, display_name)
    VALUES (NEW.rowid, NEW.email, COALESCE(NEW.display_name, ''));
END;

CREATE TABLE IF NOT EXISTS contact_groups (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    source TEXT NOT NULL DEFAULT 'user',
    account_id TEXT,
    server_id TEXT,
    email TEXT,
    group_type TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_contact_groups_server
    ON contact_groups(account_id, server_id) WHERE server_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS contact_group_members (
    group_id TEXT NOT NULL REFERENCES contact_groups(id) ON DELETE CASCADE,
    member_type TEXT NOT NULL CHECK (member_type IN ('email', 'group')),
    member_value TEXT NOT NULL,
    PRIMARY KEY (group_id, member_type, member_value)
);

CREATE TABLE IF NOT EXISTS contact_photo_cache (
    email TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    content_hash TEXT NOT NULL,
    file_path TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    etag TEXT,
    fetched_at INTEGER NOT NULL DEFAULT (unixepoch()),
    last_accessed_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (email, account_id)
);

CREATE TABLE IF NOT EXISTS gal_cache (
    email TEXT NOT NULL,
    display_name TEXT,
    phone TEXT,
    company TEXT,
    title TEXT,
    department TEXT,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    cached_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (account_id, email)
);
CREATE INDEX IF NOT EXISTS idx_gal_cache_email ON gal_cache(email);

-- Provider-specific contact mapping tables

CREATE TABLE IF NOT EXISTS graph_contact_map (
    account_id TEXT NOT NULL,
    graph_contact_id TEXT NOT NULL,
    email TEXT NOT NULL,
    PRIMARY KEY (account_id, graph_contact_id, email),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_graph_contact_map_email ON graph_contact_map(email);

CREATE TABLE IF NOT EXISTS google_contact_map (
    resource_name TEXT NOT NULL,
    account_id TEXT NOT NULL,
    contact_email TEXT NOT NULL,
    PRIMARY KEY (resource_name, account_id),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_google_contact_map_email ON google_contact_map(contact_email);

CREATE TABLE IF NOT EXISTS google_other_contact_map (
    resource_name TEXT NOT NULL,
    account_id TEXT NOT NULL,
    contact_email TEXT NOT NULL,
    PRIMARY KEY (resource_name, account_id),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_google_other_contact_map_email ON google_other_contact_map(contact_email);

CREATE TABLE IF NOT EXISTS carddav_contact_map (
    uri TEXT NOT NULL,
    account_id TEXT NOT NULL,
    contact_email TEXT NOT NULL,
    etag TEXT,
    PRIMARY KEY (uri, account_id),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_carddav_contact_map_email ON carddav_contact_map(contact_email);

CREATE TABLE IF NOT EXISTS graph_contact_delta_tokens (
    account_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,
    delta_link TEXT NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    PRIMARY KEY (account_id, folder_id),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);

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

-- ── Calendar ────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS calendars (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    provider TEXT NOT NULL DEFAULT 'google',
    remote_id TEXT NOT NULL,
    display_name TEXT,
    color TEXT,
    is_primary INTEGER DEFAULT 0,
    is_visible INTEGER DEFAULT 1,
    sync_token TEXT,
    ctag TEXT,
    created_at INTEGER DEFAULT (unixepoch()),
    updated_at INTEGER DEFAULT (unixepoch()),
    sort_order INTEGER NOT NULL DEFAULT 0,
    is_default INTEGER NOT NULL DEFAULT 0,
    provider_id TEXT,
    UNIQUE(account_id, remote_id)
);
CREATE INDEX IF NOT EXISTS idx_calendars_account ON calendars(account_id);

CREATE TABLE IF NOT EXISTS calendar_events (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    google_event_id TEXT NOT NULL,
    summary TEXT,
    description TEXT,
    location TEXT,
    start_time INTEGER NOT NULL,
    end_time INTEGER NOT NULL,
    is_all_day INTEGER DEFAULT 0,
    status TEXT DEFAULT 'confirmed',
    organizer_email TEXT,
    attendees_json TEXT,
    html_link TEXT,
    updated_at INTEGER DEFAULT (unixepoch()),
    calendar_id TEXT REFERENCES calendars(id) ON DELETE CASCADE,
    remote_event_id TEXT,
    etag TEXT,
    ical_data TEXT,
    uid TEXT,
    title TEXT,
    timezone TEXT,
    recurrence_rule TEXT,
    organizer_name TEXT,
    rsvp_status TEXT,
    created_at INTEGER,
    availability TEXT,
    visibility TEXT,
    UNIQUE(account_id, google_event_id)
);
CREATE INDEX IF NOT EXISTS idx_cal_events_time ON calendar_events(account_id, start_time, end_time);
CREATE INDEX IF NOT EXISTS idx_cal_events_calendar ON calendar_events(calendar_id);

CREATE TABLE IF NOT EXISTS calendar_attendees (
    event_id TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    name TEXT,
    rsvp_status TEXT DEFAULT 'needs-action',
    is_organizer INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, event_id, email)
);
CREATE INDEX IF NOT EXISTS idx_calendar_attendees_event ON calendar_attendees(account_id, event_id);

CREATE TABLE IF NOT EXISTS calendar_reminders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    minutes_before INTEGER NOT NULL,
    method TEXT DEFAULT 'popup'
);
CREATE INDEX IF NOT EXISTS idx_calendar_reminders_event ON calendar_reminders(account_id, event_id);

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
    category TEXT NOT NULL,
    is_bundled INTEGER DEFAULT 1,
    delivery_enabled INTEGER DEFAULT 0,
    delivery_schedule TEXT,
    last_delivered_at INTEGER,
    created_at INTEGER DEFAULT (unixepoch()),
    UNIQUE(account_id, category)
);
CREATE INDEX IF NOT EXISTS idx_bundle_rules_account ON bundle_rules(account_id);

CREATE TABLE IF NOT EXISTS bundled_threads (
    account_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    category TEXT NOT NULL,
    held_until INTEGER,
    PRIMARY KEY (account_id, thread_id),
    FOREIGN KEY (account_id, thread_id) REFERENCES threads(account_id, id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_bundled_held ON bundled_threads(held_until);

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

-- ── Sync state ──────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS folder_sync_state (
    account_id TEXT NOT NULL,
    folder_path TEXT NOT NULL,
    uidvalidity INTEGER,
    last_uid INTEGER DEFAULT 0,
    modseq INTEGER,
    last_sync_at INTEGER,
    last_deletion_check_at INTEGER,
    PRIMARY KEY (account_id, folder_path),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS jmap_sync_state (
    account_id TEXT NOT NULL,
    type TEXT NOT NULL,
    state TEXT NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    shared_account_id TEXT,
    PRIMARY KEY (account_id, type),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_jmap_sync_state_shared
    ON jmap_sync_state(account_id, COALESCE(shared_account_id, ''), type);

CREATE TABLE IF NOT EXISTS graph_folder_delta_tokens (
    account_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,
    delta_link TEXT NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    PRIMARY KEY (account_id, folder_id),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS graph_shared_mailbox_delta_tokens (
    account_id TEXT NOT NULL,
    mailbox_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,
    delta_link TEXT NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    PRIMARY KEY (account_id, mailbox_id, folder_id),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS shared_mailbox_sync_state (
    account_id TEXT NOT NULL,
    mailbox_id TEXT NOT NULL,
    display_name TEXT,
    is_sync_enabled INTEGER NOT NULL DEFAULT 0,
    last_synced_at INTEGER,
    sync_error TEXT,
    email_address TEXT,
    PRIMARY KEY (account_id, mailbox_id),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS jmap_push_state (
    account_id TEXT NOT NULL PRIMARY KEY,
    push_state TEXT,
    ws_url TEXT,
    is_push_enabled INTEGER NOT NULL DEFAULT 0,
    last_connected_at INTEGER,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);

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

-- ── UI state ────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS thread_ui_state (
    thread_id TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    attachments_collapsed INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, thread_id)
);

-- ── Graph subscriptions ─────────────────────────────────────

CREATE TABLE IF NOT EXISTS graph_subscriptions (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    resource TEXT NOT NULL,
    notification_url TEXT NOT NULL,
    client_state TEXT NOT NULL,
    expiration_date_time TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_graph_subscriptions_account ON graph_subscriptions(account_id);
CREATE INDEX IF NOT EXISTS idx_graph_subscriptions_expiry ON graph_subscriptions(expiration_date_time);

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

    "#,
    },
];

/// Split SQL into individual statements, respecting BEGIN...END blocks
/// (e.g. inside CREATE TRIGGER).
fn split_statements(sql: &str) -> Vec<&str> {
    let mut stmts: Vec<&str> = Vec::new();
    let upper = sql.to_uppercase();
    let bytes = sql.as_bytes();
    let ubytes = upper.as_bytes();
    let len = bytes.len();
    let mut start = 0;
    let mut depth: u32 = 0;
    let mut i = 0;

    while i < len {
        // Skip -- line comments (everything until newline)
        if i + 1 < len && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // Skip /* block comments */ (may span multiple lines)
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2; // skip closing */
            continue;
        }

        // Skip single-quoted string literals (handle '' escapes)
        if bytes[i] == b'\'' {
            i += 1;
            while i < len {
                if bytes[i] == b'\'' {
                    if i + 1 < len && bytes[i + 1] == b'\'' {
                        i += 2; // escaped quote
                    } else {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            i += 1; // skip closing quote
            continue;
        }

        // Check for BEGIN keyword at word boundary
        if i + 5 <= len
            && &ubytes[i..i + 5] == b"BEGIN"
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric())
            && (i + 5 >= len || !bytes[i + 5].is_ascii_alphanumeric())
        {
            depth += 1;
        }
        // Check for END keyword at word boundary
        if i + 3 <= len
            && &ubytes[i..i + 3] == b"END"
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric())
            && (i + 3 >= len || !bytes[i + 3].is_ascii_alphanumeric())
            && depth > 0
        {
            depth -= 1;
        }

        if bytes[i] == b';' && depth == 0 {
            let trimmed = sql[start..i].trim();
            if !trimmed.is_empty() {
                stmts.push(trimmed);
            }
            start = i + 1;
        }

        i += 1;
    }

    let trimmed = sql[start..].trim();
    if !trimmed.is_empty() {
        stmts.push(trimmed);
    }

    stmts
}

/// Run all pending migrations. Called once at startup.
pub fn run_all(conn: &Connection) -> Result<(), String> {
    // Ensure migrations table exists
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _migrations (
           version INTEGER PRIMARY KEY,
           description TEXT,
           applied_at INTEGER DEFAULT (unixepoch())
         )",
    )
    .map_err(|e| format!("create _migrations: {e}"))?;

    // Collect applied versions
    let mut stmt = conn
        .prepare("SELECT version FROM _migrations ORDER BY version")
        .map_err(|e| format!("prepare: {e}"))?;
    let applied: std::collections::HashSet<u32> = stmt
        .query_map([], |row| row.get::<_, u32>("version"))
        .map_err(|e| format!("query: {e}"))?
        .filter_map(Result::ok)
        .collect();
    drop(stmt);

    // Detect stale pre-collapse databases. The old schema had versions 1-80;
    // the collapsed schema starts fresh at version 1 with different content.
    // If we see any version not in our MIGRATIONS array, this DB predates
    // the collapse and must be recreated.
    let known_versions: std::collections::HashSet<u32> =
        MIGRATIONS.iter().map(|m| m.version).collect();
    let stale_versions: Vec<u32> = applied
        .iter()
        .filter(|v| !known_versions.contains(v))
        .copied()
        .collect();
    if !stale_versions.is_empty() {
        return Err(format!(
            "Database was created with a previous schema version \
             (found migration versions {stale_versions:?} which are not in \
             the current schema). Delete ratatoskr.db and re-seed."
        ));
    }

    // ── Run pending migrations ─────────────────────────────────
    for m in MIGRATIONS {
        if applied.contains(&m.version) {
            continue;
        }

        log::info!("Running migration v{}: {}", m.version, m.description);

        let stmts = split_statements(m.sql);

        conn.execute_batch("BEGIN")
            .map_err(|e| format!("begin: {e}"))?;

        for s in &stmts {
            if let Err(e) = conn.execute_batch(s) {
                log::error!("Migration v{} failed: {e}", m.version);
                drop(conn.execute_batch("ROLLBACK"));
                return Err(format!("migration v{}: {e}", m.version));
            }
        }

        conn.execute(
            "INSERT OR IGNORE INTO _migrations (version, description) VALUES (?1, ?2)",
            rusqlite::params![m.version, m.description],
        )
        .map_err(|e| format!("record migration: {e}"))?;
        conn.execute_batch("COMMIT")
            .map_err(|e| format!("commit: {e}"))?;
    }

    log::info!("All migrations applied.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_handles_triggers() {
        let sql =
            "CREATE TRIGGER t AFTER INSERT ON m BEGIN INSERT INTO f VALUES(1); END; SELECT 1;";
        let stmts = split_statements(sql);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].contains("BEGIN"));
        assert!(stmts[0].contains("END"));
        assert_eq!(stmts[1], "SELECT 1");
    }

    #[test]
    fn migrations_run_on_fresh_db() {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("pragmas");
        run_all(&conn).expect("migrations should succeed");

        // Verify key tables exist
        let count: u32 = conn
            .query_row(
                "SELECT COUNT(*) AS cnt FROM sqlite_master WHERE type='table' AND name='threads'",
                [],
                |row| row.get("cnt"),
            )
            .expect("query");
        assert_eq!(count, 1);

        // Verify latest migration recorded
        let max_ver: u32 = conn
            .query_row("SELECT MAX(version) AS max_ver FROM _migrations", [], |row| row.get("max_ver"))
            .expect("query");
        let expected = MIGRATIONS.last().expect("at least one migration").version;
        assert_eq!(max_ver, expected);
    }

    #[test]
    fn split_skips_semicolons_in_line_comments() {
        let sql = r#"
            ALTER TABLE t ADD COLUMN c TEXT;
            -- NULL c = primary; non-NULL = shared.
            CREATE INDEX idx ON t(c);
        "#;
        let stmts = split_statements(sql);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].starts_with("ALTER"));
        assert!(stmts[1].starts_with("-- NULL"));
    }

    #[test]
    fn split_skips_semicolons_in_block_comments() {
        let sql = "SELECT 1; /* a; b; c */ SELECT 2;";
        let stmts = split_statements(sql);
        assert_eq!(stmts.len(), 2);
        assert_eq!(stmts[0], "SELECT 1");
        assert!(stmts[1].contains("SELECT 2"));
    }

    #[test]
    fn split_skips_semicolons_in_string_literals() {
        let sql = "INSERT INTO t VALUES('a;b'); SELECT 1;";
        let stmts = split_statements(sql);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].contains("'a;b'"));
        assert_eq!(stmts[1], "SELECT 1");
    }
}
