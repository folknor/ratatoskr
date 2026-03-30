# Search: Spec vs. Code Discrepancies

Audit date: 2026-03-30

This file consolidates:
- the prior 2026-03-22 search audit,
- the current repo state,
- a deeper review of search semantics, parser/planner fidelity, app integration, pinned searches, and performance.

It aims to preserve earlier audit information while correcting a few places where the old notes had gone stale.

---

## Remaining Discrepancies

### High

1. **Combined path still applies free text in SQL before Tantivy ranking.**
   The product/spec model says SQL should narrow by structured operators while Tantivy handles free-text relevance. Current `search_combined()` passes the full parsed query into `query_threads()`, and `query_threads()` always includes `build_free_text_clause()`. Mixed queries are therefore constrained by a SQL `LIKE` candidate set before ranking.

2. **Combined path still does broad Tantivy search, then intersects in application code.**
   This works, but it does not implement the intended "SQL narrows corpus first" performance model. As mailbox sizes grow, the combined path will do more Tantivy work than the spec implies.

3. **Tantivy-only thread cards can show best-matching message metadata instead of latest-message metadata.**
   The product spec says thread cards should always show the latest message in the thread, with ranking only affecting order. Current Tantivy-only path groups by highest-scoring message and uses that message's subject/snippet/sender. Only the combined path re-enriches from `DbThread`.

4. **Date boundary semantics differ across engines.**
   SQL uses strict `<` / `>` for `before:` / `after:`. Tantivy uses inclusive bounds. The same query can therefore include boundary-day messages in one path and exclude them in another.

5. **Unknown `is:` / `has:` values are consumed and dropped instead of falling back to free text.**
   The query language says anything not recognized as an operator should behave like free text. Current parser behavior recognizes `is:` / `has:` syntactically, then drops unknown values during operator application. Example: `is:important` disappears entirely.

6. **`is:tagged` currently includes system-folder membership such as Inbox.**
   The implementation checks only whether any `thread_labels` row exists for the thread. In practice that means ordinary Inbox mail often counts as "tagged", which is much broader than normal user expectation.

7. **`folder:` semantics are still fuzzy substring matching, not true folder-path semantics.**
   The spec calls for path-aware folder matching with cross-provider normalization. Current SQL lowers `folder:` to `%LIKE%` against `labels.name` or `imap_folder_path`, with no provider-agnostic normalized `folder_path`.

### Medium

8. **`label:` matching is not normalized to the cross-account label model.**
   Current matching is `LOWER(l.name) = LOWER(?)`. There is no trimming or alignment with the normalized-name grouping behavior described elsewhere in the search and labels docs.

9. **`to:` semantics are incomplete.**
   Current SQL only checks `to_addresses` and `cc_addresses`. There is no contact expansion and no `bcc` coverage.

10. **`from:` contact expansion uses a weaker path than the spec describes.**
    The docs call for `contacts_fts`-style resolution. Current SQL uses `display_name LIKE` expansion only, not the richer FTS-backed contact lookup path.

11. **`has:contact` is sender-only.**
    Current implementation checks only whether `m.from_address` exists in contacts. The docs frame this operator more broadly around known participants, and the implementation-spec leaves sender-vs-any-participant unresolved.

12. **Free-text Tantivy search does not cover all indexed address fields.**
    The index stores `from_address` and `to_addresses`, but the free-text query parser searches only `subject`, `from_name`, `body_text`, and `snippet`.

13. **`has_attachments` is still missing from unified search results.**
    `UnifiedSearchResult` still lacks a `has_attachments` field, and the app currently maps search results to thread cards with `has_attachments: false`. Search results therefore do not show attachment indicators consistently.

14. **Smart-folder execution is only partially migrated.**
    The reachable app flow for sidebar smart-folder selection uses the unified search path, but the legacy `crates/smart-folder/src/lib.rs` facade still exists and still runs through the SQL-only path. `count_smart_folder_unread()` also remains SQL-only by design.

15. **Legacy smart-folder token migration is still runtime, not a one-time DB migration.**
    `migrate_legacy_tokens()` still rewrites old `__LAST_7_DAYS__`-style tokens at execution time. The implementation docs proposed an actual persisted migration.

16. **SQL fallback search is a real semantics downgrade.**
    If `SearchState` is unavailable, free-text search falls back to a simple thread-level `LIKE` search on subject/snippet. That preserves basic usability but not the full search contract.

17. **Pinned-search graduation does not remove the pinned search.**
    "Save as Smart Folder" creates the smart folder, but the pinned search remains. The pinned-search spec says promotion should remove the pinned search entry.

18. **Pinned-search freshness is not shown near the search bar.**
    Sidebar cards show relative age, but the pinned-search spec also calls for a subtle "Last updated ..." indicator near the search bar when a pinned search is active.

19. **Search history frontend is still missing.**
    Recent search queries are loaded from `pinned_searches`, but there is still no frontend interaction for browsing history from the search bar.

20. **Typeahead has no "keep as text" fallback item.**
    The product spec says the last suggestion should always let the user keep the raw input. Current typeahead only shows matched suggestions.

21. **`label:` / `folder:` typeahead is not scoped by existing `account:` filters.**
    Both operators currently route through the same unscoped label search. There is no account-aware narrowing and no real folder-path source.

22. **Contact typeahead uses a different source than the docs describe.**
    The docs say `from:` / `to:` typeahead should use `contacts_fts`. Current app code queries `seen_addresses` with `LIKE`.

23. **Date typeahead lacks the "Pick a date..." path.**
    Current implementation offers static presets only.

24. **Static typeahead coverage is incomplete.**
    Missing from the current static suggestion lists: `is:tagged`, `has:powerpoint`, `has:spreadsheet`, `has:calendar`, and `has:contact`.

25. **Result limits are still fixed and engine-specific.**
    Combined search uses one SQL candidate limit, Tantivy uses its own limit, and SQL fallback uses another hardcoded limit. Broad searches can truncate in engine-specific ways before paging/refinement exists.

26. **The SQL builder still relies heavily on `%LIKE%` scans.**
    This is primarily a performance/scale risk, but it is worth tracking as an implementation discrepancy because the docs set a very high responsiveness bar for large local stores.

27. **Undocumented `in:` shorthands exist in code.**
    The implementation currently supports `archive` and `important` in `in:` even though they are not part of the documented operator surface.

28. **Clear-all pinned-search support exists in code but not in the visible product surface.**
    There is a handler and DB operation for deleting all pinned searches. However, the pinned-search spec promises a clear-all affordance, and the current UI still does not expose one.

29. **Smart-folder management beyond "Save Search" is still incomplete.**
    The search docs describe rename/delete/update flows for smart folders. The current command surface still centers on save, and the broader management flow described in the docs is not yet fully present.

---

## Resolved Since The Previous Audit

These items were previously open and are now in place:

- `SearchBlur` now uses the blur-sink focus trick and is no longer a no-op.
- `UnifiedSearchResult` naming was cleaned up.
- `PinnedSearch.thread_ids` exists and is lazily loaded.
- Sidebar smart-folder clicks now execute through the unified search flow.
- Pinned-search auto-expiry runs on a periodic subscription rather than only at startup.
- Operator typeahead exists for static and async-backed operators.
- "Search here" is implemented for sidebar folders/labels.
- "Save as Smart Folder" exists as a real command.
- Pinned-search cards show relative staleness in the sidebar.
- Search-result highlighting exists in the reading pane.
- `SearchState` is initialized at app boot and reused; it is no longer re-initialized per search dispatch.

---

## Preserved Notes From The Earlier Audit

These do not all count as product discrepancies, but they are still useful context:

1. **Earlier audit position on clear-all pinned searches.**
   The previous audit treated missing clear-all UI as "not a discrepancy" because pinned searches auto-expire and can be dismissed individually. After re-reading `docs/search/pinned-searches.md`, this broader audit now treats it as a real spec mismatch because that doc explicitly promises a clear-all affordance.

2. **Form-based smart-folder editor.**
   The old settings-based smart-folder editor is still correctly treated as superseded. The current docs favor search-bar editing plus command-palette save/update flows, so the lack of a settings-form editor is not considered a gap.

---

## Stale Or Over-Specific Spec Content (Not Code Bugs)

These are cases where the docs describe a slightly different implementation shape than the current code, but the difference itself is not a user-visible bug:

- Generational tracking uses branded `GenerationCounter<T>` types rather than a raw `u64`.
- The app stores search query state in wrapper/state structs rather than a bare `String`.
- The async bridge is `db.with_conn()` / existing app task plumbing rather than the exact pseudo-code in the spec.
- Folder-view restoration re-queries from DB instead of restoring a cloned thread list, which is actually the better design.

---

## Confirmed Intentional Limitations

These are explicitly deferred in the product docs and therefore are not discrepancies:

- No explicit `AND`, `OR`, or `NOT` operators in V1.
- No grouping via parentheses in V1.
- No negation syntax in V1.
- No pagination yet; result limits are fixed.
