# Pop-Out Message View: Implementation Spec

Phased implementation spec for the pop-out message view window. This is the simpler half of the pop-out windows feature — compose pop-out is a separate spec with heavier dependencies (editor, contacts autocomplete).

**Product spec:** `docs/pop-out-windows/problem-statement.md`
**Tier:** 3 in `docs/implementation-plan.md` — mostly independent, no heavy blockers.

**Shared infrastructure note:** Phase 1 of this spec establishes multi-window architecture that all pop-out windows will share — compose, message view, and eventually calendar. Implementers should understand that Phase 1 is shared platform work, not feature-local scaffolding. The window registry, daemon migration, per-window routing, and cascade-close behavior are foundational infrastructure reused by every future pop-out spec.

## Table of Contents

1. [Multi-Window Architecture in iced](#phase-1-multi-window-architecture)
2. [Message View Window](#phase-2-message-view-window)
3. [Rendering Modes](#phase-3-rendering-modes)
4. [Action Buttons](#phase-4-action-buttons)
5. [Session Restore](#phase-5-session-restore)
6. [Save As (.eml, .txt)](#phase-6-save-as)

---

## Phase 1: Multi-Window Architecture

The biggest technical challenge. No iced project in the ecosystem survey uses multi-window. This phase establishes the infrastructure that all pop-out windows (message view, compose, calendar) will share.

### The `application` vs `daemon` Decision

The app currently uses `iced::application`, which provides a single-window API:

```rust
// Current: view takes &self only, no window ID
fn view(&self) -> Element<'_, Message>
fn title(&self) -> String  // via .title() builder
```

For multi-window, iced provides `iced::daemon`, which exposes window IDs:

```rust
// Daemon: view receives the window ID
fn view(&self, window: window::Id) -> Element<'_, Message>
fn title(&self, window: window::Id) -> String  // per-window titles
```

Internally, both map to the same `Program` trait which always takes `window::Id` for `view()` and `title()`. The `application` API wraps user closures to discard the ID. Switching to `daemon` is the correct path for multi-window support.

**Key difference:** A `daemon` does not open a window by default and does not exit when all windows close. The app must explicitly open the main window in `boot()` and call `iced::exit()` when the main window closes.

### Migration from `application` to `daemon`

The current `main()` initializer:

```rust
let mut app = iced::application(App::boot, App::update, App::view)
    .title("Ratatoskr (iced prototype)")
    .theme(App::theme)
    .scale_factor(|app| app.settings.scale)
    .subscription(App::subscription)
    .default_font(font::text())
    .window(window.to_window_settings());
```

Becomes:

```rust
let mut app = iced::daemon(App::boot, App::update, App::view)
    .title(App::title)
    .theme(App::theme)
    .scale_factor(|app, _window| app.settings.scale)
    .subscription(App::subscription)
    .default_font(font::text());
```

The `.window()` call is removed — the main window is opened explicitly in `boot()` via `iced::window::open()`.

### Window Registry

A central `HashMap<window::Id, PopOutWindow>` on `App` tracks all open pop-out windows. The main window is identified by a stored `window::Id` field.

```rust
/// Identifies what a window is showing.
#[derive(Debug, Clone)]
pub enum PopOutWindow {
    MessageView(MessageViewState),
    // Future variants:
    // Compose(ComposeWindowState),
    // Calendar(CalendarWindowState),
}

struct App {
    // ... existing fields ...

    /// The main window's ID, assigned during boot.
    main_window_id: window::Id,

    /// All open pop-out windows. The main window is NOT in this map.
    pop_out_windows: HashMap<window::Id, PopOutWindow>,
}
```

### Boot: Opening the Main Window

In `boot()`, the app opens the main window explicitly and stores its ID:

```rust
fn boot() -> (Self, Task<Message>) {
    let db = Arc::clone(DB.get().expect("DB not initialized"));
    let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
    let window = window_state::WindowState::load(data_dir);

    let (main_window_id, open_task) = iced::window::open(window.to_window_settings());

    let app = Self {
        db,
        main_window_id,
        pop_out_windows: HashMap::new(),
        // ... rest unchanged ...
    };

    let load_gen = app.nav_generation;
    let db_ref = Arc::clone(&app.db);

    (app, Task::batch([
        open_task.discard(),
        Task::perform(
            async move { (load_gen, load_accounts(db_ref).await) },
            |(g, result)| Message::AccountsLoaded(g, result),
        ),
    ]))
}
```

### View Routing

The `view` method now takes a `window::Id` and routes to the correct view:

```rust
fn view(&self, window_id: window::Id) -> Element<'_, Message> {
    if window_id == self.main_window_id {
        return self.view_main_window();
    }

    if let Some(pop_out) = self.pop_out_windows.get(&window_id) {
        return match pop_out {
            PopOutWindow::MessageView(state) => {
                self.view_message_window(window_id, state)
            }
        };
    }

    // Fallback for unknown window IDs (should not happen)
    widgets::empty_placeholder("Window not found", "").into()
}
```

The existing `view()` body moves into `view_main_window()`.

### Title Routing

```rust
fn title(&self, window_id: window::Id) -> String {
    if window_id == self.main_window_id {
        return "Ratatoskr".to_string();
    }

    if let Some(PopOutWindow::MessageView(state)) = self.pop_out_windows.get(&window_id) {
        let subject = state.subject.as_deref().unwrap_or("(no subject)");
        let sender = state.from_address.as_deref().unwrap_or("unknown");
        return format!("{subject} \u{2014} {sender}");
    }

    "Ratatoskr".to_string()
}
```

### Message Routing

All pop-out window messages are wrapped in a single `Message` variant that carries the window ID:

```rust
#[derive(Debug, Clone)]
pub enum Message {
    // ... existing variants ...

    /// A message targeting a specific pop-out window.
    PopOut(window::Id, PopOutMessage),

    /// Open a message view pop-out for a specific message.
    OpenMessageView {
        account_id: String,
        thread_id: String,
        message_id: String,
    },

    /// A pop-out window was resized.
    PopOutResized(window::Id, Size),

    /// A pop-out window was moved.
    PopOutMoved(window::Id, Point),
}

/// Messages internal to pop-out windows.
#[derive(Debug, Clone)]
pub enum PopOutMessage {
    MessageView(MessageViewMessage),
    // Future: Compose(ComposeMessage), Calendar(CalendarMessage)
}
```

### Window Close Handling

The existing `WindowCloseRequested` handler must distinguish the main window from pop-outs:

```rust
fn handle_window_close(&mut self, id: window::Id) -> Task<Message> {
    if id == self.main_window_id {
        // Save session state (main window + all pop-outs)
        self.save_session_state();
        // Close all pop-out windows, then exit
        let mut tasks: Vec<Task<Message>> = self.pop_out_windows
            .keys()
            .map(|&win_id| iced::window::close(win_id))
            .collect();
        tasks.push(iced::window::close(id));
        tasks.push(iced::exit());
        return Task::batch(tasks);
    }

    // Pop-out window closed — just remove it from the registry
    self.pop_out_windows.remove(&id);
    iced::window::close(id)
}
```

### Subscription Updates

Window events (resize, move, close) now carry window IDs. The subscription must route them:

```rust
fn subscription(&self) -> iced::Subscription<Message> {
    let mut subs = vec![
        appearance::subscription().map(Message::AppearanceChanged),
        iced::window::close_requests().map(Message::WindowCloseRequested),
        iced::window::resize_events().map(|(id, size)| {
            Message::WindowResized(id, size)  // now includes window ID
        }),
        iced::event::listen_with(|event, _status, id| {
            if let iced::Event::Window(iced::window::Event::Moved(point)) = event {
                Some(Message::WindowMoved(id, point))
            } else {
                None
            }
        }),
        // ... component subscriptions unchanged ...
    ];
    // ...
}
```

The `WindowResized` and `WindowMoved` message variants must be updated to carry `window::Id`:

```rust
WindowResized(window::Id, Size),
WindowMoved(window::Id, Point),
```

The handler checks which window the event targets:

```rust
Message::WindowResized(id, size) => {
    if id == self.main_window_id {
        self.window.set_size(size);
        if size.width < RIGHT_SIDEBAR_AUTO_COLLAPSE_WIDTH
            && self.right_sidebar_open
        {
            self.right_sidebar_open = false;
        }
    } else if let Some(PopOutWindow::MessageView(state)) =
        self.pop_out_windows.get_mut(&id)
    {
        state.width = size.width;
        state.height = size.height;
    }
    Task::none()
}
```

### Keyboard Shortcuts

Escape closes the focused pop-out window. This is handled via the event subscription:

```rust
iced::event::listen_with(|event, _status, window_id| {
    match event {
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
            ..
        }) => Some(Message::WindowCloseRequested(window_id)),
        iced::Event::Window(iced::window::Event::Moved(point)) => {
            Some(Message::WindowMoved(window_id, point))
        }
        _ => None,
    }
})
```

Note: Escape on the main window should not close it — the close handler already distinguishes main from pop-out. However, Escape on the main window may have other meanings (dismiss command palette, deselect thread). The Escape subscription should only fire `WindowCloseRequested` for pop-out window IDs, not the main window:

```rust
iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
    ..
}) => {
    // Only close pop-out windows with Escape; main window Escape
    // is handled by the command palette / focus system.
    // The window_id is provided by the subscription — we'll check
    // in the handler whether it's a pop-out.
    Some(Message::EscapePressed(window_id))
}
```

Then in `update`:

```rust
Message::EscapePressed(id) => {
    if id != self.main_window_id && self.pop_out_windows.contains_key(&id) {
        self.handle_window_close(id)
    } else {
        // Dispatch to main window's Escape handling
        // (dismiss palette, deselect, etc.)
        self.handle_main_escape()
    }
}
```

### Phase 1 Deliverables

1. Switch from `iced::application` to `iced::daemon`.
2. Open main window in `boot()`, store `main_window_id`.
3. Add `pop_out_windows: HashMap<window::Id, PopOutWindow>` to `App`.
4. Route `view()` and `title()` by window ID.
5. Update `WindowResized`, `WindowMoved`, `WindowCloseRequested` to carry window IDs.
6. Main window close cascades to all pop-outs, then exits.
7. Pop-out close removes from registry.
8. Escape closes pop-out windows.

---

## Phase 2: Message View Window

### Opening a Message View

Triggered by double-clicking a message card in the conversation view. The reading pane's expanded message card needs a double-click handler.

#### Trigger: Double-Click on Message Card

Add a new event from `ReadingPane`:

```rust
#[derive(Debug, Clone)]
pub enum ReadingPaneEvent {
    AttachmentCollapseChanged { thread_key: String, collapsed: bool },
    /// User double-clicked a message; open it in a pop-out window.
    OpenMessagePopOut { message_index: usize },
}
```

The expanded message card (in `widgets.rs`) needs to emit this on double-click. Since iced's `button` widget does not have a double-click event, this requires either:

1. **A `mouse_area` wrapper** with custom double-click detection (track time between clicks).
2. **A dedicated message** where the app tracks click timing.

Option 1 is cleaner. Wrap the message card in a `mouse_area` that detects double-click via `on_double_click` if available in the iced fork, or track via `on_press` with a timestamp. The implementation detail depends on whether the local iced fork has `on_double_click` on `mouse_area`.

Fallback approach: Add a small "pop out" icon button on each expanded message card header that opens the pop-out on single click. This is more discoverable and avoids the double-click detection problem. **Recommendation: implement both** — the icon button as the primary affordance, double-click as the power-user shortcut.

#### Pop-Out Icon Button

Add to the expanded message card header row (right-aligned, next to the date):

```rust
// In expanded_message_card(), add to the header's top-right area:
let pop_out_btn = button(
    icon::external_link().size(ICON_MD).style(text::secondary)
)
.on_press(on_pop_out(index))
.padding(PAD_ICON_BTN)
.style(theme::ButtonClass::BareIcon.style());
```

This requires adding an `on_pop_out` callback parameter to `expanded_message_card` (or emitting a new `ReadingPaneMessage::PopOut(usize)` variant).

#### Opening the Window

When `App` receives the pop-out event, it opens a new window:

```rust
fn open_message_view_window(
    &mut self,
    message_index: usize,
) -> Task<Message> {
    // Get the message data from the reading pane
    let Some(msg) = self.reading_pane.thread_messages.get(message_index) else {
        return Task::none();
    };

    let state = MessageViewState::from_thread_message(msg);

    let settings = window::Settings {
        size: Size::new(
            MESSAGE_VIEW_DEFAULT_WIDTH,
            MESSAGE_VIEW_DEFAULT_HEIGHT,
        ),
        min_size: Some(Size::new(
            MESSAGE_VIEW_MIN_WIDTH,
            MESSAGE_VIEW_MIN_HEIGHT,
        )),
        exit_on_close_request: false,
        ..Default::default()
    };

    let (window_id, open_task) = iced::window::open(settings);
    self.pop_out_windows.insert(
        window_id,
        PopOutWindow::MessageView(state),
    );

    open_task.discard()
}
```

### Message View State

```rust
/// Per-window state for a message view pop-out.
#[derive(Debug, Clone)]
pub struct MessageViewState {
    // ── Identity ──
    /// The message's unique ID (for session restore).
    pub message_id: String,
    pub thread_id: String,
    pub account_id: String,

    // ── Header data ──
    pub from_name: Option<String>,
    pub from_address: Option<String>,
    pub to_addresses: Option<String>,
    pub cc_addresses: Option<String>,
    pub subject: Option<String>,
    pub date: Option<i64>,

    // ── Body ──
    /// Plain text body (from body store or snippet fallback).
    pub body_text: Option<String>,
    /// HTML body (from body store).
    pub body_html: Option<String>,
    /// Raw email source (headers + MIME), loaded lazily.
    pub raw_source: Option<String>,

    // ── Attachments ──
    pub attachments: Vec<MessageViewAttachment>,

    // ── Window-local state (not persisted beyond session restore) ──
    pub rendering_mode: RenderingMode,
    pub scroll_offset: f32,

    // ── Window geometry ──
    pub width: f32,
    pub height: f32,
    pub x: Option<f32>,
    pub y: Option<f32>,
}

/// Attachment data for a single message (no deduplication needed).
#[derive(Debug, Clone)]
pub struct MessageViewAttachment {
    pub id: String,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size: Option<i64>,
}

/// Rendering mode for the message body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderingMode {
    PlainText,
    #[default]
    SimpleHtml,
    OriginalHtml,
    Source,
}
```

#### Construction

```rust
// V1 data model note: MessageViewState is seeded from the app's ThreadMessage
// type, which is a prototype-level model missing some fields (cc_addresses,
// bcc_addresses). The long-term path is to seed from core's ThreadDetailMessage
// (from get_thread_detail()), which has the full field set. This is acceptable
// for the first iteration because the async body/attachment loads fill in the
// critical missing data after window creation.
impl MessageViewState {
    pub fn from_thread_message(msg: &ThreadMessage) -> Self {
        Self {
            message_id: msg.id.clone(),
            thread_id: msg.thread_id.clone(),
            account_id: msg.account_id.clone(),
            from_name: msg.from_name.clone(),
            from_address: msg.from_address.clone(),
            to_addresses: msg.to_addresses.clone(),
            cc_addresses: None, // ThreadMessage doesn't have cc yet
            subject: msg.subject.clone(),
            date: msg.date,
            body_text: None,   // Loaded async after window opens
            body_html: None,   // Loaded async after window opens
            raw_source: None,  // Loaded lazily on Source mode
            attachments: Vec::new(), // Loaded async
            rendering_mode: RenderingMode::default(),
            scroll_offset: 0.0,
            width: MESSAGE_VIEW_DEFAULT_WIDTH,
            height: MESSAGE_VIEW_DEFAULT_HEIGHT,
            x: None,
            y: None,
        }
    }
}
```

The body and attachments are loaded asynchronously after the window opens, using the same DB access pattern as the reading pane's thread detail loading. A generation counter per pop-out window prevents stale responses.

### Messages

```rust
#[derive(Debug, Clone)]
pub enum MessageViewMessage {
    /// Body content loaded from the body store.
    BodyLoaded(Result<(Option<String>, Option<String>), String>),
    /// Attachments loaded for this message.
    AttachmentsLoaded(Result<Vec<MessageViewAttachment>, String>),
    /// Raw source loaded (for Source rendering mode).
    RawSourceLoaded(Result<String, String>),
    /// User changed the rendering mode toggle.
    SetRenderingMode(RenderingMode),
    /// Reply/Reply All/Forward button pressed.
    Reply,
    ReplyAll,
    Forward,
    /// Overflow menu actions.
    Archive,
    Delete,
    Print,
    SaveAs,
    /// Overflow menu toggle.
    ToggleOverflowMenu,
    /// No-op (placeholder for unimplemented actions).
    Noop,
}
```

### Layout Constants

Add to `layout.rs`:

```rust
// ── Message view pop-out window ─────────────────────────
pub const MESSAGE_VIEW_DEFAULT_WIDTH: f32 = 800.0;
pub const MESSAGE_VIEW_DEFAULT_HEIGHT: f32 = 600.0;
pub const MESSAGE_VIEW_MIN_WIDTH: f32 = 480.0;
pub const MESSAGE_VIEW_MIN_HEIGHT: f32 = 320.0;
```

### View: Message View Window

The message view window is a single scrollable column.

```rust
fn view_message_window<'a>(
    &'a self,
    window_id: window::Id,
    state: &'a MessageViewState,
) -> Element<'a, Message> {
    let header = message_view_header(state);
    let body = message_view_body(state);
    let attachments = message_view_attachments(&state.attachments);

    let mut content = column![header].spacing(SPACE_0);

    content = content.push(widgets::divider());
    content = content.push(body);

    if !state.attachments.is_empty() {
        content = content.push(widgets::divider());
        content = content.push(attachments);
    }

    let scrollable_content = scrollable(content)
        .spacing(SCROLLBAR_SPACING)
        .height(Length::Fill);

    container(scrollable_content)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::ContainerClass::Content.style())
        .into()
}
```

### Header Layout

The header follows the problem statement's layout:

```
From: Alice Smith              [Reply] [Reply All] [Forward] [...]
      alice@corp.com
To: Bob Jones, Charlie

Re: Sprint Planning       Wed, Mar 19, 2026 10:42 AM
```

```rust
fn message_view_header<'a>(
    state: &'a MessageViewState,
) -> Element<'a, Message> {
    let sender_name = state.from_name.as_deref()
        .or(state.from_address.as_deref())
        .unwrap_or("(unknown)");
    let sender_email = state.from_address.as_deref().unwrap_or("");

    // Action buttons (right-aligned on sender name row)
    let actions = row![
        widgets::action_icon_button(
            icon::reply(),
            "Reply",
            Message::PopOut(window_id, PopOutMessage::MessageView(
                MessageViewMessage::Reply,
            )),
        ),
        widgets::action_icon_button(
            icon::reply_all(),
            "Reply All",
            Message::PopOut(window_id, PopOutMessage::MessageView(
                MessageViewMessage::ReplyAll,
            )),
        ),
        widgets::action_icon_button(
            icon::forward(),
            "Forward",
            Message::PopOut(window_id, PopOutMessage::MessageView(
                MessageViewMessage::Forward,
            )),
        ),
        // Overflow menu button (Phase 4)
    ]
    .spacing(SPACE_XXS);

    // From row: name + actions
    let from_row = row![
        column![
            text(sender_name)
                .size(TEXT_LG)
                .font(font::text_semibold())
                .style(text::base),
            text(sender_email)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        ]
        .spacing(SPACE_XXXS)
        .width(Length::Fill),
        actions,
    ]
    .align_y(Alignment::Start);

    // To row
    let to_text = state.to_addresses.as_deref().unwrap_or("");
    let mut header_fields = column![from_row].spacing(SPACE_XS);

    if !to_text.is_empty() {
        header_fields = header_fields.push(
            row![
                text("To: ").size(TEXT_SM).style(theme::TextClass::Tertiary.style()),
                text(to_text).size(TEXT_SM).style(text::secondary),
            ]
            .spacing(SPACE_XXS),
        );
    }

    // Cc row (if present)
    if let Some(cc) = &state.cc_addresses {
        if !cc.is_empty() {
            header_fields = header_fields.push(
                row![
                    text("Cc: ").size(TEXT_SM).style(theme::TextClass::Tertiary.style()),
                    text(cc.as_str()).size(TEXT_SM).style(text::secondary),
                ]
                .spacing(SPACE_XXS),
            );
        }
    }

    // Subject + date row
    let date_str = state.date
        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
        .map(|dt| dt.format("%a, %b %d, %Y, %l:%M %p").to_string())
        .unwrap_or_default();

    let subject = state.subject.as_deref().unwrap_or("(no subject)");
    let subject_row = row![
        text(subject)
            .size(TEXT_HEADING)
            .style(text::base)
            .width(Length::Fill),
        text(date_str.trim())
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style()),
    ]
    .align_y(Alignment::End);

    header_fields = header_fields.push(subject_row);

    container(header_fields)
        .padding(PAD_CONTENT)
        .width(Length::Fill)
        .into()
}
```

### Body Rendering

**Phased deviation from product spec:** The problem statement treats full rendered message body as core to the message-view window. Phase 2 delivers plain text only (using `body_text` or snippet fallback). Full HTML rendering (Simple HTML, Original HTML with remote-content controls) arrives in Phase 3. This is a deliberate phasing choice — the multi-window infrastructure and basic message display ship first, rendering fidelity follows. Phase 2 is functionally useful (users can reference message content) but visually incomplete.

```rust
fn message_view_body<'a>(
    state: &'a MessageViewState,
) -> Element<'a, Message> {
    let body_content = match state.rendering_mode {
        RenderingMode::PlainText => {
            let txt = state.body_text.as_deref()
                .or(state.subject.as_deref()) // fallback
                .unwrap_or("(no content)");
            text(txt).size(TEXT_LG).style(text::secondary).into()
        }
        RenderingMode::SimpleHtml | RenderingMode::OriginalHtml => {
            // Phase 3: HTML rendering pipeline
            // For now, fall back to plain text
            let txt = state.body_text.as_deref()
                .unwrap_or("(no content)");
            text(txt).size(TEXT_LG).style(text::secondary).into()
        }
        RenderingMode::Source => {
            let src = state.raw_source.as_deref()
                .unwrap_or("Loading source...");
            text(src)
                .size(TEXT_SM)
                .font(font::monospace())
                .style(text::secondary)
                .into()
        }
    };

    container(body_content)
        .padding(PAD_CONTENT)
        .width(Length::Fill)
        .into()
}
```

### Attachment List

Single-message attachments — no deduplication needed (per the problem statement).

```rust
fn message_view_attachments<'a>(
    attachments: &'a [MessageViewAttachment],
) -> Element<'a, Message> {
    let header = text(format!("Attachments ({})", attachments.len()))
        .size(TEXT_MD)
        .font(font::text_semibold())
        .style(text::base);

    let mut col = column![header].spacing(SPACE_XS);

    for att in attachments {
        let filename = att.filename.as_deref().unwrap_or("(unnamed)");
        let file_icon = file_type_icon(att.mime_type.as_deref());
        let size_str = format_file_size(att.size);
        let type_label = mime_to_type_label(att.mime_type.as_deref());
        let meta = format!("{type_label} \u{00B7} {size_str}");

        let card = container(
            column![
                row![
                    container(file_icon.size(ICON_MD).style(text::secondary))
                        .align_y(Alignment::Center),
                    container(text(filename).size(TEXT_MD).style(text::base))
                        .align_y(Alignment::Center),
                ]
                .spacing(SPACE_XS)
                .align_y(Alignment::Center),
                text(meta).size(TEXT_SM).style(theme::TextClass::Tertiary.style()),
            ]
            .spacing(SPACE_XXXS),
        )
        .padding(PAD_NAV_ITEM)
        .style(theme::ContainerClass::Elevated.style())
        .width(Length::Fill);

        col = col.push(card);
    }

    container(col)
        .padding(PAD_CONTENT)
        .width(Length::Fill)
        .into()
}
```

### Async Data Loading

When a message view window opens, the body and attachments must be loaded from the database. This follows the same pattern as the reading pane's thread detail loading.

After inserting the `MessageViewState` and getting the `window_id`, the app dispatches load tasks:

```rust
fn load_message_view_data(
    &self,
    window_id: window::Id,
    state: &MessageViewState,
) -> Task<Message> {
    let db = Arc::clone(&self.db);
    let account_id = state.account_id.clone();
    let message_id = state.message_id.clone();

    let db2 = Arc::clone(&self.db);
    let account_id2 = account_id.clone();
    let message_id2 = message_id.clone();

    Task::batch([
        // Load body
        Task::perform(
            async move {
                db.load_message_body(account_id, message_id).await
            },
            move |result| Message::PopOut(
                window_id,
                PopOutMessage::MessageView(MessageViewMessage::BodyLoaded(result)),
            ),
        ),
        // Load attachments
        Task::perform(
            async move {
                db2.load_message_attachments(account_id2, message_id2).await
            },
            move |result| Message::PopOut(
                window_id,
                PopOutMessage::MessageView(
                    MessageViewMessage::AttachmentsLoaded(result),
                ),
            ),
        ),
    ])
}
```

New `Db` methods needed:

```rust
impl Db {
    /// Load the body (text + HTML) for a single message.
    /// In the full app, this would query the body store (bodies.db).
    /// For the prototype, it uses the snippet as a fallback.
    pub async fn load_message_body(
        &self,
        account_id: String,
        message_id: String,
    ) -> Result<(Option<String>, Option<String>), String> {
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT snippet FROM messages
                 WHERE account_id = ?1 AND id = ?2"
            ).map_err(|e| e.to_string())?;

            let snippet: Option<String> = stmt
                .query_row(params![account_id, message_id], |row| row.get(0))
                .map_err(|e| e.to_string())?;

            // Return snippet as body_text; body_html is None for now.
            // Full implementation queries BodyStoreState for decompressed
            // HTML/text bodies.
            Ok((snippet, None))
        })
        .await
    }

    /// Load attachments for a single message.
    pub async fn load_message_attachments(
        &self,
        account_id: String,
        message_id: String,
    ) -> Result<Vec<MessageViewAttachment>, String> {
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, filename, mime_type, size
                 FROM attachments
                 WHERE account_id = ?1 AND message_id = ?2
                   AND is_inline = 0
                   AND filename IS NOT NULL AND filename != ''
                 ORDER BY filename ASC"
            ).map_err(|e| e.to_string())?;

            stmt.query_map(params![account_id, message_id], |row| {
                Ok(MessageViewAttachment {
                    id: row.get("id")?,
                    filename: row.get("filename")?,
                    mime_type: row.get("mime_type")?,
                    size: row.get("size")?,
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

### Update Routing

Pop-out messages route through the `App::update` match:

```rust
Message::PopOut(window_id, pop_out_msg) => {
    let Some(window) = self.pop_out_windows.get_mut(&window_id) else {
        return Task::none();
    };
    match (window, pop_out_msg) {
        (
            PopOutWindow::MessageView(state),
            PopOutMessage::MessageView(msg),
        ) => self.handle_message_view_update(window_id, state, msg),
        #[allow(unreachable_patterns)]
        _ => Task::none(),
    }
}
```

```rust
fn handle_message_view_update(
    &mut self,
    window_id: window::Id,
    state: &mut MessageViewState,
    msg: MessageViewMessage,
) -> Task<Message> {
    match msg {
        MessageViewMessage::BodyLoaded(Ok((body_text, body_html))) => {
            state.body_text = body_text;
            state.body_html = body_html;
            Task::none()
        }
        MessageViewMessage::BodyLoaded(Err(_)) => Task::none(),
        MessageViewMessage::AttachmentsLoaded(Ok(attachments)) => {
            state.attachments = attachments;
            Task::none()
        }
        MessageViewMessage::AttachmentsLoaded(Err(_)) => Task::none(),
        MessageViewMessage::SetRenderingMode(mode) => {
            let needs_source = mode == RenderingMode::Source
                && state.raw_source.is_none();
            state.rendering_mode = mode;
            if needs_source {
                self.load_raw_source(window_id, state)
            } else {
                Task::none()
            }
        }
        MessageViewMessage::RawSourceLoaded(Ok(source)) => {
            state.raw_source = Some(source);
            Task::none()
        }
        MessageViewMessage::RawSourceLoaded(Err(_)) => {
            state.raw_source = Some("(failed to load source)".to_string());
            Task::none()
        }
        MessageViewMessage::Reply
        | MessageViewMessage::ReplyAll
        | MessageViewMessage::Forward => {
            // Phase 4: dispatch to compose window
            Task::none()
        }
        MessageViewMessage::Archive
        | MessageViewMessage::Delete
        | MessageViewMessage::Print
        | MessageViewMessage::SaveAs
        | MessageViewMessage::ToggleOverflowMenu
        | MessageViewMessage::Noop => Task::none(),
    }
}
```

### Phase 2 Deliverables

1. `MessageViewState` struct and `MessageViewMessage` enum.
2. Pop-out icon button on expanded message cards in the reading pane.
3. `open_message_view_window()` — opens window, inserts state, dispatches data loads.
4. `view_message_window()` — header, body (plain text), attachments.
5. `Db::load_message_body()` and `Db::load_message_attachments()`.
6. Update routing for `PopOutMessage::MessageView`.
7. Layout constants for message view window sizing.

---

## Phase 3: Rendering Modes

### Rendering Mode Toggle

A row of four chip-style buttons below the header, above the body:

```rust
fn rendering_mode_toggle<'a>(
    current: RenderingMode,
    window_id: window::Id,
) -> Element<'a, Message> {
    let modes = [
        (RenderingMode::PlainText, "Plain Text"),
        (RenderingMode::SimpleHtml, "Simple HTML"),
        (RenderingMode::OriginalHtml, "Original HTML"),
        (RenderingMode::Source, "Source"),
    ];

    let mut toggle_row = row![].spacing(SPACE_XS);
    for (mode, label) in modes {
        let is_active = current == mode;
        toggle_row = toggle_row.push(
            button(
                text(label).size(TEXT_SM).style(if is_active {
                    text::primary
                } else {
                    text::secondary
                }),
            )
            .on_press(Message::PopOut(
                window_id,
                PopOutMessage::MessageView(
                    MessageViewMessage::SetRenderingMode(mode),
                ),
            ))
            .padding(PAD_ICON_BTN)
            .style(theme::ButtonClass::Chip { active: is_active }.style()),
        );
    }

    container(toggle_row)
        .padding(Padding {
            top: 0.0,
            right: SPACE_LG,
            bottom: SPACE_SM,
            left: SPACE_LG,
        })
        .width(Length::Fill)
        .into()
}
```

### Default Rendering Mode

The default mode is a system-wide setting, stored in the user's settings. `MessageViewState` initializes `rendering_mode` from this setting rather than hardcoding `SimpleHtml`:

```rust
// In from_thread_message or a constructor that takes settings:
rendering_mode: app_settings.default_rendering_mode,
```

The per-window override is not persisted — it reverts to the system default when the window is closed and reopened.

### Plain Text Mode

Renders `body_text` from the body store. If only HTML is available, strip tags (reuse the `make_collapsed_summary` tag-stripping logic from `thread_detail.rs`, but without truncation). Display in a `text` widget with the standard body font.

### Simple HTML Mode

Renders the sanitized HTML — this is the same pipeline the reading pane uses for message bodies. The HTML sanitizer (`ratatoskr-provider-utils` crate) strips scripts, remote content, heavy styling, and returns safe HTML. The iced rendering converts sanitized HTML to widget trees.

**Implementation note:** The reading pane currently shows snippets, not rendered HTML. Full HTML rendering depends on the HTML-to-iced-widget pipeline (cedilla/frostmark pattern from the ecosystem survey). Until that pipeline exists, Simple HTML falls back to plain text with basic formatting hints. This is acceptable for the initial implementation — the rendering pipeline is a cross-cutting concern that benefits both the reading pane and pop-out views.

### Original HTML Mode

Renders the unsanitized HTML with remote content. Subject to the app's remote-content and tracking-pixel controls (see `docs/roadmap/tracking-blocking.md`).

When the user switches to Original HTML and remote content is blocked globally, a banner appears at the top of the body area:

```rust
fn remote_content_banner<'a>(
    window_id: window::Id,
) -> Element<'a, Message> {
    container(
        row![
            text("Remote content is blocked.")
                .size(TEXT_SM)
                .style(text::secondary),
            button(
                text("Load for this message")
                    .size(TEXT_SM)
                    .style(theme::TextClass::Accent.style()),
            )
            .on_press(Message::PopOut(
                window_id,
                PopOutMessage::MessageView(
                    MessageViewMessage::LoadRemoteContent,
                ),
            ))
            .padding(PAD_ICON_BTN)
            .style(theme::ButtonClass::Ghost.style()),
        ]
        .spacing(SPACE_SM)
        .align_y(Alignment::Center),
    )
    .padding(PAD_CARD)
    .style(theme::ContainerClass::Elevated.style())
    .width(Length::Fill)
    .into()
}
```

Add `LoadRemoteContent` to `MessageViewMessage` and a `remote_content_loaded: bool` field to `MessageViewState`.

### Source Mode

Raw email source (headers + MIME body) in a monospaced font. Loaded lazily — the raw source is not fetched until the user switches to Source mode.

```rust
fn load_raw_source(
    &self,
    window_id: window::Id,
    state: &MessageViewState,
) -> Task<Message> {
    let db = Arc::clone(&self.db);
    let account_id = state.account_id.clone();
    let message_id = state.message_id.clone();

    Task::perform(
        async move {
            db.load_raw_source(account_id, message_id).await
        },
        move |result| Message::PopOut(
            window_id,
            PopOutMessage::MessageView(
                MessageViewMessage::RawSourceLoaded(result),
            ),
        ),
    )
}
```

The raw source may come from the body store or a provider-specific API (e.g., Gmail's `messages.get` with `format=raw`). For the prototype, it can be synthesized from available headers + body. The full implementation queries the raw message if cached locally.

### Monospace Font

Source mode needs a monospace font. Add to `font.rs`:

```rust
/// Monospace font for source view and code blocks.
pub fn monospace() -> iced::Font {
    iced::Font {
        family: iced::font::Family::Monospace,
        ..Default::default()
    }
}
```

### Phase 3 Deliverables

1. Rendering mode toggle (four chip buttons).
2. System-wide default rendering mode setting.
3. Plain text rendering.
4. Simple HTML rendering (placeholder until HTML pipeline exists).
5. Original HTML rendering with remote content banner.
6. Source mode with lazy loading and monospace font.

---

## Phase 4: Action Buttons

### Primary Actions: Reply, Reply All, Forward

Each opens a compose pop-out window pre-filled for the corresponding action. Since compose pop-out is a separate spec, these are initially no-ops that can either:

1. Show a toast/status message ("Compose not yet implemented").
2. Dispatch to the command palette (if the command registry is wired up).

When the compose window spec is implemented, these become:

```rust
MessageViewMessage::Reply => {
    self.open_compose_window(ComposeMode::Reply {
        message_id: state.message_id.clone(),
        account_id: state.account_id.clone(),
    })
}
```

### Overflow Menu

A `[...]` button that opens a floating menu with secondary actions.

```rust
/// State for the overflow menu.
pub overflow_menu_open: bool,  // Add to MessageViewState
```

The overflow menu uses the same `popover` widget pattern as the dropdown in the sidebar:

```rust
fn overflow_menu<'a>(
    open: bool,
    window_id: window::Id,
) -> Element<'a, Message> {
    let trigger = button(
        icon::more_horizontal().size(ICON_MD).style(text::secondary),
    )
    .on_press(Message::PopOut(
        window_id,
        PopOutMessage::MessageView(MessageViewMessage::ToggleOverflowMenu),
    ))
    .padding(PAD_ICON_BTN)
    .style(theme::ButtonClass::BareIcon.style());

    if !open {
        return trigger.into();
    }

    let menu_items = column![
        menu_item(icon::archive(), "Archive", Message::PopOut(
            window_id,
            PopOutMessage::MessageView(MessageViewMessage::Archive),
        )),
        menu_item(icon::trash(), "Delete", Message::PopOut(
            window_id,
            PopOutMessage::MessageView(MessageViewMessage::Delete),
        )),
        menu_item(icon::printer(), "Print", Message::PopOut(
            window_id,
            PopOutMessage::MessageView(MessageViewMessage::Print),
        )),
        menu_item(icon::download(), "Save As", Message::PopOut(
            window_id,
            PopOutMessage::MessageView(MessageViewMessage::SaveAs),
        )),
    ]
    .spacing(SPACE_XXS);

    let menu = container(menu_items)
        .padding(PAD_DROPDOWN)
        .style(theme::ContainerClass::SelectMenu.style());

    crate::ui::popover::popover(trigger)
        .popup(menu)
        .on_dismiss(Message::PopOut(
            window_id,
            PopOutMessage::MessageView(MessageViewMessage::ToggleOverflowMenu),
        ))
        .into()
}

fn menu_item<'a>(
    ico: iced::widget::Text<'a>,
    label: &'a str,
    on_press: Message,
) -> Element<'a, Message> {
    button(
        row![
            container(ico.size(ICON_MD).style(text::secondary))
                .align_y(Alignment::Center),
            container(text(label).size(TEXT_MD).style(text::base))
                .align_y(Alignment::Center),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .on_press(on_press)
    .padding(PAD_NAV_ITEM)
    .height(DROPDOWN_ITEM_HEIGHT)
    .style(theme::ButtonClass::Action.style())
    .width(Length::Fill)
    .into()
}
```

### Archive and Delete

These are thread-level mutations. They use the existing `ProviderOps` trait methods. Since the pop-out shows a single message but archive/delete operate on threads, the action uses the `thread_id` from the message state:

```rust
MessageViewMessage::Archive => {
    // Dispatch archive via the core provider ops.
    // The ProviderOps::archive_threads method takes thread IDs.
    // For the prototype, this is a no-op.
    // Full implementation:
    // self.archive_thread(state.account_id.clone(), state.thread_id.clone())
    Task::none()
}
```

### Print

OS print dialog integration. Platform-specific code with no iced precedent. Deferred to a later phase — for now, a no-op with a status message.

### Phase 4 Deliverables

1. Overflow menu with popover.
2. Reply/Reply All/Forward as command dispatch points (no-op until compose spec).
3. Archive/Delete wired to core provider ops.
4. Print as a stubbed no-op.

---

## Phase 5: Session Restore

### Session State Structure

Extend the existing `window.json` to include pop-out window state:

```rust
/// Full session state, saved on app close.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    /// Main window geometry and layout state.
    pub main_window: WindowState,

    /// Open message view pop-out windows.
    #[serde(default)]
    pub message_views: Vec<MessageViewSessionEntry>,

    // Future:
    // pub compose_windows: Vec<ComposeSessionEntry>,
    // pub calendar_window: Option<CalendarSessionEntry>,
}

/// Minimal data needed to restore a message view window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageViewSessionEntry {
    pub message_id: String,
    pub thread_id: String,
    pub account_id: String,

    // Window geometry
    pub width: f32,
    pub height: f32,
    pub x: Option<f32>,
    pub y: Option<f32>,
}
```

### Save on Close

When the main window closes (which cascades to all pop-outs), the app serializes the current state:

```rust
fn save_session_state(&self) {
    let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");

    let message_views: Vec<MessageViewSessionEntry> = self
        .pop_out_windows
        .values()
        .filter_map(|w| match w {
            PopOutWindow::MessageView(state) => Some(MessageViewSessionEntry {
                message_id: state.message_id.clone(),
                thread_id: state.thread_id.clone(),
                account_id: state.account_id.clone(),
                width: state.width,
                height: state.height,
                x: state.x,
                y: state.y,
            }),
        })
        .collect();

    let session = SessionState {
        main_window: self.window.clone(),
        message_views,
    };

    // Save as session.json alongside window.json
    let path = data_dir.join("session.json");
    if let Ok(json) = serde_json::to_string_pretty(&session) {
        let _ = std::fs::write(path, json);
    }
}
```

### Restore on Launch

In `boot()`, after opening the main window, restore pop-out windows:

```rust
fn boot() -> (Self, Task<Message>) {
    // ... existing boot logic ...

    let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
    let session = SessionState::load(data_dir);

    let (main_window_id, open_main) = iced::window::open(
        session.main_window.to_window_settings(),
    );

    let mut app = Self {
        main_window_id,
        pop_out_windows: HashMap::new(),
        // ...
    };

    let mut tasks = vec![
        open_main.discard(),
        // ... existing account load task ...
    ];

    // Restore message view windows
    for entry in &session.message_views {
        let settings = window::Settings {
            size: Size::new(entry.width, entry.height),
            position: match (entry.x, entry.y) {
                (Some(x), Some(y)) if x >= 0.0 && y >= 0.0 => {
                    window::Position::Specific(Point::new(x, y))
                }
                _ => window::Position::default(),
            },
            min_size: Some(Size::new(
                MESSAGE_VIEW_MIN_WIDTH,
                MESSAGE_VIEW_MIN_HEIGHT,
            )),
            exit_on_close_request: false,
            ..Default::default()
        };

        let (window_id, open_task) = iced::window::open(settings);

        let state = MessageViewState {
            message_id: entry.message_id.clone(),
            thread_id: entry.thread_id.clone(),
            account_id: entry.account_id.clone(),
            // ... other fields initialized to defaults ...
            width: entry.width,
            height: entry.height,
            x: entry.x,
            y: entry.y,
            ..Default::default()
        };

        app.pop_out_windows.insert(window_id, PopOutWindow::MessageView(state));
        tasks.push(open_task.discard());
        // Queue data load for this restored window
        // (done after boot completes, see below)
    }

    (app, Task::batch(tasks))
}
```

After `boot` completes, restored windows need their data loaded. This can be triggered by the `WindowOpened` event or by a post-boot message.

### Best-Effort Restore

Per the problem statement, restoration is best-effort. Specific failure and edge cases:

**Message deleted:** The data load query returns no results. The window shows an error banner: "This message is no longer available." The window remains open (user can close it) but the body area shows the error.

**Body/attachment load failure:** Network or DB error during async load. The window shows the header (from session data) but the body area shows a "Failed to load message body" error with a retry button.

**Rendering mode:** Not persisted — restored windows reset to the system default rendering mode. Per-window rendering mode overrides are transient (problem statement: "not persisted").

**Scroll offset:** Not restored. Restoring exact scroll position after async body load is unreliable (content may have changed, HTML layout differs). Windows open scrolled to top.

**Reloading body and attachments:** Restored windows trigger the same async data loads as newly opened windows. The session entry provides the message_id and account_id needed to query the body store and attachments table. Body/attachment data is never cached in the session file — it is always loaded fresh from the database.

```rust
/// Error state for a message view that failed to restore.
pub error_banner: Option<String>,  // Add to MessageViewState
```

In the body loaded handler:

```rust
MessageViewMessage::BodyLoaded(Err(e)) => {
    state.error_banner = Some(
        "This message is no longer available. It may have been \
         deleted or moved."
            .to_string(),
    );
    Task::none()
}
```

The view renders the banner above the body:

```rust
if let Some(error) = &state.error_banner {
    content = content.push(
        container(
            text(error)
                .size(TEXT_MD)
                .style(theme::TextClass::Tertiary.style()),
        )
        .padding(PAD_CONTENT)
        .width(Length::Fill),
    );
}
```

### Migration: `window.json` to `session.json`

The existing `window.json` stores only main window state. The new `session.json` subsumes it. On first launch after this change:

1. If `session.json` exists, use it.
2. If only `window.json` exists, load it as the `main_window` field with empty `message_views`.
3. If neither exists, use defaults.

Going forward, save only `session.json`. The old `window.json` can be left in place (harmless) or cleaned up.

### Phase 5 Deliverables

1. `SessionState` struct with `MessageViewSessionEntry`.
2. `save_session_state()` on main window close.
3. Restore pop-out windows in `boot()`.
4. Best-effort error banner for deleted messages.
5. Migration from `window.json` to `session.json`.

---

## Phase 6: Save As (.eml, .txt)

**Product-surface deviation:** The problem statement specifies three formats: `.eml`, `.pdf`, and `.txt`. This phase delivers `.eml` and `.txt` only. PDF export is deliberately deferred per the updated problem statement ("PDF export should be treated as a later-phase feature") because it requires a separate rendering pipeline to faithfully paginate HTML to PDF. The `.eml` and `.txt` formats are straightforward serialization and cover the primary use cases (archival and plain-text reference).

### File Picker

Use `rfd` (Rust File Dialogs) crate for the native file picker. This is already a common pattern in the Rust desktop ecosystem.

Add to `crates/app/Cargo.toml`:

```toml
rfd = "0.15"
```

### Save As Flow

1. User clicks "Save As" in the overflow menu.
2. A file picker dialog opens with format filter options.
3. The user chooses a location and format.
4. The file is written.

Since `rfd` dialogs are async and blocking, they must run on a background task:

```rust
MessageViewMessage::SaveAs => {
    let message_id = state.message_id.clone();
    let account_id = state.account_id.clone();
    let subject = state.subject.clone()
        .unwrap_or_else(|| "message".to_string());
    let db = Arc::clone(&self.db);

    Task::perform(
        async move {
            save_message_dialog(db, account_id, message_id, subject).await
        },
        move |result| match result {
            Ok(()) => Message::PopOut(
                window_id,
                PopOutMessage::MessageView(MessageViewMessage::Noop),
            ),
            Err(_) => Message::PopOut(
                window_id,
                PopOutMessage::MessageView(MessageViewMessage::Noop),
            ),
        },
    )
}
```

### .eml Export

RFC 5322 format — the full message with headers and MIME body. This is the raw message source, which may already be available from the body store or raw message cache.

```rust
async fn save_as_eml(
    db: Arc<Db>,
    account_id: String,
    message_id: String,
    path: std::path::PathBuf,
) -> Result<(), String> {
    let raw = db.load_raw_source(account_id, message_id).await?;
    tokio::fs::write(&path, raw.as_bytes())
        .await
        .map_err(|e| format!("write failed: {e}"))
}
```

### .txt Export

Plain text body only. Includes a minimal header block for context:

```rust
async fn save_as_txt(
    db: Arc<Db>,
    account_id: String,
    message_id: String,
    path: std::path::PathBuf,
) -> Result<(), String> {
    let (body_text, _body_html) = db
        .load_message_body(account_id.clone(), message_id.clone())
        .await?;

    // Build a minimal text representation
    let mut output = String::new();

    // Header block
    // (Would need to query header fields — for now, use what's available)
    if let Some(text) = body_text {
        output.push_str(&text);
    }

    tokio::fs::write(&path, output.as_bytes())
        .await
        .map_err(|e| format!("write failed: {e}"))
}
```

The full implementation includes a header block (From, To, Cc, Date, Subject) above the body text, separated by a blank line.

### .pdf Export

**Deferred** per the problem statement. PDF export requires rendering the message HTML faithfully to a paginated PDF, which is a separate rendering pipeline from the screen display. Not part of this spec.

### File Picker Dialog

```rust
async fn save_message_dialog(
    db: Arc<Db>,
    account_id: String,
    message_id: String,
    subject: String,
) -> Result<(), String> {
    // Sanitize subject for use as filename
    let safe_name: String = subject
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' })
        .collect();
    let safe_name = safe_name.trim().to_string();

    let file_handle = rfd::AsyncFileDialog::new()
        .set_title("Save Message")
        .set_file_name(format!("{safe_name}.eml"))
        .add_filter("Email Message (.eml)", &["eml"])
        .add_filter("Plain Text (.txt)", &["txt"])
        .save_file()
        .await;

    let Some(handle) = file_handle else {
        return Ok(()); // User cancelled
    };

    let path = handle.path().to_path_buf();
    let extension = path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("eml");

    match extension {
        "txt" => save_as_txt(db, account_id, message_id, path).await,
        _ => save_as_eml(db, account_id, message_id, path).await,
    }
}
```

### Phase 6 Deliverables

1. Add `rfd` dependency.
2. Save As dialog with .eml and .txt format filters.
3. `.eml` export (raw source).
4. `.txt` export (header block + plain text body).

---

## File Organization

### New Files

| File | Contents |
|------|----------|
| `crates/app/src/pop_out.rs` | `PopOutWindow` enum, `PopOutMessage` enum, shared pop-out infrastructure |
| `crates/app/src/pop_out/message_view.rs` | `MessageViewState`, `MessageViewMessage`, `RenderingMode`, view functions |
| `crates/app/src/pop_out/session.rs` | `SessionState`, `MessageViewSessionEntry`, save/restore logic |

### Modified Files

| File | Changes |
|------|---------|
| `crates/app/src/main.rs` | Switch to `daemon`, add `main_window_id`, `pop_out_windows`, route `view`/`title` by window ID, update `Message` enum |
| `crates/app/src/ui/layout.rs` | Add message view window sizing constants |
| `crates/app/src/ui/widgets.rs` | Add pop-out icon button to `expanded_message_card` |
| `crates/app/src/ui/reading_pane.rs` | Add `OpenMessagePopOut` event |
| `crates/app/src/window_state.rs` | Absorbed into `SessionState` or kept as-is with session layered on top |
| `crates/app/src/db.rs` | Add `load_message_body()`, `load_message_attachments()`, `load_raw_source()` |
| `crates/app/src/font.rs` | Add `monospace()` function |
| `crates/app/Cargo.toml` | Add `rfd` dependency (Phase 6) |

### Module Registration

In `crates/app/src/main.rs`:

```rust
mod pop_out;
```

The `pop_out` module is a directory:

```
crates/app/src/pop_out/
    mod.rs           // PopOutWindow, PopOutMessage
    message_view.rs  // MessageViewState, views, messages
    session.rs       // SessionState, save/restore
```

---

## Dependency Graph

```
Phase 1: Multi-Window Architecture (daemon migration)
    |
    v
Phase 2: Message View Window (basic content)
    |
    +---> Phase 3: Rendering Modes (toggleable body rendering)
    |
    +---> Phase 4: Action Buttons (reply dispatch, overflow menu)
    |
    +---> Phase 5: Session Restore (save/restore pop-out state)
    |
    +---> Phase 6: Save As (.eml, .txt via rfd)
```

Phases 3-6 are independent of each other and can be implemented in any order after Phase 2. Phase 1 is the foundation — it must be done first and is the riskiest (no precedent in the iced ecosystem for multi-window).

---

## Risk Assessment

### Phase 1 Risks

**`iced::daemon` behavioral differences.** The daemon does not exit when all windows close — the app must explicitly call `iced::exit()`. If this is missed, the process lingers. Mitigation: the main window close handler always calls `iced::exit()`.

**Window ID stability.** `window::Id::unique()` generates IDs that are only valid for the current session. They cannot be persisted across sessions. Session restore creates new IDs. This is fine — the session entries identify windows by message ID, not window ID.

**Scale factor per window.** The `scale_factor` callback currently returns a single value. With `daemon`, it takes `(&App, window::Id)` — all windows share the same scale factor, which is the correct behavior (system DPI scaling applies uniformly).

**Theme per window.** The `theme` callback takes `(&App, window::Id)`. All windows should use the same theme. No per-window theme overrides.

### Phase 2 Risks

**Double-click detection.** iced does not natively expose double-click events. The pop-out icon button is the primary affordance; double-click is a stretch goal that may require a custom widget or mouse_area extension.

### Phase 3 Risks

**HTML rendering pipeline.** Full HTML-to-widget rendering is a cross-cutting concern shared with the reading pane. Until it exists, Simple HTML and Original HTML modes fall back to plain text. This is acceptable — the rendering pipeline is not gated by this spec.

### Phase 5 Risks

**Stale session data.** Messages may be deleted between sessions. Best-effort restore with error banners handles this gracefully per the problem statement.

---

## Open Questions

1. **Command palette in pop-out windows.** Should the command palette work within pop-out windows? If yes, the palette subscription and overlay must be per-window. If no, only the main window gets the palette. The command palette spec should decide this — for now, pop-out windows do not include the palette.

2. **Focus tracking across windows.** The `FocusedRegion` enum (from the command palette context) does not currently have a `PopOutMessageView` variant. If commands need to know which window type is focused, this enum needs extending.

3. **Inline composer prohibition.** The problem statement says "No inline composer in pop-out message windows." Reply/Reply All/Forward always open a new compose pop-out, never an inline reply area. This is already the design in this spec.
