# Rich Text Editor: Spec vs Implementation Discrepancies

Audit date: 2026-03-21. Compared `docs/editor/architecture.md` and
`docs/editor/research-summary.md` against actual code in
`crates/rich-text-editor/`.

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

- **EditOp** — All 8 variants present: `InsertText`, `DeleteRange`, `SplitBlock`,
  `MergeBlocks`, `ToggleInlineStyle`, `SetBlockType`, `InsertBlock`,
  `RemoveBlock`. Each has apply + invert. `DeletedContent` captures full block
  structure for undo.

- **PosMap** — `PosMapEntry { old_offset, old_len, new_len }` and
  `StructuralChange` enum with `Split`, `Merge`, `Insert`, `Remove`,
  `CrossBlockDelete` variants. Matches spec.

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
  `html_parse` is a subdirectory (mod.rs + dom.rs) rather than a single file,
  but this is a minor organizational detail. Matches spec intent.

---

## Discrepancies

### 1. Architecture doc says `draw_list_marker()` is "not wired into the runtime draw path yet" -- but it IS wired in

The architecture doc (line 383-385) states: "`draw_list_marker()` exists but is
not wired into the runtime draw path yet (lists currently render as a combined
placeholder paragraph)."

**Actual code:** `draw_list_marker()` is called at `widget/mod.rs:529` in the
main draw loop. Lists render with proper bullet/number markers, not as
placeholder paragraphs. The doc is stale.

### 2. Test count is higher than documented

Architecture doc claims "428 tests." Actual count is **652 tests** (grep for
`#[test]` across all source files in the crate). The doc has not been updated to
reflect tests added since the architecture was written.

### 3. `StructuralChange::Insert` has a `count` field not in the spec

The architecture doc shows `Insert { block_index }`. The actual code has
`Insert { block_index: usize, count: usize }`, supporting multi-block insert
tracking. Minor divergence; code is more capable than spec.

### 4. `_last_click` field is dead code

`WidgetState._last_click: Option<Click>` is prefixed with underscore and
annotated "(future use)" in the comment at `widget/mod.rs:76-77`. It is
initialized to `None` at line 818 and never read or written afterward.
Double/triple click detection is not implemented.

### 5. `prepare_move_up()` / `prepare_move_down()` are unused

These public functions exist in `widget/cursor.rs` (lines 407, 455) and are
tested, but are never called from the widget's `update()` method. The
architecture doc acknowledges this: "These helpers exist and are tested, but the
widget currently uses a simpler adjacent-block fallback." The functions are
effectively dead code in the runtime path.

### 6. `html_parse` is a module directory, not a single file

Architecture doc lists `html_parse.rs` as a single file. Actual structure is
`html_parse/mod.rs` + `html_parse/dom.rs` (TreeSink implementation separated
into its own file). Functionally equivalent; doc filename is slightly misleading.

### 7. `editor_state.rs` is a separate file not listed in the crate structure

The architecture doc's crate structure section shows `widget/mod.rs` as owning
"EditorState, Action, RichTextEditor widget (Widget trait impl)." In reality,
`EditorState` and `Action` live in `widget/editor_state.rs`, a separate file.
The widget's `Widget` trait impl remains in `widget/mod.rs`. This is a clean
separation but the doc doesn't reflect it.

### 8. `SetBlockAttrs` operation noted as "missing" in spec -- still missing

Architecture doc (line 178-179) notes: "`SetBlockAttrs` for block-level
attributes that aren't type changes (text alignment, list indent level). Add
when implementing alignment or indentation." This operation has not been added.
This is expected (documented as deferred).

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

Three instances identified:

1. **`_last_click: Option<Click>`** in `WidgetState` -- initialized to `None`,
   never used. Reserved for future double/triple click detection.

2. **`prepare_move_up()` / `prepare_move_down()`** in `widget/cursor.rs` --
   public functions with tests, but never called from the widget. The widget
   uses a simpler column-preserving fallback for vertical movement instead.

3. **`split_runs_at_char_offset()`** in `document.rs` -- public wrapper for
   the private `split_runs_at()`. Called from `editor_state.rs` (line 784) for
   structured paste, so this is actually live code. Not dead.

No `#[allow(dead_code)]` or `#[allow(unused)]` attributes found in the crate.
The `_last_click` field uses the underscore prefix convention to suppress the
unused field warning.
