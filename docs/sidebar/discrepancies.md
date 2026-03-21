# Sidebar: Spec vs. Code Discrepancies

Audit date: 2026-03-21

---

## Divergences

### Pinned search card styling

**Spec (1E.4):** Uses `theme::ButtonClass::Nav { active }`.

**Code** (`sidebar.rs:435`): Uses `theme::ButtonClass::PinnedSearch { active }`. Purpose-built style class for visual distinction.

### Pinned search event carries only ID

**Spec (1E.3):** `PinnedSearchSelected(i64, String)` carries ID + query.

**Code** (`sidebar.rs:47`): `PinnedSearchSelected(i64)` carries only ID. Parent handles query lookup.

### Pinned search positioning includes mode toggle

**Spec (1E.5):** Pinned searches between scope dropdown and compose button.

**Code** (`sidebar.rs:198-212`): Header row now includes mode toggle button (calendar toggle) left of scope dropdown, which was not in the original spec layout. Pinned searches appear between this header row and compose button.

### SidebarEvent::CycleAccount is dead code

`SidebarEvent::CycleAccount` variant exists (`sidebar.rs:43`) but is never emitted. The `SidebarMessage::CycleAccount` handler (`sidebar.rs:122-134`) directly updates state and emits `SidebarEvent::AccountSelected(next)`. The parent's `CycleAccount` arm (`main.rs:898`) maps to `Task::none()` and is dead code.

---

## Not implemented

### Scope persistence

Problem statement open question #6: `selected_account` is in-memory state, resets to `None` on launch. Documented as deferred.
- Spec: `docs/sidebar/problem-statement.md` open question #6

### NavigationTarget enum

Spec 1A transitional note: `selected_label: Option<String>` is semantically muddy (universal folders, smart folders, and account labels share one field). Deferred to a future `NavigationTarget` enum.
- Spec: `docs/sidebar/implementation-spec.md` section 1A

### Mixed drafts list view

Problem statement: clicking Drafts should show server-synced + local-only drafts in a mixed view. Count path (`get_draft_count_with_local`) handles both. List path (`get_draft_threads` in `scoped_queries.rs`) returns only server-synced drafts. Blocked on a design decision between a `DraftItem` enum or query-time promotion of local drafts.
- Code: `crates/core/src/db/queries_extra/scoped_queries.rs` (`get_draft_threads`)
