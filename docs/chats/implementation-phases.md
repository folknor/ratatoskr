# Chats: Implementation Phases

Revised after review agent sweep. Each phase is deployable independently.

## Phase 1: Data Model + Thread Participants + Chat Contact Queries

**Goal:** Schema foundations, normalized participant data, designation, and the core queries.

### 1a. `thread_participants` table

The existing `to_addresses`/`cc_addresses`/`bcc_addresses` TEXT fields on `messages` are comma-separated strings — useful for display and reply-all, but not queryable for participant-level operations. Add a normalized projection alongside them (not replacing them):

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

Populated during sync: when `upsert_messages` runs, parse `from_address`, `to_addresses`, `cc_addresses` and INSERT OR IGNORE into `thread_participants`. All 4 providers go through the same message persistence path, so one insertion point covers all.

Backfill migration: one-time pass parsing existing `messages` rows. This is the expensive part — parse every message's address fields. Run outside the migration transaction (post-migration fixup) to avoid holding the DB lock.

This table enables:
- **1:1 detection:** `SELECT COUNT(DISTINCT email) FROM thread_participants WHERE account_id = ? AND thread_id = ?` — if exactly 2, it's a two-party thread
- **Per-contact thread discovery:** `SELECT thread_id FROM thread_participants WHERE email = ? AND account_id = ?` — find all threads involving a contact
- **Chat timeline construction:** resolve eligible thread IDs first, then load messages from those threads using existing indexes

### 1b. `chat_contacts` table

Keyed by normalized email — no `account_id`. Chat designation is a global, cross-account decision ("I want to chat-view Alice" regardless of which account I email her from). This matches the problem statement's design: "The Chats section is not affected by scope."

```sql
CREATE TABLE chat_contacts (
    email TEXT PRIMARY KEY COLLATE NOCASE,
    designated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    sort_order INTEGER NOT NULL DEFAULT 0
);
```

No CASCADE FK needed — email is not tied to an account. Cleanup on contact undesignation is explicit (remove the row).

If Alice has multiple email addresses, each address is a separate `chat_contacts` row. The contact system's deduplication (via `contacts` table or `seen_addresses`) can group them in the UI, but the underlying query is per-email.

### 1c. `is_chat_thread` flag on `threads`

Following the `shared_mailbox_id` pattern: a denormalized flag avoids expensive subqueries on every Inbox load.

```sql
ALTER TABLE threads ADD COLUMN is_chat_thread INTEGER NOT NULL DEFAULT 0;
CREATE INDEX idx_threads_chat ON threads(account_id, is_chat_thread)
    WHERE is_chat_thread = 1;
```

Maintained when:
- A contact is designated → scan their threads, set `is_chat_thread = 1` for qualifying 1:1 threads
- A contact is undesignated → clear the flag on their threads
- A new message arrives in a thread with a chat contact → re-evaluate (a third party CC'd on reply #7 clears the flag)
- Thread participants change during sync → re-evaluate

Inbox/folder queries add `AND t.is_chat_thread = 0` (same pattern as `shared_mailbox_id IS NULL`).

### 1d. Core API

```rust
// Designation
designate_chat_contact(email: &str) -> Result<()>
undesignate_chat_contact(email: &str) -> Result<()>

// Sidebar data
get_chat_contacts() -> Vec<ChatContactSummary>
// Returns: email, display_name, latest_message_preview, latest_message_at, unread_count

// Timeline
get_chat_timeline(email: &str, limit: usize, before: Option<i64>) -> Vec<ChatMessage>
// Per-contact message stream across all accounts, paginated, newest-last
// Query strategy: find eligible thread IDs via thread_participants + is_chat_thread,
// then SELECT messages from those threads ordered by date ASC
```

**Verification:** Designate a contact → their 1:1 threads get `is_chat_thread = 1` and disappear from Inbox. Undesignate → they reappear. Timeline query returns messages in chronological order across threads.

## Phase 2: Chat Timeline View

**Goal:** Bubble-based message rendering when a chat contact is selected.

**Depends on:** Phase 1 queries.

**View architecture:**

The chat timeline is NOT a mode within ReadingPane — it's a distinct top-level content view. When a chat contact is selected, the thread list + reading pane layout is replaced by a single chat timeline panel (similar to how Calendar replaces the mail layout).

New `ChatTimeline` component:
- Messages as bubbles: sent (right-aligned, accent color), received (left-aligned, surface color)
- Ownership detection: compare `from_address` against user's account emails
- Date separators between messages on different days
- Subject change indicators (subtle text above first bubble with new subject)
- Newest at bottom, auto-scroll to latest on open
- "Show full message" affordance on each bubble (expands to show stripped content)

**Basic signature stripping (layers 1-3):**
- HTML client markers (Gmail `gmail_signature`, Outlook `stopSpelling`, Thunderbird `moz-cite-prefix`)
- RFC 3676 `-- \n` delimiter
- User's own signatures (match against `signatures` table for sent messages)
- Quoted reply blocks (`On <date>, <person> wrote:` + `>` prefixed lines)

Build stripping as a reusable module in `provider-utils` — it's useful beyond chats.

**Body loading:**
- Messages in the timeline need body text from `bodies.db` via `BodyStoreState`
- Load bodies for visible messages only (virtual scrolling or paginated loading)

**Navigation integration:**
- New `NavigationTarget::Chat { email: String }` variant
- Chat selection sets an `active_chat: Option<String>` on App (not a ViewScope variant — chats are cross-scope)
- `reset_view_state()` clears `active_chat`
- Thread detail loading is skipped when `active_chat` is set

**Verification:** Select a chat contact → see bubble timeline with signatures stripped. Full message accessible via expand. Scroll to bottom shows latest.

## Phase 3: Sidebar Integration

**Goal:** Chats section in the sidebar between pinned searches and universal folders.

**Depends on:** Phase 1 queries, Phase 2 view.

**Sidebar section:**
- "CHATS" header, collapsible, hidden when no chat contacts designated
- Each entry: avatar/initials, contact name (from contacts table or seen_addresses), message preview, relative timestamp, unread bold
- Ordered by `sort_order` (designated order for v1 — defer drag-and-drop reorder to Phase 6)
- Click → loads chat timeline, replacing thread list + reading pane

**Not affected by scope** — same list regardless of selected account.

**Unread tracking:**
- Per-contact unread count: count of unread messages in `is_chat_thread = 1` threads for that contact
- Opening a chat marks messages read — batch operation across all qualifying threads
- Mark-read dispatches through action service (`mark_read` per thread) to keep provider in sync
- Rate-limit or batch the provider dispatch to avoid hammering the API for prolific contacts

**Inbox exclusion:**
- All standard thread queries (scoped threads, unread counts, flag counts, draft counts) add `AND t.is_chat_thread = 0`
- Same pattern as `shared_mailbox_id IS NULL`
- Chat threads still appear in search results (they're normal emails in the DB)

**Verification:** Chat contacts appear in sidebar. Unread counts correct. Inbox doesn't show chat threads. Search still finds them.

## Phase 4: Chat Compose

**Goal:** Lightweight inline compose at the bottom of the chat timeline.

**Depends on:** Phase 2 timeline view, Phase 3 sidebar integration.

**Compose widget:**
- Simple text input at bottom of timeline
- No subject line — reuses latest thread's subject. New conversation: "Hello, {first_name}" (or LLM-generated if configured)
- Enter to send, Shift+Enter for newline (setting to invert)
- Input expands upward (overlay, not push) up to ~6 lines, then internal scroll
- Signature auto-appended but hidden in sender's chat view
- Drag-and-drop / paste for attachments

**Reply threading:**
- Track which thread to reply to: the most recent thread with this contact
- Set `In-Reply-To` header to the latest message ID in that thread
- If no existing thread, create a new email (new thread)
- Account selection: use the account that last sent/received with this contact

**Send path:**
- Routes through `actions::send` (existing infrastructure)
- On success: message appears as sent bubble immediately (optimistic from local draft → sent state)
- "Sending..." indicator on the bubble until provider confirms

**Emoji shortcodes:**
- Inline translation (`:thumbsup:` → 👍)
- Small shortcode table (50-100 common ones)
- Defer emoji picker to Phase 6

**Verification:** Type message, Enter sends. Bubble appears immediately. Recipient sees normal email with signature. Threading works (reply chains).

## Phase 5: Signature Stripping Refinement

**Goal:** Per-sender learned patterns for contacts without HTML markers.

**Depends on:** Phase 2 basic stripping working well enough to identify gaps.

**Per-sender learning:**
- For each chat contact, accumulate trailing blocks from their messages
- After N messages (5-10), extract the common suffix via longest-common-suffix algorithm
- Store patterns in `chat_learned_signatures (email TEXT, pattern TEXT, confidence REAL, updated_at INTEGER)`
- Apply as stripping layer 4 (between HTML markers and heuristic patterns)

**Heuristic patterns (layer 5):**
- Valediction phrases ("Best regards", "Sincerely", etc.) — configurable, language-aware
- "Sent from my iPhone/Android" boilerplate
- Separator lines (dashes, underscores)

**Confidence and graceful degradation:**
- Low confidence → show full message with subtle "show clean" toggle
- Never strip aggressively on first message from new contact
- Collapse, don't delete — zero information loss

**Verification:** Signatures stripped reliably after accumulating messages. New contacts degrade gracefully.

## Phase 6: Polish

**Goal:** Edge cases, performance, UX refinement.

- Virtual scrolling for long timelines (thousands of messages)
- Drag-and-drop reorder for chat contacts in sidebar
- Right-click context menu on chat entries (undesignate, mute, view as email)
- Cross-account message deduplication in timeline (same email via multiple accounts)
- Thread-level "view as email" toggle
- Emoji picker widget
- Undesignation confirmation dialog (threads will return to Inbox)
- Email address case normalization audit across all providers

## Risks

- **Signature stripping reliability** — mitigated by per-contact learning and collapse-not-delete
- **`thread_participants` backfill performance** — parsing all existing messages' address fields. Mitigate: run as post-migration fixup, not inside transaction
- **Chat timeline query performance** — resolved by querying via thread IDs (from `thread_participants` + `is_chat_thread`), then loading messages per-thread using existing indexes
- **Compose latency expectations** — Enter-to-send feels instant but email goes through SMTP. Need clear sending → sent transition
- **Inbox exclusion cross-cutting** — every thread query needs `is_chat_thread = 0`. Follow the `shared_mailbox_id IS NULL` pattern established in contract #10
