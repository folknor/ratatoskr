# Search: Spec vs. Code Discrepancies

Audit date: 2026-03-22

---

## Resolved (previously open)

- SearchBlur now focuses blur-sink to unfocus search bar (was a no-op)
- UnifiedSearchResult naming consolidated (Tantivy result aliased as TantivyResult)
- PinnedSearch.thread_ids field now exists (marked dead_code, lazy-loaded by design)
- Smart folders now execute via unified search pipeline (sidebar click → execute_search)
- Auto-expiry now runs hourly via iced::time::every subscription (was startup-only)
- Operator typeahead implemented (static + async DB queries for all operator types)
- "Search here" interaction implemented (right-click on sidebar folders/labels)
- "Save as Smart Folder" implemented (command palette command)
- Pinned search staleness label implemented ("outdated" + relative time)
- Search result highlighting implemented (matching messages expanded, terms highlighted in body)

## Remaining

### SearchState initialized per-search
`SearchState::init()` called inside `execute_search()` on every search dispatch rather than stored on App. Works because `init()` opens an existing index, but adds per-search overhead. Minor performance concern, not a correctness issue.

### Search history frontend
Backend ready (last 10 queries from pinned_searches, loaded on boot). Frontend not yet wired — up-arrow in empty search bar to browse history.

## Not a discrepancy

### delete_all_pinned_searches not in UI
Handler exists but no UI dispatches it. Pinned searches auto-expire and can be dismissed individually. No "Clear all" action needed.

### Smart folder form editor in settings
Settings UI shows "Coming soon" placeholder. The form-based editor was never built and is superseded by the command palette approach (Save as Smart Folder). Not a gap.
