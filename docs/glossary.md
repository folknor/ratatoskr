# Ratatoskr Glossary

Canonical definitions for terms used across the codebase and documentation. When a term here conflicts with its provider-native meaning (Gmail, IMAP, Exchange, JMAP), the Ratatoskr definition wins.

All other documents should reference this glossary, not redefine terms.

---

## The Two Things

Ratatoskr has two kinds of email organization objects: **folders** and **labels**. That's it.

### Folder

A container. A message can be in one or more folders. The UI presents folder operations as **moves** — when a user drags a thread from Inbox to Archive, it leaves Inbox and appears in Archive. The action service translates this intent to whatever the provider requires (Gmail: remove one label, add another. IMAP: COPY + DELETE. Graph: move API. JMAP: update mailbox memberships).

Stored in the `labels` table with `label_kind = 'container'`.

Displayed in sidebar section 2 (provider folders).

**What providers call their folders:**

| Provider | Provider term | Ratatoskr term |
|----------|--------------|----------------|
| Gmail | "Label" (system type) | Folder |
| Exchange/Graph | Folder | Folder |
| IMAP | Mailbox | Folder |
| JMAP | Mailbox | Folder |

Gmail calls its folders "labels." Ratatoskr does not.

### Label

An annotation you stick on a message. A message can have any number of labels. Applying a label does not affect which folder the message is in. Operations on labels are **additive**.

Stored in the `labels` table with `label_kind = 'tag'`.

Displayed in sidebar section 4 (labels).

**What providers call their labels:**

| Provider | Provider term | Ratatoskr term |
|----------|--------------|----------------|
| Gmail | "Label" (user type) | Label |
| Exchange/Graph | Category | Label |
| IMAP | Keyword | Label |
| JMAP | Keyword | Label |

### Why both are in the `labels` table

The `labels` table stores both folders and labels. The `label_kind` column distinguishes them: `'container'` for folders, `'tag'` for labels. This is a storage-level discriminant, not a separate concept. There is one table, two kinds of rows.

The `thread_labels` junction table records associations between threads and both folders and labels. For a folder row, it means "this thread is in this folder." For a label row, it means "this thread has this label."

---

## Identity

### Label ID

The identifier for a folder or label, scoped to an account: `(account_id, label_id)`.

For **system folders** (Inbox, Trash, Spam, etc.), Ratatoskr defines its own canonical label IDs that are provider-independent. The sync pipeline normalizes provider-native IDs to these on ingest:

| Ratatoskr label ID | What it represents |
|---------------------|--------------------|
| `"INBOX"` | Inbox |
| `"TRASH"` | Trash / Deleted Items |
| `"SPAM"` | Spam / Junk |
| `"SENT"` | Sent |
| `"DRAFT"` | Drafts |
| `"archive"` | Archive |
| `"STARRED"` | Starred / Flagged |

Every provider's inbox is stored with label ID `"INBOX"`, regardless of what the provider calls it natively. A Graph account's inbox (which has an opaque GUID on the server) is stored as `"INBOX"` in the local DB. This means `remove_label(conn, account_id, thread_id, "INBOX")` works for any provider.

The normalization mapping lives in `SYSTEM_FOLDER_ROLES` (`crates/provider-utils/src/folder_roles.rs`).

**Verified:** All four provider sync pipelines write canonical IDs for system folders:
- **Gmail** — native IDs happen to match canonical (`"INBOX"`, `"TRASH"`, etc.). Written directly.
- **Graph** — well-known folder aliases resolved to canonical IDs via `graph_well_known_aliases()` in `crates/graph/src/folder_mapper.rs`. User folders stored as `graph-{guid}`.
- **JMAP** — server-assigned mailbox IDs resolved to canonical IDs via `system_folder_by_jmap_role()` in `crates/jmap/src/mailbox_mapper.rs`. User mailboxes stored as `jmap-{id}`.
- **IMAP** — special-use attributes resolved to canonical IDs via `imap_special_use_to_label_id()` in `crates/imap/src/folder_mapper.rs`. Name-based fallback for servers without special-use. User folders stored as `folder-{path}`.

This means `remove_label(conn, account_id, thread_id, "INBOX")` works for any provider. The action service can use canonical label IDs directly for system folder operations without per-account resolution.

For **non-system folders and labels**, the label ID is provider-specific with a crate prefix:
- Gmail user labels: native Gmail label ID (no prefix — Gmail IDs are already unique strings)
- Exchange categories: prefixed `cat:{name}`
- IMAP keywords: prefixed `kw:{keyword}`
- JMAP keywords: prefixed as keywords in the labels table
- Graph user folders: prefixed `graph-{guid}`
- JMAP user mailboxes: prefixed `jmap-{id}`
- IMAP user folders: prefixed `folder-{path}`

### Name is not identity

Two labels with the same display name on different accounts are distinct objects. Identity is `(account_id, label_id)`. The normalized display name is used only for cross-account presentation grouping in the sidebar's labels section.

---

## System Folders

### System folder

One of the well-known folders that every email account has: Inbox, Sent, Drafts, Trash, Spam, Archive, Starred. Stored in the `labels` table with canonical Ratatoskr label IDs (see above). Discovered during sync and mapped via `SYSTEM_FOLDER_ROLES`.

### Universal folder

UI term for the aggregate view of a system folder across all accounts. When no specific account is selected, the sidebar shows "Inbox" (all accounts' inboxes combined), "Trash" (all accounts' trash combined), etc. These are virtual — they query across accounts using the canonical label ID.

---

## Operations

### Move (folder operation)

A user-facing operation: "put this thread in that folder." At the local DB level: add the target folder's label ID to `thread_labels`, and remove the source folder's label ID if the operation implies removal (e.g., archive removes from inbox). At the provider level: whatever the provider needs (Gmail: modify labels. IMAP: COPY + DELETE. Graph: move API. JMAP: update mailbox memberships).

Archive, trash, spam, and move-to-folder are all moves. Not all moves remove from a source — a thread can be in multiple folders simultaneously.

### Apply / Remove (label operation)

Adding or removing a label from a thread. Local DB: insert or delete a row in `thread_labels`. Provider: flag/property set operations (IMAP STORE +FLAGS, Exchange PATCH categories, JMAP keyword set, Gmail label modify).

Does not affect folder membership.

### Provider dispatch

The step where a local state change is propagated to the remote server. The action service does local DB first (optimistic), then provider dispatch. The action service owns this sequence — the app crate never dispatches to providers directly.

### Local-only by design

An action that intentionally has no provider dispatch. Pin and mute are local-only — no provider has a native equivalent. Distinct from "local-only because provider dispatch failed."

---

## Provider Translation

When reading provider documentation or provider crate code, use this table to translate back to Ratatoskr terms:

| Provider says | Ratatoskr means |
|---------------|-----------------|
| Gmail "label" (system) | Folder |
| Gmail "label" (user) | Label |
| Gmail "modify labels" | Move (if system labels) or Apply/Remove (if user labels) |
| Exchange "folder" | Folder |
| Exchange "category" | Label |
| IMAP "mailbox" | Folder |
| IMAP "keyword" | Label |
| IMAP "flag" | Depends — `\Seen`, `\Flagged` etc. are boolean fields, not labels |
| JMAP "mailbox" | Folder |
| JMAP "keyword" | Label |

---

## Database Quick Reference

### `labels` table

All folders and labels for all accounts. Key columns:
- `id` — label ID (Ratatoskr canonical for system folders, provider-native for others)
- `account_id` — which account
- `name` — display name
- `label_kind` — `'container'` (folder) or `'tag'` (label)

### `thread_labels` table

Junction table: `(account_id, thread_id, label_id)`. For folders: "this thread is in this folder." For labels: "this thread has this label."

### `label_kind` values

- `'container'` — this row is a folder
- `'tag'` — this row is a label

These are storage discriminants. The user-facing concepts are "folder" and "label."

---

## Terms NOT Used in Ratatoskr

These terms appear in provider documentation but are **not** Ratatoskr concepts:

- **Tag** — not a Ratatoskr term. What providers call tags, categories, or keywords are **labels** in Ratatoskr. The `'tag'` value in `label_kind` is a database discriminant, not a concept.
- **Mailbox** — not a Ratatoskr term. What IMAP/JMAP call mailboxes are **folders** in Ratatoskr.
- **Category** — not a Ratatoskr term. What Exchange calls categories are **labels** in Ratatoskr.

## Known Terminology Debt: "category" in the Codebase

The word "category" currently means three unrelated things in the code. This is a source of confusion and needs cleanup.

### 1. Provider labels (Exchange categories, IMAP keywords, JMAP keywords)

The old pre-unification system. `apply_category()`/`remove_category()` on `ProviderOps`, the `categories` table, the `message_categories` table, `graph/src/category_sync.rs`. These should all be renamed to use "label" terminology, and the `categories`/`message_categories` tables should be dropped once all sync paths use the unified `labels`/`thread_labels` system (labels unification Phase 6).

**Files:** `provider-utils/src/ops.rs` (trait methods), `graph/src/category_sync.rs`, `gmail/src/ops.rs`, `jmap/src/ops.rs`, `imap/src/ops.rs`, `db/src/db/migrations.rs` (table definitions), `core/src/actions/label.rs` (dispatch routing).

### 2. AI inbox bundles (Primary, Updates, Promotions, Social, Newsletters)

The `thread_categories` table and `sync/src/categorization.rs`. This is an automated classification system for inbox bundling — completely unrelated to user-facing labels. Should be renamed to `thread_bundles` / `bundling` / `classifications` to eliminate the name collision.

**Files:** `sync/src/categorization.rs`, `core/src/db/queries_extra/bundles_categories.rs`, `db/src/db/queries.rs`, `ai/src/` (orchestration, parsing, types), `sync/src/notifications.rs`, `app/src/command_dispatch.rs` (`CATEGORY_PRIMARY` etc.).

### 3. Exchange color presets

`label-colors/src/category_colors.rs` — the 25 Exchange preset colors used as the canonical label color palette. The colors themselves are fine; the module name should change to `preset_colors.rs` or `exchange_presets.rs`.

**Files:** `label-colors/src/category_colors.rs`, `label-colors/src/lib.rs`.

### Cleanup plan

This is not urgent but should happen before 1.0. Each is independent:

- [ ] Rename `apply_category`/`remove_category` → merge into `add_tag`/`remove_tag` or rename to `apply_label`/`remove_label` on `ProviderOps`. Label dispatch in `core/src/actions/label.rs` routes by `label_kind` and would need updating.
- [ ] Drop `categories` and `message_categories` tables (labels unification Phase 6).
- [ ] Rename `thread_categories` → `thread_bundles`, `categorization.rs` → `bundling.rs`, `bundles_categories.rs` → `bundles.rs`.
- [ ] Rename `CATEGORY_PRIMARY` etc. → `BUNDLE_PRIMARY` etc. in `command_dispatch.rs` and navigation.
- [ ] Rename `category_colors.rs` → `preset_colors.rs` or `exchange_presets.rs`.
- [ ] Rename `category_sync.rs` → `label_sync.rs` in the graph crate.
