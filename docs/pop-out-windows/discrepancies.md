# Pop-Out Windows: Spec vs. Implementation Discrepancies

Audit date: 2026-03-21. Compared `problem-statement.md` and `message-view-implementation-spec.md` against codebase in `crates/app/src/`.

---

## What's Implemented and Matches the Spec

### Phase 1: Multi-Window Architecture (complete)
- **Daemon migration**: `main()` uses `iced::daemon` with `App::boot`, `App::update`, `App::view` — matches spec exactly.
- **Main window opened in `boot()`**: `iced::window::open(window.to_window_settings())` stores `main_window_id` — matches spec.
- **Window registry**: `pop_out_windows: HashMap<window::Id, PopOutWindow>` on `App` — matches spec.
- **`PopOutWindow` enum**: Has `MessageView(MessageViewState)` and `Compose(ComposeState)` variants — matches spec (compose is ahead of spec, which listed it as a future variant).
- **`PopOutMessage` enum**: Routes `MessageView` and `Compose` sub-messages — matches spec.
- **View routing**: `view()` takes `window::Id`, dispatches to `view_main_window()`, `view_message_window()`, or `view_compose_window()` — matches spec.
- **Title routing**: `title()` dispatches per window, message view shows `"subject — sender"` — matches spec.
- **Window close cascade**: Main window close cascades to all pop-outs then calls `iced::exit()` — matches spec.
- **Pop-out close**: Removes from registry — matches spec.
- **Escape closes pop-out**: Keyboard handler checks `window_id != self.main_window_id && pop_out_windows.contains_key(...)` — matches spec's `EscapePressed` pattern, though implemented inside the existing `KeyPressed` handler rather than a dedicated `EscapePressed` message variant.
- **Window resize/move events carry window ID**: `WindowResized(window::Id, Size)` and `WindowMoved(window::Id, Point)` — matches spec.
- **Subscription updates**: `resize_events`, `close_requests`, `listen_with` for moved events all carry window IDs — matches spec.

### Phase 2: Message View Window (mostly complete)
- **`MessageViewState` struct**: Fields match spec for identity, header, body, attachments, geometry — implemented.
- **`MessageViewMessage` enum**: `BodyLoaded`, `AttachmentsLoaded`, `Reply`, `ReplyAll`, `Forward`, `Noop` — matches spec subset.
- **`from_thread_message()` constructor**: Seeds from `ThreadMessage` — matches spec.
- **`open_message_view_window()`**: Creates `MessageViewState`, opens window with correct settings and min sizes, inserts into registry, dispatches body+attachment loads — matches spec.
- **View layout**: Header (from name/email, To, subject+date row, action buttons), divider, body, conditional attachment section — matches spec.
- **Attachment cards**: File icon by MIME type, filename, size formatting, type label — matches spec.
- **Async data loads**: `Db::load_message_body()` and `Db::load_message_attachments()` implemented with raw SQL — matches spec.
- **Layout constants**: `MESSAGE_VIEW_DEFAULT_WIDTH/HEIGHT` and `MESSAGE_VIEW_MIN_WIDTH/HEIGHT` used — matches spec.
- **Pop-out icon button in reading pane**: `ReadingPaneMessage::PopOut(usize)` emits `ReadingPaneEvent::OpenMessagePopOut` — matches spec.

### Compose Window (beyond current spec scope, but partially implemented)
- **`ComposeState`** with recipients (token input), from account picker, subject, body (text editor), mode — implemented.
- **`ComposeMode` enum** with `New`, `Reply`, `ReplyAll`, `Forward` — implemented.
- **Cc/Bcc toggle buttons** that disappear after activation — matches problem statement.
- **`open_compose_from_message_view()`**: Reply/ReplyAll/Forward from message view opens compose — implemented.
- **Recipient token input**: Paste handling, backspace selection, tokenization — implemented.

---

## What Diverges from Spec

### MessageViewState missing fields
- **`cc_addresses`**: Spec includes `cc_addresses: Option<String>` — not present in implementation. Code has a `// TODO: cc_addresses not yet in MessageViewState` comment in `open_compose_from_message_view()`.
- **`raw_source`**: Spec includes `raw_source: Option<String>` for Source rendering mode — not in implementation.
- **`rendering_mode`**: Spec includes `rendering_mode: RenderingMode` field — not in implementation. No `RenderingMode` enum exists.
- **`scroll_offset`**: Spec includes `scroll_offset: f32` — not in implementation.
- **`x` / `y` position fields**: Spec includes position tracking on `MessageViewState` — not in implementation. Pop-out window positions are not tracked.
- **`error_banner`**: Spec includes `error_banner: Option<String>` for best-effort restore — not in implementation.
- **`overflow_menu_open`**: Spec includes this for the overflow menu state — not in implementation.
- **`remote_content_loaded`**: Spec includes this for Original HTML mode — not in implementation.

### MessageViewMessage missing variants
- **`RawSourceLoaded`**: Not present (no source mode).
- **`SetRenderingMode`**: Not present (no rendering mode toggle).
- **`Archive`, `Delete`, `Print`, `SaveAs`, `ToggleOverflowMenu`**: Not present (no overflow menu actions).
- **`LoadRemoteContent`**: Not present (no remote content controls).

### Body rendering is plain text only
- The view uses a simple `text()` widget with `body_text` or snippet fallback. No rendering mode toggle, no HTML rendering pipeline, no monospace source view. This is acknowledged in the spec as a Phase 2 limitation, but Phases 3-6 are entirely unimplemented.

### Error handling on body load
- Spec says `BodyLoaded(Err(_))` should set an `error_banner`. Implementation just `eprintln!`s the error — no user-visible feedback.

### Compose window uses `text_editor` not rich text
- Spec and problem statement describe a rich text WYSIWYG editor with formatting toolbar. Implementation uses `iced::widget::text_editor` (plain text). Comment says "plain text for V1". No formatting toolbar present.

### Compose Send is a stub
- `ComposeMessage::Send` validates recipients exist but shows "Send not yet wired" — no actual email sending.

### Compose auto-save not implemented
- Problem statement requires auto-save every ~30 seconds. No `iced::time::every(30s)` subscription exists for compose windows.

### Compose attachment handling not implemented
- No file picker integration, no drag-and-drop zones, no attachment compression via squeeze crate, no attachment size tracking.

### `Db::load_message_body()` uses raw SQL, not body store
- The spec acknowledges this: "For the prototype, it uses the snippet as a fallback." The implementation queries `messages.snippet` directly instead of the body store (`BodyStoreState` in `crates/stores/`). This is a known prototype simplification.

---

## What's Missing (Not Implemented at All)

### Phase 3: Rendering Modes
- No `RenderingMode` enum, no rendering mode toggle UI, no plain/HTML/source switching, no monospace font for source, no remote content banner, no system-wide default rendering mode setting.

### Phase 4: Action Buttons (overflow menu)
- Reply/ReplyAll/Forward are implemented as primary buttons. The overflow menu with Archive, Delete, Print, Save As is entirely missing. No popover menu.

### Phase 5: Session Restore
- No `SessionState` struct, no `session.json`, no `save_session_state()` on close, no pop-out window restoration on launch, no migration from `window.json` to `session.json`. The main window saves its geometry via `window.json` but pop-out windows are ephemeral.

### Phase 6: Save As (.eml, .txt)
- No `rfd` integration, no file picker, no `.eml` or `.txt` export.

### Compose window features from problem statement
- **Signature insertion**: Not implemented.
- **Quoted content (attribution line)**: Partially implemented — quoted body is prefixed with `> ` but no "On [date], [name] wrote:" attribution line.
- **Draft persistence**: No draft saving to the drafts folder.
- **Discard confirmation**: Discard immediately closes the window. No "unsaved content" prompt.
- **Formatting toolbar**: Not implemented.
- **Emoji picker**: Not wired to compose.
- **Print from compose**: Not implemented.

---

## Cross-Cutting Concern Status

### a. Generational Load Tracking
**Not used for pop-out windows.** The main app uses `nav_generation` and `thread_generation` for staleness detection on navigation and thread loads. Pop-out window data loads (`BodyLoaded`, `AttachmentsLoaded`) carry no generation counter. The spec mentions "A generation counter per pop-out window prevents stale responses" but this is not implemented. In practice, the window ID in the `PopOut(window_id, ...)` message provides implicit staleness protection (if the window is closed, the message is dropped in `handle_pop_out_message` because the key is missing from the map), but there is no protection against interleaved loads if the same window were to reload.

### b. Component Trait
**Pop-out windows do not implement the `Component` trait.** The main app components (`Sidebar`, `ThreadList`, `ReadingPane`, `Settings`, `StatusBar`) all implement `Component` with `Message`, `Event`, `update()`, `view()`, and `subscription()`. Pop-out windows use free functions (`view_message_window()`, `view_compose_window()`, `update_compose()`, `handle_message_view_update()`) with routing in `App::handle_pop_out_message()`. This is a different architectural pattern — functional rather than trait-based. It works but is inconsistent with the rest of the codebase.

### c. Token-to-Catalog Theming (Named Style Classes)
**Correctly used.** Pop-out windows use named style classes throughout: `theme::ContainerClass::Content`, `theme::ContainerClass::Elevated`, `theme::TextClass::Tertiary`, `theme::ButtonClass::Ghost`, `theme::ButtonClass::Primary`, `theme::PickListClass::Ghost`. This matches the codebase convention. No raw color values or inline styles.

### d. iced_drop Drag-and-Drop
**Not implemented.** The problem statement describes a two-zone drag-and-drop overlay for compose attachments (inline vs. attachment). No `iced_drop`, `FilesHovered`, or `FilesDropped` handling exists in the pop-out code. Attachment drag-and-drop is entirely unimplemented.

### e. Subscription Orchestration
**No pop-out-specific subscriptions exist.** Pop-out windows do not have their own subscriptions. They rely on the global keyboard subscription (for Escape handling) and the global window event subscriptions (resize, move, close). The compose window has no auto-save timer subscription. The spec mentions `iced::time::every(30s)` for draft auto-save — this is absent. The spec's rendering mode toggle for Source mode implies lazy loading via tasks (not subscriptions), which is the correct pattern but is also unimplemented.

### f. Core CRUD Bypassed (Raw SQL)
**Yes, raw SQL is used in the app crate's `Db` module.** `load_message_body()` runs `SELECT snippet FROM messages WHERE account_id = ?1 AND id = ?2` directly. `load_message_attachments()` runs a raw `SELECT` on the `attachments` table. These are in `crates/app/src/db/connection.rs`, not in `crates/core/`. Per the CLAUDE.md architecture: "Business logic belongs in `ratatoskr-core`. The app crate calls core functions directly." These DB queries should live in core, not the app crate. The body load should also use `BodyStoreState` from `crates/stores/` rather than querying the `messages` table directly for snippets.

### g. Dead Code
**No significant dead code found.** All implemented pop-out code is reachable:
- `MessageViewMessage::Reply/ReplyAll/Forward` are wired to `open_compose_from_message_view()`.
- `MessageViewMessage::Noop` is matched in `handle_message_view_update()`.
- `MessageViewMessage::BodyLoaded/AttachmentsLoaded` are dispatched from `open_message_view_window()`.
- `ComposeMessage` variants are all handled in `update_compose()`.
- `ComposeState::new_reply()` is called from `open_compose_from_message_view()`.

The `body_html` field on `MessageViewState` is populated by `BodyLoaded` but never read by `message_view_body()` (which only uses `body_text` or `snippet`). This is inert data, not dead code per se, but it is unused.

---

## Summary

| Spec Phase | Status |
|---|---|
| Phase 1: Multi-Window Architecture | Complete |
| Phase 2: Message View Window | Mostly complete (missing cc, error banner, some state fields) |
| Phase 3: Rendering Modes | Not started |
| Phase 4: Action Buttons (overflow) | Not started (primary buttons done) |
| Phase 5: Session Restore | Not started |
| Phase 6: Save As | Not started |
| Compose Window (separate spec) | Partially implemented (UI shell, no sending/drafts/attachments/rich text) |
