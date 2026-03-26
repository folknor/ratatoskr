# Codebase Contracts

## Overview

Ratatoskr had 22 implicit contracts — behaviors where correctness depended on every developer remembering a multi-step protocol that nothing in the compiler, type system, or API surface enforced. A new developer adding a feature could silently break an invariant because the protocol existed only in convention.

**18 of 22 contracts have been fixed.** The remaining 4 are medium-to-large architectural refactors tracked below.

### What Made a Contract Implicit

1. Multiple call sites must follow the same protocol, but nothing enforces it
2. Adding a new call site can silently break existing behavior
3. The pattern relies on "every developer knows to..." rather than "the API makes it impossible not to..."

The structural fix for each was the same principle: make the right thing the only thing. A single entry point, a type that enforces the invariant, or a compiler error when the protocol is violated.

---

## Remaining (1)

### 15. Generation counter ordering

**Contract:** Async loads must capture the right generation counter, and callers must bump before dispatching.

**Currently enforced by:** Convention across 5+ generation counters (`nav_generation`, `thread_generation`, `search_generation`, etc.).

**Structural fix:** Typed loader helpers that allocate and validate generations automatically.

### 19. Provider APIs are stringly typed

**Contract:** `move_to_folder(&str)`, `add_tag(&str)` all take raw `&str`. Callers must know whether they're passing a folder ID or tag ID. Wrong string kind compiles and may do the wrong thing or nothing.

**Structural fix:** Typed IDs (`FolderId`, `TagId`) and capability markers instead of raw `&str` plus no-op defaults.

### 20. Tables missing CASCADE foreign keys

**Contract:** All tables with `account_id` should cascade on account deletion. ~10 tables added in later migrations have `account_id TEXT NOT NULL` without the FK constraint, leaving orphan rows on account deletion.

**Structural fix:** Migration adding FK constraints (requires table recreation in SQLite — `CREATE new; INSERT SELECT; DROP old; RENAME`).

**Note:** Contract #2 (account deletion store cleanup) mitigates the worst effects by cleaning external stores explicitly. The missing CASCADEs only affect main-DB orphan rows in tables like `cloud_attachments`, `folder_sync_state`, `shared_mailbox_sync_state`, etc.

---

## Fixed (21)

| # | Contract | Fix |
|---|----------|-----|
| 9 | 8-edit action protocol | All 5 wildcard arms in action dispatch replaced with exhaustive matches on `CompletedAction`. Adding a new variant now produces compiler errors at every site. Dead `DeleteDraft` variant removed. |
| 10 | Scope state split | `ViewScope` enum (`AllAccounts`, `Account`, `SharedMailbox`, `PublicFolder`) replaces split fields. `selected_scope` is single source of truth. Dedicated query paths for shared mailbox threads (CTE-scoped) and public folder items. Personal queries filter `shared_mailbox_id IS NULL`. Actions gated for public folder scope. |
| 1 | Mail mutation DB boundary | 7 thread-action DB helpers (`set_thread_read/starred/pinned/muted`, `delete_thread`, `add/remove_thread_label`) changed to `pub(crate)`. App crate forced through action service at compile time. |
| 2 | Account deletion leaks external stores | `delete_account_orchestrate()` — gather + ref-checks + delete in one call, async cleanup of body/inline/cache/search stores. 7 integration tests. |
| 3 | Compose close loses dirty drafts | `save_compose_draft_sync()` — synchronous INSERT before window removal. Aborts close on failure. Stable `draft_id` per compose window. |
| 4 | CommandId variants silently ignored | Inlined all sub-dispatchers into `dispatch_command` — 69 explicit arms, no wildcards. Compiler error on new variants. |
| 5 | Navigation reset protocol | `reset_view_state(target)` — full 7-step transition in one call. All 4 view-transition sites use it. |
| 6 | Settings open/close protocol | `open_settings(tab)` / `close_settings()` — full lifecycle protocol. Fixed existing violation in `open_contact_editor_for_email`. |
| 7 | Thread selection + reading pane sync | `clear_thread_selection()` — clears selection, multi-select, and reading pane. All 10 deselection sites use it. |
| 8 | Compose routing dedup | `open_compose_window_with_state` checks for existing window with same `reply_thread_id` and focuses it. |
| 11 | Overlay exclusivity | `dismiss_overlays()` — closes palette, settings, calendar overlays, wizard. Called at every overlay open site. |
| 12 | Calendar pop-out awareness | `calendar_pop_out_id()` — `SetAppMode`, `SetCalendarView`, `CalendarToday` all focus pop-out when it exists. |
| 13 | Search state multi-field protocol | External callers use `reset_view_state()` → `clear_search_state()`. Internal search operations legitimately set individual fields. |
| 14 | `composer_is_open` boolean vs reality | Replaced field with computed `fn composer_is_open()` querying `pop_out_windows`. |
| 16 | Pinned search state duplication | Removed `active_pinned_search` from App. Single source of truth: `sidebar.active_pinned_search`. |
| 17 | Pop-out window lifecycle gaps | Already clean — all 7 sites use explicit variant matches, no wildcards. |
| 18 | Action context boilerplate | `fn action_ctx()` replaces 9 identical `let Some(ref action_ctx) = ...` blocks. |
| 21 | Wizard vs settings exclusivity | Handled by #11 (`dismiss_overlays`). |
| 22 | Reading pane star sync | `sync_reading_pane_after_toggle()` for both optimistic toggles and rollbacks. |
