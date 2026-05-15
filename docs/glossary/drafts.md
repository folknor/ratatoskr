# Drafts Glossary

A "draft" in Ratatoskr is one of two distinct things stored in two distinct tables. The sidebar's Drafts view shows both as a single chronological list. This document defines what each is, how they relate, and where the boundary between them lives in the code.

If you're touching anything that lists, counts, opens, sends, or syncs a draft, read this first.

## The two kinds

### Local draft

A composition the user has started but has not yet sent or synced to the provider. Stored in `local_drafts` (`crates/db/src/db/schema/04_compose.sql:51`).

Key fields: `id` (primary key, TEXT, app-generated UUID), `account_id`, `to_addresses` / `cc_addresses` / `bcc_addresses`, `subject`, `body_html`, `reply_to_message_id`, `thread_id`, `from_email`, `remote_draft_id`, `attachments`, `created_at`, `updated_at`, `sync_status`.

`sync_status` is the state machine:

- `pending` (default) - saved locally, no remote write attempted.
- `queued` - flagged for the next send pass.
- `sending` - currently being submitted to the provider.
- `failed` - submission attempted and rejected. Reset to `failed` on app restart if `sync_status = 'sending'` is found at boot (`crates/db/src/db/queries_extra/account_sync_writes.rs:12-17`), so a crashed send never leaves a row stuck in `'sending'`.
- `synced` - successfully submitted; `remote_draft_id` populated.

A local draft is **deleted** from `local_drafts` once it has been sent (not just synced) - see the `DELETE FROM local_drafts WHERE id = ?1` path in `crates/db/src/db/queries_extra/compose.rs:663`. Synced-but-not-sent drafts stay in `local_drafts` with `sync_status = 'synced'` until either the user sends them or they get cleaned up.

### Server-synced draft

A draft that exists on the provider as a message with the `DRAFT` system folder membership. Modelled as a normal `DbThread` row (in `threads`) with a `thread_labels` row pointing at the `DRAFT` label, plus message rows in `messages`. Indistinguishable from any other thread except for the `DRAFT` label membership.

Folder semantics for `DRAFT` are documented in `docs/glossary/folders-labels.md`: it's a container, not a tag. Per-provider mapping: Gmail `DRAFT` label, Graph `drafts` well-known folder, JMAP drafts-role mailbox, IMAP `\Drafts` special-use mailbox.

### Why the split

Local drafts are not threads. A draft you've started typing into has no message-id, no provider-side identity, no thread to belong to until either (a) it's a reply, in which case it inherits `reply_to_message_id` / `thread_id`, or (b) it's a brand-new compose, in which case the provider will eventually mint identifiers when the draft is synced. Forcing every keystroke into the `threads` / `messages` tables would mean inventing provisional IDs, plumbing them through sync, and reconciling on conflict. The dedicated `local_drafts` table sidesteps that.

## The mixed Drafts list

The sidebar's Drafts view must show local drafts and server-synced drafts together, ordered chronologically. There is one user-visible "Drafts" destination, not two.

**Count** is unified at the core query layer: `get_draft_count_with_local` (`crates/db/src/db/queries_extra/scoped_queries.rs:590`) sums `get_thread_count_scoped(label="DRAFT")` and `count_local_drafts`. Single number, no app-side merge.

**List** is unified at the app layer in `crates/app/src/helpers.rs:167-175`:

1. Call `get_threads_scoped(label="DRAFT")` for the synced subset.
2. Call `get_local_draft_summaries` for the local subset.
3. Coerce each `LocalDraftSummary` into the app's `Thread` shape via `local_draft_to_app_thread` (`crates/app/src/helpers.rs:361`).
4. Concatenate, then sort by `last_message_at desc`.

`local_draft_to_app_thread` hardcodes thread fields that don't have a natural value on a local draft:

- `is_read: true` - a draft you wrote yourself isn't "unread."
- `is_starred: false`, `is_replied: false`, `is_forwarded: false`, `is_pinned: false`, `is_muted: false`, `has_attachments: false` - no provider interactions yet.
- `label_color_bgs: Vec::new()` - not labelled.
- `message_count: 1` - a draft is a single composition; the card's multi-message indicator is suppressed.
- `is_local_draft: true` - the discriminator.

These are sensible defaults for a not-yet-synced composition, but they're a second source of truth for "what a `Thread` looks like" parallel to the canonical `db_thread_to_app_thread`. Any new field on `Thread` that needs a meaningful local-draft value has to be updated in both places by hand. See `docs/glossary/discrepancies.md` § "Mixed drafts list merged at the app layer" for the broader pattern.

## Click semantics

Clicking a row in the Drafts list diverges on `is_local_draft`:

- **Local draft** (`crates/app/src/handlers/core.rs:891-903`) - intercepted before reading-pane routing. Loads the full `DbLocalDraft` via `db_get_local_draft` and opens a compose pop-out for editing.
- **Server-synced draft** - routes to the reading pane like any other thread.

The visual cue is a "Draft" pill rendered in the thread card (`crates/app/src/ui/widgets/cards.rs:80`), shown only when `is_local_draft = true`. Synced drafts get no pill - they look like ordinary threads with the DRAFT folder membership reflected in the sidebar selection state.

## Sync lifecycle

A local draft becomes a server-synced draft when the user explicitly saves it remotely (currently driven by the compose flow). The path is:

1. App writes the local draft via `SAVE_LOCAL_DRAFT_SQL` (`crates/db/src/db/queries_extra/compose.rs:562`) with `sync_status = 'pending'`.
2. A sync pass picks it up: `SELECT * FROM local_drafts WHERE account_id = ?1 AND sync_status = 'pending'`.
3. Provider call creates the remote draft (provider-specific - JMAP `EmailSet`, Graph `messages` POST, Gmail `drafts.create`, IMAP APPEND with `\Draft`).
4. On success: `UPDATE local_drafts SET sync_status = 'synced', remote_draft_id = ? WHERE id = ?` (`compose.rs:652`).
5. The next sync pass ingests the remote draft as a thread with `DRAFT` membership. At this point the same composition appears in **both** tables: as a `local_drafts` row with `sync_status = 'synced'` *and* as a `DbThread` carrying `DRAFT`.

The double-appearance is by design - the local row is the editable source of truth, and the synced thread is what shows on other devices. The current mixed-list code does not de-duplicate by `remote_draft_id`; if the same composition exists in both forms, the list will show two rows. That's an open issue, not handled today.

**Send** is a different path: when the user clicks Send, the provider's submission API is called and the local row is deleted (`DELETE FROM local_drafts WHERE id = ?1`, `compose.rs:663`). The corresponding DRAFT-labelled thread, if any, is removed by the provider's own state machine and picked up on the next sync.

## Count semantics

The Drafts pill shows the **total** drafts (synced + local), not unread. This is the only universal-folder pill in the sidebar that shows total rather than unread. The inconsistency is intentional in a sense - "unread drafts" is a weak concept, since you wrote them - but the contract that pills mean "unread" everywhere except Drafts is unsignalled in the UI. Tracked in `TODO.md` as "Sidebar pill semantics: Drafts (and other non-unread folders)."

## What NOT to do

- Don't call `get_draft_threads` directly when you want "the Drafts folder." You'll get the synced subset only and silently disagree with `get_draft_count_with_local` and with the sidebar list. Use the app-layer merge in `helpers.rs`, or fold the local-draft fetch into your call site explicitly. The function's doc comment now flags this.
- Don't render a local draft through the canonical `db_thread_to_app_thread`. There's no `DbThread` to feed it - local drafts go through `local_draft_to_app_thread`.
- Don't treat `is_local_draft` as derivable from other fields. It's the only correct discriminator at the app boundary; nothing else (`message_count == 1`, empty labels, etc.) is reliable.
- Don't expect a local draft's `id` to be a thread ID. The `id` is the `local_drafts.id` (a UUID); the thread doesn't exist yet. Routing code that assumes "click row -> open thread" has to check `is_local_draft` first.
