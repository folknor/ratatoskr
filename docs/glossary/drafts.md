# Drafts Glossary

A "draft" in Ratatoskr is one of two distinct things stored in two distinct tables. The sidebar's Drafts view shows both as a single chronological list. This document defines what each is, how they relate, and where the boundary between them lives in the code.

If you're touching anything that lists, counts, opens, sends, or syncs a draft, read this first.

## The two kinds

### Local draft

A composition the user has started but has not yet sent or synced to the provider. Stored in `local_drafts` (`crates/db/src/db/schema/04_compose.sql`).

Key fields: `id` (primary key, TEXT, app-generated UUID), `account_id`, `to_addresses` / `cc_addresses` / `bcc_addresses`, `subject`, `body_html`, `reply_to_message_id`, `thread_id`, `from_email`, `remote_draft_id`, `attachments`, `created_at`, `updated_at`, `sync_status`.

`sync_status` is the state machine:

- `pending` (default) - saved locally, no remote write attempted.
- `queued` - flagged for the next send pass.
- `sending` - currently being submitted to the provider.
- `failed` - submission attempted and rejected. Reset to `failed` on app restart if `sync_status = 'sending'` is found at boot (`crates/db/src/db/queries_extra/account_sync_writes.rs`), so a crashed send never leaves a row stuck in `'sending'`.
- `synced` - successfully submitted; `remote_draft_id` populated.

A local draft is **deleted** from `local_drafts` once it has been sent (not just synced) - see the `DELETE FROM local_drafts WHERE id = ?1` path in `crates/db/src/db/queries_extra/compose.rs`. Synced-but-not-sent drafts stay in `local_drafts` with `sync_status = 'synced'` until either the user sends them or they get cleaned up.

### Server-synced draft

A draft that exists on the provider as a message with the `DRAFT` system folder membership. Modelled as a normal `DbThread` row (in `threads`) with a `thread_folders` row pointing at the `DRAFT` folder, plus message rows in `messages`. Indistinguishable from any other thread except for the `DRAFT` folder membership.

Folder semantics for `DRAFT` are documented in `docs/glossary/folders-labels.md`: it's a container, not a tag. Per-provider mapping: Gmail `DRAFT` label, Graph `drafts` well-known folder, JMAP drafts-role mailbox, IMAP `\Drafts` special-use mailbox.

### Why the split

Local drafts are not threads. A draft you've started typing into has no message-id, no provider-side identity, no thread to belong to until either (a) it's a reply, in which case it inherits `reply_to_message_id` / `thread_id`, or (b) it's a brand-new compose, in which case the provider will eventually mint identifiers when the draft is synced. Forcing every keystroke into the `threads` / `messages` tables would mean inventing provisional IDs, plumbing them through sync, and reconciling on conflict. The dedicated `local_drafts` table sidesteps that.

## The mixed Drafts list

The sidebar's Drafts view must show local drafts and server-synced drafts together, ordered chronologically. There is one user-visible "Drafts" destination, not two.

**Total count** for callers that want one (pane headers, compose-pane indicators) is unified at the DB query layer: `get_draft_count_with_local` (`crates/db/src/db/queries_extra/scoped_queries.rs`) sums server-synced `DRAFT` threads and local unsynced drafts. Single number, no app-side count merge. The sidebar *pill* does not use this - see "Count semantics" below.

**List membership** is unified at the DB query layer: `get_drafts_view` (`crates/db/src/db/queries_extra/scoped_queries.rs`) is the only public Drafts-list query. It returns a sealed `DraftsView` containing both server-synced `DbThread` rows and unsynced `LocalDraftSummary` rows. The synced-only helpers (`get_draft_threads_synced` and `get_local_draft_summaries`) are crate-private so external callers cannot silently ask for only half of Drafts.

The app still performs the presentation merge in `crates/app/src/helpers.rs`:

1. Call `get_drafts_view`.
2. Split the `DraftsView` with `into_parts`.
3. Convert synced rows with `Thread::from_db_thread`.
4. Convert local rows with `Thread::from_local_draft`.
5. Concatenate, sort by `last_message_at desc`, then apply thread decorations.

`Thread::from_local_draft` assigns thread fields that don't have a natural value on a local draft:

- `is_read: true` - a draft you wrote yourself isn't "unread."
- `is_starred: false`, `is_replied: false`, `is_forwarded: false`, `is_pinned: false`, `is_muted: false`, `has_attachments: false` - no provider interactions yet.
- `label_paints: Vec::new()` - not labelled.
- `message_count: 1` - a draft is a single composition; the card's multi-message indicator is suppressed.
- `is_local_draft: true` - the discriminator.

These defaults are the local-draft constructor contract. Any new field on `Thread` that needs a meaningful local-draft value must be handled in `Thread::from_local_draft`, not patched at a call site.

## Click semantics

Clicking a row in the Drafts list diverges on `is_local_draft`:

- **Local draft** (`crates/app/src/handlers/core.rs`) - intercepted before reading-pane routing. Loads the full `DbLocalDraft` via `db_get_local_draft` and opens a compose pop-out for editing.
- **Server-synced draft** - routes to the reading pane like any other thread.

The visual cue is a "Draft" pill rendered in the thread card (`crates/app/src/ui/widgets/cards.rs`), shown only when `is_local_draft = true`. Synced drafts get no pill - they look like ordinary threads with the DRAFT folder membership reflected in the sidebar selection state.

## Sync lifecycle

A local draft becomes a server-synced draft when the user explicitly saves it remotely (currently driven by the compose flow). The path is:

1. App writes the local draft via `SAVE_LOCAL_DRAFT_SQL` (`crates/db/src/db/queries_extra/compose.rs`) with `sync_status = 'pending'`.
2. A sync pass picks it up: `SELECT * FROM local_drafts WHERE account_id = ?1 AND sync_status = 'pending'`.
3. Provider call creates the remote draft (provider-specific - JMAP `EmailSet`, Graph `messages` POST, Gmail `drafts.create`, IMAP APPEND with `\Draft`).
4. On success: `UPDATE local_drafts SET sync_status = 'synced', remote_draft_id = ? WHERE id = ?` (`compose.rs`).
5. The next sync pass ingests the remote draft as a thread with `DRAFT` membership. At this point the same composition can exist in **both** tables: as a `local_drafts` row with `sync_status = 'synced'` and as a `DbThread` carrying `DRAFT`.

The visible Drafts list does not show the synced local row. `get_local_draft_summaries` filters to `sync_status != 'synced'`, so after the remote draft is ingested the list shows the server-synced thread side. The local row remains as send/edit bookkeeping until send or cleanup removes it.

**Send** is a different path: when the user clicks Send, the provider's submission API is called and the local row is deleted (`DELETE FROM local_drafts WHERE id = ?1`, `compose.rs`). The corresponding DRAFT-labelled thread, if any, is removed by the provider's own state machine and picked up on the next sync.

## Count semantics

Every universal-folder pill in the sidebar - Drafts included - counts the `is_read = 0` subset of the folder's synced thread membership. There is one uniform rule across the sidebar: pills mean "unread within folder membership," with no per-folder predicate variants and no per-folder pill styling.

For Drafts this typically means an empty pill: a draft you authored is read by you, so `is_read` is usually 1. That is the accepted cost of a single legible rule, and applies equally to Sent, Trash, Spam, and Archive. `get_draft_count_with_local` still exists for callers that genuinely want a total (pane headers, compose-pane indicators) but is not routed to the sidebar pill.

Local drafts have no `is_read` column and are not in the read/unread state space; they are pre-sync compositions, not threads. They appear in the Drafts *list* (via `get_drafts_view`) because the list answers "what compositions are pending my attention," but they do not contribute to the pill. Background and rationale: `docs/glossary/discrepancies.md` § "Drafts Pill Semantics."

## What NOT to do

- Don't expose or call synced-only helpers when you want "the Drafts folder." Use `get_drafts_view`; `get_draft_threads_synced` and `get_local_draft_summaries` are intentionally crate-private halves.
- Don't render a local draft through `Thread::from_db_thread`. There's no `DbThread` to feed it - local drafts go through `Thread::from_local_draft`.
- Don't treat `is_local_draft` as derivable from other fields. It's the only correct discriminator at the app boundary; nothing else (`message_count == 1`, empty labels, etc.) is reliable.
- Don't expect a local draft's `id` to be a thread ID. The `id` is the `local_drafts.id` (a UUID); the thread doesn't exist yet. Routing code that assumes "click row -> open thread" has to check `is_local_draft` first.
