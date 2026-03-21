# Rich Text Editor Architecture

WYSIWYG rich text editor for email composition in iced. Built from scratch — no
existing rich text editor exists for iced.

Design informed by deep study of ProseMirror (schema + transactions + position
mapping), Slate.js (normalization + path-based addressing + operation
invertibility), Quill (delta algebra), and fleather (native Flutter editor — the
only reference project that solves rendering + input on a declarative UI framework
without contentEditable). See `docs/editor/research-summary.md` for detailed
analysis of all four.

**Crate:** `crates/rich-text-editor/` — 14,300+ lines, 680+ tests, zero clippy
warnings. Pure-Rust core modules (no iced dependency) + feature-gated widget.

---

## Document Model

**Status: fully implemented.**

Block tree with inline runs. Maps naturally to both HTML (DOM) and iced rendering
(column of `Paragraph` widgets).

```
Document
  blocks: Vec<Arc<Block>>       // Arc for structural sharing

Block
  Paragraph  { runs: Vec<StyledRun> }
  Heading    { level: HeadingLevel, runs: Vec<StyledRun> }
  ListItem   { ordered: bool, indent_level: u8, runs: Vec<StyledRun> }
  BlockQuote { blocks: Vec<Arc<Block>> }
  HorizontalRule
  Image      { src: String, alt: String, width: Option<u32>, height: Option<u32> }

StyledRun
  text: String
  style: InlineStyle
  link: Option<String>          // href

InlineStyle (bitflags)
  BOLD
  ITALIC
  UNDERLINE
  STRIKETHROUGH
```

Note: `link` is a field on `StyledRun`, not on `InlineStyle`. Links are
semantically different from formatting flags, and keeping them separate
simplifies the `same_formatting()` check used by normalization.

**Flat list model:** Lists use `Block::ListItem` with `indent_level` rather
than a nested `Block::List { items: Vec<ListItem> }` container. This makes
every list item cursor-addressable (it has runs, like `Paragraph`), enabling
direct editing, splitting, merging, and the auto-exit rule. The HTML
serializer reconstructs `<ul>`/`<ol>` nesting from consecutive `ListItem`
blocks by tracking indent_level transitions. This matches what Gmail, Outlook,
and most web editors do internally.

### Immutability and structural sharing

`Document.blocks` is `Vec<Arc<Block>>`. After an edit, only the affected block
gets a new `Arc` allocation. Unchanged blocks are `Arc::clone` — cheap pointer
copies. `BlockQuote.blocks` also uses `Arc<Block>`. `ListItem` is a flat
inline block (no nested `Arc`).

### Normalization invariant

Adjacent `StyledRun`s within the same block must have different `(style, link)`
pairs. After every edit, adjacent runs with identical formatting merge. This
keeps run counts small and makes boundary operations predictable.

### Cursor and selection

```
DocPosition
  block_index: usize
  offset: usize               // char offset in block's flattened text

DocSelection
  anchor: DocPosition          // where selection started
  focus: DocPosition           // where the caret visually is
```

Flattened char offsets (not `(run_index, char_offset)`) — stable across run
restructuring. `DocPosition` implements `Ord` for range comparisons.
`DocSelection` provides `start()`, `end()`, `is_collapsed()`, `block_range()`.

### DocSlice (clipboard)

```
DocSlice
  blocks: Vec<Block>
  open_start: bool
  open_end: bool
```

Used for cross-block copy/paste. `Document::slice(start, end)` extracts a
slice from a selection. `DocSlice::inline_fragment(runs)` creates a
single-block open-ended fragment.

### Key helpers on document types

- `StyledRun::split_at(char_offset) -> (StyledRun, StyledRun)`
- `isolate_runs(runs, start, end) -> Range<usize>` — split runs at boundaries
  for surgical style application
- `Block::resolve_offset(offset) -> Option<(run_index, offset_in_run)>`
- `Block::flattened_text()`, `Block::char_len()`, `Block::kind() -> BlockKind`
- `Document::slice()`, `Document::clamp_position()`, `Document::end_position()`

---

## Editing Operations

**Status: fully implemented. All 9 variants with apply + invert.**

Operation-based for undo/redo, not patch-based. Every user action creates an
`EditOp` that knows how to apply and reverse itself. Each `apply()` returns a
`PosMap` describing what shifted. Each `invert()` returns the exact inverse
operation.

```
EditOp
  InsertText       { position, text }
  DeleteRange      { start, end, deleted: DeletedContent }
  SplitBlock       { position }
  MergeBlocks      { block_index, saved: Block, merge_offset: usize }
  ToggleInlineStyle { start, end, style_bit }
  SetBlockType     { block_index, old: BlockKind, new: BlockKind }
  SetBlockAttrs    { block_index, old: BlockAttrs, new: BlockAttrs }
  InsertBlock      { index, block }
  RemoveBlock      { index, saved: Block }

DeletedContent
  blocks: Vec<Block>    // full run structure preserved for undo
```

### Implementation notes

- `MergeBlocks` stores `merge_offset` (char length of the block being merged
  into). This allows `invert()` to produce a correct `SplitBlock` without
  needing the document.
- `DeleteRange` undo uses a sentinel pattern: `invert()` always produces
  `DeleteRange { start, end: start, deleted }` regardless of single-block or
  cross-block. `apply()` detects start == end with non-empty deleted content
  and routes to `apply_restore_deleted()`, which reconstructs the original
  block structure including all run styling and links.
- `ToggleInlineStyle` uses `isolate_runs()` for surgical run splitting. Checks
  `all_text_has_style()` to decide add vs. remove. Self-inverse.
- Cross-block `DeleteRange` truncates the start block, removes middle blocks,
  and appends the end block's tail. The inverse reconstructs all blocks.

### Position mapping (PosMap)

```
PosMap
  block_index: usize
  entries: Vec<PosMapEntry>               // char-level changes
  structural: Option<StructuralChange>    // block-level changes

PosMapEntry { old_offset, old_len, new_len }

StructuralChange
  Split   { block_index, split_offset }
  Merge   { block_index, merge_offset }
  Insert  { block_index, count }
  Remove  { block_index }
  CrossBlockDelete { start_block, removed_count, start_offset }
```

`PosMap::map(pos)` applies structural changes first, then char-level entry
mapping. Split remaps positions after the split offset into the new block.
Merge adds merge_offset when collapsing positions. CrossBlockDelete collapses
positions in deleted blocks to the deletion point.

### SetBlockAttrs

`SetBlockAttrs` modifies block-level attributes (alignment, indent level)
without changing the block type. Currently wired for `indent_level` on
`ListItem` blocks. `TextAlignment` is defined but not yet stored on block
variants (future extension). Self-inverse (swap old/new).

### Undo stack

**Status: fully implemented.**

`Vec<UndoGroup>` where each `UndoGroup` is a batch of `EditOp`s from one
logical user action, plus cursor positions before/after.

Consecutive `InsertText` at adjacent positions merge into one group. A new
group starts on: pause (`break_group()`), format change, cursor jump, or
different operation type. Redo stack cleared on new push. Max 100 entries with
oldest eviction.

`map_cursors(&PosMap)` maps stored cursor bookmarks through edits
(infrastructure in place, delegating to `PosMap::map`).

### Format toggle logic

**With selection:** `ToggleInlineStyle` operation — walks blocks, uses
`isolate_runs()` to split at boundaries, flips the style bit. If all text
already has the style, removes it; otherwise adds it. Normalization merges
same-style runs afterward.

**Without selection (caret):** Toggles `pending_style` flag on `EditorState`.
On the next `InsertText`, the rules engine compares the desired style (from
pending_style or cursor context) against the run at the insertion point. If
they differ, it emits `ToggleInlineStyle` ops after the insert to correct the
styling. Pending style is cleared after the edit.

---

## Normalization

**Status: fully implemented.**

Slate-inspired dirty-block normalization with safety valve
(max iterations = dirty_count × 42).

Two entry points:
- `normalize(doc)` — normalize entire document
- `normalize_blocks(doc, dirty_indices)` — fast path, most edits dirty 1–2 blocks

Invariants enforced:
1. Adjacent `StyledRun`s with identical `(style, link)` merge
2. Empty runs removed (but keep one empty run per inline block for cursor anchoring)
3. Every inline block has ≥1 run
4. Every `ListItem` has ≥1 run (same as Paragraph/Heading)
5. Every `BlockQuote` has ≥1 block
6. Document has ≥1 block

Normalization is recursive for container blocks (BlockQuote children).
`ListItem` normalizes the same as `Paragraph` (inline runs).

---

## Heuristic Rules Engine

**Status: mostly implemented. See gaps below.**

Chain of responsibility pattern. `rules::resolve(doc, selection, action,
pending_style) -> Vec<EditOp>` dispatches to per-action resolvers.

### Insert rules

| Rule | Status | Notes |
|------|--------|-------|
| Insert replaces selection | Done | Emits DeleteRange before InsertText |
| Inline style inheritance | Done | `resolve_style_at()` with left-affinity heuristic |
| Link boundary exclusivity | Done | At link edges, style resolves to non-link neighbor |
| Pending style override | Done | Emits ToggleInlineStyle after InsertText if style differs |
| Heading reset on split at end | Done | SplitBlock + SetBlockType to Paragraph |
| Preserve block style on split | Done | SplitBlock preserves heading/paragraph type |
| Auto-exit block | Done | Enter on empty ListItem → SetBlockType to Paragraph |
| Block embed isolation | Done | Enter on Image → SplitBlock rules in rules.rs |

### Delete rules

| Rule | Status |
|------|--------|
| Delete selection first | Done |
| Backspace at block start merges | Done |
| Delete forward at block end merges | Done |
| Merge preserves first block's type | Done |
| Backspace at document start is no-op | Done |
| Document minimum (≥1 block) | Done |
| Block embed protection | Done | Backspace/delete at Image boundary → RemoveBlock (rules.rs) |

### Format rules

| Rule | Status | Notes |
|------|--------|-------|
| Toggle with selection → ToggleInlineStyle | Done | |
| Toggle at caret → pending style | Done | |
| Link formatting at caret | Done | `find_link_boundaries` in rules.rs, tested at lines 1627-1779 |
| Line vs inline scope | Done | ToggleInlineStyle only applies to inline blocks |

---

## HTML Serialization

**Status: fully implemented.**

### Document → HTML

Recursive walk (~140 lines). Consistent nesting order:
`<a><strong><em><u><s>text</s></u></em></strong></a>`

```
Paragraph  → <p>{runs}</p>
Heading(n) → <h{n}>{runs}</h{n}>
List(ord)  → <ol>/<ul> with <li> children
BlockQuote → <blockquote>{children}</blockquote>
HRule      → <hr>
```

HTML escaping for `&`, `<`, `>`, `"` in both text content and href attributes.
Empty runs skipped.

### HTML → Document

Parse with html5ever via custom `TreeSink` implementation (Rc<RefCell<Node>>
handles). Recursive DOM walk with `StyleContext` accumulating inline styles.

- Block elements: `<p>`, `<h1>`–`<h6>` (H4-H6 → H3), `<ul>`, `<ol>`, `<li>`,
  `<blockquote>`, `<div>`, `<hr>`, `<pre>`
- Inline elements: `<strong>`/`<b>`, `<em>`/`<i>`, `<u>`, `<s>`/`<strike>`/`<del>`, `<a>`
- Tables and complex layouts flatten to text paragraphs
- Unknown block elements → recurse; unknown inline → pass through content
- Whitespace collapsing (runs of whitespace → single space)
- 37 tests including round-trip tests against html_serialize output

**Scope is narrow.** Only handles the editor's own HTML subset (drafts,
signatures, reply-quoted content). Arbitrary wild HTML is rendered by
`litehtml-rs`, a separate pure-Rust HTML rendering pipeline.

---

## Widget

**Status: implemented with known limitations.**

Custom `Widget` trait implementation for iced. Not a `TextEditor` fork.

### Architecture

The widget follows iced's ownership pattern:

- `EditorState` — application-owned mutable state (Document, selection,
  undo stack, pending style, cursor state, drag state)
- `RichTextEditor<'a, Message>` — the widget struct, created in `view()` with
  `&'a EditorState`. Builder pattern for font, colors, padding, dimensions.
- `Action` — events emitted by the widget to the application
- `WidgetState` — internal tree state holding `ParagraphCache` and focus timing

The application calls `EditorState::perform(action)` in its `update()` to
apply each action.

### EditorState

```rust
pub struct EditorState {
    pub document: Document,
    pub selection: DocSelection,
    pub undo_stack: UndoStack,
    pub pending_style: InlineStyle,
    cursor: CursorState,
    drag: Option<DragState>,
}
```

Key methods:
- `perform(action: Action)` — central dispatch for all editor actions
- `apply_action(EditAction)` — resolve through rules, apply ops, normalize,
  push to undo
- `undo()` / `redo()` — apply inverse/forward ops, restore cursor
- `to_html()` / `from_html()` / `selection_text()`

### Action enum

```rust
pub enum Action {
    Edit(EditAction),
    Move(MoveAction),
    Select(MoveAction),
    SelectAll,
    Undo, Redo,
    Copy, Cut, Paste(String),
    Click(DocPosition),      // resolved by widget via Paragraph::hit_test
    DoubleClick(DocPosition), // select word at position
    TripleClick(DocPosition), // select block at position
    Drag(DocPosition),
    LinkClicked(String),
    Focus, Blur,
}
```

### Rendering (widget/render.rs)

- `ParagraphCache<P>` — one `CacheEntry` per block with dirty tracking and
  y-offsets. Only dirty entries re-layout via `Paragraph::with_spans()`.
- `build_spans_for_block()` — converts StyledRuns to iced Spans with correct
  font (bold/italic/bold-italic variants), size, color, underline, strikethrough
- Font sizes: H1 = 18px, H2 = 16px, H3 = 14px, body = 13px
- Block spacing, blockquote border/indent, list marker width constants
- Drawing helpers: `draw_horizontal_rule()`, `draw_blockquote_border()`,
  `draw_paragraph()`, `draw_list_marker()`. Lists render with proper
  bullet/number markers via `draw_list_marker()` called in the main draw loop.

### Hit testing and cursor (widget/cursor.rs)

- `BlockLayout` — per-block layout info (y_offset, height, content_offset)
- `CursorState` — blink timing (500ms), focus tracking, target_x for vertical
  movement
- `DragState` — anchor position and active flag
- `hit_test()` / `find_block_at_point()` — binary search blocks by y_offset,
  translate to block-local coordinates
- `selection_block_ranges()` — decompose selection into per-block participation
  (Single/First/Full/Last)
- `prepare_move_up()` / `prepare_move_down()` — infrastructure for
  pixel-precise vertical cursor movement with cross-block boundary handling.
  These helpers are tested and ready to wire. Currently, vertical movement
  uses a simpler column-offset fallback in `EditorState::apply_move()`.
  Wiring requires the widget to intercept Up/Down at the render layer where
  the paragraph cache is available.

Hit testing in the Widget's `update()` method uses `ParagraphCache::block_at_y()`
to find the clicked block, then `Paragraph::hit_test()` on the cached paragraph
to get the character offset. Block-type-specific x-offsets (list marker width,
blockquote indent) are accounted for.

### Input handling (widget/input.rs)

- `map_key_event(key, modifiers, text) -> KeyAction` — central key binding
  dispatch. Standard desktop bindings: arrows (±Shift ±Ctrl), Home/End,
  Backspace/Delete, Enter, Ctrl+B/I/U, Ctrl+C/X/V, Ctrl+Z/Y/Shift+Z, Ctrl+A
- `KeyAction` enum: Edit, Move, Select, SelectAll, Copy, Cut, Paste, Undo, Redo
- `MoveAction` enum: Left, Right, Up, Down, Home, End, WordLeft, WordRight,
  DocumentStart, DocumentEnd
- Cursor movement helpers: `move_left()`, `move_right()`, `word_left()`,
  `word_right()`, `home()`, `end()`, `document_start()`, `document_end()`
- Word boundary detection: three-class heuristic (word chars, whitespace, other)

### Widget trait implementation

- `layout()` — uses ParagraphCache to compute block paragraph layouts, clamps
  scroll offset, auto-scrolls to keep cursor visible
- `draw()` — renders via `renderer.with_layer()` (clipping) +
  `renderer.with_translation()` (scroll offset). Draws paragraphs, HR/blockquote
  decorations, list markers (bullet/number per item), per-line selection
  rectangles with precise start/end x-coordinates, and blinking cursor at exact
  grapheme position.
- `update()` — handles window focus/blink, keyboard events (mapped via
  input::map_key_event), mouse click/drag/release with hit testing (accounts
  for scroll offset), mouse wheel scrolling (Lines + Pixels deltas)
- `mouse_interaction()` — Text cursor when hovering, NotAllowed when disabled

### Scrolling

Vertical scroll support with:
- Mouse wheel handling (ScrollDelta::Lines at 20px/line, ScrollDelta::Pixels
  for trackpad)
- Auto-scroll on cursor movement tracks actual caret line within wrapped
  paragraphs via `grapheme_pixel_position()`, not just block top
- Scroll offset clamped to `[0, max_scroll]` on resize and content changes
- Content drawn with `with_layer()` (clip) + `with_translation()` (offset).
  Cursor drawn inside the translation layer — no manual viewport clipping.
- Hit testing converts viewport-relative to content-relative coordinates
- Drag auto-scroll: proportional speed when dragging above/below viewport,
  capped at 30px/frame, continuous redraws while mouse held outside bounds

### Known limitations

- **IME:** Basic keyboard input works; no preedit/commit or platform IME
  protocol integration.
- **Vertical cursor movement precision:** Uses character offset preservation
  (target_column) across blocks, but not pixel-precise x-coordinate via
  `Paragraph::grapheme_position()` (which would need the paragraph cache in
  EditorState).
- **Drag auto-scroll:** Implemented. Scrolls proportionally to cursor distance
  from viewport edge, capped at 30px/frame, with continuous redraws.
- **Image rendering:** Images render as placeholder rectangles with alt text.
  Actual image loading (`inline-image:<hash>` resolution, data-URI decoding)
  is the app's responsibility — the editor stores src references only.

---

## Crate Structure

```
crates/rich-text-editor/
  Cargo.toml
  src/
    lib.rs                    // re-exports + feature gate
    document.rs               // Document, Block (6 variants), StyledRun, InlineStyle, DocPosition, DocSlice
    operations.rs             // EditOp, PosMap, apply/invert, run splitting helpers
    normalize.rs              // Normalization pass: merge runs, enforce structural invariants
    rules.rs                  // Heuristic rules: insert/delete/format behavior
    undo.rs                   // UndoStack, UndoGroup, cursor bookmark mapping
    html_serialize.rs         // Document → HTML (including <img>)
    html_parse/
      mod.rs                  // HTML → Document (block/inline classification, whitespace collapsing)
      dom.rs                  // html5ever TreeSink implementation
    compose.rs                // Compose document assembly: signatures, reply quoting, forward headers
    widget/
      mod.rs                  // RichTextEditor widget (Widget trait impl), click detection
      editor_state.rs         // EditorState, Action, InternalClipboard, perform() dispatch
      input.rs                // Key binding → action mapping, cursor movement helpers
      render.rs               // ParagraphCache, span building, draw helpers
      cursor.rs               // CursorState, hit testing, selection rects, vertical movement
```

**Pure Rust** (no iced dependency): `document`, `operations`, `normalize`,
`rules`, `undo`, `html_serialize`, `html_parse`. Unit-testable without a GUI.

**Feature-gated** (`widget` feature, default on): `widget/` module depends on iced.

**Dependencies:** `bitflags` 2, `html5ever` 0.35, `markup5ever` 0.16, `iced`
(optional, path dep to local fork).

### App crate integration

```rust
// In the app's state:
editor: EditorState,

// In view():
rich_text_editor(&self.editor)
    .on_action(Message::EditorAction)
    .font(font::text())
    .into()

// In update():
Message::EditorAction(action) => {
    self.editor.perform(action);
}
```

The toolbar is standard iced buttons in the app crate (not in the editor crate),
sending `Action::Edit(EditAction::ToggleInlineStyle(...))` etc.

---

## Implementation Status

### What's done (Phases 1–5 editor-side complete)

- Document model with all 6 block types (Paragraph, Heading, List, BlockQuote,
  HorizontalRule, Image), Arc structural sharing, DocSlice
- All 9 EditOp variants with correct apply, invert, and PosMap (including
  split_offset, merge_offset, start_offset on structural changes,
  SetBlockAttrs for indent level)
- Normalization with dirty tracking and safety valve
- Rules engine with insert/delete/format behavior, including pending style
  application, link boundary exclusivity, link formatting at caret, heading
  reset on split-at-end, image block embed rules (backspace/delete remove,
  Enter inserts paragraph after), auto-exit list (Enter on empty ListItem →
  Paragraph)
- Undo/redo with grouping, cursor bookmarks, and correct PosMap mapping
- HTML serialization and parsing with round-trip tests (including `<img>`
  with src, alt, width, height)
- Structured clipboard: internal copy preserves DocSlice with formatting +
  links, paste uses block-swap strategy (redo-safe), external paste falls
  back to plain text
- Compose document assembly: signature insertion/removal/replacement, reply
  attribution (italic), forward header, quoted content in BlockQuote. Blank
  signature detection, out-of-range index clamping.
- Block::Image: atomic block embed with src/alt/width/height. Widget renders
  placeholder rectangle with alt text; actual image loading is app-side.
- Flat list model: `Block::ListItem { ordered, indent_level, runs }` — each
  list item is cursor-addressable. HTML serializer reconstructs `<ul>`/`<ol>`
  nesting. HTML parser flattens into items with tracked indent_level.
- Widget: paragraph caching, exact cursor placement via grapheme_position,
  per-line selection rectangles, scrolling (wheel + auto-scroll + drag
  auto-scroll), mouse hit testing with Paragraph::hit_test
- Double-click (select word) and triple-click (select block) detection
  using iced's `Click` type with `kind()` detection
- 680+ tests across all modules

### What remains

**App-crate integration (not in the editor crate):**
- Settings UI: signature list, signature editor overlay with rich text editor
- Compose window: wire up compose document assembly, From-account signature
  switching, draft persistence
- Actual image loading: resolve `inline-image:<hash>` src URIs via the
  inline image store, render real images instead of placeholders

**Editor-crate deferred:**
- Paste HTML from external clipboard (iced's Clipboard trait only provides
  plain text; HTML clipboard needs platform-specific code)
- IME preedit/commit integration

---

## Key References

| File | What to steal |
|------|---------------|
| **JS editors** | |
| ProseMirror `prosemirror-transform/src/map.ts` | StepMap triples — informed our PosMap design |
| ProseMirror `prosemirror-model/src/replace.ts` | Slice — informed our DocSlice |
| ProseMirror `prosemirror-transform/src/mark_step.ts` | AddMarkStep — informed our ToggleInlineStyle + isolate_runs |
| Slate `packages/slate/src/editor/normalize.ts` | Dirty path tracking — adopted directly (safety valve × 42) |
| Slate `packages/slate/src/interfaces/operation.ts` | 9 invertible operations — informed our 8 EditOp variants |
| Slate `packages/slate-history/src/with-history.ts` | History batching — adopted for UndoStack grouping |
| **Flutter editors** | |
| fleather `packages/parchment/lib/src/heuristics/insert_rules.dart` | Insert rules — adopted for rules.rs |
| fleather `packages/parchment/lib/src/heuristics/delete_rules.dart` | Delete rules — adopted for rules.rs |
| fleather `packages/fleather/lib/src/rendering/editor.dart` | Cascading hit test — adopted for widget/cursor.rs |
| fleather `packages/parchment/lib/src/document/leaf.dart` | split/isolate/optimize — adopted for document.rs run splitting |
| **iced ecosystem** | |
| iced `widget/src/text_editor.rs` | Input handling, focus/blink, clipboard patterns |
| iced `widget/src/text/rich.rs` | `Paragraph::with_spans`, span→font mapping |
| `crates/app/src/font.rs` | Font variants (text_bold, text_italic, etc.) |
| `crates/app/src/ui/layout.rs` | Type scale and spacing constants |
| `crates/provider-utils/src/html_sanitizer.rs` | Sanitizer pipeline — runs before html5ever parse |
| research/frostmark `renderer.rs` | ChildData bitflags — adopted for html_parse.rs StyleContext |
| research/halloy `selectable_rich_text.rs` | Selection math reference |
| **Email rendering** | |
| `/home/folk/Programs/litehtml-rs/` | Separate HTML email viewer — editor does NOT handle arbitrary HTML |
