# Rich Text Editor: Spec vs. Code Discrepancies

Audit date: 2026-03-21

---

## Divergences

### `prepare_move_up()` / `prepare_move_down()` not wired into widget event path

These public functions exist in `widget/cursor.rs:413,463` and are tested (7 tests).
They provide pixel-precise vertical cursor movement across block boundaries.
Currently, vertical movement uses a simpler column-offset fallback in
`EditorState::apply_move()`. Never called outside `cursor.rs` tests.

**Why not wired:** Wiring requires the widget's keyboard handler to intercept
Up/Down events at the render layer (where the paragraph cache is available),
determine `is_first_line` / `is_last_line` from the cached paragraph, and then
hit-test the target block. Currently, all movement goes through `Action::Move`
-> `EditorState::perform()` which has no renderer access.

**Status:** Infrastructure ready. The column-offset fallback works correctly
for email-length documents but does not handle wrapped lines within a single
block.

### `TextAlignment` not yet stored on block variants

`TextAlignment` enum (`document.rs:150`) and `BlockAttrs.alignment` field
(`document.rs:169`) are defined, but no `Block` variant stores alignment.
`Block::attrs()` returns `TextAlignment::Left` hardcoded for all block types
(`document.rs:332-338`). `Block::with_attrs()` ignores alignment for `ListItem`
(only sets `indent_level`, `document.rs:351-355`). Adding alignment storage
requires an `alignment` field on `Paragraph`, `Heading`, and `ListItem`.

---

## Cross-cutting

### Generational load tracking

**Not used.** `ParagraphCache` uses pessimistic `mark_all_dirty()` on every `layout()` call, with a code comment noting that generation-counter or per-block dirty tracking could replace this for very long documents. Performance optimization opportunity, not a correctness issue.
- Code: `crates/rte/src/widget/render.rs:477`

### Component trait

**Not used.** The editor is a standalone `Widget` trait implementation. It emits `Action`s that the host app processes via `EditorState::perform()`.
- Code: `crates/rte/src/widget/mod.rs` (Widget impl)

### Token-to-Catalog theming

**Not used.** Colors passed via builder methods. Font sizes hardcoded (H1=18, H2=16, H3=14, body=13). The `_theme` parameter in `draw()` is unused.
- Code: `crates/rte/src/widget/mod.rs:947`

### iced_drop drag-and-drop

**Not applicable.** Internal drag selection implemented via mouse event handling with `DragState` tracking. External DND not implemented.
- Code: `crates/rte/src/widget/mod.rs` (mouse handling)

### Subscription orchestration

**Not used.** Cursor blink handled via `shell.request_redraw_at()`.
- Code: `crates/rte/src/widget/mod.rs:244-249`

### Core CRUD bypassed

**Not applicable.** The editor is a pure UI component with no database or network access.
- Code: `crates/rte/src/compose.rs`

### Dead code

One instance: `prepare_move_up()` / `prepare_move_down()` -- public functions with tests, never called from the widget. Intentionally retained as infrastructure.
- Code: `crates/rte/src/widget/cursor.rs:413,463`
