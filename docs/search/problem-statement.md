# Search: Problem Statement

## Overview

Search is the primary way enterprise users find email. Users processing 200+ messages/day don't browse folders — they search. Ratatoskr's search must feel instant, support structured operators for power users, and produce ranked results for everyone else. There is one search bar, one query language, and one API surface.

This document covers the search UX, the query language, and the unification of the two existing search backends into a single pipeline. All search is local — Ratatoskr syncs the full mailbox locally (with zstd compression and inline image deduplication), so there is no need for provider-side search delegation.

## Current State

### Two Search Engines That Don't Talk

**Tantivy** (full-text engine):
- Indexes subject, from_name, to_addresses, body_text, snippet per message
- Tokenized, ranked results with phrase matching
- Supports date range and boolean flag filters (is_read, is_starred, has_attachment)
- Cannot filter by labels, thread flags (snoozed/pinned/muted/important), or structured operators
- Single-account only (SearchParams takes one account_id)

**Smart folder SQL engine** (structured queries):
- Parses operator syntax: `from:`, `to:`, `subject:`, `has:attachment`, `is:unread`, `is:starred`, `is:snoozed`, `is:pinned`, `is:muted`, `is:important`, `before:`, `after:`, `label:`
- Builds parameterized SQL with LIKE clauses
- No ranking — results sorted by pinned + date only
- Free-text falls back to LIKE on subject + from + snippet (no tokenization, no stemming, no ranking)
- Supports cross-account queries via AccountScope

### What's Wrong

- **No unified query path.** A user can't type `from:alice meeting notes` and get ranked results filtered by sender. Tantivy handles `meeting notes` with ranking but can't filter by `from:`. The smart folder engine handles `from:alice` but does LIKE matching on the free text with no ranking.
- **Smart folder creation is over-engineered.** There's a settings UI for building smart folder queries through form fields. This is backwards — the natural flow is: search for something, get results, save the search. The form-based editor is a worse version of the query syntax.
- **Two APIs for the same thing.** The frontend calls either `search_messages` (Tantivy) or `execute_smart_folder_query` (SQL) depending on context. The UI shouldn't need to know which engine to use.
- **Cross-account gap.** Tantivy is single-account. Smart folders support cross-account. A unified search must work across all accounts.

## Design: One Search Pipeline

### The Model

Search has one entry point: a query string. The pipeline:

```
query string
    → parse operators + free text
    → Tantivy: score free-text matches, apply supported filters
    → SQL: apply remaining filters (labels, thread flags) as post-filter
    → merge, rank, return
```

Or, if it's simpler and fast enough at the data volumes we handle:

```
query string
    → parse operators + free text
    → SQL: apply all structured filters, get candidate thread set
    → Tantivy: score candidates by free-text relevance
    → return ranked results
```

The exact pipeline architecture is an implementation decision. The user-facing contract is: **one query, ranked results, all operators work, all accounts searchable.**

### Query Language

The query language is the smart folder syntax, extended. Users type into one search bar. Everything that isn't a recognized operator is free-text.

#### Operators

##### Core operators

| Operator | Meaning | Example |
|----------|---------|---------|
| `from:` | Sender name or address | `from:alice`, `from:"Alice Smith"` |
| `to:` | Recipient address | `to:bob@example.com` |
| ~~`subject:`~~ | *(removed — free text already searches subject via Tantivy with natural ranking; see below)* | |
| `account:` | Limit to a specific account | `account:FooCorp`, `account:"Work Gmail"` |
| `label:` | Label/folder membership | `label:Clients` |
| `folder:` | Folder path (Exchange/IMAP/JMAP hierarchy) | `folder:Projects`, `folder:"Projects/Q2"` |
| `in:` | Universal folder shorthand | `in:inbox`, `in:sent`, `in:trash`, `in:starred` |
| `before:` | Messages before date | `before:2026/03/01` |
| `after:` | Messages after date | `after:2026/01/01` |

##### Flag operators (`is:`)

| Operator | Meaning |
|----------|---------|
| `is:unread` | Unread messages |
| `is:read` | Read messages |
| `is:starred` | Starred/flagged messages |
| `is:snoozed` | Snoozed messages |
| `is:pinned` | Pinned threads |
| `is:muted` | Muted threads |
| `is:tagged` | Threads with any label/tag applied |

##### Attachment operators (`has:`)

| Operator | Expands to | Matches |
|----------|-----------|---------|
| `has:attachment` | *(native)* | Any attachment |
| `has:pdf` | `type:application/pdf` | PDF files |
| `has:image` | `type:image/jpeg \| type:image/png \| type:image/gif \| type:image/webp \| type:image/svg+xml` | Common image formats |
| `has:excel` | `type:application/vnd.ms-excel \| type:application/vnd.openxmlformats-officedocument.spreadsheetml.sheet \| type:application/vnd.oasis.opendocument.spreadsheet \| type:text/csv` | Spreadsheets (.xls, .xlsx, .ods, .csv) |
| `has:word` | `type:application/msword \| type:application/vnd.openxmlformats-officedocument.wordprocessingml.document \| type:application/vnd.oasis.opendocument.text \| type:application/rtf` | Word processors (.doc, .docx, .odt, .rtf) |
| `has:powerpoint` | `type:application/vnd.ms-powerpoint \| type:application/vnd.openxmlformats-officedocument.presentationml.presentation \| type:application/vnd.oasis.opendocument.presentation` | Presentations (.ppt, .pptx, .odp) |
| `has:spreadsheet` | *(alias for `has:excel`)* | Spreadsheets |
| `has:document` | `has:word \| has:pdf` | Any document (word processors + PDF) |
| `has:archive` | `type:application/zip \| type:application/gzip \| type:application/x-tar \| type:application/x-7z-compressed \| type:application/x-rar-compressed` | Compressed archives |
| `has:video` | `type:video/*` | Video files |
| `has:audio` | `type:audio/*` | Audio files |
| `has:calendar` | `type:text/calendar \| type:application/ics` | Calendar invites (.ics) |
| `has:contact` | *(native)* | Any sender/recipient exists as a stored contact |

##### Low-level type operator

| Operator | Meaning | Example |
|----------|---------|---------|
| `type:` | Match attachment MIME type | `type:application/pdf`, `type:image/*` |

The `has:` shorthands are syntactic sugar that expand to `type:` expressions during parsing. `type:` supports glob patterns (`image/*`, `video/*`) for broad category matching. Users can use `type:` directly for MIME types not covered by a shorthand.

##### Expansion model

Shorthand expansion happens at parse time, before the query hits either search engine. The parser maintains a mapping of `has:` names to `type:` expansions. This means:

- New `has:` shorthands can be added by updating the mapping — no engine changes
- Smart folders can use either `has:pdf` or `type:application/pdf` — they're equivalent after parsing
- The search bar could display the expanded form on hover/focus for transparency, but stores the shorthand in saved queries for readability

##### Scoping operators: `account:`, `folder:`, `label:`, `in:`

These four operators control *where* to search. They compose naturally:

- `meeting notes` — search all accounts, all folders
- `account:FooCorp meeting notes` — search only Foo Corp's account
- `account:FooCorp folder:Projects meeting notes` — search only Foo Corp's Projects folder
- `folder:Inbox is:unread` — search Inbox across all accounts that have one
- `in:sent from:me report` — search Sent folder across all accounts
- `label:Clients` — threads tagged with "Clients" label (Gmail/tags model)

The distinction between `folder:`, `label:`, and `in:`:

- **`in:`** — universal folder shorthands only (inbox, sent, drafts, trash, spam, starred, snoozed). These map to provider-agnostic predicates. Works cross-account.
- **`folder:`** — provider-specific folder paths (Exchange folders, IMAP hierarchy, JMAP mailboxes). Supports `/`-separated paths for nested folders: `folder:"Projects/Q2/Reviews"`. A message lives in exactly one folder.
- **`label:`** — provider-specific labels/tags (Gmail labels). A message can have multiple labels. On providers that only have folders (Exchange, IMAP), `label:` and `folder:` are equivalent.

When `account:` is omitted, `folder:` and `label:` match across all accounts that have a folder/label with that name. If the name is ambiguous (same folder name on multiple accounts), all matching accounts are included. Use `account:` to disambiguate.

##### "Search here" interaction

Right-clicking a folder or label in the sidebar and choosing "Search here" (a command palette action) prefills the search bar with the appropriate scope operators:

- Right-click "Projects" folder under Foo Corp → search bar prefills `account:FooCorp folder:Projects `
- Right-click "Clients" label under Gmail → search bar prefills `account:Gmail label:Clients `
- Right-click "Inbox" universal folder (scoped to Foo Corp) → search bar prefills `account:FooCorp in:inbox `
- Right-click "Inbox" universal folder (All Accounts scope) → search bar prefills `in:inbox `

The trailing space is intentional — the cursor is positioned after the operators so the user can immediately type their search terms. The prefilled operators are editable; the user can remove or modify them.

This interaction is the primary way users discover scope operators — they right-click, see the generated query, and learn the syntax by example.

Free text is everything else: `from:alice meeting notes` → operator `from:alice` + free text `meeting notes`.

#### Operator Typeahead

When the user types an operator followed by a value (`from:ali`), a popup appears anchored below the search bar showing matches from the relevant data source. This applies to operators whose values come from a known set:

| Operator | Typeahead source | Display |
|----------|-----------------|---------|
| `from:` | `contacts_fts` (email + display_name) | Name + email address |
| `to:` | `contacts_fts` | Name + email address |
| `account:` | Accounts list | Account name |
| `label:` | Labels table (scoped by `account:` if present) | Label name |
| `folder:` | Folders table (scoped by `account:` if present) | Folder path |

**Interaction model:**

```
┌────────────────────────────────────┐
│ 🔍 from:ali                    ✕  │
│ ┌────────────────────────────┐     │
│ │ Alice Smith                │     │
│ │ asmith@corp.com            │     │
│ │────────────────────────────│     │
│ │ Alicia Jones               │     │
│ │ alicia@example.com         │     │
│ │────────────────────────────│     │
│ │ ali (keep as text)         │     │
│ └────────────────────────────┘     │
├────────────────────────────────────┤
│ [thread cards...]                  │
```

- **↑/↓** navigate the popup, **Enter** selects
- Selecting a contact replaces the typed value with the resolved identifier: `from:ali` → `from:asmith@corp.com`
- **Last option is always "keep as text"** — uses the raw input for LIKE matching. This is the fallback for searching by partial strings that don't match a contact.
- **Escape** dismisses the popup and keeps the raw text
- If the user keeps typing and hits **space** or types another operator, the popup dismisses and the raw text is used as-is
- The popup updates live as the user types — each keystroke re-queries the data source

**Contact resolution for `from:` and `to:`:**

The typeahead hits `contacts_fts` (the existing FTS5 index on contact email + display_name). When a contact is selected, the search uses their email address — this means `from:smith` can find emails from `a.s@corp.com` if the user has a contact named "Alice Smith" with that address, even though neither the message's from_address nor from_name contain "smith."

When the user skips the typeahead (keeps raw text or types too fast), the SQL builder falls back to LIKE matching against `from_address` and `from_name` directly, plus a contact expansion subquery: any contact matching the raw text contributes their email addresses to the filter. This way `from:smith` finds contact-matched results even without explicit typeahead selection.

**Scoping in typeahead:**

`label:` and `folder:` typeahead results are scoped by `account:` if one is already present in the query. If the user has typed `account:FooCorp folder:`, only Foo Corp's folders appear in the popup. Without an `account:` operator, folders/labels from all accounts appear with account names for disambiguation.

**Date picker for `before:` and `after:`:**

When the user types `before:` or `after:`, a popup appears with common presets and an option to pick a specific date:

```
┌────────────────────────────────────┐
│ 🔍 after:                      ✕  │
│ ┌────────────────────────────┐     │
│ │ Today                      │     │
│ │ Yesterday                  │     │
│ │ Last 7 days                │     │
│ │ Last 30 days               │     │
│ │ Last 3 months              │     │
│ │ Last year                  │     │
│ │────────────────────────────│     │
│ │ 📅 Pick a date...          │     │
│ └────────────────────────────┘     │
├────────────────────────────────────┤
```

- **↑/↓** navigate, **Enter** selects
- Selecting a preset inserts a relative offset: `after:-7` (7 days ago), `after:-30`, `after:-90`, etc.
- "Pick a date" opens a calendar widget. Selecting a specific date inserts an absolute value: `after:2026/03/11`.
- Typing either format directly skips the popup — power users don't need the picker.

**Relative offsets vs absolute dates:**

| Input | Meaning |
|-------|---------|
| `after:-1` | Yesterday |
| `after:-7` | 7 days ago |
| `after:-30` | 30 days ago |
| `after:-90` | 3 months ago |
| `after:-365` | 1 year ago |
| `after:0` | Today |
| `before:-7` | Older than 7 days |
| `after:2025` | After January 1, 2025 |
| `after:202603` | After March 1, 2026 |
| `after:20260311` | After March 11, 2026 |

Absolute dates accept any reasonable separator or none: `2026/03/11`, `2026-03-11`, `2026 03 11`, `20260311` are all equivalent. The date parser greedily consumes subsequent tokens that look like date parts (bare digits, separators), so spaces don't need quoting — `after:2026 03 11 from:alice` parses correctly because the date parser grabs `2026`, `03`, `11` and the main lexer resumes at `from:`.

Relative offsets resolve at query time. A smart folder with `after:-7` always shows the last week's email. A smart folder with `after:2026/03/11` is frozen to that date. For ad-hoc search, the distinction is invisible — both resolve identically.

The existing date token system (`__LAST_7_DAYS__`, `__TODAY__`, etc.) in the smart folder engine is an internal implementation detail. The user-facing syntax is always the offset format. The parser translates `after:-7` to the appropriate absolute date before the query hits the engines.

#### Why No `subject:` Operator

`subject:` was removed because free text already searches subject lines via Tantivy, which naturally ranks subject matches higher than body-only matches. The only value `subject:` would add is *excluding* body hits — a rare need that doesn't justify the parsing ambiguity it creates.

The ambiguity: `subject:hallo frank from:frank` — does the subject end at `hallo` (single token) or `hallo frank` (greedy until next operator)? Every answer is surprising to some users. Quoting (`subject:"hallo frank"`) solves it but nobody remembers to quote. Dropping the operator entirely sidesteps the problem — `hallo frank from:frank` just works, with Tantivy ranking subject matches appropriately.

If user feedback shows a genuine need for subject-exclusive filtering, it can be re-added with mandatory quoting for multi-word values.

#### Implicit Boolean Logic

There are no explicit `AND`, `OR`, `NOT` keywords and no grouping with parentheses. Boolean logic is implicit based on a simple rule: **same operator repeated = OR, different operators = AND.**

**Same operator = OR** (widening the net):
- `from:alice from:bob` → from alice OR bob
- `label:Clients label:Projects` → in either label
- `has:pdf has:excel` → either attachment type
- `account:FooCorp account:Gmail` → from either account
- `in:inbox in:sent` → in either folder

**Different operators = AND** (narrowing):
- `from:alice to:bob` → from alice AND to bob
- `from:alice is:unread` → from alice AND unread
- `from:alice has:attachment` → from alice AND has attachments
- `after:-7 before:-1` → both date constraints (range)
- `from:alice after:-30` → from alice AND in the last 30 days

This matches natural language. "Emails from Alice and Bob" means from either. "Emails from Alice that have attachments" means both conditions. No user types `from:alice from:bob` expecting messages that are simultaneously from both.

**Free text is always AND with everything.** `from:alice meeting notes` → from alice AND free text matches "meeting notes".

**No negation in V1.** There's no `NOT` or `-` prefix. Revisit if users request it — the most likely need is `-from:noreply` or `-label:Newsletters` to exclude noise.

#### What We Don't Need (Yet)

- **Explicit boolean operators** (`AND`, `OR`, `NOT`): Implicit logic covers the common cases. Explicit operators add parsing complexity and the grouping problem (`(from:alice OR from:bob) AND has:attachment`) without proportional value.
- **Grouping** (`(...)`): Not needed without explicit boolean operators.
- **Regex**: No.
- **Provider-specific syntax** (Gmail's `category:`, `larger:`, etc.): Not in V1. The local query language is provider-agnostic.

### Smart Folders Are Saved Searches

A smart folder is a persisted query string. Nothing more.

**Creation flow:**
1. User types a search query in the search bar
2. Results appear in the thread list
3. User likes the results → opens palette → "Save as Smart Folder"
4. Prompt for a name and optional icon
5. Query string is saved to the smart_folders table
6. Folder appears in the sidebar under Smart Folders

**Editing flow:**
1. User clicks a smart folder in the sidebar → thread list shows results, search bar shows the query string
2. User modifies the query in the search bar
3. Results update live
4. User saves again via palette → "Update Smart Folder" (overwrites the query)

**No form-based editor.** The query syntax *is* the editor. If the syntax is good enough to type, it's good enough to edit. A visual query builder is a crutch for a bad syntax — we should make the syntax good instead.

The existing smart folder settings UI should be removed. Smart folder management (rename, delete, reorder) moves to the command palette.

### Relative Dates in Smart Folders

Smart folders use relative date offsets that resolve at query time:

Example: a smart folder with query `is:unread after:-7` always shows unread messages from the last week. The `-7` resolves to an absolute date each time the query runs.

The existing internal token system (`__LAST_7_DAYS__`, etc.) is replaced by the offset syntax in all user-facing contexts. The parser handles the translation.

## Search UX

### Search Bar Placement

The search bar lives above the thread list, inline with the thread list panel. It is always visible — not hidden behind a button or shortcut. Pressing `/` focuses it from anywhere in the app.

```
┌──────────────┬────────────────────────┬──────────────────────┐
│              │ 🔍 Search...           │                      │
│   Sidebar    │────────────────────────│    Reading Pane      │
│              │                        │                      │
│              │   Thread List          │                      │
│              │                        │                      │
└──────────────┴────────────────────────┴──────────────────────┘
```

### Search Behavior

1. **Typing filters the thread list in place.** The thread list switches from "current folder's threads" to "search results" as the user types. No separate search results page.

2. **Results are ranked.** Free-text matches are scored by Tantivy's relevance ranking. Results with more matching terms, better term positions, and higher term frequency rank higher.

3. **Results are scoped to the current account scope.** If the sidebar is scoped to "All Accounts," search spans all accounts. If scoped to a specific account, search is limited to that account. The user doesn't need to specify account — it follows the existing scope model.

4. **Operators autocomplete.** When the user types `from:` or `is:`, the search bar could offer completions (contact names for `from:`, flag names for `is:`). This is a polish feature, not V1 — the query works without autocomplete.

5. **Clearing search returns to the folder view.** Pressing Escape or clearing the search bar restores the previous thread list (inbox, label, whatever was active before searching).

6. **Search is instant.** Local search against Tantivy + SQLite should return results in single-digit milliseconds for typical mailbox sizes (50K-200K messages). No loading spinners, no debounce delays visible to the user.

### Search + Smart Folder Interaction

When a smart folder is selected in the sidebar:
- The thread list shows the smart folder's results
- The search bar shows the smart folder's query string (editable)
- The user can modify the query to refine results
- Modified query is not auto-saved — it's ephemeral until explicitly saved

This means the search bar does double duty: it's both the search input and the smart folder query display. The query string is the universal representation.

### Keyboard Interaction

| Key | Action |
|-----|--------|
| `/` | Focus search bar from anywhere |
| `Escape` | Clear search and return to folder view (if searching); blur search bar (if empty) |
| `Enter` | Execute search (if debounce hasn't fired yet); move focus to first result |
| `↓` | Move focus from search bar to first result in thread list |

## Search API

### Single Endpoint

The UI calls one search function:

```
search(query: String, scope: AccountScope) -> Vec<SearchResult>
```

The backend:
1. Parses the query string (operators + free text)
2. Routes to the appropriate engine(s)
3. Returns ranked, deduplicated results as threads

The caller doesn't know or care whether Tantivy, SQL, or both were involved.

### SearchResult

```
SearchResult {
    thread_id: String,
    account_id: String,
    subject: String,
    snippet: String,
    from_name: String,
    from_address: String,
    date: i64,
    is_read: bool,
    is_starred: bool,
    message_count: u32,
    rank: f32,           // relevance score (0.0 if no free text)
}
```

This matches the data needed by thread cards. The UI renders search results using the same thread card component as the folder view.

## Implementation Sequence

### Phase 1: Unify the Query Parser

Merge the smart folder parser and the Tantivy search interface behind a single `search()` function. The parser already handles operators and free text — it just needs to route free text to Tantivy for ranking instead of LIKE.

- Extend `ParsedQuery` to distinguish "free text for ranking" from "structured filter"
- Build a single `search()` function in core that uses both engines
- Add cross-account support to the Tantivy path (remove single-account restriction)

### Phase 2: Search Bar in iced

- Wire the search bar above the thread list to the unified search API
- Results replace the thread list content
- `/` to focus, Escape to clear
- Smart folder selection populates the search bar with the saved query

### Phase 3: Smart Folder Simplification

- Remove the smart folder form-based editor from settings
- Add "Save as Smart Folder" to the command palette
- Smart folder CRUD moves entirely to the palette (rename, delete, reorder)
- Sidebar smart folder click → search bar shows query, thread list shows results

### Phase 4: Polish

- Operator autocomplete in search bar
- Search history (recent queries, accessible via ↑ in empty search bar)
- Highlighted matching terms in thread list snippets
- Search result count indicator

## Open Questions

1. **Pipeline architecture**: Resolved. Three paths based on query content:

   - **Operators only, no free text** → SQL only, date-sorted. No Tantivy involvement — there's nothing to rank.
   - **Free text only, no operators** → Tantivy only, relevance-ranked. No SQL involvement — Tantivy searches the full index.
   - **Both operators and free text** → SQL narrows candidates via relational filters (labels, folders, accounts, flags, contacts), then Tantivy scores the candidate set by free-text relevance. Result sets are intersected in application code.

   This is the only sane architecture because operators like `label:`, `folder:`, `account:`, `has:contact` require relational joins that Tantivy cannot do. They'd be post-filters regardless. SQL eliminates the bulk of the corpus first — `account:FooCorp folder:Inbox is:unread` might narrow 200K messages to 50, and Tantivy scoring 50 documents is trivial. For the broad free-text-only case, Tantivy searches the full index directly, which is what it's built for.

3. **Search scope vs sidebar scope**: Resolved. Search is always cross-account by default, independent of the sidebar's scope. Users narrow via `account:` and `folder:` operators. The "Search here" right-click action on sidebar items prefills scope operators for discoverability. This matches the mental model: scope is for browsing, search is for finding.

4. **Provider-side search**: Not needed. Ratatoskr syncs everything locally — even 300GB mailboxes. Between zstd body compression and multipart inline image deduplication, local storage is significantly more efficient than what providers keep server-side. Since the full corpus is always local, Tantivy searches against complete data. There is no "not yet synced" gap to fill with provider API search.

5. **Body text indexing**: Resolved. Ratatoskr syncs the full mailbox locally, so body text is always available for indexing. The Tantivy index is rebuilt from the body store, which contains every message. No on-demand fetching needed.

## Dependencies

- **Tantivy cross-account**: The current SearchParams takes a single account_id. This needs to accept AccountScope (All, Single, Multiple) to match the smart folder engine's capability.
- **Unified parser**: The smart folder parser already handles the operator syntax. It needs to be extended to produce output that both Tantivy and SQL can consume.
- **Command palette**: "Save as Smart Folder" and smart folder management commands need to be registered. The command palette infrastructure (Slices 1-2) already supports this pattern.
- **Thread list component**: Search results must render using the same thread card component as folder views. This is a UI concern for the iced prototype, not a backend issue.
