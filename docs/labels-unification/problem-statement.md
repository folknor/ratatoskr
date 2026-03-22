# Labels Unification: Problem Statement

## Overview

This document specifies how Ratatoskr unifies provider-specific email organization concepts (Gmail labels, Exchange folders + categories, IMAP folders + keywords, JMAP mailboxes + keywords) into a single user-facing label system.

The unification operates at three layers:

- **Presentation layer**: The user sees one label system. Labels appear as pills in the reading pane, as sidebar entries, and as search operators. The user does not need to know whether a label is backed by a Gmail label, an Exchange category, or an IMAP keyword.
- **Local model layer**: The `labels` table stores all provider objects with a `label_kind` field distinguishing containers (folders/mailboxes) from tags (categories/keywords). Canonical identity is `(account_id, provider_label_id)`. Cross-account grouping uses normalized display name as a presentation heuristic.
- **Remote reality layer**: Providers have fundamentally different object types. Gmail labels are dual-purpose (folder + tag). Exchange has separate folders and categories. IMAP has exclusive-membership mailboxes and optional keywords. JMAP has mailboxes and keywords. The local model and sync pipeline must respect these differences even when the UI abstracts them away.

## Non-Goals

This spec does NOT unify:

- **AI thread categorization** — the `thread_categories` table (Primary/Updates/Promotions/Social/Newsletters) is a separate computed classification system for inbox bundling. It is unrelated to user-facing labels.
- **Account-specific system folders** — universal folders (Inbox, Starred, Snoozed, Sent, Drafts, Trash) are handled by the existing navigation system, not the labels table.
- **Exclusive folder membership semantics** — the local model tracks container vs tag behavior. Moving a message between IMAP folders remains a move operation at the provider level, even though the UI presents it as label changes.
- **Provider-specific admin structures** — public folders, shared mailboxes, and delegated access are separate systems with their own specs.

## Current State

The codebase has three separate systems that should be one:

1. **Labels** (`labels` + `thread_labels` tables) — stores Gmail labels, IMAP folders, Exchange folders, JMAP mailboxes. Used by `add_tag()`/`remove_tag()` on ProviderOps.
2. **Exchange categories** (`categories` + `message_categories` tables) — synced from Exchange's `masterCategories` API. Has `apply_category()`/`remove_category()` on ProviderOps. Not displayed in sidebar.
3. **IMAP/JMAP keywords** — `apply_category()`/`remove_category()` are no-ops. Not stored or displayed.

## Target State

All provider tagging mechanisms (Exchange categories, IMAP keywords, JMAP keywords) are stored as rows in the `labels` table alongside existing provider folders/labels. Each row retains its `(account_id, id)` canonical identity.

The `apply_category()`/`remove_category()` methods on ProviderOps become redundant — `add_tag()`/`remove_tag()` handles everything.

The `categories` and `message_categories` tables become redundant for user-facing purposes (they may remain for sync state tracking during a transition period).

## Data Model

### Container vs Tag

Each label row has a `label_kind` field:

- **`container`** (folder/mailbox): Exclusive-membership semantics at the provider level. A message "lives in" a container. Provider operations use move semantics (IMAP COPY+DELETE, Exchange folder move, JMAP mailbox update). Displayed in the sidebar's folder section (section 2).
- **`tag`** (category/keyword): Additive semantics. A message can have any number of tags. Provider operations use flag/property set semantics (IMAP STORE +FLAGS, Exchange PATCH categories, JMAP keyword set). Displayed in the sidebar's labels section (section 4).

The UI presents both through the same label metaphor, but the ProviderOps dispatch layer uses `label_kind` to select the correct remote operation.

### Canonical Identity

Labels are identified by `(account_id, id)` — the provider-specific ID scoped to an account. This is the canonical identity used for all operations: sync, apply, remove, search.

**Name is NOT identity.** The normalized display name is used only for cross-account presentation grouping in the sidebar. Two labels with the same name on different accounts are distinct objects that happen to be displayed together.

### Name Normalization

For cross-account display grouping, label names are compared using:

- Case-insensitive comparison (handles IMAP keyword case insensitivity per RFC, and cross-provider mismatches like "work" vs "Work")
- Leading/trailing whitespace trimmed

Two labels with the same normalized name from different accounts appear as one entry in the sidebar's labels section. This is a **presentation grouping**, not semantic equivalence.

## Sidebar Structure

The sidebar has four sections. Only section 2 changes based on the account selector.

### All Accounts selected:
1. **Pinned searches** — ephemeral saved search snapshots
2. **Universal folders** — Inbox, Starred, Snoozed, Sent, Drafts, Trash
3. **Smart folders** — permanent saved queries
4. **Labels** — all tag-type labels from all accounts, grouped by normalized name

### Single Account selected:
1. **Pinned searches** — same (not scoped)
2. **Provider folders** — that account's container-type labels, with hierarchy preserved as the server provides it
3. **Smart folders** — same (not scoped)
4. **Labels** — same (not scoped, all accounts)

Pinned searches, smart folders, and labels are **always the same** regardless of which account is selected. Only the folder section changes.

### Labels section is flat

Labels in section 4 have no hierarchy. A Gmail user label named "Projects/Alpha" appears as a single flat entry with that name. Hierarchy is a provider folder concept — it belongs in section 2 only.

### Labels section ordering

Labels in section 4 are sorted **alphabetically by normalized name**. No drag-and-drop reordering — alphabetical is predictable and scales to large label counts. Users who want prioritized access to specific labels should use smart folders or pinned searches.

### Section 2 vs Section 4 semantics

Sections 2 and 4 have fundamentally different interaction semantics:

- **Section 2 (folders)**: Operations are **moves**. Clicking "Archive" removes from Inbox. Dragging a thread from Inbox to a folder moves it. These are exclusive-membership operations at the provider level (IMAP COPY+DELETE, Exchange folder move, JMAP mailbox update). `add_tag()`/`remove_tag()` is NOT the right abstraction for section 2 — folder operations use provider-specific move methods.
- **Section 4 (labels)**: Operations are **additive tags**. Applying a label adds it. Removing a label removes it. Neither affects folder membership. These are the operations that go through `add_tag()`/`remove_tag()` on ProviderOps.

## Provider Mapping

| Provider | Folders — section 2 (`label_kind = 'container'`) | Labels — section 4 (`label_kind = 'tag'`) |
|----------|--------------------------------------------------|-------------------------------------------|
| **Gmail** | System labels (`type: "system"`) — INBOX, SENT, TRASH, SPAM, DRAFTS, STARRED, CATEGORY_* | User labels (`type: "user"`) |
| **Exchange** | Exchange folders | Exchange categories |
| **IMAP** | IMAP mailboxes/folders | IMAP keywords (when server supports it — see below) |
| **JMAP** | JMAP mailboxes | JMAP keywords |

### Gmail

Gmail's API marks labels with `type: "system"` vs `type: "user"`. This is a product decision to use that API distinction for routing: system labels → folders, user labels → tags. Gmail does not enforce this boundary — all Gmail labels technically have the same behavior. But the split matches user expectations: INBOX and SENT feel like folders, "Receipts" and "Travel" feel like tags.

### IMAP keyword capability

IMAP keywords are only usable when the server advertises `PERMANENTFLAGS` that includes `\*` (arbitrary keywords allowed). When a server does not support arbitrary keywords:

- **Fixed keywords only**: If the server advertises specific permanent flags (e.g., `$label1` through `$label5`), those can be exposed as labels.
- **No custom flags**: If the server supports no custom flags at all, that account contributes no tag-type labels. The "apply label" action is unavailable for messages on that account — the UI grays it out or shows a notice.
- **Capability is checked once per session** and cached on the account's sync state.

### Provider keyword/category limits

Providers impose caps on the number of tags:

- **Exchange**: 25 categories maximum per mailbox
- **IMAP**: Server-dependent. Many servers cap custom keywords at ~30 (some less).
- **Gmail**: No hard cap on user labels (practical limit ~10,000).
- **JMAP**: Server-dependent. No standard cap.

When creating a label hits a provider cap, creation silently fails on that account. The label exists locally and on other accounts. The user is not prompted — the failure is logged and the label works everywhere except that account. If the user later tries to apply the label to a message on the capped account, the apply operation fails with a notice: "This account has reached its label limit."

## Label Colors

Color resolution priority:

1. **User-configured color** — the user explicitly set a color for this label name in Ratatoskr. Always wins. Stored in a local-only table keyed by normalized label name.
2. **Server-synced color** — from the provider (Gmail label color, Exchange category color preset). If exactly one backing label across all accounts provides an explicit server color, use it. If multiple accounts have conflicting server colors for the same name, fall through to hash.
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

When a user creates a new label in Ratatoskr, it creates a **tag-type entity** (not a folder/mailbox) on every account that supports it:

- **Gmail**: creates a new user label via Labels API
- **Exchange**: creates a new category via masterCategories API
- **IMAP**: implicit — keywords are created by first use (no pre-creation API). The label exists locally immediately and is written to the server when first applied to a message on that account.
- **JMAP**: implicit — same as IMAP.

Creation failures on individual accounts are non-fatal — the label exists locally and on accounts where creation succeeded. Accounts that don't support tag creation (e.g., IMAP servers without `PERMANENTFLAGS \*`) are silently skipped.

**Folder/mailbox creation is a separate operation.** It is NOT triggered by creating a label. Folders are created through a dedicated "create folder" action in section 2, which is provider-specific and account-scoped.

## Applying and Removing Labels

Users apply and remove labels via pills in the reading pane and message view pop-out.

### Apply to thread

When a user applies a label to a thread, Ratatoskr applies it to **all existing messages** in that thread. Future messages arriving in the thread do not automatically inherit the label — labels are applied to messages, not threads. (`thread_labels` is a rollup of per-message state, not an independent assignment.)

For each message, the correct provider operation is used based on the message's account:

- **Gmail**: `add_tag()` with the Gmail label ID resolved from the label name for that account
- **Exchange**: `add_tag()` sets a category on the message
- **IMAP**: `add_tag()` sets a keyword flag via `STORE +FLAGS`
- **JMAP**: `add_tag()` sets a keyword via `Email/set`

The provider-specific label ID is resolved by looking up the `labels` table for a matching `(account_id, name)` pair where `account_id` matches the message's account.

### Remove from thread

When a user removes a label from a thread, Ratatoskr removes it from **all messages** in that thread that currently have it. Same per-message provider dispatch as apply.

### Cross-account threads

Threads are always single-account in Ratatoskr (a thread belongs to one account). Apply/remove always targets one provider.

## Deletion

### Server-side deletion (incoming sync)

When a label is deleted server-side (e.g., an Exchange admin removes a category, or the user deletes it in another client):

- The label row is removed from the `labels` table for that account during sync
- `thread_labels` entries for that account are cleaned up by cascade
- If the same label name exists on other accounts, it remains in the sidebar
- No user prompt — deletion is silent and consistent with how folder deletion already works

### User-initiated deletion (from Ratatoskr UI)

When a user deletes a label from the sidebar's labels section (section 4):

- The label is deleted on **every account** that has a tag-type label with that normalized name
- Provider-specific delete operations are dispatched (Gmail delete label, Exchange delete category, IMAP/JMAP — keyword removal is implicit, no explicit delete)
- The `label_color_overrides` entry for that name is removed
- Deletion failures on individual accounts are non-fatal — the label is removed locally regardless

**Container-type labels (folders) are never deleted through the labels section.** Folder deletion is a separate action in section 2, scoped to one account, with its own confirmation flow.

## Sync

### Incoming (server → local)

Exchange category changes arrive via delta sync (`/messages?$deltatoken=...`). Changed messages include their full `categories` array. The sync pipeline diffs against local state and updates `labels` + `thread_labels`.

IMAP keyword changes arrive via `FLAGS` responses. CONDSTORE/QRESYNC provides efficient change detection.

JMAP keyword changes arrive via `Email/changes` + `Email/get`.

All three write to the `labels` + `thread_labels` tables, the same as existing folder/label sync.

### Outgoing (local → server)

Label apply/remove operations go through `add_tag()`/`remove_tag()` on ProviderOps, which uses `label_kind` to select the correct remote operation (folder move vs tag set).

### Thread-level rollup

Labels are applied at the message level on most providers, but `thread_labels` is thread-level. A thread has a label if **any message** in it has that label. This is intentionally sticky — once any message in a thread is labeled, the thread has that label until all messages with that label are either unlabeled or deleted.

**Recalculation triggers:**
- When a label is removed from a message, recalculate whether the thread still has that label (check remaining messages).
- When a labeled message is deleted, recalculate.
- When a new message arrives in the thread, it does NOT automatically inherit the thread's labels. The thread retains the label from existing messages; the new message is unlabeled.

The existing label sync already handles this rollup for Gmail labels and provider folders — the same logic applies to categories/keywords.

## Search Integration

The unified label system integrates with the search pipeline. The `label:` operator in the query language matches both container-type and tag-type labels by name:

```
label:Project-Alpha     — matches threads with that label (any account)
label:"Red Category"    — quoted names with spaces
```

Since labels are grouped by normalized name for display, `label:` search also matches by normalized name across all accounts — the same behavior as clicking the label in the sidebar.

Smart folder queries can reference labels: a smart folder with query `label:Important is:unread` shows unread threads with the "Important" label across all accounts.

## Unread Counts

Section 4 labels display unread counts. For a grouped label (same normalized name across multiple accounts), the unread count is the **sum** across all accounts:

```sql
SELECT COUNT(DISTINCT tl.thread_id)
FROM thread_labels tl
JOIN threads t ON tl.thread_id = t.id AND tl.account_id = t.account_id
JOIN labels l ON tl.label_id = l.id AND tl.account_id = l.account_id
WHERE LOWER(TRIM(l.name)) = ?1
  AND l.label_kind = 'tag'
  AND t.is_read = 0
```

Computed alongside navigation state loading and cached in sidebar state.

## Migration

### Phase 1: Schema ✅

Migration 67 adds `label_kind TEXT NOT NULL DEFAULT 'container'` to `labels`. Backfills `type='user'` rows to `label_kind='tag'`. Creates `label_color_overrides` table.

### Phase 2: Exchange category sync ✅

`graph_categories_sync` upserts categories as `label_kind='tag'` labels (prefixed `cat:`). `store_thread_to_db` writes category-backed `thread_labels` entries. Both run alongside legacy `categories`/`message_categories` writes.

### Phase 3: IMAP/JMAP keyword sync ✅

IMAP: `FlagChange` now carries `keywords: Vec<String>` extracted from `Flag::Custom`. `apply_flag_changes` writes keywords as `label_kind='tag'` labels (prefixed `kw:`) with `thread_labels` entries.

JMAP: `sync_keyword_categories` upserts keywords as tag-type labels alongside the legacy categories writes.

### Phase 4: Local label dispatch ✅

`EmailAction::AddLabel`/`RemoveLabel` now perform actual DB operations — `thread_labels` INSERT/DELETE for all selected threads. Local-first (optimistic); provider write-back via sync.

Note: `apply_category()`/`remove_category()` not yet removed from ProviderOps — deferred until provider client access is available from the app layer for full write-back.

### Phase 5: Sidebar restructure ✅

`FolderKind::AccountTag` variant added. `build_account_labels` routes by `label_kind`. New `build_all_account_tags` loads tag-type labels from all accounts with cross-account unread aggregation. Sidebar renders tags in section 4 ("LABELS"), always visible.

### Phase 6: Deprecate old tables

Deferred. Once all sync paths are verified on the unified system, drop `categories` and `message_categories` tables.

## Accepted Trade-offs

- **Presentation grouping is not semantic equivalence.** "Project Alpha" on a work Exchange account and "Project Alpha" on a personal Gmail account may be unrelated. They are grouped by name in the sidebar for convenience. Users who need disambiguation can rename labels on one account.

- **Gmail label hierarchy is lost in section 4.** Gmail nested labels like "Projects/Alpha" appear as flat entries in the labels section. The hierarchy is visible in section 2 (provider folders) when the Gmail account is selected.

- **Renaming is complex.** Renaming a label means renaming on every account that has it. Partial failures leave split state. This is inherent to name-based grouping and is acceptable — renaming labels is rare.

- **IMAP keyword support is not universal.** Some IMAP servers cannot store custom keywords. Those accounts silently have no tag-type labels.

- **Provider caps may silently limit labels.** Exchange (25 categories) and many IMAP servers (~30 keywords) have hard limits. Label creation and application fail silently on capped accounts.

## Future Considerations

- **Rename `thread_categories` table.** The AI inbox bundling system (`thread_categories`) shares the word "categories" with Exchange categories, causing confusion. A future rename to `thread_bundles` or `thread_classifications` would reduce ambiguity. Out of scope for this spec.
