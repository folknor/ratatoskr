use rusqlite::Connection;

struct Migration {
    version: u32,
    description: &'static str,
    sql: &'static str,
}

static MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        description: "Initial schema",
        sql: r#"
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
        updated_at INTEGER DEFAULT (unixepoch())
      );
      CREATE TABLE IF NOT EXISTS labels (
        id TEXT NOT NULL,
        account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        type TEXT NOT NULL,
        color_bg TEXT,
        color_fg TEXT,
        visible INTEGER DEFAULT 1,
        sort_order INTEGER DEFAULT 0,
        PRIMARY KEY (account_id, id)
      );
      CREATE INDEX IF NOT EXISTS idx_labels_account ON labels(account_id);
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
        PRIMARY KEY (account_id, id)
      );
      CREATE INDEX IF NOT EXISTS idx_threads_date ON threads(account_id, last_message_at DESC);
      CREATE INDEX IF NOT EXISTS idx_threads_snoozed ON threads(is_snoozed, snooze_until);
      CREATE TABLE IF NOT EXISTS thread_labels (
        thread_id TEXT NOT NULL,
        account_id TEXT NOT NULL,
        label_id TEXT NOT NULL,
        PRIMARY KEY (account_id, thread_id, label_id),
        FOREIGN KEY (account_id, thread_id) REFERENCES threads(account_id, id) ON DELETE CASCADE
      );
      CREATE INDEX IF NOT EXISTS idx_thread_labels_label ON thread_labels(account_id, label_id);
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
        body_html TEXT,
        body_text TEXT,
        body_cached INTEGER DEFAULT 0,
        raw_size INTEGER,
        internal_date INTEGER,
        PRIMARY KEY (account_id, id),
        FOREIGN KEY (account_id, thread_id) REFERENCES threads(account_id, id) ON DELETE CASCADE
      );
      CREATE INDEX IF NOT EXISTS idx_messages_thread ON messages(account_id, thread_id, date ASC);
      CREATE INDEX IF NOT EXISTS idx_messages_date ON messages(account_id, date DESC);
      CREATE INDEX IF NOT EXISTS idx_messages_from ON messages(from_address);
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
        FOREIGN KEY (account_id, message_id) REFERENCES messages(account_id, id) ON DELETE CASCADE
      );
      CREATE INDEX IF NOT EXISTS idx_attachments_message ON attachments(account_id, message_id);
      CREATE INDEX IF NOT EXISTS idx_attachments_cid ON attachments(content_id);
      CREATE TABLE IF NOT EXISTS contacts (
        id TEXT PRIMARY KEY,
        email TEXT NOT NULL UNIQUE,
        display_name TEXT,
        avatar_url TEXT,
        frequency INTEGER DEFAULT 1,
        last_contacted_at INTEGER,
        created_at INTEGER DEFAULT (unixepoch()),
        updated_at INTEGER DEFAULT (unixepoch())
      );
      CREATE INDEX IF NOT EXISTS idx_contacts_email ON contacts(email);
      CREATE INDEX IF NOT EXISTS idx_contacts_frequency ON contacts(frequency DESC);
      CREATE TABLE IF NOT EXISTS signatures (
        id TEXT PRIMARY KEY,
        account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        body_html TEXT NOT NULL,
        is_default INTEGER DEFAULT 0,
        sort_order INTEGER DEFAULT 0,
        created_at INTEGER DEFAULT (unixepoch())
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
        created_at INTEGER DEFAULT (unixepoch())
      );
      CREATE INDEX IF NOT EXISTS idx_scheduled_status ON scheduled_emails(status, scheduled_at);
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
        ('font_size', 'default');
      CREATE TABLE IF NOT EXISTS _migrations (
        version INTEGER PRIMARY KEY,
        description TEXT,
        applied_at INTEGER DEFAULT (unixepoch())
      );
    "#,
    },
    Migration {
        version: 2,
        description: "Full-text search",
        sql: r#"
      CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
        subject, from_name, from_address, body_text, snippet,
        content='messages', content_rowid='rowid', tokenize='trigram'
      );
      CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
        INSERT INTO messages_fts(rowid, subject, from_name, from_address, body_text, snippet)
        VALUES (new.rowid, new.subject, new.from_name, new.from_address, new.body_text, new.snippet);
      END;
      CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
        INSERT INTO messages_fts(messages_fts, rowid, subject, from_name, from_address, body_text, snippet)
        VALUES ('delete', old.rowid, old.subject, old.from_name, old.from_address, old.body_text, old.snippet);
      END;
      CREATE TRIGGER IF NOT EXISTS messages_au AFTER UPDATE ON messages BEGIN
        INSERT INTO messages_fts(messages_fts, rowid, subject, from_name, from_address, body_text, snippet)
        VALUES ('delete', old.rowid, old.subject, old.from_name, old.from_address, old.body_text, old.snippet);
        INSERT INTO messages_fts(rowid, subject, from_name, from_address, body_text, snippet)
        VALUES (new.rowid, new.subject, new.from_name, new.from_address, new.body_text, new.snippet);
      END;
    "#,
    },
    Migration {
        version: 3,
        description: "Add List-Unsubscribe header storage",
        sql: "ALTER TABLE messages ADD COLUMN list_unsubscribe TEXT;",
    },
    Migration {
        version: 4,
        description: "Filter rules, templates, image allowlist",
        sql: r#"
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
      INSERT OR IGNORE INTO settings (key, value) VALUES ('block_remote_images', 'true');
    "#,
    },
    Migration {
        version: 5,
        description: "Pin support, AI cache, thread categories, calendar events",
        sql: r#"
      ALTER TABLE threads ADD COLUMN is_pinned INTEGER DEFAULT 0;
      CREATE INDEX idx_threads_pinned ON threads(account_id, is_pinned DESC, last_message_at DESC);
      CREATE TABLE ai_cache (
        id TEXT PRIMARY KEY,
        account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        thread_id TEXT NOT NULL,
        type TEXT NOT NULL,
        content TEXT NOT NULL,
        created_at INTEGER DEFAULT (unixepoch()),
        UNIQUE(account_id, thread_id, type)
      );
      CREATE INDEX idx_ai_cache_lookup ON ai_cache(account_id, thread_id, type);
      CREATE TABLE thread_categories (
        account_id TEXT NOT NULL,
        thread_id TEXT NOT NULL,
        category TEXT NOT NULL,
        is_manual INTEGER DEFAULT 0,
        created_at INTEGER DEFAULT (unixepoch()),
        PRIMARY KEY (account_id, thread_id),
        FOREIGN KEY (account_id, thread_id) REFERENCES threads(account_id, id) ON DELETE CASCADE
      );
      CREATE INDEX idx_thread_categories_cat ON thread_categories(account_id, category);
      CREATE TABLE calendar_events (
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
        UNIQUE(account_id, google_event_id)
      );
      CREATE INDEX idx_cal_events_time ON calendar_events(account_id, start_time, end_time);
      ALTER TABLE contacts ADD COLUMN first_contacted_at INTEGER;
      ALTER TABLE attachments ADD COLUMN cached_at INTEGER;
      ALTER TABLE attachments ADD COLUMN cache_size INTEGER;
      INSERT OR IGNORE INTO settings (key, value) VALUES
        ('ai_enabled', 'true'),
        ('ai_auto_categorize', 'true'),
        ('ai_auto_summarize', 'true'),
        ('contact_sidebar_visible', 'true'),
        ('attachment_cache_max_mb', '500'),
        ('calendar_enabled', 'false');
    "#,
    },
    Migration {
        version: 6,
        description: "Follow-up reminders, smart notifications, unsubscribe, bundling",
        sql: r#"
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
      CREATE INDEX idx_followup_status ON follow_up_reminders(status, remind_at);
      CREATE INDEX idx_followup_thread ON follow_up_reminders(account_id, thread_id);
      CREATE TABLE IF NOT EXISTS notification_vips (
        id TEXT PRIMARY KEY,
        account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        email_address TEXT NOT NULL,
        display_name TEXT,
        created_at INTEGER DEFAULT (unixepoch()),
        UNIQUE(account_id, email_address)
      );
      CREATE INDEX idx_notification_vips ON notification_vips(account_id, email_address);
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
      CREATE INDEX idx_unsub_account ON unsubscribe_actions(account_id, status);
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
      CREATE INDEX idx_bundle_rules_account ON bundle_rules(account_id);
      CREATE TABLE IF NOT EXISTS bundled_threads (
        account_id TEXT NOT NULL,
        thread_id TEXT NOT NULL,
        category TEXT NOT NULL,
        held_until INTEGER,
        PRIMARY KEY (account_id, thread_id),
        FOREIGN KEY (account_id, thread_id) REFERENCES threads(account_id, id) ON DELETE CASCADE
      );
      CREATE INDEX idx_bundled_held ON bundled_threads(held_until);
      ALTER TABLE messages ADD COLUMN list_unsubscribe_post TEXT;
      INSERT OR IGNORE INTO settings (key, value) VALUES
        ('smart_notifications', 'true'),
        ('notify_categories', 'Primary'),
        ('auto_archive_after_unsubscribe', 'true');
    "#,
    },
    Migration {
        version: 7,
        description: "Send-as aliases",
        sql: r#"
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
      CREATE INDEX idx_send_as_account ON send_as_aliases(account_id);
    "#,
    },
    Migration {
        version: 8,
        description: "Smart folders",
        sql: r#"
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
      CREATE INDEX idx_smart_folders_account ON smart_folders(account_id);
      INSERT INTO smart_folders (id, account_id, name, query, icon, sort_order, is_default) VALUES
        ('sf-unread', NULL, 'Unread', 'is:unread', 'MailOpen', 0, 1),
        ('sf-attachments', NULL, 'Has Attachments', 'has:attachment', 'Paperclip', 1, 1),
        ('sf-starred-recent', NULL, 'Starred This Week', 'is:starred after:__LAST_7_DAYS__', 'Star', 2, 1);
    "#,
    },
    Migration {
        version: 9,
        description: "Email authentication results",
        sql: "ALTER TABLE messages ADD COLUMN auth_results TEXT;",
    },
    Migration {
        version: 10,
        description: "Mute thread support",
        sql: r#"
      ALTER TABLE threads ADD COLUMN is_muted INTEGER DEFAULT 0;
      CREATE INDEX idx_threads_muted ON threads(account_id, is_muted);
    "#,
    },
    Migration {
        version: 11,
        description: "Phishing detection cache and allowlist",
        sql: r#"
      CREATE TABLE IF NOT EXISTS link_scan_results (
        message_id TEXT NOT NULL,
        account_id TEXT NOT NULL,
        result_json TEXT NOT NULL,
        scanned_at INTEGER DEFAULT (unixepoch()),
        PRIMARY KEY (account_id, message_id)
      );
      CREATE TABLE IF NOT EXISTS phishing_allowlist (
        id TEXT PRIMARY KEY,
        account_id TEXT NOT NULL,
        sender_address TEXT NOT NULL,
        created_at INTEGER DEFAULT (unixepoch()),
        UNIQUE(account_id, sender_address)
      );
      INSERT OR IGNORE INTO settings (key, value) VALUES
        ('phishing_detection_enabled', 'true'),
        ('phishing_sensitivity', 'default');
    "#,
    },
    Migration {
        version: 12,
        description: "Quick steps",
        sql: r#"
      CREATE TABLE IF NOT EXISTS quick_steps (
        id TEXT PRIMARY KEY,
        account_id TEXT NOT NULL,
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
      CREATE INDEX idx_quick_steps_account ON quick_steps(account_id);
    "#,
    },
    Migration {
        version: 13,
        description: "Contact notes",
        sql: "ALTER TABLE contacts ADD COLUMN notes TEXT;",
    },
    Migration {
        version: 14,
        description: "IMAP/SMTP provider support",
        sql: r#"
      ALTER TABLE accounts ADD COLUMN provider TEXT DEFAULT 'gmail_api';
      ALTER TABLE accounts ADD COLUMN imap_host TEXT;
      ALTER TABLE accounts ADD COLUMN imap_port INTEGER;
      ALTER TABLE accounts ADD COLUMN imap_security TEXT;
      ALTER TABLE accounts ADD COLUMN smtp_host TEXT;
      ALTER TABLE accounts ADD COLUMN smtp_port INTEGER;
      ALTER TABLE accounts ADD COLUMN smtp_security TEXT;
      ALTER TABLE accounts ADD COLUMN auth_method TEXT DEFAULT 'oauth';
      ALTER TABLE accounts ADD COLUMN imap_password TEXT;
      ALTER TABLE messages ADD COLUMN message_id_header TEXT;
      ALTER TABLE messages ADD COLUMN references_header TEXT;
      ALTER TABLE messages ADD COLUMN in_reply_to_header TEXT;
      ALTER TABLE messages ADD COLUMN imap_uid INTEGER;
      ALTER TABLE messages ADD COLUMN imap_folder TEXT;
      ALTER TABLE labels ADD COLUMN imap_folder_path TEXT;
      ALTER TABLE labels ADD COLUMN imap_special_use TEXT;
      ALTER TABLE attachments ADD COLUMN imap_part_id TEXT;
      CREATE TABLE IF NOT EXISTS folder_sync_state (
        account_id TEXT NOT NULL,
        folder_path TEXT NOT NULL,
        uidvalidity INTEGER,
        last_uid INTEGER DEFAULT 0,
        modseq INTEGER,
        last_sync_at INTEGER,
        PRIMARY KEY (account_id, folder_path),
        FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
      );
      CREATE INDEX IF NOT EXISTS idx_messages_imap_uid ON messages(account_id, imap_folder, imap_uid);
      CREATE INDEX IF NOT EXISTS idx_messages_message_id ON messages(message_id_header);
    "#,
    },
    Migration {
        version: 15,
        description: "OAuth2 provider support for IMAP/SMTP",
        sql: r#"
      ALTER TABLE accounts ADD COLUMN oauth_provider TEXT;
      ALTER TABLE accounts ADD COLUMN oauth_client_id TEXT;
      ALTER TABLE accounts ADD COLUMN oauth_client_secret TEXT;
    "#,
    },
    Migration {
        version: 16,
        description: "Optional IMAP/SMTP username override",
        sql: "ALTER TABLE accounts ADD COLUMN imap_username TEXT;",
    },
    Migration {
        version: 17,
        description: "Offline mode: pending operations queue and local drafts",
        sql: r#"
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
        sync_status TEXT DEFAULT 'pending'
      );
    "#,
    },
    Migration {
        version: 18,
        description: "AI auto-drafts writing style profiles and task manager",
        sql: r#"
      CREATE TABLE IF NOT EXISTS writing_style_profiles (
        id TEXT PRIMARY KEY,
        account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        profile_text TEXT NOT NULL,
        sample_count INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER DEFAULT (unixepoch()),
        updated_at INTEGER DEFAULT (unixepoch()),
        UNIQUE(account_id)
      );
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
      INSERT OR IGNORE INTO settings (key, value) VALUES
        ('ai_auto_draft_enabled', 'true'),
        ('ai_writing_style_enabled', 'true');
    "#,
    },
    Migration {
        version: 19,
        description: "CalDAV calendar integration",
        sql: r#"
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
        UNIQUE(account_id, remote_id)
      );
      CREATE INDEX IF NOT EXISTS idx_calendars_account ON calendars(account_id);
      ALTER TABLE calendar_events ADD COLUMN calendar_id TEXT REFERENCES calendars(id) ON DELETE CASCADE;
      ALTER TABLE calendar_events ADD COLUMN remote_event_id TEXT;
      ALTER TABLE calendar_events ADD COLUMN etag TEXT;
      ALTER TABLE calendar_events ADD COLUMN ical_data TEXT;
      ALTER TABLE calendar_events ADD COLUMN uid TEXT;
      CREATE INDEX IF NOT EXISTS idx_cal_events_calendar ON calendar_events(calendar_id);
      ALTER TABLE accounts ADD COLUMN caldav_url TEXT;
      ALTER TABLE accounts ADD COLUMN caldav_username TEXT;
      ALTER TABLE accounts ADD COLUMN caldav_password TEXT;
      ALTER TABLE accounts ADD COLUMN caldav_principal_url TEXT;
      ALTER TABLE accounts ADD COLUMN caldav_home_url TEXT;
      ALTER TABLE accounts ADD COLUMN calendar_provider TEXT;
    "#,
    },
    Migration {
        version: 20,
        description: "Fix IMAP attachment part IDs and trigger resync",
        sql: r#"
      DELETE FROM attachments
        WHERE account_id IN (SELECT id FROM accounts WHERE provider = 'imap');
      DELETE FROM folder_sync_state
        WHERE account_id IN (SELECT id FROM accounts WHERE provider = 'imap');
    "#,
    },
    Migration {
        version: 21,
        description: "Force IMAP full resync for corrected attachment part IDs",
        sql: r#"
      UPDATE accounts SET history_id = NULL WHERE provider = 'imap';
      DELETE FROM folder_sync_state
        WHERE account_id IN (SELECT id FROM accounts WHERE provider = 'imap');
      DELETE FROM attachments
        WHERE account_id IN (SELECT id FROM accounts WHERE provider = 'imap');
    "#,
    },
    Migration {
        version: 22,
        description: "Add smart label rules table for AI-powered auto-labeling",
        sql: r#"
      CREATE TABLE smart_label_rules (
        id TEXT PRIMARY KEY,
        account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        label_id TEXT NOT NULL,
        ai_description TEXT NOT NULL,
        criteria_json TEXT,
        is_enabled INTEGER DEFAULT 1,
        sort_order INTEGER DEFAULT 0,
        created_at INTEGER DEFAULT (unixepoch())
      );
      CREATE INDEX idx_smart_label_rules_account ON smart_label_rules(account_id);
    "#,
    },
    Migration {
        version: 23,
        description: "Accept self-signed certificates for IMAP/SMTP",
        sql: "ALTER TABLE accounts ADD COLUMN accept_invalid_certs INTEGER DEFAULT 0;",
    },
    Migration {
        version: 24,
        description: "Post-release consolidation: JMAP, Graph delta tokens, FTS5 removal, auth normalization",
        sql: r#"
            -- Normalize auth_method 'oauth' to 'oauth2'
            UPDATE accounts SET auth_method = 'oauth2' WHERE auth_method = 'oauth';

            -- JMAP provider support
            ALTER TABLE accounts ADD COLUMN jmap_url TEXT;
            CREATE TABLE IF NOT EXISTS jmap_sync_state (
                account_id TEXT NOT NULL,
                type TEXT NOT NULL,
                state TEXT NOT NULL,
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                PRIMARY KEY (account_id, type),
                FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
            );

            -- Graph provider per-folder delta tokens
            CREATE TABLE IF NOT EXISTS graph_folder_delta_tokens (
                account_id TEXT NOT NULL,
                folder_id TEXT NOT NULL,
                delta_link TEXT NOT NULL,
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                PRIMARY KEY (account_id, folder_id),
                FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
            );

            -- Drop FTS5 (replaced by tantivy) and unused body columns (moved to body store)
            DROP TRIGGER IF EXISTS messages_ai;
            DROP TRIGGER IF EXISTS messages_ad;
            DROP TRIGGER IF EXISTS messages_au;
            DROP TABLE IF EXISTS messages_fts;
            ALTER TABLE messages DROP COLUMN body_html;
            ALTER TABLE messages DROP COLUMN body_text;
        "#,
    },
    Migration {
        version: 25,
        description: "Add content_hash column to attachments for content-addressed caching",
        sql: r#"
            ALTER TABLE attachments ADD COLUMN content_hash TEXT;
            CREATE INDEX IF NOT EXISTS idx_attachments_content_hash ON attachments(content_hash);
        "#,
    },
    Migration {
        version: 26,
        description: "Store OAuth token URL with IMAP accounts",
        sql: "ALTER TABLE accounts ADD COLUMN oauth_token_url TEXT;",
    },
    Migration {
        version: 28,
        description: "Track initial sync completion explicitly",
        sql: r#"
            ALTER TABLE accounts ADD COLUMN initial_sync_completed INTEGER NOT NULL DEFAULT 0;

            UPDATE accounts
            SET initial_sync_completed = 1
            WHERE
                (provider = 'gmail_api' AND history_id IS NOT NULL)
                OR (
                    provider = 'imap'
                    AND (
                        history_id IS NOT NULL
                        OR EXISTS (
                            SELECT 1 FROM folder_sync_state f WHERE f.account_id = accounts.id
                        )
                    )
                )
                OR (
                    provider = 'jmap'
                    AND EXISTS (
                        SELECT 1 FROM jmap_sync_state s
                        WHERE s.account_id = accounts.id AND s.type = 'Email'
                    )
                )
                OR (
                    provider = 'graph'
                    AND EXISTS (
                        SELECT 1 FROM graph_folder_delta_tokens g WHERE g.account_id = accounts.id
                    )
                );
        "#,
    },
    Migration {
        version: 29,
        description: "Backfill per-account OAuth credentials from global settings",
        sql: r#"
            UPDATE accounts
            SET oauth_client_id = (SELECT value FROM settings WHERE key = 'google_client_id'),
                oauth_client_secret = (SELECT value FROM settings WHERE key = 'google_client_secret')
            WHERE provider = 'gmail_api'
              AND (oauth_client_id IS NULL OR oauth_client_id = '');

            UPDATE accounts
            SET oauth_client_id = (SELECT value FROM settings WHERE key = 'microsoft_client_id')
            WHERE provider = 'graph'
              AND (oauth_client_id IS NULL OR oauth_client_id = '');
        "#,
    },
    Migration {
        version: 30,
        description: "Remove orphaned global OAuth credential settings",
        sql: r#"
            DELETE FROM settings WHERE key IN (
                'google_client_id',
                'google_client_secret',
                'microsoft_client_id'
            );
        "#,
    },
    Migration {
        version: 31,
        description: "Seen addresses table for auto-collected contacts",
        sql: r#"
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
        "#,
    },
    Migration {
        version: 32,
        description: "FTS5 prefix search index for contacts",
        sql: r#"
            CREATE VIRTUAL TABLE IF NOT EXISTS contacts_fts USING fts5(
                email,
                display_name,
                content=contacts,
                content_rowid=rowid,
                tokenize="unicode61 tokenchars '@._-'",
                prefix='2,3'
            );

            INSERT INTO contacts_fts(contacts_fts) VALUES('rebuild');

            CREATE TRIGGER IF NOT EXISTS contacts_fts_insert AFTER INSERT ON contacts
            BEGIN
                INSERT INTO contacts_fts(rowid, email, display_name)
                    VALUES (NEW.rowid, NEW.email, COALESCE(NEW.display_name, ''));
            END;

            CREATE TRIGGER IF NOT EXISTS contacts_fts_delete AFTER DELETE ON contacts
            BEGIN
                INSERT INTO contacts_fts(contacts_fts, rowid, email, display_name)
                    VALUES ('delete', OLD.rowid, OLD.email, COALESCE(OLD.display_name, ''));
            END;

            CREATE TRIGGER IF NOT EXISTS contacts_fts_update AFTER UPDATE ON contacts
            BEGIN
                INSERT INTO contacts_fts(contacts_fts, rowid, email, display_name)
                    VALUES ('delete', OLD.rowid, OLD.email, COALESCE(OLD.display_name, ''));
                INSERT INTO contacts_fts(rowid, email, display_name)
                    VALUES (NEW.rowid, NEW.email, COALESCE(NEW.display_name, ''));
            END;
        "#,
    },
    Migration {
        version: 33,
        description: "Graph contact sync: source tracking, contact map, delta tokens",
        sql: r#"
            ALTER TABLE contacts ADD COLUMN source TEXT NOT NULL DEFAULT 'user';
            ALTER TABLE contacts ADD COLUMN display_name_overridden INTEGER NOT NULL DEFAULT 0;

            CREATE TABLE IF NOT EXISTS graph_contact_map (
                account_id TEXT NOT NULL,
                graph_contact_id TEXT NOT NULL,
                email TEXT NOT NULL,
                PRIMARY KEY (account_id, graph_contact_id, email),
                FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_graph_contact_map_email ON graph_contact_map(email);

            CREATE TABLE IF NOT EXISTS graph_contact_delta_tokens (
                account_id TEXT NOT NULL,
                folder_id TEXT NOT NULL,
                delta_link TEXT NOT NULL,
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                PRIMARY KEY (account_id, folder_id),
                FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
            );
        "#,
    },
    Migration {
        version: 34,
        description: "Local contact groups with nested group support",
        sql: r#"
            CREATE TABLE IF NOT EXISTS contact_groups (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                updated_at INTEGER NOT NULL DEFAULT (unixepoch())
            );

            CREATE TABLE IF NOT EXISTS contact_group_members (
                group_id TEXT NOT NULL REFERENCES contact_groups(id) ON DELETE CASCADE,
                member_type TEXT NOT NULL CHECK (member_type IN ('email', 'group')),
                member_value TEXT NOT NULL,
                PRIMARY KEY (group_id, member_type, member_value)
            );
        "#,
    },
    Migration {
        version: 35,
        description: "MDN suppression: mdn_requested flag and read receipt policy table",
        sql: r#"
            ALTER TABLE messages ADD COLUMN mdn_requested INTEGER NOT NULL DEFAULT 0;

            CREATE TABLE IF NOT EXISTS read_receipt_policy (
                id TEXT PRIMARY KEY,
                account_id TEXT NOT NULL,
                scope TEXT NOT NULL,
                policy TEXT NOT NULL DEFAULT 'never',
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                UNIQUE(account_id, scope)
            );

            INSERT OR IGNORE INTO settings (key, value) VALUES ('default_read_receipt_policy', 'never');
        "#,
    },
    Migration {
        version: 36,
        description: "Master category list for provider-agnostic color categories",
        sql: r#"
            CREATE TABLE IF NOT EXISTS categories (
                id TEXT PRIMARY KEY,
                account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
                display_name TEXT NOT NULL,
                color_preset TEXT,
                color_bg TEXT,
                color_fg TEXT,
                provider_id TEXT,
                sync_state TEXT NOT NULL DEFAULT 'synced',
                sort_order INTEGER NOT NULL DEFAULT 0,
                UNIQUE(account_id, display_name)
            );

            CREATE INDEX IF NOT EXISTS idx_categories_account ON categories(account_id);
        "#,
    },
    Migration {
        version: 37,
        description: "Message reactions table for exchange/gmail/imap emoji reactions",
        sql: r#"
            CREATE TABLE IF NOT EXISTS message_reactions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                message_id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                reactor_email TEXT NOT NULL,
                reactor_name TEXT,
                reaction_type TEXT NOT NULL,
                reacted_at INTEGER,
                source TEXT NOT NULL,
                UNIQUE(message_id, account_id, reactor_email, reaction_type)
            );

            CREATE INDEX IF NOT EXISTS idx_message_reactions_message ON message_reactions(message_id, account_id);
        "#,
    },
    Migration {
        version: 38,
        description: "BIMI indicator cache for sender domain logo lookups",
        sql: r#"
            CREATE TABLE IF NOT EXISTS bimi_cache (
                domain TEXT PRIMARY KEY,
                has_bimi INTEGER NOT NULL DEFAULT 0,
                logo_uri TEXT,
                authority_uri TEXT,
                fetched_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                expires_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_bimi_cache_expires ON bimi_cache(expires_at);
        "#,
    },
    Migration {
        version: 39,
        description: "Cloud attachments table for large file upload tracking",
        sql: r#"
            CREATE TABLE IF NOT EXISTS cloud_attachments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                message_id TEXT,
                account_id TEXT NOT NULL,
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
        "#,
    },
    Migration {
        version: 40,
        description: "Mentions table and is_mentioned flag on messages",
        sql: r#"
            ALTER TABLE messages ADD COLUMN is_mentioned INTEGER NOT NULL DEFAULT 0;
            CREATE INDEX IF NOT EXISTS idx_messages_is_mentioned ON messages(is_mentioned) WHERE is_mentioned = 1;

            CREATE TABLE IF NOT EXISTS mentions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                message_id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                mention_id TEXT,
                mentioned_name TEXT,
                mentioned_address TEXT NOT NULL,
                created_by_name TEXT,
                created_by_address TEXT,
                created_at INTEGER,
                UNIQUE(message_id, account_id, mentioned_address)
            );
            CREATE INDEX IF NOT EXISTS idx_mentions_message ON mentions(message_id, account_id);
        "#,
    },
    Migration {
        version: 41,
        description: "Signature sync columns for bidirectional provider sync",
        sql: r#"
            ALTER TABLE signatures ADD COLUMN server_id TEXT;
            ALTER TABLE signatures ADD COLUMN body_text TEXT;
            ALTER TABLE signatures ADD COLUMN is_reply_default INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE signatures ADD COLUMN source TEXT NOT NULL DEFAULT 'local';
            ALTER TABLE signatures ADD COLUMN last_synced_at INTEGER;
            ALTER TABLE signatures ADD COLUMN server_html_hash TEXT;
            CREATE UNIQUE INDEX IF NOT EXISTS idx_signatures_server ON signatures(account_id, server_id);
        "#,
    },
    Migration {
        version: 42,
        description: "Send identities table for unified From/delegate/alias management",
        sql: r#"
            CREATE TABLE IF NOT EXISTS send_identities (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL,
                email TEXT NOT NULL,
                display_name TEXT,
                mailbox_id TEXT,
                send_mode TEXT NOT NULL DEFAULT 'send_as',
                save_to_personal_sent INTEGER NOT NULL DEFAULT 1,
                is_primary INTEGER NOT NULL DEFAULT 0,
                UNIQUE(account_id, email)
            );
            CREATE INDEX IF NOT EXISTS idx_send_identities_account ON send_identities(account_id);
        "#,
    },
    Migration {
        version: 43,
        description: "Server delegation columns for scheduled emails",
        sql: r#"
            ALTER TABLE scheduled_emails ADD COLUMN delegation TEXT NOT NULL DEFAULT 'local';
            ALTER TABLE scheduled_emails ADD COLUMN remote_message_id TEXT;
            ALTER TABLE scheduled_emails ADD COLUMN remote_status TEXT;
            ALTER TABLE scheduled_emails ADD COLUMN timezone TEXT;
            ALTER TABLE scheduled_emails ADD COLUMN from_email TEXT;
            ALTER TABLE scheduled_emails ADD COLUMN error_message TEXT;
            ALTER TABLE scheduled_emails ADD COLUMN retry_count INTEGER NOT NULL DEFAULT 0;
        "#,
    },
    Migration {
        version: 44,
        description: "Flag reaction-only messages for hiding from thread display",
        sql: r#"
            ALTER TABLE messages ADD COLUMN is_reaction INTEGER NOT NULL DEFAULT 0;
        "#,
    },
    Migration {
        version: 45,
        description: "Message-level categories join table for per-message category tracking",
        sql: r#"
            CREATE TABLE IF NOT EXISTS message_categories (
                account_id TEXT NOT NULL,
                message_id TEXT NOT NULL,
                category TEXT NOT NULL,
                PRIMARY KEY (account_id, message_id, category),
                FOREIGN KEY (account_id, message_id) REFERENCES messages(account_id, id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_message_categories_cat ON message_categories(account_id, category);
        "#,
    },
    Migration {
        version: 46,
        description: "MDN sent tracking: local flag to avoid duplicate read receipts",
        sql: r#"
            ALTER TABLE messages ADD COLUMN mdn_sent INTEGER NOT NULL DEFAULT 0;
        "#,
    },
    Migration {
        version: 47,
        description: "Public folder schema: folders, items, pins, sync state",
        sql: r#"
            CREATE TABLE IF NOT EXISTS public_folders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL,
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
                account_id TEXT NOT NULL,
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
            CREATE INDEX IF NOT EXISTS idx_public_folder_items_folder ON public_folder_items(account_id, folder_id, received_at DESC);

            CREATE TABLE IF NOT EXISTS public_folder_pins (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL,
                folder_id TEXT NOT NULL,
                sync_enabled INTEGER NOT NULL DEFAULT 1,
                sync_depth_days INTEGER NOT NULL DEFAULT 30,
                last_sync_at INTEGER,
                UNIQUE(account_id, folder_id)
            );

            CREATE TABLE IF NOT EXISTS public_folder_sync_state (
                account_id TEXT NOT NULL,
                folder_id TEXT NOT NULL,
                last_sync_timestamp INTEGER,
                last_full_scan_at INTEGER,
                PRIMARY KEY(account_id, folder_id)
            );
        "#,
    },
    Migration {
        version: 48,
        description: "Add last_deletion_check_at to folder_sync_state for IMAP deletion detection",
        sql: r#"
            ALTER TABLE folder_sync_state ADD COLUMN last_deletion_check_at INTEGER;
        "#,
    },
    Migration {
        version: 49,
        description: "Google People API contact mapping table",
        sql: r#"
            CREATE TABLE IF NOT EXISTS google_contact_map (
                resource_name TEXT NOT NULL,
                account_id TEXT NOT NULL,
                contact_email TEXT NOT NULL,
                PRIMARY KEY (resource_name, account_id),
                FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_google_contact_map_email
                ON google_contact_map(contact_email);
        "#,
    },
    Migration {
        version: 50,
        description: "Google otherContacts mapping table",
        sql: r#"
            CREATE TABLE IF NOT EXISTS google_other_contact_map (
                resource_name TEXT NOT NULL,
                account_id TEXT NOT NULL,
                contact_email TEXT NOT NULL,
                PRIMARY KEY (resource_name, account_id),
                FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_google_other_contact_map_email
                ON google_other_contact_map(contact_email);
        "#,
    },
    Migration {
        version: 51,
        description: "Shared mailbox sync state and delta token tables",
        sql: r#"
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
                PRIMARY KEY (account_id, mailbox_id),
                FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
            );
        "#,
    },
    Migration {
        version: 52,
        description: "Add Exchange group sync tracking columns to contact_groups",
        sql: r#"
            ALTER TABLE contact_groups ADD COLUMN source TEXT NOT NULL DEFAULT 'user';
            ALTER TABLE contact_groups ADD COLUMN account_id TEXT;
            ALTER TABLE contact_groups ADD COLUMN server_id TEXT;
            ALTER TABLE contact_groups ADD COLUMN email TEXT;
            ALTER TABLE contact_groups ADD COLUMN group_type TEXT;

            CREATE UNIQUE INDEX IF NOT EXISTS idx_contact_groups_server
              ON contact_groups(account_id, server_id) WHERE server_id IS NOT NULL;
        "#,
    },
    Migration {
        version: 53,
        description: "Contact photo cache table",
        sql: r#"
            CREATE TABLE IF NOT EXISTS contact_photo_cache (
                email TEXT NOT NULL,
                account_id TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                file_path TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                etag TEXT,
                fetched_at INTEGER NOT NULL DEFAULT (unixepoch()),
                last_accessed_at INTEGER NOT NULL DEFAULT (unixepoch()),
                PRIMARY KEY (email, account_id)
            );
        "#,
    },
    Migration {
        version: 54,
        description: "Track IMAP namespace type on labels/folders",
        sql: r#"
            ALTER TABLE labels ADD COLUMN namespace_type TEXT;
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

    for i in 0..len {
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
    let mut applied: std::collections::HashSet<u32> = stmt
        .query_map([], |row| row.get::<_, u32>(0))
        .map_err(|e| format!("query: {e}"))?
        .filter_map(Result::ok)
        .collect();
    drop(stmt);

    // ── Repair logic ───────────────────────────────────────────
    // If v14 is marked applied but imap_folder_path column is missing, re-run v14+
    if applied.contains(&14) {
        let has_col: bool = conn
            .prepare("PRAGMA table_info(labels)")
            .and_then(|mut s| {
                let cols: Vec<String> = s
                    .query_map([], |row| row.get::<_, String>(1))?
                    .filter_map(Result::ok)
                    .collect();
                Ok(cols.iter().any(|c| c == "imap_folder_path"))
            })
            .unwrap_or(false);
        if !has_col {
            log::warn!(
                "Migration v14 marked applied but imap_folder_path column missing — re-running"
            );
            let _ = conn.execute("DELETE FROM _migrations WHERE version >= 14", []);
            applied.retain(|v| *v < 14);
        }
    }
    // If v18 is marked applied but tasks table is missing, re-run
    if applied.contains(&18) {
        let has_table: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='tasks'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_table {
            log::warn!("Migration v18 marked applied but tasks table missing — re-running");
            let _ = conn.execute("DELETE FROM _migrations WHERE version = 18", []);
            applied.remove(&18);
        }
    }
    // Clean up stale TS-only migration records (v25-27) that the old TS runner
    // may have inserted with different content than Rust's v24. Since we folded
    // everything post-v23 into Rust v24, remove these so they don't confuse things.
    // v24 itself uses INSERT OR IGNORE so it won't conflict.
    if applied.contains(&25) || applied.contains(&26) || applied.contains(&27) {
        log::info!("Cleaning up stale TS migration records v25-v27");
        let _ = conn.execute("DELETE FROM _migrations WHERE version IN (25, 26, 27)", []);
        applied.remove(&25);
        applied.remove(&26);
        applied.remove(&27);
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
                let msg = e.to_string();
                if msg.contains("duplicate column")
                    || msg.contains("no such column")  // DROP COLUMN on already-dropped column
                    || msg.contains("no such table")
                // DROP TABLE on already-dropped table
                {
                    log::warn!("Skipping in v{}: {msg}", m.version);
                } else {
                    log::error!("Migration v{} failed: {msg}", m.version);
                    drop(conn.execute_batch("ROLLBACK"));
                    return Err(format!("migration v{}: {msg}", m.version));
                }
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

    // ── One-time repairs ───────────────────────────────────────
    // Force IMAP attachment resync with corrected Rust binary (runs once)
    let repair_done: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM settings WHERE key = 'imap_attachment_repair_v1'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;
    if !repair_done {
        let imap_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM accounts WHERE provider = 'imap'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if imap_count > 0 {
            log::info!("[repair] Forcing IMAP attachment resync with corrected part IDs...");
            let _ = conn.execute(
                "DELETE FROM attachments WHERE account_id IN (SELECT id FROM accounts WHERE provider = 'imap')",
                [],
            );
            let _ = conn.execute(
                "DELETE FROM folder_sync_state WHERE account_id IN (SELECT id FROM accounts WHERE provider = 'imap')",
                [],
            );
            let _ = conn.execute(
                "UPDATE accounts SET history_id = NULL WHERE provider = 'imap'",
                [],
            );
        }
        let _ = conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('imap_attachment_repair_v1', '1')",
            [],
        );
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
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='threads'",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(count, 1);

        // Verify latest migration recorded
        let max_ver: u32 = conn
            .query_row("SELECT MAX(version) FROM _migrations", [], |row| row.get(0))
            .expect("query");
        assert_eq!(max_ver, 54);
    }
}
