# Sidebar: Implementation Spec

Phased implementation spec for the sidebar described in `docs/sidebar/problem-statement.md`. Each phase lists what exists, what must be built, concrete type definitions, file changes, and dependencies.

---

## Table of Contents

1. [Inventory: What Exists vs What Remains](#inventory)
2. [Phase 1A: Live Data Wiring](#phase-1a-live-data-wiring)
3. [Phase 1B: Smart Folder Scoping Fix](#phase-1b-smart-folder-scoping-fix)
4. [Phase 1C: Unread Counts for Smart Folders and Labels](#phase-1c-unread-counts)
5. [Phase 1D: Hierarchy Support](#phase-1d-hierarchy-support)
6. [Phase 1E: Pinned Searches Section](#phase-1e-pinned-searches)
7. [Phase 2: Strip Actions](#phase-2-strip-actions)
8. [Data Flow and Refresh Policy](#data-flow-and-refresh-policy)
9. [Dependency Graph](#dependency-graph)

---

## Inventory

### What exists and works today

| Component | File | Status |
|-----------|------|--------|
| `Sidebar` struct with `Component` trait | `crates/app/src/ui/sidebar.rs` | Working. Owns accounts, labels, selected_account, selected_label, section-expand booleans. |
| `SidebarMessage` / `SidebarEvent` enums | `crates/app/src/ui/sidebar.rs` | Working. SelectAccount, SelectAllAccounts, CycleAccount, SelectLabel, toggle messages. |
| Scope dropdown (Option A) | `crates/app/src/ui/sidebar.rs` `scope_dropdown()` | Working. Shows "All Accounts" or per-account entries using `widgets::dropdown`. |
| Universal folders section | `crates/app/src/ui/sidebar.rs` `nav_items()` | **Hardcoded placeholder counts.** Items list is correct (Inbox, Starred, Snoozed, Sent, Drafts, Trash; Spam/All Mail when scoped). |
| Smart folders section | `crates/app/src/ui/sidebar.rs` `smart_folders()` | **Hardcoded to two fake entries** ("VIP", "Newsletters"). Not driven by DB. |
| Labels section | `crates/app/src/ui/sidebar.rs` `labels()` | Working. Shows up to 12 non-system labels when scoped to an account. Labels loaded via `Db::get_labels`. |
| `get_navigation_state()` | `crates/core/src/db/queries_extra/navigation.rs` | Working. Returns `NavigationState` with universal folders (real unread counts), smart folders (unread=0), account labels (unread=0). |
| Scoped query infrastructure | `crates/core/src/db/queries_extra/scoped_queries.rs` | Working. `AccountScope`, `get_threads_scoped`, `get_unread_counts_by_folder`, `get_draft_count_with_local`, `get_starred_threads`, `get_snoozed_threads`. |
| `count_smart_folder_unread()` | `crates/smart-folder/src/lib.rs` | Working. Takes `(conn, query, scope)`, returns `Result<i64, String>`. |
| `NavigationFolder` / `NavigationState` types | `crates/core/src/db/queries_extra/navigation.rs` | Flat. No `parent_id`, no hierarchy, no label-vs-folder discriminator. |
| `DbLabel` with `imap_folder_path` | `crates/db/src/db/types.rs` | Has `imap_folder_path: Option<String>` and `label_type: Option<String>`. Not exposed through `NavigationFolder`. |
| App `Label` type | `crates/app/src/db.rs` | Minimal: only `id` and `name`. Missing `account_id`, `label_type`, `imap_folder_path`, `color_bg/fg`. |
| `nav_generation` counter | `crates/app/src/main.rs` | Working. Incremented on scope change, used to discard stale `AccountsLoaded`/`LabelsLoaded`/`ThreadsLoaded`. |

### What does not exist

- Sidebar driven by `NavigationState` (live data wiring)
- Smart folder entries from DB (currently hardcoded)
- Smart folder unread counts (backend exists, not wired)
- Per-label unread counts (not computed)
- Hierarchy support (`parent_id` / `path` on `NavigationFolder`)
- Label-vs-folder discriminator on `NavigationFolder`
- Tree view widget for nested folders
- Pinned searches section in sidebar
- `NavigationLoaded` message variant (sidebar data arrives via separate `AccountsLoaded`/`LabelsLoaded` today)

---

## Phase 1A: Live Data Wiring

**Goal:** Replace all hardcoded placeholder data in the sidebar with live data from `get_navigation_state()`. No new backend features - just wiring what already exists.

**Depends on:** Nothing.

**Transitional note:** This phase replaces ad hoc sidebar data with `NavigationState`, but it still uses `selected_label: Option<String>` as a flat destination marker for universal folders, smart folders, and labels alike. That is semantically muddy - a starred view, a smart folder, and a Gmail label are very different navigation targets sharing one `Option<String>`. This is acceptable as a transitional step because the current sidebar already works this way, and refactoring the navigation model is a larger change. However, the app should eventually grow an explicit `NavigationTarget` enum (as proposed in the command palette app-integration spec) that distinguishes between folder types, smart folders, pinned searches, and search results. Phase 1A intentionally does not attempt that refactor - it wires live data into the existing state shape.

### 1A.1 New Message Variant

Add a message to carry the `NavigationState` from the async load to the `update()` handler.

**File: `crates/app/src/main.rs`**

```rust
// Add to Message enum:
NavigationLoaded(u64, Result<NavigationState, String>),
```

The `u64` is the `nav_generation` counter for stale-load rejection (same pattern as `AccountsLoaded`, `LabelsLoaded`, `ThreadsLoaded`).

Import the type:

```rust
use rtsk::db::queries_extra::navigation::NavigationState;
```

Note: the app crate currently uses its own `db.rs` module with local types (`Account`, `Label`, `Thread`). `NavigationState` comes from the core crate. The app already depends on `rtsk` (or will need to). If the dependency is not yet present in `crates/app/Cargo.toml`, add it.

### 1A.2 Sidebar State Changes

Replace `Sidebar`'s data-holding fields with a single `NavigationState` plus the accounts list (which `NavigationState` does not include).

**File: `crates/app/src/ui/sidebar.rs`**

```rust
use rtsk::db::queries_extra::navigation::{
    FolderKind, NavigationFolder, NavigationState,
};

pub struct Sidebar {
    pub accounts: Vec<Account>,
    pub nav_state: Option<NavigationState>,
    pub selected_account: Option<usize>,
    pub selected_label: Option<String>,
    pub scope_dropdown_open: bool,
    pub labels_expanded: bool,
    pub smart_folders_expanded: bool,
}
```

The `labels: Vec<Label>` field is removed. Labels now come from `nav_state.folders` filtered by `FolderKind::AccountLabel`.

### 1A.3 Navigation Load Function

Replace the separate `load_labels` + `load_threads` sequence with a single `load_navigation` that calls `get_navigation_state`.

**File: `crates/app/src/main.rs`**

```rust
async fn load_navigation(
    db: Arc<Db>,
    scope: AccountScope,
) -> Result<NavigationState, String> {
    db.with_conn(move |conn| {
        get_navigation_state(conn, &scope)
    }).await
}
```

`AccountScope` is derived from `selected_account`:

```rust
fn current_scope(&self) -> AccountScope {
    match self.sidebar.selected_account {
        Some(idx) => {
            let Some(account) = self.sidebar.accounts.get(idx) else {
                return AccountScope::All;
            };
            AccountScope::Single(account.id.clone())
        }
        None => AccountScope::All,
    }
}
```

### 1A.4 Fire Navigation Load on Boot and Scope Change

**Boot:** After `AccountsLoaded` succeeds and auto-selects the first account, fire `load_navigation`:

```rust
// In handle_accounts_loaded, after setting accounts:
let scope = self.current_scope();
let db = Arc::clone(&self.db);
let gen = self.nav_generation;
Task::batch([
    Task::perform(
        async move { (gen, load_navigation(db, scope).await) },
        |(g, r)| Message::NavigationLoaded(g, r),
    ),
    // Also load threads for the default folder (Inbox):
    self.load_threads_for_current_view(),
])
```

**Scope change:** In `handle_sidebar_event` for `AccountSelected` and `AllAccountsSelected`, bump `nav_generation` and fire `load_navigation` with the new scope.

### 1A.5 Handle NavigationLoaded

**File: `crates/app/src/main.rs`**

```rust
Message::NavigationLoaded(g, _) if g != self.nav_generation => Task::none(),
Message::NavigationLoaded(_, Ok(nav_state)) => {
    self.sidebar.nav_state = Some(nav_state);
    Task::none()
}
Message::NavigationLoaded(_, Err(e)) => {
    self.status = format!("Navigation error: {e}");
    Task::none()
}
```

### 1A.6 Update Sidebar View to Read from NavigationState

**File: `crates/app/src/ui/sidebar.rs`**

Replace `nav_items()`:

```rust
fn nav_items(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let folders = sidebar.nav_state.as_ref()
        .map(|ns| &ns.folders[..])
        .unwrap_or(&[]);

    let universal: Vec<NavItem<'_>> = folders
        .iter()
        .filter(|f| matches!(f.folder_kind, FolderKind::Universal))
        .filter(|f| {
            // Spam and All Mail only when scoped
            if sidebar.is_all_accounts() {
                !matches!(f.id.as_str(), "SPAM" | "ALL_MAIL")
            } else {
                true
            }
        })
        .map(|f| NavItem {
            label: &f.name,
            id: &f.id,
            unread: f.unread_count,
        })
        .collect();

    widgets::nav_group(
        &universal,
        &sidebar.selected_label,
        SidebarMessage::SelectLabel,
    )
}
```

Replace `smart_folders()`:

```rust
fn smart_folders(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let folders = sidebar.nav_state.as_ref()
        .map(|ns| &ns.folders[..])
        .unwrap_or(&[]);

    let children: Vec<Element<'_, SidebarMessage>> = folders
        .iter()
        .filter(|f| matches!(f.folder_kind, FolderKind::SmartFolder))
        .map(|f| {
            widgets::nav_button(
                None,
                &f.name,
                sidebar.selected_label.as_deref() == Some(&f.id),
                widgets::NavSize::Compact,
                Some(f.unread_count),
                SidebarMessage::SelectLabel(Some(f.id.clone())),
            )
        })
        .collect();

    widgets::collapsible_section(
        "SMART FOLDERS",
        sidebar.smart_folders_expanded,
        SidebarMessage::ToggleSmartFoldersSection,
        children,
    )
}
```

Replace `labels()`:

```rust
fn labels(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let folders = sidebar.nav_state.as_ref()
        .map(|ns| &ns.folders[..])
        .unwrap_or(&[]);

    let children: Vec<Element<'_, SidebarMessage>> = folders
        .iter()
        .filter(|f| matches!(f.folder_kind, FolderKind::AccountLabel))
        .take(12)
        .map(|f| {
            let active = sidebar.selected_label.as_deref() == Some(&f.id);
            widgets::label_nav_item(
                &f.name,
                &f.id,
                theme::avatar_color(&f.name),
                active,
                SidebarMessage::SelectLabel(Some(f.id.clone())),
            )
        })
        .collect();

    widgets::collapsible_section(
        "LABELS",
        sidebar.labels_expanded,
        SidebarMessage::ToggleLabelsSection,
        children,
    )
}
```

### 1A.7 Thread Loading Wired to Scope

When a sidebar folder is selected, the thread list load must use the correct scoped query. The current `load_threads` always passes a single `account_id`. Replace with scope-aware loading.

**File: `crates/app/src/main.rs`**

```rust
fn load_threads_for_current_view(&mut self) -> Task<Message> {
    self.nav_generation += 1;
    let db = Arc::clone(&self.db);
    let scope = self.current_scope();
    let label_id = self.sidebar.selected_label.clone();
    let gen = self.nav_generation;

    Task::perform(
        async move {
            let result = db.with_conn(move |conn| {
                get_threads_scoped(
                    conn, &scope, label_id.as_deref(),
                    Some(50), None,
                )
            }).await;
            (gen, result)
        },
        |(g, result)| Message::ThreadsLoaded(g, result),
    )
}
```

This replaces the current `load_threads(db, account_id, label_id)` pattern with the scoped query infrastructure.

### 1A.8 Sidebar `view()` Adjustment

The `view()` function's conditional label section uses an explicit scope check, not the presence of data in `nav_state`. The product rule is: labels appear only when scoped to a single account. That rule should be enforced in the app, not inferred from backend data - if the backend ever returned account-label-like data in an unexpected context, the UI should not drift.

```rust
fn view(&self) -> Element<'_, SidebarMessage> {
    let show_labels = self.selected_account.is_some();

    let mut col = column![
        scope_dropdown(self),
        Space::new().height(SPACE_XXS),
        widgets::compose_button(SidebarMessage::Compose),
        Space::new().height(SPACE_XS),
        nav_items(self),
        widgets::section_break(),
        smart_folders(self),
    ]
    .spacing(0)
    .width(Length::Fill);

    if show_labels {
        col = col.push(widgets::section_break::<SidebarMessage>());
        col = col.push(labels(self));
    }

    // ... rest unchanged
}
```

### Files Modified (Phase 1A)

| File | Change |
|------|--------|
| `crates/app/Cargo.toml` | Add `rtsk` dependency if not present |
| `crates/app/src/main.rs` | Add `NavigationLoaded` variant, `load_navigation()`, `current_scope()`, update `handle_accounts_loaded`, `handle_sidebar_event`, `handle_label_selected` |
| `crates/app/src/ui/sidebar.rs` | Replace `labels` field with `nav_state`, update `nav_items`, `smart_folders`, `labels`, `view` |

---

## Phase 1B: Smart Folder Scoping Fix

**Goal:** Smart folders must always appear regardless of scope, per the problem statement. Currently `query_smart_folders_sync` in `navigation.rs` filters by scope.

**Depends on:** Nothing (can be done before or after 1A).

### Change

**File: `crates/core/src/db/queries_extra/navigation.rs`**

`build_smart_folders` must ignore the `scope` parameter and always return all smart folders:

```rust
fn build_smart_folders(
    conn: &Connection,
    _scope: &AccountScope,
) -> Result<Vec<NavigationFolder>, String> {
    let smart_folders = query_all_smart_folders_sync(conn)?;
    // ... rest unchanged
}

fn query_all_smart_folders_sync(
    conn: &Connection,
) -> Result<Vec<DbSmartFolder>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT * FROM smart_folders ORDER BY sort_order, created_at",
        )
        .map_err(|e| e.to_string())?;
    stmt.query_map([], DbSmartFolder::from_row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}
```

The old `query_smart_folders_sync` that accepted `scope` is removed. Smart folder query *execution* (when the user clicks one) still uses `AccountScope` -- the change here only affects which smart folders appear in the sidebar list.

### Files Modified (Phase 1B)

| File | Change |
|------|--------|
| `crates/core/src/db/queries_extra/navigation.rs` | Replace `query_smart_folders_sync` with `query_all_smart_folders_sync` |

---

## Phase 1C: Unread Counts

**Goal:** Wire real unread counts for smart folders and per-label items. Currently scaffolded as 0.

**Depends on:** Phase 1A (NavigationState wiring must exist for counts to be visible).

### Smart Folder Unread Counts

The function `count_smart_folder_unread(conn, query, scope)` already exists in `crates/smart-folder/src/lib.rs`. It parses the query, resolves date tokens, builds SQL with an `is_read = 0` filter, and returns a count.

**Cost analysis:** Each smart folder requires one SQL query. With N smart folders, that is N queries per sidebar refresh. For typical usage (2-5 smart folders), this is acceptable. The queries are simple COUNT operations on indexed columns.

**Caching strategy:** Not needed for V1. If profiling shows smart folder counts are slow (e.g., complex queries or many folders), introduce a `smart_folder_unread_cache: HashMap<String, (i64, Instant)>` in the sidebar model with a 30-second TTL. Invalidate on thread mutations and sync completions.

**Change in `crates/core/src/db/queries_extra/navigation.rs`:**

```rust
use smart_folder::count_smart_folder_unread;

fn build_smart_folders(
    conn: &Connection,
    scope: &AccountScope,
) -> Result<Vec<NavigationFolder>, String> {
    let smart_folders = query_all_smart_folders_sync(conn)?;

    smart_folders
        .into_iter()
        .map(|sf| {
            let unread = count_smart_folder_unread(conn, &sf.query, scope)
                .unwrap_or(0);
            Ok(NavigationFolder {
                id: sf.id,
                name: sf.name,
                folder_kind: FolderKind::SmartFolder,
                unread_count: unread,
                account_id: sf.account_id,
            })
        })
        .collect()
}
```

Note: `scope` is passed to `count_smart_folder_unread` because the count should reflect the current view scope (a smart folder with `account:foo` will always return the same count regardless, but a scope-less query like `is:unread after:-7` should count across the active scope). **Wait -- the problem statement says smart folders are scope-exempt.** Their queries run independently of the scope selector. So the scope passed to `count_smart_folder_unread` should always be `AccountScope::All`:

```rust
let unread = count_smart_folder_unread(conn, &sf.query, &AccountScope::All)
    .unwrap_or(0);
```

### Per-Label Unread Counts

Labels appear only when scoped to a single account. The count is per-label, per-account.

**File: `crates/core/src/db/queries_extra/navigation.rs`**

Replace the per-label 0 scaffold in `build_account_labels` with a batched GROUP BY query:

```rust
fn build_account_labels(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<NavigationFolder>, String> {
    let all_labels = get_labels(conn, account_id.to_owned())?;
    let system_ids = system_label_ids();
    let label_counts = get_label_unread_counts(conn, account_id)?;

    Ok(all_labels
        .into_iter()
        .filter(|label| !system_ids.contains(&label.id.as_str()))
        .filter(|label| label.visible)
        .map(|label| {
            let unread = label_counts
                .get(&label.id)
                .copied()
                .unwrap_or(0);
            NavigationFolder {
                id: label.id,
                name: label.name,
                folder_kind: FolderKind::AccountLabel,
                unread_count: unread,
                account_id: Some(label.account_id),
            }
        })
        .collect())
}
```

New helper function:

```rust
use std::collections::HashMap;

fn get_label_unread_counts(
    conn: &Connection,
    account_id: &str,
) -> Result<HashMap<String, i64>, String> {
    let mut stmt = conn.prepare(
        "SELECT tl.label_id, COUNT(*) AS unread_count
         FROM threads t
         INNER JOIN thread_labels tl
           ON tl.account_id = t.account_id AND tl.thread_id = t.id
         WHERE t.account_id = ?1 AND t.is_read = 0
         GROUP BY tl.label_id"
    ).map_err(|e| e.to_string())?;

    let mut counts = HashMap::new();
    let rows = stmt.query_map(rusqlite::params![account_id], |row| {
        Ok((
            row.get::<_, String>("label_id")?,
            row.get::<_, i64>("unread_count")?,
        ))
    }).map_err(|e| e.to_string())?;

    for row in rows {
        let (label_id, count) = row.map_err(|e| e.to_string())?;
        counts.insert(label_id, count);
    }
    Ok(counts)
}
```

This is a single query regardless of label count (batched GROUP BY), so it scales well.

### Files Modified (Phase 1C)

| File | Change |
|------|--------|
| `crates/core/src/db/queries_extra/navigation.rs` | Wire `count_smart_folder_unread`, add `get_label_unread_counts` |
| `crates/core/Cargo.toml` | Ensure `smart-folder` is a dependency |

---

## Phase 1D: Hierarchy Support

**Goal:** Support tree rendering for Exchange/IMAP/JMAP folder hierarchies in the Labels section when scoped to a single account.

**Depends on:** Phase 1A (NavigationState must be wired).

This is the largest piece of work and the biggest ecosystem gap.

**Blast radius warning:** This phase is not just sidebar work. Adding `parent_label_id` to the `labels` table is a schema change that touches sync logic across three provider crates (Graph, JMAP, IMAP), label loading paths, and migration behavior. The sidebar is the motivating use case, but the change is cross-provider data-model evolution. Plan accordingly - the DB migration and provider sync changes should be reviewed and tested independently of the sidebar UI work.

**Gmail stays flat.** Gmail labels are semantically flat tags, not hierarchical folders, even though Gmail's UI visually nests labels whose names contain `/` separators. This spec does not retrofit Gmail labels into a parent/child hierarchy. Gmail's `parent_label_id` is always `NULL`. The tree renderer only activates for providers where `parent_id` data actually exists (Exchange, JMAP, IMAP). If the product decision changes in the future, Gmail visual nesting can be derived from name splitting without schema changes.

### 1D.1 Backend: Extend NavigationFolder

**File: `crates/core/src/db/queries_extra/navigation.rs`**

```rust
/// Whether a navigation item is a non-exclusive tag or an exclusive folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LabelSemantics {
    /// Non-exclusive tag (Gmail labels). A message can have multiple.
    Tag,
    /// Exclusive folder (Exchange, IMAP, JMAP). A message lives in exactly one.
    Folder,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigationFolder {
    pub id: String,
    pub name: String,
    pub folder_kind: FolderKind,
    pub unread_count: i64,
    pub account_id: Option<String>,
    /// Parent folder ID for tree rendering. `None` means top-level.
    pub parent_id: Option<String>,
    /// Tag vs Folder semantics. Only meaningful for `AccountLabel` items.
    pub label_semantics: Option<LabelSemantics>,
}
```

### 1D.2 Backend: Populate Hierarchy from DbLabel

The `DbLabel` type already has `imap_folder_path: Option<String>` and `label_type: Option<String>`. The `labels` table schema also has `type` (stores "user" / "system" for Gmail; could store "folder" for Exchange/IMAP).

**New columns needed on `labels` table:**

```sql
ALTER TABLE labels ADD COLUMN parent_label_id TEXT;
```

This column is populated by each provider's sync logic:

- **Gmail**: always `NULL` (flat labels, even though Gmail shows visual nesting via `/` in label names).
- **Exchange/Graph**: set to the parent folder ID from `parentFolderId` in the Graph API response.
- **JMAP**: set to the parent mailbox ID from `parentId` in the Mailbox object.
- **IMAP**: derived from `imap_folder_path` by splitting on the hierarchy delimiter and looking up the parent path.

**Migration file: `crates/db/src/db/migrations.rs`**

Add a new migration step:

```rust
// Migration N: Add parent_label_id for folder hierarchy
"ALTER TABLE labels ADD COLUMN parent_label_id TEXT;"
```

### 1D.3 Backend: LabelSemantics from Provider

The `accounts` table has a `provider` column (`gmail_api`, `graph`, `jmap`, `imap`). The semantics are:

- `gmail_api` -> `LabelSemantics::Tag`
- `graph`, `jmap`, `imap` -> `LabelSemantics::Folder`

**File: `crates/core/src/db/queries_extra/navigation.rs`**

```rust
fn label_semantics_for_provider(provider: &str) -> LabelSemantics {
    match provider {
        "gmail_api" => LabelSemantics::Tag,
        _ => LabelSemantics::Folder,
    }
}
```

Update `build_account_labels` to look up the account's provider and set `label_semantics` and `parent_id`:

```rust
fn build_account_labels(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<NavigationFolder>, String> {
    let provider = get_account_provider(conn, account_id)?;
    let semantics = label_semantics_for_provider(&provider);
    let all_labels = get_labels(conn, account_id.to_owned())?;
    let system_ids = system_label_ids();
    let label_counts = get_label_unread_counts(conn, account_id)?;

    Ok(all_labels
        .into_iter()
        .filter(|label| !system_ids.contains(&label.id.as_str()))
        .filter(|label| label.visible)
        .map(|label| {
            let unread = label_counts
                .get(&label.id)
                .copied()
                .unwrap_or(0);
            NavigationFolder {
                id: label.id,
                name: label.name,
                folder_kind: FolderKind::AccountLabel,
                unread_count: unread,
                account_id: Some(label.account_id),
                parent_id: label.parent_label_id,
                label_semantics: Some(semantics.clone()),
            }
        })
        .collect())
}

fn get_account_provider(
    conn: &Connection,
    account_id: &str,
) -> Result<String, String> {
    conn.query_row(
        "SELECT provider FROM accounts WHERE id = ?1",
        rusqlite::params![account_id],
        |row| row.get::<_, String>(0),
    )
    .map_err(|e| e.to_string())
}
```

### 1D.4 Backend: Extend DbLabel and get_labels

`DbLabel` already has `imap_folder_path` but not `parent_label_id`. Add it.

**File: `crates/db/src/db/types.rs`**

```rust
pub struct DbLabel {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub label_type: Option<String>,
    pub color_bg: Option<String>,
    pub color_fg: Option<String>,
    pub visible: bool,
    pub sort_order: i64,
    pub imap_folder_path: Option<String>,
    pub imap_special_use: Option<String>,
    pub parent_label_id: Option<String>,  // NEW
}
```

Update the `FromRow` implementation and the `get_labels` query to include `parent_label_id`.

### 1D.5 UI: Tree Rendering

No iced tree view widget exists in the ecosystem. The sidebar must compose one from primitives.

**Approach:** Build a flat-list renderer that uses `parent_id` to compute indent depth and expand/collapse state. This is not a standalone widget (that would be premature abstraction) -- it is a view function in `sidebar.rs`.

**State needed in `Sidebar`:**

```rust
pub struct Sidebar {
    // ... existing fields ...
    /// Set of folder IDs whose children are collapsed (hidden).
    /// When a folder ID is in this set, its descendants are not rendered.
    pub collapsed_folders: HashSet<String>,
}
```

**New message:**

```rust
pub enum SidebarMessage {
    // ... existing ...
    ToggleFolderExpand(String),  // folder_id
}
```

**Tree sort helper** (converts flat list with parent_ids into depth-first display order):

```rust
struct TreeNode<'a> {
    folder: &'a NavigationFolder,
    depth: u16,
}

/// Sort folders into depth-first tree order and compute indent depth.
/// Roots (parent_id == None) come first, then their children recursively.
///
/// Safety: provider data can be messy (missing parents, cycles). Items whose
/// parent_id references a non-existent folder are treated as roots (depth 0).
/// A max-depth cap (e.g., 10) prevents infinite recursion from cycles.
fn tree_sort<'a>(folders: &'a [NavigationFolder]) -> Vec<TreeNode<'a>> {
    let children_of: HashMap<Option<&str>, Vec<&NavigationFolder>> = {
        let mut map: HashMap<Option<&str>, Vec<&NavigationFolder>> = HashMap::new();
        for f in folders {
            map.entry(f.parent_id.as_deref()).or_default().push(f);
        }
        map
    };

    let mut result = Vec::with_capacity(folders.len());
    const MAX_DEPTH: u16 = 10; // cap to prevent cycles
    fn walk<'a>(
        parent: Option<&str>,
        depth: u16,
        children_of: &HashMap<Option<&str>, Vec<&'a NavigationFolder>>,
        result: &mut Vec<TreeNode<'a>>,
    ) {
        if depth > MAX_DEPTH { return; } // cycle protection
        let Some(children) = children_of.get(&parent) else { return };
        for child in children {
            result.push(TreeNode { folder: child, depth });
            walk(Some(&child.id), depth + 1, children_of, result);
        }
    }
    walk(None, 0, &children_of, &mut result);
    // Items with missing parents (parent_id points to non-existent folder)
    // won't appear in the tree. Collect orphans and add as roots:
    let in_tree: HashSet<&str> = result.iter().map(|n| n.folder.id.as_str()).collect();
    for f in folders {
        if !in_tree.contains(f.id.as_str()) {
            result.push(TreeNode { folder: f, depth: 0 });
        }
    }
    result
}
```

**Rendering** in the `labels` function:

```rust
fn labels(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let folders = sidebar.nav_state.as_ref()
        .map(|ns| &ns.folders[..])
        .unwrap_or(&[]);

    let account_labels: Vec<&NavigationFolder> = folders
        .iter()
        .filter(|f| matches!(f.folder_kind, FolderKind::AccountLabel))
        .collect();

    let has_hierarchy = account_labels.iter().any(|f| f.parent_id.is_some());

    let children: Vec<Element<'_, SidebarMessage>> = if has_hierarchy {
        render_label_tree(sidebar, &account_labels)
    } else {
        render_flat_labels(sidebar, &account_labels)
    };

    widgets::collapsible_section(
        "LABELS",
        sidebar.labels_expanded,
        SidebarMessage::ToggleLabelsSection,
        children,
    )
}
```

**Tree item rendering** with indentation:

```rust
/// Indent step per tree depth level.
const TREE_INDENT: f32 = 16.0;  // SPACE_MD from layout.rs

fn render_label_tree<'a>(
    sidebar: &'a Sidebar,
    labels: &[&'a NavigationFolder],
) -> Vec<Element<'a, SidebarMessage>> {
    let tree = tree_sort_refs(labels);
    let mut elements = Vec::new();

    for node in &tree {
        // Skip if any ancestor is collapsed
        if is_hidden_by_collapsed_ancestor(node.folder, labels, &sidebar.collapsed_folders) {
            continue;
        }

        let has_children = labels.iter().any(|f|
            f.parent_id.as_deref() == Some(&node.folder.id)
        );
        let is_collapsed = sidebar.collapsed_folders.contains(&node.folder.id);
        let active = sidebar.selected_label.as_deref() == Some(&node.folder.id);
        let indent = TREE_INDENT * f32::from(node.depth);

        let mut item_row = row![].spacing(SPACE_XXS).align_y(Alignment::Center);

        // Indent spacer
        if indent > 0.0 {
            item_row = item_row.push(Space::new().width(indent));
        }

        // Expand/collapse chevron (only for parent folders)
        if has_children {
            let chevron = if is_collapsed {
                icon::chevron_right()
            } else {
                icon::chevron_down()
            };
            item_row = item_row.push(
                button(chevron.size(ICON_XS).style(theme::TextClass::Tertiary.style()))
                    .on_press(SidebarMessage::ToggleFolderExpand(
                        node.folder.id.clone(),
                    ))
                    .padding(SPACE_XXXS)
                    .style(theme::ButtonClass::Ghost.style()),
            );
        }

        // Color dot + label name
        item_row = item_row.push(
            widgets::color_dot(theme::avatar_color(&node.folder.name))
        );
        let label_style: fn(&Theme) -> text::Style = if active {
            text::primary
        } else {
            text::secondary
        };
        item_row = item_row.push(
            text(&node.folder.name).size(TEXT_MD).style(label_style)
        );

        // Unread badge
        if node.folder.unread_count > 0 {
            item_row = item_row
                .push(Space::new().width(Length::Fill))
                .push(widgets::count_badge(node.folder.unread_count));
        }

        elements.push(
            button(
                container(item_row)
                    .padding(PAD_NAV_ITEM)
                    .width(Length::Fill),
            )
            .on_press(SidebarMessage::SelectLabel(Some(node.folder.id.clone())))
            .padding(0)
            .style(theme::ButtonClass::Nav { active }.style())
            .width(Length::Fill)
            .into(),
        );
    }

    elements
}
```

**Flat label rendering** (Gmail) stays close to the current implementation:

```rust
fn render_flat_labels<'a>(
    sidebar: &'a Sidebar,
    labels: &[&'a NavigationFolder],
) -> Vec<Element<'a, SidebarMessage>> {
    labels
        .iter()
        .take(12)
        .map(|f| {
            let active = sidebar.selected_label.as_deref() == Some(&f.id);
            widgets::label_nav_item(
                &f.name,
                &f.id,
                theme::avatar_color(&f.name),
                active,
                SidebarMessage::SelectLabel(Some(f.id.clone())),
            )
        })
        .collect()
}
```

### 1D.6 Provider Sync Changes

Each provider's sync logic must populate `parent_label_id` when upserting labels. This is provider-specific work:

| Provider | Source field | Notes |
|----------|-------------|-------|
| Gmail (`crates/gmail/`) | Not applicable | Gmail labels are flat. `parent_label_id` stays `NULL`. Gmail's visual nesting (via `/` separator in names) is display-only and irrelevant to hierarchy. |
| Exchange/Graph (`crates/graph/`) | `parentFolderId` from Graph API `mailFolder` resource | Set on every folder sync. The root folder has `parentFolderId` pointing to the well-known root; filter it to `NULL`. |
| JMAP (`crates/jmap/`) | `parentId` from JMAP `Mailbox` object | Direct mapping. `NULL` for top-level mailboxes. |
| IMAP (`crates/imap/`) | Derived from `imap_folder_path` | Split path on hierarchy delimiter (e.g., `INBOX.Clients.Active` -> parent is `INBOX.Clients`). Look up parent's label ID by path. |

### Files Modified (Phase 1D)

| File | Change |
|------|--------|
| `crates/db/src/db/types.rs` | Add `parent_label_id` to `DbLabel` |
| `crates/db/src/db/migrations.rs` | New migration: `ALTER TABLE labels ADD COLUMN parent_label_id TEXT` |
| `crates/db/src/db/from_row_impls.rs` | Update `DbLabel` FromRow impl |
| `crates/core/src/db/queries_extra/navigation.rs` | Add `LabelSemantics`, `parent_id`, `label_semantics` to `NavigationFolder`; update `build_account_labels` |
| `crates/core/src/db/queries/mod.rs` | Update `get_labels` query to include `parent_label_id` |
| `crates/app/src/ui/sidebar.rs` | Add `collapsed_folders`, `ToggleFolderExpand`, tree rendering functions |
| `crates/graph/src/sync.rs` | Populate `parent_label_id` from `parentFolderId` |
| `crates/jmap/src/sync.rs` | Populate `parent_label_id` from `parentId` |
| `crates/imap/src/sync.rs` | Derive `parent_label_id` from `imap_folder_path` |

---

## Phase 1E: Pinned Searches

**Goal:** Render pinned searches at the top of the sidebar, above universal folders.

**Depends on:** Search app integration (Tier 2 in `docs/implementation-plan.md`). Pinned searches require the search bar to be functional. However, the *sidebar rendering* can be scaffolded with a data model and placeholder data before search is wired.

Full lifecycle is in `docs/search/pinned-searches.md`. This section covers only the sidebar rendering integration.

**Important distinction:** Pinned searches are not navigation items and must not inherit sidebar action semantics. They are ephemeral working contexts - visually distinct from folders/labels, not subject to Phase 2 action stripping (dismiss is their own lifecycle, not an "action on email"), and not affected by scope changes. The rendering must reinforce this: card-like containers with elevated background, not nav-button style items.

### 1E.1 Types

**File: `crates/app/src/ui/sidebar.rs`** (or a shared types module)

```rust
/// A pinned search entry for sidebar rendering.
/// Populated from the `pinned_searches` SQLite table.
#[derive(Debug, Clone)]
pub struct PinnedSearchEntry {
    pub id: i64,
    pub query: String,
    pub updated_at: i64,
}
```

### 1E.2 Sidebar State

```rust
pub struct Sidebar {
    // ... existing fields ...
    pub pinned_searches: Vec<PinnedSearchEntry>,
    pub active_pinned_search: Option<i64>,  // id of the selected pinned search
}
```

### 1E.3 Messages and Events

```rust
pub enum SidebarMessage {
    // ... existing ...
    SelectPinnedSearch(i64),
    DismissPinnedSearch(i64),
}

pub enum SidebarEvent {
    // ... existing ...
    PinnedSearchSelected(i64, String),  // (id, query)
    PinnedSearchDismissed(i64),
}
```

### 1E.4 View Function

The pinned searches section appears between the compose button and universal folders. Per the pinned searches spec: no section header, visually distinct cards, dismiss button always visible.

```rust
fn pinned_searches(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    if sidebar.pinned_searches.is_empty() {
        return Space::new().width(0).height(0).into();
    }

    let items: Vec<Element<'_, SidebarMessage>> = sidebar
        .pinned_searches
        .iter()
        .map(|ps| pinned_search_card(ps, sidebar.active_pinned_search == Some(ps.id)))
        .collect();

    column(items).spacing(SPACE_XXS).into()
}

fn pinned_search_card(
    ps: &PinnedSearchEntry,
    active: bool,
) -> Element<'_, SidebarMessage> {
    let relative_time = format_relative_time(ps.updated_at);

    let dismiss = button(
        icon::x().size(ICON_XS).style(theme::TextClass::Muted.style())
    )
    .on_press(SidebarMessage::DismissPinnedSearch(ps.id))
    .padding(SPACE_XXXS)
    .style(theme::ButtonClass::Ghost.style());

    let content = row![
        column![
            text(&ps.query)
                .size(TEXT_SM)
                .style(text::base)
                .wrapping(text::Wrapping::None),
            text(relative_time)
                .size(TEXT_XS)
                .style(theme::TextClass::Muted.style()),
        ]
        .spacing(SPACE_XXXS)
        .width(Length::Fill),
        dismiss,
    ]
    .spacing(SPACE_XXS)
    .align_y(Alignment::Center);

    let card_style = if active {
        theme::ButtonClass::Nav { active: true }
    } else {
        theme::ButtonClass::Nav { active: false }
    };

    button(
        container(content)
            .padding(PAD_NAV_ITEM)
            .width(Length::Fill)
            .style(theme::ContainerClass::Elevated.style()),
    )
    .on_press(SidebarMessage::SelectPinnedSearch(ps.id))
    .padding(0)
    .style(card_style.style())
    .width(Length::Fill)
    .into()
}
```

**Relative time formatting** (use `chrono` which is already a dependency):

```rust
fn format_relative_time(unix_ts: i64) -> String {
    let Some(dt) = chrono::DateTime::from_timestamp(unix_ts, 0) else {
        return String::new();
    };
    let now = chrono::Utc::now();
    let diff = now.signed_duration_since(dt);

    if diff.num_minutes() < 1 {
        "Just now".to_string()
    } else if diff.num_hours() < 1 {
        format!("{} min ago", diff.num_minutes())
    } else if diff.num_hours() < 24 {
        format!("{} hours ago", diff.num_hours())
    } else if diff.num_days() < 7 {
        format!("{} days ago", diff.num_days())
    } else {
        dt.format("%b %d").to_string()
    }
}
```

### 1E.5 Updated Sidebar view()

```rust
fn view(&self) -> Element<'_, SidebarMessage> {
    let mut col = column![
        scope_dropdown(self),
        Space::new().height(SPACE_XXS),
        pinned_searches(self),       // NEW: pinned searches above compose
        widgets::compose_button(SidebarMessage::Compose),
        Space::new().height(SPACE_XS),
        nav_items(self),
        widgets::section_break(),
        smart_folders(self),
    ]
    .spacing(0)
    .width(Length::Fill);

    // ... labels section unchanged
}
```

### 1E.6 Backend: Load Pinned Searches

**File: `crates/app/src/main.rs`** (or `crates/app/src/db.rs`)

```rust
pub async fn load_pinned_searches(
    db: Arc<Db>,
) -> Result<Vec<PinnedSearchEntry>, String> {
    db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, query, updated_at FROM pinned_searches
             ORDER BY updated_at DESC"
        ).map_err(|e| e.to_string())?;

        stmt.query_map([], |row| {
            Ok(PinnedSearchEntry {
                id: row.get("id")?,
                query: row.get("query")?,
                updated_at: row.get("updated_at")?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
    }).await
}
```

Pinned searches are loaded once on boot and refreshed after search operations. They are scope-independent (always shown regardless of account scope).

### Files Modified (Phase 1E)

| File | Change |
|------|--------|
| `crates/app/src/ui/sidebar.rs` | Add `pinned_searches`, `active_pinned_search` fields; add `PinnedSearchEntry` type; add `SelectPinnedSearch`/`DismissPinnedSearch` messages; add `PinnedSearchSelected`/`PinnedSearchDismissed` events; add rendering functions |
| `crates/app/src/main.rs` | Add `PinnedSearchesLoaded` message variant; load on boot; handle events |
| `crates/app/src/db.rs` | Add `load_pinned_searches` function |

---

## Phase 2: Strip Actions

**Goal:** Remove all action affordances from the sidebar. The sidebar becomes a pure read-only navigation surface.

**Depends on:** Command palette Slice 6 (app integration). Specifically:
- `NavigateToLabel` parameterized command with cross-account disambiguation must be working.
- Label CRUD commands must be in the palette.
- Context menu equivalent actions must be in the palette.

### What to Remove

1. **Any inline label editing UI** -- currently none exists in the sidebar, but if added before Phase 2 ships, it must be removed.
2. **Context menu handlers** -- any right-click or long-press behavior.
3. **`is_system_label` guard** -- the current hardcoded system label filter in `sidebar.rs` should be replaced by the backend's filtering in `build_account_labels` (which already filters system labels). The app-side `is_system_label` function becomes dead code.

### What Stays

- Scope dropdown (read + select, no create/edit)
- Compose button (this is not an "action on sidebar items" -- it creates a new entity)
- Folder/label click to navigate
- Pinned search dismiss button (this is curating the sidebar's own content, not acting on email)
- Settings button at the bottom

### Files Modified (Phase 2)

| File | Change |
|------|--------|
| `crates/app/src/ui/sidebar.rs` | Remove `is_system_label`, remove any context menu or edit-related code |

---

## Data Flow and Refresh Policy

### How SidebarModel Gets Populated

```
Boot
  |
  v
load_accounts(db) ---> AccountsLoaded
  |
  v
  Auto-select first account
  |
  v
  load_navigation(db, scope) ---> NavigationLoaded
  |                                     |
  v                                     v
  load_threads_for_current_view()   sidebar.nav_state = nav_state
  |
  v
  ThreadsLoaded ---> thread_list.set_threads()
```

### When to Refresh Navigation

| Trigger | Action |
|---------|--------|
| **Scope change** (SelectAccount / SelectAllAccounts) | Bump `nav_generation`, fire `load_navigation` with new scope. |
| **After sync completes** | Fire `load_navigation` to pick up new unread counts, new labels, new smart folders. |
| **After thread mutation** (mark read, star, move, delete) | Fire `load_navigation` to update affected unread counts. |
| **After smart folder CRUD** | Fire `load_navigation` to pick up added/removed/renamed smart folders. |
| **After pinned search mutation** | Fire `load_pinned_searches` only (pinned searches are independent of NavigationState). |

### Generational Load Tracking

The existing `nav_generation: u64` counter prevents stale navigation state from overwriting fresh state during rapid scope switching.

**Problem it solves:** User clicks Account A, then quickly clicks Account B. The `load_navigation` for Account A is still in flight. When it resolves, its `NavigationState` (scoped to A) must not overwrite the B-scoped state that may already be displayed.

**Mechanism:**
1. Every scope change bumps `nav_generation`.
2. Every `load_navigation` captures the current `nav_generation` at dispatch time.
3. `NavigationLoaded(g, _)` is silently dropped if `g != self.nav_generation`.

This pattern already exists for `AccountsLoaded`, `LabelsLoaded`, `ThreadsLoaded`, and `ThreadMessagesLoaded`. The `NavigationLoaded` variant follows the same pattern.

### Scope Derivation

The sidebar stores `selected_account: Option<usize>` (index into `accounts` vec). The core queries use `AccountScope`. Conversion:

```rust
impl App {
    fn current_scope(&self) -> AccountScope {
        match self.sidebar.selected_account {
            Some(idx) => match self.sidebar.accounts.get(idx) {
                Some(account) => AccountScope::Single(account.id.clone()),
                None => AccountScope::All,
            },
            None => AccountScope::All,
        }
    }
}
```

**Risk:** If the accounts list changes (account added/removed) while `selected_account` is an index, the index may point to the wrong account or out of bounds. Mitigation: after any account list refresh, validate `selected_account` against the new list length and reset to `None` if invalid.

---

## Dependency Graph

```
Phase 1A: Live Data Wiring
  (no dependencies -- connects existing backend to existing UI)

Phase 1B: Smart Folder Scoping Fix
  (no dependencies -- backend-only change to navigation.rs)

Phase 1C: Unread Counts
  depends on: 1A (NavigationState must be wired for counts to be visible)

Phase 1D: Hierarchy Support
  depends on: 1A (NavigationState must be wired)
  depends on: DB migration (parent_label_id column)
  depends on: Provider sync changes (Exchange, JMAP, IMAP)

Phase 1E: Pinned Searches Section
  depends on: 1A (sidebar view structure must be updated)
  depends on: Search app integration (for full lifecycle)
  scaffolding possible without search (render from DB, no search execution)

Phase 2: Strip Actions
  depends on: Command Palette Slice 6 (app integration)
  depends on: NavigateToLabel with cross-account disambiguation
```

**Suggested execution order:**

1. **1A + 1B** in parallel (both are straightforward wiring)
2. **1C** immediately after 1A lands (small delta, high value -- live unread counts)
3. **1D** is the largest piece; start DB migration + backend early, UI tree rendering can follow
4. **1E** can start scaffolding any time after 1A; full lifecycle waits on search
5. **2** is blocked on command palette work and ships separately

Phase 1A is the critical path. Everything else builds on it.
