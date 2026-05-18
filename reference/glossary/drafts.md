# Drafts Glossary

A "draft" in Ratatoskr is one of two distinct things stored in two distinct tables. The sidebar's Drafts view shows both as a single chronological list. This document defines what each is, how they relate, and where the boundary between them lives in the code.

If you're touching anything that lists, counts, opens, sends, or syncs a draft, read this first.

## The two kinds

### Local draft

A composition the user has started but has not yet sent or synced to the provider. Stored in `local_drafts` (`crates/db/src/db/schema/04_compose.sql`).

Key fields: `id` (primary key, TEXT, app-generated UUID), `account_id`, `to_addresses` / `cc_addresses` / `bcc_addresses`, `subject`, `body_html`, `reply_to_message_id`, `thread_id`, `from_email`, `signature_id`, `signature_separator_index`, `remote_draft_id`, `attachments`, `created_at`, `updated_at`, `sync_status`.

`sync_status` is the state machine. The live transitions (`crates/db/src/db/queries_extra/draft_lifecycle.rs`) are `pending → sending → (row deleted on success)`, with `failed` reachable from any state:

- `pending` (default) - saved locally, no remote submission attempted. Set by `persist_draft_pending_sync` and `SAVE_LOCAL_DRAFT_SQL`.
- `sending` - currently being submitted to the provider. Set by `mark_draft_sending_sync` at the start of `send_email` (`crates/service/src/actions/send.rs`).
- `failed` - submission rejected. Set on send error (`mark_draft_failed_sync`), and on boot if a `'sending'` row is found stranded from a crashed send (`crates/db/src/db/queries_extra/account_sync_writes.rs`, also `pending_ops.rs`).

On a successful send the row is deleted via `delete_draft_sync` (`draft_lifecycle.rs`) - the sent message returns through provider sync as a regular thread in the Sent folder, so the local row has no further purpose. Same `delete_draft_sync` underlies the `delete_draft` action in `crates/service/src/actions/send.rs`.

Two additional values exist in the schema as forward-looking outbox/auto-save scaffolding but are not reached by current call sites:

- `queued` - reserved for a future explicit-queue send pass. Today no caller writes it; boot sweeps any `'queued'` rows to `'failed'` (`db_mark_queued_drafts_failed_sync`, run via `BootPhase::SweepingQueuedDrafts` in `crates/service/src/boot.rs`).
- `synced` - reserved for a future "draft persisted to the provider as a remote draft, not yet sent" flow. `db_mark_draft_synced` and `db_get_unsynced_drafts` (`crates/db/src/db/queries_extra/compose.rs`) are defined but currently uncalled; `get_local_draft_summaries` already filters `sync_status != 'synced'` in anticipation.

### Server-synced draft

A draft that exists on the provider as a message with the `DRAFT` system folder membership. Modelled as a normal `DbThread` row (in `threads`) with a `thread_folders` row pointing at the `DRAFT` folder, plus message rows in `messages`. Indistinguishable from any other thread except for the `DRAFT` folder membership.

Folder semantics for `DRAFT` are documented in `reference/glossary/folders-labels.md`: it's a container, not a tag. Per-provider mapping: Gmail `DRAFT` label, Graph `drafts` well-known folder, JMAP drafts-role mailbox, IMAP `\Drafts` special-use mailbox.

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

`Thread::from_local_draft` (`crates/app/src/db/types.rs`, paired with `Thread::from_db_thread`) assigns thread fields that don't have a natural value on a local draft:

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

## Lifecycle

Today there is no "save as remote draft" path - a local draft stays in `local_drafts` until the user sends it. The synced-as-remote-draft scaffolding (`db_get_unsynced_drafts`, `db_mark_draft_synced`, `db_delete_local_draft` in `crates/db/src/db/queries_extra/compose.rs`) is in place but uncalled, awaiting an outbox/auto-save flow.

**Save**: the compose flow writes via `SAVE_LOCAL_DRAFT_SQL` (`crates/db/src/db/queries_extra/compose.rs`) with `sync_status = 'pending'`. Subsequent edits upsert the same `id` and reset `sync_status` back to `'pending'`.

**Send** (`send_email` in `crates/service/src/actions/send.rs`):

1. `mark_draft_sending_sync` transitions the row to `'sending'` (rejecting if it's already sending).
2. The provider's submission API is called with the assembled MIME.
3. On success: `delete_local_draft` (`crates/service/src/send.rs`, wrapping `delete_draft_sync`) removes the row. The sent message returns through provider sync as a thread in the Sent folder; the local row would otherwise show up forever as a phantom entry in the Drafts pane.
4. On failure: `mark_draft_failed_sync` writes `sync_status = 'failed'`. Boot recovery resurrects any stranded `'sending'` rows as `'failed'` so a crashed send can't leave one stuck (`account_sync_writes.rs`, `pending_ops.rs`).

If the send was a reply/forward, a parallel `mark_send_intent` writeback runs against the source message (replied/forwarded flag), both provider-side and local. Failure of that side-effect is logged but does not fail the send.

Server-synced drafts (the other half of the Drafts list) arrive through the normal sync pipeline as threads with `DRAFT` folder membership. They have no relationship to the `local_drafts` table - they're regular threads that happen to live in the DRAFT folder.

`get_local_draft_summaries` filters `sync_status != 'synced'`, which is currently a no-op because nothing writes `'synced'`. The filter is in place for the future outbox flow.

## Count semantics

Every universal-folder pill in the sidebar - Drafts included - counts the `is_read = 0` subset of the folder's synced thread membership. There is one uniform rule across the sidebar: pills mean "unread within folder membership," with no per-folder predicate variants and no per-folder pill styling.

For Drafts this typically means an empty pill: a draft you authored is read by you, so `is_read` is usually 1. That is the accepted cost of a single legible rule, and applies equally to Sent, Trash, Spam, and Archive. `get_draft_count_with_local` still exists for callers that genuinely want a total (pane headers, compose-pane indicators) but is not routed to the sidebar pill.

Local drafts have no `is_read` column and are not in the read/unread state space; they are pre-sync compositions, not threads. They appear in the Drafts *list* (via `get_drafts_view`) because the list answers "what compositions are pending my attention," but they do not contribute to the pill. The disjoint `UniversalUnreadCount` and `DraftTotalCount` wrapper types (`crates/db/src/db/types.rs`) make this a compile-time invariant: a future contributor cannot route synced+local totals back through the pill without a type error.

## What NOT to do

- Don't expose or call synced-only helpers when you want "the Drafts folder." Use `get_drafts_view`; `get_draft_threads_synced` and `get_local_draft_summaries` are intentionally crate-private halves.
- Don't render a local draft through `Thread::from_db_thread`. There's no `DbThread` to feed it - local drafts go through `Thread::from_local_draft`.
- Don't treat `is_local_draft` as derivable from other fields. It's the only correct discriminator at the app boundary; nothing else (`message_count == 1`, empty labels, etc.) is reliable.
- Don't expect a local draft's `id` to be a thread ID. The `id` is the `local_drafts.id` (a UUID); the thread doesn't exist yet. Routing code that assumes "click row -> open thread" has to check `is_local_draft` first.
