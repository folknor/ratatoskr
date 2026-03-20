# Contacts: Autocomplete & Token Input — Implementation Spec

## Scope

This spec covers the compose-critical half of the contacts feature:

1. **Token input widget** — custom `advanced::Widget` for chip/tag input with inline tokens
2. **Autocomplete dropdown** — unified local search across contacts, seen addresses, and cached GAL
3. **Paste handling** — tokenize pasted addresses with RFC 5322 parsing
4. **Token drag and drop** — move tokens between To/Cc/Bcc fields
5. **Contact group tokens** — atomic group tokens with expand-via-context-menu
6. **Data layer** — unified search function, deduplication, result types

Explicitly **excluded**: contact management UI, import wizard, inline contact editing popover, GAL sync pipeline (backend exists). Those are covered in separate specs.

## Integration Points

The token input widget is used in three contexts:

- **Compose recipient fields** — To, Cc, Bcc in the compose window (pop-out or inline)
- **Calendar event attendee field** — identical behavior to compose recipients
- **Contact group editor** — groups-only matching variant (no individual contacts in suggestions)

All three share the same widget. The caller configures which search pools to include.

---

## Phase 1: Token Input Widget (Custom `advanced::Widget`)

This is the largest custom widget effort for contacts. iced has no chip/tag input widget. The token input must be built as a custom `advanced::Widget` implementation.

### File Location

`crates/app/src/ui/token_input.rs` — standalone module, imported by compose and calendar views.

### Data Types

```rust
// crates/app/src/ui/token_input.rs

use iced::advanced::{self, layout, mouse, overlay, renderer, widget, Clipboard, Layout, Shell, Widget};
use iced::{Color, Element, Event, Length, Padding, Point, Rectangle, Renderer, Size, Theme, Vector};

/// A single token displayed inline in the input field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// Unique ID for this token instance (index-stable across reorders).
    pub id: TokenId,
    /// The email address this token represents.
    pub email: String,
    /// Display label shown on the token chip. Resolved from display name
    /// if available, otherwise the email address.
    pub label: String,
    /// Whether this token represents a contact group.
    pub is_group: bool,
    /// Group ID if this is a group token (for expand operations).
    pub group_id: Option<String>,
}

/// Opaque token identifier. Wraps a u64 counter, monotonically increasing
/// per widget instance to guarantee uniqueness across add/remove cycles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TokenId(pub u64);

/// Which recipient field this widget instance represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecipientField {
    To,
    Cc,
    Bcc,
}

/// Internal selection state for keyboard/mouse navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenSelection {
    /// No token selected; cursor is in the text input area.
    None,
    /// A specific token is selected (highlighted for deletion/action).
    Selected(TokenId),
}
```

### State

The widget maintains both persistent state (owned by the parent via the message/model pattern) and transient layout state (computed during `layout()` and cached).

```rust
/// Persistent state owned by the caller (lives in the compose model).
/// Passed as data to the widget constructor — the widget does not own this.
pub struct TokenInputValue {
    /// Current tokens in this field.
    pub tokens: Vec<Token>,
    /// Current text being typed (after the last token).
    pub text: String,
    /// Next token ID counter.
    pub next_id: u64,
}

impl TokenInputValue {
    pub fn new() -> Self {
        Self {
            tokens: Vec::new(),
            text: String::new(),
            next_id: 0,
        }
    }

    pub fn next_token_id(&mut self) -> TokenId {
        let id = TokenId(self.next_id);
        self.next_id += 1;
        id
    }
}

/// Transient widget state for layout caching and interaction tracking.
/// Created via `widget::Id` and stored in iced's widget state tree.
#[derive(Debug, Default)]
pub struct TokenInputState {
    /// Cached per-token bounds from the last layout pass (relative to widget origin).
    token_bounds: Vec<Rectangle>,
    /// Bounds of the text input area (after all tokens).
    text_bounds: Rectangle,
    /// Which token is currently selected (keyboard or click).
    selection: TokenSelection,
    /// Whether the text input is focused.
    is_focused: bool,
    /// Cursor position within the text (character offset).
    cursor_position: usize,
    /// Drag state for token reordering / cross-field moves.
    drag: Option<DragState>,
}

#[derive(Debug, Clone)]
struct DragState {
    token_id: TokenId,
    origin: Point,
    current: Point,
}
```

### Messages

```rust
/// Messages emitted by the token input widget upward to the caller.
#[derive(Debug, Clone)]
pub enum TokenInputMessage {
    /// The text input content changed.
    TextChanged(String),
    /// A token should be added from raw text input.
    /// The caller validates and creates the Token, then updates the model.
    TokenizeText(String),
    /// A token was removed by backspace or delete key.
    RemoveToken(TokenId),
    /// A token was clicked (selected).
    SelectToken(TokenId),
    /// Click in empty area / text area — deselect any token.
    DeselectTokens,
    /// Focus was gained by this field.
    Focused,
    /// Focus was lost.
    Blurred,
    /// A paste event with raw text. Caller parses and tokenizes.
    Paste(String),
    /// Right-click on a token — emit position for context menu.
    TokenContextMenu(TokenId, Point),
    /// Drag started on a token.
    DragStarted(TokenId),
    /// Token dropped on this field (from another field via drag).
    TokenDropped {
        token: Token,
        source_field: RecipientField,
    },
    /// Keyboard event requesting to move a token to another field.
    MoveToken {
        token_id: TokenId,
        target_field: RecipientField,
    },
}
```

### Widget Constructor

```rust
/// Creates a token input widget. Pure function — all state is external.
///
/// # Arguments
/// * `field` — which recipient field this instance represents
/// * `tokens` — current tokens (from the model)
/// * `text` — current input text (from the model)
/// * `placeholder` — placeholder text when empty (e.g., "Add recipients...")
/// * `on_message` — callback converting TokenInputMessage to the caller's message type
pub fn token_input<'a, M: Clone + 'a>(
    field: RecipientField,
    tokens: &'a [Token],
    text: &'a str,
    placeholder: &'a str,
    on_message: impl Fn(TokenInputMessage) -> M + 'a,
) -> Element<'a, M> {
    TokenInput {
        field,
        tokens,
        text,
        placeholder,
        on_message: Box::new(on_message),
    }
    .into()
}
```

### Layout Algorithm

The token input uses a **wrapping flow layout**. Tokens and the text input are laid out left-to-right, wrapping to new rows when the available width is exceeded.

```
┌─────────────────────────────────────────────────────┐
│ [Bob Jones] [alice@corp.com] [Engineering ▪3]       │
│ [frank@other.com] [type here...                   ] │
└─────────────────────────────────────────────────────┘
```

The `layout()` implementation:

1. Measure each token's width: `label_text_width + PAD_TOKEN.left + PAD_TOKEN.right`
2. Walk tokens left-to-right. If adding a token would exceed `limits.max().width - field_padding`, wrap to a new row.
3. After all tokens, place the text input. It fills the remaining width on the current row. If less than `MIN_TEXT_INPUT_WIDTH` remains, wrap to a new row where it takes full width.
4. Total height = `row_count * TOKEN_ROW_HEIGHT + (row_count - 1) * TOKEN_ROW_SPACING + field_padding_vertical`

Layout constants (added to `layout.rs`):

```rust
/// Token chip height
pub const TOKEN_HEIGHT: f32 = 24.0;
/// Token chip border radius
pub const TOKEN_RADIUS: f32 = RADIUS_SM; // 4px
/// Token chip internal padding
pub const PAD_TOKEN: Padding = Padding {
    top: 2.0,
    right: 8.0,
    bottom: 2.0,
    left: 8.0,
};
/// Spacing between tokens (horizontal)
pub const TOKEN_SPACING: f32 = SPACE_XXS; // 4px
/// Spacing between token rows (vertical)
pub const TOKEN_ROW_SPACING: f32 = SPACE_XXS; // 4px
/// Token input field internal padding
pub const PAD_TOKEN_INPUT: Padding = Padding {
    top: 4.0,
    right: 8.0,
    bottom: 4.0,
    left: 8.0,
};
/// Minimum width for the text input portion before wrapping
pub const TOKEN_TEXT_MIN_WIDTH: f32 = 120.0;
/// Group icon size on group tokens
pub const TOKEN_GROUP_ICON_SIZE: f32 = ICON_XS; // 10px
```

### `Widget` Trait Implementation Outline

```rust
impl<'a, M: Clone> Widget<M, Theme, Renderer> for TokenInput<'a, M> {
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<TokenInputState>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(TokenInputState::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(
        &self,
        tree: &mut widget::Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        // 1. Measure each token label width using renderer.measure()
        // 2. Flow layout: walk tokens, track (x, y, row_height)
        // 3. Store per-token bounds in state.token_bounds
        // 4. Place text input in remaining space
        // 5. Return Node with total size
        todo!()
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<TokenInputState>();
        let bounds = layout.bounds();

        // 1. Draw field background (border, rounded corners)
        //    - Use theme palette: base background, weak border
        //    - Focused state: primary border color
        // 2. Draw each token chip:
        //    - Background: palette weak/weaker
        //    - Selected: primary background with on-primary text
        //    - Group tokens: prepend group icon (people icon)
        //    - Text: TEXT_MD size, primary text color
        //    - Rounded rectangle with TOKEN_RADIUS
        // 3. Draw text input:
        //    - Cursor blink (if focused)
        //    - Placeholder text (if empty and no tokens)
        //    - Current text value
    }

    fn on_event(
        &mut self,
        tree: &mut widget::Tree,
        event: Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) -> iced::event::Status {
        let state = tree.state.downcast_mut::<TokenInputState>();

        match event {
            // --- Keyboard events (when focused) ---
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                match key {
                    // Backspace at position 0 with tokens: select last token
                    // Backspace with token selected: remove it
                    Key::Named(Named::Backspace) => { /* ... */ }

                    // Left/Right arrow: navigate between tokens and text
                    Key::Named(Named::ArrowLeft) => { /* ... */ }
                    Key::Named(Named::ArrowRight) => { /* ... */ }

                    // Enter/Tab/Space/Comma/Semicolon: tokenize current text
                    // (only if no autocomplete suggestion is being accepted —
                    //  the parent handles autocomplete selection and suppresses
                    //  these keys when the dropdown has a highlighted item)
                    Key::Named(Named::Enter | Named::Tab) |
                    Key::Character(ref c) if c == " " || c == "," || c == ";" => {
                        // Emit TokenizeText with current text
                    }

                    // Escape: blur the field
                    Key::Named(Named::Escape) => { /* ... */ }

                    // Ctrl+A: select all tokens (future enhancement)
                    // Ctrl+V: handled via paste event below
                    _ => {}
                }
            }

            // --- Text input events ---
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: Key::Character(ref c), ..
            }) if !modifiers.command() => {
                // Append character to text, emit TextChanged
            }

            // --- Paste ---
            // Detected via Ctrl+V / Cmd+V, read from clipboard
            // Emit Paste(clipboard_text) for the parent to parse

            // --- Mouse events ---
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                // Hit-test against token_bounds -> SelectToken
                // Hit-test text area -> DeselectTokens + Focused
                // Outside all -> blur
            }

            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                // Hit-test against token_bounds -> TokenContextMenu
            }

            // --- Drag initiation ---
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if cursor_over_token => {
                // Start drag tracking
            }

            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                // Update drag position if dragging
                // If moved beyond threshold, emit DragStarted
            }

            _ => {}
        }

        iced::event::Status::Ignored
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<TokenInputState>();
        if let Some(position) = cursor.position_in(layout.bounds()) {
            // Over a token: Grab cursor (draggable)
            // Over text area: Text cursor
            // Over field but not token/text: default
            mouse::Interaction::Text
        } else {
            mouse::Interaction::default()
        }
    }
}
```

### Token Chip Drawing

Each token chip is drawn directly via the renderer (not composed from iced widgets, since `advanced::Widget::draw` works at the renderer level):

1. **Background quad** — rounded rectangle (`TOKEN_RADIUS`), fill color from theme palette:
   - Normal: `palette.background.weak` (one step lighter than field bg)
   - Selected: `palette.primary.base`
   - Hovered: `palette.background.weaker` (one step hover rule)
2. **Label text** — `TEXT_MD` size, `font::text()`, clipped to chip bounds
   - Normal: `palette.text` color
   - Selected: `palette.primary.strong` (on-primary text)
3. **Group indicator** — for group tokens, a small people icon (`TOKEN_GROUP_ICON_SIZE`) before the label text, plus member count suffix (e.g., "Engineering (12)")

### Keyboard Interaction State Machine

```
                    ┌──────────────┐
                    │  No tokens,  │
              ┌────►│  text empty  │◄───── Blur
              │     └──────┬───────┘
              │            │ Type character
              │            ▼
              │     ┌──────────────┐
              │     │ Text has     │──── Tokenizer key ──► Add token, clear text
              │     │ content      │
              │     └──────┬───────┘
              │            │ Backspace (text empty, tokens exist)
              │            ▼
              │     ┌──────────────┐
  Backspace   │     │ Last token   │──── Backspace again ──► Remove token
  (no tokens) │     │ selected     │
              │     └──────┬───────┘
              │            │ Type character
              │            ▼
              │     ┌──────────────┐
              └─────│ Text has     │
                    │ content      │
                    └──────────────┘
```

Arrow key navigation:

- **Left arrow** at text position 0: select the last token. Continue pressing left to select earlier tokens.
- **Right arrow** with a token selected: move selection to the next token, or if at the last token, deselect and focus text input.
- Any character key while a token is selected: deselect the token, focus text input, and type the character.

### Text Measurement

Token label widths are measured using `iced::advanced::text::Renderer::measure()`:

```rust
fn measure_token_width(renderer: &Renderer, label: &str) -> f32 {
    let size = iced::Pixels(TEXT_MD);
    let font = font::text();
    // Measure the label text
    let text_size = renderer.measure(
        label,
        size,
        iced::advanced::text::LineHeight::default(),
        font,
        Size::INFINITY,
        iced::advanced::text::Shaping::Advanced,
    );
    text_size.width + PAD_TOKEN.left + PAD_TOKEN.right
}
```

---

## Phase 2: Autocomplete Dropdown

The autocomplete dropdown is **not part of the token input widget**. It is a separate overlay managed by the parent component (compose view). This separation keeps the token input widget focused on input/display and avoids embedding search logic in a widget.

### Architecture

```
ComposeView (Component)
├── token_input(To, ...)    ← widget, emits TokenInputMessage
├── token_input(Cc, ...)
├── token_input(Bcc, ...)
└── autocomplete_dropdown   ← overlay, positioned below the focused field
```

The compose view:

1. Receives `TokenInputMessage::TextChanged(query)` from the focused field
2. Runs the unified contact search (debounced, local-only)
3. Stores results in `autocomplete_results: Vec<ContactSearchResult>`
4. Renders the dropdown as a popover overlay below the focused token input
5. Handles dropdown selection by creating a `Token` and adding it to the model

### Contact Search Result Type

```rust
// crates/core/src/contacts/search.rs (new module in core)

/// A single result from unified contact search.
/// Blends synced contacts, seen addresses, and cached GAL entries.
#[derive(Debug, Clone)]
pub struct ContactSearchResult {
    /// The email address (primary key for dedup).
    pub email: String,
    /// Display name, resolved from highest-priority source.
    pub display_name: Option<String>,
    /// Avatar URL if available (synced contacts only).
    pub avatar_url: Option<String>,
    /// Relevance score for ranking (higher = more relevant).
    pub score: f64,
    /// The kind of result (person vs group).
    pub kind: ContactSearchKind,
}

#[derive(Debug, Clone)]
pub enum ContactSearchKind {
    /// An individual contact (synced, seen, or GAL).
    Person,
    /// A contact group. `group_id` is used for token creation and expansion.
    Group {
        group_id: String,
        member_count: i64,
    },
}
```

### Unified Search Function

```rust
// crates/core/src/contacts/search.rs

use crate::db::{DbState, types::DbContact};
use crate::db::queries::search_contacts;
use crate::db::queries_extra::contact_groups::db_search_contact_groups;
use std::collections::HashMap;

/// Search mode controls which pools are included.
#[derive(Debug, Clone, Copy)]
pub enum ContactSearchMode {
    /// All pools: synced contacts, seen addresses, GAL cache, groups.
    /// Used in compose To/Cc/Bcc and calendar attendee fields.
    All,
    /// Groups only. Used in the contact group editor.
    GroupsOnly,
}

/// Unified contact search across all local pools.
///
/// Returns results ranked by recency/relevance, deduplicated by email.
/// Group results are interleaved with person results but ranked primarily
/// on group name match (groups matching on member names are suppressed).
pub fn search_contacts_unified(
    conn: &rusqlite::Connection,
    query: &str,
    mode: ContactSearchMode,
    limit: i64,
) -> Result<Vec<ContactSearchResult>, String> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();
    let mut seen_emails: HashMap<String, usize> = HashMap::new();

    match mode {
        ContactSearchMode::All => {
            // 1. Search synced contacts + seen addresses via existing FTS
            //    (search_contacts already does FTS with LIKE fallback and
            //    deduplicates contacts vs seen_addresses)
            let contacts = search_contacts(conn, query.to_string(), limit)?;
            for c in contacts {
                let idx = results.len();
                seen_emails.insert(c.email.to_lowercase(), idx);
                results.push(ContactSearchResult {
                    email: c.email,
                    display_name: c.display_name,
                    avatar_url: c.avatar_url,
                    score: c.frequency as f64,
                    kind: ContactSearchKind::Person,
                });
            }

            // 2. Search GAL cache (same table structure, separate source flag)
            //    GAL entries already participate in the contacts_fts index.
            //    Dedup: skip any GAL email already in results.

            // 3. Search contact groups by name
            let groups = db_search_contact_groups_sync(conn, query, limit)?;
            for g in groups {
                results.push(ContactSearchResult {
                    email: String::new(), // Groups don't have a single email
                    display_name: Some(g.name.clone()),
                    avatar_url: None,
                    score: 0.0, // Groups ranked by name match quality
                    kind: ContactSearchKind::Group {
                        group_id: g.id,
                        member_count: g.member_count,
                    },
                });
            }
        }
        ContactSearchMode::GroupsOnly => {
            let groups = db_search_contact_groups_sync(conn, query, limit)?;
            for g in groups {
                results.push(ContactSearchResult {
                    email: String::new(),
                    display_name: Some(g.name.clone()),
                    avatar_url: None,
                    score: 0.0,
                    kind: ContactSearchKind::Group {
                        group_id: g.id,
                        member_count: g.member_count,
                    },
                });
            }
        }
    }

    // Truncate to limit
    results.truncate(limit as usize);
    Ok(results)
}
```

**Note on synchronous vs async:** The existing `search_contacts` function in `queries.rs` is synchronous (takes `&Connection`). The unified search function is also synchronous, called from `Task::perform(spawn_blocking(...))` in the app layer. This matches the established pattern — DB access through `DbState::with_conn()`.

### Dropdown Rendering

The dropdown is rendered as a popover overlay, using the existing `popover` module.

```rust
// In the compose view's view() method:

fn view_autocomplete_dropdown<'a>(&self) -> Option<Element<'a, ComposeMessage>> {
    if self.autocomplete_results.is_empty() || self.autocomplete_query.is_empty() {
        return None;
    }

    let items: Vec<Element<'_, ComposeMessage>> = self.autocomplete_results
        .iter()
        .enumerate()
        .map(|(idx, result)| {
            let is_highlighted = self.autocomplete_highlighted == Some(idx);
            autocomplete_row(result, is_highlighted, idx)
        })
        .collect();

    let menu = container(
        column(items).spacing(SPACE_0).width(Length::Fill),
    )
    .padding(PAD_DROPDOWN)
    .style(theme::ContainerClass::Floating.style())
    .max_height(AUTOCOMPLETE_MAX_HEIGHT);

    Some(menu.into())
}
```

Each suggestion row layout:

```
┌──────────────────────────────────────────────────┐
│ Alice Smith                  alice.smith@corp.com │  <- Person
│ [👥] Engineering (12)                            │  <- Group
│ bob@example.com                                  │  <- No display name
└──────────────────────────────────────────────────┘
```

Row structure for a person result:

```rust
fn autocomplete_row<'a, M: Clone + 'a>(
    result: &'a ContactSearchResult,
    highlighted: bool,
    index: usize,
) -> Element<'a, M> {
    match &result.kind {
        ContactSearchKind::Person => {
            let name_text = result.display_name.as_deref().unwrap_or(&result.email);
            let email_text = if result.display_name.is_some() {
                &result.email
            } else {
                "" // Don't duplicate email if it's already the name
            };

            // row: [name (Fill, left)] [email (Shrink, right, muted)]
        }
        ContactSearchKind::Group { member_count, .. } => {
            let name = result.display_name.as_deref().unwrap_or("(unnamed group)");
            // row: [group_icon] [name (Fill)] [member_count (Shrink, muted)]
        }
    }
}
```

Layout constants (added to `layout.rs`):

```rust
/// Maximum height of the autocomplete dropdown
pub const AUTOCOMPLETE_MAX_HEIGHT: f32 = 300.0;
/// Height of each autocomplete suggestion row
pub const AUTOCOMPLETE_ROW_HEIGHT: f32 = 32.0;
```

### Dropdown Lifecycle

The dropdown's visibility is controlled by three conditions, all of which must be true:

1. A token input field is focused
2. The text input has content (`!query.is_empty()`)
3. There are matching results (`!autocomplete_results.is_empty()`)

When any condition becomes false, the dropdown disappears. When all three become true again (e.g., user starts typing a new token), it reappears.

State tracking in the compose model:

```rust
/// Autocomplete state in the compose model.
struct AutocompleteState {
    /// Which field is currently showing autocomplete.
    active_field: Option<RecipientField>,
    /// Current search query (mirrors the focused field's text).
    query: String,
    /// Search results from the last query.
    results: Vec<ContactSearchResult>,
    /// Index of the highlighted result (keyboard navigation).
    highlighted: Option<usize>,
    /// Generation counter to discard stale search results.
    search_generation: u64,
}
```

### Keyboard Navigation in Dropdown

When the dropdown is visible, arrow keys and selection keys are intercepted at the compose view level **before** they reach the token input widget:

- **Down arrow / Up arrow** — move `highlighted` index through results
- **Enter / Tab** — accept the highlighted result (or top result if none highlighted), create a token, clear text, dismiss dropdown
- **Escape** — dismiss dropdown, keep text

When the dropdown is **not visible** (no matches), these keys pass through to the token input:

- **Enter / Tab / Space / Comma / Semicolon** — tokenize raw text directly

This two-layer dispatch is critical. The compose view's `update()` checks `autocomplete_state.results.is_empty()` to decide whether to handle the key or forward it.

### Debounce

For local SQLite search, debounce is minimal — 10-20ms, just enough to coalesce rapid keystrokes. Implemented as a `Task::perform` with the generation counter pattern:

```rust
// In compose update():
ComposeMessage::TokenInput(field, TokenInputMessage::TextChanged(text)) => {
    self.autocomplete.query = text.clone();
    self.autocomplete.active_field = Some(field);
    self.autocomplete.search_generation += 1;
    let gen = self.autocomplete.search_generation;
    let db = Arc::clone(&self.db);
    let query = text;

    Task::perform(
        async move {
            // Minimal debounce
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
            db.with_conn(move |conn| {
                search_contacts_unified(conn, &query, ContactSearchMode::All, 10)
            }).await
        },
        move |results| ComposeMessage::AutocompleteResults(gen, results),
    )
}

ComposeMessage::AutocompleteResults(gen, results) => {
    // Discard stale results
    if gen != self.autocomplete.search_generation {
        return Task::none();
    }
    match results {
        Ok(results) => {
            self.autocomplete.results = results;
            self.autocomplete.highlighted = if self.autocomplete.results.is_empty() {
                None
            } else {
                Some(0) // Auto-highlight first result
            };
        }
        Err(_) => {
            self.autocomplete.results.clear();
            self.autocomplete.highlighted = None;
        }
    }
    Task::none()
}
```

### Tokenizer Keys Behavior Summary

| Key | Dropdown visible | Dropdown hidden |
|-----|-----------------|-----------------|
| Enter | Accept highlighted suggestion | Tokenize raw text |
| Tab | Accept highlighted (or top) suggestion | Tokenize raw text |
| Space | Type space (continue searching) | Tokenize raw text |
| Comma | Tokenize raw text, dismiss dropdown | Tokenize raw text |
| Semicolon | Tokenize raw text, dismiss dropdown | Tokenize raw text |
| Down arrow | Navigate dropdown | No-op |
| Up arrow | Navigate dropdown | No-op |
| Escape | Dismiss dropdown, keep text | Blur field |

**Comma and semicolon** always tokenize immediately (even with dropdown open) because they unambiguously signal "I'm done with this address." Space tokenizes only when the dropdown is hidden, because users might type "John Smith" as a search query.

---

## Phase 3: Paste Handling

### RFC 5322 Address Parsing

Pasted text is parsed into individual addresses. The parser handles:

1. **Bare email**: `alice@corp.com`
2. **Name + angle-bracket**: `Alice Smith <alice@corp.com>`
3. **Quoted name + angle-bracket**: `"Alice Smith" <alice@corp.com>`
4. **Multiple addresses**: separated by commas, semicolons, or newlines
5. **Mixed formats**: `Alice <alice@corp.com>, bob@example.com, "Charlie D" <charlie@d.com>`

```rust
// crates/app/src/ui/token_input_parse.rs

/// Parse pasted text into (display_name, email) pairs.
/// Returns all valid addresses found. Invalid fragments are silently dropped.
pub fn parse_pasted_addresses(input: &str) -> Vec<ParsedAddress> {
    // 1. Split on comma, semicolon, or newline
    // 2. For each fragment, try to parse as RFC 5322 mailbox
    // 3. Extract display name and email
    // 4. Basic email validation (contains @, non-empty local and domain parts)
    todo!()
}

#[derive(Debug, Clone)]
pub struct ParsedAddress {
    pub email: String,
    pub display_name: Option<String>,
}
```

The parsing function is **not** a full RFC 5322 parser. It handles the common formats that users encounter when copying from spreadsheets, other email clients, and contact lists. Edge cases (comments in addresses, group syntax, encoded words) are not worth the complexity.

### Paste Flow

1. User pastes text (Ctrl+V / Cmd+V)
2. Token input widget emits `TokenInputMessage::Paste(clipboard_text)`
3. Compose view's `update()` calls `parse_pasted_addresses()`
4. For each parsed address: create a `Token` with the extracted display name and email
5. Add all tokens to the model, clear text input
6. **No autocomplete dropdown** — paste bypasses search entirely

### Bcc Suggestion Banner

When a paste tokenizes 10+ addresses, the compose view shows a dismissible banner:

```
┌─────────────────────────────────────────────────────────┐
│ ℹ 14 addresses pasted. Save as a contact group?  [Save] [Dismiss] │
└─────────────────────────────────────────────────────────┘
```

State in compose model:

```rust
struct BulkPasteBanner {
    /// Whether the banner is visible.
    visible: bool,
    /// The pasted addresses (for group creation).
    addresses: Vec<ParsedAddress>,
}
```

Clicking "Save" opens the group creation flow (pre-populated). This is a future integration point with the contact management spec — for now, the banner is rendered but the "Save" action can emit a placeholder event.

---

## Phase 4: Token Context Menu

### Trigger

Right-click on a token emits `TokenInputMessage::TokenContextMenu(token_id, position)`. The compose view opens a context menu overlay at that position.

### Menu Items

For a **person token** in compose:

```
┌─────────────────────┐
│ Cut           Ctrl+X │
│ Copy          Ctrl+C │
│ Paste         Ctrl+V │
│ ──────────────────── │
│ Delete               │
│ ──────────────────── │
│ Move to Cc           │  <- Only if token is in To or Bcc
│ Move to Bcc          │  <- Only if token is in To or Cc
│ Move to To           │  <- Only if token is in Cc or Bcc
└─────────────────────┘
```

For a **group token** in compose:

```
┌─────────────────────┐
│ Cut           Ctrl+X │
│ Copy          Ctrl+C │
│ Paste         Ctrl+V │
│ ──────────────────── │
│ Expand group         │  <- Replaces group with individual tokens
│ Delete               │
│ ──────────────────── │
│ Move to Cc           │
│ Move to Bcc          │
└─────────────────────┘
```

For tokens in **calendar attendee** fields: no "Move to" options, no "Expand group."

### Context Menu Messages

```rust
#[derive(Debug, Clone)]
pub enum TokenContextAction {
    Cut(TokenId),
    Copy(TokenId),
    Paste,
    Delete(TokenId),
    ExpandGroup(TokenId),
    MoveTo { token_id: TokenId, target: RecipientField },
}
```

### Group Expansion

When "Expand group" is selected:

1. Fetch group members via `db_expand_contact_group(group_id)` (recursive expansion, handles nested groups)
2. For each email in the result: look up display name from contacts table (if available)
3. Remove the group token
4. Insert individual person tokens for each member
5. All in a single model update (no intermediate render with partial state)

---

## Phase 5: Token Drag and Drop

### Approach

Token drag-and-drop between To/Cc/Bcc fields uses iced's mouse events directly (not `iced_drop`). The token input widget tracks drag state internally and emits events. The compose view coordinates cross-field moves.

### Drag Detection

1. Mouse down on a token starts tracking in `DragState { token_id, origin, current }`
2. If the cursor moves more than 4px from origin while the button is held, the drag is activated
3. During active drag: the original token is visually dimmed (alpha 0.3), and a ghost token follows the cursor
4. The ghost is drawn in the compose view's `draw()` as an overlay, not inside the token input widget

### Drop Detection

Each token input widget reports its bounds. The compose view checks which field the cursor is over when the mouse button is released:

```rust
// In compose update():
ComposeMessage::DragEnd(position) => {
    if let Some(drag) = self.drag_state.take() {
        // Hit-test position against field bounds
        let target = self.field_at_position(position);
        if let Some(target_field) = target {
            if target_field != drag.source_field {
                // Move token from source to target
                self.move_token(drag.token_id, drag.source_field, target_field);
            }
        }
    }
    Task::none()
}
```

### Visual Feedback

During drag:

- Source token: dimmed (reduced opacity)
- Target field: highlighted border (primary color) when cursor is over it
- Ghost token: follows cursor with slight offset, drawn at normal opacity

---

## Phase 6: Contact Group Tokens & Banners

### Group Token Rendering

Group tokens are visually distinct from person tokens:

- **Icon prefix**: a small people/group icon before the label text
- **Label format**: `"Group Name (N)"` where N is the member count
- **Same chip style**: same background, radius, padding as person tokens

Group tokens are **atomic** — they cannot be partially modified in the compose field. The user cannot remove individual members from a group token. To modify group membership, they use the contact management interface.

### Bcc Nudge Banner

When a contact group is added to the To or Cc field, a dismissible banner appears:

```
┌─────────────────────────────────────────────────────────────┐
│ ℹ "Engineering" has 12 members. Move to Bcc?  [Move] [Dismiss] │
└─────────────────────────────────────────────────────────────┘
```

State:

```rust
struct BccNudgeBanner {
    visible: bool,
    group_name: String,
    token_id: TokenId,
    source_field: RecipientField,
}
```

The banner is field-specific: adding a group to Bcc does not trigger it. Adding multiple groups shows one banner per group (stacked). Dismissing is per-banner. Accepting moves the group token to Bcc and dismisses the banner.

---

## Compose View Integration

### Compose Model State

```rust
// crates/app/src/ui/compose.rs

pub struct ComposeState {
    /// Recipient field values.
    pub to: TokenInputValue,
    pub cc: TokenInputValue,
    pub bcc: TokenInputValue,

    /// Which field is currently focused.
    pub focused_field: Option<RecipientField>,

    /// Autocomplete state.
    pub autocomplete: AutocompleteState,

    /// Whether Cc field is visible.
    pub show_cc: bool,
    /// Whether Bcc field is visible.
    pub show_bcc: bool,

    /// Bcc nudge banners for group tokens.
    pub bcc_nudges: Vec<BccNudgeBanner>,
    /// Bulk paste banner.
    pub bulk_paste_banner: Option<BulkPasteBanner>,

    /// Active drag state (if a token is being dragged).
    pub drag: Option<ComposeTokenDrag>,

    /// Token context menu state.
    pub context_menu: Option<ContextMenuState>,

    // ... subject, body, attachments (from other specs)
}

struct ContextMenuState {
    token_id: TokenId,
    field: RecipientField,
    position: Point,
    is_group: bool,
}

struct ComposeTokenDrag {
    token_id: TokenId,
    source_field: RecipientField,
    current_position: Point,
}
```

### Compose Message Enum

```rust
#[derive(Debug, Clone)]
pub enum ComposeMessage {
    /// Token input event from a specific field.
    TokenInput(RecipientField, TokenInputMessage),

    /// Autocomplete results arrived (with generation).
    AutocompleteResults(u64, Result<Vec<ContactSearchResult>, String>),
    /// User selected an autocomplete result.
    AutocompleteSelect(usize),
    /// Arrow key navigation in dropdown.
    AutocompleteNavigate(i32), // +1 down, -1 up
    /// Dismiss autocomplete dropdown.
    AutocompleteDismiss,

    /// Context menu action.
    ContextMenuAction(TokenContextAction),
    /// Dismiss context menu.
    ContextMenuDismiss,

    /// Show Cc field.
    ShowCc,
    /// Show Bcc field.
    ShowBcc,

    /// Bcc nudge: move group to Bcc.
    BccNudgeAccept(TokenId),
    /// Bcc nudge: dismiss.
    BccNudgeDismiss(TokenId),

    /// Bulk paste: save as group.
    BulkPasteSaveGroup,
    /// Bulk paste: dismiss.
    BulkPasteDismiss,

    /// Drag events.
    DragMove(Point),
    DragEnd(Point),
    DragCancel,

    // ... subject, body, send, etc.
}
```

### Update Flow for Token Addition (Autocomplete Path)

1. User types "ali" in the To field
2. `TokenInput` emits `TextChanged("ali")`
3. Compose dispatches search, receives `AutocompleteResults` with `[Alice Smith <alice@corp.com>, ...]`
4. Dropdown renders. User presses Enter.
5. `AutocompleteSelect(0)` triggers:
   - Create `Token { id: next_id(), email: "alice@corp.com", label: "Alice Smith", is_group: false, group_id: None }`
   - Push to `self.to.tokens`
   - Clear `self.to.text`
   - Clear `self.autocomplete`

### Update Flow for Token Addition (Raw Text Path)

1. User types "bob@example.com" — no autocomplete matches
2. Dropdown hidden. User presses comma.
3. `TokenInput` emits `TokenizeText("bob@example.com")`
4. Compose validates the text looks like an email (contains `@`, non-empty parts)
5. Creates `Token { id: next_id(), email: "bob@example.com", label: "bob@example.com", is_group: false, group_id: None }`
6. Push to tokens, clear text

### Email Validation for Raw Tokenization

Minimal validation — the goal is to catch typos, not enforce RFC 5321:

```rust
fn is_plausible_email(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let parts: Vec<&str> = trimmed.splitn(2, '@').collect();
    parts.len() == 2 && !parts[0].is_empty() && parts[1].contains('.')
}
```

---

## Implementation Order

### Phase 1: Core Token Input Widget
**Files:** `crates/app/src/ui/token_input.rs`, `crates/app/src/ui/layout.rs` (new constants)

1. Define `Token`, `TokenId`, `TokenInputValue`, `TokenInputState` types
2. Implement `Widget` trait: `layout()` with wrapping flow algorithm
3. Implement `draw()`: field background, token chips, text cursor
4. Implement `on_event()`: text input, backspace deletion, click selection
5. Implement keyboard navigation (arrow keys between tokens)
6. Wire into a test harness (standalone compose-like view with hardcoded tokens)

**Verification:** Tokens render correctly, wrap on narrow widths, backspace removes last token, clicking selects tokens, typing produces text.

### Phase 2: Autocomplete Dropdown
**Files:** `crates/core/src/contacts/search.rs` (new), `crates/app/src/ui/compose.rs`

1. Implement `search_contacts_unified()` in core
2. Add `ContactSearchResult` and `ContactSearchKind` types
3. Build autocomplete state management in compose model
4. Render dropdown as popover overlay below focused field
5. Implement keyboard navigation in dropdown (Up/Down/Enter/Tab/Escape)
6. Implement the key dispatch split (dropdown visible vs hidden)
7. Wire generation-tracked search with minimal debounce

**Verification:** Typing shows matching contacts, arrow keys navigate, Enter selects and tokenizes, Escape dismisses, comma/semicolon always tokenize.

### Phase 3: Paste Handling
**Files:** `crates/app/src/ui/token_input_parse.rs` (new)

1. Implement `parse_pasted_addresses()` with common RFC 5322 formats
2. Handle paste event in compose: parse, create tokens, skip dropdown
3. Implement bulk paste banner (10+ addresses)
4. Unit tests for parse function (bare emails, name+bracket, quoted names, mixed, semicolons, newlines)

**Verification:** Paste from Outlook/Gmail address fields creates correct tokens with display names. Banner appears for bulk paste.

### Phase 4: Context Menu & Group Expansion
**Files:** Compose view updates

1. Implement right-click detection in token input widget
2. Build context menu overlay with field-aware items
3. Implement Cut/Copy/Paste/Delete actions
4. Implement "Expand group" (fetch members, replace token)
5. Implement "Move to To/Cc/Bcc"

**Verification:** Right-click shows correct menu items per field. Expand replaces group with individual tokens. Move transfers token between fields.

### Phase 5: Token Drag and Drop
**Files:** Token input widget + compose view

1. Implement drag detection in token input (4px threshold)
2. Draw ghost token during drag (compose view overlay)
3. Highlight target field during drag
4. Implement drop handling (move token between fields)
5. Cancel drag on Escape

**Verification:** Dragging a token from To to Bcc moves it. Visual feedback during drag. Cancel works.

### Phase 6: Group Tokens & Banners
**Files:** Compose view updates

1. Implement group token rendering (icon prefix, member count)
2. Implement Bcc nudge banner when group added to To/Cc
3. Wire banner accept (move to Bcc) and dismiss
4. Integrate bulk paste banner "Save as group" with group creation (placeholder until management spec)

**Verification:** Group tokens visually distinct. Bcc nudge appears and functions. Banners dismiss correctly.

---

## Testing Strategy

### Unit Tests

- `parse_pasted_addresses`: comprehensive format coverage
- `is_plausible_email`: edge cases (no @, no domain dot, empty, whitespace)
- `search_contacts_unified`: requires test DB with contacts, seen_addresses, and groups seeded
- Token flow logic: add/remove/reorder tokens, ID stability

### Integration Tests

- Full compose recipient round-trip: type query, select from dropdown, verify token appears
- Paste round-trip: paste multiline addresses, verify correct tokens created
- Cross-field drag: drag token from To to Cc, verify model state

### Manual Testing Checklist

- [ ] Tokens wrap correctly at different window widths
- [ ] Backspace at empty text selects last token; second backspace removes it
- [ ] Arrow keys navigate through tokens and back to text input
- [ ] Autocomplete dropdown appears/disappears based on match state
- [ ] Tab accepts top suggestion when dropdown is visible
- [ ] Comma always tokenizes, even with dropdown visible
- [ ] Paste from Outlook "Name <email>; Name2 <email2>" format works
- [ ] Right-click context menu shows correct items for person vs group tokens
- [ ] Expand group replaces group token with individual member tokens
- [ ] Drag token between To/Cc/Bcc works
- [ ] Bcc nudge banner appears when adding group to To field
- [ ] Bulk paste banner appears for 10+ pasted addresses
- [ ] Field grows vertically as tokens wrap to new rows
- [ ] Dismissing autocomplete with Escape keeps typed text
- [ ] Group tokens show icon and member count

---

## Dependencies

### Crate Dependencies (no new external crates required)

- **iced advanced widget API** — already available via `iced::advanced`
- **rusqlite** — existing, for synchronous DB search
- **tokio** — existing, for async search dispatch

### Internal Dependencies

- `crates/core/src/db/queries.rs` — `search_contacts()` (exists)
- `crates/core/src/db/queries_extra/contact_groups.rs` — `db_search_contact_groups()`, `db_expand_contact_group()` (exist)
- `crates/db/src/db/types.rs` — `DbContact`, `DbContactGroup` (exist)
- `crates/seen-addresses/src/types.rs` — `SeenAddressMatch` (exists)
- `crates/app/src/ui/popover.rs` — popover overlay positioning (exists)
- `crates/app/src/ui/theme.rs` — needs new style functions for token chips and autocomplete rows
- `crates/app/src/ui/layout.rs` — needs new constants (listed above)

### New Files

| File | Purpose |
|------|---------|
| `crates/app/src/ui/token_input.rs` | Custom `advanced::Widget` for token input |
| `crates/app/src/ui/token_input_parse.rs` | RFC 5322 address parsing for paste |
| `crates/core/src/contacts/search.rs` | Unified contact search function |
| `crates/core/src/contacts/mod.rs` | Module declaration |

### Modified Files

| File | Changes |
|------|---------|
| `crates/app/src/ui/layout.rs` | Add token/autocomplete layout constants |
| `crates/app/src/ui/theme.rs` | Add token chip and autocomplete row styles |
| `crates/app/src/ui/mod.rs` | Declare `token_input` and `token_input_parse` modules |
| `crates/core/src/lib.rs` | Declare `contacts` module |
