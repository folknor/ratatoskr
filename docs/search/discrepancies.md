# Search: Spec vs Implementation Discrepancies

Audit date: 2026-03-21 (updated after implementation pass)

## What Matches the Spec

### Backend (Slices 1-4) -- Fully Implemented

- **Parser overhaul (Slice 1):** `crates/smart-folder/src/parser.rs` matches the spec. `ParsedQuery` has all specified fields (`Vec<String>` for OR-capable operators, `attachment_types`, `has_contact`, `is_tagged`, `in_folder`, `folder`, `account`). `HAS_EXPANSIONS` table matches the spec exactly. `has_any_operator()` covers all fields. Greedy date parsing with `extract_date_value` is implemented. `subject:` and `is:important` are removed as specified.

- **SQL builder (Slice 2):** `crates/smart-folder/src/sql_builder.rs` implements all clause builders specified: `account:` (LIKE on display_name/email), `folder:` (label name + imap_folder_path), `in:` (label-based and flag-based shorthands), `is:tagged`, `has:contact`, `type:`/`has:` MIME filtering with glob support, `from:`/`to:` with contact expansion. OR semantics for repeated operators implemented correctly.

- **Tantivy cross-account (Slice 3):** `crates/search/src/lib.rs` has `SearchParams.account_ids: Option<Vec<String>>` (not single `account_id`). `group_by_thread()` helper exists and is public. **`SearchParams.from` and `SearchParams.to` are now `Vec<String>`** for proper OR semantics across all search paths.

- **Unified pipeline (Slice 4):** `crates/core/src/search_pipeline.rs` implements the three-path router (`search_sql_only`, `search_tantivy_only`, `search_combined`) exactly as specified. `UnifiedSearchResult` type matches. The combined path does SQL-first then Tantivy intersection as designed. **`build_tantivy_params()` now passes all from/to values**, not just the first.

### App Integration (Slice 5) -- Wired to Unified Pipeline

- **Generational load tracking:** Implemented. `search_generation: u64` in `App`, incremented before each dispatch, stale results silently dropped via `g != self.search_generation` guard. Also incremented on `SearchClear`.

- **Message enum:** All specified variants present: `SearchQueryChanged`, `SearchExecute`, `SearchResultsLoaded(u64, ...)`, `SearchClear`, `FocusSearchBar`, `SearchBlur`.

- **Debounce subscription:** Implemented with `search_debounce_deadline: Option<iced::time::Instant>` and 50ms polling timer, matching the spec's V1 timer strategy.

- **ThreadListMode:** Implemented as `enum ThreadListMode { Folder, Search }` on `ThreadList`.

- **Search bar widget:** Real `text_input` in `thread_list_header` with `SearchInput`/`SearchSubmit` messages, mapped through `ThreadListEvent` to `App` messages.

- **Component trait:** `ThreadList` implements `Component` trait (as do `Sidebar`, `ReadingPane`, `StatusBar`, `Settings`, `AddAccountWizard`). The search bar is part of the `ThreadList` component, not a separate component -- this matches the app-integration-spec which says "It is not a separate Component."

- **Keyboard shortcuts:** `/` to focus and `Escape` to clear are implemented via event listeners in the subscription.

- **Search execution:** `execute_search()` now calls the unified pipeline (`search_pipeline::search()`) when the Tantivy index is available, with a graceful SQL-only fallback when it is not. The fallback uses the smart folder parser and SQL builder to support structured operators, and falls back to LIKE search only for pure free-text queries without an index.

- **SearchBlur unfocus:** `SearchBlur` handler now focuses a dummy widget ID to effectively remove focus from the search bar (iced does not expose a native `unfocus` operation).

### Pinned Searches -- Substantially Implemented

- **PinnedSearch type:** `crates/app/src/db/pinned_searches.rs` has the `PinnedSearch` struct (without `thread_ids` field -- loaded lazily as spec allows).

- **CRUD functions:** All specified: `create_or_update_pinned_search`, `update_pinned_search`, `delete_pinned_search`, `delete_all_pinned_searches`, `list_pinned_searches`, `get_pinned_search_thread_ids`, `get_threads_by_ids`, `expire_stale_pinned_searches`. Uses transactions for atomicity.

- **Sidebar rendering:** `pinned_searches_section` and `pinned_search_card` in `sidebar.rs`. Has `ButtonClass::PinnedSearch { active }` style in the theme.

- **Lifecycle state machine:** `active_pinned_search`, `editing_pinned_search` state in `App`. Edit-in-place updates existing pinned search; new searches create new entries. Navigation away clears pinned search context.

- **Generational tracking for pinned search loads:** Uses `nav_generation` for `PinnedSearchThreadIdsLoaded` and `PinnedSearchThreadsLoaded` staleness checks.

- **Auto-expiry:** `expire_stale_pinned_searches(1_209_600)` (14 days) called after initial load.

### Smart Folder Token Migration (Slice 6) -- Partial

- **Token system deprecated:** `execute_smart_folder_query` and `count_smart_folder_unread` now use `migrate_legacy_tokens()` which translates `__LAST_7_DAYS__` -> `-7`, `__LAST_30_DAYS__` -> `-30`, `__TODAY__` -> `0` inline. The parser handles relative offsets natively. `resolve_query_tokens` is no longer re-exported from the crate. `count_matching` is now exported for direct use.

### Dead Code Cleanup

- **`SearchState.index` and `SearchState.schema`** fields removed from `SearchState` struct. No longer `#[allow(dead_code)]`.
- **`SearchState::search()`** simple free-text method removed. Only `search_with_filters()` remains.
- **`SearchParams.label`** field removed entirely. Label filtering is handled by the SQL builder, not Tantivy.
- **`group_by_thread()` deduplication:** The private copy in `search_pipeline.rs` now delegates to the public `ratatoskr_search::group_by_thread()`, wrapping the results in `UnifiedSearchResult`.

---

## What Diverges from the Spec

### UnifiedSearchResult vs SearchResult naming

The problem statement spec defines `SearchResult`. The implementation names it `UnifiedSearchResult`. The app-integration-spec acknowledges four result types in play (`UnifiedSearchResult`, `Thread`, `DbThread`, `SearchResult`) but the Tantivy crate's `SearchResult` and the core's `UnifiedSearchResult` are separate types. This is noted as a known seam in the spec but remains unresolved.

### PinnedSearch struct diverges from spec

The pinned-searches spec defines `PinnedSearch` with a `thread_ids: Vec<(String, String)>` field. The implementation omits this field from the struct entirely, loading thread IDs via a separate `get_pinned_search_thread_ids()` call. This is a deliberate design choice (lazy loading) but diverges from the spec's data model.

### Smart folder execution path not fully migrated

`execute_smart_folder_query` still uses its own direct path (parse -> SQL builder -> execute) rather than calling `search()` from the unified pipeline. This is intentional: the unified pipeline lives in `ratatoskr-core` which depends on `ratatoskr-smart-folder`, so calling back would create a circular dependency. The token system has been deprecated in favor of inline migration, but the execution path remains SQL-only for smart folders (no Tantivy ranking for smart folder queries that contain free text).

### SearchState initialization is lazy

`SearchState` is initialized per-search in `execute_search()` by calling `SearchState::init()` each time, rather than being initialized once at app startup and stored on `App`. This works because `SearchState::init()` opens an existing index (cheap) and the index directory is reused. However, storing it on `App` would avoid the per-search overhead.

---

## What's Missing (Not Yet Built)

### Operator Typeahead (Phase 3 of app-integration-spec) -- Implemented

Typeahead popup is implemented in `crates/app/src/ui/thread_list.rs` with `TypeaheadState`, `TypeaheadItem`, and `TypeaheadDirection` types. Cursor context analysis lives in `crates/smart-folder/src/parser.rs` (`analyze_cursor_context()`). Covers all specified operators:

- **Static presets:** `has:`, `is:`, `in:`, `before:`, `after:` populate items synchronously from const preset arrays.
- **Dynamic DB queries:** `from:`/`to:` search contacts via `search_autocomplete()`, `account:` lists accounts, `label:`/`folder:` search labels across all accounts via `search_labels_for_typeahead()`.
- **Keyboard navigation:** Arrow Up/Down to navigate, Enter/Tab to accept, Escape to dismiss. Handled in `handlers/keyboard.rs` before the captured-event skip.
- **Selection insertion:** `apply_typeahead_selection()` replaces the partial value, quoting values with spaces, and appending a trailing space.
- **Visual design:** Uses `ContainerClass::Elevated` styling, `ButtonClass::Dropdown` for items, stacked over the thread list body.

**Minor divergences from spec:**
- No debounce on `from:`/`to:` contact lookups (spec calls for 50ms). The query fires immediately. The DB query is fast enough that debounce is not needed for typical mailbox sizes.
- Label/folder typeahead is not scoped by `account:` operator in the query. It searches across all accounts and shows the account email in the detail field.
- `analyze_cursor_context()` assumes cursor is at end of query string (iced text_input on_input provides the full value but not cursor position). Mid-query editing may not trigger the correct context.

### "Search here" Interaction (Phase 4 of app-integration-spec)

No right-click context menu on sidebar folders/labels to prefill the search bar with scope operators. No evidence of this interaction in the sidebar component.

### Smart Folder "Save as Smart Folder" from Search

No command palette command to save the current search query as a smart folder. No graduation path from pinned search to smart folder exists in the command palette.

### Smart Folder Form Editor Removal

The settings UI still has a "Smart Folders" section (`settings/tabs.rs:563`) showing "Coming soon." The spec calls for removing the form-based editor entirely, but it was never built in the first place (it's a placeholder).

### Search Result Highlighting in Reading Pane

The spec calls for matching messages to be expanded and matching terms highlighted in the reading pane when a search result is selected. This is not implemented.

### Search History

No search history feature (recent queries via up-arrow in empty search bar).

### Search Result Count Indicator

No result count shown in the UI (though `self.status` is set to `"{n} results"` in the status bar on search completion, which may partially satisfy this).

---

## Cross-Cutting Concern Status

### a. Generational Load Tracking

**Implemented.** `search_generation: u64` follows the bloom pattern. Incremented on `SearchExecute` and `SearchClear`. Stale `SearchResultsLoaded` silently dropped. Pinned search thread loads use `nav_generation` for the same purpose.

### b. Component Trait

**Used.** `ThreadList` implements `Component`. The search bar lives inside `ThreadList::view()` as specified (not a separate component). `Sidebar` also implements `Component` and handles pinned search card rendering.

### c. Token-to-Catalog Theming

**Partially used.** The search bar uses `theme::TextClass::Tertiary.style()` for context labels. Pinned search cards use `theme::ButtonClass::PinnedSearch { active }`. Layout constants (`TEXT_SM`, `SPACE_XXS`, `PAD_PANEL_HEADER`, `PAD_INPUT`, `TEXT_MD`) are used from `layout.rs`. This follows the named-style-class pattern.

### d. iced_drop Drag-and-Drop

**N/A.** No drag-and-drop in search features.

### e. Subscription Orchestration

**Used for debounce.** The search debounce timer uses `iced::time::every(50ms)` polling subscription, active only when `search_debounce_deadline` is `Some`. This is acknowledged in the spec as a "slightly wasteful but acceptable for V1" approach. No `subscription::channel` for off-main-thread search execution -- search uses `Task::perform` instead.

### f. Core CRUD Bypassed

**No, for search execution.** `execute_search()` now calls the unified search pipeline from `ratatoskr_core::search_pipeline::search()`, with a SQL-only fallback that still uses the smart folder parser and SQL builder. Raw SQL LIKE is only used as a last-resort fallback for pure free-text when no Tantivy index exists.

**No, for pinned searches.** Pinned search CRUD is in `crates/app/src/db/pinned_searches.rs`, which is app-level code but uses proper parameterized queries and transactions. The spec itself places pinned search CRUD in the app's `db.rs`, so this is intentional -- pinned searches are local UI state, not core domain logic.

### g. Dead Code

- `group_by_thread()` in search crate is the canonical version; `search_pipeline.rs` delegates to it via `group_by_thread_unified()`.
- `tokens.rs` in the smart-folder crate is no longer re-exported. `migrate_legacy_tokens()` handles the translation inline. The `tokens` module is retained for the test suite during the deprecation period.
