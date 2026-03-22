# Labels Unification: Problem Statement

## Overview

Ratatoskr's UI presents a unified label-like surface for email organization. Under the hood, the data model preserves the distinction between **containers** (folders/mailboxes — a message lives in one or more) and **tags** (categories/keywords — applied to messages as metadata). This distinction matters for provider sync and write-back, but the user interacts with both through the same label UI.

This document specifies how provider-specific concepts are unified for display, how the sidebar structures them, and how the user applies and removes labels across accounts and providers.

## Current State

The codebase has three separate systems that should be one:

1. **Labels** (`labels` + `thread_labels` tables) — stores Gmail labels, IMAP folders, Exchange folders, JMAP mailboxes. Used by `add_tag()`/`remove_tag()` on ProviderOps.
2. **Exchange categories** (`categories` + `message_categories` tables) — synced from Exchange's `masterCategories` API. Has `apply_category()`/`remove_category()` on ProviderOps. Not displayed in sidebar.
3. **IMAP/JMAP keywords** — `apply_category()`/`remove_category()` are no-ops. Not stored or displayed.

Additionally, there is a separate **AI thread categorization** system (`thread_categories` table) for inbox bundling (Primary/Updates/Promotions/Social/Newsletters). This is unrelated to user-facing labels and is NOT part of this unification.

## Target State

All provider tagging mechanisms (Exchange categories, IMAP keywords, JMAP keywords) are stored as rows in the `labels` table alongside existing provider folders/labels. Each row retains its `(account_id, id)` canonical identity — the provider-specific ID is the source of truth.

The `apply_category()`/`remove_category()` methods on ProviderOps become redundant — `add_tag()`/`remove_tag()` handles everything.

The `categories` and `message_categories` tables become redundant for user-facing purposes (they may remain for sync state tracking during a transition period).

## Data Model

### Container vs Tag

Each label row in the `labels` table has a **semantics** field that distinguishes containers from tags:

- **Container** (folder/mailbox): Gmail system labels, IMAP folders, Exchange folders, JMAP mailboxes. Displayed in the sidebar's folder section. Provider operations use folder move semantics.
- **Tag** (category/keyword): Gmail user labels, Exchange categories, IMAP keywords, JMAP keywords. Displayed in the sidebar's labels section. Provider operations use tag apply/remove semantics.

This distinction is already partially captured by the existing `label_semantics` field on `NavigationFolder`. It needs to be made explicit on the `labels` table itself.

### Canonical Identity

Labels are identified by `(account_id, provider_label_id)` — NOT by name. Name is the **display merge key** for cross-account presentation grouping. The canonical identity is always provider- and account-scoped.

When the sidebar groups labels from multiple accounts by normalized display name, this is a presentation heuristic. It does not imply semantic equivalence. "Project Alpha" on a work Exchange account and "Project Alpha" on a personal Gmail account may be unrelated — they are grouped for convenience.

### Name Normalization

For cross-account display grouping, label names are normalized:

- Case-insensitive comparison (handles IMAP keyword case insensitivity per RFC, and cross-provider mismatches like "work" vs "Work")
- Leading/trailing whitespace trimmed

Two labels with the same normalized name from different accounts appear as one entry in the sidebar's labels section.

## Sidebar Structure

The sidebar has four sections. Only section 2 changes based on the account selector.

### All Accounts selected:
1. **Pinned searches** — ephemeral saved search snapshots
2. **Universal folders** — Inbox, Starred, Snoozed, Sent, Drafts, Trash
3. **Smart folders** — permanent saved queries
4. **Labels** — all tag-type labels merged across all accounts, grouped by normalized name

### Single Account selected:
1. **Pinned searches** — same (not scoped)
2. **Provider folders** — that account's container-type labels, with hierarchy preserved as the server provides it
3. **Smart folders** — same (not scoped)
4. **Labels** — same (not scoped, all accounts merged)

Pinned searches, smart folders, and labels are **always the same** regardless of which account is selected. Only the folder section changes.

### Labels section (section 4) is flat

Labels in section 4 have no hierarchy. A Gmail user label named "Projects/Alpha" appears as a single flat entry with that name. Hierarchy is a provider folder concept — it belongs in section 2 only.

## Provider Mapping

| Provider | Folders — section 2 (container semantics) | Labels — section 4 (tag semantics) |
|----------|-------------------------------------------|-------------------------------------|
| **Gmail** | System labels (`type: "system"`) — INBOX, SENT, TRASH, SPAM, DRAFTS, STARRED, CATEGORY_* | User labels (`type: "user"`) |
| **Exchange** | Exchange folders | Exchange categories |
| **IMAP** | IMAP mailboxes/folders | IMAP keywords (when server supports `PERMANENTFLAGS \*`) |
| **JMAP** | JMAP mailboxes | JMAP keywords |

### Gmail distinction

Gmail's API marks labels with `type: "system"` vs `type: "user"`. System labels behave like folders (INBOX, SENT, TRASH, etc.) and appear in section 2 when the Gmail account is selected. User-created labels behave like tags and appear in section 4 alongside labels from all other accounts.

### IMAP keyword capability

IMAP keywords are only usable when the server advertises `PERMANENTFLAGS` that includes `\*` (arbitrary keywords allowed). When a server does not support arbitrary keywords, labels cannot be applied to messages on that account. The UI should indicate this limitation — e.g., graying out the "apply label" action for messages from that account, or showing a brief notice.

Fallback: if the server supports a fixed set of permanent flags (e.g., `$label1` through `$label5`), those can be exposed as labels. If the server supports no custom flags at all, that account has no section 4 labels.

## Label Colors

Color resolution priority:

1. **User-configured color** — the user explicitly set a color for this label name in Ratatoskr. Always wins. Stored in a local-only table keyed by normalized label name.
2. **Server-synced color** — from the provider (Gmail label color, Exchange category color preset). If only one backing label provides a color, use it. If multiple accounts have the same label name with different colors, use the one with an explicit server color (non-null `color_bg`). If multiple have explicit colors, prefer the one from the account with the lowest sort order.
3. **Hash fallback** — deterministic color from the 25-preset palette based on the normalized label name string.

### Color override storage

```sql
CREATE TABLE IF NOT EXISTS label_color_overrides (
    label_name TEXT NOT NULL PRIMARY KEY COLLATE NOCASE,
    color_bg TEXT NOT NULL
);
```

Local-only, never synced to any provider. Keyed by normalized label name.

## Creating Labels

When a user creates a new label in Ratatoskr, it creates a **tag-type entity** (not a folder) on every account that supports it:

- **Gmail**: creates a new user label via Labels API
- **Exchange**: creates a new category via masterCategories API
- **IMAP**: implicit — keywords are created by first use (no pre-creation API). The label exists locally immediately and is written to the server when first applied to a message.
- **JMAP**: implicit — same as IMAP.

Creation failures on individual accounts are non-fatal — the label exists locally and on accounts where creation succeeded. Accounts that don't support tag creation (e.g., IMAP servers without `PERMANENTFLAGS \*`) are silently skipped.

Folder/mailbox creation is a separate operation — it is NOT triggered by creating a label. Users create folders through provider-specific UI (or a future "create folder" action in section 2).

## Applying and Removing Labels

Users apply and remove labels via pills in the reading pane and message view pop-out.

### Apply to thread

When a user applies a label to a thread, Ratatoskr applies it to **all messages** in that thread. For each message, the correct provider operation is used:

- **Gmail**: `add_tag()` with the Gmail label ID resolved from the label name for that account
- **Exchange**: `add_tag()` sets a category on the message (currently `apply_category()`, to be unified into `add_tag()`)
- **IMAP**: `add_tag()` sets a keyword flag via `STORE +FLAGS`
- **JMAP**: `add_tag()` sets a keyword via `Email/set`

The provider-specific label ID is resolved by looking up the `labels` table for a matching `(account_id, name)` pair where `account_id` matches the message's account.

### Remove from thread

When a user removes a label from a thread, Ratatoskr removes it from **all messages** in that thread that have it. Same per-message provider dispatch as apply.

### Cross-account threads

Threads are always single-account in Ratatoskr (a thread belongs to one account). So apply/remove always targets one provider.

## Sync

### Incoming (server → local)

Exchange category changes arrive via delta sync (`/messages?$deltatoken=...`). Changed messages include their full `categories` array. The sync pipeline diffs against local state and updates `labels` + `thread_labels`.

IMAP keyword changes arrive via `FLAGS` responses. CONDSTORE/QRESYNC provides efficient change detection.

JMAP keyword changes arrive via `Email/changes` + `Email/get`.

All three write to the `labels` + `thread_labels` tables, the same as existing folder/label sync.

### Outgoing (local → server)

Label apply/remove operations go through `add_tag()`/`remove_tag()` on ProviderOps, which translates to the correct API call per provider.

### Thread-level rollup

Labels are applied at the message level on most providers, but `thread_labels` is thread-level. A thread has a label if **any message** in it has that label. The existing label sync already handles this rollup for Gmail labels and provider folders — the same logic applies to categories/keywords.

### Server-side deletion

When a label is deleted server-side (e.g., an Exchange admin removes a category, or the user deletes it in another client):

- The label row is removed from the `labels` table for that account during sync
- `thread_labels` entries for that account are cleaned up
- If the same label name exists on other accounts, it remains in the sidebar
- No user prompt — deletion is silent and consistent with how folder deletion already works

## Unread Counts

Section 4 labels display unread counts. For a grouped label (same name across multiple accounts), the unread count is the **sum** across all accounts that have that label. Computed via:

```sql
SELECT COUNT(DISTINCT tl.thread_id)
FROM thread_labels tl
JOIN threads t ON tl.thread_id = t.id AND tl.account_id = t.account_id
JOIN labels l ON tl.label_id = l.id AND tl.account_id = l.account_id
WHERE LOWER(TRIM(l.name)) = LOWER(TRIM(?1))
  AND t.is_read = 0
```

This is computed alongside navigation state loading and cached in sidebar state.

## Migration

### Phase 1: Schema

Add a `label_kind` column to the `labels` table:

```sql
ALTER TABLE labels ADD COLUMN label_kind TEXT NOT NULL DEFAULT 'container';
-- Values: 'container' (folder/mailbox) or 'tag' (category/keyword)
```

Populate for existing rows:
- Gmail labels with `type = 'system'` → `'container'`
- Gmail labels with `type = 'user'` → `'tag'`
- All other provider labels → `'container'` (existing behavior)

Create the `label_color_overrides` table.

### Phase 2: Exchange category sync

Modify Exchange sync to write categories as `label_kind = 'tag'` rows in the `labels` table instead of (or in addition to) the `categories` table. Update `thread_labels` instead of `message_categories`.

### Phase 3: IMAP/JMAP keyword sync

Modify IMAP sync to write keywords as `label_kind = 'tag'` rows in the `labels` table. Respect `PERMANENTFLAGS` capability — only sync keywords the server supports.

Modify JMAP sync similarly for keywords.

### Phase 4: Unify ProviderOps

Remove `apply_category()`/`remove_category()` from ProviderOps. Update `add_tag()`/`remove_tag()` to handle tag-type labels (categories/keywords) in addition to container-type labels (folders).

### Phase 5: Sidebar restructure

Update sidebar to implement the four-section structure described above, using `label_kind` to route labels to section 2 (containers) or section 4 (tags).

### Phase 6: Deprecate old tables

Once all sync paths write to `labels`/`thread_labels`, the `categories` and `message_categories` tables can be dropped in a future migration.

## Accepted Trade-offs

- **False grouping**: Two accounts having "Personal" with different meanings will appear as one label in the sidebar. This is an accepted trade-off for a clean cross-account experience. Users who need disambiguation can rename labels on one account.

- **Gmail label hierarchy lost in section 4**: Gmail nested labels like "Projects/Alpha" appear as flat entries in the labels section. The hierarchy is visible in section 2 (provider folders) when the Gmail account is selected.

- **Renaming is complex**: Renaming a label means renaming on every account that has it. Partial failures leave split state. This is inherent to name-based grouping and is acceptable — renaming labels is rare.
