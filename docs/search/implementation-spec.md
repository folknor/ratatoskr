# Search: Backend Implementation Spec

Implementation plan for unifying the search backend per `docs/search/problem-statement.md`. All work is in `src-tauri/core/`. The TS search layer is being replaced by the iced UI and is not covered here.

## Current State

Two separate search engines with no bridge:

- **Tantivy** (`core/src/search/mod.rs`) — full-text ranked search. Single-account only. Accepts pre-parsed `SearchParams`. Returns message-level results.
- **Smart folder SQL** (`core/src/smart_folder/`) — operator-based SQL queries. Cross-account via `AccountScope`. No ranking. Returns thread-level results.

## Target State

One function: `search(query: &str, scope: AccountScope) -> Result<Vec<SearchResult>, Error>`

Three internal paths based on parsed query content:

1. **Operators only** → SQL, date-sorted
2. **Free text only** → Tantivy, relevance-ranked
3. **Both** → SQL narrows candidates, Tantivy scores them

## Slice 1: Parser Overhaul

Rewrite `core/src/smart_folder/parser.rs`. The parser is the foundation — everything else builds on its output.

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

The `__LAST_7_DAYS__` / `__LAST_30_DAYS__` / `__TODAY__` token system in `tokens.rs` becomes unnecessary once the parser handles relative offsets natively. Steps:

1. Add relative offset support to the parser (this slice)
2. Migrate any persisted smart folder queries that use tokens to offset syntax (DB migration or on-read translation)
3. Keep `resolve_query_tokens` as a backward-compatibility shim until migration is confirmed complete
4. Remove `tokens.rs` once no queries use the old format

## Slice 2: SQL Builder Expansion

Extend `core/src/smart_folder/sql_builder.rs` to handle all new operators.

### New clause builders

**`account:` operator:**
- Match by account name (not ID): `JOIN accounts a ON m.account_id = a.id WHERE a.name LIKE ?`
- OR semantics for multiple: `a.name LIKE ? OR a.name LIKE ?`
- Replaces the current `AccountScope` parameter for query-driven scoping. When `account:` operators are present, they override the `scope` parameter.

**`folder:` operator:**
- Match by folder/mailbox name or path: `EXISTS (SELECT 1 FROM thread_labels tl JOIN labels l ON tl.label_id = l.id WHERE tl.thread_id = t.id AND l.name LIKE ?)`
- For hierarchical paths (`folder:"Projects/Q2"`): match the full path or the leaf name depending on how folder hierarchy is stored. May need a `path` column on labels or a recursive match.
- OR semantics for multiple folder values.

**`in:` operator (universal folder shorthands):**
- Map shorthands to provider-agnostic predicates:

| Shorthand | Predicate |
|-----------|-----------|
| `in:inbox` | `label.role = 'inbox'` or label name match |
| `in:sent` | `label.role = 'sent'` |
| `in:drafts` | `label.role = 'drafts'` |
| `in:trash` | `label.role = 'trash'` |
| `in:spam` | `label.role = 'spam'` |
| `in:starred` | `t.is_starred = 1` |
| `in:snoozed` | `t.is_snoozed = 1` |

- Starred and snoozed are thread flags, not label joins. The builder must handle the mapping.

**`is:tagged` operator:**
- `EXISTS (SELECT 1 FROM thread_labels WHERE thread_id = t.id)`

**`has:contact` operator:**
- `EXISTS (SELECT 1 FROM contacts WHERE email = m.from_address)` for sender
- Optionally also check recipient addresses — TBD whether `has:contact` means "sender is a contact" or "any participant is a contact"

**`type:` / attachment MIME filtering:**
- `EXISTS (SELECT 1 FROM attachments WHERE message_id = m.id AND content_type LIKE ?)`
- For glob patterns (`video/*`): `content_type LIKE 'video/%'`
- For exact types: `content_type = ?`
- OR semantics: multiple types from `has:` expansion become `(content_type LIKE ? OR content_type LIKE ? OR ...)`
- Prerequisite: verify the `attachments` table has a `content_type` column. If not, add via migration.

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
EXISTS (SELECT 1 FROM attachments WHERE ... AND content_type = 'application/pdf')
```

### Result shape

The SQL builder already returns `Vec<DbThread>` (thread-level). This is correct for the operators-only path and for generating candidate IDs for the Tantivy path.

## Slice 3: Tantivy Cross-Account Support

Modify `core/src/search/mod.rs` to support multi-account search.

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

## Slice 4: Unified Pipeline

New module: `core/src/search/unified.rs` (or extend `core/src/search/mod.rs`).

### The router

```rust
pub fn search(
    query: &str,
    scope: AccountScope,
    search_state: &SearchState,
    db: &Connection,
) -> Result<Vec<SearchResult>, Error> {
    let parsed = parse_query(query);

    let has_free_text = parsed.free_text.is_some();
    let has_operators = parsed.has_any_operator();

    match (has_free_text, has_operators) {
        (false, false) => Ok(vec![]),  // empty query
        (false, true) => search_sql_only(&parsed, scope, db),
        (true, false) => search_tantivy_only(&parsed, scope, search_state),
        (true, true) => search_combined(&parsed, scope, search_state, db),
    }
}
```

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

The `scope` parameter and `account:` operators interact:

- If `account:` operators are present in the query, they override the `scope` parameter
- If no `account:` operators, use the `scope` parameter (which defaults to `All` for search)
- Resolve account names to account IDs before passing to either engine

## Slice 5: Tauri Command

New command replacing `search_messages`:

```rust
#[tauri::command]
pub async fn search(
    query: String,
    scope: AccountScope,
    db: State<'_, DbState>,
    search: State<'_, SearchState>,
) -> Result<Vec<SearchResult>, String> {
    let db = db.lock()?;
    unified::search(&query, scope, &search, &db)
}
```

The old `search_messages` command stays for backward compatibility with the React frontend until it's replaced. The iced UI calls the new `search` command directly (or the core function, since iced doesn't need Tauri commands).

## Slice 6: Smart Folder Migration

Smart folders become thin wrappers around the search pipeline.

### Execution path change

Current: `execute_smart_folder_query` → parse → build SQL → execute SQL
New: `execute_smart_folder_query` → call `unified::search(folder.query, scope, ...)`

This means smart folders automatically get:
- Tantivy ranking (if the query has free text)
- All new operators
- Cross-account support
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

### Attachments table: `content_type` column

Verify the `attachments` table has a `content_type` (MIME type) column. If not:

```sql
ALTER TABLE attachments ADD COLUMN content_type TEXT DEFAULT '';
```

And backfill from existing attachment data during sync or via a migration that re-parses stored attachment metadata.

### Labels table: role column

The `in:` operator maps shorthands to label roles (inbox, sent, drafts, trash, spam). Verify the `labels` table has a `role` or `type` column that identifies well-known folders. The provider sync already normalizes these — confirm the column name and values.

## Dependency Graph

```
Slice 1 (parser)
  └── Slice 2 (SQL builder)
        └── Slice 4 (unified pipeline)
              ├── Slice 5 (Tauri command)
              └── Slice 6 (smart folder migration)

Slice 3 (Tantivy cross-account)
  └── Slice 4 (unified pipeline)
```

Slices 1 and 3 can be done in parallel. Slice 2 depends on 1. Slice 4 depends on 2 and 3. Slices 5 and 6 depend on 4.
