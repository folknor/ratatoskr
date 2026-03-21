# Search: Spec vs. Code Discrepancies

Audit date: 2026-03-21

---

## Divergences

### SearchBlur does not unfocus

`Message::SearchBlur => Task::none()` — the handler is a no-op. The spec (app-integration-spec Phase 1) calls for focusing a dummy widget ID to remove focus from the search bar. No unfocus behavior exists.
- Code: `crates/app/src/main.rs:640`

### UnifiedSearchResult vs SearchResult naming

The implementation uses `UnifiedSearchResult`; the spec defines `SearchResult`. The Tantivy crate has its own `SearchResult` type. Two parallel result types remain.
- Code: `crates/app/src/search_pipeline.rs:18`, `crates/search/src/lib.rs`

### PinnedSearch struct omits thread_ids field

Spec defines `PinnedSearch` with `thread_ids: Vec<(String, String)>`. Implementation omits this field entirely; thread IDs are loaded lazily via `get_pinned_search_thread_ids()`. Deliberate design choice but diverges from spec data model.
- Code: `crates/app/src/db/pinned_searches.rs:10-15`

### Smart folder execution path not migrated to unified pipeline

`execute_smart_folder_query` uses its own direct path (parse -> SQL builder -> execute) rather than calling `search()` from the unified pipeline. This is a circular-dependency constraint (`ratatoskr-core` depends on `ratatoskr-smart-folder`). Smart folders get no Tantivy ranking for free-text queries.
- Code: `crates/smart-folder/src/lib.rs:26-33`

### SearchState initialized per-search, not stored on App

`SearchState::init()` is called inside `execute_search()` on every search dispatch. Not stored on `App` as the spec implies. Works because `init()` opens an existing index, but adds per-search overhead.
- Spec: `docs/search/app-integration-spec.md` line ~1250
- Code: `crates/app/src/handlers/search.rs:356`

### delete_all_pinned_searches exists but is not wired

The function exists but no `Message` variant or handler dispatches to it. Dead code from the user's perspective. The spec calls for a "Clear all" sidebar action.
- Code: `crates/app/src/db/pinned_searches.rs:285`

### Auto-expiry is startup-only, not periodic

`expire_stale_pinned_searches(1_209_600)` runs once after initial `PinnedSearchesLoaded`, guarded by `expiry_ran: bool`. The spec calls for a daily periodic subscription via `iced::time::every(86400s)`. No `RunPinnedSearchExpiry` message exists.
- Code: `crates/app/src/handlers/search.rs:130-136`, `crates/app/src/main.rs:285`

---

## Not implemented

### Operator typeahead (Phase 3)

No typeahead popup implementation. No contact lookup for `from:`/`to:`, no account-scoped `label:`/`folder:` suggestions, no date presets for `before:`/`after:`. The search bar is a plain `text_input` without any popup or overlay.
- Spec: `docs/search/app-integration-spec.md` Phase 3

### "Search here" interaction (Phase 4)

No right-click context menu on sidebar folders/labels to prefill the search bar with scope operators.
- Spec: `docs/search/app-integration-spec.md` Phase 4

### "Save as Smart Folder" from search

No command palette command to save the current search query as a smart folder. No graduation path from pinned search to smart folder.
- Spec: `docs/search/app-integration-spec.md`

### Pinned search staleness label in sidebar

Spec defines a `staleness: Option<String>` field and relative-time label ("Last updated 5 minutes ago") rendered below the search bar. Not implemented.
- Spec: `docs/search/pinned-searches-implementation-spec.md` lines ~1153-1205

### Search result highlighting in reading pane
Spec calls for matching messages to be expanded and matching terms highlighted when a search result is selected. Not implemented.
- Spec: `docs/search/app-integration-spec.md`

### Search history
No search history feature (recent queries via up-arrow in empty search bar).
- Spec: `docs/search/app-integration-spec.md`

### Smart folder form editor removal

Settings UI has "Smart Folders" section showing "Coming soon." The spec calls for removing the form-based editor, but it was never built (placeholder only).
- Code: `crates/app/src/ui/settings/tabs.rs:564`

---

## Cross-cutting

### Keyboard shortcuts

Implemented via command dispatch. `/` bound to `CommandId::AppSearch` -> `Message::FocusSearch` -> `Message::FocusSearchBar`. `Escape` dispatches `Message::SearchClear` when search is active. Both wired through command dispatch, not raw event listeners.
- Code: `crates/command-palette/src/registry.rs:703`, `crates/app/src/main.rs:611-615,627,639`

### Generational tracking

Implemented. `search_generation: u64` incremented on `SearchExecute` and `SearchClear`. Stale `SearchResultsLoaded` dropped. Pinned search loads use `nav_generation`.
- Code: `crates/app/src/main.rs:636,645,651`

### Search result count

Partial. `self.status` set to `"{n} results"` and `"{n} threads (pinned search)"`. Shown in status bar, not in thread list header.
- Code: `crates/app/src/handlers/search.rs:65,218`
