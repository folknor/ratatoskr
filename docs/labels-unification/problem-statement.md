# Labels Unification: Problem Statement

## Overview

Ratatoskr treats all email organization as **tags/labels**. There are no "folders" in Ratatoskr's model — a message either has a label or it doesn't. Under the hood, providers use different mechanisms (Gmail labels, Exchange folders + categories, IMAP folders + keywords, JMAP mailboxes + keywords), but the user sees one unified label system.

This document specifies how provider-specific concepts are unified, how the sidebar displays them, and how the user interacts with labels across accounts.

## Current State

The codebase has three separate systems that should be one:

1. **Labels** (`labels` + `thread_labels` tables) — stores Gmail labels, IMAP folders, Exchange folders, JMAP mailboxes. Used by `add_tag()`/`remove_tag()` on ProviderOps.
2. **Exchange categories** (`categories` + `message_categories` tables) — synced from Exchange's `masterCategories` API. Has `apply_category()`/`remove_category()` on ProviderOps. Not displayed in sidebar.
3. **IMAP/JMAP keywords** — `apply_category()`/`remove_category()` are no-ops. Not stored or displayed.

Additionally, there is a separate **AI thread categorization** system (`thread_categories` table) for inbox bundling (Primary/Updates/Promotions/Social/Newsletters). This is unrelated to user-facing labels and is NOT part of this unification.

## Target State

All provider tagging mechanisms (Exchange categories, IMAP keywords, JMAP keywords) are stored as rows in the `labels` table alongside existing provider folders/labels. The `apply_category()`/`remove_category()` methods on ProviderOps become redundant — `add_tag()`/`remove_tag()` handles everything.

The `categories` and `message_categories` tables become redundant for user-facing purposes (they may remain for sync state tracking during a transition period).

## Sidebar Structure

The sidebar has four sections. Only section 2 changes based on the account selector.

### All Accounts selected:
1. **Pinned searches** — ephemeral saved search snapshots
2. **Universal folders** — Inbox, Starred, Snoozed, Sent, Drafts, Trash
3. **Smart folders** — permanent saved queries
4. **Labels** — all labels merged across all accounts, deduplicated by name

### Single Account selected:
1. **Pinned searches** — same as above (not scoped)
2. **Provider folders** — that account's server-side folder structure (Gmail labels, IMAP folders, Exchange folders, JMAP mailboxes)
3. **Smart folders** — same as above (not scoped)
4. **Labels** — same as above (not scoped, all accounts merged)

Pinned searches, smart folders, and labels are **always the same** regardless of which account is selected. Only the folder section changes.

## Label Identity

Labels are identified by **name**, not by provider-specific ID. The sidebar shows deduplicated label names. When a label called "Project Alpha" exists on multiple accounts:

- It appears once in the sidebar
- Clicking it queries all accounts for threads with that label
- Applying it to a message creates the appropriate provider-specific entity (Gmail label, Exchange category, IMAP keyword, JMAP keyword)

## Label Colors

Color resolution priority:

1. **User-configured color** — the user explicitly set a color for this label name in Ratatoskr. Always wins. Stored in a local-only `label_color_overrides` table keyed by label name.
2. **Server-synced color** — from the provider (Gmail label color, Exchange category color preset). First account by sort order breaks ties when multiple accounts have the same label with different colors.
3. **Hash fallback** — deterministic color from the 25-preset palette based on the label name string.

### Color override storage

```sql
CREATE TABLE IF NOT EXISTS label_color_overrides (
    label_name TEXT PRIMARY KEY,
    color_bg TEXT NOT NULL
);
```

Local-only, never synced to any provider.

## Creating Labels

When a user creates a new label in Ratatoskr:

- The label is created on **every account** that supports it
- Gmail: creates a new label via Labels API
- Exchange: creates a new category via masterCategories API
- IMAP: keyword creation is implicit (keywords are set per-message, no pre-creation needed)
- JMAP: keyword creation is implicit

Creation failures on individual accounts are non-fatal — the label still exists locally and on accounts where creation succeeded.

## Applying/Removing Labels

Users apply and remove labels via pills in the reading pane and message view pop-out.

When a label is applied to a message:
- The correct provider operation is used for that message's account (`add_tag()` with the provider-specific label ID resolved from the label name)
- For Exchange messages, this means setting a category (currently `apply_category()`, to be unified into `add_tag()`)
- For IMAP messages, this means setting a keyword flag
- For JMAP messages, this means setting a keyword

When a label is removed, the inverse operation applies.

## Provider Mapping

| Provider | Folders (section 2) | Labels/Tags (section 4) |
|----------|-------------------|------------------------|
| **Gmail** | All Gmail labels (treated as folders since we can't distinguish) | — (Gmail labels already in folders section) |
| **Exchange** | Exchange folders | Exchange categories |
| **IMAP** | IMAP mailboxes/folders | IMAP keywords |
| **JMAP** | JMAP mailboxes | JMAP keywords |

Gmail is the exception — since Gmail doesn't distinguish between folders and tags, all Gmail labels appear in the provider folders section (section 2) when that account is selected. They do NOT also appear in the labels section.

## Sync

### Incoming (server → local)

Exchange category changes arrive via delta sync (`/messages?$deltatoken=...`). Changed messages include their full `categories` array. The sync pipeline diffs against local state and updates `thread_labels`.

IMAP keyword changes arrive via `FLAGS` responses. CONDSTORE/QRESYNC provides efficient change detection.

JMAP keyword changes arrive via `Email/changes` + `Email/get`.

All three write to the `labels` + `thread_labels` tables, the same as existing folder/label sync.

### Outgoing (local → server)

Label apply/remove operations go through `add_tag()`/`remove_tag()` on ProviderOps, which translates to the correct API call per provider.

### Thread-level rollup

Labels are applied at the message level on most providers, but `thread_labels` is thread-level. A thread has a label if any message in it has that label. The existing label sync already handles this rollup for Gmail labels and provider folders — the same logic applies to categories/keywords.

## Open Questions

1. **Server-side deletion** — When a label is deleted server-side (e.g., an Exchange admin removes a category), it disappears from that account's data during sync. If the same label name exists on other accounts, it remains in the sidebar. Threads from the affected account lose the tag. Is this the right behavior, or should we prompt the user?

2. **Gmail label ambiguity** — Since all Gmail labels are treated as folders in section 2, Gmail users effectively have no section 4 labels. Is this acceptable? Could we heuristically distinguish user-created Gmail labels (which behave like tags) from system labels (which behave like folders)?
