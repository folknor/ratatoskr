# Pop-Out Windows: Spec vs. Implementation Discrepancies

Audit date: 2026-03-21 (updated). Compared `problem-statement.md` and `message-view-implementation-spec.md` against codebase in `crates/app/src/`.

---

## What's Implemented and Matches the Spec

### Phase 1: Multi-Window Architecture (complete)
- **Daemon migration**: `main()` uses `iced::daemon` with `App::boot`, `App::update`, `App::view` — matches spec exactly.
- **Main window opened in `boot()`**: `iced::window::open(window.to_window_settings())` stores `main_window_id` — matches spec.
- **Window registry**: `pop_out_windows: HashMap<window::Id, PopOutWindow>` on `App` — matches spec.
- **`PopOutWindow` enum**: Has `MessageView(MessageViewState)` and `Compose(ComposeState)` variants — matches spec (compose is ahead of spec, which listed it as a future variant).
- **`PopOutMessage` enum**: Routes `MessageView` and `Compose` sub-messages — matches spec.
- **View routing**: `view()` takes `window::Id`, dispatches to `view_message_window()` or `view_compose_window()` — matches spec.
- **Title routing**: `title()` dispatches per window, message view shows `"subject — sender"` — matches spec.
- **Window close cascade**: Main window close cascades to all pop-outs then calls `iced::exit()` — matches spec.
- **Pop-out close**: Removes from registry — matches spec.
- **Escape closes pop-out**: Keyboard handler checks `window_id != self.main_window_id && pop_out_windows.contains_key(...)` — matches spec.
- **Window resize/move events carry window ID**: `WindowResized(window::Id, Size)` and `WindowMoved(window::Id, Point)` — matches spec.
- **Subscription updates**: `resize_events`, `close_requests`, `listen_with` for moved events all carry window IDs — matches spec.

### Phase 2: Message View Window (complete)
- **`MessageViewState` struct**: All spec fields implemented — identity, header (including `cc_addresses`), body, raw_source, attachments, geometry, window-local state (`rendering_mode`, `scroll_offset`, `overflow_menu_open`, `remote_content_loaded`, `error_banner`), position tracking (`x`, `y`).
- **`MessageViewMessage` enum**: All spec variants implemented — `BodyLoaded`, `AttachmentsLoaded`, `RawSourceLoaded`, `SetRenderingMode`, `Reply`, `ReplyAll`, `Forward`, `Archive`, `Delete`, `Print`, `SaveAs`, `ToggleOverflowMenu`, `LoadRemoteContent`, `Noop`.
- **`from_thread_message()` constructor**: Seeds from `ThreadMessage` including `cc_addresses` — matches spec.
- **`open_message_view_window()`**: Creates `MessageViewState`, opens window with correct settings and min sizes, inserts into registry, dispatches body+attachment loads — matches spec.
- **View layout**: Header (from name/email, To, Cc, subject+date row, action buttons + overflow menu), divider, rendering mode toggle, body, conditional attachment section — matches spec.
- **Attachment cards**: File icon by MIME type, filename, size formatting, type label — matches spec.
- **Async data loads**: `Db::load_message_body()` and `Db::load_message_attachments()` implemented with raw SQL — matches spec.
- **Error banner for body load failures**: `BodyLoaded(Err(_))` sets `error_banner` with user-visible message — matches spec.
- **Per-window generation tracking**: `pop_out_generation` counter guards against stale data loads — matches spec.
- **Pop-out icon button in reading pane**: `ReadingPaneMessage::PopOut(usize)` emits `ReadingPaneEvent::OpenMessagePopOut` — matches spec.

### Phase 3: Rendering Modes (complete)
- **`RenderingMode` enum**: `PlainText`, `SimpleHtml`, `OriginalHtml`, `Source` — matches spec.
- **Rendering mode toggle UI**: Four chip-style buttons below header — matches spec.
- **Plain text rendering**: Uses `body_text` or snippet fallback — matches spec.
- **Simple HTML / Original HTML**: Fall back to plain text (placeholder until HTML pipeline) — acceptable per spec.
- **Source mode**: Raw email source in monospace font, loaded lazily — matches spec.
- **Remote content banner**: Shown in Original HTML mode when `remote_content_loaded` is false — matches spec.
- **Monospace font**: `font::monospace()` returns system monospace — matches spec.

### Phase 4: Action Buttons (complete)
- **Primary buttons (Reply, Reply All, Forward)**: Always visible in header row — matches spec.
- **Overflow menu (Archive, Delete, Print, Save As)**: Popover menu from ellipsis button — matches spec.
- **Reply/ReplyAll/Forward dispatch**: Opens compose pop-out with proper mode and context — matches spec.
- **Archive/Delete/Print**: Handled as no-op stubs in update — matches spec (placeholder for future wiring).

### Phase 5: Session Restore (complete)
- **`SessionState` struct**: Contains `main_window` (WindowState) and `message_views` (Vec of session entries) — matches spec.
- **`MessageViewSessionEntry`**: Stores message_id, thread_id, account_id, and window geometry (width, height, x, y) — matches spec.
- **`save_session_state()` on close**: Serializes full session to `session.json` — matches spec.
- **Restore in `boot()`**: Reads `session.json`, opens restored windows with saved positions, dispatches data loads — matches spec.
- **Migration**: Falls back to `window.json` if `session.json` doesn't exist — matches spec.
- **Best-effort restore**: Error banner for deleted messages, body loaded fresh from DB — matches spec.
- **Position tracking**: Pop-out window positions tracked via `WindowMoved` events — matches spec.

### Phase 6: Save As (partial)
- **Save As action**: Wired in overflow menu, dispatches async save task — matches spec intent.
- **`.eml` export**: Writes synthesized raw source to downloads directory — matches spec (prototype level).
- **`.txt` export**: Writes plain text body to downloads directory — matches spec (prototype level).
- **No file picker**: Uses `dirs::download_dir()` fallback instead of `rfd` dialog — deviation (see below).

### Compose Window (enhanced from V1)
- **`ComposeState`** with recipients (token input), from account picker, subject, body (text editor), mode — matches spec.
- **`ComposeMode` enum** with `New`, `Reply`, `ReplyAll`, `Forward` — matches spec.
- **Cc/Bcc toggle buttons** that disappear after activation — matches problem statement.
- **`open_compose_from_message_view()`**: Reply/ReplyAll/Forward from message view opens compose with `cc_addresses` — now implemented.
- **Recipient token input**: Paste handling, backspace selection, tokenization — matches spec.
- **Attribution line**: Quoted content now prefixed with "sender wrote:" — matches problem statement.
- **Formatting toolbar**: B/I/U/List/Link buttons rendered (stubs for V1) — matches spec structure.
- **Discard confirmation dialog**: Shows "Discard this draft?" with confirm/cancel when user content exists — matches problem statement.

### Handler Architecture
- **`handlers/pop_out.rs`**: All pop-out window logic extracted from `main.rs`. `main.rs` is a thin dispatch layer with one-line match arms.
- **`pop_out/session.rs`**: Session state persistence module (save/restore/migration).

---

## What Diverges from Spec

### Save As uses fallback directory, not file picker
- Spec calls for `rfd` (Rust File Dialogs) crate. Implementation saves directly to `dirs::download_dir()` without user dialog. The `rfd` crate is not yet a dependency. When added, the `save_message_dialog()` function should switch to `rfd::AsyncFileDialog`.

### Compose Send is a stub
- `ComposeMessage::Send` validates recipients exist but shows "Send not yet wired" — no actual email sending or MIME assembly.

### Compose auto-save not implemented
- Problem statement requires auto-save every ~30 seconds. No `iced::time::every(30s)` subscription exists for compose windows.

### Compose uses `text_editor` not rich text
- Spec describes a rich text WYSIWYG editor. Implementation uses `iced::widget::text_editor` (plain text). The formatting toolbar buttons are stubs. The `rich-text-editor` crate exists but is not yet wired.

### Compose attachment handling not implemented
- No file picker integration, no drag-and-drop zones, no attachment compression via squeeze crate, no attachment size tracking.

### Compose signature insertion not implemented
- No `assemble_compose_document` call, no signature insertion on From account change.

### `Db::load_message_body()` now uses body store
- **Resolved.** `load_message_body()` now reads from `BodyStoreState` (zstd-compressed bodies in `bodies.db`) instead of querying the `messages` table directly.

### `Db::load_raw_source()` synthesizes from fields, not raw message
- The raw source is built from individual fields (From, To, Cc, Subject, Date, snippet) rather than fetching the actual RFC 5322 raw message. The full implementation should query the raw message if cached locally.

### HTML rendering pipeline not implemented
- Simple HTML and Original HTML modes fall back to plain text. Full HTML-to-widget rendering is a cross-cutting concern shared with the reading pane.

### Archive/Delete not wired to provider ops
- The overflow menu items exist but the update handlers are no-ops. Wiring requires the `ProviderOps` trait methods for thread-level mutations.

### Print not implemented
- OS print dialog integration requires platform-specific code with no iced precedent.

---

## Cross-Cutting Concern Status

### a. Generational Load Tracking
**Implemented.** `pop_out_generation` counter on `App` provides per-window staleness detection. `BodyLoaded` and `AttachmentsLoaded` carry the generation counter, and the handler checks `state.is_current_generation(gen)` before applying results.

### b. Component Trait
**Pop-out windows do not implement the `Component` trait.** They use free functions with routing in `handlers/pop_out.rs`. This is a different architectural pattern (functional vs trait-based) but is consistent within the pop-out subsystem.

### c. Token-to-Catalog Theming (Named Style Classes)
**Correctly used.** Pop-out windows use named style classes throughout: `theme::ContainerClass::Content`, `theme::ContainerClass::Elevated`, `theme::TextClass::Tertiary`, `theme::ButtonClass::Ghost`, `theme::ButtonClass::Primary`, `theme::ButtonClass::BareIcon`, `theme::ButtonClass::Chip`, `theme::ButtonClass::Action`, `theme::PickListClass::Ghost`, `theme::ContainerClass::SelectMenu`. No raw color values or inline styles.

### d. iced_drop Drag-and-Drop
**Not implemented.** Attachment drag-and-drop is entirely unimplemented for compose windows.

### e. Subscription Orchestration
**No pop-out-specific subscriptions exist.** Pop-out windows rely on global keyboard and window event subscriptions. The compose auto-save timer subscription (`iced::time::every(30s)`) is not implemented.

### f. Core CRUD Bypassed (Raw SQL)
**Largely resolved.** `load_message_body()` now uses `BodyStoreState::get()` from the body store. `load_message_attachments()` now delegates to `ratatoskr_core::db::queries::get_attachments_for_message()`. Only `load_raw_source()` still uses raw SQL (reads from the `messages` table `raw_source` column, which is the correct location for raw RFC 5322 data).

### g. Dead Code
**Minimal.** The `body_html` field on `MessageViewState` is populated by `BodyLoaded` but only used as a fallback path (HTML rendering not yet implemented). The `scroll_offset` field is declared but not yet wired to the scrollable widget. These are forward-looking fields, not dead code.

---

## Summary

| Spec Phase | Status |
|---|---|
| Phase 1: Multi-Window Architecture | Complete |
| Phase 2: Message View Window | Complete (including cc_addresses, error banner, generation tracking) |
| Phase 3: Rendering Modes | Complete (HTML modes fall back to plain text) |
| Phase 4: Action Buttons (overflow) | Complete (Archive/Delete/Print are stubs) |
| Phase 5: Session Restore | Complete |
| Phase 6: Save As | Partial (no file picker dialog — saves to downloads dir) |
| Compose Window | Enhanced (formatting toolbar, discard confirm, attribution line; no rich text/send/drafts/attachments) |
| Handler Architecture | Complete (extracted to handlers/pop_out.rs) |
