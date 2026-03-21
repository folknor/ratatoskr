# Main Layout: Spec vs. Code Discrepancies

Audit date: 2026-03-21

---

## Divergences

### Core's `get_thread_detail()` not wired
The bridge module `crates/app/src/db/threads.rs` defines `load_thread_detail()` which calls core's `get_thread_detail()`, but it is **never called**. No `ThreadDetailLoaded` message exists in the `Message` enum (`main.rs:148-238`). The actual thread selection path at `main.rs:1328-1341` uses `Db::get_thread_messages()` + `Db::get_thread_attachments()` (raw SQL in `db/threads.rs:163-241`). Consequences:

- `body_html` and `body_text` are always `None` (raw SQL sets them to `None` at `db/threads.rs:194-195`)
- `is_own_message` is always `false` (hardcoded at `db/threads.rs:198`)
- No collapsed summaries from core (uses `snippet` from messages table)
- No resolved label colors from core (`thread_labels` on ReadingPane is always empty)
- No `BodyStoreState` on `App` struct (`main.rs:240-289`)

**Files:** `crates/app/src/main.rs:1315-1345`, `crates/app/src/db/threads.rs:163-206`

### No PaneGrid — custom divider implementation
The problem statement references `PaneGrid`. The implementation uses manual divider widgets with `mouse_area` drag tracking. Conscious divergence.
- Code: `crates/app/src/main.rs:1511-1525`

### Thread list label dots always empty
`label_colors` is hardcoded to `&[]`. The `label_dot()` widget exists but never receives real data because core's `get_thread_detail()` is unwired.
- Code: `crates/app/src/ui/thread_list.rs:280`, `crates/app/src/ui/widgets.rs:798`

### Calendar CRUD bypasses core
Raw SQL for calendar event CRUD and schema management (`CREATE TABLE IF NOT EXISTS`). Outside main-layout scope but establishes app-level schema precedent.
- Code: `crates/app/src/db/connection.rs`

### Right sidebar shows placeholder content only
Renders static "Calendar placeholder" and "No pinned items" text. Calendar is a separate full-page mode (`AppMode::Calendar`), not integrated into the sidebar.
- Code: `crates/app/src/ui/right_sidebar.rs:28-60`

### HTML rendering pipeline exists but cannot fire in reading pane
`render_html()` is called from `expanded_message_card()` when `msg.body_html` is `Some`. However, since core's `get_thread_detail()` is unwired, `body_html` is always `None` in the main window path. The pipeline code is correct but dead in practice.
- Code: `crates/app/src/ui/html_render.rs:70`, `crates/app/src/ui/widgets.rs:1077`

### Message collapse rule 4 (own messages) cannot work
`ReadingPane::apply_message_expansion()` checks `is_own_message`, but since the raw SQL path hardcodes `is_own_message: false`, rule 4 never triggers.
- Code: `crates/app/src/db/threads.rs:198`

### Attachment collapse persistence partially wired
`persist_attachments_collapsed()` calls core's `set_attachments_collapsed()`. However, the read path (`get_thread_detail` which returns `attachments_collapsed`) is unwired. The in-memory cache in `ReadingPane` provides fast reads but the persisted value from core is never loaded.
- Code: `crates/app/src/db/threads.rs:142-157`

---

## Not implemented

### Interaction flow (Phase 3, partial)
- **Keyboard shortcuts** — j/k, Enter, Escape wired via command palette. Email action shortcuts available.
- **Auto-advance** after archive/trash/move — not implemented. No `get_adjacent_thread()` or advance logic anywhere in `crates/app/src/`.
- **Multi-select** (Shift+click range, Ctrl+click toggle) — not implemented. No multi-select code found in `crates/app/src/`.
- **Inline reply composer** — not implemented.
- **Context-dependent shortcut dispatch** — `FocusedRegion` exists on `App` (`main.rs:268`) but region-specific key behavior table not fully implemented.
- Code: `crates/app/src/main.rs:268`

### Scroll virtualization
Thread list renders all cards in `column![]` inside `scrollable`. No virtualization. `THREAD_CARD_HEIGHT` exists for future use.
- Code: `crates/app/src/ui/thread_list.rs`

### Image hover preview on attachment cards
Not implemented.
- Spec: `docs/main-layout/problem-statement.md`

### Attachment save/open behavior
Attachment cards are display-only. No click/save/open wiring.
- Spec: `docs/main-layout/problem-statement.md`

### Search typeahead popups
Search bar is a plain `text_input` with no typeahead overlay.
- Code: `crates/app/src/ui/sidebar.rs` (search bar)

---

## Cross-cutting

### Generational load tracking
Implemented. Three counters: `nav_generation`, `thread_generation`, `search_generation`. Guards `ThreadMessagesLoaded`/`ThreadAttachmentsLoaded`. Stale responses discarded.
- Code: `crates/app/src/main.rs:329-330,339`

### Core CRUD bypass
Mostly raw SQL. Thread detail: raw SQL (`Db::get_thread_messages` + `Db::get_thread_attachments`). Thread listing: `get_threads_scoped` from core is primary path. Attachment collapse writes: core's `set_attachments_collapsed`. Everything else (accounts, labels, calendar, pinned searches, contacts): raw SQL.
- Code: `crates/app/src/main.rs:56`

### Dead code
- `PendingChord::started` — `#[allow(dead_code)]` at `crates/app/src/main.rs:143-144`
- `load_thread_detail`, `persist_attachments_collapsed` read path — `crates/app/src/db/threads.rs:118-139`
- `init_body_store` — `crates/app/src/db/threads.rs:328-333`
