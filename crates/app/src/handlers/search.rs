//! Search handler — placeholder for future extraction from `main.rs`.
//!
//! The search execution logic currently lives in `main.rs` as free functions
//! (`execute_search`, `execute_search_sql_fallback`, `unified_result_to_thread`)
//! and dispatch arms in `App::update()` for `Message::Search*` variants.
//!
//! When the handlers module is wired into the app, these functions should
//! be extracted here as `impl App` methods following the pattern described
//! in `UI.md` under "Handler modules".
//!
//! ## Current search flow
//!
//! 1. User types in search bar -> `SearchQueryChanged` updates `search_query`
//!    and sets a debounce deadline (150ms).
//! 2. Timer fires `SearchExecute` when deadline passes.
//! 3. `execute_search` runs off-thread via `Task::perform`:
//!    - Tries unified pipeline (Tantivy + SQL) via `search_pipeline::search()`
//!    - Falls back to SQL-only via `smart_folder::query_threads()` if no index
//!    - Falls back to LIKE search for pure free-text without operators
//! 4. Results arrive as `SearchResultsLoaded(generation, Result)`.
//!    Stale generations are silently dropped.
//! 5. Results are displayed and a pinned search snapshot is created/updated.
