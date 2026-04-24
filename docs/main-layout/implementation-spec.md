# Main Layout: Backend Implementation Spec (Phase 1)

Backend prerequisites for the initial main layout UI per `docs/main-layout/problem-statement.md`. All work is in `crates/core/` (the `rtsk` crate). No UI work.

**Scope note:** This document covers four specific early backend slices (label colors, thread detail, attachment collapse, focused region) that were prerequisites for the initial reading pane and conversation view. It is not the full backend spec for the main layout - the broader product surface (sidebar navigation, search pipeline, command dispatch, pinned searches, multi-window, calendar mode) implies a much larger backend/query surface documented in their respective specs. Treat this as a phase-specific implementation record, not the living authority for all main-layout backend needs.

## Implementation Status

| Slice | Status | Commits |
|-------|--------|---------|
| Slice 1: Label Color Fallback | ✅ Complete | `286bc92` |
| Slice 2: Thread Detail Data Layer | ✅ Complete | `d1b70d0` |
| Slice 3: Attachment Collapse Persistence | ✅ Complete | `286bc92` |
| Slice 4: FocusedRegion on CommandContext | ✅ Complete | `286bc92` |
| Phase 3: Auto-Advance | ⏳ Not started | Deferred until Phase 3 UI work begins |
| ~~Tauri command wrappers~~ | N/A | Tauri/React frontend permanently removed; iced calls core directly |

### Deviations from spec

All deviations are minor improvements:

- **Slice 1**: Uses `all_presets()` accessor instead of referencing `PRESETS` directly (encapsulation)
- **Slice 2**: Factored into named helper functions instead of one monolithic function (stays under 100-line limit). Adds `&#39;` HTML entity decoding. Uses char-aware truncation instead of byte-aware (multi-byte safety).
- **Slice 3**: Module registered as `mod thread_ui_state` + `pub use *` instead of `pub mod` (items accessible, module path private - no functional impact)

## Current State

The main layout needs four backend capabilities that do not exist:

1. **Label color fallback** - Gmail labels have `color_bg`/`color_fg` from the API. All other providers store `None`. Thread card label dots need colors for every label.
2. **Thread detail data layer** - The conversation view's message collapsing rules need per-message read state, message ownership detection, and quote-stripped summaries. No single function returns this data.
3. **Attachment collapse persistence** - A per-thread boolean for whether the attachment group is collapsed. No storage exists.
4. **`focused_region` on `CommandContext`** - Context-dependent keyboard shortcuts need focus tracking. `CommandContext` has no focus field.

## Slice 1: Label Color Fallback

Add a deterministic color assignment for labels that have no `color_bg`/`color_fg` (all non-Gmail providers).

### Approach

The `category_colors.rs` module already defines 25 Exchange preset colors as `PRESETS: &[(&str, &str, &str)]` (preset name, background hex, foreground hex) and exposes `all_presets()`, `preset_to_hex()`, and `nearest_exchange_preset()`. This palette is the canonical color set for the entire app.

Labels need a **deterministic hash-based assignment**: given a label name (and optionally account_id as a namespace), produce a stable index into the 25-preset palette. The same label name always gets the same color. No database writes - this is a pure function applied at read time.

### New function in `core/src/category_colors.rs`

```rust
/// Deterministic color assignment for a label that has no synced color.
///
/// Hashes the label name to produce a stable index into the 25-preset
/// palette. The `namespace` parameter (typically account_id) ensures
/// labels with the same name on different accounts can get different
/// colors if desired, but can be set to `""` for global consistency.
///
/// Returns `(bg_hex, fg_hex)`.
pub fn color_for_label(label_name: &str, namespace: &str) -> (&'static str, &'static str) {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    namespace.hash(&mut hasher);
    label_name.hash(&mut hasher);
    let index = (hasher.finish() as usize) % PRESETS.len();
    let (_, bg, fg) = PRESETS[index];
    (bg, fg)
}
```

### Resolution function for `DbLabel`

A helper that returns resolved colors for any label, preferring synced values and falling back to the hash:

```rust
/// Resolve display colors for a label.
///
/// If the label has synced `color_bg`/`color_fg` (Gmail), return those.
/// Otherwise, deterministically assign from the preset palette.
pub fn resolve_label_color(label: &DbLabel) -> (&str, &str) {
    match (&label.color_bg, &label.color_fg) {
        (Some(bg), Some(fg)) => (bg.as_str(), fg.as_str()),
        _ => color_for_label(&label.name, &label.account_id),
    }
}
```

This function lives in a new `core/src/label_colors.rs` module (imports from `category_colors` and `db::types::DbLabel`). Alternatively it can go in `category_colors.rs` if the `DbLabel` dependency is acceptable there.

### Why not write colors to the database?

Writing fallback colors to the `labels` table would conflate synced (authoritative) and local (derived) values. If a user later connects the same IMAP account to a provider that does support colors, the local values would fight with the synced ones. Keeping the fallback as a pure function avoids this - synced colors always win, and the hash is recomputed on every read.

### No migration needed

This is purely application-layer logic. No schema changes.

## Slice 2: Thread Detail Data Layer

New function: `get_thread_detail()` in `core/src/db/queries_extra/thread_detail.rs` (new file). Returns everything the conversation view needs for a single thread in one call.

### Return type

```rust
/// A single message within a thread detail response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadDetailMessage {
    pub id: String,
    pub thread_id: String,
    pub account_id: String,

    // Sender
    pub from_address: Option<String>,
    pub from_name: Option<String>,

    // Recipients (raw JSON strings from the messages table)
    pub to_addresses: Option<String>,
    pub cc_addresses: Option<String>,
    pub bcc_addresses: Option<String>,

    // Timestamps
    pub date: i64,

    // Content
    pub subject: Option<String>,
    pub body_html: Option<String>,
    pub body_text: Option<String>,

    // Flags
    pub is_read: bool,
    pub is_starred: bool,

    // Computed fields
    pub is_own_message: bool,
    pub collapsed_summary: Option<String>,
}

/// Labels with resolved colors, for the thread header label toggles
/// and thread card label dots.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadLabel {
    pub label_id: String,
    pub name: String,
    pub color_bg: String,
    pub color_fg: String,
}

/// Attachments grouped for the conversation view's attachment panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadAttachment {
    pub id: String,
    pub message_id: String,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size: Option<i64>,
    pub content_id: Option<String>,
    pub is_inline: bool,
    pub local_path: Option<String>,
    pub content_hash: Option<String>,
    pub gmail_attachment_id: Option<String>,
    // Context from parent message
    pub from_name: Option<String>,
    pub from_address: Option<String>,
    pub date: i64,
}

/// Complete thread detail response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadDetail {
    pub thread_id: String,
    pub account_id: String,
    pub subject: Option<String>,
    pub is_starred: bool,
    pub is_snoozed: bool,
    pub is_pinned: bool,
    pub is_muted: bool,

    /// Messages ordered newest-first (conversation view order).
    pub messages: Vec<ThreadDetailMessage>,

    /// Labels with resolved colors.
    pub labels: Vec<ThreadLabel>,

    /// Non-inline attachments across all messages, ordered by date desc.
    pub attachments: Vec<ThreadAttachment>,

    /// Whether the attachment group is collapsed (persisted per-thread).
    pub attachments_collapsed: bool,
}
```

### Function signature

```rust
/// Fetch everything needed to render the conversation view for a thread.
///
/// Joins thread metadata, messages (with bodies from BodyStore), labels
/// (with resolved colors), attachments, and attachment collapse state.
/// Each message is annotated with `is_own_message` (ownership detection)
/// and `collapsed_summary` (quote/signature-stripped preview).
pub fn get_thread_detail(
    conn: &Connection,
    body_store_conn: &Connection,
    account_id: &str,
    thread_id: &str,
) -> Result<ThreadDetail, String>
```

Note: this is a **synchronous** function taking `&Connection` references (not `&DbState`). The Tauri command wrapper (or iced message handler) obtains the connections and calls this on a blocking thread. This follows the pattern in `navigation.rs` (`get_navigation_state` takes `&Connection`).

The body store is a separate SQLite database (`bodies.db`). The function needs both connections: the main DB connection for thread/message/label/attachment queries, and the body store connection for decompressed body text.

### Implementation steps

#### Step 1: Query messages

```sql
SELECT id, thread_id, account_id, from_address, from_name,
       to_addresses, cc_addresses, bcc_addresses,
       subject, date, is_read, is_starred, snippet
FROM messages
WHERE account_id = ?1 AND thread_id = ?2
ORDER BY date DESC
```

This returns a subset of message fields - **not** `DbMessage`, which has ~20 additional columns (reply_to, body_cached, raw_size, internal_date, unsubscribe headers, auth headers, IMAP metadata) and a custom row mapper in `queries.rs` that expects `SELECT *`. The `ThreadDetailMessage` struct defined above is the purpose-built row type for this query. Map rows directly into `ThreadDetailMessage` fields (the body fields are populated separately from the body store in Step 2).

Ordered newest-first for the conversation view.

#### Step 2: Fetch bodies from body store

Query the body store for all message IDs in the thread:

```sql
SELECT message_id, body_html, body_text
FROM bodies
WHERE message_id IN (?1, ?2, ...)
```

Uses the same chunked approach as `BodyStoreState::get_batch`, but synchronous (direct `&Connection` instead of `async with_conn`). Decompress.

#### Step 3: Detect message ownership

Query the account's identity addresses:

```sql
-- From send_identities table (migration v42)
SELECT email FROM send_identities WHERE account_id = ?1

UNION

-- From send_as_aliases table (migration v10)
SELECT email FROM send_as_aliases WHERE account_id = ?1

UNION

-- The account's own email
SELECT email FROM accounts WHERE id = ?1
```

Collect into a `HashSet<String>` (lowercased). For each message, `is_own_message = identity_emails.contains(&message.from_address.to_lowercase())`.

The three sources cover:
- `accounts.email` - the primary account email, always present
- `send_identities` - delegated/shared mailbox identities (migration v42)
- `send_as_aliases` - Gmail send-as aliases, IMAP aliases (migration v10)

#### Step 4: Generate collapsed summaries

For each message, produce a ~60-character plain-text summary suitable for collapsed message display. The summary shows the first meaningful content, stripped of quoted text and signatures.

```rust
/// Strip quoted lines and signature blocks, return the first ~60 chars
/// of meaningful body text.
///
/// Stripping rules:
/// 1. Prefer `body_text` over `body_html` (plain text is cleaner).
///    If only HTML is available, strip tags first.
/// 2. Remove lines starting with `>` (quoted text).
/// 3. Remove everything after the signature delimiter (`-- \n` - note
///    the trailing space per RFC 3676).
/// 4. Collapse whitespace, trim, truncate to ~60 chars with ellipsis.
fn make_collapsed_summary(body_text: Option<&str>, body_html: Option<&str>) -> Option<String>
```

For HTML-only messages, a minimal tag stripper is sufficient - remove all `<...>` tags, decode `&amp;` / `&lt;` / `&gt;` / `&nbsp;` / `&quot;`, collapse whitespace. No need for a full HTML parser; these summaries are lossy previews.

The function processes lines in order:
1. Split by `\n`
2. Stop at `-- \n` (signature delimiter - note the trailing space)
3. Skip lines starting with `>` (after optional whitespace)
4. Skip blank lines
5. Join remaining lines with spaces, collapse runs of whitespace
6. Truncate to 60 chars, append `...` if truncated

#### Step 5: Query labels with resolved colors

```sql
SELECT l.id, l.name, l.color_bg, l.color_fg, l.account_id
FROM thread_labels tl
JOIN labels l ON l.account_id = tl.account_id AND l.id = tl.label_id
WHERE tl.account_id = ?1 AND tl.thread_id = ?2
ORDER BY l.sort_order ASC, l.name ASC
```

For each label, resolve colors using `resolve_label_color()` from Slice 1. System labels (from `SYSTEM_FOLDER_ROLES`) are filtered out - they are structural, not display labels.

#### Step 6: Query attachments

```sql
SELECT a.id, a.message_id, a.filename, a.mime_type, a.size,
       a.content_id, a.is_inline, a.local_path, a.content_hash,
       a.gmail_attachment_id,
       m.from_name, m.from_address, m.date
FROM attachments a
JOIN messages m ON a.message_id = m.id AND a.account_id = m.account_id
WHERE a.account_id = ?1 AND m.thread_id = ?2
  AND a.is_inline = 0
  AND a.filename IS NOT NULL AND a.filename != ''
ORDER BY m.date DESC
```

Non-inline only (`is_inline = 0`). Includes message context (sender, date) for the attachment card's "Mar 14 from Alice" line.

#### Step 7: Query attachment collapse state

```sql
SELECT attachments_collapsed FROM thread_ui_state
WHERE account_id = ?1 AND thread_id = ?2
```

Returns `false` (expanded) if no row exists (default is expanded per the problem statement). Uses the same `(account_id, thread_id)` compound key as Slice 3's schema.

#### Step 8: Assemble `ThreadDetail`

Combine all the above into the return struct. The thread-level fields (`is_starred`, `is_snoozed`, etc.) come from the `threads` table:

```sql
SELECT subject, is_starred, is_snoozed, is_pinned, is_muted
FROM threads
WHERE account_id = ?1 AND id = ?2
```

### Connection access

Neither `DbState` nor `BodyStoreState` originally exposed a `conn()` accessor - both only had `async with_conn(closure)` which runs the closure on `spawn_blocking` internally. The thread detail function needs both connections simultaneously, so nesting `with_conn` calls would be awkward.

**Implemented:** `pub fn conn(&self) -> Arc<Mutex<Connection>>` accessors were added to both `DbState` and `BodyStoreState`. Both structs already hold `conn: Arc<Mutex<Connection>>` - the accessor just clones the Arc. This is used extensively by the iced frontend for synchronous multi-database access.

*Note: The Tauri command wrappers originally specified here are no longer relevant - the Tauri/React frontend has been permanently removed in favor of iced. The iced app calls core functions directly via the `conn()` accessors.*

### Why synchronous?

The function touches two SQLite databases but does no I/O beyond disk reads (which SQLite handles internally). Running it on `spawn_blocking` with both connections locked is simpler and faster than interleaving async calls. The iced frontend will call the core function directly (no Tauri command needed) - it just needs the two `&Connection` references.

## Slice 3: Attachment Collapse Persistence

A small SQLite table storing per-thread UI state. Currently only one field (attachment group collapsed), but the table is named generically to accommodate future per-thread UI state.

### Migration (v59)

```sql
CREATE TABLE IF NOT EXISTS thread_ui_state (
    thread_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    attachments_collapsed INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, thread_id)
);
```

Compound primary key on `(account_id, thread_id)` - thread IDs are unique per account but not globally (IMAP providers may reuse IDs across accounts).

No foreign key to `threads` - the UI state should survive thread table rebuilds during re-sync. Orphaned rows are harmless (tiny, boolean-only) and can be cleaned up by a periodic vacuum if needed.

### Core functions in `core/src/db/queries_extra/thread_ui_state.rs` (new file)

```rust
/// Get whether the attachment group is collapsed for a thread.
/// Returns `false` (expanded) if no row exists.
pub fn get_attachments_collapsed(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
) -> Result<bool, String>

/// Set the attachment group collapse state for a thread.
/// Uses INSERT OR REPLACE - creates the row on first toggle.
pub fn set_attachments_collapsed(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
    collapsed: bool,
) -> Result<(), String>
```

SQL for `set_attachments_collapsed`:

```sql
INSERT INTO thread_ui_state (account_id, thread_id, attachments_collapsed)
VALUES (?1, ?2, ?3)
ON CONFLICT(account_id, thread_id) DO UPDATE SET attachments_collapsed = ?3
```

*Note: Tauri command wrappers originally specified here are no longer relevant - the iced app calls these core functions directly.*

### Integration with `get_thread_detail`

Slice 2's `get_thread_detail` queries this table in Step 7 and includes `attachments_collapsed: bool` in the `ThreadDetail` response. The setter is called independently when the user toggles the collapse state (not part of the detail query).

## Slice 4: `focused_region` on `CommandContext`

Add a focus region enum to `CommandContext` in `core/src/command_palette/context.rs`.

### New enum

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FocusedRegion {
    ThreadList,
    ReadingPane,
    Composer,
    SearchBar,
    Sidebar,
}
```

### `CommandContext` change

Add one field:

```rust
pub struct CommandContext {
    // ... existing fields ...

    pub focused_region: Option<FocusedRegion>,
}
```

`Option<FocusedRegion>` rather than a bare `FocusedRegion` - `None` means no region has focus (e.g., app just launched, or focus is on a dialog/overlay that isn't one of the five regions). This avoids inventing a "no focus" variant that commands would need to handle.

### `empty_context()` update

The test helper `empty_context()` sets `focused_region: None`.

### Helper methods

```rust
impl CommandContext {
    // ... existing methods ...

    pub fn is_focused(&self, region: FocusedRegion) -> bool {
        self.focused_region == Some(region)
    }
}
```

This is used by the command palette's keybinding dispatch to route context-dependent shortcuts (e.g., `Enter` does different things in ThreadList vs ReadingPane).

### App-layer responsibility

Core defines the `FocusedRegion` enum and the `CommandContext` field. The app layer is responsible for populating it:

- **Tauri/React**: The TS side tracks which panel has focus and includes `focusedRegion` when constructing the `CommandContext` for palette queries. This can be driven by DOM `focus`/`blur` events on the panel containers.
- **iced**: The iced app tracks focus in its model (e.g., an `active_pane: Option<FocusedRegion>` field on `App`) and sets it on `CommandContext` before querying the registry. Focus changes come from click events on panes and keyboard navigation (Tab, Escape).

Without the app layer populating this field, context-dependent shortcuts (main-layout Phase 3) will not work - the field will remain `None` and all shortcuts will behave as if no region is focused.

### No migration needed

`CommandContext` is an in-memory struct passed from the UI to the command system. It is not persisted.

## Prerequisites / Schema Changes

### New migration: v59

One new table (`thread_ui_state`) as described in Slice 3. Added to `core/src/db/migrations.rs`:

```rust
Migration {
    version: 59,
    description: "Per-thread UI state (attachment collapse)",
    sql: r#"
        CREATE TABLE IF NOT EXISTS thread_ui_state (
            thread_id TEXT NOT NULL,
            account_id TEXT NOT NULL,
            attachments_collapsed INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (account_id, thread_id)
        );
    "#,
},
```

### BodyStoreState accessor

Add to `core/src/body_store/mod.rs`:

```rust
impl BodyStoreState {
    /// Access the underlying connection Arc for synchronous use.
    pub fn conn(&self) -> Arc<Mutex<Connection>> {
        Arc::clone(&self.conn)
    }
}
```

### New files

| File | Contents |
|------|----------|
| `core/src/label_colors.rs` | `color_for_label()`, `resolve_label_color()` - Slice 1 |
| `core/src/db/queries_extra/thread_detail.rs` | `ThreadDetail`, `ThreadDetailMessage`, `ThreadLabel`, `ThreadAttachment`, `get_thread_detail()` - Slice 2 |
| `core/src/db/queries_extra/thread_ui_state.rs` | `get_attachments_collapsed()`, `set_attachments_collapsed()` - Slice 3 |

### Module registration

Add to `core/src/lib.rs`:

```rust
pub mod label_colors;
```

Add to `core/src/db/queries_extra.rs` (the flat module file, not a `mod.rs` - this is the existing pattern):

```rust
pub mod thread_detail;
pub mod thread_ui_state;
pub use thread_detail::*;
pub use thread_ui_state::*;
```

## Dependency Graph

```
Slice 1 (label color fallback)
  └── Slice 2 (thread detail data layer) - uses resolve_label_color()

Slice 3 (attachment collapse persistence)
  └── Slice 2 (thread detail data layer) - reads attachments_collapsed

Slice 4 (focused_region on CommandContext) - independent
```

Slice 1 and Slice 3 can be done in parallel. Slice 2 depends on both. Slice 4 is independent and can be done at any time.

Build order: **Slice 1 + Slice 3 + Slice 4 in parallel**, then **Slice 2**.

## Ecosystem Patterns

This is a backend-only spec, so overlap with the [iced ecosystem survey](../iced-ecosystem-cross-reference.md) is limited to how backend data will be consumed by the iced frontend. No changes to the backend spec are warranted based on the survey, but the following patterns inform how the iced app layer will integrate with these backend slices.

| Spec Slice | Survey Pattern | Action |
|---|---|---|
| Slice 2 (`get_thread_detail`) | bloom generational load tracking | Implement generation counter in iced app's thread selection handler to discard stale detail responses when the user navigates rapidly |
| Slice 4 (`FocusedRegion`) | trebuchet Component trait + raffi query routing | Structure panel system around Component trait; filter commands by `focused_region` for context-dependent shortcuts |
| Slice 1 (label colors) | shadcn-rs/iced-plus theming | Register 25 presets as named tokens; build `hex_to_iced_color()` utility for the theme's Token-to-Catalog bridge |
| Slice 3+2 (attachments) | shadcn-rs resizable panels | Auto-save resizable panels for the attachment panel's collapse/expand state |
| Auto-advance | pikeru subscriptions | Multiplex provider mutation and local DB update channels so the UI can react to completed actions |

The backend slices themselves (SQL queries, color hashing, `CommandContext` fields) are framework-agnostic and require no ecosystem-specific patterns. The survey patterns listed above apply at the iced app layer when wiring these backend functions into the UI.

## Phase 3 Backend: What's Already Done vs What's Missing

The main-layout Phase 3 (Interaction Flow) requires email mutations and compose/send - these **already exist** in the backend:

- **Archive, trash, star, mark read, move to folder** - implemented for all four providers via `ProviderOps` trait (`core/src/provider/ops.rs`), with Tauri command wrappers in `src/provider/commands.rs`.
- **Send email** - `send_email` on `ProviderOps`, implemented per provider.
- **Label toggle (add/remove)** - `add_label` / `remove_label` on `ProviderOps`.

The one missing backend piece for Phase 3 is **auto-advance**:

### Auto-Advance (Phase 3 prerequisite)

After a destructive/filing action (archive, trash, move), the UI needs to know which thread to select next. This requires a function that returns the adjacent thread given the current position:

```rust
/// Return the thread ID adjacent to `current_thread_id` in the current
/// view's sort order. `direction` controls whether to advance to the
/// next (older) or previous (newer) thread.
///
/// Returns `None` if the current thread is at the boundary (first or last).
pub fn get_adjacent_thread(
    conn: &Connection,
    account_id: &str,       // or AccountScope for unified view
    current_thread_id: &str,
    folder_or_label_id: Option<&str>,  // current sidebar selection
    direction: AdvanceDirection,
) -> Result<Option<String>, String>
```

```rust
pub enum AdvanceDirection {
    Next,     // older (default)
    Previous, // newer
}
```

The query needs to find the current thread's position in the sorted list and return the next/previous ID. This depends on the current view (folder, label, starred, smart folder) and sort order (`last_message_at DESC`). The implementation is a `WHERE last_message_at < ?` (or `>` for Previous) query with `LIMIT 1`, using the current thread's `last_message_at` as the pivot.

Everything else in Phase 3 (wiring mutations to command palette dispatch, keyboard shortcuts, multi-select) is app-layer work, not core backend.
