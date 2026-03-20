# Rich Text Editor Architecture

WYSIWYG rich text editor for email composition in iced. Built from scratch — no
existing rich text editor exists for iced.

Design informed by deep study of ProseMirror (schema + transactions + position
mapping), Slate.js (normalization + path-based addressing + operation
invertibility), Quill (delta algebra), and fleather (native Flutter editor — the
only reference project that solves rendering + input on a declarative UI framework
without contentEditable). See `docs/editor/research-summary.md` for detailed
analysis of all four.

**Crate:** `crates/rich-text-editor/` — 11,400+ lines, 368 tests, zero clippy
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
  List       { ordered: bool, items: Vec<ListItem> }
  BlockQuote { blocks: Vec<Arc<Block>> }
  HorizontalRule

ListItem
  blocks: Vec<Arc<Block>>       // usually one Paragraph; can nest Lists

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

Note: `link` is a field on `StyledRun`, not on `InlineStyle`. This was a
deliberate deviation from the original spec (which had `link` as an
`InlineStyle` optional field) — links are semantically different from
formatting flags, and keeping them separate simplifies the `same_formatting()`
check used by normalization.

### Immutability and structural sharing

`Document.blocks` is `Vec<Arc<Block>>`. After an edit, only the affected block
gets a new `Arc` allocation. Unchanged blocks are `Arc::clone` — cheap pointer
copies. `ListItem.blocks` and `BlockQuote.blocks` also use `Arc<Block>`.

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

**Status: fully implemented. All 8 variants with apply + invert.**

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
  Insert  { block_index }
  Remove  { block_index }
  CrossBlockDelete { start_block, removed_count, start_offset }
```

`PosMap::map(pos)` applies structural changes first, then char-level entry
mapping. Split remaps positions after the split offset into the new block.
Merge adds merge_offset when collapsing positions. CrossBlockDelete collapses
positions in deleted blocks to the deletion point.

### Missing operation

`SetBlockAttrs` for block-level attributes that aren't type changes (text
alignment, list indent level). Add when implementing alignment or indentation.

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
4. Every `ListItem` has ≥1 block
5. Every `BlockQuote` has ≥1 block
6. Document has ≥1 block

Normalization is recursive for container blocks (List items, BlockQuote children).

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
| Auto-exit block | **Not done** | Needs list item nesting context |
| Block embed isolation | **Not done** | Needs Block::Image (Phase 5) |

### Delete rules

| Rule | Status |
|------|--------|
| Delete selection first | Done |
| Backspace at block start merges | Done |
| Delete forward at block end merges | Done |
| Merge preserves first block's type | Done |
| Backspace at document start is no-op | Done |
| Document minimum (≥1 block) | Done |
| Block embed protection | **Not done** (needs Block::Image) |

### Format rules

| Rule | Status | Notes |
|------|--------|-------|
| Toggle with selection → ToggleInlineStyle | Done | |
| Toggle at caret → pending style | Done | |
| Link formatting at caret | **Not done** | Find link boundaries, format whole link |
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
  `draw_list_marker()`, `draw_paragraph()`

### Hit testing and cursor (widget/cursor.rs)

- `BlockLayout` — per-block layout info (y_offset, height, content_offset)
- `CursorState` — blink timing (500ms), focus tracking, target_x for vertical
  movement
- `DragState` — anchor position and active flag
- `hit_test()` / `find_block_at_point()` — binary search blocks by y_offset,
  translate to block-local coordinates
- `selection_block_ranges()` — decompose selection into per-block participation
  (Single/First/Full/Last)
- `prepare_move_up()` / `prepare_move_down()` — vertical cursor movement with
  cross-block boundary handling

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

- `layout()` — uses ParagraphCache to compute block paragraph layouts
- `draw()` — iterates blocks: fills paragraphs, draws HR/blockquote/list
  decorations, renders selection highlight rectangles, draws blinking cursor
- `update()` — handles window focus/blink, keyboard events (mapped via
  input::map_key_event), mouse click/drag/release with hit testing
- `mouse_interaction()` — Text cursor when hovering, NotAllowed when disabled

### Known limitations

- **Container block rendering:** Lists and blockquotes use a combined-text
  placeholder paragraph. Proper rendering needs per-item/per-child paragraphs
  in the cache.
- **Cursor placement precision:** The cursor is drawn at the block's x-origin,
  not at the exact grapheme position from `Paragraph::grapheme_position()`.
  Selection highlights cover full block height, not per-line rectangles.
- **Vertical cursor movement:** Currently moves to the same offset in the
  adjacent block. Proper behavior needs `Paragraph::grapheme_position()` to
  maintain x-coordinate across visual lines.
- **Scrolling:** No scroll offset implemented yet.
- **IME:** Basic keyboard input works; no preedit/commit or platform IME
  protocol integration.

---

## Crate Structure

```
crates/rich-text-editor/
  Cargo.toml
  src/
    lib.rs                    // re-exports + feature gate
    document.rs               // Document, Block, StyledRun, InlineStyle, DocPosition, DocSlice
    operations.rs             // EditOp, PosMap, apply/invert, run splitting helpers
    normalize.rs              // Normalization pass: merge runs, enforce structural invariants
    rules.rs                  // Heuristic rules: insert/delete/format behavior
    undo.rs                   // UndoStack, UndoGroup, cursor bookmark mapping
    html_serialize.rs         // Document → HTML
    html_parse.rs             // HTML → Document (html5ever TreeSink)
    widget/
      mod.rs                  // EditorState, Action, RichTextEditor widget (Widget trait impl)
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

### What's done (Phases 1–3 complete)

- Document model with all 5 block types, Arc structural sharing, DocSlice
- All 8 EditOp variants with correct apply, invert, and PosMap
- Normalization with dirty tracking and safety valve
- Rules engine with insert/delete/format behavior
- Undo/redo with grouping and cursor bookmarks
- HTML serialization and parsing with round-trip tests
- Widget with paragraph caching, keyboard input, mouse hit testing,
  cursor blink, selection rendering, clipboard (Ctrl+C/X/V)
- 368 tests across all modules

### What remains

**Phase 3 gaps (rules):**
- Auto-exit block (double-Enter exits list/quote) — needs list item context
- Block embed isolation — deferred until Block::Image exists
- Link formatting at caret — find link boundaries, format whole link

**Phase 4: Clipboard (partially scaffolded)**
- DocSlice type exists with open_start/open_end
- Copy/cut/paste actions defined and wired
- Not yet done: paste HTML from clipboard (detect text/html, parse via
  html_parse, apply with open-end merging), copy as text/html + text/plain

**Phase 5: Signatures, inline images, reply quoting**
- Not started
- Block::Image variant, signature insertion, reply quoting with attribution,
  platform IME refinement, plain-text projection for IME sync

**Widget polish:**
- Per-item paragraph cache for lists and blockquotes
- Exact cursor placement via `Paragraph::grapheme_position()`
- Per-line selection rectangles
- Scroll offset
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
