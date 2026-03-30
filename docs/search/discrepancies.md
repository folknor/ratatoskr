# Search: Spec vs. Code Discrepancies

Audit date: 2026-03-30 (consolidated from 3 Opus agents + 5-wave outside review)

---

## Pipeline Semantics (High)

1. **Combined path applies free text in SQL, breaking the documented model.** Spec says SQL narrows by operators, Tantivy ranks free text. `search_combined()` passes the full parsed query into `query_threads()` which always includes `build_free_text_clause()`. Mixed queries are intersected against a SQL LIKE candidate set, so valid Tantivy hits can be dropped before ranking.

2. **Combined path does full Tantivy search then intersects — no corpus narrowing.** `search_combined()` runs Tantivy broadly and filters in application code. Does not implement the spec's "SQL narrows corpus first" performance model. Gets more expensive as mailbox size grows.

3. **Tantivy-only results show best-match message, not thread's latest.** Spec says search cards show latest-message metadata. Tantivy path groups by thread keeping the highest-ranked message. Only the combined SQL path re-enriches from DbThread. Free-text-only results can show wrong sender/snippet/subject.

4. **Tantivy account name-to-ID resolution broken.** `account:` operator values (display names like "Work") are passed directly as `account_ids` to Tantivy, which indexes UUIDs. `account:Work` in a free-text-only query will never match.

5. **Date boundary semantics inconsistent across engines.** SQL uses strict `<` / `>`. Tantivy uses inclusive range bounds. Same query can include boundary-day messages in one engine and exclude them in the other.

## Parser / Operator Semantics (High)

6. **Unrecognized `is:` and `has:` values silently dropped.** Spec says anything not recognized as an operator is free text. Parser recognizes `is:`/`has:` as operators first, then drops unknown values in `apply_is_operator()`/`apply_has_operator()`. `is:important` is consumed, no flag set, no free text remains.

7. **`is:tagged` includes universal folders (INBOX, etc.).** Implementation checks for any `thread_labels` row. Test explicitly treats INBOX membership as tagged. Much broader than user expectation — most normal inbox mail is "tagged."

8. **`folder:` semantics are fuzzy substring, not exact folder-path.** Spec calls for path-aware, cross-provider normalized matching. Implementation does `%LIKE%` against `l.name` or `l.imap_folder_path`. No normalized `folder_path` column. Only works for IMAP.

9. **`label:` matching not normalized.** Uses `LOWER(l.name) = LOWER(?)` — no trim, no alignment with labels-unification grouped-name normalization.

## Operator Gaps (Medium)

10. **`to:` has no contact expansion.** Only does LIKE against `m.to_addresses` and `m.cc_addresses`. No contact lookup, no BCC coverage.

11. **`from:` contact expansion uses LIKE, not FTS5.** Spec calls for `contacts_fts` MATCH. Implementation uses `contacts.display_name LIKE`. Doesn't search contact emails in subquery.

12. **`has:contact` is sender-only.** Spec frames it as "any sender/recipient exists as a stored contact." Implementation checks only `m.from_address`.

13. **Free-text Tantivy search doesn't cover all indexed address fields.** Indexes `to_addresses` and `from_address` but free-text query parser only searches `subject`, `from_name`, `body_text`, `snippet`.

14. **`in:` operator has undocumented shorthands.** Code supports `archive` and `important` not in the spec. Not a bug but spec/code drift.

## App Integration (Medium)

15. **`has_attachments` always false in search results.** `UnifiedSearchResult` has no such field. `unified_result_to_thread()` hardcodes `false`. Search results never show attachment indicator.

16. **SmartFolder Update/Delete/Rename commands missing.** Only `SmartFolderSave` exists. No UI for editing or deleting smart folders.

17. **Search history UI not wired.** `search_history` loaded on boot, never rendered. No up-arrow interaction.

18. **Smart folder execution not routed through unified pipeline.** `execute_smart_folder_query` goes directly to SQL builder. `count_smart_folder_unread` gets SQL-only behavior without Tantivy ranking.

19. **Token migration DB migration not done.** Runtime `migrate_legacy_tokens()` shim instead of one-time migration.

20. **SQL fallback is a material behavior downgrade.** If SearchState absent, free text collapses to `threads.subject LIKE ? OR threads.snippet LIKE ?`. Worse recall, no ranking, different semantics.

## Typeahead (Medium)

21. **No "keep as text" fallback option.** Spec says last option is always raw text. Current typeahead only returns matched items.

22. **`label:` / `folder:` typeahead not scoped by `account:`.** Both dispatch to same `search_labels_for_typeahead()` with raw string. No account filtering, no folder-path source, no folder/label separation.

23. **Contact typeahead uses wrong source.** Spec says `contacts_fts`. Implementation queries `seen_addresses` with LIKE. Weaker dataset.

24. **Date typeahead missing "Pick a date" path.** Only preset list.

25. **Missing typeahead items:** `is:tagged`, `has:powerpoint`, `has:spreadsheet`, `has:calendar`, `has:contact`.

## Pinned Searches (Medium)

26. **Graduation does not remove the pinned search.** `handle_save_as_smart_folder` creates smart folder but pinned search survives. Spec requires deletion. `GraduatePinnedSearch` message never created.

27. **No staleness label near search bar.** Sidebar card shows relative time, but spec's "Last updated X ago" near the search bar not implemented.

28. **"Clear All Pinned Searches" has no UI path.** Handler and DB function exist. No CommandId in palette, no sidebar button.

## Performance (Medium)

29. **SQL builder relies heavily on `%LIKE%` scans.** `from:`, `to:`, `account:`, `folder:`, and free-text fallback all use substring patterns. Degrades under large local stores.

30. **Result limits are fixed and engine-specific.** Combined uses `DEFAULT_QUERY_LIMIT` for SQL, Tantivy hardcodes 200, SQL fallback hardcodes 200. Broad searches can truncate in engine-specific ways before paging.

## Dead Code

31. **`tokens.rs` orphaned.** `crates/smart-folder/src/tokens.rs` not declared as module in `lib.rs`. Unreachable.

32. **`parse_date_string` in `search_pipeline.rs` dead.** Only used in tests, simple `i64` parsing. Leftover.

## Stale Spec Content (not bugs)

33. Generation tracking uses branded `GenerationCounter<Search>`, not raw `u64`.
34. `active_smart_folder_id` replaced by pinned search system.
35. `search_query` is wrapper type, not `String`.
36. Async bridge is `db.with_conn()`, not `tokio::spawn_blocking`.
37. Folder restore re-queries DB, not clone — explicitly better.

## Confirmed Limitations (not discrepancies)

38. No negation (`NOT`) or grouping (parentheses) — deferred in spec, matches code.
39. No pagination — limits are fixed per path.

## Resolved from previous audit (2026-03-22)

- SearchBlur focuses blur-sink correctly
- UnifiedSearchResult naming consolidated
- PinnedSearch.thread_ids field exists (lazy-loaded)
- Smart folders execute via unified pipeline from sidebar click
- Auto-expiry runs hourly via subscription
- Operator typeahead implemented (static + async DB)
- "Search here" interaction implemented
- "Save as Smart Folder" implemented
- Pinned search staleness in sidebar cards implemented
- Search result highlighting implemented
- SearchState per-search concern downgraded — Arc clone, not re-init
