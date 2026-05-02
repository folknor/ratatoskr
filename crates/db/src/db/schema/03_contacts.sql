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
