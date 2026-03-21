# Rich Text Editor: Spec vs Implementation Discrepancies

Audit date: 2026-03-21. Updated after implementation pass.

---

## What matches the spec

These elements are implemented and match the architecture document:

- **Document model** — `Document`, `Block` (all 6 variants: Paragraph, Heading,
  ListItem, BlockQuote, HorizontalRule, Image), `StyledRun`, `InlineStyle`
  (bitflags), `DocPosition`, `DocSelection`, `DocSlice` with `open_start`/
  `open_end`. Arc structural sharing on `Document.blocks`. All exactly as
  specified.

- **Flat list model** — `Block::ListItem { ordered, indent_level, runs }` with
  cursor-addressable items, indent tracking, and HTML serializer reconstructing
  `<ul>`/`<ol>` nesting. Matches spec.

- **EditOp** — All 9 variants present: `InsertText`, `DeleteRange`, `SplitBlock`,
  `MergeBlocks`, `ToggleInlineStyle`, `SetBlockType`, `SetBlockAttrs`,
  `InsertBlock`, `RemoveBlock`. Each has apply + invert. `DeletedContent`
  captures full block structure for undo.

- **PosMap** — `PosMapEntry { old_offset, old_len, new_len }` and
  `StructuralChange` enum with `Split`, `Merge`, `Insert` (with `count`
  field), `Remove`, `CrossBlockDelete` variants. Matches spec.

- **Normalization** — Slate-inspired dirty-block normalization with safety valve
  (42x multiplier). Both `normalize()` and `normalize_blocks()` entry points.
  All 6 invariants enforced. Recursive for BlockQuote children. Matches spec.

- **Rules engine** — `resolve()` dispatching to `resolve_insert`,
  `resolve_delete_backward`, `resolve_delete_forward`, `resolve_delete_selection`,
  `resolve_split_block`, `resolve_toggle_style`. All insert rules (selection
  replacement, style inheritance, link boundary exclusivity, pending style,
  heading reset, block preserve, auto-exit, image embed), delete rules, and
  format rules listed in the spec are present.

- **Undo stack** — `UndoGroup` with ops + cursor bookmarks. Merge heuristic for
  consecutive InsertText. `force_new_group` flag. Redo cleared on new push. Max
  entries with oldest eviction. Matches spec.

- **HTML serialization** — `to_html()` with list grouping, consistent inline
  nesting order, HTML escaping. Matches spec.

- **HTML parsing** — html5ever `TreeSink` implementation in `html_parse/dom.rs`.
  Block/inline classification, `StyleContext` accumulation (like frostmark's
  `ChildData`), whitespace collapsing. Handles `<img>` with src/alt/width/height.
  Matches spec.

- **Compose assembly** — `assemble_compose_document()` with signature insertion
  (HR separator + parsed signature blocks), reply attribution (italic), forward
  headers, blockquote wrapping. Blank signature detection. Matches spec.

- **Widget** — Custom `Widget` trait implementation (not a TextEditor fork).
  `EditorState` / `RichTextEditor` / `Action` / `WidgetState` architecture as
  specified. Builder pattern for font/colors/padding/dimensions. `perform()`
  central dispatch.

- **ParagraphCache** — One entry per block with dirty flags and y-offsets. Present
  and functional.

- **Input handling** — `map_key_event()` with standard desktop bindings. `KeyAction`
  enum. `MoveAction` enum with all 10 variants (Left, Right, Up, Down, Home, End,
  WordLeft, WordRight, DocumentStart, DocumentEnd). Word boundary detection with
  three-class heuristic. Matches spec.

- **Scrolling** — Mouse wheel (Lines + Pixels), auto-scroll on cursor movement
  via `grapheme_pixel_position()`, scroll offset clamping, `with_layer()` +
  `with_translation()` rendering, drag auto-scroll. Matches spec.

- **Structured clipboard** — `InternalClipboard` with `DocSlice` + plain-text
  fingerprint for round-trip detection. Matches spec.

- **Crate structure** — Feature-gated widget module, pure-Rust core modules.
  `html_parse` is a subdirectory (`mod.rs` + `dom.rs`). `editor_state.rs` is a
  separate file from `widget/mod.rs`. Both correctly reflected in the architecture
  doc.

- **Double/triple click** — `WidgetState.last_click` tracks click state for
  word selection (double-click) and block selection (triple-click) using iced's
  `Click` type with `kind()` detection. `Action::DoubleClick` and
  `Action::TripleClick` variants handled by `EditorState::perform()`.

- **SetBlockAttrs** — `EditOp::SetBlockAttrs` for block-level attributes
  (alignment, indent level). Currently wired for `indent_level` on `ListItem`
  blocks. `TextAlignment` enum defined for future use. Self-inverse (swap
  old/new). Tested.

---

## Remaining items

### 1. `prepare_move_up()` / `prepare_move_down()` not wired into widget event path

These public functions exist in `widget/cursor.rs` and are tested. They provide
pixel-precise vertical cursor movement across block boundaries. Currently,
vertical movement uses a simpler column-offset fallback in
`EditorState::apply_move()`.

**Why not wired:** Wiring requires the widget's keyboard handler to intercept
Up/Down events at the render layer (where the paragraph cache is available),
determine `is_first_line` / `is_last_line` from the cached paragraph, and then
hit-test the target block. This is a significant refactoring of the move
dispatch path — currently, all movement goes through `Action::Move` →
`EditorState::perform()` which has no renderer access.

**Status:** Infrastructure ready. The functions have doc comments explaining
their status. The column-offset fallback works correctly for email-length
documents but doesn't handle wrapped lines within a single block.

### 2. `TextAlignment` not yet stored on block variants

`TextAlignment` enum and `BlockAttrs.alignment` field are defined, but no
`Block` variant currently stores alignment. `SetBlockAttrs` accepts alignment
in its `BlockAttrs` but only `ListItem.indent_level` is actually persisted.
Adding alignment storage requires adding an `alignment` field to `Paragraph`,
`Heading`, and `ListItem` variants — a straightforward but broad change.

---

## Cross-Cutting Concerns

### a. Generational load tracking

**Not used.** The editor has no async loading operations that would need
generation counters. The `ParagraphCache` uses a pessimistic `mark_all_dirty()`
strategy on every `layout()` call (line 477 of `widget/render.rs`), with a code
comment noting that a "generation counter or per-block dirty tracking" could
replace this for very long documents. This is a performance optimization
opportunity, not a correctness issue. For typical email-length documents (tens
of blocks), the current approach is adequate.

### b. Component trait

**Not used.** The editor is a standalone `Widget` trait implementation, not an
iced `Component`. This is the correct design choice -- `Component` is for
self-contained widgets with their own internal message type, while the editor
emits `Action`s that the host app processes via `EditorState::perform()`. The
host app owns all state mutation, which is the standard iced pattern for complex
widgets.

### c. Token-to-Catalog theming

**Not used.** The widget accepts colors directly via builder methods
(`text_color()`, `link_color()`, `cursor_color()`, `selection_color()`) rather
than using iced's `Catalog`/`StyleSheet` pattern with named style classes. Font
sizes are hardcoded constants in `render.rs` (H1=18, H2=16, H3=14, body=13).
The widget impl takes `iced::Theme` as a generic parameter but does not read
from it (the `_theme` parameter in `draw()` at line 926 is unused). This means
the editor will not automatically adapt to theme changes -- colors must be
explicitly passed by the host app.

### d. iced_drop drag-and-drop

**Not applicable.** The editor does not use `iced_drop` for drag-and-drop of
external content (files, images). Internal drag selection (click-drag to select
text) is implemented via mouse event handling in the Widget's `update()` method
with `DragState` tracking. External DND (dropping attachments or images into the
editor) is not implemented and would be an app-level concern.

### e. Subscription orchestration

**Not used.** The editor does not create or manage any iced subscriptions. Cursor
blink timing is handled via `shell.request_redraw_at()` in the widget's event
handler (lines 244-249 of `widget/mod.rs`), which is the correct approach for
periodic widget-internal redraws. No background tasks, polling, or external event
sources are needed.

### f. Core CRUD bypassed

**Not applicable.** The editor crate is a pure UI component with no database or
network access. It operates on in-memory `Document` types. The compose assembly
module (`compose.rs`) accepts HTML strings and returns `Document` objects --
all I/O (loading drafts, saving to DB, fetching signatures) is the app crate's
responsibility.

### g. Dead code

One instance remains:

1. **`prepare_move_up()` / `prepare_move_down()`** in `widget/cursor.rs` —
   public functions with tests, but never called from the widget. The widget
   uses a simpler column-preserving fallback for vertical movement instead.
   These are intentionally retained as infrastructure for pixel-precise
   vertical movement — see "Remaining items" above.

No `#[allow(dead_code)]` or `#[allow(unused)]` attributes found in the crate.
