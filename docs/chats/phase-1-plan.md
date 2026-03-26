# Chats Phase 1: Data Model + Thread Participants + Chat Contact Queries

Revised after second review sweep.

## Overview

Phase 1 builds the data foundations. No UI changes — this is all schema, sync pipeline, and core query functions. The output is an API surface that Phase 2 (timeline view) and Phase 3 (sidebar) can build on.

Four deliverables:
1. `thread_participants` table + sync population
2. `chat_contacts` table with denormalized summary columns + designation API
3. `is_chat_thread` flag + maintenance logic
4. Chat timeline query + sidebar summary query

## 1. Thread Participants Table

### Migration

```sql
CREATE TABLE thread_participants (
    account_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    email TEXT NOT NULL COLLATE NOCASE,
    PRIMARY KEY (account_id, thread_id, email),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);
CREATE INDEX idx_thread_participants_email
    ON thread_participants(email, account_id);
```

### Sync Population

**Address extraction:** Parse `from_address`, `to_addresses`, `cc_addresses`, and `bcc_addresses` from each message. All four fields — BCC is needed because a sent message with a BCC recipient is a 3-party conversation from the user's side, and excluding it would misclassify as 1:1. Use `parse_address_list()` from `provider-utils/src/email_parsing.rs`. Normalize to lowercase.

**New function in `crates/sync/src/persistence.rs`:**

```rust
pub fn upsert_thread_participants(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
    from_address: Option<&str>,
    to_addresses: Option<&str>,
    cc_addresses: Option<&str>,
    bcc_addresses: Option<&str>,
) -> Result<(), String>
```

**Provider-specific insertion points:**

The 4 providers have different sync architectures. The critical constraint: `thread_participants` must be populated with **final** thread IDs.

| Provider | Insertion point | Why |
|----------|----------------|-----|
| Graph | Inside `store_thread_to_db()`, after `upsert_messages` | Thread IDs are provider-assigned and final at insert time |
| JMAP | Inside `store_thread_to_db()`, after `upsert_messages` | Same — JMAP thread IDs are final |
| Gmail | Inside `store_thread_to_db()`, after `upsert_messages` | Same — Gmail thread IDs are final |
| IMAP | Inside `pipeline::store_threads()`, **after** JWZ threading resolves final thread IDs | IMAP initially inserts messages with `thread_id = message_id` as a placeholder. JWZ threading later rewrites these to final thread IDs. Populating participants at initial insert would key them to placeholder IDs that drift from the final model. |

For IMAP: after `store_threads` completes JWZ grouping, query the affected messages' address fields and populate `thread_participants` for each resolved thread.

### Maintenance on Message Deletion

`delete_messages_and_cleanup_threads` in `crates/sync/src/persistence.rs` handles message removal. When a thread loses all its messages, it's deleted. When messages are removed but the thread survives, it's re-aggregated. Add participant recomputation:

- When a thread is deleted: CASCADE handles `thread_participants` cleanup (FK on account_id covers this via thread deletion in the same transaction)
- When messages are removed but thread survives: recompute participants for that thread by re-scanning its remaining messages' address fields

### No Backfill

Old threads won't have `thread_participants` data until they receive a new message (which triggers sync and population). This is acceptable — "chat view shows conversations from after the feature was enabled." A manual "scan history" button per contact can be added later if users want to pull in historical conversations.

## 2. Chat Contacts Table

### Migration

```sql
CREATE TABLE chat_contacts (
    email TEXT PRIMARY KEY COLLATE NOCASE,
    designated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    sort_order INTEGER NOT NULL DEFAULT 0,
    display_name TEXT,
    latest_message_at INTEGER,
    latest_message_preview TEXT,
    unread_count INTEGER NOT NULL DEFAULT 0,
    contact_id TEXT
);
```

**Key design decisions:**

- **No `account_id`** — chat designation is global, cross-account. Matches the problem statement: "The Chats section is not affected by scope."
- **Denormalized summary columns** (`latest_message_at`, `latest_message_preview`, `unread_count`) — the sidebar loads this table on every navigation event, scope change, and sync completion. Query-time aggregation (window functions over all chat messages) is too expensive. Maintained during sync alongside `maybe_update_chat_flag`.
- **`contact_id TEXT`** — optional FK to the `contacts` table. Allows grouping multiple email addresses under one contact in the future. Not used in Phase 1, but avoids a migration later.
- **`display_name TEXT`** — cached from contacts/seen_addresses at designation time. Updated periodically.

### Designation API

In `crates/core/src/chat.rs`:

```rust
/// Designate an email address as a chat contact.
/// Scans existing threads for 1:1 eligibility and sets is_chat_thread.
pub async fn designate_chat_contact(
    db: &DbState,
    email: &str,
    user_emails: &[String],
) -> Result<(), String>

/// Remove chat contact designation.
/// Clears is_chat_thread on all affected threads.
pub async fn undesignate_chat_contact(
    db: &DbState,
    email: &str,
) -> Result<(), String>
```

**`designate_chat_contact` steps:**
1. `INSERT INTO chat_contacts (email) VALUES (?)`
2. Resolve display name from contacts/seen_addresses, update `chat_contacts.display_name`
3. Find all 1:1 threads (see §3 below) and set `is_chat_thread = 1`
4. Compute initial summary (latest message, unread count) and update denormalized columns

**`undesignate_chat_contact` steps:**
1. `UPDATE threads SET is_chat_thread = 0 WHERE ...` for all affected threads
2. `DELETE FROM chat_contacts WHERE email = ?`

## 3. `is_chat_thread` Flag

### Migration

```sql
ALTER TABLE threads ADD COLUMN is_chat_thread INTEGER NOT NULL DEFAULT 0;
CREATE INDEX idx_threads_chat ON threads(account_id, is_chat_thread)
    WHERE is_chat_thread = 1;
```

### 1:1 Detection Query

A thread qualifies as a chat thread if:
1. It has exactly 2 distinct participants in `thread_participants`
2. One of them is a user email (any account)
3. The other is the designated chat contact's email
4. No BCC recipients expand the participant set beyond 2

```sql
UPDATE threads SET is_chat_thread = 1
WHERE (account_id, id) IN (
    SELECT tp.account_id, tp.thread_id
    FROM thread_participants tp
    WHERE tp.thread_id IN (
        -- Candidate threads: threads where the contact participates
        SELECT thread_id FROM thread_participants
        WHERE LOWER(email) = LOWER(?1)
    )
    GROUP BY tp.account_id, tp.thread_id
    HAVING COUNT(DISTINCT LOWER(tp.email)) = 2
       AND SUM(CASE WHEN LOWER(tp.email) IN (/* user emails */) THEN 1 ELSE 0 END) >= 1
       AND SUM(CASE WHEN LOWER(tp.email) = LOWER(?1) THEN 1 ELSE 0 END) >= 1
);
```

The HAVING clause ensures:
- Exactly 2 distinct participants (including BCC, which is in `thread_participants`)
- At least one is a user email
- At least one is the designated contact
- Self-to-self threads are excluded (both conditions must be met by different addresses)

### Maintenance During Sync

**`maybe_update_chat_summary` — called after `upsert_thread_participants`:**

```rust
fn maybe_update_chat_summary(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
    user_emails: &[String],
) -> Result<(), String>
```

Steps:
1. Check if any participant in this thread is a designated chat contact
2. Count distinct participants — if exactly 2 (user + contact), set `is_chat_thread = 1`; otherwise clear it
3. If `is_chat_thread` changed, update the `chat_contacts` denormalized summary (latest message, preview, unread count)

This handles:
- New message from chat contact → flag stays set, summary updated
- Third party CC'd on reply → participant count becomes 3 → flag cleared, thread returns to Inbox
- New thread with chat contact → flag set if 1:1

### Inbox Exclusion

Add `AND t.is_chat_thread = 0` to all personal-account thread queries. Same files and pattern as the `shared_mailbox_id IS NULL` filter from contract #10:

- `scoped_queries.rs` — `get_threads_scoped` (with label and without), `get_flag_threads`, `get_draft_threads`, `get_thread_count_scoped`, `get_unread_count_scoped`
- `scoped_queries.rs` — all unread count functions (label folder, flag folder, by-account variants)
- `navigation.rs` — `build_all_account_tags`, `get_label_unread_counts`

## 4. Queries

### Chat Contact Sidebar Summary

With denormalized columns on `chat_contacts`, the sidebar query is trivial:

```sql
SELECT cc.email, cc.display_name, cc.latest_message_at,
       cc.latest_message_preview, cc.unread_count, cc.sort_order,
       cpc.file_path AS avatar_path
FROM chat_contacts cc
LEFT JOIN contact_photo_cache cpc ON LOWER(cpc.email) = cc.email
ORDER BY cc.sort_order ASC;
```

No window functions, no subqueries. Fast on every sidebar load.

**Summary maintenance** happens in `maybe_update_chat_summary` during sync:

```sql
-- Latest message (from either direction, not just from contact)
SELECT m.snippet, m.date FROM messages m
INNER JOIN threads t ON m.thread_id = t.id AND m.account_id = t.account_id
WHERE t.is_chat_thread = 1
  AND (account_id, thread_id) IN (
      SELECT account_id, thread_id FROM thread_participants WHERE LOWER(email) = ?1
  )
ORDER BY m.date DESC LIMIT 1;

-- Unread count
SELECT COUNT(*) FROM messages m
INNER JOIN threads t ON m.thread_id = t.id AND m.account_id = t.account_id
WHERE t.is_chat_thread = 1 AND m.is_read = 0
  AND (m.account_id, m.thread_id) IN (
      SELECT account_id, thread_id FROM thread_participants WHERE LOWER(email) = ?1
  );
```

These run during sync (when a chat contact's threads are updated), not on every sidebar render.

### Chat Timeline

Two-step query (unchanged from previous version — the reviewers confirmed this approach is correct):

**Step 1: Find eligible thread IDs**

```sql
SELECT DISTINCT tp.account_id, tp.thread_id
FROM thread_participants tp
INNER JOIN threads t ON t.id = tp.thread_id AND t.account_id = tp.account_id
WHERE LOWER(tp.email) = LOWER(?1)
  AND t.is_chat_thread = 1
```

**Step 2: Load messages from those threads**

```sql
SELECT m.id, m.account_id, m.thread_id, m.from_address, m.from_name,
       m.date, m.is_read, m.has_attachments, m.subject
FROM messages m
WHERE (m.account_id, m.thread_id) IN (/* thread IDs from step 1 */)
  AND (?2 IS NULL OR m.date < ?2)  -- pagination cursor
ORDER BY m.date ASC
LIMIT ?3
```

Body text loaded separately from `BodyStoreState` for visible messages only.

## Data Types

```rust
pub struct ChatContactSummary {
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_path: Option<String>,
    pub latest_message_preview: Option<String>,
    pub latest_message_at: Option<i64>,
    pub unread_count: i64,
    pub sort_order: i64,
}

pub struct ChatMessage {
    pub message_id: String,
    pub account_id: String,
    pub thread_id: String,
    pub from_address: String,
    pub from_name: Option<String>,
    pub date: i64,
    pub subject: Option<String>,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub is_read: bool,
    pub is_from_user: bool,
    pub has_attachments: bool,
}
```

## Files to Create

- `crates/core/src/chat.rs` — designation API, sidebar summary, timeline query
- `crates/core/src/db/queries_extra/chat.rs` — raw SQL query functions

## Files to Modify

- `crates/db/src/db/migrations.rs` — `thread_participants`, `chat_contacts`, `is_chat_thread`
- `crates/sync/src/persistence.rs` — `upsert_thread_participants`, participant recomputation on delete
- `crates/graph/src/sync/persistence.rs` — call `upsert_thread_participants` in `store_thread_to_db`
- `crates/jmap/src/sync/storage.rs` — call `upsert_thread_participants` in `store_thread_to_db`
- `crates/gmail/src/sync/storage.rs` — call `upsert_thread_participants` in `store_thread_to_db`
- `crates/sync/src/pipeline.rs` — call `upsert_thread_participants` in `store_threads` after JWZ (IMAP path)
- `crates/core/src/db/queries_extra/scoped_queries.rs` — add `is_chat_thread = 0` filter
- `crates/core/src/db/queries_extra/navigation.rs` — add `is_chat_thread = 0` filter
- `crates/core/src/lib.rs` — register `chat` module

## Verification

1. Sync emails with Graph/JMAP/Gmail → `thread_participants` populated with correct lowercase addresses (including BCC for sent mail)
2. Sync emails with IMAP → `thread_participants` populated with final JWZ thread IDs (not placeholder IDs)
3. Designate a contact → their 1:1 threads get `is_chat_thread = 1`, `chat_contacts` summary populated
4. Inbox thread list no longer shows those threads
5. Unread counts exclude chat threads
6. `get_chat_contacts()` returns the contact with correct denormalized summary
7. `get_chat_timeline()` returns messages in chronological order across threads, both sent and received
8. Undesignate → threads return to Inbox, flag cleared, `chat_contacts` row deleted
9. CC a third party on a chat thread → participant count becomes 3 → flag auto-clears, thread returns to Inbox
10. Delete a message from a 3-party thread leaving only 2 participants → flag re-set if contact is designated
11. New message from chat contact → `thread_participants` updated, `is_chat_thread` maintained, summary updated
