# Pop-Out Windows: Spec vs. Code Discrepancies

Audit date: 2026-03-23

---

## Resolved (previously open)

- Session save/restore wired (called in boot and handle_window_close)
- Save As uses rfd file picker with .eml/.txt format filters
- Compose uses rich text editor (EditorState) with formatting toolbar
- Compose Send fully implemented (MIME build + local_drafts queue)
- Compose auto-save implemented (30s subscription)
- Compose attachment handling implemented (rfd file picker + size tracking)
- Compose signature insertion wired (resolve + replace_signature on account switch)
- Db::load_message_body delegates to core message_queries
- Archive/Delete wired to core label operations
- Link insertion dialog fully implemented
- Dead code (save_session_state, restore_pop_out_windows, SessionState::load) — no longer dead
- scroll_offset dead field removed
- Body loading now uses BodyStore first (full zstd-decompressed bodies), falling back to DB snippet *(2026-03-23)*

## Remaining

### HTML rendering not used in pop-out
SimpleHtml and OriginalHtml modes fall back to plain text. Depends on DOM-to-widget pipeline. Tracked in TODO.md.

### Print not implemented
No OS print dialog integration. Tracked in TODO.md.

### Default rendering mode hardcoded
`MessageViewState` uses `RenderingMode::default()` (SimpleHtml) instead of loading from user settings. Tracked in TODO.md.
