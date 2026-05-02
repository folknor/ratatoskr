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
    -- 1 = the authenticated user has write access to this calendar.
    -- 0 = read-only (shared/subscribed calendar without edit rights).
    -- Source: Graph `canEdit`; mapped to 1 for owned Google/JMAP/CalDAV
    -- calendars until provider sync paths plumb a permission signal.
    can_edit INTEGER NOT NULL DEFAULT 1,
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
    -- RECURRENCE-ID for VEVENTs that override a single instance of a master
    -- recurring event (RFC 5545 sec 3.8.4.4). Stored in canonical wall-clock
    -- form so the value is independent of the host's local timezone:
    --   YYYYMMDD                         all-day (VALUE=DATE)
    --   YYYYMMDDTHHMMSSZ                  UTC (Z-suffix)
    --   YYYYMMDDTHHMMSS                   floating (no TZID, no Z)
    --   YYYYMMDDTHHMMSS;TZID=<id>         zoned
    -- Resolving to a Unix timestamp at upsert time (the previous behavior)
    -- silently re-parsed floating and all-day RECURRENCE-IDs through
    -- chrono::Local, making the storage key host-dependent: a sync run on
    -- UTC and another on a non-UTC host produced two distinct keys for the
    -- same override, leaving an orphan row on TZ change. Master rows leave
    -- this NULL.
    recurrence_id TEXT,
    UNIQUE(account_id, google_event_id)
);
CREATE INDEX IF NOT EXISTS idx_cal_events_time ON calendar_events(account_id, start_time, end_time);
CREATE INDEX IF NOT EXISTS idx_cal_events_calendar ON calendar_events(calendar_id);
-- Load-path phantom-dedup walks (account_id, uid) groups to subtract
-- override slots from master expansion. Without this index that becomes a
-- table scan on every calendar render.
CREATE INDEX IF NOT EXISTS idx_cal_events_uid ON calendar_events(account_id, uid);

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

-- CalDAV URI -> ETag map. Drives incremental sync: list_events returns
-- (uri, etag) pairs, we diff against the stored map to decide what to
-- fetch / update / delete. Keyed on (calendar_id, uri); calendar_id
-- cascades from `calendars`, so deleting an account cleans up the map
-- via the chain account -> calendars -> caldav_event_map. event_uid
-- duplicates the iCal UID so per-resource lookups can resolve back to
-- the calendar_events row without re-parsing the iCal payload.
CREATE TABLE IF NOT EXISTS caldav_event_map (
    uri TEXT NOT NULL,
    calendar_id TEXT NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
    event_uid TEXT NOT NULL,
    etag TEXT NOT NULL,
    PRIMARY KEY (calendar_id, uri)
);
CREATE INDEX IF NOT EXISTS idx_caldav_event_map_calendar ON caldav_event_map(calendar_id);
