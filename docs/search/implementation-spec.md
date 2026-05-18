# Search: Backend Implementation Spec

Implementation plan for unifying the search backend per `docs/search/problem-statement.md`. Work spans three crates: `crates/search/` (Tantivy full-text), `crates/smart-folder/` (operator-based SQL queries), and `crates/core/` (unified pipeline, DB queries, types).

## Current State (2026-05-18)

Slices 1-5 are landed. Slice 6 is split: the iced app routes smart-folder selection through the unified pipeline at the dispatch layer, but the legacy `execute_smart_folder_query` facade in `crates/smart-folder/src/lib.rs` is still SQL-only and is no longer on the reachable app path.

- **Tantivy** (`crates/search/src/lib.rs`) - full-text ranked search. Cross-account (multi-account filter via `BooleanQuery`). Returns message-level results with `group_by_thread()` helper.
- **Smart folder SQL** (`crates/smart-folder/src/sql_builder.rs`) - operator-based SQL queries via `query_threads_read()` / `count_matching_read()`. Cross-account via `AccountScope`. Returns thread-level results. Supports all operators below.
- **Unified pipeline** (`crates/core/src/search_pipeline.rs`) - routes queries through SQL, Tantivy, or both based on parsed content. Entry point: `search(query, search_state, conn, scope, body_read) -> Result<SearchResults, String>`, where `SearchResults` is the enum `FullIndex(Vec<UnifiedSearchResult>) | Degraded(Vec<UnifiedSearchResult>)`. `Degraded` is returned when `SearchReadState` is unavailable and the pipeline falls back to a SQL-only `LIKE` path - see "Known semantic issues" below.
- **App dispatch** (`crates/app/src/handlers/search.rs`) - `SearchIntent` (`AdHoc` / `SmartFolder` / `PinnedActivation` / `PinnedRefresh`) is resolved to a `ResolvedSearch` (`SearchExecution` + `SearchCompletionBehavior`) that drives both query execution and the side-effects on pinned-search persistence and folder-view restoration. All four entry points route through `search_pipeline::search()`.

## Target State

One function: `search(query: &str, search_state: &SearchState, db: &Connection) -> Result<Vec<SearchResult>, Error>`

Always cross-account. Users narrow via `account:` operators in the query string.

Three internal paths based on parsed query content:

1. **Operators only** -> SQL, date-sorted
2. **Free text only** -> Tantivy, relevance-ranked
3. **Both** -> SQL narrows candidates, Tantivy scores them

## Slice 1: Parser Overhaul

**Status: Complete.** `crates/smart-folder/src/parser.rs` rewritten. 40 parser tests.

### ParsedQuery changes

```rust
pub struct ParsedQuery {
    pub free_text: Option<String>,

    // Repeated operators = OR (Vec instead of Option)
    pub from: Vec<String>,
    pub to: Vec<String>,
    pub account: Vec<String>,
    pub label: Vec<String>,
    pub folder: Vec<String>,
    pub in_folder: Vec<String>,        // "in:" universal folder shorthands

    // Attachment filtering
    pub has_attachment: bool,          // has:attachment
    pub attachment_types: Vec<String>, // resolved MIME types from has:/type: operators
    pub has_contact: bool,             // has:contact (native, not MIME-based)

    // Flags (unchanged, single bool each)
    pub is_unread: Option<bool>,
    pub is_read: Option<bool>,
    pub is_starred: Option<bool>,
    pub is_snoozed: Option<bool>,
    pub is_pinned: Option<bool>,
    pub is_muted: Option<bool>,
    pub is_tagged: Option<bool>,       // any label applied

    // Date (unchanged)
    pub before: Option<i64>,
    pub after: Option<i64>,
}
```

Key changes from current:
- `Option<String>` -> `Vec<String>` for operators that support OR (from, to, account, label, folder, in)
- Remove `subject` (free text covers it via Tantivy)
- Remove `is_important` (not in the design doc)
- Add `account`, `folder`, `in_folder`, `attachment_types`, `has_contact`, `is_tagged`

### New operators to parse

| Operator | Parser action |
|----------|--------------|
| `account:` | Push to `account` vec |
| `folder:` | Push to `folder` vec |
| `in:` | Push to `in_folder` vec |
| `is:tagged` | Set `is_tagged = Some(true)` |
| `has:contact` | Set `has_contact = true` |
| `has:pdf` | Expand and push MIME types to `attachment_types` |
| `has:image` | Expand and push MIME types to `attachment_types` |
| `has:excel` | Expand and push MIME types to `attachment_types` |
| `has:word` | Expand and push MIME types to `attachment_types` |
| `has:powerpoint` | Expand and push MIME types to `attachment_types` |
| `has:spreadsheet` | Alias for `has:excel` |
| `has:document` | Expand `has:word` + `has:pdf` |
| `has:archive` | Expand and push MIME types |
| `has:video` | Push `video/*` pattern |
| `has:audio` | Push `audio/*` pattern |
| `has:calendar` | Expand and push MIME types |
| `type:` | Push raw MIME type/glob to `attachment_types` |

### has: expansion table

A static mapping in the parser:

```rust
const HAS_EXPANSIONS: &[(&str, &[&str])] = &[
    ("pdf", &["application/pdf"]),
    ("image", &["image/jpeg", "image/png", "image/gif", "image/webp", "image/svg+xml"]),
    ("excel", &[
        "application/vnd.ms-excel",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "application/vnd.oasis.opendocument.spreadsheet",
        "text/csv",
    ]),
    ("word", &[
        "application/msword",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "application/vnd.oasis.opendocument.text",
        "application/rtf",
    ]),
    ("powerpoint", &[
        "application/vnd.ms-powerpoint",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "application/vnd.oasis.opendocument.presentation",
    ]),
    ("spreadsheet", &[/* alias: same as excel */]),
    ("document", &[/* union of word + pdf */]),
    ("archive", &[
        "application/zip", "application/gzip", "application/x-tar",
        "application/x-7z-compressed", "application/x-rar-compressed",
    ]),
    ("video", &["video/*"]),
    ("audio", &["audio/*"]),
    ("calendar", &["text/calendar", "application/ics"]),
];
```

### Date parsing overhaul

Replace `parse_date_to_timestamp` with a function that handles:

| Input | Interpretation |
|-------|---------------|
| `-7` | 7 days before today |
| `-30` | 30 days before today |
| `0` | Today (start of day) |
| `2025` | January 1, 2025 |
| `202603` | March 1, 2026 |
| `20260311` | March 11, 2026 |
| `2026/03/11` | March 11, 2026 |
| `2026-03-11` | March 11, 2026 |
| `2026 03 11` | March 11, 2026 (greedy consumption) |

Detection logic:
1. Starts with `-` or is `0` -> relative offset, compute from today
2. Digits only -> count digits: 4=year, 6=year+month, 8=full date
3. Contains `/` or `-` -> split on separator, parse segments
4. Space-separated -> greedy: after consuming the first token, peek at next tokens; if they're 1-2 digit numbers, consume them as month/day

The greedy space consumption requires the parser to look ahead past whitespace, which changes the current `extract_value` (stops at whitespace). The date operator parser needs its own sub-lexer that can consume multiple whitespace-separated tokens.

### Remove subject: and is:important

- Remove `subject` from the operator matching list
- Remove `is_important` from the flag matching list
- Remove corresponding fields from `ParsedQuery`

### Token system deprecation

The `__LAST_7_DAYS__` / `__LAST_30_DAYS__` / `__TODAY__` token system in `crates/smart-folder/src/tokens.rs` becomes unnecessary once the parser handles relative offsets natively. Steps:

1. Add relative offset support to the parser (this slice)
2. Migrate any persisted smart folder queries that use tokens to offset syntax (DB migration or on-read translation)
3. Keep `resolve_query_tokens` as a backward-compatibility shim until migration is confirmed complete
4. Remove `tokens.rs` once no queries use the old format

## Slice 2: SQL Builder Expansion

**Status: Complete.** All new clause builders implemented. 13 integration tests with in-memory SQLite.

### New clause builders

**`account:` operator:**
- Match by account `display_name` or `email` (not a `name` column - that doesn't exist). The `DbAccount` struct has `display_name: Option<String>` and `email: String`. The SQL: `JOIN accounts a ON m.account_id = a.id WHERE (a.display_name LIKE ? OR a.email LIKE ?)`
- OR semantics for multiple: `(a.display_name LIKE ?1 OR a.email LIKE ?1) OR (a.display_name LIKE ?2 OR a.email LIKE ?2)`
- Resolve matched account IDs early, then use ID-based filtering downstream (more efficient than repeated joins). When `account:` operators are present, they override any scope parameter.

**`folder:` operator:**
- Match by folder/mailbox name or path through the folder aggregate: `EXISTS (SELECT 1 FROM thread_folders tf JOIN folders f ON tf.folder_id = f.id AND tf.account_id = f.account_id WHERE tf.thread_id = t.id AND f.name LIKE ?)`
- For hierarchical paths (`folder:"Projects/Q2"`): IMAP folders use `folders.imap_folder_path`. Other providers can match display name today; a provider-neutral path column can be added later if folder-path search needs to be exact across Graph and JMAP.
- OR semantics for multiple folder values.

**`in:` operator (universal folder shorthands):**
- Map shorthands to provider-agnostic predicates. System folders are identified via canonical folder IDs in `SYSTEM_FOLDER_ROLES`, which maps well-known folder IDs (e.g., `"INBOX"`, `"SENT"`, `"DRAFT"`, `"TRASH"`, `"SPAM"`) across providers. The SQL builder should match against these folder IDs via `thread_folders`, not a role column:

| Shorthand | Predicate |
|-----------|-----------|
| `in:inbox` | `tf.folder_id = 'INBOX'` (via thread_folders join) |
| `in:sent` | `tf.folder_id = 'SENT'` |
| `in:drafts` | `tf.folder_id = 'DRAFT'` |
| `in:trash` | `tf.folder_id = 'TRASH'` |
| `in:spam` | `tf.folder_id = 'SPAM'` |
| `in:starred` | `t.is_starred = 1` (thread flag, not label join) |
| `in:snoozed` | `t.is_snoozed = 1` (thread flag, not label join) |

- Starred and snoozed are thread flags, not label joins. The builder must handle the mapping.

**`is:tagged` operator:**
- Matches threads that render at least one explicit label group, either through `thread_label_groups` or through `thread_labels` joined to `label_group_members`. See `label_group_rendered_fragment` in `crates/smart-folder/src/sql_builder.rs` for the canonical SQL shape - both operators below share it.

**`label:` operator:**
- Resolves to a row in `label_groups` by case-insensitive name. `label_groups` has no `account_id` column - the binding is workspace-wide. The shape is the same as `is:tagged` plus a `LOWER(lg.name) = LOWER(?N)` predicate on the group join.
- SQL (via the shared rendering-paths helper):
  ```sql
  EXISTS (SELECT 1 FROM thread_label_groups tlg
    JOIN label_groups lg ON lg.id = tlg.group_id
    WHERE tlg.account_id = m.account_id
      AND tlg.thread_id = m.thread_id
      AND LOWER(lg.name) = LOWER(?N))
  OR EXISTS (SELECT 1 FROM thread_labels tl
    JOIN label_group_members lgm
      ON lgm.account_id = tl.account_id AND lgm.label_id = tl.label_id
    JOIN label_groups lg ON lg.id = lgm.group_id
    WHERE tl.account_id = m.account_id
      AND tl.thread_id = m.thread_id
      AND LOWER(lg.name) = LOWER(?N))
  ```
- OR semantics for multiple `label:` values: parts joined with `OR`. The binding is by name, not group_id, so a group rename changes which group a persisted query resolves to. Stable group-id binding for persisted queries is tracked in `TODO.md`.
- Threads carrying raw labels that are not members of any group stop matching. Raw `(account_id, label_id)` membership has no user-facing operator post-split.

**`has:contact` operator:**
- `EXISTS (SELECT 1 FROM contacts WHERE email = m.from_address)` for sender
- Optionally also check recipient addresses - TBD whether `has:contact` means "sender is a contact" or "any participant is a contact"

**`type:` / attachment MIME filtering:**
- `EXISTS (SELECT 1 FROM attachments WHERE message_id = m.id AND mime_type LIKE ?)`
- For glob patterns (`video/*`): `mime_type LIKE 'video/%'`
- For exact types: `mime_type = ?`
- OR semantics: multiple types from `has:` expansion become `(mime_type LIKE ? OR mime_type LIKE ? OR ...)`
- Prerequisite: verify the `attachments` table has a `mime_type` column. If not, add via migration.

**Contact expansion for `from:` / `to:`:**
- Current: `(m.from_address LIKE ? OR m.from_name LIKE ?)`
- New: `(m.from_address LIKE ? OR m.from_name LIKE ? OR m.from_address IN (SELECT email FROM contacts WHERE email MATCH ? OR display_name MATCH ?))`
- Uses `contacts_fts` for the expansion subquery (already exists as FTS5 index)

### OR semantics

All `Vec<String>` fields generate OR-grouped clauses:

```sql
-- from:alice from:bob
(
    (m.from_address LIKE '%alice%' OR m.from_name LIKE '%alice%' OR m.from_address IN (...contacts...))
    OR
    (m.from_address LIKE '%bob%' OR m.from_name LIKE '%bob%' OR m.from_address IN (...contacts...))
)
```

Different operators remain AND:

```sql
-- from:alice has:pdf
(m.from_address LIKE '%alice%' OR m.from_name LIKE '%alice%' OR ...)
AND
EXISTS (SELECT 1 FROM attachments WHERE ... AND mime_type = 'application/pdf')
```

### Result shape

The SQL builder already returns `Vec<DbThread>` (thread-level). This is correct for the operators-only path and for generating candidate IDs for the Tantivy path.

## Slice 3: Tantivy Cross-Account Support

**Status: Complete.** `SearchParams.account_ids: Option<Vec<String>>`, `group_by_thread()` helper. 9 tests.

### SearchParams changes

The existing `SearchParams` struct is an internal detail - the unified API takes a raw query string. But Tantivy still needs parameters internally:

- Change `account_id: String` to `account_ids: Option<Vec<String>>` - `None` means all accounts
- In `search_with_filters`, replace the single `TermQuery` on account_id with:
  - `None` -> no account filter (search all)
  - `Some(ids)` -> `BooleanQuery` with `Should` clauses for each account ID

### UnifiedSearchResult

The implemented type in `crates/core/src/search_pipeline.rs`:

```rust
pub struct UnifiedSearchResult {
    pub thread_id: String,
    pub account_id: String,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub from_name: Option<String>,
    pub from_address: Option<String>,
    pub date: Option<i64>,
    pub is_read: bool,
    pub is_starred: bool,
    pub message_count: Option<i64>,
    pub rank: f32,                          // BM25, or 0.0 for SQL-only
    pub match_kind: Option<MatchKind>,      // Phase 7-8 attribution
    pub also_matched: Vec<MatchKind>,       // secondary fields above 50% of top score
}
```

`match_kind` and `also_matched` are the Phase 7-8 attribution outputs: the pipeline knows whether the hit came from subject, from-name, body, or snippet, and surfaces secondary fields the result also matched. The Tantivy paths optionally re-read body text from the body store (`body_read: Option<&BodyStoreReadState>`) to refine attribution.

`has_attachments` is intentionally absent from `UnifiedSearchResult` and is tracked as an open issue (see "Known semantic issues" below) - the thread card cannot display the attachment indicator from search results today.

For the Tantivy-only path: query returns message-level hits, group by `thread_id`, take the highest score per thread, enrich with thread metadata from SQLite.

For the SQL-narrowed-Tantivy path: SQL provides the thread metadata, Tantivy provides the score.

## Slice 4: Unified Pipeline

**Status: Complete.** `crates/core/src/search_pipeline.rs` with 3-path router and `UnifiedSearchResult` type. 14 tests.

### The router

```rust
pub fn search(
    query: &str,
    search_state: &SearchState,
    db: &Connection,
) -> Result<Vec<SearchResult>, Error> {
    let parsed = parse_query(query);

    let has_free_text = parsed.free_text.is_some();
    let has_operators = parsed.has_any_operator();

    match (has_free_text, has_operators) {
        (false, false) => Ok(vec![]),  // empty query
        (false, true) => search_sql_only(&parsed, db),
        (true, false) => search_tantivy_only(&parsed, search_state),
        (true, true) => search_combined(&parsed, search_state, db),
    }
}
```

No `scope` parameter - search is always cross-account. Account narrowing is done via `account:` operators in the query string, resolved to account IDs during parsing.

### Path 1: SQL only (operators, no free text)

1. Build SQL from parsed operators (slice 2's SQL builder)
2. Execute against SQLite
3. Return `Vec<SearchResult>` with `rank: 0.0`, sorted by date descending

### Path 2: Tantivy only (free text, no operators)

1. Build Tantivy query from free text
2. Apply account scope as Tantivy filter (slice 3)
3. Collect message-level hits with scores
4. Group by thread_id, take max score per thread
5. Enrich with thread metadata from SQLite (subject, snippet, is_read, is_starred, message_count)
6. Return sorted by rank descending

### Path 3: Combined (both operators and free text)

1. SQL builder generates candidate thread IDs from operators
2. Tantivy searches free text across all indexed messages
3. Intersect: keep only Tantivy hits whose thread_id is in the SQL candidate set
4. Group by thread_id, take max score per thread
5. Enrich with thread metadata from SQL results (already fetched in step 1)
6. Return sorted by rank descending

The intersection is done in application code - collect SQL thread IDs into a `HashSet`, filter Tantivy results against it. This is simple and fast for typical result sizes.

### Account scope resolution

Search is always cross-account. Account narrowing is controlled entirely by `account:` operators in the query:

- If `account:` operators are present, resolve account display names / emails to account IDs and filter both engines to those accounts
- If no `account:` operators, search all accounts
- Resolution happens during parsing, before either engine is invoked

## Slice 5: App Integration

**Status: Landed.** Details live in `docs/search/app-integration-spec.md`. Summary of the implemented shape:

- The iced app calls `search_pipeline::search()` directly from `crates/app/src/handlers/search.rs`.
- Dispatch goes through `SearchIntent` (`AdHoc` / `SmartFolder` / `PinnedActivation` / `PinnedRefresh`) → `resolve_search_intent` → `ResolvedSearch`, which carries both the execution (Query vs Snapshot) and the completion behavior (pinned-search persistence, folder-restore policy, post-success effects).
- Generational tracking uses branded `GenerationCounter<Search>` / `GenerationCounter<Nav>` tokens (`rtsk::generation`) instead of a raw `u64`. The `SearchFreshness` enum routes which token gates a given dispatch (queries use `Search`; snapshot activations use `Nav`).
- `SearchReadState` is initialized once at boot and reused; the app holds it as `Option<Arc<SearchReadState>>`.

The scope parameter is real and supplied by the dispatcher (always one of `ViewScope::AllAccounts` / `Account` for ad-hoc; `QueryIntrinsic` for smart folders that encode scope in their query string). Search is still cross-account by default - `ViewScope::AllAccounts` is the default for ad-hoc searches.

## Slice 6: Smart Folder Migration

**Status: Split.** Sidebar smart-folder selection in the iced app routes through `search_pipeline::search()` via `handle_smart_folder_selected` → `SearchIntent::SmartFolder` → `resolve_search_intent`. So in practice smart folders already get Tantivy ranking when their query has free text, all new operators, and contact expansion.

The legacy `execute_smart_folder_query` facade in `crates/smart-folder/src/lib.rs` is still SQL-only - it calls `query_threads_read()` after `migrate_legacy_tokens()` / `parse_query()`. The facade is not on the reachable app path today; cleanup is tracked under "Known semantic issues" below.

### Token migration

`migrate_legacy_tokens()` rewrites `__LAST_7_DAYS__` / `__LAST_30_DAYS__` / `__TODAY__` at parse time. A one-time DB migration that rewrites these on disk has not been done; see "Known semantic issues."

### Unread counts

`count_smart_folder_unread` remains SQL-only. Unread counts don't need Tantivy ranking, only a count of matching unread threads. `get_navigation_state()` returns scaffolded zeros for smart-folder unread counts today; wiring `count_smart_folder_unread` into the navigation-state computation is still pending.

## Prerequisites / Schema Changes

### Attachments table: `mime_type` column

**Already exists.** The `attachments` table has a `mime_type TEXT` column (see `crates/db/src/db/migrations.rs`, `DbAttachment.mime_type` in `crates/db/src/db/types.rs`). No migration needed for MIME-type filtering.

### Folders table: system folder identification

The `folders` table has no generic `role` column. System folders are identified by well-known canonical folder IDs (`"INBOX"`, `"SENT"`, `"DRAFT"`, `"TRASH"`, `"SPAM"`, etc.) defined in `SYSTEM_FOLDER_ROLES`. The `in:` operator's SQL builder matches against these IDs via `thread_folders.folder_id`, not a role column. Provider-specific folder metadata such as `imap_folder_path` and `imap_special_use` lives on `folders` and is available to the `folder:` operator.

## Dependency Graph

```
Slice 1 (parser)
  -> Slice 2 (SQL builder)
        -> Slice 4 (unified pipeline)
              -> Slice 5 (app integration - trivial wiring)
              -> Slice 6 (smart folder migration)

Slice 3 (Tantivy cross-account)
  -> Slice 4 (unified pipeline)
```

Slices 1-4 are complete. Slice 5 is trivial wiring. Slice 6 depends on 4.

## Ecosystem Patterns

Patterns from the [iced ecosystem survey](../iced-ecosystem-survey.md) that apply to the search pipeline. The backend slices (1-4) are largely framework-agnostic, so the survey's value concentrates on Slice 5 (app integration) and Slice 6 (smart folder migration).

| Slice | Pattern (Source) | How It Applies |
|---|---|---|
| Slice 4 (3-way router) | Enum dispatch (raffi `route_query()`) | Validates the `(has_free_text, has_operators)` match approach; consider a `SearchMode` enum if routing modes grow beyond 3 |
| Slice 5 (app integration) | Generational load tracking (bloom) | **Critical**: Add a `search_generation: u64` counter to the app state. Increment on every keystroke or search submission. Tag each search `Task` with its generation and discard results whose generation is stale. Without this, incremental typing produces flickering or wrong results. |
| Slice 5 (app integration) | Subscription orchestration (pikeru) | Consider parallelizing SQL and Tantivy queries in the combined path using `subscription::channel` for off-main-thread execution |
| Slice 5 (results display) | Data table patterns (shadcn-rs) | Sort/filter patterns for the search result list; dual sorting (relevance vs date) maps to shadcn-rs column sort model |
| Slice 6 (smart folders) | Module trait (Lumin) | If search backends proliferate beyond SQL+Tantivy, formalize with a trait registry rather than hardcoded match arms |

### Most impactful finding

Bloom's **generational load tracking** is the single most impactful pattern for this spec. The implementation spec treats Slice 5 app integration as "trivial wiring," but without stale-result cancellation the search UX will break during incremental typing. The implemented form uses branded `GenerationCounter<T>` / `GenerationToken<T>` (`rtsk::generation`) rather than raw `u64`, with phantom-type brands preventing cross-counter comparison. The same pattern is used across calendar, main layout, sidebar, command palette, pinned searches, status bar, and contacts.

---

## Known semantic issues

Open items as of 2026-05-18. UI-side items live in `app-integration-spec.md`; pinned-search items live in `pinned-searches-implementation-spec.md`.

### High

1. **Combined path applies free text in SQL before Tantivy ranking.** `search_combined` passes the full parsed query into `query_threads_read()`, which always includes `build_free_text_clause()`. Mixed queries are constrained by a SQL `LIKE` candidate set before ranking, defeating the "SQL narrows by structured operators; Tantivy ranks free text" intent.
2. **Combined path does a broad Tantivy search, then intersects in application code.** Works correctly but does not implement the "SQL narrows corpus first" performance model.
3. **Tantivy-only thread cards can show best-matching-message metadata instead of latest-message metadata.** The product spec says thread cards always show the latest message; ranking should only affect order. The Tantivy-only path groups by highest-scoring message per thread and uses that message's subject/snippet/sender. Only the combined path re-enriches from `DbThread`.
4. **Date boundary semantics differ across engines.** SQL uses strict `<` / `>` for `before:` / `after:`. Tantivy uses inclusive bounds. The same query can include boundary-day messages in one path and exclude them in another.
5. **`folder:` is still fuzzy substring matching, not true folder-path semantics.** Current SQL lowers `folder:` to `%LIKE%` against `folders.name` or `imap_folder_path`. The spec calls for path-aware folder matching with cross-provider normalization.
6. **`has_attachments` is missing from `UnifiedSearchResult`.** Search results cannot show the attachment indicator on the thread card. Fix: extend the struct, populate from `DbThread.has_attachments` in SQL paths, default to `false` in the Tantivy-only path.
7. **SQL fallback search is a real semantics downgrade.** When `SearchReadState` is unavailable, free-text search falls back to a thread-level `LIKE` on subject/snippet. The pipeline marks the result set `SearchResults::Degraded` so callers can warn, but the contract is not the same.

### Medium

8. **`label:` matching is not normalized to the cross-account label model.** Current predicate is `LOWER(lg.name) = LOWER(?)`. There is no trimming or alignment with the normalized-name grouping behavior described in `reference/glossary/folders-labels.md`.
9. **`to:` semantics are incomplete.** SQL checks only `to_addresses` and `cc_addresses`. No `bcc_addresses` coverage. No contact expansion. (Note: `from:` *does* expand via `contacts_fts` - the asymmetry is unintentional.)
10. **`has:contact` is sender-only.** Implementation checks only `m.from_address IN (SELECT email FROM contacts)`. The product surface implies "any known participant"; sender-vs-any-participant remains unresolved.
11. **Free-text Tantivy search does not cover all indexed address fields.** The index stores `from_address` and `to_addresses`, but the free-text query parser searches only `subject`, `from_name`, `body_text`, and `snippet`.
12. **Legacy `execute_smart_folder_query` facade is still SQL-only.** The reachable app path uses the unified pipeline; the facade is leftover. Either delete it or convert it to a thin wrapper around the unified pipeline.
13. **Legacy date-token migration is still runtime, not a one-time DB migration.** `migrate_legacy_tokens()` rewrites `__LAST_7_DAYS__`-style tokens at execution time. A persisted SQL migration was specified but never written.
14. **Smart-folder unread counts are scaffolded as 0.** `get_navigation_state()` returns 0 for every smart folder's unread count. Wiring `count_smart_folder_unread` into navigation-state computation is still pending (with batching, to avoid N+1 queries per sidebar refresh).
15. **Result limits are fixed and engine-specific.** Combined search uses one SQL candidate limit, Tantivy uses its own, SQL fallback uses another. Broad searches can truncate in engine-specific ways before paging / refinement exists.

### Low

16. **SQL builder relies heavily on `%LIKE%` scans.** Primarily a performance/scale risk for large local stores; tracked as a known limit, not a correctness bug.
17. **`in:` accepts undocumented shorthands.** `archive` and `important` are matched in code but absent from the documented operator surface in `problem-statement.md`. Decide: extend the docs or drop the shorthands.

## Stale spec content (kept here so it doesn't drift back)

These items appeared in earlier audit notes as "spec says X, code does Y." The current code is the better design; the spec wording has been updated to match:

- Generational tracking uses branded `GenerationCounter<T>` / `GenerationToken<T>` rather than a raw `u64`.
- The app's search query state is `UndoableText`, not a bare `String`.
- The async bridge uses `db.with_conn()` / `tokio::task::spawn_blocking`; the pseudo-code in older drafts is illustrative, not normative.
- Folder-view restoration re-queries from DB rather than restoring a cloned thread list (handled at the dispatch layer via `FolderRestoreBehavior`).
