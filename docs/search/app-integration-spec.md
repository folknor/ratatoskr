# Search App Integration: Implementation Spec

Detailed implementation specification for wiring the unified search pipeline into the iced app and completing the smart folder migration. The backend (parser, SQL builder, Tantivy cross-account, unified pipeline) is complete — slices 1-4 in `docs/search/implementation-spec.md`. This document covers slices 5-6 in full: the search bar widget, search execution with generational tracking, result rendering, smart folder migration, operator typeahead, and the "Search here" interaction.

## References

- **Product spec:** `docs/search/problem-statement.md`
- **Backend spec:** `docs/search/implementation-spec.md` (slices 1-4 complete)
- **Unified pipeline:** `crates/core/src/search_pipeline.rs` — `search()` entry point
- **Parser:** `crates/smart-folder/src/parser.rs` — `ParsedQuery`, `parse_query()`
- **SQL builder:** `crates/smart-folder/src/sql_builder.rs` — `query_threads()`, `count_matching()`
- **Token system:** `crates/smart-folder/src/tokens.rs` — `resolve_query_tokens()` (to be deprecated)
- **Smart folder facade:** `crates/smart-folder/src/lib.rs` — `execute_smart_folder_query()`, `count_smart_folder_unread()`
- **App entry:** `crates/app/src/main.rs` — `App`, `Message` enum, generational tracking
- **Thread list:** `crates/app/src/ui/thread_list.rs` — `ThreadList`, `ThreadListMessage`
- **Component trait:** `crates/app/src/component.rs` — `Component` trait
- **App types:** `crates/app/src/db.rs` — `Thread`, `Account`, `Label`
- **Layout constants:** `crates/app/src/ui/layout.rs` — spacing, text, padding tokens
- **Pinned searches:** `docs/search/pinned-searches.md` (downstream dependency)
- **Sidebar spec:** `docs/sidebar/problem-statement.md`
- **Implementation plan:** `docs/implementation-plan.md`

## Phasing

| Phase | Scope | Depends on |
|-------|-------|------------|
| **Phase 1** | Search bar widget + execution + generational tracking + result rendering | Backend slices 1-4 (done) |
| **Phase 2** | Smart folder migration to unified pipeline | Phase 1 |
| **Phase 3** | Operator typeahead popup | Phase 1 |
| **Phase 4** | "Search here" sidebar interaction + polish | Phase 1, sidebar Phase 1 |

Phases 2-4 are independent of each other and can proceed in parallel once Phase 1 is complete.

---

## Phase 1: Search Bar + Execution + Generational Tracking

### 1.1 Thread List Mode

The thread list operates in one of two modes: **folder view** (browsing a label/folder) or **search results** (displaying search results). The mode determines what the thread list shows, how it sorts, and what Escape does.

#### Type definitions

In `crates/app/src/ui/thread_list.rs`:

```rust
/// What the thread list is currently displaying.
#[derive(Debug, Clone)]
pub enum ThreadListMode {
    /// Browsing a folder or label — threads loaded from scoped DB query.
    Folder,
    /// Displaying search results — threads came from the unified search pipeline.
    Search,
}
```

Add to the `ThreadList` struct:

```rust
pub struct ThreadList {
    pub threads: Vec<Thread>,
    pub selected_thread: Option<usize>,
    pub folder_name: String,
    pub scope_name: String,
    pub mode: ThreadListMode,
}
```

The mode is set to `ThreadListMode::Search` when search results arrive (via `SearchResultsLoaded`) and back to `ThreadListMode::Folder` when a folder/label is selected from the sidebar or when the user presses Escape in the search bar.

### 1.2 Search State in App

Add search-related state to the `App` struct in `crates/app/src/main.rs`:

```rust
struct App {
    // ... existing fields ...

    /// Monotonically increasing counter for search result freshness.
    /// Incremented on every search dispatch. Results tagged with a
    /// generation older than this are silently dropped.
    search_generation: u64,

    /// The query string currently in the search bar. Kept in App state
    /// (not just widget state) because smart folder selection and
    /// "Search here" both need to set it programmatically.
    search_query: String,

    /// When a smart folder is selected, stores its ID so that
    /// "Update Smart Folder" knows which folder to update.
    active_smart_folder_id: Option<i64>,
}
```

Initialize in `boot()`:

```rust
search_generation: 0,
search_query: String::new(),
active_smart_folder_id: None,
```

### 1.3 Message Enum Extensions

Add to the `Message` enum in `crates/app/src/main.rs`:

```rust
pub enum Message {
    // ... existing variants ...

    /// The search bar text changed (debounced or immediate).
    SearchQueryChanged(String),

    /// User pressed Enter in the search bar or debounce timer fired.
    /// Triggers search execution.
    SearchExecute,

    /// Search results arrived from the async search task.
    /// The u64 is the search generation for staleness detection.
    SearchResultsLoaded(u64, Result<Vec<Thread>, String>),

    /// User pressed Escape in the search bar — clear search and
    /// return to folder view.
    SearchClear,

    /// Global `/` keypress — focus the search bar.
    FocusSearchBar,
}
```

### 1.4 Generational Load Tracking

This is the single most important correctness mechanism in the search integration. Without it, rapid typing produces flickering or displays results from a stale query.

#### The problem

The user types "m", "me", "mee", "meet", "meeti", "meetin", "meeting" in quick succession. Each keystroke (after debounce) dispatches a search `Task`. These tasks are async and may complete out of order — the search for "me" (broad, many results) might take longer than "meeting" (narrow, few results). If the "me" results arrive after the "meeting" results, the thread list flickers from correct results back to stale ones.

#### The solution

A `search_generation: u64` counter in the `App` struct:

1. **Increment** the counter before every search dispatch (in `SearchExecute` handling).
2. **Tag** the `Task` with the current generation value.
3. **Check** when results arrive: if the result's generation is less than `self.search_generation`, silently drop it.

This is the same pattern already used by `nav_generation` and `thread_generation` in the existing codebase. The search generation is a third independent counter.

#### Implementation

In `App::update`:

```rust
Message::SearchQueryChanged(query) => {
    self.search_query = query;
    // Debounce is handled by the search bar widget's subscription
    // (see 1.5). The widget emits SearchExecute after the debounce
    // interval, or immediately on Enter.
    Task::none()
}

Message::SearchExecute => {
    let query = self.search_query.trim().to_string();
    if query.is_empty() {
        // Empty query = clear search, return to folder view
        return self.restore_folder_view();
    }

    self.search_generation += 1;
    let generation = self.search_generation;
    let db = Arc::clone(&self.db);

    Task::perform(
        async move {
            let result = execute_search(db, query).await;
            (generation, result)
        },
        |(g, result)| Message::SearchResultsLoaded(g, result),
    )
}

Message::SearchResultsLoaded(g, _) if g != self.search_generation => {
    // Stale results — a newer search has been dispatched.
    // Silently drop.
    Task::none()
}

Message::SearchResultsLoaded(_, Ok(threads)) => {
    self.thread_list.mode = ThreadListMode::Search;
    self.thread_list.set_threads(threads);
    self.thread_list.selected_thread = None;
    self.status = format!("{} results", self.thread_list.threads.len());
    Task::none()
}

Message::SearchResultsLoaded(_, Err(e)) => {
    self.status = format!("Search error: {e}");
    Task::none()
}

Message::SearchClear => {
    self.search_query.clear();
    self.active_smart_folder_id = None;
    self.search_generation += 1; // Invalidate any in-flight search
    self.restore_folder_view()
}

Message::FocusSearchBar => {
    // Return a Task that focuses the search bar widget by ID.
    iced::widget::operation::focus("search-bar".to_string())
}
```

#### Invariants

- `search_generation` is incremented exactly once per search dispatch and once on `SearchClear`.
- It is never decremented.
- The `SearchResultsLoaded` handler rejects results where `g != self.search_generation` (matches the existing pattern for `AccountsLoaded`, `LabelsLoaded`, `ThreadsLoaded`).
- `SearchClear` increments the generation to invalidate any in-flight search — without this, clearing the search bar could be followed by stale results popping in.

### 1.5 Search Bar Widget

The search bar is an `iced::widget::text_input` with surrounding container styling, integrated into the thread list header. It is not a separate `Component` — it lives within the `ThreadList` component's view, with its state managed at the `App` level (because smart folder selection and "Search here" need to set the query programmatically).

#### Thread list header change

Replace the current placeholder search bar in `thread_list_header` (`crates/app/src/ui/thread_list.rs`) with a real `text_input`:

```rust
fn thread_list_header<'a>(
    folder_name: &'a str,
    scope_name: &'a str,
    search_query: &'a str,
    mode: &ThreadListMode,
) -> Element<'a, ThreadListMessage> {
    let search_input = text_input("Search...", search_query)
        .id("search-bar")
        .on_input(ThreadListMessage::SearchInput)
        .on_submit(ThreadListMessage::SearchSubmit)
        .size(TEXT_MD)
        .padding(PAD_INPUT);

    let context_row = row![
        text(folder_name)
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style()),
        Space::new().width(Length::Fill),
        text(scope_name)
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style()),
    ]
    .align_y(iced::Alignment::Center);

    container(
        column![search_input, context_row].spacing(SPACE_XXS),
    )
    .padding(PAD_PANEL_HEADER)
    .into()
}
```

#### ThreadListMessage extensions

```rust
#[derive(Debug, Clone)]
pub enum ThreadListMessage {
    SelectThread(usize),
    SearchInput(String),
    SearchSubmit,
}
```

#### ThreadListEvent extensions

```rust
#[derive(Debug, Clone)]
pub enum ThreadListEvent {
    ThreadSelected(usize),
    SearchQueryChanged(String),
    SearchExecute,
}
```

The `ThreadList::update` handler maps:
- `SearchInput(query)` -> emits `SearchQueryChanged(query)` event
- `SearchSubmit` -> emits `SearchExecute` event

The `App` maps `ThreadListEvent::SearchQueryChanged` to `Message::SearchQueryChanged` and `ThreadListEvent::SearchExecute` to `Message::SearchExecute`.

#### Passing search query to the thread list view

The search query string is owned by `App` (not `ThreadList`) because external events (smart folder click, "Search here") need to set it. The `ThreadList::view` method receives it as a parameter. Adjust the `Component` trait usage: since `view()` in the `Component` trait takes `&self`, the search query must be stored on `ThreadList` as well, set by `App` before calling `view()`. Add to `ThreadList`:

```rust
pub search_query: String,
```

`App` sets `self.thread_list.search_query = self.search_query.clone()` before `view()` is called — or more idiomatically, keep the query on `ThreadList` and have the `ThreadListEvent::SearchQueryChanged` event propagate it up to `App` for dispatch. The source of truth for "what to search" is the event flow, not shared state.

### 1.6 Keyboard Shortcuts

#### `/` to focus search bar

In `App::subscription`, add a keyboard event listener:

```rust
iced::event::listen_with(|event, _status, _id| {
    if let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
        key: iced::keyboard::Key::Character(ref c),
        modifiers,
        ..
    }) = event {
        if c.as_str() == "/" && modifiers.is_empty() {
            return Some(Message::FocusSearchBar);
        }
    }
    // Escape handling for search clear
    if let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
        ..
    }) = event {
        return Some(Message::SearchClear);
    }
    None
})
```

**Important:** The `/` shortcut must only fire when the search bar is not already focused and no other text input has focus. Iced's `_status` parameter in `listen_with` can be used to check if the event was captured by a widget. If `status == Status::Captured`, skip it — a text input consumed the keypress.

**Escape behavior:**
- If search bar has content: clear the query, increment `search_generation`, restore folder view.
- If search bar is empty and focused: blur the search bar.
- If search bar is empty and not focused: no-op (or propagate to other handlers).

### 1.7 Debounce

Search should execute after a brief debounce (150ms) while the user is typing, and immediately on Enter. The debounce prevents hammering the search pipeline on every keystroke while keeping the feel instant.

#### Implementation via subscription

Use an iced `Subscription` that watches for a "pending search" flag with a timestamp:

Add to `App`:

```rust
/// When set, a search execution is pending after this instant.
search_debounce_deadline: Option<iced::time::Instant>,
```

When `SearchQueryChanged` arrives:

```rust
Message::SearchQueryChanged(query) => {
    self.search_query = query;
    self.thread_list.search_query = self.search_query.clone();
    if self.search_query.trim().is_empty() {
        self.search_debounce_deadline = None;
        // Optionally restore folder view immediately on empty
    } else {
        self.search_debounce_deadline =
            Some(iced::time::Instant::now() + std::time::Duration::from_millis(150));
    }
    Task::none()
}
```

In `App::subscription`, add a debounce timer:

```rust
if let Some(deadline) = self.search_debounce_deadline {
    subs.push(
        iced::time::every(std::time::Duration::from_millis(50))
            .map(move |_| {
                if iced::time::Instant::now() >= deadline {
                    Message::SearchExecute
                } else {
                    Message::Noop
                }
            }),
    );
}
```

When `SearchExecute` fires:

```rust
Message::SearchExecute => {
    self.search_debounce_deadline = None; // Clear the debounce
    // ... dispatch search as in 1.4 ...
}
```

Enter (`SearchSubmit`) bypasses debounce:

```rust
Message::SearchExecute => {
    self.search_debounce_deadline = None;
    // ... (same handler, debounce is already cleared)
}
```

### 1.8 Search Execution (Async Bridge)

The unified search pipeline (`crates/core/src/search_pipeline.rs`) takes `&SearchState` and `&Connection` — both are synchronous, blocking operations. The app must call it off the main thread.

#### The async wrapper

In `crates/app/src/main.rs` (or a dedicated `search.rs` module):

```rust
async fn execute_search(
    db: Arc<Db>,
    query: String,
) -> Result<Vec<Thread>, String> {
    tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn_guard = conn.lock().map_err(|e| e.to_string())?;

        // TODO: SearchState needs to be accessible. Either:
        // (a) Store SearchState in App alongside Db, or
        // (b) Create a new SearchState per search (expensive — avoid).
        // Option (a) is correct: SearchState wraps Arc<...> and is Clone.

        let results = ratatoskr_core::search_pipeline::search(
            &query,
            &search_state,
            &conn_guard,
        )?;

        // Convert UnifiedSearchResult -> app::db::Thread
        Ok(results.into_iter().map(unified_to_app_thread).collect())
    })
    .await
    .map_err(|e| e.to_string())?
}
```

#### SearchState availability

`SearchState` (from `crates/search/`) wraps a Tantivy index reader. It must be initialized at app startup and stored in `App`:

```rust
struct App {
    // ... existing ...
    search_state: SearchState,
}
```

Initialized in `boot()` alongside `Db`:

```rust
let search_state = SearchState::open(&app_data_dir.join("search_index"))
    .map_err(|e| iced::Error::WindowCreationFailed(e.into()))?;
```

Since `SearchState` is `Clone` (wraps `Arc`), pass a clone into the search task.

#### UnifiedSearchResult to Thread conversion

```rust
fn unified_to_app_thread(r: UnifiedSearchResult) -> Thread {
    Thread {
        id: r.thread_id,
        account_id: r.account_id,
        subject: r.subject,
        snippet: r.snippet,
        last_message_at: r.date,
        message_count: r.message_count.unwrap_or(1),
        is_read: r.is_read,
        is_starred: r.is_starred,
        has_attachments: false, // Not in UnifiedSearchResult — see note
        from_name: r.from_name,
        from_address: r.from_address,
    }
}
```

**Note on `has_attachments`:** `UnifiedSearchResult` does not carry `has_attachments`. Two options:

1. Add `has_attachments: bool` to `UnifiedSearchResult` — populated from `DbThread.has_attachments` in the SQL paths, defaulting to `false` in the Tantivy-only path.
2. Accept the missing field for now. The thread card will not show the attachment indicator for search results from the Tantivy-only path.

Option (1) is correct and should be done. Extend `UnifiedSearchResult` in `crates/core/src/search_pipeline.rs`:

```rust
pub struct UnifiedSearchResult {
    // ... existing fields ...
    pub has_attachments: bool,
}
```

Populate from `DbThread.has_attachments` in `db_thread_to_unified()` and `enrich_from_sql()`. Default to `false` in `tantivy_result_to_unified()`.

### 1.9 Result Rendering

Search results reuse the same `thread_card` widget function in `crates/app/src/ui/widgets.rs`. No new widget is needed — search results are `Vec<Thread>` by the time they reach the thread list, identical to folder-view threads.

#### Sort order

The sort order is determined before results reach the thread list:

- **Free text present (rank > 0.0):** Results arrive sorted by relevance (descending rank) from the unified pipeline.
- **Operators only (rank == 0.0):** Results arrive sorted by date (descending) from the SQL-only path.

The thread list renders in the order received. No client-side re-sorting.

#### Result count

Display the result count in the context row below the search bar when in search mode:

```rust
let context_row = match mode {
    ThreadListMode::Folder => row![
        text(folder_name).size(TEXT_SM).style(theme::TextClass::Tertiary.style()),
        Space::new().width(Length::Fill),
        text(scope_name).size(TEXT_SM).style(theme::TextClass::Tertiary.style()),
    ],
    ThreadListMode::Search => row![
        text(format!("{} results", thread_count))
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style()),
    ],
};
```

### 1.10 Restoring Folder View

When the user clears the search (Escape or clearing the search bar), the thread list must return to whatever it was showing before the search began. This requires storing the pre-search state.

Add to `App`:

```rust
/// Threads that were displayed before the current search.
/// Restored when the user clears the search.
pre_search_threads: Option<Vec<Thread>>,
```

When entering search mode (first `SearchExecute` while in `Folder` mode):

```rust
if self.thread_list.mode == ThreadListMode::Folder {
    self.pre_search_threads = Some(self.thread_list.threads.clone());
}
```

When restoring (`SearchClear`):

```rust
fn restore_folder_view(&mut self) -> Task<Message> {
    self.thread_list.mode = ThreadListMode::Folder;
    self.search_query.clear();
    self.thread_list.search_query.clear();
    self.active_smart_folder_id = None;
    if let Some(threads) = self.pre_search_threads.take() {
        self.thread_list.set_threads(threads);
    }
    Task::none()
}
```

**Memory consideration:** Cloning the thread list before search is acceptable. A list of 1000 `Thread` structs is roughly 100-200 KB — trivial. The alternative (re-querying the database for the folder's threads) is correct but slower and produces a visible reload flicker.

---

## Phase 2: Smart Folder Migration

### 2.1 Execution Path Change

Smart folders currently use `execute_smart_folder_query()` (`crates/smart-folder/src/lib.rs`), which calls `resolve_query_tokens()` then `parse_query()` then `query_threads()` — the SQL-only path. After migration, smart folders call the unified `search()` pipeline from `crates/core/src/search_pipeline.rs`, gaining:

- Tantivy ranking for smart folders that contain free text
- All new operators (`account:`, `folder:`, `in:`, `type:`, `has:` shorthands)
- Contact expansion for `from:` / `to:`

#### Migration strategy

Replace `execute_smart_folder_query()` internals:

```rust
/// Execute a smart folder query string against the database.
///
/// Routes through the unified search pipeline. For queries with free text,
/// Tantivy provides relevance ranking. For operator-only queries (the common
/// case for smart folders), the SQL path runs directly.
pub fn execute_smart_folder_query(
    conn: &Connection,
    search_state: &SearchState,
    params: &SmartFolderParams<'_>,
) -> Result<Vec<DbThread>, String> {
    let results = ratatoskr_core::search_pipeline::search(
        params.query,
        search_state,
        conn,
    )?;

    // Convert UnifiedSearchResult -> DbThread for backward compatibility.
    // For the SQL-only path (most smart folders), the conversion is lossless
    // because the results originated from DbThread.
    Ok(results.into_iter().map(unified_to_db_thread).collect())
}
```

**Signature change:** `execute_smart_folder_query` now requires `&SearchState` in addition to `&Connection`. All call sites must be updated. Since smart folder execution currently happens in:
- Sidebar unread count computation (`count_smart_folder_unread`)
- Thread list loading when a smart folder is selected

Both paths already have access to the connection; `SearchState` must be threaded through.

**Unread count path:** `count_smart_folder_unread` should remain SQL-only — unread counts don't need Tantivy ranking. Keep it as-is but update it to use the new parser directly (it already does):

```rust
pub fn count_smart_folder_unread(
    conn: &Connection,
    query: &str,
    scope: &AccountScope,
) -> Result<i64, String> {
    // No token resolution needed — parser handles relative offsets natively
    let mut parsed = parse_query(query);
    parsed.is_unread = Some(true);
    sql_builder::count_matching(conn, &parsed, scope)
}
```

### 2.2 UnifiedSearchResult to DbThread Adapter

```rust
fn unified_to_db_thread(r: UnifiedSearchResult) -> DbThread {
    DbThread {
        id: r.thread_id,
        account_id: r.account_id,
        subject: r.subject,
        snippet: r.snippet,
        last_message_at: r.date.map(|d| d.to_string()),
        message_count: r.message_count.unwrap_or(1),
        is_read: r.is_read,
        is_starred: r.is_starred,
        is_important: false,
        has_attachments: r.has_attachments,
        is_snoozed: false,   // Not in UnifiedSearchResult
        snooze_until: None,
        is_pinned: false,     // Not in UnifiedSearchResult
        is_muted: false,      // Not in UnifiedSearchResult
        from_name: r.from_name,
        from_address: r.from_address,
    }
}
```

**Missing fields:** `is_snoozed`, `is_pinned`, `is_muted` are thread-level flags not carried in `UnifiedSearchResult`. For smart folder display this is acceptable — these flags affect the thread card UI minimally (snooze icon, pin icon). To fix properly, extend `UnifiedSearchResult` with these fields. The SQL paths already have them (they come from `DbThread`); the Tantivy path defaults to `false`. This is a minor enhancement — not blocking.

### 2.3 Token Migration

The `__LAST_7_DAYS__`, `__LAST_30_DAYS__`, `__TODAY__` token system in `crates/smart-folder/src/tokens.rs` is superseded by the parser's native relative offset support (`after:-7`, `after:-30`, `after:0`).

#### DB migration

Add a SQLite migration in `crates/db/src/db/migrations.rs`:

```sql
-- Migrate smart folder queries from token syntax to offset syntax.
UPDATE smart_folders SET query = REPLACE(query, '__LAST_7_DAYS__', '-7')
    WHERE query LIKE '%__LAST_7_DAYS__%';
UPDATE smart_folders SET query = REPLACE(query, '__LAST_30_DAYS__', '-30')
    WHERE query LIKE '%__LAST_30_DAYS__%';
UPDATE smart_folders SET query = REPLACE(query, '__TODAY__', '0')
    WHERE query LIKE '%__TODAY__%';
```

#### Backward compatibility

Keep `resolve_query_tokens()` for one release cycle as a fallback. In `execute_smart_folder_query`, call it before `search()` to handle any un-migrated queries:

```rust
let resolved = resolve_query_tokens(params.query);
let results = search(&resolved, search_state, conn)?;
```

Remove `resolve_query_tokens()` and `tokens.rs` after the migration is confirmed complete (no queries in the wild use the old format).

### 2.4 Smart Folder Selection in Sidebar

When the user clicks a smart folder in the sidebar:

1. The search bar fills with the smart folder's query string.
2. The thread list shows the smart folder's results.
3. `active_smart_folder_id` is set so "Update Smart Folder" knows which folder to update.

#### Event flow

Sidebar emits a new event:

```rust
pub enum SidebarEvent {
    // ... existing variants ...
    SmartFolderSelected { id: i64, query: String },
}
```

`App` handles:

```rust
SidebarEvent::SmartFolderSelected { id, query } => {
    self.search_query = query;
    self.thread_list.search_query = self.search_query.clone();
    self.active_smart_folder_id = Some(id);

    // Store pre-search state for Escape restoration
    if matches!(self.thread_list.mode, ThreadListMode::Folder) {
        self.pre_search_threads = Some(self.thread_list.threads.clone());
    }

    // Execute the smart folder query
    self.search_generation += 1;
    let generation = self.search_generation;
    let db = Arc::clone(&self.db);
    let search_state = self.search_state.clone();
    let query = self.search_query.clone();

    Task::perform(
        async move {
            let result = execute_search_with_state(db, search_state, query).await;
            (generation, result)
        },
        |(g, result)| Message::SearchResultsLoaded(g, result),
    )
}
```

#### Editing a smart folder query

When the search bar shows a smart folder's query and the user modifies it:
- Results update live (via the normal debounce -> search -> generational tracking flow).
- The modified query is ephemeral — not auto-saved.
- `active_smart_folder_id` remains set, so "Update Smart Folder" is available in the command palette.

### 2.5 Smart Folder CRUD via Command Palette

Smart folder management moves from the settings UI to the command palette. The settings-based smart folder editor is removed.

#### "Save as Smart Folder"

Available when the search bar has a non-empty query (regardless of whether it came from a smart folder or ad-hoc search):

```rust
Command {
    id: "save_as_smart_folder",
    label: "Save as Smart Folder",
    available: |ctx| !ctx.search_query.is_empty(),
    action: CommandAction::PromptInput {
        placeholder: "Smart folder name...",
        on_confirm: |name, ctx| {
            // INSERT INTO smart_folders (name, query, icon) VALUES (?, ?, NULL)
            // Refresh sidebar
        },
    },
}
```

#### "Update Smart Folder"

Available when `active_smart_folder_id` is `Some` and the query has been modified from the saved version:

```rust
Command {
    id: "update_smart_folder",
    label: "Update Smart Folder",
    available: |ctx| ctx.active_smart_folder_id.is_some(),
    action: CommandAction::Immediate(|ctx| {
        // UPDATE smart_folders SET query = ? WHERE id = ?
        // using ctx.search_query and ctx.active_smart_folder_id
    }),
}
```

#### "Delete Smart Folder" / "Rename Smart Folder"

Available when a smart folder is selected:

```rust
Command {
    id: "delete_smart_folder",
    label: "Delete Smart Folder",
    available: |ctx| ctx.active_smart_folder_id.is_some(),
    action: CommandAction::Confirm {
        message: "Delete this smart folder?",
        on_confirm: |ctx| {
            // DELETE FROM smart_folders WHERE id = ?
            // Clear active_smart_folder_id, restore folder view
        },
    },
}

Command {
    id: "rename_smart_folder",
    label: "Rename Smart Folder",
    available: |ctx| ctx.active_smart_folder_id.is_some(),
    action: CommandAction::PromptInput {
        placeholder: "New name...",
        on_confirm: |name, ctx| {
            // UPDATE smart_folders SET name = ? WHERE id = ?
        },
    },
}
```

### 2.6 Smart Folder Unread Counts

Currently scaffolded as 0 in `get_navigation_state()`. Wire them using `count_smart_folder_unread()`:

```rust
for folder in &mut nav_state.smart_folders {
    folder.unread_count = count_smart_folder_unread(
        conn,
        &folder.query,
        &AccountScope::All, // Smart folders always run cross-account
    )
    .unwrap_or(0);
}
```

**Performance concern:** Each smart folder requires a SQL query for its unread count. With 10 smart folders, that's 10 queries per sidebar refresh. Mitigation:

1. **Batch on startup.** Compute all smart folder unread counts in a single `spawn_blocking` call during `AccountsLoaded` handling.
2. **Refresh on thread mutation.** When a thread is marked read/unread/archived, recompute affected smart folder counts.
3. **Do not recompute on every sidebar render.** Cache the counts in sidebar state and only refresh on explicit triggers.

---

## Phase 3: Operator Typeahead

### 3.1 Overview

When the cursor is inside an operator value (e.g., `from:ali|`), a popup appears below the search bar showing matches from the relevant data source. This requires:

1. **Cursor-local token detection** — identifying which operator the cursor is positioned inside.
2. **Per-operator data source routing** — querying the appropriate DB table.
3. **Popup rendering** — an overlay anchored below the search bar.
4. **Selection interaction** — arrow keys to navigate, Enter to select, Escape to dismiss.

### 3.2 Cursor Token Detection

Given a query string and a cursor position (byte offset), determine which operator (if any) the cursor is inside, and extract the partial value typed so far.

```rust
/// Result of analyzing cursor position within a query string.
#[derive(Debug, Clone)]
pub enum CursorContext {
    /// Cursor is in free text (no operator context).
    FreeText,
    /// Cursor is inside an operator value.
    InsideOperator {
        /// The operator name (e.g., "from", "to", "label").
        operator: String,
        /// The partial value typed so far (e.g., "ali" from "from:ali").
        partial_value: String,
        /// Byte offset where the operator value starts in the query string.
        value_start: usize,
        /// Byte offset where the partial value ends (cursor position).
        value_end: usize,
    },
}

/// Analyze the cursor position in a query string to determine operator context.
pub fn analyze_cursor_context(query: &str, cursor_pos: usize) -> CursorContext {
    // Walk backward from cursor_pos to find the nearest `operator:` prefix.
    // If found and no whitespace between the colon and cursor, we're inside
    // that operator's value.
    // ...
}
```

This function lives in `crates/smart-folder/src/parser.rs` alongside the main parser, since it shares knowledge of operator names.

### 3.3 Typeahead State

Add typeahead state to the search bar's state:

```rust
/// State for the operator typeahead popup.
#[derive(Debug, Clone, Default)]
pub struct TypeaheadState {
    /// Whether the popup is visible.
    pub visible: bool,
    /// The operator context that triggered the popup.
    pub context: Option<CursorContext>,
    /// Matching items from the data source.
    pub items: Vec<TypeaheadItem>,
    /// Currently highlighted item index.
    pub selected: usize,
}

#[derive(Debug, Clone)]
pub struct TypeaheadItem {
    /// Display label (e.g., "Alice Smith").
    pub label: String,
    /// Secondary text (e.g., "asmith@corp.com").
    pub detail: Option<String>,
    /// The value to insert into the query when selected.
    pub insert_value: String,
}
```

### 3.4 Per-Operator Data Sources

| Operator | Query function | Data |
|----------|---------------|------|
| `from:` | `SELECT display_name, email FROM contacts WHERE display_name LIKE ? OR email LIKE ? LIMIT 10` | Contact name + email |
| `to:` | Same as `from:` | Contact name + email |
| `account:` | `SELECT display_name, email FROM accounts WHERE display_name LIKE ? OR email LIKE ? LIMIT 10` | Account name |
| `label:` | `SELECT DISTINCT name FROM labels WHERE name LIKE ? AND account_id IN (?) LIMIT 10` | Label names, scoped by `account:` if present |
| `folder:` | `SELECT DISTINCT name FROM labels WHERE (name LIKE ? OR imap_folder_path LIKE ?) AND account_id IN (?) LIMIT 10` | Folder names, scoped by `account:` |
| `in:` | Static list: `inbox`, `sent`, `drafts`, `trash`, `spam`, `starred`, `snoozed` | Universal folder names |
| `is:` | Static list: `unread`, `read`, `starred`, `snoozed`, `pinned`, `muted`, `tagged` | Flag names |
| `has:` | Static list: `attachment`, `pdf`, `image`, `excel`, `word`, `document`, `archive`, `video`, `audio`, `calendar`, `contact` | Has values |
| `before:` / `after:` | Static presets: Today, Yesterday, Last 7 days, Last 30 days, Last 3 months, Last year | Date presets |

#### Account scoping for label/folder typeahead

When the query already contains an `account:` operator, `label:` and `folder:` typeahead results are scoped to that account. Parse the query up to the cursor position to extract any `account:` values, resolve them to account IDs, and pass those IDs as a filter to the label/folder queries.

When no `account:` is present, return results from all accounts. If the same label name exists on multiple accounts, append the account name in the `detail` field for disambiguation: "Clients (Work Account)".

### 3.5 Date Presets

For `before:` and `after:`, show a popup with common presets:

```rust
const DATE_PRESETS: &[(&str, &str)] = &[
    ("Today", "0"),
    ("Yesterday", "-1"),
    ("Last 7 days", "-7"),
    ("Last 30 days", "-30"),
    ("Last 3 months", "-90"),
    ("Last year", "-365"),
];
```

Selecting a preset inserts the offset value (e.g., `after:-7`). The user can also type a date directly, which dismisses the popup.

### 3.6 Popup Rendering

The typeahead popup is rendered as an iced overlay anchored below the search bar. It uses the same overlay positioning infrastructure as the existing dropdown/popover widgets (see `UI.md` note on popover positioning with translation).

```rust
fn typeahead_popup<'a>(
    state: &'a TypeaheadState,
) -> Element<'a, ThreadListMessage> {
    if !state.visible || state.items.is_empty() {
        return Space::new().into();
    }

    let mut list = column![].spacing(0);
    for (i, item) in state.items.iter().enumerate() {
        let is_selected = i == state.selected;
        let item_row = typeahead_item_view(item, is_selected);
        list = list.push(
            button(item_row)
                .on_press(ThreadListMessage::TypeaheadSelect(i))
                .style(if is_selected {
                    theme::ButtonClass::NavActive.style()
                } else {
                    theme::ButtonClass::NavInactive.style()
                })
                .width(Length::Fill),
        );
    }

    // "Keep as text" option at the bottom
    let keep_text = button(
        text("Keep as text")
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style()),
    )
    .on_press(ThreadListMessage::TypeaheadDismiss)
    .width(Length::Fill);

    container(column![list, rule::horizontal(1), keep_text])
        .style(theme::ContainerClass::Elevated.style())
        .width(Length::Fill)
        .max_height(300.0)
        .into()
}
```

### 3.7 Typeahead Message Variants

```rust
#[derive(Debug, Clone)]
pub enum ThreadListMessage {
    // ... existing ...
    TypeaheadSelect(usize),
    TypeaheadDismiss,
    TypeaheadNavigate(TypeaheadDirection),
    TypeaheadItemsLoaded(Vec<TypeaheadItem>),
}

#[derive(Debug, Clone)]
pub enum TypeaheadDirection {
    Up,
    Down,
}
```

### 3.8 Selection Behavior

When the user selects a typeahead item:

1. Determine the `value_start` and `value_end` from the `CursorContext`.
2. Replace the substring `query[value_start..value_end]` with the selected item's `insert_value`.
3. If the `insert_value` contains spaces, wrap it in quotes: `from:"Alice Smith"`.
4. Append a trailing space after the inserted value so the cursor moves to the next position.
5. Dismiss the popup.
6. Trigger a search execution (the query changed).

```rust
fn apply_typeahead_selection(
    query: &str,
    context: &CursorContext,
    item: &TypeaheadItem,
) -> String {
    if let CursorContext::InsideOperator { value_start, value_end, .. } = context {
        let value = if item.insert_value.contains(' ') {
            format!("\"{}\" ", item.insert_value)
        } else {
            format!("{} ", item.insert_value)
        };
        format!("{}{}{}", &query[..*value_start], value, &query[*value_end..])
    } else {
        query.to_string()
    }
}
```

---

## Phase 4: "Search Here" and Polish

### 4.1 "Search Here" Sidebar Interaction

Right-clicking a folder or label in the sidebar prefills the search bar with scope operators. This is implemented as a command palette action available via right-click context on sidebar items.

#### Sidebar event

```rust
pub enum SidebarEvent {
    // ... existing ...
    SearchHere {
        /// Pre-built query prefix, e.g. "account:FooCorp folder:Projects "
        query_prefix: String,
    },
}
```

#### Building the query prefix

The query prefix depends on what was right-clicked and the current scope:

```rust
fn build_search_here_prefix(
    item: &SidebarItem,
    scope: &SidebarScope,
) -> String {
    match item {
        // Universal folder in a specific account scope
        SidebarItem::UniversalFolder { name, .. } if scope.is_single_account() => {
            let account_name = scope.account_display_name();
            format!("account:{} in:{} ", quote_if_needed(account_name), name.to_lowercase())
        }
        // Universal folder in All Accounts scope
        SidebarItem::UniversalFolder { name, .. } => {
            format!("in:{} ", name.to_lowercase())
        }
        // Account-specific label
        SidebarItem::Label { name, account_name, .. } => {
            format!("account:{} label:{} ", quote_if_needed(account_name), quote_if_needed(name))
        }
        // Account-specific folder
        SidebarItem::Folder { name, account_name, .. } => {
            format!("account:{} folder:{} ", quote_if_needed(account_name), quote_if_needed(name))
        }
        _ => String::new(),
    }
}

fn quote_if_needed(s: &str) -> String {
    if s.contains(' ') {
        format!("\"{s}\"")
    } else {
        s.to_string()
    }
}
```

#### App handling

```rust
SidebarEvent::SearchHere { query_prefix } => {
    self.search_query = query_prefix;
    self.thread_list.search_query = self.search_query.clone();
    self.active_smart_folder_id = None;

    // Store pre-search state
    if matches!(self.thread_list.mode, ThreadListMode::Folder) {
        self.pre_search_threads = Some(self.thread_list.threads.clone());
    }

    // Focus the search bar — cursor at end, ready for typing
    iced::widget::operation::focus("search-bar".to_string())

    // Do NOT execute search yet — the prefix alone may return
    // too many results. Wait for the user to type something.
    // However, if the prefix forms a valid query (e.g., "in:inbox"),
    // it could be executed immediately. Decision: execute immediately
    // so the user sees filtered results, matching the smart folder
    // selection behavior.
}
```

### 4.2 Smart Folder and Search Interaction Summary

The interaction model when a smart folder is selected:

| State | Search bar | Thread list | Active smart folder |
|-------|-----------|-------------|-------------------|
| Smart folder clicked | Shows saved query | Shows query results | Set to folder ID |
| User modifies query | Shows modified query | Results update live | Still set (enables "Update") |
| User presses Escape | Clears | Returns to pre-search folder view | Cleared |
| User saves via palette | Shows saved query | No change | Updated if renamed/query changed |
| User clicks different sidebar item | Clears | Shows new folder's threads | Cleared |

### 4.3 Pinned Search Integration Point

Phase 4 establishes the integration point for pinned searches (`docs/search/pinned-searches.md`). Every search execution that produces results creates or updates a pinned search entry. This spec does not implement pinned searches — it defines the hook:

```rust
Message::SearchResultsLoaded(_, Ok(threads)) if !threads.is_empty() => {
    // ... existing result handling ...

    // Hook for pinned searches (Phase: pinned-searches spec)
    // self.upsert_pinned_search(&self.search_query, &thread_ids);

    Task::none()
}
```

The pinned search spec will define `upsert_pinned_search`, the sidebar section, and the staleness label.

### 4.4 Staleness Label for Pinned Searches

When a pinned search is active, a staleness label appears below the search bar: "Last updated 3 days ago". This is rendered conditionally:

```rust
fn staleness_label<'a>(updated_at: Option<i64>) -> Element<'a, ThreadListMessage> {
    match updated_at {
        Some(ts) => {
            let relative = format_relative_time(ts); // "3 days ago", "2 hours ago"
            text(format!("Last updated {relative}"))
                .size(TEXT_XS)
                .style(theme::TextClass::Tertiary.style())
                .into()
        }
        None => Space::new().into(),
    }
}
```

The `format_relative_time` function uses `chrono-humanize` or a simple custom implementation.

This label is only visible when `active_pinned_search_id` is `Some` — a field added by the pinned searches spec, not this one.

---

## Cross-Cutting Concerns

### Error Handling

All search errors (`String` from the unified pipeline) are displayed in `self.status` and do not crash the app. The thread list remains in its current state on error — it does not clear.

### Thread Selection After Search

When search results load, `selected_thread` is reset to `None`. The reading pane clears. The user can click a result to open it, which follows the existing `SelectThread` flow.

Pressing `Down` from the search bar should select the first result. This requires the search bar's `on_submit` (Enter) to also set focus to the thread list:

```rust
ThreadListMessage::SearchSubmit => {
    // Emit SearchExecute event AND move focus to first result
    (Task::none(), Some(ThreadListEvent::SearchExecute))
}
```

The Enter -> Down flow is: Enter executes the search (or the debounce already did), then Down (or Enter again) selects the first result. This is the standard keyboard model for search interfaces.

### No Loading Spinners

The product spec mandates that search must feel instant. The local search pipeline (Tantivy + SQLite) should complete in single-digit milliseconds for typical mailbox sizes. Therefore:

- No loading spinner in the thread list during search.
- No "Searching..." placeholder text.
- The thread list updates in place when results arrive.

If search ever takes long enough to be perceptible (which would indicate a bug or an unusually large mailbox), the generational tracking ensures correctness — the user sees the results of whatever they last typed, not an intermediate state.

### Accessibility

- The search bar has a widget ID (`"search-bar"`) for programmatic focus.
- Typeahead items are navigable via arrow keys.
- The active typeahead item has visual distinction (same as `NavActive` button style).
- Screen reader labels: the search bar should have an accessible label ("Search emails"), and typeahead items should announce their label and detail.

---

## Summary of File Changes

### New files
- None. All changes are additions to existing files.

### Modified files (by phase)

**Phase 1:**
- `crates/app/src/main.rs` — `App` struct (new fields: `search_generation`, `search_query`, `search_state`, `active_smart_folder_id`, `search_debounce_deadline`, `pre_search_threads`), `Message` enum (new variants), `update()` handlers, `subscription()` keyboard listener, `execute_search()` async fn
- `crates/app/src/ui/thread_list.rs` — `ThreadListMode` enum, `ThreadList` struct (new fields: `mode`, `search_query`), `ThreadListMessage` (new variants), `ThreadListEvent` (new variants), `thread_list_header()` rewrite with real text_input
- `crates/app/src/ui/layout.rs` — No changes needed (existing `PAD_INPUT`, `TEXT_MD` etc. are sufficient)
- `crates/core/src/search_pipeline.rs` — Add `has_attachments` to `UnifiedSearchResult`

**Phase 2:**
- `crates/smart-folder/src/lib.rs` — `execute_smart_folder_query()` signature change (add `&SearchState`), internals changed to call unified pipeline
- `crates/smart-folder/src/tokens.rs` — Retained for one release cycle, then removed
- `crates/db/src/db/migrations.rs` — Token migration SQL
- `crates/app/src/ui/sidebar.rs` — `SidebarEvent::SmartFolderSelected` variant
- `crates/app/src/main.rs` — Smart folder selection handler

**Phase 3:**
- `crates/smart-folder/src/parser.rs` — `analyze_cursor_context()` function
- `crates/app/src/ui/thread_list.rs` — `TypeaheadState`, typeahead message variants, popup rendering, selection logic

**Phase 4:**
- `crates/app/src/ui/sidebar.rs` — `SidebarEvent::SearchHere`, right-click handling, `build_search_here_prefix()`
- `crates/app/src/main.rs` — `SearchHere` event handler
