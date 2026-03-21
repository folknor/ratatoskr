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

### Compose Window (complete compose workflow)
- **Rich text editor**: `RichTextEditor` from `crates/rich-text-editor/` replaces `iced::widget::text_editor`. Uses `EditorState::new()` / `EditorState::from_document()`, wired through `EditorAction` / `perform()`.
- **Formatting toolbar**: B/I/U buttons emit `Action::Edit(EditAction::ToggleInlineStyle(InlineStyle::BOLD))` etc. through `EditorState::perform()`. List/Link are stubs (block-type toggle not yet exposed via `EditAction`).
- **Signature insertion**: `assemble_compose_document()` called via async `SignatureResolved` message at compose window creation. Resolves account-specific signature from DB using alias-level overrides, reply-default, and default signature resolution order.
- **Draft auto-save**: `iced::time::every(30s)` subscription active when any compose window has `draft_dirty` set. Saves to `local_drafts` table via `Db::with_write_conn`. `Message::ComposeDraftTick` variant dispatches `auto_save_compose_drafts()`.
- **Attachment handling**: Attach button in footer. Tracks `ComposeAttachment` list (name, path, size). Attachment section with file icon, name, size, remove button. File picker is stubbed (returns empty list) — `rfd` crate not yet a dependency.
- **Send path**: Validates recipients, serializes editor to HTML via `EditorState::to_html()`, calls `finalize_compose_html()` from core to wrap signature in identifying div, saves finalized draft to `local_drafts` with `sync_status = 'finalized'`. Actual provider send is a separate concern.
- **Discard confirmation**: Checks `has_user_content()` (scans blocks before signature separator, checks recipients/subject/attachments) before prompting.
- **`ComposeState`** with recipients (token input), from account picker, subject, rich text editor, mode, signature tracking, attachments, draft persistence ID — complete.
- **`ComposeMode` enum** with `New`, `Reply`, `ReplyAll`, `Forward` plus `is_reply()` helper.
- **Cc/Bcc toggle buttons** that disappear after activation.
- **`open_compose_from_message_view()`**: Reply/ReplyAll/Forward from message view opens compose with `cc_addresses`.
- **Recipient token input**: Paste handling, backspace selection, tokenization.
- **Attribution line**: Quoted content via `assemble_compose_document` with `QuotedContent` struct for rich-text blockquote.

### Handler Architecture
- **`handlers/pop_out.rs`**: All pop-out window logic extracted from `main.rs`. `main.rs` is a thin dispatch layer with one-line match arms.
- **`pop_out/session.rs`**: Session state persistence module (save/restore/migration).
- **Compose-specific handlers**: `handle_compose_send()`, `handle_compose_attach_files()`, `save_compose_draft()`, `resolve_signature_for_compose()` in `handlers/pop_out.rs`.

---

## What Diverges from Spec

### Save As uses fallback directory, not file picker
- Spec calls for `rfd` (Rust File Dialogs) crate. Implementation saves directly to `dirs::download_dir()` without user dialog. The `rfd` crate is not yet a dependency. When added, the `save_message_dialog()` function should switch to `rfd::AsyncFileDialog`.

### Compose Send saves draft, does not send via provider
- `ComposeMessage::Send` validates recipients, finalizes HTML (wraps signature in identifying div), and saves to `local_drafts` with `sync_status = 'finalized'`. Actual email sending via provider ops is a separate concern not yet wired.

### Compose attachment file picker is a stub
- The `handle_compose_attach_files()` method returns an empty file list. The `rfd` crate is not yet a dependency. When added, this will use `rfd::AsyncFileDialog`. Attachment tracking, display, and removal are fully implemented.

### Block-type format toggles are stubs
- `FormatList` and `FormatBlockquote` toolbar buttons do not yet toggle block types. The editor supports `ListItem` and `BlockQuote` block types, but the `EditAction` enum does not expose a `SetBlockType` variant. When `EditAction::SetBlockType` is added to the editor crate, these buttons will wire through.

### Link insertion is a stub
- `FormatLink` toolbar button is a no-op. Link insertion requires a URL input dialog which is not yet implemented.

### `Db::load_message_body()` uses raw SQL, not body store
- The implementation queries `messages.snippet` directly instead of the body store (`BodyStoreState` in `crates/stores/`). This is a known prototype simplification per the spec.

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
**Compose auto-save subscription implemented.** `iced::time::every(30s)` subscription fires `Message::ComposeDraftTick` when any compose window has `draft_dirty` set. Other pop-out windows rely on global keyboard and window event subscriptions.

### f. Core CRUD Bypassed (Raw SQL)
**Partially.** `load_message_body()`, `load_message_attachments()`, and `load_raw_source()` still use raw SQL in the app crate's `Db` module. Compose draft save uses raw SQL through `Db::with_write_conn` writing to the `local_drafts` table (same schema as `db_save_local_draft` in core). Signature resolution uses raw SQL matching core's `db_resolve_signature_for_compose` logic.

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
| Compose: Rich Text Editor | Complete (RichTextEditor wired, formatting toolbar, EditorState/Action/perform) |
| Compose: Signature Insertion | Complete (assemble_compose_document, async DB resolution, alias overrides) |
| Compose: Draft Auto-Save | Complete (30s subscription, local_drafts table, dirty tracking) |
| Compose: Attachments | Partial (tracking/display/remove done, file picker stubbed — rfd not a dep) |
| Compose: Send Path | Partial (finalize HTML + save draft; provider send not wired) |
| Compose: Discard Confirmation | Complete (checks has_user_content before prompting) |
| Handler Architecture | Complete (extracted to handlers/pop_out.rs) |
