# Main Layout: iced Implementation Spec

UI-only spec for the crates/app main layout work defined by `docs/main-layout/problem-statement.md`. All work is in `crates/app/`. No backend changes.

## Implementation Status

| Phase | Status | Commits |
|-------|--------|---------|
| Phase 1: Thread List Polish + Layout | ✅ Complete | `286bc92` |
| Phase 2: Conversation View (snippet-only) | ✅ Complete | `d1b70d0` |
| Phase 3: Interaction Flow | ⏳ Deferred | Blocked on command palette iced integration |
| Phase 4: Polish | ✅ Complete | `75c0dd4`, `2c81b1a` |

### Deviations from spec

- **1.1**: `RIGHT_SIDEBAR_AUTO_COLLAPSE_WIDTH` placed in `layout.rs` instead of inline in `main.rs` (follows project's no-magic-numbers convention)
- **1.7**: `sanitize()` clamps sidebar_width to 180.0 (matching default) instead of 200.0 (drag minimum). Prevents width jump on first save/reopen cycle.
- **2.3**: `date_display` stored on `SettingsState` instead of directly on `App` (lives with other settings)
- **2.9**: Attachment deduplication by filename added (not in original spec). `attachment_card` gains a `version_count` parameter with "N versions" badge. Attachment group header count shows unique files, not raw rows.
- **4.2**: Collapse cache key uses `account_id:thread_id` compound key instead of just `thread_id` (matches backend's compound PK, prevents cross-account collision)

### Post-implementation bug fixes (`2c81b1a`)

1. **Stale conversation rendering**: `SelectThread` now clears `thread_messages`/`thread_attachments`/`message_expanded` immediately before firing async loads, preventing stale messages from rendering under a new thread's header.
2. **Cache key scoping**: Changed from `thread_id` only to `account_id:thread_id`.
3. **Sidebar width clamp**: Reduced from 200 to 180 to match default.
4. **Attachment dedup**: Added per the design spec's deduplication/versioning requirement.

---

## Phase 1: Thread List Polish + Layout

### 1.1 Layout Constants (`src/ui/layout.rs`)

**Rename and update panel widths:**

```rust
// Change:
pub const THREAD_LIST_WIDTH: f32 = 280.0;
pub const CONTACT_SIDEBAR_WIDTH: f32 = 240.0;

// To:
pub const THREAD_LIST_WIDTH: f32 = 400.0;
pub const RIGHT_SIDEBAR_WIDTH: f32 = 240.0;
```

**New constants:**

```rust
/// Label dot diameter on thread cards (line 3 indicators)
pub const LABEL_DOT_SIZE: f32 = 6.0;

/// Thread card fixed height (three lines + padding)
pub const THREAD_CARD_HEIGHT: f32 = 68.0;

/// Right sidebar section header padding
pub const PAD_RIGHT_SIDEBAR: Padding = Padding {
    top: 12.0,
    right: 12.0,
    bottom: 12.0,
    left: 12.0,
};

/// Starred thread card warm background alpha
pub const STARRED_BG_ALPHA: f32 = 0.12;
```

The existing `DOT_SIZE` (8.0) is used in the sidebar label nav items. `LABEL_DOT_SIZE` (6.0) is specific to thread card indicator dots, which are smaller for density.

### 1.2 Thread Card Redesign (`src/ui/widgets.rs`)

Replace the current `thread_card` function. The current implementation has an avatar column, uses `AVATAR_THREAD_CARD` (28px), and arranges sender/subject/snippet with avatar leading. The new design removes the avatar and uses a full-width three-line layout.

**Current widget tree (to be replaced):**

```
button
  container(PAD_THREAD_CARD)
    row[avatar, column[top_row, subject_row, snippet_row]]
```

**New widget tree:**

```
button(thread_card_button(selected, starred))
  container(PAD_THREAD_CARD).height(THREAD_CARD_HEIGHT)
    column(spacing=SPACE_XXXS).width(Fill)
      // Line 1: sender + date
      row(align_y=Center)
        sender_slot: container(text(sender).size(TEXT_MD).font(sender_font)).width(Fill)
        date_slot: container(text(date).size(TEXT_XS).style(text_tertiary))

      // Line 2: subject
      row
        subject_slot: container(text(subject).size(TEXT_MD).style(subject_style)
            .font(subject_font).wrapping(None)).width(Fill)

      // Line 3: snippet + indicators
      row(align_y=Center)
        snippet_slot: container(text(snippet).size(TEXT_SM).style(text_secondary)
            .wrapping(None)).width(Fill)
        indicators_slot: row(spacing=SPACE_XXS, align_y=Center)
          [label dots...]
          [attachment icon]
```

**New function signature:**

```rust
pub fn thread_card(
    thread: &Thread,
    index: usize,
    selected: bool,
    label_colors: &[(Color,)],  // resolved label dot colors for this thread
) -> Element<'_, Message>
```

Until the backend provides per-thread label colors, `label_colors` will be an empty slice. The parameter is included now so the widget API is stable.

**Sender font logic:**

```rust
let sender_font = if thread.is_read {
    font::TEXT           // normal weight for read
} else {
    font::TEXT_SEMIBOLD  // semibold for unread
};
```

The current code uses `font::TEXT` with `Weight::Bold` inline. Use the pre-defined `font::TEXT_SEMIBOLD` constant instead.

**Subject style logic:**

```rust
let subject_style: fn(&Theme) -> text::Style = if thread.is_read {
    theme::text_muted    // muted for read
} else {
    theme::text_accent   // primary/accent for unread
};
let subject_font = font::TEXT;  // always normal weight (color is the unread signal)
```

**Snippet style:** Always `text::secondary` (existing).

**Label dots (line 3):**

```rust
// Inside indicators_slot, before attachment icon:
for &(color,) in label_colors {
    indicators = indicators.push(label_dot(color));
}
```

New helper in `widgets.rs`:

```rust
pub fn label_dot<'a>(color: Color) -> Element<'a, Message> {
    let dot = Canvas::new(DotPainter { color })
        .width(LABEL_DOT_SIZE)
        .height(LABEL_DOT_SIZE);
    container(dot)
        .center_y(Length::Shrink)
        .into()
}
```

This reuses the existing `DotPainter` struct but with `LABEL_DOT_SIZE` instead of `DOT_SIZE`.

**Attachment icon (line 3):**

```rust
if thread.has_attachments {
    indicators = indicators.push(
        icon::paperclip().size(ICON_XS).style(theme::text_tertiary)
    );
}
```

Same as current, just moved to line 3 indicators row.

**Remove from thread cards:**
- Avatar circle (`avatar_circle(sender, AVATAR_THREAD_CARD)`)
- Star icon indicator (star styling is now via background color)
- Message count badge

### 1.3 Starred Thread Card Style (`src/ui/theme.rs`)

Update `thread_card_button` to accept a `starred` parameter:

```rust
pub fn thread_card_button(
    selected: bool,
    starred: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let p = theme.palette();
        let base_bg = if starred {
            // Warm golden tint from warning color
            mix(p.background.base.color, p.warning.base.color, STARRED_BG_ALPHA)
        } else if selected {
            p.background.weakest.color
        } else {
            p.background.base.color
        };

        match status {
            button::Status::Hovered => button::Style {
                background: Some(if starred {
                    // One step lighter than starred base
                    mix(p.background.weakest.color, p.warning.base.color, STARRED_BG_ALPHA).into()
                } else {
                    p.background.weakest.color.into()
                }),
                text_color: p.background.base.text,
                ..Default::default()
            },
            _ => button::Style {
                background: Some(base_bg.into()),
                text_color: p.background.base.text,
                ..Default::default()
            },
        }
    }
}
```

The `mix` helper already exists in `theme.rs`. Import `STARRED_BG_ALPHA` from `layout.rs`.

The hover step discipline is maintained: starred cards rest on a warning-tinted `base`, hover one step toward `weakest`. Non-starred cards follow the existing pattern.

### 1.4 Thread List Update (`src/ui/thread_list.rs`)

Update the `view` function call to pass `label_colors`:

```rust
pub fn view<'a>(
    threads: &'a [Thread],
    selected_thread: Option<usize>,
) -> Element<'a, Message> {
    // ...
    for (i, thread) in threads.iter().enumerate() {
        // Empty label colors for now — backend integration later
        let label_colors: &[(Color,)] = &[];
        list = list.push(widgets::thread_card(thread, i, selected_thread == Some(i), label_colors));
    }
    // ...
}
```

**Context line below search placeholder:**

Add a context line below the search input placeholder. This shows the current folder name (left) and account scope (right). Fixed height, always visible.

```
container(PAD_PANEL_HEADER)
  column(spacing=SPACE_XXS)
    // Search placeholder (existing)
    container(text("Search...").size(TEXT_MD).style(text_tertiary))
      .padding(PAD_INPUT).style(elevated_container)
    // Context line (new)
    row(align_y=Center)
      context_label_slot: text(folder_name).size(TEXT_SM).style(text_tertiary)
      Space::Fill
      context_scope_slot: text(scope_name).size(TEXT_SM).style(text_tertiary)
```

The `view` function needs two new parameters for the context line:

```rust
pub fn view<'a>(
    threads: &'a [Thread],
    selected_thread: Option<usize>,
    folder_name: &'a str,
    scope_name: &'a str,
) -> Element<'a, Message>
```

`folder_name` = currently selected label name or "Inbox". `scope_name` = selected account display name or "All". These are derived from existing `App` state in `main.rs`.

### 1.5 Right Sidebar (`src/ui/right_sidebar.rs`)

New file replacing `src/ui/contact_sidebar.rs`. The module registration in `src/ui/mod.rs` changes from `pub mod contact_sidebar;` to `pub mod right_sidebar;`.

**Function signature:**

```rust
pub fn view<'a>(open: bool) -> Element<'a, Message> {
    if !open {
        return Space::new().width(0).height(0).into();
    }

    let content = column![
        calendar_section(),
        widgets::divider(),
        pinned_section(),
    ]
    .spacing(0)
    .width(Length::Fill);

    container(scrollable(content).height(Length::Fill))
        .width(RIGHT_SIDEBAR_WIDTH)
        .height(Length::Fill)
        .style(theme::sidebar_container)
        .into()
}
```

**Calendar section (scaffold):**

```rust
fn calendar_section<'a>() -> Element<'a, Message> {
    container(
        column![
            widgets::section_header("CALENDAR"),
            container(
                text("Calendar placeholder")
                    .size(TEXT_SM)
                    .style(theme::text_tertiary)
            ).padding(PAD_ICON_BTN),
        ]
        .spacing(SPACE_XXS),
    )
    .padding(PAD_RIGHT_SIDEBAR)
    .into()
}
```

**Pinned items section (scaffold):**

```rust
fn pinned_section<'a>() -> Element<'a, Message> {
    container(
        column![
            widgets::section_header("PINNED ITEMS"),
            container(
                text("No pinned items")
                    .size(TEXT_SM)
                    .style(theme::text_tertiary)
            ).padding(PAD_ICON_BTN),
        ]
        .spacing(SPACE_XXS),
    )
    .padding(PAD_RIGHT_SIDEBAR)
    .into()
}
```

### 1.6 Main Layout Changes (`src/main.rs`)

**New Message variant:**

```rust
pub enum Message {
    // ... existing variants ...
    ToggleRightSidebar,
}
```

**New App fields:**

```rust
struct App {
    // ... existing fields ...
    right_sidebar_open: bool,
}
```

Initialize `right_sidebar_open: false` in `boot()`.

**View function update:**

Replace the contact sidebar reference with the right sidebar. The current view builds:

```
row![sidebar, divider_sidebar, thread_list, divider_thread, reading_pane]
```

After the reading pane, conditionally add the right sidebar:

```rust
let right_sidebar = ui::right_sidebar::view(self.right_sidebar_open);

let layout = row![
    sidebar, divider_sidebar,
    thread_list, divider_thread,
    reading_pane,
    right_sidebar,
]
.height(Length::Fill);
```

When `right_sidebar_open` is false, `right_sidebar::view` returns a zero-width Space, so no layout impact. When open, the reading pane (which is `Length::Fill`) shrinks by `RIGHT_SIDEBAR_WIDTH`.

No divider between reading pane and right sidebar — the right sidebar has a fixed width and is not resizable.

**Update handler for `ToggleRightSidebar`:**

```rust
Message::ToggleRightSidebar => {
    self.right_sidebar_open = !self.right_sidebar_open;
    Task::none()
}
```

**Auto-collapse on window resize.** The problem statement requires the right sidebar to auto-collapse when window width drops below ~1200px. Add to the existing `WindowResized` handler in `main.rs`:

```rust
Message::WindowResized(size) => {
    // ... existing resize handling ...

    // Auto-collapse right sidebar below 1200px
    const RIGHT_SIDEBAR_AUTO_COLLAPSE_WIDTH: f32 = 1200.0;
    if size.width < RIGHT_SIDEBAR_AUTO_COLLAPSE_WIDTH && self.right_sidebar_open {
        self.right_sidebar_open = false;
    }

    Task::none()
}
```

Add `RIGHT_SIDEBAR_AUTO_COLLAPSE_WIDTH` to `layout.rs`:

```rust
pub const RIGHT_SIDEBAR_AUTO_COLLAPSE_WIDTH: f32 = 1200.0;
```

Note: this only auto-collapses. It does not auto-expand when the window grows back above 1200px — the user must manually reopen. This avoids the sidebar popping in unexpectedly during resize.

**Remove contact_sidebar usage.** The current `main.rs` view does not render the contact sidebar inline (it was removed from the row layout already), but `contact_sidebar.rs` still exists. After creating `right_sidebar.rs`, remove or leave `contact_sidebar.rs` dead — it is no longer referenced.

**Update imports in `main.rs`:**

```rust
use ui::layout::{
    SIDEBAR_MIN_WIDTH, SIDEBAR_WIDTH,
    THREAD_LIST_MIN_WIDTH, THREAD_LIST_WIDTH,
    RIGHT_SIDEBAR_WIDTH,
};
```

### 1.7 WindowState Extension (`src/window_state.rs`)

**Add fields to `WindowState`:**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowState {
    pub width: f32,
    pub height: f32,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub maximized: bool,
    // New fields:
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: f32,
    #[serde(default = "default_thread_list_width")]
    pub thread_list_width: f32,
    #[serde(default)]
    pub right_sidebar_open: bool,
}

fn default_sidebar_width() -> f32 { 180.0 }  // SIDEBAR_WIDTH
fn default_thread_list_width() -> f32 { 400.0 }  // THREAD_LIST_WIDTH
```

The `#[serde(default = ...)]` annotations ensure backward compatibility — existing `window.json` files without these fields will deserialize cleanly with the default values.

**Update `Default` impl:**

```rust
impl Default for WindowState {
    fn default() -> Self {
        Self {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            x: None,
            y: None,
            maximized: false,
            sidebar_width: 180.0,
            thread_list_width: 400.0,
            right_sidebar_open: false,
        }
    }
}
```

**Update `sanitize`:**

```rust
fn sanitize(&mut self) {
    self.width = self.width.max(MIN_WIDTH);
    self.height = self.height.max(MIN_HEIGHT);
    self.sidebar_width = self.sidebar_width.max(200.0);  // SIDEBAR_MIN_WIDTH
    self.thread_list_width = self.thread_list_width.max(250.0);  // THREAD_LIST_MIN_WIDTH
    // ... existing position sanitization ...
}
```

**Load panel widths in `boot()`:**

```rust
fn boot() -> (Self, Task<Message>) {
    // ... existing setup ...
    let app = Self {
        // ...
        sidebar_width: window.sidebar_width,
        thread_list_width: window.thread_list_width,
        right_sidebar_open: window.right_sidebar_open,
        // ...
    };
    // ...
}
```

**Save panel widths on close:**

In the `WindowCloseRequested` handler, update `self.window` with current panel state before saving:

```rust
Message::WindowCloseRequested(id) => {
    let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
    self.window.sidebar_width = self.sidebar_width;
    self.window.thread_list_width = self.thread_list_width;
    self.window.right_sidebar_open = self.right_sidebar_open;
    self.window.save(data_dir);
    iced::window::close(id)
}
```

### 1.8 Module Registration (`src/ui/mod.rs`)

```rust
pub mod animated_toggler;
pub mod layout;
pub mod popover;
pub mod reading_pane;
pub mod right_sidebar;    // was: contact_sidebar
pub mod settings;
pub mod sidebar;
pub mod theme;
pub mod thread_list;
pub mod widgets;
```

---

## Phase 2: Conversation View (Snippet-Only)

### 2.1 DB Types Extension (`src/db.rs`)

**New type for messages in a thread detail view:**

```rust
#[derive(Debug, Clone)]
pub struct ThreadMessage {
    pub id: String,
    pub thread_id: String,
    pub account_id: String,
    pub from_name: Option<String>,
    pub from_address: Option<String>,
    pub to_addresses: Option<String>,
    pub date: Option<i64>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub is_read: bool,
    pub is_starred: bool,
}
```

**New type for thread attachments:**

```rust
#[derive(Debug, Clone)]
pub struct ThreadAttachment {
    pub id: String,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size: Option<i64>,
    pub from_name: Option<String>,
    pub date: Option<i64>,
}
```

**New DB query:**

```rust
impl Db {
    pub async fn get_thread_messages(
        &self,
        account_id: String,
        thread_id: String,
    ) -> Result<Vec<ThreadMessage>, String> {
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, thread_id, account_id, from_name, from_address,
                        to_addresses, date, subject, snippet, is_read, is_starred
                 FROM messages
                 WHERE account_id = ?1 AND thread_id = ?2
                 ORDER BY date DESC"
            ).map_err(|e| e.to_string())?;

            stmt.query_map(params![account_id, thread_id], |row| {
                Ok(ThreadMessage {
                    id: row.get("id")?,
                    thread_id: row.get("thread_id")?,
                    account_id: row.get("account_id")?,
                    from_name: row.get("from_name")?,
                    from_address: row.get("from_address")?,
                    to_addresses: row.get("to_addresses")?,
                    date: row.get("date")?,
                    subject: row.get("subject")?,
                    snippet: row.get("snippet")?,
                    is_read: row.get::<_, i64>("is_read")? != 0,
                    is_starred: row.get::<_, i64>("is_starred")? != 0,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }

    pub async fn get_thread_attachments(
        &self,
        account_id: String,
        thread_id: String,
    ) -> Result<Vec<ThreadAttachment>, String> {
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT a.id, a.filename, a.mime_type, a.size,
                        m.from_name, m.date
                 FROM attachments a
                 JOIN messages m ON a.message_id = m.id AND a.account_id = m.account_id
                 WHERE a.account_id = ?1 AND m.thread_id = ?2
                   AND a.is_inline = 0
                   AND a.filename IS NOT NULL AND a.filename != ''
                 ORDER BY m.date DESC"
            ).map_err(|e| e.to_string())?;

            stmt.query_map(params![account_id, thread_id], |row| {
                Ok(ThreadAttachment {
                    id: row.get("id")?,
                    filename: row.get("filename")?,
                    mime_type: row.get("mime_type")?,
                    size: row.get("size")?,
                    from_name: row.get("from_name")?,
                    date: row.get("date")?,
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

### 2.2 Message Variants (`src/main.rs`)

**New variants:**

```rust
pub enum Message {
    // ... existing ...
    /// thread_id is included so stale responses can be discarded.
    /// If the user clicks thread A then thread B quickly, the late
    /// response for A arrives after B is selected — the handler
    /// checks thread_id against the currently selected thread and
    /// drops the response if they don't match.
    ThreadMessagesLoaded(String, Result<Vec<ThreadMessage>, String>),  // (thread_id, result)
    ThreadAttachmentsLoaded(String, Result<Vec<ThreadAttachment>, String>),  // (thread_id, result)
    ToggleMessageExpanded(usize),  // index into thread_messages
    ToggleAllMessages,             // expand all / collapse all
    ToggleAttachmentsCollapsed,
    SetDateDisplay(DateDisplay),
}
```

**New enum for date display setting:**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateDisplay {
    RelativeOffset,  // Option A: "+14d"
    Absolute,        // Option B: "Mar 12, 2026 at 2:34 PM"
}
```

### 2.3 App State Extension (`src/main.rs`)

**New fields on `App`:**

```rust
struct App {
    // ... existing ...
    thread_messages: Vec<ThreadMessage>,
    thread_attachments: Vec<ThreadAttachment>,
    message_expanded: Vec<bool>,        // parallel to thread_messages
    attachments_collapsed: bool,
    date_display: DateDisplay,
}
```

Initialize in `boot()`:

```rust
thread_messages: Vec::new(),
thread_attachments: Vec::new(),
message_expanded: Vec::new(),
attachments_collapsed: false,
date_display: DateDisplay::RelativeOffset,  // default to Option A
```

### 2.4 Thread Selection Loading

Update `SelectThread` handler to fire parallel loads:

```rust
Message::SelectThread(idx) => {
    self.selected_thread = Some(idx);
    if let Some(thread) = self.threads.get(idx) {
        let db = Arc::clone(&self.db);
        let account_id = thread.account_id.clone();
        let thread_id = thread.id.clone();
        let db2 = Arc::clone(&self.db);
        let account_id2 = account_id.clone();
        let thread_id2 = thread_id.clone();
        let tid = thread_id.clone();
        let tid2 = thread_id.clone();
        return Task::batch([
            Task::perform(
                async move { db.get_thread_messages(account_id, thread_id).await },
                move |result| Message::ThreadMessagesLoaded(tid.clone(), result),
            ),
            Task::perform(
                async move { db2.get_thread_attachments(account_id2, thread_id2).await },
                move |result| Message::ThreadAttachmentsLoaded(tid2.clone(), result),
            ),
        ]);
    }
    Task::none()
}
```

### 2.5 Collapse Rules

In `ThreadMessagesLoaded` handler, compute `message_expanded` from collapse rules:

```rust
Message::ThreadMessagesLoaded(thread_id, Ok(messages)) => {
    // Discard stale response — user may have selected a different thread
    let current_thread_id = self.selected_thread
        .and_then(|i| self.threads.get(i))
        .map(|t| t.id.as_str());
    if current_thread_id != Some(thread_id.as_str()) {
        return Task::none();
    }

    let len = messages.len();
    let mut expanded = vec![false; len];

    for (i, msg) in messages.iter().enumerate() {
        let is_most_recent = i == 0;          // newest first
        let is_initial = i == len - 1;        // oldest = initial
        let is_unread = !msg.is_read;

        // Rule 1: unread — always expanded
        if is_unread {
            expanded[i] = true;
            continue;
        }
        // Rule 2: most recent — always expanded
        if is_most_recent {
            expanded[i] = true;
            continue;
        }
        // Rule 3: initial message — expanded (ownership check deferred)
        if is_initial {
            expanded[i] = true;
            continue;
        }
        // Rule 4: user's own messages — collapsed (needs identity matching, deferred)
        // Rule 5: everything else — collapsed
    }

    self.message_expanded = expanded;
    self.thread_messages = messages;
    Task::none()
}
```

**Intentionally approximate.** Rule 4 (own-message detection) requires identity matching from the backend (implementation spec Slice 2, Step 3). Rule 5's collapsed summaries should be quote/signature-stripped (implementation spec Slice 2, Step 4), but this prototype uses raw snippet truncation. Both will converge to the full spec when the backend `get_thread_detail()` function lands. The rule structure and UI components are in place — only the data source changes.

### 2.6 Reading Pane Redesign (`src/ui/reading_pane.rs`)

The current reading pane receives a single `&Thread` and renders one message card. The redesign renders a full conversation view with stacked messages.

**New function signature:**

```rust
pub fn view<'a>(
    thread: Option<&'a Thread>,
    messages: &'a [ThreadMessage],
    message_expanded: &'a [bool],
    attachments: &'a [ThreadAttachment],
    attachments_collapsed: bool,
    date_display: DateDisplay,
) -> Element<'a, Message>
```

**Widget tree (thread selected):**

```
column(spacing=0).width(Fill)
  // Thread header
  container(PAD_CONTENT)
    column(spacing=SPACE_XXS)
      row(align_y=Center)
        text(subject).size(TEXT_HEADING)
        Space::Fill
        star_toggle_button
      row(align_y=Center)
        text("{n} messages").size(TEXT_SM).style(text_tertiary)
        Space::Fill
        expand_collapse_toggle

  // Attachment group (if any non-inline attachments)
  attachment_group(attachments, attachments_collapsed)

  // Scrollable message list
  scrollable(height=Fill)
    column(spacing=SPACE_XS).padding([0, SPACE_LG])
      for (i, msg) in messages.iter().enumerate():
        if message_expanded[i]:
          expanded_message_card(msg, i, date_display)
        else:
          collapsed_message_row(msg, i)
      Space::height(SPACE_MD)
```

### 2.7 Expanded Message Card (`src/ui/widgets.rs`)

New widget function:

```rust
pub fn expanded_message_card<'a>(
    msg: &'a ThreadMessage,
    index: usize,
    date_display: DateDisplay,
    first_message_date: Option<i64>,
) -> Element<'a, Message>
```

**Widget tree:**

```
button(on_press=ToggleMessageExpanded(index)).padding(0).style(bare_transparent_button).width(Fill)
  container(PAD_CARD).style(message_card_container).width(Fill)
    column(spacing=SPACE_XS)
      // Header
      row(spacing=SPACE_SM, align_y=Start)
        avatar_slot: avatar_circle(sender, AVATAR_MESSAGE_CARD)
        column(spacing=SPACE_XXXS).width(Fill)
          row(align_y=Center)
            sender_slot: text(sender).size(TEXT_LG).font(TEXT_SEMIBOLD)
            Space::Fill
            date_slot: text(formatted_date).size(TEXT_SM).style(text_tertiary)
          recipients_slot: text(recipients).size(TEXT_SM).style(text_tertiary)

      // Body (snippet placeholder)
      container(PAD_BODY)
        text(snippet).size(TEXT_LG).style(text_secondary)

      // Per-message actions
      row(spacing=SPACE_XS)
        reply_action("Reply", icon::reply())
        reply_action("Reply All", icon::reply_all())
        reply_action("Forward", icon::forward())
```

Clicking the expanded card sends `ToggleMessageExpanded(index)` to collapse it.

**Date formatting:**

```rust
fn format_message_date(
    timestamp: Option<i64>,
    first_message_timestamp: Option<i64>,
    display: DateDisplay,
) -> String {
    let Some(ts) = timestamp else { return String::new() };
    let Some(dt) = chrono::DateTime::from_timestamp(ts, 0) else { return String::new() };

    match display {
        DateDisplay::RelativeOffset => {
            // Absolute date + relative offset from first message
            let abs = dt.format("%b %d, %Y, %l:%M %p").to_string();
            match first_message_timestamp.and_then(|fts| chrono::DateTime::from_timestamp(fts, 0)) {
                Some(first_dt) => {
                    let days = (dt - first_dt).num_days();
                    if days == 0 {
                        abs.trim().to_string()
                    } else {
                        format!("{} (+{}d)", abs.trim(), days)
                    }
                }
                None => abs.trim().to_string(),
            }
        }
        DateDisplay::Absolute => {
            dt.format("%b %d, %Y, %l:%M %p").to_string().trim().to_string()
        }
    }
}
```

The first message timestamp is `messages.last().and_then(|m| m.date)` (oldest, since the list is newest-first).

### 2.8 Collapsed Message Row (`src/ui/widgets.rs`)

New widget function:

```rust
pub fn collapsed_message_row<'a>(
    msg: &'a ThreadMessage,
    index: usize,
) -> Element<'a, Message>
```

**Widget tree:**

```
button(style=collapsed_message_button).width(Fill).on_press(ToggleMessageExpanded(index))
  container(padding=[SPACE_XXS, SPACE_SM]).width(Fill)
    row(spacing=SPACE_XS, align_y=Center)
      dash_slot: text("—").size(TEXT_SM).style(text_tertiary)
      sender_slot: text(sender).size(TEXT_SM).font(TEXT_SEMIBOLD)
      dot_slot: text("·").size(TEXT_SM).style(text_tertiary)
      date_slot: text(short_date).size(TEXT_SM).style(text_tertiary)
      dot_slot: text("·").size(TEXT_SM).style(text_tertiary)
      snippet_slot: container(text(truncated_snippet).size(TEXT_SM).style(text_tertiary)
          .wrapping(None)).width(Fill)
```

**New style in `theme.rs`:**

```rust
pub fn collapsed_message_button(theme: &Theme, status: button::Status) -> button::Style {
    let p = theme.palette();
    match status {
        button::Status::Hovered => button::Style {
            background: Some(p.background.weakest.color.into()),
            text_color: p.background.base.text,
            border: border::rounded(RADIUS_SM),
            ..Default::default()
        },
        _ => button::Style {
            background: None,
            text_color: p.background.base.text,
            border: border::rounded(RADIUS_SM),
            ..Default::default()
        },
    }
}
```

Hover step: rests on `base` (transparent), hover on `weakest` (one step).

**Snippet truncation:**

```rust
fn truncate_snippet(snippet: Option<&str>, max_chars: usize) -> String {
    let s = snippet.unwrap_or("");
    if s.len() <= max_chars {
        s.to_string()
    } else {
        format!("{}...", &s[..s.floor_char_boundary(max_chars)])
    }
}
```

Use `max_chars = 60` per the problem statement.

### 2.9 Attachment Group (`src/ui/reading_pane.rs`)

Local function, not a widget (it is reading-pane-specific):

```rust
fn attachment_group<'a>(
    attachments: &'a [ThreadAttachment],
    collapsed: bool,
) -> Element<'a, Message>
```

Only rendered when `!attachments.is_empty()`.

**Widget tree:**

```
container(PAD_CONTENT).width(Fill)
  container(style=elevated_container, padding=PAD_CARD)
    column(spacing=SPACE_XS)
      // Header row
      button(on_press=ToggleAttachmentsCollapsed).style(ghost_button).width(Fill)
        row(align_y=Center)
          chevron_slot: icon (chevron_right if collapsed, chevron_down if expanded).size(ICON_XS)
          Space::width(SPACE_XXS)
          title_slot: text("Attachments ({count})").size(TEXT_MD).font(TEXT_SEMIBOLD)
          Space::Fill
          action_slot: text("Save All").size(TEXT_SM).style(text_accent)

      // Attachment cards (only if expanded)
      if !collapsed:
        for att in attachments:
          attachment_card(att)
```

### 2.10 Attachment Card (`src/ui/widgets.rs`)

New widget function:

```rust
pub fn attachment_card<'a>(att: &'a ThreadAttachment) -> Element<'a, Message>
```

**Widget tree:**

```
container(padding=PAD_NAV_ITEM).style(elevated_container).width(Fill)
  column(spacing=SPACE_XXXS)
    // Line 1: icon + filename
    row(spacing=SPACE_XS, align_y=Center)
      icon_slot: container(file_type_icon(mime_type).size(ICON_MD).style(text_secondary))
      filename_slot: text(filename).size(TEXT_MD).wrapping(None)

    // Line 2: type label + size + date + sender
    text(metadata_line).size(TEXT_SM).style(text_tertiary)
```

**File type icon mapping:**

```rust
fn file_type_icon(mime_type: Option<&str>) -> iced::widget::Text<'_> {
    match mime_type.unwrap_or("") {
        t if t.starts_with("image/") => icon::image(),
        t if t.contains("pdf") => icon::file_text(),
        t if t.contains("spreadsheet") || t.contains("excel") => icon::file_spreadsheet(),
        _ => icon::file(),
    }
}
```

This requires verifying which icon codepoints exist in `src/icon.rs`. Add any missing ones (`file`, `image`, `file-text`, `file-spreadsheet`) to the icon module.

**Metadata line formatting:**

```rust
fn format_attachment_meta(att: &ThreadAttachment) -> String {
    let type_label = mime_to_type_label(att.mime_type.as_deref());
    let size = format_file_size(att.size);
    let date = att.date
        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
        .map(|dt| dt.format("%b %d").to_string())
        .unwrap_or_default();
    let sender = att.from_name.as_deref().unwrap_or("unknown");
    format!("{type_label} · {size} · {date} from {sender}")
}

fn mime_to_type_label(mime: Option<&str>) -> &'static str {
    match mime.unwrap_or("") {
        t if t.starts_with("image/") => "Image",
        t if t.contains("pdf") => "PDF",
        t if t.contains("spreadsheet") || t.contains("excel") => "Excel",
        t if t.contains("word") || t.contains("document") => "Word",
        t if t.contains("zip") || t.contains("archive") => "Archive",
        _ => "File",
    }
}

fn format_file_size(size: Option<i64>) -> String {
    match size {
        None => "—".to_string(),
        Some(b) if b < 1024 => format!("{b} B"),
        Some(b) if b < 1024 * 1024 => format!("{:.0} KB", b as f64 / 1024.0),
        Some(b) => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}
```

### 2.11 Star Toggle

A button in the thread header that toggles starred state visually:

```rust
let star_icon_style: fn(&Theme) -> text::Style = if thread.is_starred {
    text::warning
} else {
    text::secondary
};
let star_btn_style: fn(&Theme, button::Status) -> button::Style = if thread.is_starred {
    theme::star_active_button
} else {
    theme::bare_icon_button
};

let star_btn = button(
    icon::star().size(ICON_XL).style(star_icon_style)
)
.on_press(Message::Noop)  // visual only for now
.padding(PAD_ICON_BTN)
.style(star_btn_style);
```

Note: both the icon style and button style must be set explicitly. The button's `text_color` does not propagate to children with their own `.style()` (per CLAUDE.md: "Button `text_color` doesn't reach children with explicit `.style()`").

**New style in `theme.rs`:**

```rust
pub fn star_active_button(theme: &Theme, status: button::Status) -> button::Style {
    let p = theme.palette();
    match status {
        button::Status::Hovered => button::Style {
            background: Some(p.warning.base.color.scale_alpha(0.2).into()),
            text_color: p.warning.base.color,
            border: border::rounded(RADIUS_SM),
            ..Default::default()
        },
        _ => button::Style {
            background: Some(p.warning.base.color.scale_alpha(0.1).into()),
            text_color: p.warning.base.color,
            border: border::rounded(RADIUS_SM),
            ..Default::default()
        },
    }
}
```

### 2.12 Empty State

When no thread is selected, the reading pane shows:

```rust
widgets::empty_placeholder("No conversation selected", "Select a thread to read")
```

This already exists. No change needed.

### 2.13 Expand/Collapse All

A subtle toggle in the thread header:

```rust
let toggle_label = if message_expanded.iter().all(|&e| e) {
    "Collapse all"
} else {
    "Expand all"
};

button(text(toggle_label).size(TEXT_SM).style(theme::text_tertiary))
    .on_press(Message::ToggleAllMessages)
    .style(theme::ghost_button)
    .padding(PAD_ICON_BTN)
```

**Handler:**

```rust
Message::ToggleAllMessages => {
    let all_expanded = self.message_expanded.iter().all(|&e| e);
    for e in &mut self.message_expanded {
        *e = !all_expanded;
    }
    Task::none()
}
```

### 2.14 Date Display Setting

Add to `SettingsState` (in `src/ui/settings.rs`):

```rust
pub date_display: DateDisplay,
```

Add a new `SettingsMessage` variant:

```rust
DateDisplayChanged(String),
```

In the General settings tab, add a row using the existing `select` widget:

```rust
// Date display format
settings_row(
    "Message Dates",
    "How dates appear on messages in the conversation view",
    select(
        &["Relative Offset", "Absolute"],
        match self.date_display {
            DateDisplay::RelativeOffset => "Relative Offset",
            DateDisplay::Absolute => "Absolute",
        },
        self.open_select == Some(SelectField::DateDisplay),
        SettingsMessage::ToggleSelect(SelectField::DateDisplay),
        |s| SettingsMessage::DateDisplayChanged(s),
    ),
)
```

Add `DateDisplay` to the `SelectField` enum in settings. The setting value flows from `App.date_display` through to `reading_pane::view`.

---

## Phase 3: Deferred

**Keyboard shortcuts, auto-advance, and multi-select are deferred.** The main-layout problem statement requires shortcuts routed through the command palette with `focused_region`-aware dispatch (`docs/main-layout/problem-statement.md` § Context-Dependent Shortcuts). Building a direct keyboard dispatch system in crates/app would create throwaway code that conflicts with the command palette integration.

Phase 3 work resumes when:
1. Backend Slice 4 (`FocusedRegion` on `CommandContext`) is implemented
2. The command palette has an iced integration path (see `docs/command-palette/problem-statement.md`)

Until then, all interaction is mouse-only: click to select threads, click to expand/collapse messages, click action buttons.

---

## Phase 4: Polish

### 4.1 Panel Width Persistence Verification

Verify that panel widths round-trip correctly through `window.json`:
- Drag sidebar divider, close app, reopen — sidebar width persists
- Drag thread list divider, close app, reopen — thread list width persists
- Toggle right sidebar, close app, reopen — right sidebar state persists
- Delete `window.json`, reopen — defaults applied cleanly

### 4.2 Attachment Collapse Cache

In-memory `HashMap<String, bool>` on `App` keyed by `thread_id`:

```rust
struct App {
    // ... existing ...
    attachment_collapse_cache: std::collections::HashMap<String, bool>,
}
```

When `ToggleAttachmentsCollapsed` fires, update the cache:

```rust
Message::ToggleAttachmentsCollapsed => {
    self.attachments_collapsed = !self.attachments_collapsed;
    if let Some(thread) = self.selected_thread.and_then(|i| self.threads.get(i)) {
        self.attachment_collapse_cache
            .insert(thread.id.clone(), self.attachments_collapsed);
    }
    Task::none()
}
```

When selecting a thread, check the cache:

```rust
// In SelectThread handler, after setting selected_thread:
self.attachments_collapsed = self
    .selected_thread
    .and_then(|i| self.threads.get(i))
    .and_then(|t| self.attachment_collapse_cache.get(&t.id))
    .copied()
    .unwrap_or(false);  // default expanded
```

This is intentionally ephemeral (lost on app restart). The product spec requires per-thread persistence in SQLite (`thread_ui_state` table, backend implementation spec Slice 3). The in-memory HashMap is the interim implementation — it will be replaced by calls to `get_attachments_collapsed()` / `set_attachments_collapsed()` once the backend migration and functions land. The HashMap provides the correct UX within a session; persistence across sessions is blocked on backend work, not a design disagreement.

### 4.3 Empty States

**Thread list empty state** (no threads in current folder):

In `thread_list::view`, when `threads.is_empty()`, replace the scrollable list with:

```rust
widgets::empty_placeholder("No conversations", "This folder is empty")
```

**Right sidebar empty sections** are already handled by the scaffold placeholder text.

**Reading pane empty state** already exists: `"No conversation selected"`.

### 4.4 Micro-Interactions

**Thread card press feedback:** The button style already handles hover via `thread_card_button`. No additional press state needed.

**Collapse/expand transition:** iced does not support animated height transitions natively. Collapse/expand is instant (toggle visibility). If animation is desired later, it requires a custom widget with `animation::spring` — out of scope for Phase 4.

**Right sidebar slide-in:** The settings overlay already uses `OverlayAnim` for slide animation. The right sidebar is a layout panel (not an overlay), so instant toggle is appropriate.

---

## File Change Summary

### New files

| File | Phase | Purpose |
|------|-------|---------|
| `src/ui/right_sidebar.rs` | 1 | Right sidebar (replaces contact_sidebar) |

### Modified files

| File | Phase | Changes |
|------|-------|---------|
| `src/ui/layout.rs` | 1 | `THREAD_LIST_WIDTH` 280->400, rename `CONTACT_SIDEBAR_WIDTH`->`RIGHT_SIDEBAR_WIDTH`, add `LABEL_DOT_SIZE`, `THREAD_CARD_HEIGHT`, `PAD_RIGHT_SIDEBAR`, `STARRED_BG_ALPHA` |
| `src/ui/theme.rs` | 1,2 | Update `thread_card_button` signature (add `starred`), add `collapsed_message_button`, `star_active_button` |
| `src/ui/widgets.rs` | 1,2 | Redesign `thread_card` (no avatar, 3-line, label dots), add `label_dot`, `expanded_message_card`, `collapsed_message_row`, `attachment_card` |
| `src/ui/thread_list.rs` | 1 | Update `view` signature (add `folder_name`, `scope_name`), add context line, pass `label_colors` |
| `src/ui/reading_pane.rs` | 2 | Full redesign: stacked messages, collapse, attachment group, star toggle, expand/collapse all |
| `src/ui/mod.rs` | 1 | Replace `contact_sidebar` with `right_sidebar` |
| `src/main.rs` | 1,2 | Add `Message` variants, `App` fields, thread message loading with stale-response guards, collapse handlers, right sidebar toggle + auto-collapse |
| `src/window_state.rs` | 1 | Add `sidebar_width`, `thread_list_width`, `right_sidebar_open` fields |
| `src/db.rs` | 2 | Add `ThreadMessage`, `ThreadAttachment` types and queries |
| `src/ui/settings.rs` | 2 | Add date display setting (`DateDisplay` select, `SelectField::DateDisplay`) |

### Deleted files

| File | Phase | Reason |
|------|-------|--------|
| `src/ui/contact_sidebar.rs` | 1 | Replaced by `right_sidebar.rs` |

### Constants reference

All new constants use the existing spacing/type/icon scales:

- `LABEL_DOT_SIZE` = 6.0 (between `SPACE_XXS` 4 and `SPACE_XS` 8)
- `THREAD_CARD_HEIGHT` = 68.0 (3 lines of `TEXT_MD`/`TEXT_SM` + `PAD_THREAD_CARD` vertical)
- `PAD_RIGHT_SIDEBAR` = `Padding::new(12.0)` (matches `PAD_PANEL_HEADER` scale)
- `STARRED_BG_ALPHA` = 0.12 (consistent with existing alpha usage in `chip_button`)
- `RIGHT_SIDEBAR_AUTO_COLLAPSE_WIDTH` = 1200.0 (window width threshold for auto-collapse)

---

## Ecosystem Patterns

Patterns from the [iced ecosystem survey](../iced-ecosystem-survey.md) that apply to this spec, adapted from the [cross-reference](../iced-ecosystem-cross-reference.md).

### Requirements → Survey Matches

| Requirement | Primary Source | How It Applies |
|---|---|---|
| Resizable panels (sidebar, thread list) | shadcn-rs resizable panels | `auto_save_id` could replace manual persistence; min/max constraints more robust than `sanitize()` clamp |
| Starred thread card golden tint | rustcast `tint()`/`with_alpha()` | Validates spec's existing `mix()` helper approach |
| Stale thread detail responses | bloom generational tracking | Replace thread_id staleness check with `load_generation` counter for robustness (handles re-selecting same thread) |
| Phase 3 keyboard shortcuts | raffi query routing + trebuchet Component trait + cedilla key bindings + feu raw keyboard | Component trait is highest-impact — prevents Message enum explosion |
| Data table selection model | shadcn-rs data table | `selected_indices: HashSet`, `anchor_index` for shift-range, `active_index` for keyboard nav |
| Attachment collapse toggle | bloom config shadow | HashMap cache is correct for interim; bloom pattern informs SQLite migration |

### Gaps

- **Thread list virtualization**: No surveyed project implements virtualized scrolling for iced (fixed `THREAD_CARD_HEIGHT` enables future virtualization)
- **Auto-collapse right sidebar below 1200px**: One-directional collapse policy is custom; shadcn-rs panels don't encode this
