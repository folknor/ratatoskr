# Folders and Labels Glossary

This document is the single source of truth for everything Ratatoskr classifies as a folder, a label, or neither. If code disagrees with this document, the code is wrong.

The whole job of this glossary is to answer one question: **for any primitive coming out of any provider, what is it in Ratatoskr?** Each rule below is binding on both user-facing copy and code identifiers.

---

## The Rule

Ratatoskr has exactly two organisation primitives: **folders** and **labels**. Everything else is either message state (a per-message boolean like read / starred) or transient (expunge markers, deprecated flags).

### Folder
<!-- coverage: glossary.folders_labels.folder_rows_are_containers enforcement=lua-harness -->

A container. A thread is in one or more folders. Operations are **moves** - when the user drags a thread from Inbox to Archive, the thread leaves Inbox and appears in Archive. The action service translates the move to whatever each provider needs (Gmail: add/remove labels. Graph: move API. IMAP: COPY + EXPUNGE. JMAP: mailbox membership update).

Stored in the `labels` table with `label_kind = 'container'`. Rendered inline in the sidebar's universal section: system folders (Inbox, Starred, Snoozed, Sent, Drafts, Archive, Trash, Spam, All Mail) in a fixed order, with the active account's user-created folders directly below Inbox when one account is scoped. There is no separate folders-section header; all containers share one navigation surface. **Folders never carry a coloured dot in the sidebar.**

### Label
<!-- coverage: glossary.folders_labels.label_rows_are_tags enforcement=lua-harness -->

An additive annotation. A thread carries any number of labels independently of which folder(s) it lives in. Operations are **apply** and **remove**, never "move". Stored in the `labels` table with `label_kind = 'tag'`. Rendered in the sidebar's collapsible LABELS section, **always with a coloured dot**. The coloured dot is the visual contract: dot = label, no dot = folder.

### Why both are in the `labels` table
<!-- coverage: glossary.folders_labels.labels_table_discriminates_folders_and_labels enforcement=lua-harness -->

The `labels` table is a storage-layer term that predates the Ratatoskr glossary. It stores both folders and labels, discriminated by `label_kind`: `'container'` for folders, `'tag'` for labels. The `thread_labels` junction table records associations between threads and both. For a folder row it means "this thread is in this folder"; for a label row it means "this thread has this label." This is the **only** place in the code where the word `label` is allowed to refer to anything except a tag.

### Code identifier rule

`folder` in a code identifier always refers to a container. `label` only ever refers to a tag, with the single storage-layer exception above (`labels` table, `label_kind` column, `thread_labels` junction). A type, function, enum variant, column, or message that names a folder with `label` is wrong and must be renamed.

Examples:
- `FolderKind::AccountFolder` - correct (a user-created provider folder).
- `FolderKind::AccountLabel` - correct (a user-created label).
- `FolderKind::AccountLabel` meaning a folder - wrong. Rename.
- `account_label_id: String` holding a folder ID - wrong. Rename.

If you're reading code and the name says "label" but the value is a folder, treat that as a bug to fix, not as a permission to mix terms in new code.

---

## Per-Provider Mapping
<!-- coverage: glossary.folders_labels.provider_terms_translate_to_folder_label_semantics enforcement=lua-harness -->

For each provider, exhaustively: what counts as a folder, what counts as a label, and what counts as message state. Anything not listed here is either not surfaced or is transient and ignored.

### Gmail

| Provider primitive | Ratatoskr classification |
|---|---|
| System labels (`INBOX`, `SENT`, `DRAFT`, `TRASH`, `SPAM`, `STARRED`, `IMPORTANT`, `CHAT`, `CATEGORY_*`) | **Folder** |
| User-created labels | **Label** |
| `UNREAD` system label absence | Message state - read |
| `STARRED` system label | Message state - starred (also rendered as the universal Starred folder for navigation; the underlying signal is the `is_starred` boolean) |
| Replied / forwarded | Message state - derived from `SENT` thread membership (see "Message state" below) |

Gmail's API permits system labels and user labels to coexist on a single message. Ratatoskr models the system ones as containers anyway because their UI semantics are move-style (archive removes `INBOX`, trash moves to `TRASH`, etc.).

### Microsoft Graph (Exchange)

| Provider primitive | Ratatoskr classification |
|---|---|
| Mail folders (`inbox`, `sentItems`, `drafts`, `deletedItems`, `junkEmail`, `archive`, user-created) | **Folder** |
| `categories[]` | **Label** |
| `importance` enum (`low` / `high`) | **Label** - synthesised `"Low importance"` / `"High importance"`. `normal` synthesises nothing. Mutually exclusive at write time: the action service clears the opposite slot when one is set. |
| `isRead` | Message state - read |
| `flag.flagStatus` | Message state - starred. Outlook's follow-up flag is the closest native analog; Ratatoskr loses the optional `startDateTime` / `dueDateTime` metadata. |
| `PR_LAST_VERB_EXECUTED` (extended MAPI property 0x1081) | Message state - replied / forwarded (see "Message state" below) |

Graph has no native starring primitive; the follow-up flag is the only option. This is an explicit, accepted trade-off - across providers a Ratatoskr "star" maps to whatever each provider's closest single-bit favourite-marker is.

### IMAP

| Provider primitive | Ratatoskr classification |
|---|---|
| Mailboxes - special-use (`\Inbox`, `\Sent`, `\Drafts`, `\Trash`, `\Junk`, `\Archive`) and user-created | **Folder** |
| Custom keywords (anything not in the system-flag list below) | **Label** |
| `\Seen` | Message state - read |
| `\Flagged` | Message state - starred |
| `\Answered` | Message state - replied |
| `$Forwarded` (RFC 5788 system keyword) | Message state - forwarded |
| `\Draft` | Folded into Drafts-folder membership; not a separate primitive |
| `\Deleted`, `\Recent` | Transient / deprecated; ignored |

`$Forwarded` lives in the IMAP keyword namespace technically, but Ratatoskr treats it as message state, not a user-visible label. The RFC 5788 system keywords (`$Forwarded`, `$MDNSent`, `$Junk`, `$NotJunk`, `$Phishing`) are all reserved and never appear in the LABELS section.

### JMAP

| Provider primitive | Ratatoskr classification |
|---|---|
| Mailboxes (system-role and user-created) | **Folder** |
| Custom keywords (anything outside the RFC system set) | **Label** |
| `$seen` | Message state - read |
| `$flagged` | Message state - starred |
| `$answered` | Message state - replied |
| `$forwarded` | Message state - forwarded |
| `$draft` | Folded into Drafts-folder membership; not a separate primitive |

JMAP technically allows a message to belong to multiple mailboxes (Gmail-influenced). Ratatoskr models system mailboxes as containers anyway, mirroring the Gmail treatment, because the UI semantics are move-style.

---

## Message State (Neither Folder Nor Label)

Some provider primitives are per-message booleans. They drive inline glyphs or filter behaviour and never appear in the sidebar.

| State | Column / source | Glyph | Provider sources |
|---|---|---|---|
| Read | `threads.is_read` (boolean) | - | Gmail: `UNREAD` absent · Graph: `isRead` · IMAP: `\Seen` · JMAP: `$seen` |
| Starred | `threads.is_starred` (boolean) | ★ | Gmail: `STARRED` system label · Graph: `flag.flagStatus == flagged` · IMAP: `\Flagged` · JMAP: `$flagged` |
| Replied | `messages.is_replied` (boolean) | ↩ | Gmail: derive from `SENT` thread membership + `In-Reply-To` / `References` headers · Graph: `PR_LAST_VERB_EXECUTED` ∈ {102 reply, 103 reply-all} · IMAP: `\Answered` · JMAP: `$answered` |
| Forwarded | `messages.is_forwarded` (boolean) | ↪ | Gmail: derive from `SENT` thread membership + `Subject` `Fwd:` / `FW:` prefix · Graph: `PR_LAST_VERB_EXECUTED == 104` · IMAP: `$Forwarded` system keyword · JMAP: `$forwarded` |

`is_replied` and `is_forwarded` are independent: both can be true on the same message (you replied, then later forwarded). Thread-level rendering ORs across messages.

---

## Identity
<!-- coverage: glossary.folders_labels.label_identity_is_account_scoped enforcement=lua-harness -->
<!-- coverage: glossary.folders_labels.system_folder_ids_are_canonical enforcement=lua-harness -->
<!-- coverage: glossary.folders_labels.non_system_ids_keep_provider_prefixes enforcement=lua-harness -->

The identifier for a folder or label is `(account_id, label_id)`. Names are presentational only; two labels with the same display name on different accounts are distinct objects.

For **system folders**, Ratatoskr defines its own canonical label IDs that are provider-independent. The sync pipeline normalises provider-native IDs to these on ingest.

| Ratatoskr label ID | What it represents |
|---|---|
| `"INBOX"` | Inbox |
| `"SENT"` | Sent |
| `"DRAFT"` | Drafts |
| `"TRASH"` | Trash / Deleted Items |
| `"SPAM"` | Spam / Junk |
| `"archive"` | Archive |
| `"STARRED"` | Starred / Flagged |
| `"SNOOZED"` | Snoozed |
| `"all-mail"` | All Mail (single-account only) |

`remove_label(conn, account_id, thread_id, "INBOX")` works for any provider - the canonical ID is provider-agnostic. The normalisation mapping lives in `SYSTEM_FOLDER_ROLES` (`crates/db/src/db/folder_roles.rs`, re-exported through `common::folder_roles`).

For **non-system folders and labels**, IDs are provider-specific with a crate prefix where required:

- Gmail user labels - native Gmail label ID, no prefix.
- Exchange categories - `cat:{name}`.
- IMAP keywords - `kw:{keyword}`.
- JMAP keywords - keyword string with the JMAP keyword prefix conventions.
- Graph user folders - `graph-{guid}`.
- JMAP user mailboxes - `jmap-{id}`.
- IMAP user folders - `folder-{path}`.

---

## Universal Folders

UI term for the aggregate view of a system folder across accounts. The universal section shows: Inbox, Starred, Snoozed, Sent, Drafts, Archive, Trash, Spam, and (in single-account scope) All Mail.

The Sent / Drafts / Archive / Trash / Spam aggregates are straightforward unions of the per-account canonical label IDs.

**Inbox is the exception.** In the All-Accounts scope, Inbox means "every thread that isn't a draft, sent, archived, trashed, or spam" - not just rows tagged with `INBOX`. This catches archived-but-relabelled mail, threads sitting only in user folders, and any orphan thread that lost its `INBOX` tag in some sync corner case. The single-account Inbox view keeps the strict `INBOX`-label semantics. Implementation: `BROAD_INBOX_EXCLUSIONS` in `crates/db/src/db/queries_extra/scoped_queries.rs`.

The "All Mail" universal item is single-account only - it shows literally every thread for one account (including drafts, sent, trash, spam) and has no meaningful cross-account aggregate.

---

## Operations

### Move (folder operation)

"Put this thread in that folder." Local DB: add the target folder's label ID to `thread_labels`, and remove the source folder's label ID if the operation implies removal (archive removes from Inbox; trash moves to Trash). Provider dispatch: Gmail modifies labels; IMAP does COPY + EXPUNGE; Graph uses the move API; JMAP updates mailbox memberships.

Archive, trash, spam, and move-to-folder are all moves. Not all moves remove from a source - a thread can be in multiple folders simultaneously when the provider permits.

### Apply / Remove (label operation)

Adding or removing a label. Local DB: insert or delete a row in `thread_labels`. Provider dispatch: IMAP STORE +FLAGS, Graph PATCH `categories`, JMAP keyword set, Gmail label modify. Does not affect folder membership.

### Message-state toggles (read, starred, replied, forwarded)

Not folder or label operations. Read and starred have UI controls. Replied and forwarded are derived from outgoing sends, not toggled directly. The action service routes the change to the appropriate provider primitive per the per-provider mapping above.

### Provider dispatch

The step where a local state change is propagated to the remote server. The action service does local DB first (optimistic), then provider dispatch. The action service owns this sequence - the app crate never dispatches to providers directly.

### Local-only by design

An action that intentionally has no provider dispatch. Pin and mute are local-only - no provider has a native equivalent. Distinct from "local-only because provider dispatch failed."

---

## Database Quick Reference

### `labels` table

All folders and labels for all accounts. Key columns:
- `id` - label ID (Ratatoskr canonical for system folders, provider-native with prefix for others).
- `account_id` - which account.
- `name` - display name.
- `label_kind` - `'container'` (folder) or `'tag'` (label).

### `thread_labels` table

Junction: `(account_id, thread_id, label_id)`. For folders: "this thread is in this folder." For labels: "this thread has this label."

### `threads` table - message-state columns

- `is_read`, `is_starred`, `is_snoozed`, `is_pinned`, `is_muted` - booleans driving sidebar filters and inline glyphs.

### `messages` table - message-state columns

- `is_replied`, `is_forwarded` - booleans driving thread-list glyphs. Thread-level renderings OR across messages.

---

## Terms NOT Used in Ratatoskr

These terms appear in provider documentation but are not Ratatoskr concepts. They never appear in user-facing copy and never appear in code identifiers except inside transport-layer or sync-layer code that's literally talking to a provider.

- **Tag** - not used. What providers call tags, categories, or keywords are *labels* in Ratatoskr. The `'tag'` value in `label_kind` is a storage-layer discriminant, not a user-facing concept.
- **Mailbox** - not used. What IMAP and JMAP call mailboxes are *folders* in Ratatoskr.
- **Category** - not used. What Exchange calls categories are *labels* in Ratatoskr.
- **Flag** (Outlook) - not used. The follow-up flag is mapped to the *starred* message state.
- **Verb** (Outlook) - not used. `PR_LAST_VERB_EXECUTED` is mapped to the *replied* and *forwarded* message states.
- **Importance** - not used as a top-level Ratatoskr concept. The `high` / `low` values become the synthesised labels `"High importance"` / `"Low importance"`; `normal` is no label.

Provider-native names still appear in code and docs where Ratatoskr is translating provider behaviour rather than defining user-facing concepts. For example:

- Exchange still has a `masterCategories` API even though Ratatoskr treats those objects as labels.
- Gmail still exposes provider-native `CATEGORY_*` bundle labels even though Ratatoskr treats inbox bundling as a separate bundles/classification system.

When those provider-native names appear, they are transport-layer or sync-layer terminology, not Ratatoskr concepts. They must not leak into core types, UI code, or user-facing strings.
