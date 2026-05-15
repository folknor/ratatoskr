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

CREATE TABLE IF NOT EXISTS thread_bundles (
    account_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    bundle TEXT NOT NULL,
    is_manual INTEGER DEFAULT 0,
    created_at INTEGER DEFAULT (unixepoch()),
    PRIMARY KEY (account_id, thread_id),
    FOREIGN KEY (account_id, thread_id) REFERENCES threads(account_id, id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_thread_bundles ON thread_bundles(account_id, bundle);

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
    -- Unix milliseconds since epoch. Invariant across providers:
    -- JMAP/Gmail/Graph write ms natively; IMAP normalizes its
    -- seconds-scale INTERNALDATE/header `Date` value to ms in
    -- `imap::parse::parse_message`. Eviction and prefetch retention
    -- queries assume ms and multiply seconds-scale window cutoffs by
    -- 1000 to match.
    date INTEGER NOT NULL,
    is_read INTEGER DEFAULT 0,
    is_starred INTEGER DEFAULT 0,
    is_replied INTEGER NOT NULL DEFAULT 0,
    is_forwarded INTEGER NOT NULL DEFAULT 0,
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
    -- Set to 1 when the message includes an iMIP / iCalendar (text/calendar)
    -- payload. Drives meeting-invite UI affordances (calendar pill on the
    -- thread card, RSVP buttons in the reading pane). Populated at message-
    -- insert time from the attachment list.
    has_meeting_invite INTEGER NOT NULL DEFAULT 0,
    -- iCalendar METHOD parameter (REQUEST/REPLY/CANCEL/COUNTER). Useful for
    -- the UI to differentiate fresh invitations from RSVP responses without
    -- re-parsing the iCal payload.
    meeting_invite_method TEXT,
    -- iCalendar UID, used to match this message to a calendar event row
    -- after CalDAV/Graph/JMAP/Gmail calendar sync stores the event.
    meeting_invite_uid TEXT,
    PRIMARY KEY (account_id, id),
    FOREIGN KEY (account_id, thread_id) REFERENCES threads(account_id, id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_messages_thread ON messages(account_id, thread_id, date ASC);
CREATE INDEX IF NOT EXISTS idx_messages_date ON messages(account_id, date DESC);
CREATE INDEX IF NOT EXISTS idx_messages_from ON messages(from_address);
CREATE INDEX IF NOT EXISTS idx_messages_imap_uid ON messages(account_id, imap_folder, imap_uid);
CREATE INDEX IF NOT EXISTS idx_messages_message_id ON messages(message_id_header);
CREATE INDEX IF NOT EXISTS idx_messages_invite_uid ON messages(account_id, meeting_invite_uid)
    WHERE meeting_invite_uid IS NOT NULL;

-- ── Attachments ─────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS attachments (
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    filename TEXT,
    mime_type TEXT,
    size INTEGER,
    remote_attachment_id TEXT,
    content_id TEXT,
    is_inline INTEGER DEFAULT 0,
    content_hash BLOB,
    -- Phase 7: pointer to attachment_extracted_text.extracted_at for
    -- the row keyed by content_hash. NULL means "not yet extracted."
    -- Backfill scan joins attachment_blobs to filter rows whose bytes
    -- are still live in the pack store.
    text_indexed_at INTEGER,
    FOREIGN KEY (account_id, message_id) REFERENCES messages(account_id, id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_attachments_message ON attachments(account_id, message_id);
CREATE INDEX IF NOT EXISTS idx_attachments_cid ON attachments(content_id);
CREATE INDEX IF NOT EXISTS idx_attachments_content_hash ON attachments(content_hash);
-- Phase 7 / attachments roadmap Phase 3: backfill scan target. Joined
-- against attachment_blobs (tombstoned_at IS NULL) to filter rows
-- whose bytes are still in the pack store.
CREATE INDEX IF NOT EXISTS idx_attachments_text_indexed_at
    ON attachments(text_indexed_at)
    WHERE text_indexed_at IS NULL;

-- Phase 7: attachment text extraction store, keyed by content_hash so two
-- attachments with identical bytes share one row and so the row survives
-- attachment-cache eviction (PackStore tombstones the blob in
-- `attachment_blobs`; `attachments.content_hash` is untouched).
-- status taxonomy (string-tagged so future-extensible without enum migration):
--   permanent (no retry): 'indexed', 'skipped:opaque', 'skipped:encrypted',
--     'skipped:oversize', 'skipped:encoding', 'skipped:empty',
--     'skipped:ocr', 'skipped:unknown_mime', 'skipped:privacy',
--     'skipped:zipbomb'.
--   retry-eligible: 'failed:transient', 'skipped:bytes_gone',
--     'skipped:timeout'.
-- Worker pre-flight skips only on permanent statuses; retry-eligible rows
-- re-extract on next enqueue.
CREATE TABLE IF NOT EXISTS attachment_extracted_text (
    content_hash    BLOB PRIMARY KEY,
    mime_type       TEXT,
    extracted_text  TEXT,
    status          TEXT NOT NULL,
    extracted_at    INTEGER NOT NULL,
    schema_version  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_attachment_extracted_text_schema_version
    ON attachment_extracted_text(schema_version);

-- Phase 2 (attachments roadmap): pack-store index. Bytes live in
-- `attachment_packs/data-NNNNNN.pack`; this table is the lookup from a
-- BLAKE3 content hash to its (pack, offset, length) location. No
-- refcount column - the count of referencing `attachments` rows is the
-- source of truth (see problem-statement.md "Reference counts are
-- derived, not stored"). `tombstoned_at` non-NULL means the blob is
-- logically evicted and reads must refuse it even if the bytes are
-- still in the pack.
CREATE TABLE IF NOT EXISTS attachment_blobs (
    content_hash  BLOB    PRIMARY KEY,
    pack_file_id  INTEGER NOT NULL,
    offset        INTEGER NOT NULL,
    length        INTEGER NOT NULL,
    written_at    INTEGER NOT NULL,
    last_read_at  INTEGER,
    tombstoned_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_attachment_blobs_tombstoned
    ON attachment_blobs(tombstoned_at);

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

-- ── UI state ────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS thread_ui_state (
    thread_id TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    attachments_collapsed INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, thread_id)
);
