# Chats Phase 2: Chat Timeline View

Revised after arch review.

## Overview

Phase 2 adds the bubble-based timeline view that appears when a chat contact is selected. This is the core visual experience — everything else (sidebar integration in Phase 3, compose in Phase 4) builds on it.

No sidebar changes yet — this phase is activated by calling `get_chat_timeline()` from Phase 1 and rendering the result. Phase 3 adds the sidebar trigger.

## Layout Architecture

### Chat as a mail route, not a loose flag

Calendar uses `AppMode::Calendar` which replaces the entire layout including sidebar. Chat is different — the sidebar stays (it will hold the CHATS section in Phase 3). Chat view is a **mail route** — a subview within `AppMode::Mail`.

Use `NavigationTarget::Chat { email: String }` (already planned in the Phase 1 overview) to enter chat view. When the navigation target is `Chat`, the mail layout renders sidebar + chat timeline (instead of sidebar + thread list + reading pane).

No `active_chat: Option<String>` field. The navigation target IS the source of truth. All existing navigation machinery (`reset_view_state`, generation counters, keyboard routing) works through the same channel.

```rust
// NavigationTarget variant (in command_dispatch.rs)
Chat { email: String },
```

### Layout in view()

Inside the `AppMode::Mail` branch, check the navigation target:

```rust
let is_chat = matches!(self.navigation_target, Some(NavigationTarget::Chat { .. }));

if is_chat {
    // Sidebar + chat timeline (full width of remaining space)
    let chat_view = self.chat_timeline.view().map(Message::Chat);
    row![sidebar, divider_sidebar, chat_view].height(Length::Fill)
} else {
    // Normal: sidebar + thread list + divider + reading pane + right sidebar
    // ... existing layout ...
}
```

## Generation Counter

New branded counter for chat timeline loads — don't reuse `Nav` (a navigation bump while chat is loading would discard chat results):

```rust
// In generation.rs
pub enum Chat {}

// In App struct
chat_generation: GenerationCounter<Chat>,
```

## Chat Timeline Component

Lives in `crates/app/src/ui/chat_timeline.rs` as a Component (per UI.md conventions).

### State

```rust
pub struct ChatTimeline {
    /// Messages to display, ordered chronologically (oldest first).
    pub messages: Vec<ChatMessage>,
    /// Whether a load is in progress.
    pub loading: bool,
    /// The contact email this timeline is showing.
    pub contact_email: String,
    /// Scroll position — auto-scroll to bottom on initial load.
    pub scroll_id: scrollable::Id,
    /// Per-message expansion state (for "show full message").
    pub expanded: HashSet<String>,
}
```

Created fresh on each `enter_chat_view`. The `expanded` set doesn't persist across exits — acceptable for Phase 2.

### Messages

```rust
pub enum ChatTimelineMessage {
    /// User clicked "show full message" on a bubble.
    ToggleExpand(String),
    /// User scrolled to top — load older messages.
    LoadOlder,
}

pub enum ChatTimelineEvent {
    /// Request to load older messages (scroll hit top).
    LoadOlderRequested,
}
```

Timeline data loading is handled in `handlers/chat.rs`, not in the component — the component only renders and emits events.

### View

The timeline is a `scrollable` column of message bubbles, newest at bottom:

```
┌──────────────────────────────────────┐
│  ── March 25 ──                      │  ← date separator
│                                      │
│                    ┌─────────────┐   │
│                    │ hey, quick  │   │  ← sent (right, accent)
│                    │ question    │   │
│                    └─────────────┘   │
│                              10:32   │
│                                      │
│  ┌─────────────────┐                 │
│  │ yeah that works │                 │  ← received (left, surface)
│  └─────────────────┘                 │
│  10:34                               │
│                                      │
│  ── March 26 ──                      │  ← date separator
│                                      │
│  Re: project update                  │  ← subject change (subtle)
│  ┌─────────────────┐                 │
│  │ sent the deck   │                 │
│  └─────────────────┘                 │
│  09:15                               │
└──────────────────────────────────────┘
```

### Layout Constants (in `layout.rs`)

```rust
pub const CHAT_BUBBLE_MAX_WIDTH: f32 = 480.0;
pub const CHAT_BUBBLE_RADIUS: f32 = 12.0;
pub const PAD_CHAT_BUBBLE: Padding = Padding { top: 8, right: 12, bottom: 8, left: 12 };
pub const CHAT_BUBBLE_SPACING: f32 = 4.0;     // between consecutive same-sender bubbles
pub const CHAT_GROUP_SPACING: f32 = 12.0;      // between sender changes
pub const CHAT_DATE_SEPARATOR_SPACING: f32 = 16.0;
```

Timestamp uses `TEXT_XS` (10). Date separator uses `TEXT_SM` (11). Subject change indicator uses `TEXT_SM` (11).

### Theme Tokens

```rust
pub enum ContainerClass {
    // ... existing ...
    ChatBubbleSent,       // palette.primary.weak bg, high-contrast text
    ChatBubbleReceived,   // palette.background.weakest bg, standard text
}
```

Both use `CHAT_BUBBLE_RADIUS` border radius. Date separator is plain centered text, no container needed.

### Bubble Rendering

Each `ChatMessage` renders as:

- **Alignment:** sent (`is_from_user = true`) → right-aligned via `Length::FillPortion` spacer, received → left-aligned
- **Background:** sent → `ChatBubbleSent`, received → `ChatBubbleReceived`
- **Content:** body text (signature-stripped). If expanded, show full body.
- **Max width:** `CHAT_BUBBLE_MAX_WIDTH` — bubble doesn't span full width
- **Timestamp:** below bubble, `TEXT_XS`, muted, time-only for today, date+time for older

### Date Separators

Centered date label between messages on different calendar days. Format: "Today", "Yesterday", or "March 25".

### Subject Change Indicators

When consecutive messages have different `thread_id` AND different `subject`, show the new subject in `TEXT_SM` muted text above the bubble.

## Signature Stripping

Reusable module in `crates/common/src/signature_strip.rs`.

**Important distinction:** "signature stripping" and "quote collapsing" are separate transforms. Both are useful in chat view, but they're different operations:

- **Signature stripping:** Remove the sender's appended signature block
- **Quote collapsing:** Remove quoted reply content (`On <date>, <person> wrote:` + `>` lines)

Both are applied in chat view. Both are reversible (original preserved in body store, accessible via "show full message").

### API

```rust
/// Strip known signature patterns from message body.
/// `user_signatures`: the user's own configured signatures, for Layer 3 matching.
pub fn strip_signature(body: &str, is_html: bool, user_signatures: &[&str]) -> String

/// Collapse quoted reply content from message body.
pub fn collapse_quotes(body: &str, is_html: bool) -> String
```

### Signature Stripping Layers (1-3)

**Layer 1: HTML client markers** (when `is_html = true`). Use `lol_html` streaming rewriter:

| Client | Pattern |
|--------|---------|
| Gmail | `<div class="gmail_signature">` to closing `</div>` (with depth tracking for nested divs) |
| Outlook | Everything after `<hr id="stopSpelling">` |
| Thunderbird | `<div class="moz-cite-prefix">` (the prefix text only, NOT the following `<blockquote>`) |

Note: `<blockquote type="cite">` is quoted reply content, NOT a signature marker. It's handled by `collapse_quotes`, not `strip_signature`.

**Layer 2: RFC 3676 delimiter.** Strip everything after `^-- $` (dash dash space newline).

**Layer 3: User's own signatures.** For sent messages, match against `user_signatures` parameter. Exact suffix match after whitespace normalization.

### Quote Collapsing

Separate function. Removes:
- `On <date>, <person> wrote:` + `<blockquote>` blocks (HTML)
- `>` prefixed lines (plain text)
- Gmail `<div class="gmail_quote">` / `<div class="gmail_extra">` blocks
- Yahoo `<div class="yahoo_quoted">` blocks

## Body Loading

Messages from Phase 1's `get_chat_timeline()` have metadata but no body text. Bodies are loaded from `BodyStoreState`.

### Strategy

1. `enter_chat_view` calls `get_chat_timeline(email, 50, None)`
2. For the loaded batch, call `BodyStoreState::get_body(message_id)` — batch into a single `with_conn` call if possible
3. Apply `strip_signature` + `collapse_quotes` to each body
4. Store stripped bodies on `ChatMessage` (add `body_text: Option<String>` field)

For pagination (load older):
1. User scrolls near top → component emits `LoadOlderRequested`
2. Handler calls `get_chat_timeline(email, 50, before=(date, message_id))`
3. Prepend to messages, load bodies, strip

### Pagination Cursor

Use `(date, message_id)` tuple as cursor, not just `date`. Equal timestamps must not skip or duplicate messages:

```sql
WHERE (m.date < ?1 OR (m.date = ?1 AND m.id < ?2))
```

## Navigation Integration

### Handler: `handlers/chat.rs`

Per UI.md: feature logic in handlers, not main.rs. New handler file:

```rust
impl App {
    pub(crate) fn enter_chat_view(&mut self, email: String) -> Task<Message> {
        self.navigation_target = Some(NavigationTarget::Chat { email: email.clone() });
        self.clear_thread_selection();
        self.chat_timeline = ChatTimeline::new(email.clone());

        let db = Arc::clone(&self.db);
        let body_store = self.body_store.clone();
        let user_emails = self.user_emails();
        let token = self.chat_generation.next();

        // Load timeline + mark read in parallel
        Task::batch([
            // Timeline load
            Task::perform(
                async move { load_chat_timeline(db, body_store, email, user_emails, 50, None).await },
                move |result| Message::Chat(ChatTimelineMessage::TimelineLoaded(token, result)),
            ),
            // Mark read (fire-and-forget, non-blocking)
            self.mark_chat_read(&email),
        ])
    }

    pub(crate) fn exit_chat_view(&mut self) {
        self.navigation_target = None;
        // Don't call reset_view_state — just clear the target.
        // The next navigation action will set its own target.
    }
}
```

### Mark Read on Enter

**Does NOT route through the action service.** This is a navigation side effect, not a user-initiated action. No undo tokens, no toasts, no action completion handler.

```rust
fn mark_chat_read(&mut self, email: &str) -> Task<Message> {
    let db = Arc::clone(&self.db);
    let email = email.to_string();
    Task::perform(
        async move {
            // 1. Local: batch UPDATE messages + re-aggregate threads
            rtsk::chat::mark_chat_read_local(&db, &email).await?;
            // 2. Provider: fire-and-forget mark_read per affected thread.
            //    Failures create pending ops for retry on next sync.
            rtsk::chat::mark_chat_read_remote(&db, &email).await;
            Ok::<(), String>(())
        },
        |_| Message::ChatReadMarked,
    )
}
```

`mark_chat_read_local`: single transaction — `UPDATE messages SET is_read = 1`, re-aggregate `threads.is_read`, update `chat_contacts.unread_count = 0`.

`mark_chat_read_remote`: resolve affected threads, create provider, batch `provider.mark_read()` per thread. Enqueue pending ops for failures. No undo.

### Scroll Behavior

**Initial load:** snap to bottom. Use `iced::widget::scrollable::snap_to(scroll_id, RelativeOffset::END)` as a Task returned after timeline load completes.

**Older messages prepended:** This is genuinely hard in iced. The content height changes but the scroll offset doesn't auto-adjust.

**Phase 2 approach (simple):** Show a "Load older" button at the top of the timeline. Clicking it loads and prepends older messages, then the viewport stays where it is (the user is at the top, new content appears above their viewport — they scroll up to see it). No auto-scroll-position maintenance needed.

**Phase 6 approach (polished):** Investigate iced's `scroll_to` with `AbsoluteOffset` to maintain position relative to an anchor message. Defer this complexity.

### Loading State

While the timeline loads, `chat_timeline.loading = true`. The component renders a spinner or "Loading..." placeholder. Once `TimelineLoaded` arrives, `loading = false` and bubbles render.

## Files to Create

- `crates/app/src/ui/chat_timeline.rs` — Component: state, messages, view
- `crates/app/src/handlers/chat.rs` — enter/exit, load, mark-read, older-load
- `crates/common/src/signature_strip.rs` — `strip_signature` + `collapse_quotes`

## Files to Modify

- `crates/app/src/main.rs` — `Message::Chat` variant, `chat_timeline` field, `chat_generation` field, view layout branching (minimal — delegates to handler)
- `crates/app/src/command_dispatch.rs` — `NavigationTarget::Chat { email }` variant
- `crates/app/src/ui/mod.rs` — register `chat_timeline` module
- `crates/app/src/handlers/mod.rs` — register `chat` module
- `crates/app/src/ui/layout.rs` — chat-specific constants
- `crates/app/src/ui/theme.rs` — `ChatBubbleSent`, `ChatBubbleReceived` variants
- `crates/core/src/generation.rs` — `Chat` brand tag
- `crates/core/src/chat.rs` — `mark_chat_read_local`, `mark_chat_read_remote`, update `get_chat_timeline` for tuple cursor
- `crates/common/src/lib.rs` — register `signature_strip` module

## Verification

1. Enter chat view (via command palette or test) → bubble timeline renders with messages aligned by ownership
2. Loading spinner shown while timeline loads
3. Date separators appear between messages on different days
4. Subject change indicators appear when thread changes
5. Scrolled to bottom on initial load
6. "Load older" button at top → older messages prepend
7. Signatures stripped from bubbles (Gmail signature divs, `-- ` delimiter, user's own sigs)
8. Quoted reply content collapsed separately from signatures
9. "Show full message" expands bubble to show original unstripped content
10. Navigate away (click folder) → returns to normal mail layout
11. Entering chat marks unread messages read (local immediately, provider async)
12. Chat timeline uses `GenerationCounter<Chat>` — nav bumps don't discard chat loads
