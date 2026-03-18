#!/usr/bin/env python3
"""
Seed a Ratatoskr database from Thunderbird's global-messages-db.sqlite.

Reads real message metadata (subjects, senders, dates, conversations) from
Thunderbird's index and creates a fully-formed ratatoskr.db with the correct
schema. Does NOT read email bodies (they're in mbox files).

Usage:
    python3 seed-db.py [thunderbird.sqlite] [output-dir]

Defaults:
    thunderbird.sqlite = ./thunderbird.sqlite
    output-dir = ~/.local/share/com.velo.app/
"""

import sqlite3
import os
import sys
import uuid
import re
from pathlib import Path
from urllib.parse import unquote

TB_DB = sys.argv[1] if len(sys.argv) > 1 else "thunderbird.sqlite"
OUT_DIR = Path(sys.argv[2] if len(sys.argv) > 2 else os.path.expanduser("~/.local/share/com.velo.app"))

OUT_DIR.mkdir(parents=True, exist_ok=True)
OUT_DB = OUT_DIR / "ratatoskr.db"

if OUT_DB.exists():
    print(f"Removing existing {OUT_DB}")
    OUT_DB.unlink()

print(f"Reading from: {TB_DB}")
print(f"Writing to:   {OUT_DB}")

# ── Read Thunderbird data ────────────────────────────────────

tb = sqlite3.connect(TB_DB)
tb.row_factory = sqlite3.Row

# Extract accounts from folder URIs
folders = tb.execute("SELECT * FROM folderLocations ORDER BY id").fetchall()
accounts_by_uri = {}
for f in folders:
    uri = f["folderURI"]
    # imap://user%40domain@server/FolderName
    m = re.match(r'imap://([^@]+)@([^/]+)/(.*)', uri)
    if m:
        email = unquote(m.group(1))
        server = m.group(2)
        if email not in accounts_by_uri:
            accounts_by_uri[email] = {
                "id": str(uuid.uuid4()),
                "email": email,
                "server": server,
                "folders": {},
            }
        folder_name = unquote(m.group(3))
        accounts_by_uri[email]["folders"][f["id"]] = folder_name

print(f"Found {len(accounts_by_uri)} account(s): {list(accounts_by_uri.keys())}")

if not accounts_by_uri:
    print("No IMAP accounts found in Thunderbird DB. Creating a fake account.")
    accounts_by_uri["demo@example.com"] = {
        "id": str(uuid.uuid4()),
        "email": "demo@example.com",
        "server": "imap.example.com",
        "folders": {},
    }

# Build folder_id -> account mapping
folder_to_account = {}
for acc in accounts_by_uri.values():
    for fid, fname in acc["folders"].items():
        folder_to_account[fid] = acc

# Read messages with FTS content
messages = tb.execute("""
    SELECT m.id, m.folderID, m.conversationID, m.date, m.headerMessageID,
           m.jsonAttributes, m.deleted,
           mt.c1subject as subject, mt.c3author as author, mt.c4recipients as recipients
    FROM messages m
    LEFT JOIN messagesText_content mt ON mt.docid = m.id
    WHERE m.deleted = 0 AND mt.c1subject IS NOT NULL
    ORDER BY m.date ASC
""").fetchall()

print(f"Read {len(messages)} messages with metadata")

# Read conversations
conversations = {
    r["id"]: r for r in
    tb.execute("SELECT * FROM conversations").fetchall()
}

tb.close()

# ── Create Ratatoskr DB ─────────────────────────────────────

db = sqlite3.connect(str(OUT_DB))
db.execute("PRAGMA journal_mode = WAL")
db.execute("PRAGMA foreign_keys = ON")

# Run essential schema (simplified from migrations.rs)
db.executescript("""
    CREATE TABLE accounts (
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
        provider TEXT DEFAULT 'imap',
        imap_host TEXT,
        imap_port INTEGER DEFAULT 993,
        imap_security TEXT DEFAULT 'tls',
        smtp_host TEXT,
        smtp_port INTEGER DEFAULT 587,
        smtp_security TEXT DEFAULT 'starttls',
        auth_method TEXT DEFAULT 'oauth',
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
        jmap_url TEXT
    );

    CREATE TABLE labels (
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
        PRIMARY KEY (account_id, id)
    );
    CREATE INDEX idx_labels_account ON labels(account_id);

    CREATE TABLE threads (
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
        PRIMARY KEY (account_id, id)
    );
    CREATE INDEX idx_threads_date ON threads(account_id, last_message_at DESC);

    CREATE TABLE thread_labels (
        thread_id TEXT NOT NULL,
        account_id TEXT NOT NULL,
        label_id TEXT NOT NULL,
        PRIMARY KEY (account_id, thread_id, label_id),
        FOREIGN KEY (account_id, thread_id) REFERENCES threads(account_id, id) ON DELETE CASCADE
    );
    CREATE INDEX idx_thread_labels_label ON thread_labels(account_id, label_id);

    CREATE TABLE messages (
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
        PRIMARY KEY (account_id, id),
        FOREIGN KEY (account_id, thread_id) REFERENCES threads(account_id, id) ON DELETE CASCADE
    );
    CREATE INDEX idx_messages_thread ON messages(account_id, thread_id, date ASC);
    CREATE INDEX idx_messages_date ON messages(account_id, date DESC);

    CREATE TABLE contacts (
        id TEXT PRIMARY KEY,
        email TEXT NOT NULL UNIQUE,
        display_name TEXT,
        avatar_url TEXT,
        frequency INTEGER DEFAULT 1,
        last_contacted_at INTEGER,
        notes TEXT,
        created_at INTEGER DEFAULT (unixepoch()),
        updated_at INTEGER DEFAULT (unixepoch())
    );

    CREATE TABLE settings (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
""")

# ── Insert accounts ──────────────────────────────────────────

for acc in accounts_by_uri.values():
    db.execute("""
        INSERT INTO accounts (id, email, display_name, provider, imap_host, auth_method)
        VALUES (?, ?, ?, 'imap', ?, 'oauth')
    """, (acc["id"], acc["email"], acc["email"].split("@")[0], acc["server"]))

print("Inserted accounts")

# ── Insert labels from folders ───────────────────────────────

SPECIAL_USE_MAP = {
    "INBOX": "inbox",
    "Sent": "sent",
    "Drafts": "drafts",
    "Trash": "trash",
    "Archive": "archive",
    "Junk": "junk",
    "Spam": "junk",
}

for acc in accounts_by_uri.values():
    for fid, fname in acc["folders"].items():
        label_id = str(uuid.uuid4())
        special = SPECIAL_USE_MAP.get(fname)
        db.execute("""
            INSERT INTO labels (id, account_id, name, type, imap_folder_path, imap_special_use, sort_order)
            VALUES (?, ?, ?, 'system', ?, ?, ?)
        """, (
            label_id, acc["id"], fname, fname,
            special,
            0 if fname == "INBOX" else 10,
        ))
        # Stash label_id for thread_labels
        acc["folders"][fid] = {"name": fname, "label_id": label_id}

print("Inserted labels")

# ── Parse author field ───────────────────────────────────────

def parse_author(author_str):
    """Parse 'Name email' or just 'email' from Thunderbird's author field."""
    if not author_str:
        return None, None
    # Remove trailing 'undefined' that Thunderbird sometimes appends
    author_str = author_str.replace(" undefined", "").strip()
    # Try "name <email>" pattern
    m = re.match(r'^(.*?)\s*<([^>]+)>', author_str)
    if m:
        return m.group(1).strip().strip('"') or None, m.group(2).strip()
    # Try "email name" (Thunderbird FTS format)
    parts = author_str.split()
    if len(parts) >= 1 and "@" in parts[0]:
        name = " ".join(parts[1:]) if len(parts) > 1 else None
        return name, parts[0]
    # Just return as-is
    if "@" in author_str:
        return None, author_str
    return author_str, None

# ── Group messages into threads and insert ───────────────────

# Use Thunderbird's conversationID as thread grouping
thread_map = {}  # conv_id -> {account_id, thread_id, messages, ...}
skipped = 0

for msg in messages:
    acc = folder_to_account.get(msg["folderID"])
    if not acc:
        skipped += 1
        continue

    conv_id = msg["conversationID"]
    key = (acc["id"], conv_id)

    # Thunderbird dates are in microseconds
    date_us = msg["date"] or 0
    date_s = date_us // 1_000_000

    from_name, from_address = parse_author(msg["author"])

    msg_data = {
        "id": str(uuid.uuid4()),
        "account_id": acc["id"],
        "subject": msg["subject"],
        "date": date_s,
        "from_name": from_name,
        "from_address": from_address,
        "recipients": msg["recipients"],
        "message_id_header": msg["headerMessageID"],
        "folder_id": msg["folderID"],
    }

    # Check for attachments in jsonAttributes
    has_attachments = False
    json_attr = msg["jsonAttributes"]
    if json_attr and '"51":[' in json_attr and '"51":[]' not in json_attr:
        has_attachments = True

    # Check read/starred from jsonAttributes
    is_read = True  # Default to read
    is_starred = False
    if json_attr:
        if '"59":true' in json_attr:
            is_read = True
        elif '"59":false' in json_attr:
            is_read = False
        if '"60":true' in json_attr:
            is_starred = True

    msg_data["has_attachments"] = has_attachments
    msg_data["is_read"] = is_read
    msg_data["is_starred"] = is_starred

    if key not in thread_map:
        conv = conversations.get(conv_id)
        thread_map[key] = {
            "thread_id": str(uuid.uuid4()),
            "account_id": acc["id"],
            "subject": conv["subject"] if conv else msg["subject"],
            "messages": [],
            "latest_date": date_s,
            "has_attachments": has_attachments,
            "is_read": is_read,
            "is_starred": is_starred,
            "folder_ids": set(),
        }

    t = thread_map[key]
    t["messages"].append(msg_data)
    if date_s > t["latest_date"]:
        t["latest_date"] = date_s
    if has_attachments:
        t["has_attachments"] = True
    if is_starred:
        t["is_starred"] = True
    if not is_read:
        t["is_read"] = False
    if msg["folderID"]:
        t["folder_ids"].add(msg["folderID"])

print(f"Grouped into {len(thread_map)} threads ({skipped} messages skipped, no account match)")

# ── Write threads, messages, thread_labels ───────────────────

thread_count = 0
message_count = 0

for (account_id, conv_id), t in thread_map.items():
    tid = t["thread_id"]
    msgs = t["messages"]
    latest_msg = max(msgs, key=lambda m: m["date"])
    snippet = (latest_msg["subject"] or "")[:200]

    db.execute("""
        INSERT INTO threads (id, account_id, subject, snippet, last_message_at,
                             message_count, is_read, is_starred, has_attachments)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
    """, (
        tid, account_id, t["subject"], snippet, t["latest_date"],
        len(msgs), int(t["is_read"]), int(t["is_starred"]),
        int(t["has_attachments"]),
    ))
    thread_count += 1

    # Link thread to labels based on folder
    acc = next(a for a in accounts_by_uri.values() if a["id"] == account_id)
    for fid in t["folder_ids"]:
        folder_info = acc["folders"].get(fid)
        if folder_info and isinstance(folder_info, dict):
            db.execute("""
                INSERT OR IGNORE INTO thread_labels (thread_id, account_id, label_id)
                VALUES (?, ?, ?)
            """, (tid, account_id, folder_info["label_id"]))

    for msg_data in msgs:
        to_addr = msg_data["recipients"]
        if to_addr:
            to_addr = to_addr.replace(" undefined", "").strip()

        db.execute("""
            INSERT INTO messages (id, account_id, thread_id, from_address, from_name,
                                  to_addresses, subject, snippet, date, is_read, is_starred,
                                  message_id_header, imap_folder)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """, (
            msg_data["id"], account_id, tid,
            msg_data["from_address"], msg_data["from_name"],
            to_addr, msg_data["subject"],
            (msg_data["subject"] or "")[:200],
            msg_data["date"],
            int(msg_data["is_read"]), int(msg_data["is_starred"]),
            msg_data["message_id_header"],
            acc["folders"].get(msg_data["folder_id"], {}).get("name") if isinstance(acc["folders"].get(msg_data["folder_id"]), dict) else None,
        ))
        message_count += 1

        # Upsert contact
        if msg_data["from_address"]:
            db.execute("""
                INSERT INTO contacts (id, email, display_name, frequency, last_contacted_at)
                VALUES (?, ?, ?, 1, ?)
                ON CONFLICT(email) DO UPDATE SET
                    frequency = frequency + 1,
                    display_name = COALESCE(excluded.display_name, display_name),
                    last_contacted_at = MAX(COALESCE(excluded.last_contacted_at, 0),
                                            COALESCE(last_contacted_at, 0))
            """, (
                str(uuid.uuid4()), msg_data["from_address"],
                msg_data["from_name"], msg_data["date"],
            ))

db.commit()

# ── Summary ──────────────────────────────────────────────────

contact_count = db.execute("SELECT count(*) FROM contacts").fetchone()[0]
label_count = db.execute("SELECT count(*) FROM labels").fetchone()[0]

db.close()

print(f"\nDone!")
print(f"  Accounts: {len(accounts_by_uri)}")
print(f"  Labels:   {label_count}")
print(f"  Threads:  {thread_count}")
print(f"  Messages: {message_count}")
print(f"  Contacts: {contact_count}")
print(f"\n  Database: {OUT_DB}")
print(f"  Size:     {OUT_DB.stat().st_size / 1024 / 1024:.1f} MB")
