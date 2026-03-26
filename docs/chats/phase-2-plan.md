# Chats Phase 2: Chat Timeline View

## Overview

Phase 2 adds the bubble-based timeline view that appears when a chat contact is selected. This is the core visual experience — everything else (sidebar integration in Phase 3, compose in Phase 4) builds on it.

No sidebar changes yet — this phase is activated by calling `get_chat_timeline()` from Phase 1 and rendering the result. Phase 3 adds the sidebar trigger.

## Layout Architecture

### Not a new AppMode

Calendar uses `AppMode::Calendar` which replaces the entire center layout. Chat is different — the sidebar stays (it will hold the CHATS section in Phase 3). Only the thread list + reading pane area is replaced by the chat timeline.

Add an `active_chat: Option<String>` field on App (the contact email). When set:
- The `AppMode::Mail` layout renders sidebar + chat timeline (instead of sidebar + thread list + reading pane)
- Thread list and reading pane are hidden but not destroyed (state preserved for when user exits chat)
- `active_chat` is cleared when the user navigates away (clicks a folder, account, etc.)

```rust
// In App struct
active_chat: Option<String>,  // email of the active chat contact, if any
```

### Layout in view()

Inside the `AppMode::Mail` branch of the view function:

```rust
if let Some(ref chat_email) = self.active_chat {
    // Sidebar + chat timeline (full width of remaining space)
    let chat_view = chat_timeline_view(&self.chat_timeline_state, chat_email)
        .map(Message::Chat);
    row![sidebar, divider_sidebar, chat_view].height(Length::Fill)
} else {
    // Normal: sidebar + thread list + divider + reading pane + right sidebar
    // ... existing layout ...
}
```

## Chat Timeline Component

### State

```rust
pub struct ChatTimelineState {
    /// Messages to display, ordered chronologically (oldest first).
    pub messages: Vec<ChatMessage>,
    /// Whether a load is in progress.
    pub loading: bool,
    /// The contact email this timeline is showing.
    pub contact_email: String,
    /// Scroll position — auto-scroll to bottom on initial load.
    pub scroll_id: scrollable::Id,
    /// Per-message expansion state (for "show full message").
    pub expanded: HashSet<String>,  // message IDs
}
```

### Messages

```rust
pub enum ChatTimelineMessage {
    /// Timeline data loaded from DB.
    TimelineLoaded(GenerationToken<Nav>, Result<Vec<ChatMessage>, String>),
    /// User clicked "show full message" on a bubble.
    ToggleExpand(String),  // message_id
    /// User scrolled to top — load older messages.
    LoadOlder,
    /// Older messages loaded (prepend).
    OlderLoaded(Result<Vec<ChatMessage>, String>),
}
```

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
│                                      │
│                    ┌─────────────┐   │
│                    │ thanks!     │   │
│                    └─────────────┘   │
│                              09:16   │
└──────────────────────────────────────┘
```

### Bubble Rendering

Each `ChatMessage` renders as:

- **Alignment:** sent (`is_from_user = true`) → right-aligned, received → left-aligned
- **Background:** sent → `theme::ContainerClass::ChatBubbleSent`, received → `theme::ContainerClass::ChatBubbleReceived`
- **Content:** body text (signature-stripped, see below). If expanded, show full body.
- **Timestamp:** below bubble, small muted text, relative or time-only for today
- **"Show full" affordance:** small icon button on the bubble, visible on hover. Toggles `expanded` set.

### Date Separators

Between messages on different calendar days, render a centered date label:

```rust
fn needs_date_separator(prev: &ChatMessage, curr: &ChatMessage) -> bool {
    // Compare dates (ignoring time)
    date_of(prev.date) != date_of(curr.date)
}
```

Format: "Today", "Yesterday", or "March 25" for older dates.

### Subject Change Indicators

When the subject changes between consecutive messages (different thread), show the new subject in small muted text above the bubble:

```rust
fn needs_subject_indicator(prev: &ChatMessage, curr: &ChatMessage) -> bool {
    prev.thread_id != curr.thread_id
        && curr.subject.as_deref() != prev.subject.as_deref()
}
```

### Auto-scroll

On initial timeline load, scroll to the bottom (latest message). Use `scrollable::snap_to(id, RelativeOffset::END)` as a Task returned from the load handler.

On subsequent loads (older messages prepended), maintain scroll position — don't jump to top or bottom.

## Signature Stripping (Basic — Layers 1-3)

Build as a reusable function in `crates/provider-utils/src/signature_strip.rs`:

```rust
/// Strip known signature patterns from message body text/HTML.
/// Returns the cleaned body. Non-destructive — the original is preserved
/// in the body store and accessible via "show full message".
pub fn strip_signatures(body: &str, is_html: bool) -> String
```

### Layer 1: HTML Client Markers

When `is_html = true`, remove content within known signature containers:

| Client | Pattern |
|--------|---------|
| Gmail | `<div class="gmail_signature">` to closing `</div>` |
| Gmail quotes | `<div class="gmail_quote">`, `<div class="gmail_extra">` |
| Outlook | Everything after `<hr id="stopSpelling">` |
| Thunderbird | `<div class="moz-cite-prefix">` + `<blockquote type="cite">` |
| Apple Mail | `<blockquote type="cite">` |
| Yahoo | `<div class="yahoo_quoted">` |

Use `lol_html` (already a dependency via `html_sanitizer.rs`) for efficient streaming rewrite.

### Layer 2: RFC 3676 Delimiter

Strip everything after a line matching `^-- $` (dash dash space newline). Works on both plain text and pre-processed HTML-to-text.

### Layer 3: User's Own Signatures

For sent messages (`is_from_user = true`), match against the user's configured signatures from the `signatures` table. Exact suffix match after whitespace normalization.

### Layer 4+: Deferred to Phase 5

Per-sender learned patterns and heuristic valediction phrases are Phase 5.

## Body Loading

Messages in the timeline need body text from `bodies.db`:

- `get_chat_timeline()` from Phase 1 returns `ChatMessage` without body text (just metadata from `messages` table)
- Body loading is a separate step via `BodyStoreState::get_body(message_id)`
- Load bodies for the initial visible batch (last N messages)
- Load older message bodies on demand (when user scrolls up or expands)

### Loading Strategy

1. Initial load: call `get_chat_timeline(email, limit=50, before=None)`
2. For each message, load body from body store
3. Strip signatures on the loaded body
4. Render bubbles

For pagination (scroll to top):
1. User scrolls near top → emit `LoadOlder`
2. Call `get_chat_timeline(email, limit=50, before=oldest_message.date)`
3. Prepend to `messages`, load bodies, strip signatures

## Navigation Integration

### Entering Chat View

When `active_chat` is set (will be triggered by sidebar in Phase 3, but can be tested via command palette):

```rust
fn enter_chat_view(&mut self, email: String) -> Task<Message> {
    self.active_chat = Some(email.clone());
    let db = Arc::clone(&self.db);
    let body_store = self.body_store.clone();
    let user_emails = self.user_emails();
    let token = self.nav_generation.next();
    Task::perform(
        async move {
            let messages = ratatoskr_core::chat::get_chat_timeline(
                &db, &email, &user_emails, 50, None,
            ).await?;
            // Load bodies for each message...
            Ok((token, messages))
        },
        |(token, result)| Message::Chat(ChatTimelineMessage::TimelineLoaded(token, result)),
    )
}
```

### Exiting Chat View

Any navigation away from chat clears it:

```rust
// In reset_view_state:
self.active_chat = None;
```

### Guards

- `handle_select_thread`: if `active_chat.is_some()`, ignore (no thread detail loading)
- `handle_email_action`: if `active_chat.is_some()`, ignore (actions don't apply to chat view — Phase 4 compose handles send)
- Thread list keyboard shortcuts: if `active_chat.is_some()`, ignore

### Mark Read

When entering a chat view, mark all unread messages from that contact as read:

```rust
// In enter_chat_view, after timeline loads:
ratatoskr_core::chat::mark_chat_read(&db, &email).await?;
```

This needs a new core function that:
1. Finds all unread messages in `is_chat_thread = 1` threads for the contact
2. Sets `messages.is_read = 1` and re-aggregates `threads.is_read`
3. Dispatches `mark_read` through the action service for each affected thread (to sync provider state)
4. Updates `chat_contacts.unread_count = 0`

Rate-limit the provider dispatch — batch per account, max 10 threads per mark-read cycle.

## Theme Tokens

New theme catalog entries:

```rust
pub enum ContainerClass {
    // ... existing ...
    ChatBubbleSent,
    ChatBubbleReceived,
    ChatDateSeparator,
}
```

- `ChatBubbleSent`: accent background, high-contrast text, rounded corners (larger radius than cards)
- `ChatBubbleReceived`: surface/secondary background, standard text, rounded corners
- `ChatDateSeparator`: transparent background, centered muted text with optional horizontal rules

## Files to Create

- `crates/app/src/ui/chat_timeline.rs` — component: state, messages, view
- `crates/provider-utils/src/signature_strip.rs` — reusable signature stripping

## Files to Modify

- `crates/app/src/main.rs` — `active_chat` field, view layout branching, `Message::Chat` variant, `enter_chat_view`, `reset_view_state` clears chat
- `crates/app/src/ui/mod.rs` — register `chat_timeline` module
- `crates/app/src/ui/theme.rs` — new `ContainerClass` variants
- `crates/core/src/chat.rs` — `mark_chat_read` function
- `crates/provider-utils/src/lib.rs` — register `signature_strip` module

## Verification

1. Enter chat view (via test command) → bubble timeline renders with messages aligned by ownership
2. Date separators appear between messages on different days
3. Subject change indicators appear when thread changes
4. Scrolled to bottom on initial load
5. Scroll to top → older messages load and prepend
6. Signatures stripped from bubbles (Gmail signature divs, `-- ` delimiter, user's own sigs)
7. "Show full message" expands bubble to show unstripped content
8. Navigate away (click folder) → returns to normal mail layout
9. Re-enter chat → timeline reloads from latest
10. Entering chat marks all unread messages read (local + provider dispatch)
