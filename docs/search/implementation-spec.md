# Search: Backend Implementation Spec

Implementation plan for unifying the search backend per `docs/search/problem-statement.md`. Work spans three crates: `crates/search/` (Tantivy full-text), `crates/smart-folder/` (operator-based SQL queries), and `crates/core/` (unified pipeline, DB queries, types).

## Current State

The unified search pipeline is implemented (Slices 1-4 complete). Entry point: `crates/core/src/search_pipeline.rs`.

- **Tantivy** (`crates/search/src/lib.rs`) — full-text ranked search. Cross-account (multi-account filter via `BooleanQuery`). Returns message-level results with `group_by_thread()` helper.
- **Smart folder SQL** (`crates/smart-folder/src/`) — operator-based SQL queries. Cross-account via `AccountScope`. Returns thread-level results. Supports all operators below.
- **Unified pipeline** (`crates/core/src/search_pipeline.rs`) — routes queries through SQL, Tantivy, or both based on parsed content.

## Target State

One function: `search(query: &str, search_state: &SearchState, db: &Connection) -> Result<Vec<SearchResult>, Error>`

Always cross-account. Users narrow via `account:` operators in the query string.

Three internal paths based on parsed query content:

1. **Operators only** → SQL, date-sorted
2. **Free text only** → Tantivy, relevance-ranked
3. **Both** → SQL narrows candidates, Tantivy scores them

## Slice 1: Parser Overhaul ✅

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
- `Option<String>` → `Vec<String>` for operators that support OR (from, to, account, label, folder, in)
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
1. Starts with `-` or is `0` → relative offset, compute from today
2. Digits only → count digits: 4=year, 6=year+month, 8=full date
3. Contains `/` or `-` → split on separator, parse segments
4. Space-separated → greedy: after consuming the first token, peek at next tokens; if they're 1-2 digit numbers, consume them as month/day

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

## Slice 2: SQL Builder Expansion ✅

**Status: Complete.** All new clause builders implemented. 13 integration tests with in-memory SQLite.

### New clause builders

**`account:` operator:**
- Match by account `display_name` or `email` (not a `name` column — that doesn't exist). The `DbAccount` struct has `display_name: Option<String>` and `email: String`. The SQL: `JOIN accounts a ON m.account_id = a.id WHERE (a.display_name LIKE ? OR a.email LIKE ?)`
- OR semantics for multiple: `(a.display_name LIKE ?1 OR a.email LIKE ?1) OR (a.display_name LIKE ?2 OR a.email LIKE ?2)`
- Resolve matched account IDs early, then use ID-based filtering downstream (more efficient than repeated joins). When `account:` operators are present, they override any scope parameter.

**`folder:` operator:**
- Match by folder/mailbox name or path: `EXISTS (SELECT 1 FROM thread_labels tl JOIN labels l ON tl.label_id = l.id AND tl.account_id = l.account_id WHERE tl.thread_id = t.id AND l.name LIKE ?)`
- For hierarchical paths (`folder:"Projects/Q2"`): IMAP folders have `imap_folder_path` on `DbLabel` which stores the full path. Gmail labels encode hierarchy as `/`-separated names (e.g., "Projects/Q2" is the literal label name). Exchange/JMAP folders need a normalization strategy — the current `DbLabel` has no generic `path` column. Options: (a) match against `imap_folder_path` for IMAP, label `name` for Gmail (which already contains the path), and add path normalization for Exchange/JMAP during sync; (b) add a normalized `folder_path` column populated by all providers during sync. Option (b) is cleaner but requires a migration and sync-side changes.
- OR semantics for multiple folder values.

**`in:` operator (universal folder shorthands):**
- Map shorthands to provider-agnostic predicates. The `labels` table has no generic `role` column — system folders are identified via `SYSTEM_FOLDER_ROLES` in `crates/provider-utils/src/folder_roles.rs`, which maps well-known `label_id` values (e.g., `"INBOX"`, `"SENT"`, `"DRAFT"`, `"TRASH"`, `"SPAM"`) across providers. The SQL builder should match against these label IDs, not a role column:

| Shorthand | Predicate |
|-----------|-----------|
| `in:inbox` | `tl.label_id = 'INBOX'` (via thread_labels join) |
| `in:sent` | `tl.label_id = 'SENT'` |
| `in:drafts` | `tl.label_id = 'DRAFT'` |
| `in:trash` | `tl.label_id = 'TRASH'` |
| `in:spam` | `tl.label_id = 'SPAM'` |
| `in:starred` | `t.is_starred = 1` (thread flag, not label join) |
| `in:snoozed` | `t.is_snoozed = 1` (thread flag, not label join) |

- Starred and snoozed are thread flags, not label joins. The builder must handle the mapping.

**`is:tagged` operator:**
- `EXISTS (SELECT 1 FROM thread_labels WHERE thread_id = t.id)`

**`has:contact` operator:**
- `EXISTS (SELECT 1 FROM contacts WHERE email = m.from_address)` for sender
- Optionally also check recipient addresses — TBD whether `has:contact` means "sender is a contact" or "any participant is a contact"

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

## Slice 3: Tantivy Cross-Account Support ✅

**Status: Complete.** `SearchParams.account_ids: Option<Vec<String>>`, `group_by_thread()` helper. 9 tests.

### SearchParams changes

The existing `SearchParams` struct is an internal detail — the unified API takes a raw query string. But Tantivy still needs parameters internally:

- Change `account_id: String` to `account_ids: Option<Vec<String>>` — `None` means all accounts
- In `search_with_filters`, replace the single `TermQuery` on account_id with:
  - `None` → no account filter (search all)
  - `Some(ids)` → `BooleanQuery` with `Should` clauses for each account ID

### SearchResult changes

Current result is message-level. The unified API needs thread-level:

```rust
pub struct SearchResult {
    pub thread_id: String,
    pub account_id: String,
    pub subject: String,
    pub snippet: String,
    pub from_name: String,
    pub from_address: String,
    pub date: i64,
    pub is_read: bool,
    pub is_starred: bool,
    pub message_count: u32,
    pub rank: f32,
}
```

For the Tantivy-only path: query returns message-level hits, group by `thread_id`, take the highest score per thread, enrich with thread metadata from SQLite.

For the SQL→Tantivy path: SQL provides the thread metadata, Tantivy provides the score.

## Slice 4: Unified Pipeline ✅

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

No `scope` parameter — search is always cross-account. Account narrowing is done via `account:` operators in the query string, resolved to account IDs during parsing.

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

The intersection is done in application code — collect SQL thread IDs into a `HashSet`, filter Tantivy results against it. This is simple and fast for typical result sizes.

### Account scope resolution

Search is always cross-account. Account narrowing is controlled entirely by `account:` operators in the query:

- If `account:` operators are present, resolve account display names / emails to account IDs and filter both engines to those accounts
- If no `account:` operators, search all accounts
- Resolution happens during parsing, before either engine is invoked

## Slice 5: App Integration

The app is a pure iced GUI (`crates/app/`) — there is no Tauri layer and no command wrappers. The iced app calls the unified search function from `crates/core/` (or `crates/search/`) directly, just like any other core function.

No `scope` parameter — search is always cross-account per the search problem statement. Users narrow via `account:` operators. The `DbState` and `SearchState` types are `Clone` (they wrap `Arc<Mutex<...>>`), so the app passes them into the search call. For blocking work, `DbState::conn()` provides synchronous access to the connection.

This slice is trivial — it amounts to wiring up the unified search function in the app's update/message handler.

## Slice 6: Smart Folder Migration

Smart folders become thin wrappers around the search pipeline.

### Execution path change

Current: `execute_smart_folder_query` → parse → build SQL → execute SQL → `Vec<DbThread>`
New: `execute_smart_folder_query` → call `unified::search(folder.query, ...)` → convert back to `Vec<DbThread>`

**Important:** The unified search pipeline returns `Vec<SearchResult>`, but the smart folder API must continue returning `Vec<DbThread>` — the sidebar navigation, thread list, and unread count code all depend on this type. The adapter is straightforward: `SearchResult` contains `thread_id` and `account_id`, which can be used to fetch full `DbThread` records, or the SQL-only path (operators without free text, which covers most smart folders) can return `DbThread` directly without going through `SearchResult` at all. Only smart folders with free text in their query string need the `SearchResult` → `DbThread` conversion.

This means smart folders automatically get:
- Tantivy ranking (if the query has free text)
- All new operators
- Cross-account support (smart folders always run cross-account, independent of sidebar scope — see `docs/sidebar/problem-statement.md`)
- Contact expansion

### Token migration

Persisted smart folder queries using `__LAST_7_DAYS__` etc. need migration to offset syntax:

```sql
UPDATE smart_folders SET query = REPLACE(query, '__LAST_7_DAYS__', '-7');
UPDATE smart_folders SET query = REPLACE(query, '__LAST_30_DAYS__', '-30');
UPDATE smart_folders SET query = REPLACE(query, '__TODAY__', '0');
```

Add as a DB migration. Keep `resolve_query_tokens` as a fallback for one release cycle, then remove.

### Unread counts

`count_smart_folder_unread` can reuse the SQL-only path of the unified pipeline (smart folder queries for unread counts don't need ranking — they just need a count of matching unread threads).

## Prerequisites / Schema Changes

### Attachments table: `mime_type` column

**Already exists.** The `attachments` table has a `mime_type TEXT` column (see `crates/db/src/db/migrations.rs`, `DbAttachment.mime_type` in `crates/db/src/db/types.rs`). No migration needed for MIME-type filtering.

### Labels table: system folder identification

The `labels` table has no generic `role` column. System folders are identified by well-known `label_id` values (`"INBOX"`, `"SENT"`, `"DRAFT"`, `"TRASH"`, `"SPAM"`, etc.) defined in `SYSTEM_FOLDER_ROLES` (`crates/provider-utils/src/folder_roles.rs`). The `in:` operator's SQL builder matches against these IDs via `thread_labels.label_id`, not a role column. The `labels` table also has `label_type`, `imap_folder_path`, and `imap_special_use` for provider-specific metadata — these are used by the `folder:` operator for path matching. No migration needed for `in:` support.

## Dependency Graph

```
Slice 1 (parser) ✅
  └── Slice 2 (SQL builder) ✅
        └── Slice 4 (unified pipeline) ✅
              ├── Slice 5 (app integration — trivial wiring)
              └── Slice 6 (smart folder migration)

Slice 3 (Tantivy cross-account) ✅
  └── Slice 4 (unified pipeline) ✅
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

Bloom's **generational load tracking** is the single most impactful pattern for this spec. The implementation spec treats Slice 5 app integration as "trivial wiring," but without stale-result cancellation the search UX will break during incremental typing. The pattern is simple: a monotonically increasing `u64` counter in the app state, incremented before each search dispatch and checked when results arrive. Any result tagged with a generation older than current is silently dropped. This same pattern appears across nearly every spec in the codebase (calendar, main layout, sidebar, command palette, pinned searches, status bar, contacts) and should be treated as a foundational primitive rather than a per-feature afterthought.
