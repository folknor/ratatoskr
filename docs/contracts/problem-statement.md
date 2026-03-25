# Codebase Contracts: Problem Statement

## The Problem

Ratatoskr has implicit contracts — behaviors where correctness depends on every developer remembering to follow a multi-step protocol that nothing in the compiler, type system, or API surface enforces. A new developer adding a feature can silently break an invariant because the protocol exists only in convention.

The action service (Phases 1–6) proved this is solvable: provider dispatch is now a compile-time boundary. The app crate physically cannot bypass the action service. This document catalogues every remaining implicit contract in the codebase, prioritized by risk.

## What Makes a Contract Implicit

1. Multiple call sites must follow the same protocol, but nothing enforces it
2. Adding a new call site can silently break existing behavior
3. The pattern relies on "every developer knows to..." rather than "the API makes it impossible not to..."

The structural fix for each is the same principle: make the right thing the only thing. A single entry point, a type that enforces the invariant, or a compiler error when the protocol is violated.

---

## Critical — Fix Before Adding Features

### 1. Mail mutations must go through the action service

**Contract:** Every email state mutation must flow through `core::actions::*` for local DB + provider dispatch + pending-ops + undo + in-flight guard.

**Currently enforced by:** Documentation. The Phase 6 compilation boundary prevents the app from importing provider crates, but it does not prevent direct DB label writes.

**Known violations fixed:** Pop-out archive and delete now route through `dispatch_action_service` / `dispatch_action_service_with_params` — same path as the main reading pane. Provider dispatch, pending-ops, undo, and in-flight guard all apply.

**Structural fix (remaining):** Hide raw mutating mail DB helpers (`insert_label`, `remove_label`, `remove_inbox_label`, `delete_thread`, `set_thread_starred`, `set_thread_read`, `set_thread_pinned`, `set_thread_muted`) from the app crate. Make them `pub(crate)` in `ratatoskr-core` or move them into the actions module. The app can only call `actions::archive()`, `actions::trash()`, etc. Same principle as Phase 6's provider boundary — compile-time enforcement.

### ~~2. Account deletion leaks external stores~~ ✅ Fixed

`delete_account_orchestrate()` gathers cleanup data + shared-ref checks + deletes the account row in one synchronous connection call. The app handler then does best-effort async cleanup of body store, inline image store (with separate `inline_hashes_referenced_by_other_accounts` check), attachment cache files, and search index. 6 integration tests cover the orchestration contract. Pending ops are CASCADE-deleted via FK.

### ~~3. Compose window close loses dirty drafts~~ ✅ Fixed

`save_compose_draft_sync()` does a synchronous single-row INSERT before the window is removed from the map. Called in both the pop-out close path and the main window exit path (before `iced::exit()`). Synchronous write avoids the async-vs-exit race. Skips save when `from_account` is None (unattributable draft).

### ~~4. New CommandId variants silently ignored~~ ✅ Fixed

`dispatch_other` inlined into `dispatch_command` — all 69 variants have explicit match arms, no wildcard. Adding a new `CommandId` variant without a dispatch arm is now a compiler error. Also fixed 3 variants (`CalendarPopOut`, `SwitchToCalendar`, `SwitchToMail`) missing from `ALL_IDS`/`TABLE`.

---

## High — Fix to Prevent New-Developer Mistakes

### 5. Navigation state reset protocol

**Contract:** Every "show this view" transition (account switch, label select, search clear, pinned search select, navigate-to) must: clear search state, clear pinned search, set navigation_target, clear selected_thread + reading pane, bump nav_generation + thread_generation, and reload threads.

**Currently enforced by:** Copy-paste. At least 5 call sites in `main.rs` and `handlers/navigation.rs` each do a different subset of these 7 steps.

**Violation scenario:** `SharedMailboxSelected` and `PublicFolderSelected` are currently stubs returning `Task::none()`. When implemented, the developer must replicate the exact protocol with no guide.

**Structural fix:** `fn switch_view(&mut self, target: ViewTarget) -> Task<Message>` that encapsulates the entire reset/reload sequence. All navigation paths call this.

### ~~6. Settings open/close protocol~~ ✅ Fixed

`open_settings(tab)` handles the full open protocol (show, overlay reset, animation, tab, begin_editing). `close_settings()` commits preferences and hides. All 6 open/close sites now use these helpers. Fixed the existing violation where `open_contact_editor_for_email` skipped overlay reset and `begin_editing()`.

### ~~7. Thread selection + reading pane consistency~~ ✅ Fixed

`fn clear_thread_selection(&mut self)` clears `selected_thread`, multi-select, and reading pane in one call. All 10 deselection sites in App-level code now use this helper. No more stale reading pane content after navigation, search, or account switch.

### 8. Compose routing deduplication

**Contract:** Opening a compose window should check for an existing window for the same draft/thread and focus it instead of creating a duplicate.

**Currently enforced by:** Nothing. All compose entry points (sidebar button, Reply, Forward, command palette, keyboard shortcut) call raw window-open helpers without dedup checks.

**Structural fix:** `fn compose_target(&self, draft_id: Option<&str>, thread_id: Option<&str>) -> ComposeTarget { New | Existing(window_id) }`.

### 9. New email actions require 8 parallel edits

**Contract:** Adding a new email action requires: `EmailAction` variant, `CompletedAction` variant (with `removes_from_view`, `success_label`), `BatchAction` variant, `to_batch_action` mapping, `handle_action_completed` arm, `UndoToken` variant, undo dispatch arm, `handle_email_action` arm. Missing any one silently degrades (no undo, wrong toast, etc.).

**Currently enforced by:** Convention. Wildcard arms in `to_batch_action` and undo dispatch return `None`/empty instead of failing, so missing arms are silent.

**Structural fix:** A single action descriptor table (or derive macro) that generates the classification, batch mapping, rollback policy, and undo mapping from one definition.

### 10. Scope state is split and partially ignored

**Contract:** The active scope (which account/shared mailbox/public folder) must be consistent across the sidebar, navigation context, and all DB queries.

**Currently enforced by:** Ad-hoc fields: `selected_account`, `selected_shared_mailbox`, `navigation_target`, `selected_label`. `current_scope()` only reads `selected_account` — shared mailbox and public folder selection is ignored.

**Violation scenario:** Developer adds a scoped feature assuming the sidebar's selected shared mailbox affects queries. It doesn't — all loads use `AccountScope` from the plain account selector.

**Structural fix:** A single `Scope` enum (`All`, `Account(id)`, `SharedMailbox(id)`, `PublicFolder(id)`) consumed by all query/context builders.

---

## Medium — Quality Improvements

### ~~11. Overlay exclusivity~~ ✅ Fixed

`dismiss_overlays()` closes all mutually exclusive overlays (palette, settings, calendar overlays, add-account wizard). Called at the start of every overlay open path: `open_settings()`, palette `Open`, add-account wizard, re-auth wizard. Replaces ad-hoc per-caller checks.

### 12. Calendar pop-out awareness

**Contract:** When calendar is popped out, calendar actions must route to the pop-out, not flip the main window to calendar mode.

**Currently enforced by:** Ad-hoc `.find(Calendar)` checks at 2 of 6+ calendar entry points. `SetAppMode(Calendar)`, `SetCalendarView`, `CalendarToday`, `CalendarCreateEvent` all bypass.

**Structural fix:** `fn calendar_target(&self) -> CalendarTarget { PopOut(window_id) | Inline }`.

### 13. Search state is a multi-field protocol

**Contract:** `search_query`, `thread_list.search_query`, `search_generation`, `debounce_deadline`, `thread_list.mode`, `was_in_folder_view`, and pinned search state must move together.

**Currently enforced by:** Two partial helper methods. Other paths modify individual fields directly.

**Structural fix:** `SearchContext` struct with `enter_search()`, `clear_search()`, `select_pinned_search()`.

### ~~14. `composer_is_open` boolean vs reality~~ ✅ Fixed

Replaced the manually-synced `composer_is_open: bool` field with a computed `fn composer_is_open(&self) -> bool` that queries `pop_out_windows` directly. Removed all 4 manual write sites.

### 15. Generation counter ordering

**Contract:** Async loads must capture the right generation counter, and callers must bump before dispatching.

**Currently enforced by:** Convention across 5+ generation counters (`nav_generation`, `thread_generation`, `search_generation`, etc.).

**Structural fix:** Typed loader helpers that allocate and validate generations automatically.

### ~~16. Pinned search state duplication~~ ✅ Fixed

Removed `active_pinned_search` from App. The single source of truth is now `sidebar.active_pinned_search`. All 7 read/write sites updated. Impossible to desync.

### 17. Pop-out window lifecycle gaps

**Contract:** Adding a new `PopOutWindow` variant requires handling in 7 code sites (title, view, resize, move, close, save session, message routing). 4 of 7 sites use wildcards/catch-alls that silently ignore new variants.

**Structural fix:** Remove wildcards in window management code, or a trait on `PopOutWindow` variants.

### ~~18. Action context degraded-mode boilerplate~~ ✅ Fixed

Replaced 9 identical `let Some(ref action_ctx) = self.action_ctx else { ... }` blocks with `fn action_ctx(&self) -> Option<ActionContext>`. All call sites now use `let Some(ctx) = self.action_ctx() else { return ... }`. Two sites with custom degraded-mode messages keep their custom logic in the else branch.

### 19. Provider APIs are stringly typed

**Contract:** `move_to_folder(&str)`, `add_tag(&str)`, and `apply_category(&str)` all take raw `&str`. Callers must know whether they're passing a folder ID, tag ID, or category name. `apply_category`/`remove_category` silently no-op on providers that don't support them.

**Currently enforced by:** Nothing in types. Wrong string kind compiles and may do the wrong thing or nothing.

**Structural fix:** Typed IDs (`FolderId`, `TagId`) and capability markers (`SupportsCategories`) instead of raw `&str` plus no-op defaults.

### 20. Tables missing CASCADE foreign keys

**Contract:** All tables with `account_id` should cascade on account deletion. 20+ tables added in later migrations have `account_id TEXT NOT NULL` without the foreign key constraint, leaving orphan rows on account deletion.

**Structural fix:** Migration adding FK constraints (requires table recreation in SQLite).

### ~~21. Add-account wizard vs settings exclusivity~~ ✅ Fixed

Handled by the overlay exclusivity system (#11). `dismiss_overlays()` closes settings before opening the wizard, and vice versa.

### ~~22. Reading pane star state manual sync~~ ✅ Fixed

`sync_reading_pane_after_toggle()` handles reading pane sync for both optimistic toggles (`use_new_value: true`) and rollbacks (`use_new_value: false`). Centralized in one method — future toggles that affect the reading pane add one match arm here instead of manual calls at each site.

---

## Implementation Strategy

The action service effort proved the pattern: identify the invariant, make it structural, enforce at compile time where possible. For UI contracts, compile-time enforcement isn't always possible, but centralizing behind a single function eliminates the "forgot a step" failure mode.

**Recommended order:**
1. **#1** (action service boundary for DB writes) — same pattern as Phase 6, highest leverage
2. **#3** (draft save on close) — user data loss
3. **#4** (CommandId dispatch) — one-line fix, prevents silent failures
4. **#5 + #7** (navigation + selection reset) — highest-frequency developer touchpoint
5. **#6** (settings protocol) — already violated
6. The rest can be prioritized by product need

Each contract should be its own commit (or small series), following the same plan → implement → review cycle used for the action service phases.
