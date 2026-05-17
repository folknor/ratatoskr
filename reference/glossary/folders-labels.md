# Folders and Labels Glossary

This document is the single source of truth for everything Ratatoskr classifies as a folder, a label, or neither. If code disagrees with this document, the code is wrong.

The whole job of this glossary is to answer one question: **for any primitive coming out of any provider, what is it in Ratatoskr?** Each rule below is binding on both user-facing copy and code identifiers.

---

## The Rule

Ratatoskr has exactly two organisation primitives: **folders** and **labels**. Everything else is either message state (a per-message boolean like read / starred) or transient (expunge markers, deprecated flags).

### Folder
<!-- coverage: glossary.folders_labels.folder_rows_are_containers enforcement=lua-harness -->

A container. A thread is in one or more folders. Operations are **moves** - when the user drags a thread from Inbox to Archive, the thread leaves Inbox and appears in Archive. The action service translates the move to whatever each provider needs (Gmail: add/remove labels. Graph: move API. IMAP: COPY + EXPUNGE. JMAP: mailbox membership update).

Stored in the `folders` table. Thread membership is stored in `thread_folders`. Rendered inline in the sidebar's universal section: system folders (Inbox, Sent, Drafts, Archive, Trash, Spam) in a fixed order, with virtual Starred, Snoozed, and All Mail navigation rows backed by thread booleans or no filter. The active account's user-created folders render directly below Inbox when one account is scoped. There is no separate folders-section header; all containers share one navigation surface. **Folders never carry a coloured dot in the sidebar.**

### Label
<!-- coverage: glossary.folders_labels.label_rows_are_tags enforcement=lua-harness -->

An additive annotation. A thread carries any number of labels independently of which folder(s) it lives in. Operations are **apply** and **remove**, never "move". Raw provider labels are stored in the `labels` table and thread membership is stored in `thread_labels`.

User-visible sidebar labels are explicit `label_groups`, not automatic name-collapsed provider labels. A group has its own name and colour. It renders from the user-visible label set: provider truth in `thread_labels` plus pending local intent in `pending_thread_label_intents`, joined through `label_group_members`. The sidebar LABELS section starts empty until the user creates groups.

### Storage split
<!-- coverage: glossary.folders_labels.storage_splits_folders_labels_and_groups enforcement=lua-harness -->

Folders and labels are separate storage-layer concepts. `folders` contains provider folders and system folder roles. `labels` contains only provider labels, categories, and keywords. `thread_folders` records folder membership. `thread_labels` records raw provider-label membership. `label_groups` and `label_group_members` record the user-visible grouped label model. `pending_thread_label_intents` records optimistic local label intent while provider truth is pending.

### Code identifier rule

`folder` in a code identifier always refers to a container. `label` only ever refers to a provider label. `label_group` refers to the user-visible grouped label. A type, function, enum variant, column, or message that names a folder with `label` is wrong and must be renamed.

Examples:
- `FolderKind::AccountFolder` - correct (a user-created provider folder).
- `FolderKind::LabelGroup` - correct (a user-created grouped label).
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
| System labels (`INBOX`, `SENT`, `DRAFT`, `TRASH`, `SPAM`, `IMPORTANT`, `CHAT`, `CATEGORY_*`) | **Folder** |
| User-created labels | **Label** |
| `UNREAD` system label absence | Message state - read |
| `STARRED` system label | Message state - starred (also rendered as the universal Starred folder for navigation; the underlying signal is the `is_starred` boolean) |
| Replied / forwarded | Message state - derived from `SENT` thread membership (see "Message state" below) |

Gmail's API permits system labels and user labels to coexist on a single message. Ratatoskr models the system ones as containers anyway because their UI semantics are move-style (archive removes `INBOX`, trash moves to `TRASH`, etc.).

`STARRED` is intentionally absent from the system-labels row above. It is **not** a folder in Ratatoskr - it is the message-state row below, backed by `threads.is_starred`. The universal Starred sidebar entry is a virtual navigation handle queried via `get_starred_threads`, not a `folders` row; see "Identity" below for the virtual navigation IDs that never appear in any membership table.

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

`$Forwarded` lives in the IMAP keyword namespace technically, but Ratatoskr treats it as message state, not a user-visible label. The RFC 5788 system keywords (`$Forwarded`, `$MDNSent`, `$Junk`, `$NotJunk`, `$Phishing`) are all reserved and never appear in the LABELS section. Per RFC 5788 section 2.1 the `$` prefix is reserved for IETF-defined system keywords, so Ratatoskr filters every `$`-prefixed keyword from the LABELS section, not just the named five - this is intentionally stricter than the named-set rule, on the principle that a server's future-defined `$Whatever` should never silently appear as a user label.

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
| Read | `threads.is_read` (boolean) | - | Gmail: `UNREAD` absent; Graph: `isRead`; IMAP: `\Seen`; JMAP: `$seen` |
| Starred | `threads.is_starred` (boolean) | star | Gmail: `STARRED` system label; Graph: `flag.flagStatus == flagged`; IMAP: `\Flagged`; JMAP: `$flagged` |
| Replied | `messages.is_replied` (boolean) | reply | Gmail: derive from `SENT` thread membership + `In-Reply-To` / `References` headers; Graph: `PR_LAST_VERB_EXECUTED` in {102 reply, 103 reply-all}; IMAP: `\Answered`; JMAP: `$answered` |
| Forwarded | `messages.is_forwarded` (boolean) | forward | Gmail: derive from `SENT` thread membership + `Subject` `Fwd:` / `FW:` prefix; Graph: `PR_LAST_VERB_EXECUTED == 104`; IMAP: `$Forwarded` system keyword; JMAP: `$forwarded` |

`is_replied` and `is_forwarded` are independent: both can be true on the same message (you replied, then later forwarded). Thread-level rendering ORs across messages.

### Thread-aggregate semantics

Thread-level state has two sources of truth, depending on what is being aggregated:

- **Per-message booleans aggregate per-field - the reducer is not uniform.**
    - `is_read` is **all non-reaction messages read** (MIN over per-message `is_read`, equivalently "`COUNT(*) WHERE is_read = 0 AND is_reaction = 0` is 0"). A thread is read only when every non-reaction message in it is read.
    - `is_starred`, `is_replied`, `is_forwarded` are **any non-reaction message** with the flag set (MAX / `EXISTS`). A thread is starred when at least one non-reaction message in it is starred; same for replied and forwarded.
    - `last_message_at` is `MAX(date)` over non-reaction messages.

    The reducer for each field is fixed. Adding a per-message boolean means: schema column + parser extraction + naming the reducer explicitly. Do not assume the reducer matches its neighbours; `is_read` is the only MIN, the others are ANY.

    The `query_thread_state_decorations` helper (`crates/db/src/db/queries_extra/thread_detail.rs`) computes thread-level decorations on read; the `recompute_thread_read_starred` helper writes the `threads` aggregates. Both helpers must apply the `is_reaction = 0` filter and the correct per-field reducer.
- **Folder / label memberships** are thread-level aggregates. Folder membership lives in `thread_folders`. Raw provider label membership lives in `thread_labels`. Pending same-client label intent lives in `pending_thread_label_intents` and is merged only by user-facing reads.
    - **Gmail and IMAP** sync the entire thread's message metadata before writing the aggregate. The provider-sync entry point is `replace_thread_membership_from_full_coverage` (`crates/provider-sync/src/thread_membership.rs`); destructive replace is safe because the input is the full union.
    - **Graph and JMAP** receive partial delta pages (only changed messages, not the full thread). They write per-message ground truth through `message_folders` and `message_labels` (Graph) or `message_folders` + `message_keywords` (JMAP), and the thread-level aggregate is recomputed from the per-message union via `replace_message_membership_and_recompute`. A cross-client move that subtracts the source folder/label from every message is observable in the per-message tables, so the stale source-folder row gets cleaned up on recompute.

Same-client moves are correct under both schemes because the action service updates `thread_folders` locally (removing the source folder, adding the target) before dispatching to the provider. Same-client label actions write `pending_thread_label_intents` and let provider-truth writes clear matching intent rows; see `crates/db/src/db/queries_extra/label_intent.rs` for the overlay lifecycle.

---

## Identity
<!-- coverage: glossary.folders_labels.label_identity_is_account_scoped enforcement=lua-harness -->
<!-- coverage: glossary.folders_labels.system_folder_ids_are_canonical enforcement=lua-harness -->
<!-- coverage: glossary.folders_labels.non_system_ids_keep_provider_prefixes enforcement=lua-harness -->

The identifier for a folder is `(account_id, folder_id)`. The identifier for a raw provider label is `(account_id, label_id)`. Names are presentational only; two raw labels with the same display name on different accounts are distinct objects unless the user explicitly groups them.

For **system folders**, Ratatoskr defines its own canonical label IDs that are provider-independent. The sync pipeline normalises provider-native IDs to these on ingest.

| Ratatoskr label ID | What it represents |
|---|---|
| `"INBOX"` | Inbox |
| `"SENT"` | Sent |
| `"DRAFT"` | Drafts |
| `"TRASH"` | Trash / Deleted Items |
| `"SPAM"` | Spam / Junk |
| `"archive"` | Archive |

The canonical ID is provider-agnostic. The normalisation mapping lives in `SYSTEM_FOLDER_ROLES` (`crates/db/src/db/folder_roles.rs`, re-exported through `common::folder_roles`).

Virtual navigation IDs are not folder rows: `"STARRED"` maps to `threads.is_starred`, `"SNOOZED"` maps to `threads.is_snoozed`, and `"all-mail"` means the single-account no-filter view. No `folders`, `labels`, `thread_folders`, or `thread_labels` row uses these virtual IDs.

For **non-system folders and labels**, IDs are provider-specific with a crate prefix where required:

- Gmail user labels - native Gmail label ID, no prefix.
- Exchange categories - `cat:{name}`.
- Exchange importance labels - `importance:high` / `importance:low`.
- IMAP keywords - `kw:{keyword}`.
- JMAP keywords - `kw:{keyword}` (same convention as IMAP).
- Graph user folders - `graph-{guid}`.
- JMAP user mailboxes - `jmap-{id}`.
- IMAP user folders - `folder-{path}`.

---

## Universal Folders

UI term for the aggregate view of a system folder across accounts. The universal section shows: Inbox, Starred, Snoozed, Sent, Drafts, Archive, Trash, Spam, and (in single-account scope) All Mail.

The Sent / Drafts / Archive / Trash / Spam aggregates are straightforward unions of the per-account canonical label IDs.

**Inbox is the exception.** In the All-Accounts scope, Inbox means "every thread that isn't a draft, sent, archived, trashed, or spam" - not just rows tagged with `INBOX`. This catches archived-but-relabelled mail, threads sitting only in user folders, and any orphan thread that lost its `INBOX` tag in some sync corner case. The single-account Inbox view keeps the strict `INBOX`-label semantics. Implementation: `BROAD_INBOX_EXCLUSIONS` in `crates/db/src/db/queries_extra/scoped_queries.rs`.

The "All Mail" universal item is single-account only - it shows literally every thread for one account (including drafts, sent, trash, spam) and has no meaningful cross-account aggregate.

**Query routing.** Most universal folders are queried via `get_threads_scoped` with the canonical folder_id (`INBOX`, `SENT`, `DRAFT`, `TRASH`, `SPAM`, `archive`), which joins `thread_folders`. Three are routed differently because their underlying signal is a boolean column on `threads` or no filter at all:

- **Starred** dispatches to `get_starred_threads` (queries `threads.is_starred = 1`).
- **Snoozed** dispatches to `get_snoozed_threads` (queries `threads.is_snoozed = 1`).
- **All Mail** intercepts to a `None` label_id so the no-filter scoped query runs and returns every thread for the account.
- **Inbox** uses the strict `INBOX` label_id in single-account scope and applies `BROAD_INBOX_EXCLUSIONS` in All-Accounts scope.

The dispatch happens in `crates/app/src/helpers.rs::load_threads_scoped` and `thread_query_label_for_selection`. Shared-mailbox views apply the same boolean-column routing for Starred and Snoozed via `get_threads_for_shared_mailbox`. No stored membership row references the virtual navigation IDs.

---

## Operations

### Move (folder operation)

"Put this thread in that folder." Local DB: add the target folder ID to `thread_folders`, and remove the source folder ID if the operation implies removal (archive removes from Inbox; trash moves to Trash). Provider dispatch: Gmail modifies labels; IMAP does COPY + EXPUNGE; Graph uses the move API; JMAP updates mailbox memberships.

Archive, trash, spam, and move-to-folder are all moves. Not all moves remove from a source - a thread can be in multiple folders simultaneously when the provider permits.

### Apply / Remove (label operation)

Adding or removing a user-visible label group. Local DB: upsert per-member rows in `pending_thread_label_intents`. Provider dispatch fans out to the group's raw provider-label members for the thread's account: IMAP STORE +FLAGS, Graph PATCH `categories`, JMAP keyword set, Gmail label modify. Successful provider-observable writes are reflected in `thread_labels` as confirmed provider truth and clear matching pending intents. Does not affect folder membership.

Raw provider-label apply/remove exists for sync, Settings, and the composite group fan-out path. The message UI operates on `label_groups`, not raw `(account_id, label_id)` rows.

### Message-state toggles (read, starred, replied, forwarded)

Not folder or label operations. Read and starred have UI controls. Replied and forwarded are derived from outgoing sends, not toggled directly. The action service routes the change to the appropriate provider primitive per the per-provider mapping above.

**Replied / forwarded write semantics on send.** When the user sends a Reply or Forward from Ratatoskr, two writes happen, in order:

1. **Local mark.** The action service flips `messages.is_replied` (or `is_forwarded`) on the source message immediately via `service::send::mark_send_intent_local`. This is authoritative: the glyph appears in the UI on the next thread-list decoration, no sync round-trip required.
2. **Provider write-back, best-effort.** The provider's `mark_send_intent` is then called with the source message's local DB ID and the `SendIntent`. Per provider:
    - **IMAP**: `STORE +FLAGS (\Answered)` for Reply, `STORE +FLAGS ($Forwarded)` for Forward.
    - **JMAP**: `EmailSet` keyword `$answered` (Reply) or `$forwarded` (Forward).
    - **Graph**: PATCH `singleValueExtendedProperties` for `PR_LAST_VERB_EXECUTED` (`102` reply, `104` forward).
    - **Gmail**: no explicit write; the next sync ingests the SENT message, the parser derives `is_replied` from `SENT` membership + `In-Reply-To` / `References`, and the thread aggregate picks it up.

If the provider write fails (e.g. Graph rejecting the extended-property PATCH on certain message states), the failure is logged at warn level and the local state remains the source of truth. Other clients viewing the same mailbox will not see the bit until either the user replies/forwards again from a different client, or a future reconciliation pass syncs the local state out.

### Provider dispatch

The step where a local state change is propagated to the remote server. The action service does local DB first (optimistic), then provider dispatch. The action service owns this sequence - the app crate never dispatches to providers directly.

### Local-only by design

An action that intentionally has no provider dispatch. Pin and mute are local-only - no provider has a native equivalent. Distinct from "local-only because provider dispatch failed."

---

## Database Quick Reference

### `folders` table

Provider folders and canonical system folders for all accounts. Key columns:
- `id` - folder ID (Ratatoskr canonical for system folders, provider-native with prefix for others).
- `account_id` - which account.
- `name` - display name.
- `parent_id` - optional folder hierarchy.

### `labels` table

Raw provider labels for all accounts. Key columns:
- `id` - label ID (provider-native with prefix where needed).
- `account_id` - which account.
- `name` - display name.

### `thread_folders` table

Junction: `(account_id, thread_id, folder_id)`. A row means "this thread is in this folder." This is a thread-level aggregate: a row exists when at least one message in the thread carries the folder membership.

### `thread_labels` table

Junction: `(account_id, thread_id, label_id)`. A row means "this thread has this raw provider label." This is a thread-level aggregate: a row exists when at least one message in the thread carries the membership.

### `label_groups`, `label_group_members`, `pending_thread_label_intents`

User-visible label groups. `label_groups` stores the group name and colour. `label_group_members` maps raw provider labels into at most one group. `pending_thread_label_intents` stores local add/remove intent per `(account_id, thread_id, label_id)` while provider truth is pending; group rendering derives from that overlay plus `thread_labels`.

The smart-folder `is:tagged` operator and the `label:` operator both resolve to group membership, not raw `thread_labels` membership. A consequence: threads carrying labels that are not members of any group do not match `is:tagged`. Those labels remain in the per-account view in Settings, but are not represented in any group-based query. Users who want a label to participate in `is:tagged` add it to a group.

Synthesised `importance:high` / `importance:low` rows in `labels` always carry `is_undeletable = 1`, set by both the bootstrap synth path (account add) and the on-demand escape valve `ensure_prefixed_tag_label` (`crates/service/src/actions/label.rs`). The two are mutually exclusive at the provider level - the action service clears the opposite slot when one is set - and the Settings group-member picker rejects adding `importance:high` to a group that already contains `importance:low` (and vice versa).

### `message_keywords` table

Per-message keyword membership for IMAP and JMAP. Columns: `(account_id, message_id, keyword, label_id)` with PK `(account_id, message_id, label_id)`. The thread-level `kw:%` rows in `thread_labels` are derived from the union of `message_keywords` rows for messages in the thread - this is what makes keyword removal observable. Incoming IMAP and JMAP message changes replace the message's keyword rows, then recompute the thread aggregate from the union. Graph categories are not keyword-shaped and are reconciled through the provider category path instead.

### `threads` table - message-state columns

- `is_read`, `is_starred`, `is_snoozed`, `is_pinned`, `is_muted` - booleans driving sidebar filters and inline glyphs.

### `messages` table - message-state columns

- `is_replied`, `is_forwarded` - booleans driving thread-list glyphs. Thread-level renderings OR across messages.

---

## Terms NOT Used in Ratatoskr

These terms appear in provider documentation but are not Ratatoskr concepts. They never appear in user-facing copy and never appear in code identifiers except inside transport-layer or sync-layer code that's literally talking to a provider.

- **Tag** - not used. What providers call tags, categories, or keywords are *labels* in Ratatoskr.
- **Mailbox** - not used. What IMAP and JMAP call mailboxes are *folders* in Ratatoskr.
- **Category** - not used. What Exchange calls categories are *labels* in Ratatoskr.
- **Flag** (Outlook) - not used. The follow-up flag is mapped to the *starred* message state.
- **Verb** (Outlook) - not used. `PR_LAST_VERB_EXECUTED` is mapped to the *replied* and *forwarded* message states.
- **Importance** - not used as a top-level Ratatoskr concept. The `high` / `low` values become the synthesised labels `"High importance"` / `"Low importance"`; `normal` is no label.

Provider-native names still appear in code and docs where Ratatoskr is translating provider behaviour rather than defining user-facing concepts. For example:

- Exchange still has a `masterCategories` API even though Ratatoskr treats those objects as labels.
- Gmail still exposes provider-native `CATEGORY_*` bundle labels even though Ratatoskr treats inbox bundling as a separate bundles/classification system.

When those provider-native names appear, they are transport-layer or sync-layer terminology, not Ratatoskr concepts. They must not leak into core types, UI code, or user-facing strings.
