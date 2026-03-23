# Action Service: Phase 2.1 Detailed Plan

## Goal

Migrate all thread-level actions with uniform provider support into the action service. After this phase: `dispatch_email_db_action`, `toggle_star_selected_threads`, and `toggle_bool_selected_threads` are deleted. Every thread action goes through `core::actions`. The app handler contains only service calls + UI state management.

## Provider Audit Confirmation

All actions in scope have real (non-stub) implementations across all four providers. Verified by code audit:

- **`trash()`** — Gmail: modify_thread(add TRASH, remove INBOX). Graph: move_messages to trash folder. JMAP: mailbox update (add trash, remove inbox). IMAP: move_messages to Trash folder.
- **`permanent_delete()`** — Gmail: delete_thread. Graph: batch delete_messages. JMAP: EmailSet.destroy. IMAP: delete_messages (set delete flag).
- **`mark_read(thread_id, read: bool)`** — Gmail: modify_thread (UNREAD label). Graph: patch_messages(is_read). JMAP: keyword("$seen"). IMAP: set_flags(\\Seen).
- **`star(thread_id, starred: bool)`** — Gmail: modify_thread (STARRED label). Graph: patch_messages(flag_status). JMAP: keyword("$flagged"). IMAP: set_flags(\\Flagged).
- **`spam(thread_id, is_spam: bool)`** — Gmail: modify_thread (add/remove SPAM + remove/add INBOX). Graph: move_messages to junk or inbox. JMAP: mailbox update (junk/inbox swap). IMAP: move_messages to Junk or Inbox.
- **`move_to_folder(thread_id, folder_id)`** — Gmail: modify_thread(add folder_id). Graph: move_messages to folder. JMAP: mailbox update(set target). IMAP: move_messages to destination.

All six accept the parameters the service will pass. No provider-specific gaps or interface mismatches. `spam()` takes `is_spam: bool` and handles both directions. `move_to_folder()` takes a provider-specific folder ID.

## Actions in Scope

Eight actions, grouped by pattern:

**Removes-from-view actions** (local DB + provider dispatch + auto-advance on success):
- **trash** — local: remove INBOX + add TRASH. Provider: `trash()`.
- **spam** — local: if marking spam, remove INBOX + add SPAM; if un-spamming, remove SPAM + add INBOX. Provider: `spam(thread_id, is_spam)`. Note: `ToggleSpam` is bidirectional — the app resolves the current spam state from the thread/navigation context and passes `is_spam: bool` explicitly to the service, same as other directional actions. No TOCTOU — the service takes a target state, not "toggle."
- **move_to_folder** — local: remove source label + add target label. Provider: `move_to_folder(thread_id, folder_id)`. Note: the source is not always INBOX — the user may move from Trash, Spam, or another folder. The action function takes `source_label_id: Option<&str>` (None means "don't remove any source label"). The app resolves the current navigation context to determine the source.
- **permanent_delete** — local: `delete_thread()`. Provider: `permanent_delete()`.

**Toggle actions** (optimistic UI + local DB + provider dispatch):
- **star** — local: `set_thread_starred()`. Provider: `star(thread_id, starred)`.
- **mark_read** — local: `set_thread_read()`. Provider: `mark_read(thread_id, read)`. Completion triggers nav state refresh (unread counts).

**Local-only by design** (no provider dispatch):
- **pin** — local: `set_thread_pinned()`. No provider equivalent.
- **mute** — local: `set_thread_muted()`. No provider equivalent.

**Out of scope:** snooze, label apply/remove, unsubscribe.

## Design Decisions

### Action function signatures

Each action function in core takes the parameters it needs beyond `ActionContext`, `account_id`, and `thread_id`:

```rust
pub async fn trash(ctx, account_id, thread_id) -> ActionOutcome
pub async fn spam(ctx, account_id, thread_id, is_spam: bool) -> ActionOutcome
pub async fn move_to_folder(ctx, account_id, thread_id, folder_id: &str, source_label_id: Option<&str>) -> ActionOutcome
pub async fn permanent_delete(ctx, account_id, thread_id) -> ActionOutcome
pub async fn star(ctx, account_id, thread_id, starred: bool) -> ActionOutcome
pub async fn mark_read(ctx, account_id, thread_id, read: bool) -> ActionOutcome
pub async fn pin(ctx, account_id, thread_id, pinned: bool) -> ActionOutcome
pub async fn mute(ctx, account_id, thread_id, muted: bool) -> ActionOutcome
```

Toggle actions take the *target* value (not "toggle" — the app resolves the current value and passes the new one). This avoids TOCTOU issues where the service would need to read current state.

### Completion message

```rust
enum CompletedAction {
    Archive,
    Trash,
    Spam,
    MoveToFolder,
    PermanentDelete,
    Star,
    MarkRead,
    Pin,
    Mute,
}

Message::ActionCompleted {
    action: CompletedAction,
    removes_from_view: bool,
    outcomes: Vec<ActionOutcome>,
    /// Previous values for rollback of optimistic toggle actions.
    /// Keyed by (account_id, thread_id) → previous bool value.
    /// Empty for non-toggle actions.
    rollback: Vec<(String, String, bool)>,
}
```

`CompletedAction` is an enum, not a string. Display text is derived from the enum in the handler — logic never matches on strings. The `rollback` field carries the thread identities and previous values captured before the optimistic toggle, so the handler can restore state on failure regardless of what the user has selected by the time the async result arrives.

### Toggle rollback specification

**Before the service call:**
1. Capture `Vec<(account_id, thread_id, previous_value)>` from the selected threads.
2. Apply optimistic toggle to UI: flip the bool on each thread list item. For star, also call `reading_pane.update_star()`.
3. Dispatch the service call. The captured rollback data is included in the `ActionCompleted` message.

**On completion:**
- `Success` or `LocalOnly`: do nothing (optimistic state is correct).
- `Failed` (all threads failed locally): restore previous values. Find each thread by `(account_id, thread_id)` — not by index, since selection may have changed. For star, also restore reading pane state.
- Mixed outcomes (some succeeded, some failed): restore only the failed threads. Each `ActionOutcome` in the vec corresponds positionally to the thread in the dispatch list.

**Thread identity, not index.** Rollback scans `self.thread_list.threads` for matching `(account_id, id)` pairs and restores the previous value. If a thread is no longer in the list (user navigated away), the rollback is silently skipped — the list will reload with correct state from DB anyway.

### Unread count refresh

`mark_read` completion triggers a navigation reload (`load_navigation`) to refresh sidebar unread counts. This is the same mechanism the existing thread selection path uses — no new infrastructure needed. Other actions that remove from view already trigger nav reload via the thread list change.

### Local-only actions and ActionOutcome

Pin and mute return `ActionOutcome::Success` after the local DB write. There is no `LocalOnly` or `Failed` from a remote step because there is no remote step. This means an observer seeing `Success` for pin/mute cannot distinguish "local-only by design" from "fully synced." This is an intentional limitation of Phase 2.1 — `ActionOutcome` does not carry a `local_only_by_design` flag. If this distinction becomes needed (for observability or Phase 3 failure policy), it can be added to the outcome type then.

### Batch behavior for removes-from-view

The current code already operates over multiple selected threads. Rule for mixed outcomes:

- **All succeeded (Success or LocalOnly):** auto-advance.
- **All failed:** don't advance. Show error.
- **Mixed (some succeeded, some failed):** auto-advance (the successfully-actioned threads are gone from the view). Show warning with count: "⚠ Trashed 4 of 5 threads — 1 failed."

This is the pragmatic choice — the thread list has already changed (some threads removed), so not advancing would leave the user looking at a stale position.

## Implementation Steps

### Step 1: Create action functions in core

Add eight files to `crates/core/src/actions/`. Each follows the archive pattern:

- `trash.rs` — local: `remove_label(INBOX)` + `insert_label(TRASH)`. Provider: `provider.trash()`.
- `spam.rs` — local: if `is_spam` then `remove_label(INBOX)` + `insert_label(SPAM)`, else `remove_label(SPAM)` + `insert_label(INBOX)`. Provider: `provider.spam(thread_id, is_spam)`.
- `move_to_folder.rs` — local: if `source_label_id` is Some, `remove_label(source)` + `insert_label(folder_id)`; if None, just `insert_label(folder_id)`. Provider: `provider.move_to_folder(thread_id, folder_id)`.
- `permanent_delete.rs` — local: `delete_thread()`. Provider: `provider.permanent_delete()`.
- `star.rs` — local: `set_thread_starred(starred)`. Provider: `provider.star(thread_id, starred)`.
- `mark_read.rs` — local: `set_thread_read(read)`. Provider: `provider.mark_read(thread_id, read)`.
- `pin.rs` — local: `set_thread_pinned(pinned)`. No provider call. Return `Success`.
- `mute.rs` — local: `set_thread_muted(muted)`. No provider call. Return `Success`.

Re-export all from `actions/mod.rs`.

### Step 2: Define `CompletedAction` and replace `ArchiveCompleted`

In the app crate, define `CompletedAction` enum and replace `Message::ArchiveCompleted` with `Message::ActionCompleted`. Write a single `handle_action_completed` that generalizes the current `handle_archive_completed`:

- Removes-from-view actions: map outcomes to toast, auto-advance if any succeeded. Mixed outcomes show warning with count.
- Toggle actions: on all-succeeded, do nothing. On all-failed, restore all rollback values by thread ID. On mixed outcomes, restore only the failed threads by thread ID.
- `mark_read`: additionally trigger nav reload for unread counts.

### Step 3: Build `dispatch_action` — generic dispatcher

Replace `dispatch_archive` with a generic `dispatch_action` that:

1. Checks `action_ctx` availability.
2. Clones context and thread list.
3. Spawns `Task::perform` that calls the appropriate `core::actions::*` function per thread.
4. Returns `Message::ActionCompleted` with the action kind, outcomes, and rollback data (populated for toggles, empty for removes-from-view).

This is one function with a `match` on the action kind to select the core function. The match also determines `removes_from_view` and whether to populate rollback.

### Step 4: Migrate `handle_email_action`

Restructure the match arms:

```rust
// Removes-from-view actions — service dispatch, deferred auto-advance
EmailAction::Archive | EmailAction::Trash | EmailAction::PermanentDelete
| EmailAction::ToggleSpam | EmailAction::MoveToFolder { .. } => {
    let threads = collect_selected_threads();
    self.dispatch_action(action, threads)
}

// Toggle actions — optimistic UI, then service dispatch
EmailAction::ToggleStar => {
    let rollback = self.optimistic_toggle_star();
    self.dispatch_toggle(CompletedAction::Star, rollback)
}
EmailAction::ToggleRead => {
    let rollback = self.optimistic_toggle_read();
    self.dispatch_toggle(CompletedAction::MarkRead, rollback)
}
EmailAction::TogglePin => {
    let rollback = self.optimistic_toggle_pin();
    self.dispatch_toggle(CompletedAction::Pin, rollback)
}
EmailAction::ToggleMute => {
    let rollback = self.optimistic_toggle_mute();
    self.dispatch_toggle(CompletedAction::Mute, rollback)
}

// Not yet migrated
EmailAction::AddLabel { .. } | EmailAction::RemoveLabel { .. } => { /* Phase 2.2 */ }
EmailAction::Snooze { .. } => { /* deferred */ }
EmailAction::Unsubscribe => { /* separate concern */ }
```

Each `optimistic_toggle_*` method captures previous values, applies the UI flip, and returns the rollback data. `dispatch_toggle` sends the service call with rollback attached.

### Step 5: Delete legacy code

- Delete `dispatch_email_db_action`.
- Delete `toggle_star_selected_threads`.
- Delete `toggle_bool_selected_threads`.
- Delete `dispatch_archive` and `handle_archive_completed` (subsumed by generic versions).

### Step 6: Verify

Compilation and clippy:
- `cargo check --workspace` and `cargo clippy -p app -p ratatoskr-core`.
- Verify deleted functions no longer exist.
- Verify no direct `set_thread_*`, `remove_label`, `insert_label`, or `delete_thread` calls remain in app for migrated actions.

Provider dispatch (manual smoke test or log verification):
- Trash a thread → `ProviderOps::trash()` called.
- Star a thread → `ProviderOps::star()` called.
- Mark read → `ProviderOps::mark_read()` called.
- These have never been exercised from the app.

UI correctness:
- Toggle star, then simulate provider failure → star state restores in thread list and reading pane.
- Toggle read → sidebar unread counts update.
- Trash with mixed-account selection → auto-advance fires, partial failure shows warning with count.
- All-failed removes-from-view → no auto-advance, error shown.

## Exit Criteria

1. Every thread-level email action (except snooze, labels, unsubscribe) goes through `core::actions::*`.
2. `dispatch_email_db_action`, `toggle_star_selected_threads`, and `toggle_bool_selected_threads` are deleted.
3. The app crate does not call `remove_label`, `insert_label`, `set_thread_*`, or `delete_thread` directly for any migrated action.
4. Removes-from-view actions auto-advance conditionally (success/local-only: advance; all-failed: don't). Mixed outcomes advance and show warning with count.
5. Toggle actions do optimistic UI update with captured rollback data. On all-failed, previous values are restored by thread ID (not index).
6. Star rollback restores both thread list and reading pane state.
7. `mark_read` completion triggers nav state refresh for unread counts.
8. Pin and mute go through the service with no provider dispatch. `ActionOutcome::Success` returned. The lack of a `local_only_by_design` distinction is an intentional limitation documented in this plan.
9. Provider dispatch methods that were never previously called (`trash`, `spam`, `mark_read`, `move_to_folder`, `permanent_delete`, `star`) are now exercised. Verified via manual smoke test or logs.
10. Failed toggle rollback works when the user has changed selection between dispatch and completion (thread found by ID, not index).
11. Workspace compiles and passes clippy.
