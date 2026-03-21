# Pop-Out Windows: Spec vs. Code Discrepancies

Audit date: 2026-03-21

---

## Divergences

### Session save is not wired
`save_session_state()` exists but is never called. `handle_window_close()` calls `self.window.save(data_dir)` (WindowState only), not `save_session_state()`. Pop-out window positions are not persisted.
- Code: `crates/app/src/handlers/pop_out.rs:473`, `crates/app/src/main.rs:1486-1509`

### Session restore is not wired
`restore_pop_out_windows()` exists but is never called. `boot()` loads `WindowState::load(data_dir)` only, creates no pop-out windows. `SessionState::load()` is dead code.
- Code: `crates/app/src/handlers/pop_out.rs:508`, `crates/app/src/main.rs:292-362`, `crates/app/src/pop_out/session.rs:40-55`

### Save As uses fallback directory, not file picker
Spec calls for `rfd`. Implementation saves to `dirs::download_dir()` without user dialog. No `rfd` dependency.
- Code: `crates/app/src/handlers/pop_out.rs:579-611`

### Compose uses `text_editor` not rich text
`iced::widget::text_editor` used instead of `RichTextEditor`. Formatting toolbar buttons (Bold, Italic, etc.) are stubs.
- Code: `crates/app/src/pop_out/compose.rs:403-408,740`

### Compose Send is a stub
`ComposeMessage::Send` validates recipients exist but no MIME assembly or sending occurs.
- Code: `crates/app/src/pop_out/compose.rs`

### Compose auto-save not implemented
No `iced::time::every` subscription for compose windows. No draft auto-save logic.
- Code: `crates/app/src/pop_out/compose.rs`

### Compose attachment handling not implemented
No file picker, no drag-and-drop zones, no squeeze integration, no attachment size tracking.
- Code: `crates/app/src/pop_out/compose.rs`

### Compose signature insertion not implemented
No `assemble_compose_document` call or signature insertion.
- Code: `crates/app/src/pop_out/compose.rs`

### `Db::load_message_body()` uses raw SQL, not body store
Queries `SELECT body_text, body_html FROM messages`. These columns are likely null/empty since bodies live in `BodyStoreState` (zstd-compressed in `bodies.db`). No `BodyStoreState` on `App`.
- Code: `crates/app/src/db/threads.rs:248-268`

### HTML rendering not used in pop-out
SimpleHtml and OriginalHtml modes fall back to plain text. `html_render::render_html()` exists but is not called from the pop-out view.
- Code: `crates/app/src/pop_out/message_view.rs:500-506`

### Archive/Delete not wired to provider ops
Overflow menu items exist. Update handlers are no-ops.
- Code: `crates/app/src/handlers/pop_out.rs:153-161`

### Print not implemented
No OS print dialog integration.
- Code: `crates/app/src/pop_out/message_view.rs`

### Link insertion dialog does not exist
`FormatLink` message exists but maps to a no-op stub. No link dialog UI.
- Code: `crates/app/src/pop_out/compose.rs:100,406`

---

## Cross-cutting

### Generational load tracking
Implemented. `pop_out_generation` on `App`. `BodyLoaded` and `AttachmentsLoaded` carry generation counter. Stale loads discarded.
- Code: `crates/app/src/main.rs:261`, `crates/app/src/handlers/pop_out.rs:105,122`

### Component trait
Pop-out windows do not implement `Component`. They use free functions routed via `handlers/pop_out.rs`. Consistent within the pop-out subsystem.
- Code: `crates/app/src/handlers/pop_out.rs`

### Core CRUD bypass
`load_message_body()`, `load_message_attachments()`, `load_raw_source()` all use raw SQL. `load_raw_source` queries `SELECT raw_source FROM messages` (column may be null).
- Code: `crates/app/src/db/threads.rs:244-325`

### Dead code
- `save_session_state()` at `crates/app/src/handlers/pop_out.rs:473` — never called
- `restore_pop_out_windows()` at `crates/app/src/handlers/pop_out.rs:508` — never called
- `SessionState::load()` at `crates/app/src/pop_out/session.rs:40` — never called
- `body_html` field on `MessageViewState` — populated but never rendered
- `scroll_offset` field on `MessageViewState` — declared but not wired
