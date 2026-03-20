# Pinned Searches: Implementation Spec

## Overview

This document specifies the implementation of pinned searches in Ratatoskr. Pinned searches are ephemeral, user-curated search result snapshots that live at the top of the sidebar. Every search execution automatically creates one; they persist across restarts and can be promoted to smart folders.

**Product spec:** `docs/search/pinned-searches.md`
**Depends on:** Search app integration (slices 5-6 of `docs/search/app-integration-spec.md`)
**Placement in roadmap:** Tier 3 (`docs/implementation-plan.md`)

The implementation is organized into four phases:
1. Schema + CRUD + sidebar rendering
2. Lifecycle state machine + search bar integration
3. Graduation to smart folder
4. Auto-expiry

---

## Phase 1: Schema, CRUD, and Sidebar Rendering

### 1.1 SQLite Schema

Pinned searches live in the main `ratatoskr.db` database. They are local-only state (no cross-device sync). Two tables:

```sql
CREATE TABLE IF NOT EXISTS pinned_searches (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    query TEXT NOT NULL,
    created_at INTEGER NOT NULL,  -- unix timestamp (seconds)
    updated_at INTEGER NOT NULL   -- unix timestamp (seconds)
);

-- Unique constraint prevents duplicate queries.
-- On conflict, the caller updates the existing row instead.
CREATE UNIQUE INDEX IF NOT EXISTS idx_pinned_searches_query
    ON pinned_searches(query);

CREATE TABLE IF NOT EXISTS pinned_search_threads (
    pinned_search_id INTEGER NOT NULL
        REFERENCES pinned_searches(id) ON DELETE CASCADE,
    thread_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    PRIMARY KEY (pinned_search_id, thread_id, account_id)
);
```

The `idx_pinned_searches_query` unique index enforces deduplication at the database level. When the same query string is searched again after navigating away, the application detects the existing row and updates it rather than inserting a duplicate.

**Migration:** Add a migration function to `Db::open()` that creates these tables if they don't exist. Use `CREATE TABLE IF NOT EXISTS` for forward compatibility with existing databases.

**Foreign keys:** `pinned_search_threads.pinned_search_id` cascades on delete, so dismissing a pinned search automatically cleans up its thread snapshot. The `thread_id` and `account_id` columns are NOT foreign keys to the `threads` table because threads may be deleted by sync while the pinned search persists — the query simply returns fewer results.

### 1.2 Core Types

Add to `crates/app/src/db.rs`:

```rust
/// A pinned search with its stored thread snapshot.
#[derive(Debug, Clone)]
pub struct PinnedSearch {
    pub id: i64,
    pub query: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// Thread IDs in the snapshot. Not loaded eagerly — populated
    /// only when the pinned search is selected.
    pub thread_ids: Vec<(String, String)>,  // (thread_id, account_id)
}
```

The `thread_ids` field is populated lazily. When listing pinned searches for the sidebar, only `id`, `query`, `created_at`, and `updated_at` are loaded. The thread ID list is loaded when the user clicks a pinned search.

### 1.3 CRUD Functions

Add to `Db` in `crates/app/src/db.rs`. All functions use `self.with_conn()` for async access via `spawn_blocking`, consistent with the existing pattern.

#### `create_or_update_pinned_search`

Creates a new pinned search or updates an existing one if the query string matches.

```rust
impl Db {
    /// Creates a pinned search, or updates the existing one if
    /// `query` already exists. Returns the pinned search ID.
    pub async fn create_or_update_pinned_search(
        &self,
        query: String,
        thread_ids: Vec<(String, String)>,
    ) -> Result<i64, String> {
        self.with_conn(move |conn| {
            let now = chrono::Utc::now().timestamp();

            // Check for existing pinned search with this query
            let existing_id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM pinned_searches WHERE query = ?1",
                    params![query],
                    |row| row.get(0),
                )
                .ok();

            let pinned_id = if let Some(id) = existing_id {
                // Update existing: refresh timestamp and thread snapshot
                conn.execute(
                    "UPDATE pinned_searches SET updated_at = ?1 WHERE id = ?2",
                    params![now, id],
                )
                .map_err(|e| e.to_string())?;
                id
            } else {
                // Insert new
                conn.execute(
                    "INSERT INTO pinned_searches (query, created_at, updated_at)
                     VALUES (?1, ?2, ?2)",
                    params![query, now],
                )
                .map_err(|e| e.to_string())?;
                conn.last_insert_rowid()
            };

            // Replace thread snapshot: delete old, insert new
            conn.execute(
                "DELETE FROM pinned_search_threads
                 WHERE pinned_search_id = ?1",
                params![pinned_id],
            )
            .map_err(|e| e.to_string())?;

            let mut stmt = conn
                .prepare(
                    "INSERT INTO pinned_search_threads
                        (pinned_search_id, thread_id, account_id)
                     VALUES (?1, ?2, ?3)",
                )
                .map_err(|e| e.to_string())?;

            for (thread_id, account_id) in &thread_ids {
                stmt.execute(params![pinned_id, thread_id, account_id])
                    .map_err(|e| e.to_string())?;
            }

            Ok(pinned_id)
        })
        .await
    }
}
```

#### `update_pinned_search`

Updates an existing pinned search's query and thread snapshot. Used when the user edits a query in-place.

**Conflict case:** If the new query matches a *different* existing pinned search (unique index conflict), the update must merge: delete the other row and keep this one. This can happen when the user edits a pinned search's query to something they searched before. The merge preserves the current pinned search's identity (it stays selected in the sidebar) and removes the stale duplicate.

```rust
impl Db {
    /// Updates a pinned search's query string and thread snapshot.
    /// If the new query conflicts with another pinned search, the
    /// conflicting row is deleted (merge behavior).
    pub async fn update_pinned_search(
        &self,
        id: i64,
        query: String,
        thread_ids: Vec<(String, String)>,
    ) -> Result<(), String> {
        self.with_conn(move |conn| {
            let now = chrono::Utc::now().timestamp();

            // Check for a different pinned search with this query
            let conflict_id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM pinned_searches WHERE query = ?1 AND id != ?2",
                    params![query, id],
                    |row| row.get(0),
                )
                .ok();
            if let Some(cid) = conflict_id {
                // Merge: delete the conflicting row (CASCADE deletes its threads)
                conn.execute("DELETE FROM pinned_searches WHERE id = ?1", params![cid])
                    .map_err(|e| e.to_string())?;
            }

            conn.execute(
                "UPDATE pinned_searches
                 SET query = ?1, updated_at = ?2
                 WHERE id = ?3",
                params![query, now, id],
            )
            .map_err(|e| e.to_string())?;

            conn.execute(
                "DELETE FROM pinned_search_threads
                 WHERE pinned_search_id = ?1",
                params![id],
            )
            .map_err(|e| e.to_string())?;

            let mut stmt = conn
                .prepare(
                    "INSERT INTO pinned_search_threads
                        (pinned_search_id, thread_id, account_id)
                     VALUES (?1, ?2, ?3)",
                )
                .map_err(|e| e.to_string())?;

            for (thread_id, account_id) in &thread_ids {
                stmt.execute(params![id, thread_id, account_id])
                    .map_err(|e| e.to_string())?;
            }

            Ok(())
        })
        .await
    }
}
```

#### `delete_pinned_search`

```rust
impl Db {
    pub async fn delete_pinned_search(&self, id: i64) -> Result<(), String> {
        self.with_conn(move |conn| {
            conn.execute(
                "DELETE FROM pinned_searches WHERE id = ?1",
                params![id],
            )
            .map_err(|e| e.to_string())?;
            // CASCADE handles pinned_search_threads cleanup
            Ok(())
        })
        .await
    }
}
```

#### `delete_all_pinned_searches`

```rust
impl Db {
    pub async fn delete_all_pinned_searches(&self) -> Result<(), String> {
        self.with_conn(move |conn| {
            conn.execute("DELETE FROM pinned_searches", [])
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
    }
}
```

#### `list_pinned_searches`

Returns all pinned searches ordered by most recently updated, without loading thread IDs.

```rust
impl Db {
    pub async fn list_pinned_searches(
        &self,
    ) -> Result<Vec<PinnedSearch>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, query, created_at, updated_at
                     FROM pinned_searches
                     ORDER BY updated_at DESC",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map([], |row| {
                Ok(PinnedSearch {
                    id: row.get("id")?,
                    query: row.get("query")?,
                    created_at: row.get("created_at")?,
                    updated_at: row.get("updated_at")?,
                    thread_ids: Vec::new(), // loaded lazily
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }
}
```

#### `get_pinned_search_thread_ids`

Loads the thread ID snapshot for a specific pinned search.

```rust
impl Db {
    pub async fn get_pinned_search_thread_ids(
        &self,
        pinned_search_id: i64,
    ) -> Result<Vec<(String, String)>, String> {
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT thread_id, account_id
                     FROM pinned_search_threads
                     WHERE pinned_search_id = ?1",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map(params![pinned_search_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }
}
```

#### `get_threads_by_ids`

Fetches live thread metadata for a set of thread IDs. This is the query that powers the thread list when a pinned search is selected.

```rust
impl Db {
    /// Fetches threads by (thread_id, account_id) pairs with current
    /// metadata from the database. Threads that no longer exist in the
    /// database are silently omitted.
    pub async fn get_threads_by_ids(
        &self,
        ids: Vec<(String, String)>,
    ) -> Result<Vec<Thread>, String> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        self.with_conn(move |conn| {
            // Build a VALUES clause for the thread IDs.
            // Using a temporary table or CTE for the join:
            //
            //   WITH target_ids(thread_id, account_id) AS (
            //       VALUES (?1, ?2), (?3, ?4), ...
            //   )
            //   SELECT t.*, m.from_name, m.from_address
            //   FROM target_ids ti
            //   JOIN threads t ON t.id = ti.thread_id
            //       AND t.account_id = ti.account_id
            //   LEFT JOIN messages m ON ...
            //   ORDER BY t.last_message_at DESC
            //
            // For large snapshots (100+ threads), chunking the VALUES
            // clause avoids SQLite's variable limit (999 by default).

            let chunk_size = 400; // 2 params per ID, stay under 999
            let mut results = Vec::with_capacity(ids.len());

            for chunk in ids.chunks(chunk_size) {
                let placeholders: Vec<String> = chunk
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        let p1 = i * 2 + 1;
                        let p2 = i * 2 + 2;
                        format!("(?{p1}, ?{p2})")
                    })
                    .collect();
                let values_clause = placeholders.join(", ");

                let sql = format!(
                    "WITH target_ids(tid, aid) AS (VALUES {values_clause})
                     SELECT t.*, m.from_name, m.from_address
                     FROM target_ids ti
                     JOIN threads t ON t.id = ti.tid
                         AND t.account_id = ti.aid
                     LEFT JOIN messages m
                         ON m.account_id = t.account_id
                         AND m.thread_id = t.id
                         AND m.date = (
                             SELECT MAX(m2.date) FROM messages m2
                             WHERE m2.account_id = t.account_id
                               AND m2.thread_id = t.id
                         )
                     GROUP BY t.account_id, t.id
                     ORDER BY t.last_message_at DESC"
                );

                let mut stmt =
                    conn.prepare(&sql).map_err(|e| e.to_string())?;

                let params: Vec<Box<dyn rusqlite::types::ToSql>> = chunk
                    .iter()
                    .flat_map(|(tid, aid)| {
                        vec![
                            Box::new(tid.clone()) as Box<dyn rusqlite::types::ToSql>,
                            Box::new(aid.clone()) as Box<dyn rusqlite::types::ToSql>,
                        ]
                    })
                    .collect();

                let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                    params.iter().map(|p| p.as_ref()).collect();

                let rows = stmt
                    .query_map(param_refs.as_slice(), row_to_thread)
                    .map_err(|e| e.to_string())?;

                for row in rows {
                    results.push(row.map_err(|e| e.to_string())?);
                }
            }

            Ok(results)
        })
        .await
    }
}
```

#### `expire_stale_pinned_searches`

Used by the auto-expiry system (Phase 4), but the function is defined here for completeness.

```rust
impl Db {
    /// Removes pinned searches that are older than `max_age_secs`
    /// and haven't been accessed (updated_at == created_at means
    /// never clicked/refreshed since creation).
    pub async fn expire_stale_pinned_searches(
        &self,
        max_age_secs: i64,
    ) -> Result<u64, String> {
        self.with_conn(move |conn| {
            let cutoff = chrono::Utc::now().timestamp() - max_age_secs;
            let deleted = conn
                .execute(
                    "DELETE FROM pinned_searches
                     WHERE updated_at < ?1
                       AND updated_at = created_at",
                    params![cutoff],
                )
                .map_err(|e| e.to_string())?;
            #[allow(clippy::cast_sign_loss)]
            Ok(deleted as u64)
        })
        .await
    }
}
```

**Important:** The condition `updated_at = created_at` identifies pinned searches that have never been clicked or refreshed since creation. Any user interaction (clicking, refreshing, editing) updates `updated_at`, making it differ from `created_at`. This ensures actively used pinned searches are never expired regardless of age.

### 1.4 Database Initialization

The `Db::open()` function currently sets `PRAGMA query_only = ON`, which prevents writes. Pinned searches require writes.

**Cross-cutting architecture note:** This is not just a pinned-search concern. Multiple features need local-state writes: pinned searches, attachment collapse state (`thread_ui_state`), window session restore, keybinding overrides, and usage tracking. The writable-connection decision should be made as a broader app DB architecture choice, not driven by this spec alone. This spec assumes the solution exists and uses it — it does not own the decision.

**Recommended approach (for whatever feature drives the decision first):** A separate writable connection for local app state, keeping `query_only` on the main connection for synced data safety:

```rust
pub struct Db {
    conn: Arc<Mutex<Connection>>,          // read-only (sync data)
    local_conn: Arc<Mutex<Connection>>,    // writable (local state)
}
```

The `local_conn` opens the same database file but without `PRAGMA query_only = ON`. Schema creation (`CREATE TABLE IF NOT EXISTS`) runs on `local_conn` during `Db::open()`. All pinned search CRUD functions use `local_conn`. Read queries that only touch `pinned_searches` and `pinned_search_threads` can use either connection; the `get_threads_by_ids` function that joins against `threads` uses the read-only `conn`.

Add a `with_local_conn` helper:

```rust
impl Db {
    pub async fn with_local_conn<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.local_conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
            f(&conn)
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
    }
}
```

All pinned search write operations (`create_or_update_pinned_search`, `update_pinned_search`, `delete_pinned_search`, `delete_all_pinned_searches`, `expire_stale_pinned_searches`) use `with_local_conn`. Read operations (`list_pinned_searches`, `get_pinned_search_thread_ids`) can use either `with_conn` or `with_local_conn`.

### 1.5 App State

Add to `App` in `crates/app/src/main.rs`:

```rust
struct App {
    // ... existing fields ...

    /// All pinned searches, loaded at boot. Ordered by updated_at DESC.
    pinned_searches: Vec<PinnedSearch>,

    /// The currently selected pinned search, if any.
    active_pinned_search: Option<i64>,

    /// The folder/label the user was viewing before activating a pinned
    /// search. Restored on Escape.
    pre_search_view: Option<PreSearchView>,
}
```

The `PreSearchView` captures what to restore when the user presses Escape. This restores by re-navigating to an explicit navigation target rather than replaying cached thread state — an improvement over the search app integration spec's `pre_search_threads` clone approach (which that spec labels as a V1 shortcut). Both specs should converge on this navigation-target-based restoration model.

```rust
/// Captures the sidebar state before a pinned search was activated,
/// so pressing Escape can restore the previous view.
#[derive(Debug, Clone)]
pub struct PreSearchView {
    /// The selected account index (None = All Accounts).
    pub selected_account: Option<usize>,
    /// The selected label/folder ID (None = Inbox).
    pub selected_label: Option<String>,
}
```

**Alignment note:** The search app integration spec (`docs/search/app-integration-spec.md`) currently uses `pre_search_threads: Option<Vec<Thread>>` (clone the thread list, restore on Escape). This spec uses navigation-state restoration instead, which is more robust. The search integration spec should adopt this approach — re-navigate to the saved `PreSearchView` target rather than replaying stale cached threads.

### 1.6 Message Variants

Add to the `Message` enum in `crates/app/src/main.rs`:

```rust
#[derive(Debug, Clone)]
pub enum Message {
    // ... existing variants ...

    // ── Pinned searches ─────────────────────────────
    PinnedSearchesLoaded(Result<Vec<db::PinnedSearch>, String>),
    SelectPinnedSearch(i64),
    PinnedSearchThreadIdsLoaded(u64, i64, Result<Vec<(String, String)>, String>),
    PinnedSearchThreadsLoaded(u64, Result<Vec<db::Thread>, String>),
    DismissPinnedSearch(i64),
    PinnedSearchDismissed(i64, Result<(), String>),
}
```

Add to `SidebarMessage` in `crates/app/src/ui/sidebar.rs`:

```rust
#[derive(Debug, Clone)]
pub enum SidebarMessage {
    // ... existing variants ...
    SelectPinnedSearch(i64),
    DismissPinnedSearch(i64),
}
```

Add to `SidebarEvent`:

```rust
#[derive(Debug, Clone)]
pub enum SidebarEvent {
    // ... existing variants ...
    PinnedSearchSelected(i64),
    PinnedSearchDismissed(i64),
}
```

### 1.7 Boot Sequence

In `App::boot()`, load pinned searches alongside accounts:

```rust
fn boot() -> (Self, Task<Message>) {
    // ... existing setup ...
    let db_ref2 = Arc::clone(&db);
    (app, Task::batch([
        Task::perform(
            async move { (load_gen, load_accounts(db_ref).await) },
            |(g, result)| Message::AccountsLoaded(g, result),
        ),
        Task::perform(
            async move { db_ref2.list_pinned_searches().await },
            Message::PinnedSearchesLoaded,
        ),
    ]))
}
```

Handler:

```rust
Message::PinnedSearchesLoaded(Ok(searches)) => {
    self.pinned_searches = searches;
    Task::none()
}
Message::PinnedSearchesLoaded(Err(e)) => {
    self.status = format!("Pinned searches error: {e}");
    Task::none()
}
```

### 1.8 Sidebar Rendering

#### Sidebar state additions

Add to `Sidebar` struct:

```rust
pub struct Sidebar {
    // ... existing fields ...
    pub pinned_searches: Vec<db::PinnedSearch>,
    pub active_pinned_search: Option<i64>,
}
```

**Ownership model:** `App` is the source of truth for `pinned_searches` and `active_pinned_search`. The sidebar does not own separate copies — it holds references to the App-owned data. In practice, since iced's Elm architecture passes data into `view()` functions, the sidebar's `view()` receives pinned search data as parameters from the parent rather than maintaining internal duplicates. The `Sidebar` struct's `pinned_searches` and `active_pinned_search` fields are written by `App` before each `view()` call — they are downstream mirrors, not independent state. If the sidebar component pattern requires internal fields, they should be documented as "set by parent, not independently mutated."

#### View function

The sidebar's `view()` function renders pinned searches at the top, between the scope dropdown and the compose button:

```rust
fn view(&self) -> Element<'_, SidebarMessage> {
    let mut col = column![]
        .spacing(0)
        .width(Length::Fill);

    // Scope dropdown
    col = col.push(scope_dropdown(self));
    col = col.push(Space::new().height(SPACE_XXS));

    // Pinned searches (only if non-empty)
    if !self.pinned_searches.is_empty() {
        col = col.push(pinned_searches_section(self));
        col = col.push(Space::new().height(SPACE_XXS));
    }

    // Compose button
    col = col.push(widgets::compose_button(SidebarMessage::Compose));
    col = col.push(Space::new().height(SPACE_XS));

    // Universal folders, smart folders, labels (unchanged)
    col = col.push(nav_items(self));
    col = col.push(widgets::section_break());
    col = col.push(smart_folders(self.smart_folders_expanded));

    if !self.is_all_accounts() {
        col = col.push(widgets::section_break::<SidebarMessage>());
        col = col.push(labels(self));
    }

    container(
        column![
            scrollable(col)
                .spacing(SCROLLBAR_SPACING)
                .height(Length::Fill),
            widgets::settings_button(SidebarMessage::ToggleSettings),
        ]
        .spacing(SPACE_XS),
    )
    .padding(PAD_SIDEBAR)
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::ContainerClass::Sidebar.style())
    .into()
}
```

#### Pinned search card widget

Add a new rendering function in `sidebar.rs` (not `widgets.rs`, because this is sidebar-specific assembly using domain data):

```rust
fn pinned_searches_section(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let mut col = column![].spacing(SPACE_XXS);

    for ps in &sidebar.pinned_searches {
        col = col.push(pinned_search_card(
            ps,
            sidebar.active_pinned_search == Some(ps.id),
        ));
    }

    col.into()
}
```

Each pinned search card is a two-line button with a dismiss (X) button:

```rust
fn pinned_search_card(
    ps: &db::PinnedSearch,
    active: bool,
) -> Element<'_, SidebarMessage> {
    // Line 1: date+time (primary label)
    let date_label = format_pinned_search_date(ps.updated_at);

    // Line 2: query string (muted subtitle, truncated)
    let query_display = truncate_query(&ps.query, 28);

    let date_style: fn(&Theme) -> text::Style = if active {
        text::primary
    } else {
        text::base
    };
    let query_style: fn(&Theme) -> text::Style = if active {
        text::secondary
    } else {
        theme::TextClass::Muted.style()
    };

    let text_col = column![
        text(date_label).size(TEXT_MD).style(date_style),
        text(query_display)
            .size(TEXT_SM)
            .style(query_style)
            .wrapping(text::Wrapping::None),
    ]
    .spacing(SPACE_XXXS)
    .width(Length::Fill);

    let dismiss_btn = button(
        container(
            icon::x().size(ICON_XS).style(theme::TextClass::Muted.style()),
        )
        .center(Length::Shrink),
    )
    .on_press(SidebarMessage::DismissPinnedSearch(ps.id))
    .padding(SPACE_XXXS)
    .style(theme::ButtonClass::BareIcon.style());

    let content = row![text_col, dismiss_btn]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Start);

    button(
        container(content).padding(PAD_NAV_ITEM),
    )
    .on_press(SidebarMessage::SelectPinnedSearch(ps.id))
    .padding(0)
    .style(theme::ButtonClass::PinnedSearch { active }.style())
    .width(Length::Fill)
    .into()
}
```

#### Helper functions

```rust
/// Formats a unix timestamp as "Mar 19, 14:32" for the pinned search card.
fn format_pinned_search_date(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|dt| dt.format("%b %d, %H:%M").to_string())
        .unwrap_or_else(|| "Unknown".to_string())
}

/// Truncates a query string for display, adding ellipsis if needed.
fn truncate_query(query: &str, max_chars: usize) -> String {
    if query.len() <= max_chars {
        query.to_string()
    } else {
        format!("{}...", &query[..query.floor_char_boundary(max_chars)])
    }
}
```

### 1.9 Button Style: `PinnedSearch`

Add a new variant to `ButtonClass` in `crates/app/src/ui/theme.rs`:

```rust
pub enum ButtonClass {
    // ... existing variants ...
    /// Pinned search card in the sidebar.
    PinnedSearch { active: bool },
}
```

Implementation:

```rust
fn style_pinned_search_button(
    theme: &Theme,
    status: button::Status,
    active: bool,
) -> button::Style {
    let p = theme.palette();
    match status {
        button::Status::Hovered => button::Style {
            background: Some(p.background.weak.color.into()),
            text_color: p.background.base.text,
            border: iced::Border {
                color: p.background.strongest.color.scale_alpha(0.1),
                width: 1.0,
                radius: RADIUS_MD.into(),
            },
            ..Default::default()
        },
        _ => button::Style {
            background: Some(if active {
                p.background.strong.color.into()
            } else {
                p.background.weakest.color.into()
            }),
            text_color: if active {
                p.primary.base.color
            } else {
                p.background.base.text
            },
            border: iced::Border {
                color: p.background.strongest.color.scale_alpha(0.08),
                width: 1.0,
                radius: RADIUS_MD.into(),
            },
            ..Default::default()
        },
    }
}
```

The card uses `background.weakest` as its resting background (one step up from the sidebar's `background.weaker`), with `RADIUS_MD` for subtle card elevation. The active state uses `background.strong` with accent text, consistent with the `Nav { active: true }` pattern. The 1px border at very low alpha creates the "chip-like" container effect specified in the product spec.

### 1.10 Event Handling

In `App::handle_sidebar_event`:

```rust
SidebarEvent::PinnedSearchSelected(id) => {
    // Save current view for Escape restoration
    if self.active_pinned_search.is_none() {
        self.pre_search_view = Some(PreSearchView {
            selected_account: self.sidebar.selected_account,
            selected_label: self.sidebar.selected_label.clone(),
        });
    }

    self.active_pinned_search = Some(id);
    self.sidebar.active_pinned_search = Some(id);
    self.sidebar.selected_label = None; // deselect folder

    // Bump nav_generation to invalidate stale loads
    self.nav_generation += 1;
    self.thread_generation += 1;
    self.thread_list.selected_thread = None;

    // Load thread IDs for this pinned search
    let db = Arc::clone(&self.db);
    let load_gen = self.nav_generation;
    Task::perform(
        async move {
            let ids = db.get_pinned_search_thread_ids(id).await;
            (load_gen, id, ids)
        },
        |(g, id, result)| Message::PinnedSearchThreadIdsLoaded(g, id, result),
    )
}
SidebarEvent::PinnedSearchDismissed(id) => {
    let db = Arc::clone(&self.db);
    Task::perform(
        async move {
            let result = db.delete_pinned_search(id).await;
            (id, result)
        },
        |(id, result)| Message::PinnedSearchDismissed(id, result),
    )
}
```

Thread ID loading chain:

```rust
Message::PinnedSearchThreadIdsLoaded(g, _, _) if g != self.nav_generation => {
    Task::none()
}
Message::PinnedSearchThreadIdsLoaded(_, ps_id, Ok(ids)) => {
    // Store thread IDs on the pinned search entry
    if let Some(ps) = self.pinned_searches.iter_mut().find(|p| p.id == ps_id) {
        ps.thread_ids = ids.clone();
    }

    // Load live thread metadata
    let db = Arc::clone(&self.db);
    let load_gen = self.nav_generation;
    Task::perform(
        async move {
            let result = db.get_threads_by_ids(ids).await;
            (load_gen, result)
        },
        |(g, result)| Message::PinnedSearchThreadsLoaded(g, result),
    )
}
Message::PinnedSearchThreadIdsLoaded(_, _, Err(e)) => {
    self.status = format!("Error loading pinned search: {e}");
    Task::none()
}

Message::PinnedSearchThreadsLoaded(g, _) if g != self.nav_generation => {
    Task::none()
}
Message::PinnedSearchThreadsLoaded(_, Ok(threads)) => {
    self.status = format!("{} threads (pinned search)", threads.len());
    self.thread_list.set_threads(threads);
    Task::none()
}
Message::PinnedSearchThreadsLoaded(_, Err(e)) => {
    self.status = format!("Threads error: {e}");
    Task::none()
}

Message::PinnedSearchDismissed(id, Ok(())) => {
    self.pinned_searches.retain(|ps| ps.id != id);
    self.sidebar.pinned_searches.retain(|ps| ps.id != id);
    if self.active_pinned_search == Some(id) {
        self.active_pinned_search = None;
        self.sidebar.active_pinned_search = None;
        // Restore previous view if the dismissed one was active
        if let Some(prev) = self.pre_search_view.take() {
            self.sidebar.selected_account = prev.selected_account;
            self.sidebar.selected_label = prev.selected_label;
            // Reload threads for the restored view
            return self.reload_threads_for_current_view();
        }
    }
    Task::none()
}
Message::PinnedSearchDismissed(_, Err(e)) => {
    self.status = format!("Dismiss error: {e}");
    Task::none()
}
```

### 1.11 Generational Load Tracking

Pinned search thread loads use the existing `nav_generation` counter. When the user clicks a pinned search, `nav_generation` is incremented. If the user clicks another pinned search or navigates away before the first load completes, the stale result is discarded by the generation guard:

```rust
Message::PinnedSearchThreadsLoaded(g, _) if g != self.nav_generation => Task::none(),
```

This is the same pattern already used for `AccountsLoaded`, `LabelsLoaded`, and `ThreadsLoaded`.

### 1.12 Thread List Context

When a pinned search is active, update the thread list's context display:

```rust
// In the pinned search selection handler, after loading threads:
self.thread_list.set_context(
    format!("Search: {}", truncate_query(&ps.query, 30)),
    "All Accounts".to_string(),
);
```

---

## Phase 2: Lifecycle State Machine and Search Bar Integration

This phase implements the core lifecycle: automatic creation on search execution, edit-in-place, navigate-away detection, refresh, and Escape behavior. It depends on the search app integration (slices 5-6) being complete, since pinned searches are created from search results.

### 2.1 Lifecycle State

Add to `App`:

```rust
struct App {
    // ... existing fields ...

    /// Tracks whether the current search context is "owned" by an
    /// existing pinned search. When `Some(id)`, edits to the search
    /// bar update that pinned search in place. Set to `None` when the
    /// user navigates away (clicks a folder, label, or different
    /// pinned search).
    editing_pinned_search: Option<i64>,
}
```

The distinction between `active_pinned_search` and `editing_pinned_search`:

- `active_pinned_search` — which pinned search is highlighted in the sidebar and whose threads are displayed. Set when clicking a pinned search or when a new search creates one.
- `editing_pinned_search` — which pinned search should be updated (rather than a new one created) when the user executes a search. Set when a pinned search is selected or when a new search is created. Cleared when the user navigates away.

They are usually the same value, but `active_pinned_search` can be `Some(id)` while `editing_pinned_search` is `None` (the user clicked a folder after viewing a pinned search — the pinned search is still highlighted briefly until the new view loads, but further searches should create new entries).

### 2.2 State Transitions

```
                                    ┌──────────────────────┐
                                    │                      │
                  ┌─────────────────▶  No Active Search    │
                  │                 │  editing = None      │
                  │                 │  active = None       │
                  │                 └───────┬──────────────┘
                  │                         │
                  │ Escape / Navigate       │ Execute search
                  │ to folder               │ (Enter / debounce)
                  │                         ▼
                  │                 ┌──────────────────────┐
                  │                 │                      │
                  ├─────────────────│  Search Active       │
                  │                 │  editing = Some(id)  │
                  │                 │  active = Some(id)   │
                  │                 └───────┬──────┬───────┘
                  │                         │      │
                  │         Edit query +    │      │ Click different
                  │         re-execute      │      │ pinned search
                  │         (updates in     │      │
                  │          place)         │      ▼
                  │                         │  ┌────────────────────┐
                  │                         │  │                    │
                  │                         │  │ Pinned Search      │
                  │                         │  │ Viewing            │
                  │                         │  │ editing = Some(id) │
                  │                         │  │ active = Some(id)  │
                  │                         │  └────────────────────┘
                  │                         │
                  │         Navigate away   │
                  │         (folder click)  │
                  │                         ▼
                  │                 ┌──────────────────────┐
                  │                 │                      │
                  └─────────────────│  Navigated Away      │
                                    │  editing = None      │
                                    │  active = None       │
                                    └──────────────────────┘
```

### 2.3 Search Execution Handler

New `Message` variants (added to the enum from Phase 1):

```rust
#[derive(Debug, Clone)]
pub enum Message {
    // ... existing + Phase 1 variants ...

    /// Search was executed from the search bar. The search app
    /// integration provides the query string and result thread IDs.
    SearchExecuted(String, Vec<(String, String)>),

    /// Pinned search was created or updated after a search execution.
    PinnedSearchSaved(Result<i64, String>),
}
```

Handler for `SearchExecuted`:

```rust
Message::SearchExecuted(query, thread_ids) => {
    let db = Arc::clone(&self.db);
    let query_clone = query.clone();
    let ids_clone = thread_ids.clone();

    if let Some(editing_id) = self.editing_pinned_search {
        // Edit in place: update the existing pinned search
        Task::perform(
            async move {
                db.update_pinned_search(editing_id, query_clone, ids_clone)
                    .await
                    .map(|()| editing_id)
            },
            Message::PinnedSearchSaved,
        )
    } else {
        // New search: create or deduplicate
        Task::perform(
            async move {
                db.create_or_update_pinned_search(query_clone, ids_clone).await
            },
            Message::PinnedSearchSaved,
        )
    }
}

Message::PinnedSearchSaved(Ok(id)) => {
    // Reload the pinned search list to reflect the change
    self.active_pinned_search = Some(id);
    self.sidebar.active_pinned_search = Some(id);
    self.editing_pinned_search = Some(id);

    let db = Arc::clone(&self.db);
    Task::perform(
        async move { db.list_pinned_searches().await },
        Message::PinnedSearchesLoaded,
    )
}

Message::PinnedSearchSaved(Err(e)) => {
    self.status = format!("Save pinned search error: {e}");
    Task::none()
}
```

### 2.4 Navigate-Away Detection

When the user clicks a folder, label, smart folder, or a different pinned search, clear the editing state:

In `handle_sidebar_event`, for all navigation events:

```rust
SidebarEvent::AccountSelected(idx) => {
    self.clear_pinned_search_context();
    // ... existing handler ...
}
SidebarEvent::AllAccountsSelected => {
    self.clear_pinned_search_context();
    // ... existing handler ...
}
SidebarEvent::LabelSelected(label_id) => {
    self.clear_pinned_search_context();
    // ... existing handler ...
}
```

The helper:

```rust
impl App {
    fn clear_pinned_search_context(&mut self) {
        self.active_pinned_search = None;
        self.sidebar.active_pinned_search = None;
        self.editing_pinned_search = None;
        self.pre_search_view = None;
    }
}
```

When clicking a pinned search, set `editing_pinned_search` to the new one:

```rust
SidebarEvent::PinnedSearchSelected(id) => {
    self.editing_pinned_search = Some(id);
    // ... rest of handler from Phase 1 ...
}
```

### 2.5 Search Bar Integration

The search bar is part of the thread list panel (above the thread list, per the search spec). When a pinned search is active, the search bar needs:

1. **Query text:** Pre-filled with the stored query string.
2. **Staleness label:** Shows relative time since last update.
3. **Edit + execute behavior:** Updates the pinned search in place.
4. **Escape behavior:** Clears the search bar and restores the previous folder view.

#### Search bar state

The thread list component (or a new search bar component) needs to know:

```rust
/// State pushed from App to the search bar when a pinned search is active.
#[derive(Debug, Clone)]
pub struct SearchBarState {
    /// The query to display in the search bar.
    pub query: String,
    /// If this search is from a pinned search, the staleness label.
    pub staleness: Option<String>,
}
```

#### Staleness formatting

Add a helper for relative time formatting:

```rust
/// Formats a relative time string for the staleness label.
/// Examples: "Just now", "5 minutes ago", "2 hours ago", "3 days ago"
fn format_relative_time(timestamp: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let diff_secs = now - timestamp;

    if diff_secs < 60 {
        "Just now".to_string()
    } else if diff_secs < 3600 {
        let mins = diff_secs / 60;
        if mins == 1 {
            "1 minute ago".to_string()
        } else {
            format!("{mins} minutes ago")
        }
    } else if diff_secs < 86400 {
        let hours = diff_secs / 3600;
        if hours == 1 {
            "1 hour ago".to_string()
        } else {
            format!("{hours} hours ago")
        }
    } else {
        let days = diff_secs / 86400;
        if days == 1 {
            "1 day ago".to_string()
        } else {
            format!("{days} days ago")
        }
    }
}
```

This avoids adding `chrono-humanize` as a dependency for a simple formatting function. The format matches what the product spec requests.

#### Staleness label rendering

The staleness label renders below the search bar as a small, muted text element:

```rust
// In the thread list header / search bar area:
if let Some(staleness) = &search_bar_state.staleness {
    let label = format!("Last updated {staleness}");
    text(label)
        .size(TEXT_XS)
        .style(theme::TextClass::Tertiary.style())
}
```

#### Escape handling

When the search bar has focus and the user presses Escape:

1. Clear the search bar text.
2. Deselect the active pinned search (but do NOT dismiss it).
3. Restore the previous folder view from `pre_search_view`.

New message variant:

```rust
SearchBarEscaped => {
    // Clear search context but don't dismiss the pinned search
    self.active_pinned_search = None;
    self.sidebar.active_pinned_search = None;
    self.editing_pinned_search = None;

    // Restore previous view
    if let Some(prev) = self.pre_search_view.take() {
        self.sidebar.selected_account = prev.selected_account;
        self.sidebar.selected_label = prev.selected_label.clone();
        self.update_thread_list_context_from_sidebar();
        return self.reload_threads_for_current_view();
    }
    Task::none()
}
```

### 2.6 Refresh on Click

When the user clicks a pinned search that is already active and presses Enter (re-executes the query), this is handled by the existing `SearchExecuted` flow: `editing_pinned_search` is `Some(id)`, so the existing entry is updated rather than creating a new one.

### 2.7 Thread List Rendering

When a pinned search is selected, the thread list renders identically to a folder view. The threads are fetched via `get_threads_by_ids` and set with `self.thread_list.set_threads(threads)`. The existing `thread_card` widget in `widgets.rs` renders each thread — no special handling needed.

Thread cards show live metadata (read/unread, starred, snippet, date) because `get_threads_by_ids` fetches current state from the `threads` table. The pinned search only determines *which* threads appear, not *how* they look.

No unread badges appear on pinned search entries in the sidebar. This is implicit — the `pinned_search_card` function simply doesn't include a badge.

---

## Phase 3: Graduation to Smart Folder

### 3.1 Command Palette Integration

Register a context-sensitive command:

```rust
Command {
    id: "save_as_smart_folder",
    label: "Save as Smart Folder",
    keywords: &["save", "smart", "folder", "pin", "promote"],
    available: |ctx| ctx.active_pinned_search.is_some(),
}
```

The `CommandContext` struct (from `crates/core/src/command_palette/`) already includes contextual state. Add:

```rust
pub struct CommandContext {
    // ... existing fields ...
    pub active_pinned_search: Option<i64>,
}
```

### 3.2 Graduation Flow

When the user selects "Save as Smart Folder" from the command palette:

1. The palette enters a second stage prompting for a name (text input).
2. On confirmation, the app:
   a. Creates a new smart folder with the pinned search's query string.
   b. Deletes the pinned search.
   c. Navigates to the new smart folder.

New message variants:

```rust
#[derive(Debug, Clone)]
pub enum Message {
    // ... existing variants ...
    GraduatePinnedSearch(i64, String),  // (pinned_search_id, smart_folder_name)
    PinnedSearchGraduated(Result<(), String>),
}
```

Handler:

```rust
Message::GraduatePinnedSearch(ps_id, name) => {
    let query = self.pinned_searches
        .iter()
        .find(|ps| ps.id == ps_id)
        .map(|ps| ps.query.clone());

    let Some(query) = query else {
        return Task::none();
    };

    let db = Arc::clone(&self.db);
    Task::perform(
        async move {
            // Create smart folder with the query
            db.create_smart_folder(name, query).await?;
            // Delete the pinned search
            db.delete_pinned_search(ps_id).await?;
            Ok(())
        },
        Message::PinnedSearchGraduated,
    )
}

Message::PinnedSearchGraduated(Ok(())) => {
    self.clear_pinned_search_context();
    // Reload sidebar (pinned searches + smart folders)
    self.reload_sidebar()
}

Message::PinnedSearchGraduated(Err(e)) => {
    self.status = format!("Graduation error: {e}");
    Task::none()
}
```

### 3.3 Smart Folder Creation

The `create_smart_folder` function needs to be added to `Db` if it doesn't already exist. It inserts a row into the `smart_folders` table:

```rust
impl Db {
    pub async fn create_smart_folder(
        &self,
        name: String,
        query: String,
    ) -> Result<(), String> {
        self.with_local_conn(move |conn| {
            conn.execute(
                "INSERT INTO smart_folders (name, query, sort_order)
                 VALUES (?1, ?2, (
                     SELECT COALESCE(MAX(sort_order), 0) + 1
                     FROM smart_folders
                 ))",
                params![name, query],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
    }
}
```

---

## Phase 4: Auto-Expiry

### 4.1 Expiry Policy

Pinned searches older than 14 days that have never been clicked since creation are silently removed. The condition:

```sql
DELETE FROM pinned_searches
WHERE updated_at < :cutoff
  AND updated_at = created_at
```

The `updated_at = created_at` guard ensures that any user interaction (clicking to view, refreshing, editing the query) marks the pinned search as "touched" and exempts it from auto-expiry.

**14 days in seconds:** `14 * 24 * 60 * 60 = 1_209_600`

### 4.2 Trigger Points

Auto-expiry runs at two points:

1. **App startup:** After `PinnedSearchesLoaded` completes, run expiry.
2. **Periodic:** Once per day while the app is running.

#### Startup expiry

In the `PinnedSearchesLoaded` handler:

```rust
Message::PinnedSearchesLoaded(Ok(searches)) => {
    self.pinned_searches = searches.clone();
    self.sidebar.pinned_searches = searches;

    // Run auto-expiry
    let db = Arc::clone(&self.db);
    Task::perform(
        async move {
            let expired = db
                .expire_stale_pinned_searches(1_209_600)
                .await;
            expired
        },
        Message::PinnedSearchesExpired,
    )
}
```

New message:

```rust
Message::PinnedSearchesExpired(Ok(count)) => {
    if count > 0 {
        // Reload the list to remove expired entries
        let db = Arc::clone(&self.db);
        Task::perform(
            async move { db.list_pinned_searches().await },
            Message::PinnedSearchesLoaded,
        )
    } else {
        Task::none()
    }
}
Message::PinnedSearchesExpired(Err(e)) => {
    // Non-fatal — log and continue
    self.status = format!("Expiry warning: {e}");
    Task::none()
}
```

Note: The reload after expiry will trigger `PinnedSearchesLoaded` again, which would re-trigger expiry. Guard against infinite loops by only running expiry on the first load:

```rust
struct App {
    // ... existing fields ...
    expiry_ran: bool,
}
```

```rust
Message::PinnedSearchesLoaded(Ok(searches)) => {
    self.pinned_searches = searches.clone();
    self.sidebar.pinned_searches = searches;

    if !self.expiry_ran {
        self.expiry_ran = true;
        let db = Arc::clone(&self.db);
        return Task::perform(
            async move { db.expire_stale_pinned_searches(1_209_600).await },
            Message::PinnedSearchesExpired,
        );
    }
    Task::none()
}
```

#### Periodic expiry

Use an iced `Subscription` that fires once per day:

```rust
// In App::subscription():
fn subscription(&self) -> iced::Subscription<Message> {
    let mut subs = vec![
        // ... existing subscriptions ...
    ];

    // Daily auto-expiry tick
    subs.push(
        iced::time::every(std::time::Duration::from_secs(86400))
            .map(|_| Message::RunPinnedSearchExpiry),
    );

    iced::Subscription::batch(subs)
}
```

New message:

```rust
Message::RunPinnedSearchExpiry => {
    let db = Arc::clone(&self.db);
    Task::perform(
        async move { db.expire_stale_pinned_searches(1_209_600).await },
        Message::PinnedSearchesExpired,
    )
}
```

---

## Appendix A: Complete Message Enum Additions

All new `Message` variants introduced across the four phases:

```rust
#[derive(Debug, Clone)]
pub enum Message {
    // ... existing variants ...

    // Phase 1: Schema + CRUD + Sidebar
    PinnedSearchesLoaded(Result<Vec<db::PinnedSearch>, String>),
    SelectPinnedSearch(i64),
    PinnedSearchThreadIdsLoaded(u64, i64, Result<Vec<(String, String)>, String>),
    PinnedSearchThreadsLoaded(u64, Result<Vec<db::Thread>, String>),
    DismissPinnedSearch(i64),
    PinnedSearchDismissed(i64, Result<(), String>),

    // Phase 2: Lifecycle + Search Bar
    SearchExecuted(String, Vec<(String, String)>),
    PinnedSearchSaved(Result<i64, String>),
    SearchBarEscaped,

    // Phase 3: Graduation
    GraduatePinnedSearch(i64, String),
    PinnedSearchGraduated(Result<(), String>),

    // Phase 4: Auto-Expiry
    PinnedSearchesExpired(Result<u64, String>),
    RunPinnedSearchExpiry,
}
```

## Appendix B: Complete SidebarMessage/Event Additions

```rust
#[derive(Debug, Clone)]
pub enum SidebarMessage {
    // ... existing variants ...
    SelectPinnedSearch(i64),
    DismissPinnedSearch(i64),
}

#[derive(Debug, Clone)]
pub enum SidebarEvent {
    // ... existing variants ...
    PinnedSearchSelected(i64),
    PinnedSearchDismissed(i64),
}
```

## Appendix C: New Types Summary

```rust
// crates/app/src/db.rs
#[derive(Debug, Clone)]
pub struct PinnedSearch {
    pub id: i64,
    pub query: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub thread_ids: Vec<(String, String)>,
}

// crates/app/src/main.rs
#[derive(Debug, Clone)]
pub struct PreSearchView {
    pub selected_account: Option<usize>,
    pub selected_label: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SearchBarState {
    pub query: String,
    pub staleness: Option<String>,
}
```

## Appendix D: New Layout Constants

Add to `crates/app/src/ui/layout.rs` if needed:

```rust
/// Pinned search card internal padding (reuses PAD_NAV_ITEM).
/// No new constant needed — the existing PAD_NAV_ITEM
/// (top: 4, right: 8, bottom: 4, left: 8) is appropriate.
```

No new layout constants are required. The pinned search cards reuse `PAD_NAV_ITEM` for internal padding, `RADIUS_MD` for border radius, `TEXT_MD` for the date label, `TEXT_SM` for the query subtitle, `TEXT_XS` for the staleness label, and `ICON_XS` for the dismiss button icon. All values are from the existing layout scale.

## Appendix E: Files Modified

| File | Changes |
|------|---------|
| `crates/app/src/db.rs` | `PinnedSearch` type, `Db` CRUD functions, `local_conn` field, `with_local_conn`, `get_threads_by_ids` |
| `crates/app/src/main.rs` | `App` fields (`pinned_searches`, `active_pinned_search`, `editing_pinned_search`, `pre_search_view`, `expiry_ran`), new `Message` variants, handlers, boot sequence, subscription |
| `crates/app/src/ui/sidebar.rs` | `SidebarMessage`/`SidebarEvent` variants, `Sidebar` fields, `pinned_searches_section`, `pinned_search_card`, helper functions, `view()` layout change |
| `crates/app/src/ui/theme.rs` | `ButtonClass::PinnedSearch` variant, `style_pinned_search_button` |

No new files are created. All changes are additions to existing files, consistent with the project convention of preferring edits over new files.

## Appendix F: Dependency on Search App Integration

This spec assumes the search app integration (slices 5-6) provides:

1. A `SearchExecuted` event (or equivalent) that carries the query string and the list of matching `(thread_id, account_id)` pairs.
2. A search bar component that can be pre-filled with a query string and supports an Escape handler.
3. The unified `search()` function in core that returns ranked results.

Phase 1 can be partially implemented without the search integration — the sidebar rendering, CRUD, and click-to-view flow work with manually seeded pinned searches. Phase 2 requires the search integration to be complete for automatic creation and edit-in-place behavior.
