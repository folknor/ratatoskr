# Search: Spec vs Implementation Discrepancies

Audit date: 2026-03-21

## What Matches the Spec

### Backend (Slices 1-4) -- Fully Implemented

- **Parser overhaul (Slice 1):** `crates/smart-folder/src/parser.rs` matches the spec. `ParsedQuery` has all specified fields (`Vec<String>` for OR-capable operators, `attachment_types`, `has_contact`, `is_tagged`, `in_folder`, `folder`, `account`). `HAS_EXPANSIONS` table matches the spec exactly. `has_any_operator()` covers all fields. Greedy date parsing with `extract_date_value` is implemented. `subject:` and `is:important` are removed as specified.

- **SQL builder (Slice 2):** `crates/smart-folder/src/sql_builder.rs` implements all clause builders specified: `account:` (LIKE on display_name/email), `folder:` (label name + imap_folder_path), `in:` (label-based and flag-based shorthands), `is:tagged`, `has:contact`, `type:`/`has:` MIME filtering with glob support, `from:`/`to:` with contact expansion. OR semantics for repeated operators implemented correctly.

- **Tantivy cross-account (Slice 3):** `crates/search/src/lib.rs` has `SearchParams.account_ids: Option<Vec<String>>` (not single `account_id`). `group_by_thread()` helper exists and is public.

- **Unified pipeline (Slice 4):** `crates/core/src/search_pipeline.rs` implements the three-path router (`search_sql_only`, `search_tantivy_only`, `search_combined`) exactly as specified. `UnifiedSearchResult` type matches. The combined path does SQL-first then Tantivy intersection as designed.

### App Integration (Slice 5) -- Partially Implemented

- **Generational load tracking:** Implemented. `search_generation: u64` in `App`, incremented before each dispatch, stale results silently dropped via `g != self.search_generation` guard. Also incremented on `SearchClear`.

- **Message enum:** All specified variants present: `SearchQueryChanged`, `SearchExecute`, `SearchResultsLoaded(u64, ...)`, `SearchClear`, `FocusSearchBar`, `SearchBlur`.

- **Debounce subscription:** Implemented with `search_debounce_deadline: Option<iced::time::Instant>` and 50ms polling timer, matching the spec's V1 timer strategy.

- **ThreadListMode:** Implemented as `enum ThreadListMode { Folder, Search }` on `ThreadList`.

- **Search bar widget:** Real `text_input` in `thread_list_header` with `SearchInput`/`SearchSubmit` messages, mapped through `ThreadListEvent` to `App` messages.

- **Component trait:** `ThreadList` implements `Component` trait (as do `Sidebar`, `ReadingPane`, `StatusBar`, `Settings`, `AddAccountWizard`). The search bar is part of the `ThreadList` component, not a separate component -- this matches the app-integration-spec which says "It is not a separate Component."

- **Keyboard shortcuts:** `/` to focus and `Escape` to clear are implemented via event listeners in the subscription.

### Pinned Searches -- Substantially Implemented

- **PinnedSearch type:** `crates/app/src/db/pinned_searches.rs` has the `PinnedSearch` struct (without `thread_ids` field -- loaded lazily as spec allows).

- **CRUD functions:** All specified: `create_or_update_pinned_search`, `update_pinned_search`, `delete_pinned_search`, `list_pinned_searches`, `get_pinned_search_thread_ids`, `get_threads_by_ids`, `expire_stale_pinned_searches`. Uses transactions for atomicity.

- **Sidebar rendering:** `pinned_searches_section` and `pinned_search_card` in `sidebar.rs`. Has `ButtonClass::PinnedSearch { active }` style in the theme.

- **Lifecycle state machine:** `active_pinned_search`, `editing_pinned_search` state in `App`. Edit-in-place updates existing pinned search; new searches create new entries. Navigation away clears pinned search context.

- **Generational tracking for pinned search loads:** Uses `nav_generation` for `PinnedSearchThreadIdsLoaded` and `PinnedSearchThreadsLoaded` staleness checks.

- **Auto-expiry:** `expire_stale_pinned_searches(1_209_600)` (14 days) called after initial load.

---

## What Diverges from the Spec

### Critical: execute_search is a Stub

The app's `execute_search()` function (`crates/app/src/main.rs` ~line 2776) does **not** call the unified search pipeline. It is a raw SQL `LIKE` query against `threads.subject` and `threads.snippet`:

```rust
// Stub: use the unified search pipeline if SearchState is available.
// For now, do a simple SQL LIKE search as a placeholder so the full
// message flow, debounce, and generational tracking are exercised.
```

This means:
- Tantivy ranking is never used for search
- Structured operators (`from:`, `is:`, `has:`, `label:`, etc.) are not parsed or applied
- The entire query parser and SQL builder are unused during app search
- Cross-account filtering via `account:` is not available
- Contact expansion is not available
- `SearchState` is never initialized in the app

The app comment explicitly says "TODO: Wire real SearchState once it is initialized at app startup."

### Smart Folder Migration (Slice 6) -- Not Started

- `execute_smart_folder_query` in `crates/smart-folder/src/lib.rs` still uses its own direct path (parse -> SQL builder -> execute). It has **not** been migrated to call `search()` from the unified pipeline.
- The app does not import or reference `execute_smart_folder_query` or `search_pipeline` anywhere in `crates/app/src/main.rs`.
- Smart folders in the sidebar appear to be rendered statically from navigation state; there is no evidence of smart folder query execution through the unified pipeline.
- The token system (`__LAST_7_DAYS__`, etc.) is still active in `tokens.rs` and called by `execute_smart_folder_query`. No migration to offset syntax has been performed.

### SearchParams.label is Dead

`SearchParams.label: Option<String>` in `crates/search/src/lib.rs` is annotated `#[allow(dead_code)]` and has a comment "Label filter -- not handled in tantivy; caller must post-filter." The field exists but is never used by Tantivy internally. The unified pipeline passes `parsed.label.first().cloned()` into it, which has no effect.

### Tantivy-only path: from/to only uses first value

In `build_tantivy_params()`, `from` and `to` are set from `parsed.from.first().cloned()` and `parsed.to.first().cloned()`. This discards additional `from:` or `to:` values, breaking the OR semantics spec when the Tantivy-only path is taken. The SQL builder handles multi-value correctly.

### UnifiedSearchResult vs SearchResult naming

The problem statement spec defines `SearchResult`. The implementation names it `UnifiedSearchResult`. The app-integration-spec acknowledges four result types in play (`UnifiedSearchResult`, `Thread`, `DbThread`, `SearchResult`) but the Tantivy crate's `SearchResult` and the core's `UnifiedSearchResult` are separate types. This is noted as a known seam in the spec but remains unresolved.

### PinnedSearch struct diverges from spec

The pinned-searches spec defines `PinnedSearch` with a `thread_ids: Vec<(String, String)>` field. The implementation omits this field from the struct entirely, loading thread IDs via a separate `get_pinned_search_thread_ids()` call. This is a deliberate design choice (lazy loading) but diverges from the spec's data model.

### delete_all_pinned_searches not implemented

The pinned-searches-implementation-spec specifies a `delete_all_pinned_searches` function. This function does not exist in the codebase. The "Clear all" action described in the product spec is not available.

---

## What's Missing (Not Yet Built)

### Operator Typeahead (Phase 3 of app-integration-spec)

No typeahead popup implementation exists. No contact lookup for `from:`/`to:`, no account-scoped `label:`/`folder:` suggestions, no date presets for `before:`/`after:`. The search bar is a plain `text_input` without any popup or overlay.

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

**Yes, for search execution.** The `execute_search` stub in `main.rs` writes raw SQL directly (`SELECT t.* FROM threads t WHERE t.subject LIKE ?1 ...`) rather than calling the unified search pipeline in core. This is the single largest gap.

**No, for pinned searches.** Pinned search CRUD is in `crates/app/src/db/pinned_searches.rs`, which is app-level code but uses proper parameterized queries and transactions. The spec itself places pinned search CRUD in the app's `db.rs`, so this is intentional -- pinned searches are local UI state, not core domain logic.

### g. Dead Code

- `SearchState.index` and `SearchState.schema` fields are `#[allow(dead_code)]` in `crates/search/src/lib.rs`.
- `SearchParams.label` is `#[allow(dead_code)]` -- unused by Tantivy, passed but ignored.
- `SearchState::search()` (the simple free-text method) is `#[allow(dead_code)]` -- only `search_with_filters()` is used by the pipeline.
- `group_by_thread()` is public in both `crates/search/src/lib.rs` and duplicated (private) in `crates/core/src/search_pipeline.rs`. The core version converts `TantivyResult` -> `UnifiedSearchResult` while grouping; the search crate version works with `SearchResult` directly. Both exist and neither calls the other.
- `resolve_query_tokens` in `crates/smart-folder/src/tokens.rs` is still active but should be deprecated per the spec once the parser handles relative offsets natively (which it does). The function is still called by `execute_smart_folder_query` and `count_smart_folder_unread`.
