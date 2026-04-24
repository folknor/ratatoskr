# Main Layout: Spec vs. Code Discrepancies

Audit date: 2026-03-23

---

## Resolved

### Core's `get_thread_detail()` - WIRED ✅
`ThreadDetailLoaded` message exists and is dispatched. `load_thread_detail()` calls core's `get_thread_detail()` with `BodyStoreState`. `body_html`, `body_text`, `is_own_message`, collapsed summaries, and resolved label colors all flow through. Old raw SQL fallback path and `ThreadMessagesLoaded`/`ThreadAttachmentsLoaded` message variants removed (2026-03-23).

### HTML rendering pipeline now fires ✅
With `get_thread_detail()` wired, `body_html` is populated from the body store. `render_html()` in `expanded_message_card()` now receives real data.

### Message collapse rule 4 (own messages) now works ✅
`is_own_message` is populated by core's thread detail, so `apply_message_expansion()` rule 4 triggers correctly.

### Attachment collapse persistence fully wired ✅ (2026-03-23)
`AttachmentCollapseChanged` event now calls `persist_attachments_collapsed()`. Read path loads via `get_thread_detail()`.

### Label pills in reading pane ✅ (2026-03-23)
`thread_labels` (with resolved colors from core) rendered as colored pills in the thread header row.

---

## Divergences

### No PaneGrid - custom divider implementation
The problem statement references `PaneGrid`. The implementation uses manual divider widgets with `mouse_area` drag tracking. Conscious divergence.

### Thread list label dots always empty
`label_colors` is hardcoded to `&[]` in `thread_list_body()`. The `label_dot()` widget exists but thread list queries don't include per-thread label data. Separate from reading pane labels (which are now wired).
- Code: `crates/app/src/ui/thread_list.rs`

### Calendar CRUD bypasses core
Raw SQL for calendar event CRUD and schema management. Outside main-layout scope.

### Right sidebar shows placeholder content only
Renders static "Calendar placeholder" and "No pinned items" text. Calendar is a separate full-page mode (`AppMode::Calendar`), not integrated into the sidebar.

---

## Not implemented

### Interaction flow (Phase 3, partial)
- **Keyboard shortcuts** - j/k, Enter, Escape wired via command palette. Email action shortcuts available.
- **Auto-advance** after archive/trash/move - not implemented.
- **Multi-select** (Shift+click range, Ctrl+click toggle) - not implemented.
- **Inline reply composer** - not implemented.
- **Context-dependent shortcut dispatch** - `FocusedRegion` exists but region-specific key behavior table not fully implemented.

### Scroll virtualization
Thread list renders all cards in `column![]` inside `scrollable`. No virtualization. `THREAD_CARD_HEIGHT` exists for future use.

### Image hover preview on attachment cards
Not implemented.

### Attachment save/open behavior
Attachment cards are display-only. No click/save/open wiring.

### Search typeahead popups
Search bar is a plain `text_input` with no typeahead overlay.

---

## Cross-cutting

### Generational load tracking
Implemented. Three counters: `nav_generation`, `thread_generation`, `search_generation`. Guards `ThreadDetailLoaded`. Stale responses discarded.

### Core CRUD bypass
Thread detail now uses core's `get_thread_detail()`. Thread listing uses `get_threads_scoped` from core. Attachment collapse uses core's `set_attachments_collapsed`. Calendar, contacts, and other domains have varying levels of core integration.

### Dead code
- `PendingChord::started` - `#[allow(dead_code)]`
