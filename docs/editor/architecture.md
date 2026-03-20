# Rich Text Editor Architecture

WYSIWYG rich text editor for email composition in iced. Built from scratch — no
existing rich text editor exists for iced.

Design informed by deep study of ProseMirror (schema + transactions + position
mapping), Slate.js (normalization + path-based addressing + operation
invertibility), Quill (delta algebra), and fleather (native Flutter editor — the
only reference project that solves rendering + input on a declarative UI framework
without contentEditable). See `docs/editor/research-summary.md` for detailed
analysis of all four.

---

## Document Model

Block tree with inline runs. Maps naturally to both HTML (DOM) and iced rendering
(column of `Paragraph` widgets).

```
Document
  blocks: Vec<Block>

Block
  Paragraph  { runs: Vec<StyledRun> }
  Heading    { level: u8 (1-3), runs: Vec<StyledRun> }
  List       { ordered: bool, items: Vec<ListItem> }
  BlockQuote { blocks: Vec<Block> }
  HorizontalRule

ListItem
  blocks: Vec<Block>          // usually one Paragraph; can nest Lists

StyledRun
  text: String
  style: InlineStyle

InlineStyle (bitflags + optional fields)
  BOLD
  ITALIC
  UNDERLINE
  STRIKETHROUGH
  link: Option<String>        // href
```

### Normalization invariant

Adjacent `StyledRun`s within the same block must have different `InlineStyle`s.
After every edit, adjacent runs with identical styles merge. This keeps run counts
small and makes boundary operations predictable.

### Cursor and selection

```
DocPosition
  block_index: usize
  offset: usize               // char offset in block's flattened text (all runs concatenated)

DocSelection
  anchor: DocPosition          // where selection started
  focus: DocPosition           // where the caret visually is (can be before or after anchor)
```

Why flattened offsets, not `(run_index, char_offset)`: edits frequently split and
merge runs. With flattened offsets the cursor is stable across run restructuring.
Resolving a flattened offset to a specific run is O(runs_per_block), which is
trivially small for email text.

Cross-block selection: anchor in block 3, focus in block 7 means blocks 4–6 are
fully selected. Delete, paste, and format operations handle this naturally.

---

## Widget Strategy

Custom `Widget` trait implementation. Not a `TextEditor` fork — `TextEditor`
delegates to cosmic-text's monolithic `Buffer` which has no concept of blocks,
and its `Highlighter` API only carries color + font per range (no underline,
strikethrough, size, or background).

### Rendering

One `Renderer::Paragraph` per `Block`, created via `Paragraph::with_spans()`:

- Each `StyledRun` maps to an iced `Span` with the appropriate `font`
  (bold → `font::TEXT_BOLD`, italic → `font::TEXT_ITALIC`, bold+italic →
  `font::TEXT_BOLD_ITALIC`), `underline`, `strikethrough`, `color`, and `size`.
- Blocks stacked vertically with spacing from the layout system:
  - Headings: H1 = `TEXT_HEADING` (18), H2 = `TEXT_TITLE` (16), H3 = `TEXT_XL` (14)
  - List items: bullet/number prefix in fixed-width leading container, indent for nesting
  - Block quotes: 2px left border + 16px indent + muted text color
  - Horizontal rule: `fill_quad` line across the width
- Blocks are drawn via `renderer.fill_paragraph()`.

### Cursor and selection rendering

- Cursor: `fill_quad` vertical line at `Paragraph::grapheme_position(line, index)`.
  Blinks on 500ms interval (same as iced's `text_editor`).
- Selection: blue `fill_quad` rectangles per line span, computed from
  `grapheme_position` for start and end of each selected line within each block.

### Input handling

The widget's `update()` method handles:

- **Keyboard text input** → document edit operations (insert, delete, split block)
- **Mouse click** → `hit_test()` on each block's `Paragraph` to find char offset,
  map to `DocPosition`
- **Mouse drag** → track start, update selection focus on move
- **IME** → forward preedit/commit (same pattern as iced's text_editor)
- **Format shortcuts** → intercept Ctrl+B/I/U before text insertion

### Paragraph caching

```
EditorWidgetState
  paragraphs: Vec<CachedParagraph>    // one per block, invalidated on edit
  focus: Option<Focus>                 // tracks blink timing + window focus
  scroll_offset: f32
  drag_state: Option<DragState>
  ime_preedit: Option<Preedit>

CachedParagraph
  paragraph: Renderer::Paragraph
  dirty: bool
  y_offset: f32                        // computed during layout
  height: f32
```

Only dirty paragraphs re-layout. Most edits dirty a single block.

---

## Editing Operations

Operation-based for undo/redo, not patch-based (`dissimilar`). Patch diffing
operates on flat strings — it can't capture "toggled bold on characters 12-25"
or "changed this paragraph to a heading." With a structured Document model, each
operation already knows exactly what changed and how to reverse it. Memory is
negligible: `EditOp`s are positions + small strings, not full-document snapshots.

(`dissimilar` remains appropriate for flat text fields like contact notes or
calendar descriptions that don't have a structured model.)

Every user action creates an `EditOp` that knows how to apply and reverse itself.

```
EditOp
  InsertText       { position, text }
  DeleteRange      { start, end, deleted: DeletedContent }
  SplitBlock       { position }                              // Enter key
  MergeBlocks      { block_index, saved: Block }             // Backspace at block start
  ToggleInlineStyle { start, end, style_bit }
  SetBlockType     { block_index, old: BlockKind, new: BlockKind }
  InsertBlock      { index, block }
  RemoveBlock      { index, saved: Block }

DeletedContent
  blocks: Vec<Block>    // enough to reconstruct original structure on undo
```

### Undo stack

`Vec<UndoGroup>` where each `UndoGroup` is a batch of `EditOp`s from one logical
user action, plus cursor positions before/after. Consecutive character insertions
group into one entry (split on pause, format change, or cursor jump).

### Format toggle logic

**With selection:** Walk each block in the selection range. Find overlapping runs,
split at selection boundaries, flip the style bit. If all text in the range
already has the style, remove it; otherwise, add it. Normalize (merge adjacent
same-style runs) afterward.

**Without selection (caret):** Toggle a "pending style" flag on the editor state.
The next inserted character inherits this pending style. This is standard rich
text editor behavior.

---

## HTML Serialization

### Document → HTML

Recursive walk, ~100 lines:

```
Paragraph  → <p>{runs}</p>
Heading(n) → <h{n}>{runs}</h{n}>
List(ord)  → <ol>/<ul> with <li> children
BlockQuote → <blockquote>{children}</blockquote>
HRule      → <hr>

run → wrap text in <strong>/<em>/<u>/<s>/<a> as needed
```

Nesting order: `<a><strong><em><u><s>text</s></u></em></strong></a>` — consistent
ordering avoids ambiguity.

### HTML → Document

Parse with `html5ever` into DOM tree, then recursive walk:

1. Block-level elements (`<p>`, `<h1>`–`<h6>`, `<ul>`, `<ol>`, `<li>`,
   `<blockquote>`, `<div>`, `<br>`) create `Block`s.
2. Inline elements (`<strong>`/`<b>`, `<em>`/`<i>`, `<u>`,
   `<s>`/`<strike>`/`<del>`, `<a>`) push style bits onto a stack (frostmark's
   `ChildData` bitflags pattern).
3. Text nodes create `StyledRun`s with accumulated style.
4. Unknown block elements → Paragraph. Unknown inline elements → pass through
   content, ignore tag.

The existing sanitizer pipeline (`css-inline` + `lol_html` in
`crates/provider-utils/src/html_sanitizer.rs`) runs before parsing to normalize
inline styles into tags.

**Scope is narrow.** This parser only handles the editor's own HTML subset: drafts
we previously saved, signatures from the signatures table, and reply-quoted
content. It does not need to handle arbitrary wild HTML from the internet — that's
the job of `litehtml-rs` (`/home/folk/Programs/litehtml-rs`), a separate pure Rust
rendering pipeline (scraper → lightningcss → taffy → cosmic-text → tiny-skia) that
renders arbitrary HTML emails to rasterized images in 3-23ms. The editor parser
can be strict and simple because it only round-trips its own output.

Tables and complex layouts encountered in quoted content flatten to text. This is
acceptable — the user is editing a reply, not viewing the original. The original
renders via litehtml-rs in the reading pane.

---

## Lessons from ProseMirror, Slate, and Quill

Deep study of these editors surfaced several concerns our initial design
underspecified. Each heading below describes a problem they solve and how we
should address it.

### Immutability and structural sharing

ProseMirror nodes are fully immutable — edits create new nodes, sharing unchanged
subtrees via reference. Slate uses a similar approach (new references for changed
nodes, identity comparison for "did this change?"). This is critical for:
- Efficient re-rendering (only dirty blocks need new paragraphs)
- Safe undo (old documents are preserved, not mutated)
- Predictable behavior (no aliasing bugs)

**For us:** `Document` and `Block` should be `Clone` with `Arc`-wrapped children
where structural sharing matters. After an edit, only the affected block (and its
ancestors in the document) get new allocations. Unchanged blocks are `Arc::clone`.

### Position mapping across edits

ProseMirror's `StepMap` encodes each edit as `[start, oldSize, newSize]` triples.
When a cursor or selection needs to survive an edit, it's mapped through the
StepMap. The `Mapping` class chains multiple StepMaps for multi-step transactions,
with a `mirror` system that links a step to its inverse for undo recovery.

Our architecture has `DocPosition` but no mechanism for mapping positions through
edits. This matters for:
- Selection preservation during undo/redo
- Cursor stability when operations modify earlier parts of the document
- Future: collaborative editing

**For us:** Each `EditOp::apply()` should return a `PosMap` (similar to StepMap)
describing what shifted. `DocPosition::map(pos_map)` adjusts a position through
an edit. The undo stack stores cursor positions that get mapped through subsequent
edits.

Simplified version for our block-based model: a `PosMap` is
`{ block_index: usize, old_offset: usize, old_len: usize, new_len: usize }` for
intra-block changes, plus block-level insert/remove/split/merge deltas. This is
much simpler than ProseMirror's fully general mapping because we have two levels
(block index + char offset) rather than one flat integer space.

### Normalization as a formal pass

Slate's normalization system is its most distinctive feature. After every
operation, "dirty paths" are computed and each affected node is normalized. The
default normalizer enforces:
- Elements must have at least one child
- Inline/block consistency within a parent
- Adjacent identically-formatted text nodes merge
- Inline nodes must be surrounded by text nodes

Our doc mentions "adjacent runs with identical styles merge" as an invariant but
doesn't formalize when and how normalization runs.

**For us:** Add a `normalize()` method on `Document` that runs after every edit
(or batched via `without_normalizing` for multi-op transactions):
1. Merge adjacent `StyledRun`s with identical `InlineStyle`
2. Remove empty runs (except: keep one empty run per block for cursor anchoring)
3. Ensure every `Block` has at least one run (insert empty run if needed)
4. Ensure `List` items each contain at least one block

This runs on only the affected blocks (track dirty block indices per operation),
not the entire document. Slate's safety valve (max iterations = dirty_count × 42)
is worth stealing to prevent infinite loops from buggy normalizers.

### The Slice problem (cross-block copy/paste)

ProseMirror's `Slice` = `{ content: Fragment, openStart: number, openEnd: number }`.
When you copy from the middle of one paragraph to the middle of another, the
slice captures the partial paragraphs with "open" depths indicating how many
levels are cut through.

Our architecture doesn't address how cross-block clipboard content is
represented. Without something like Slice, pasting "half a heading + two
paragraphs + half a paragraph" has no clean representation.

**For us:** Define a `DocSlice`:
```
DocSlice
  blocks: Vec<Block>
  open_start: bool    // first block is a fragment, not a complete block
  open_end: bool      // last block is a fragment, not a complete block
```

Simpler than ProseMirror's arbitrary-depth open counts because our tree is only
two levels deep (document → blocks → runs). When pasting:
- If `open_start`: merge the first slice block's runs into the block at the
  cursor position (after splitting it at the cursor offset)
- Middle blocks insert as-is
- If `open_end`: merge the last slice block's runs into the block after the split

### Operation completeness

Comparing our `EditOp` set against ProseMirror's steps and Slate's 9 operations:

| Our EditOp | PM equivalent | Slate equivalent | Notes |
|------------|---------------|------------------|-------|
| InsertText | ReplaceStep | insert_text | OK |
| DeleteRange | ReplaceStep | remove_text + remove_node | Need to handle cross-block |
| SplitBlock | ReplaceStep (structural) | split_node | OK |
| MergeBlocks | ReplaceStep (structural) | merge_node | OK |
| ToggleInlineStyle | AddMarkStep/RemoveMarkStep | set_node (on text) | OK |
| SetBlockType | ReplaceAroundStep | set_node (on element) | OK |
| InsertBlock | ReplaceStep | insert_node | OK |
| RemoveBlock | ReplaceStep | remove_node | OK |
| — | — | move_node | Not needed for email compose |
| — | AttrStep | — | Could add for block attrs (alignment, indent) |

**Missing:** A `SetBlockAttrs` operation for block-level attributes that aren't
type changes (e.g., text alignment, list indent level). ProseMirror uses
`AttrStep` for this; Slate uses `set_node`. Add when we implement alignment or
indentation.

### Invertibility must be explicit

Both ProseMirror and Slate make every operation trivially invertible:
- Slate: `insert_node` ↔ `remove_node` (same payload), `split_node` ↔
  `merge_node`, etc.
- ProseMirror: `step.invert(oldDoc)` captures the replaced content

Our `EditOp` already stores enough to invert (e.g., `DeleteRange` captures
`deleted: DeletedContent`), but we should formalize this: every `EditOp` must
implement `fn invert(&self) -> EditOp` that returns the exact inverse operation.
The undo stack stores the inverse, not the original.

### Content validation (schema-lite)

ProseMirror compiles content expressions (`"paragraph+ heading*"`) into DFAs for
validation. This is powerful but heavy. Slate has no schema — validity is
enforced purely by normalization.

For an email editor, full schema validation is overkill. But we need at least:
- `List` must contain only `ListItem` children
- `ListItem` must contain at least one block
- `BlockQuote` must contain at least one block
- `Document` must contain at least one block

**For us:** Encode these as assertions in `normalize()` rather than a separate
schema system. If normalization encounters a violation, it fixes it (insert empty
paragraph, unwrap invalid nesting). This is Slate's approach and is sufficient
for our constrained block set.

---

## Lessons from Fleather (Native Flutter Editor)

Fleather is the only reference project that solves the full stack — document
model, rendering, and input — on a declarative UI framework without
contentEditable. Flutter's `RenderBox` + `TextPainter` is analogous to iced's
`Widget` + `Paragraph`. Four additional concerns surfaced.

### Heuristic rules for edit behavior

Fleather has a formal rules engine: 15 pure functions
`(document, position, data) → operation` organized as a chain of responsibility.
Rules are tried in priority order; first match wins. This encodes "what should
happen when the user does X" separately from the low-level operation machinery.

**Insert rules (our equivalents needed):**

1. **Block embed isolation** — block-level embeds (images, HRs) must get their
   own line. If inserting adjacent to one, force a newline.
2. **Auto-exit block** — pressing Enter on an empty line at the end of a list or
   blockquote exits the block (or de-indents one level). Only fires at the last
   item. This is the "double-Enter to exit" behavior users expect.
3. **Preserve block style on split** — inserting a newline inside a list item
   creates a new list item, not a plain paragraph.
4. **Preserve line style on split** — splitting a heading mid-text creates two
   headings. But splitting at the *end* of a heading creates a heading + a
   paragraph (reset heading for the new line).
5. **Inline style inheritance** — typing inside a bold run continues bold. But
   typing at the boundary of a link does NOT extend the link (link boundaries are
   exclusive).
6. **Pending style override** — if the user toggled bold at the caret (pending
   style), that overrides inheritance for the next insertion.

**Delete rules:**

1. **Preserve line style on merge** — backspace at the start of a line merges it
   with the previous line. The surviving line keeps the *first* line's style (the
   one being merged into).
2. **Block embed protection** — cannot merge lines across a block embed.
3. **Document minimum** — cannot delete the last newline (document must always
   have at least one block).

**Format rules:**

1. **Link formatting at caret** — when the cursor is inside a link and the user
   applies link formatting, find the link boundaries and format the whole link,
   not just the zero-width caret position.
2. **Line vs inline scope** — block-level attributes (heading, list type) apply
   only to the block, not to character ranges. Inline attributes apply to
   character ranges, not blocks. The rules system enforces this separation.

**For us:** Add a `rules.rs` module. Each rule is a function:
```
fn apply(doc: &Document, pos: DocPosition, action: EditAction) -> Option<Vec<EditOp>>
```
Rules are tried in order. First `Some` return wins. The top-level `insert()`,
`delete()`, and `format()` methods on the editor state call through the rules
chain rather than directly creating operations. This keeps the "what should
happen" logic separate from the "how to apply it" operation machinery.

This is where most of the editor's user-facing behavior lives. Getting these
rules right is more important than getting the data structures right — users
notice when Enter doesn't do what they expect, not when the run merging algorithm
is suboptimal.

### Cascading position mapping (rendering ↔ document)

Fleather's rendering layer uses cascading delegation for hit testing and caret
placement:

```
RenderEditor                 → find child at pixel offset
  RenderEditableTextBlock    → subtract block's layout offset, find child line
    RenderEditableTextLine   → subtract line's layout offset, delegate to body
      RenderParagraphProxy   → delegate to Flutter's TextPainter (= cosmic-text)
```

Each level subtracts its own layout offset and delegates downward. The reverse
path (document position → pixel offset for caret rendering) follows the same
cascading pattern with `getOffsetForCaret()`.

**For us:** Our widget's `draw()` already stacks blocks vertically with
`y_offset` per `CachedParagraph`. Hit testing follows the same pattern:
1. Binary search blocks by `y_offset` to find which block was clicked
2. Subtract the block's `y_offset` from the click position
3. Call `Paragraph::hit_test()` on that block's cached paragraph
4. The returned char offset + the block index = `DocPosition`

Caret rendering reverses it:
1. Look up `CachedParagraph` at `block_index`
2. Call `Paragraph::grapheme_position(line, offset)` on it
3. Add the block's `y_offset`

Vertical cursor movement (arrow up/down) needs special handling at block
boundaries: when the cursor is on the first line of a block and moves up, the
widget must find the previous block, get its last line's height, and compute the
position at the same x-coordinate in that line. Fleather solves this with a
`VerticalCaretMovementRun` iterator that tracks the x-coordinate across
consecutive vertical moves.

### Text input strategy

ProseMirror, Slate, and Quill all use `contentEditable` and let the browser
handle text input — then reconcile. Fleather cannot do this (Flutter has no
browser) and instead uses Flutter's `DeltaTextInputClient` protocol:

1. On focus, connect to the platform IME
2. Send the current text + selection state to the platform
3. Receive delta events back (insertions, deletions, replacements)
4. Map each platform delta to a document operation
5. After the operation, sync the new text + selection back to the platform

**For us:** iced's existing text input handling (in `text_editor.rs`) already
receives keyboard events and IME preedit/commit through iced's event system. We
can start with this — it handles basic typing, composition, and clipboard on all
platforms. Proper platform IME protocol integration (IBus/Fcitx on Linux,
NSTextInputClient on macOS, TSF on Windows) can come later if iced's built-in
handling proves insufficient.

The key insight from fleather: **the editor must always be able to tell the
platform what its current text and selection state is**, so the platform's IME can
offer correct suggestions, composition, and autocorrect. This means maintaining a
plain-text projection of the document (or at least the current block) that can be
sent to the platform on demand.

### Run splitting for inline format application

Fleather's `LeafNode` has three surgical operations: `splitAt(index)`,
`cutAt(index)`, and `isolate(index, length)`. When applying bold to characters
12-25 of a paragraph:

1. `isolate(12, 13)` splits the containing run twice — once at 12, once at 25 —
   producing up to three runs: `[0..12)`, `[12..25)`, `[25..end)`
2. Apply bold to the middle run
3. `optimize()` merges adjacent runs with identical styles

This split-apply-merge pattern is the standard approach across all editors we
studied. ProseMirror's `mapFragment` in `AddMarkStep` does the same.

**For us:** `StyledRun` needs:
```
fn split_at(&self, offset: usize) -> (StyledRun, StyledRun)
fn isolate(runs: &mut Vec<StyledRun>, start: usize, end: usize) -> Range<usize>
```
Where `isolate` returns the index range of the affected runs after splitting.
These are the building blocks for `ToggleInlineStyle` and any future mark
operations. After applying the style change, `normalize()` merges adjacent
same-style runs.

### Vec vs linked list for runs

Fleather uses Dart's `LinkedList` for all node children, giving O(1)
insert/remove for the split-apply-merge pattern. Our `Vec<StyledRun>` means
splitting a run requires shifting elements.

**For us:** `Vec` is fine. Email paragraphs rarely have more than ~10 styled
runs. The shift cost for `Vec::insert` on 10 elements is negligible compared to
the text shaping cost of re-laying-out the paragraph. The cache friendliness and
simpler code of `Vec` outweigh the theoretical O(n) disadvantage. If profiling
ever shows run manipulation as a bottleneck (it won't for email), `SmallVec<[StyledRun; 8]>`
is the first optimization — not a linked list.

---

## Crate Structure

```
crates/rich-text-editor/
  Cargo.toml
  src/
    lib.rs
    document.rs           // Document, Block, StyledRun, InlineStyle, DocPosition, DocSlice
    operations.rs         // EditOp, PosMap, apply/invert, format toggle, run splitting
    normalize.rs          // Normalization pass: merge runs, enforce structural invariants
    rules.rs              // Heuristic rules: insert/delete/format behavior (chain of responsibility)
    undo.rs               // UndoStack, UndoGroup, cursor bookmark mapping
    html_serialize.rs     // Document → HTML
    html_parse.rs         // HTML → Document (html5ever)
    widget/
      mod.rs              // RichTextEditor widget (Widget trait impl)
      input.rs            // Key binding → action mapping, text input strategy
      render.rs           // Paragraph caching, draw logic, block-level rendering
      cursor.rs           // Cursor/selection rendering, hit testing, vertical movement
```

**Pure Rust** (no iced dependency): `document`, `operations`, `html_serialize`,
`html_parse`. Unit-testable without a GUI.

**Feature-gated** (`widget` feature, default on): `widget/` module depends on iced.

**Dependencies:**
- `html5ever` + `markup5ever` — HTML parsing
- `bitflags` — `InlineStyle`
- `iced` — widget (feature-gated)

### App crate integration

The app crate's compose view creates a `RichTextEditor` widget, holding a
`Document` in `App` state. The toolbar is standard iced buttons in the app crate
(not in the editor crate), sending messages like
`Message::Compose(ComposeAction::ToggleBold)` that `update()` forwards to
`document.toggle_inline_style(...)`.

---

## Implementation Phases

### Phase 1: Document model + plain text editing

- `document.rs`: `Document`, `Block::Paragraph`, `StyledRun`, `DocPosition`,
  `DocSelection`, `DocSlice`. Immutable blocks with `Arc` structural sharing.
- `operations.rs`: `InsertText`, `DeleteRange`, `SplitBlock`, `MergeBlocks`.
  Each returns a `PosMap`. Each implements `invert()`.
- `normalize.rs`: merge adjacent runs, ensure blocks have ≥1 run, dirty tracking
- `rules.rs`: basic insert rules (inline style inheritance, split-line style
  preservation), basic delete rules (merge-line style preservation, document
  minimum)
- `undo.rs`: `UndoStack` with `UndoGroup` batching, cursor bookmark mapping
  through `PosMap`s
- Custom widget: render blocks as `Paragraph::with_spans` (uniform style),
  keyboard input, mouse click/drag with cascading hit testing, cursor blink,
  scroll, vertical cursor movement across block boundaries
- Unit tests for all operations, normalization, and rules
- **Milestone:** usable as a plain text compose field with undo/redo

### Phase 2: Inline formatting

- `InlineStyle` bitflags, `ToggleInlineStyle` operation
- Run splitting: `split_at()`, `isolate()` for surgical style application
- Font mapping: bold → `font::TEXT_BOLD`, italic → `font::TEXT_ITALIC`,
  bold+italic → `font::TEXT_BOLD_ITALIC`. Underline/strikethrough via Span flags.
- Toolbar: row of icon buttons for B/I/U/S/Link
- Keyboard shortcuts: Ctrl+B, Ctrl+I, Ctrl+U
- Pending style state at caret
- Rules: link boundary exclusivity (typing at link edge doesn't extend link),
  pending style override
- Tests: format toggle, pending style, run splitting + merging, link boundaries
- **Milestone:** bold, italic, underline, strikethrough working

### Phase 3: Block types + HTML round-trip

- `Block::Heading`, `Block::List`, `Block::BlockQuote`
- `SetBlockType` operation, `SetBlockAttrs` for indent level
- Rules: auto-exit block (double-Enter exits list/quote), preserve block style
  on split, heading reset on split-at-end, block embed isolation
- Normalization: list items must contain ≥1 block, blockquotes must contain ≥1 block
- `html_serialize.rs` and `html_parse.rs`
- Link insertion (URL input dialog)
- Block-specific rendering (heading sizes, list leading widgets with
  bullet/number, quote left border, indent)
- HTML round-trip tests against real email samples
- **Milestone:** full compose workflow — type, format, send as HTML, edit drafts

### Phase 4: Clipboard

- `DocSlice` with `open_start` / `open_end` for cross-block copy/paste
- Copy: serialize selection to `DocSlice`, then to text/html + text/plain
- Paste: detect HTML on clipboard → parse to `DocSlice` via `html_parse.rs`,
  apply with open-end merging. Fall back to plain text.
- Cut: copy + delete selection
- Tests: cross-block copy/paste round-trip, paste from external HTML sources

### Phase 5: Signatures, inline images, reply quoting

- Signature insertion: parse `body_html` → Document blocks, append with separator
- Inline images: new `Block::Image` or embed run variant, render via
  `renderer.fill_quad` + `iced::widget::image`
- Reply quoting: parse replied message HTML, wrap in `BlockQuote`, prepend
  attribution line ("On {date}, {sender} wrote:")
- Platform IME refinement if iced's built-in handling proves insufficient
- Plain-text projection for IME state sync

---

## Key References

| File | What to steal |
|------|---------------|
| **JS editors** | |
| ProseMirror `prosemirror-model/src/resolvedpos.ts` | Position resolution: flat integer → tree context, caching strategy |
| ProseMirror `prosemirror-transform/src/map.ts` | StepMap (`[start, oldSize, newSize]` triples), Mapping with mirror system |
| ProseMirror `prosemirror-model/src/replace.ts` | Slice (`content, openStart, openEnd`) for cross-block clipboard |
| ProseMirror `prosemirror-transform/src/mark_step.ts` | AddMarkStep: walk inline nodes, split at boundaries, coalesce adjacent |
| Slate `packages/slate/src/editor/normalize.ts` | Dirty path tracking, normalize loop with safety valve (count × 42) |
| Slate `packages/slate/src/interfaces/operation.ts` | 9 operations, each trivially invertible |
| Slate `packages/slate-history/src/with-history.ts` | History batching: consecutive same-type at adjacent positions |
| **Flutter editors** | |
| fleather `packages/parchment/lib/src/heuristics/insert_rules.dart` | 9 insert rules: auto-exit block, style inheritance, link boundary, heading reset |
| fleather `packages/parchment/lib/src/heuristics/delete_rules.dart` | 3 delete rules: line merge style, embed protection, doc minimum |
| fleather `packages/fleather/lib/src/rendering/editor.dart` | Cascading hit test: editor → block → line → paragraph proxy |
| fleather `packages/fleather/lib/src/widgets/editor_input_client_mixin.dart` | Platform IME protocol: text/selection sync, delta-based input |
| fleather `packages/parchment/lib/src/document/leaf.dart` | `splitAt`, `isolate`, `optimize` — run splitting pattern |
| **iced ecosystem** | |
| iced `widget/src/text_editor.rs` | Input handling: key bindings, mouse click/drag, IME, focus/blink, clipboard |
| iced `widget/src/text/rich.rs` | `Paragraph::with_spans` rendering, span→font mapping, underline/strikethrough |
| `crates/app/src/font.rs` | Font variants (TEXT_BOLD, TEXT_ITALIC, etc.) |
| `crates/app/src/ui/layout.rs` | Type scale (TEXT_HEADING, TEXT_TITLE, TEXT_XL) and spacing constants |
| `crates/provider-utils/src/html_sanitizer.rs` | Sanitizer pipeline — run before html5ever parse |
| `crates/core/src/db/queries_extra/compose.rs` | Draft CRUD, signatures, templates |
| `research/frostmark/src/renderer.rs` | HTML→widget DOM traversal with ChildData bitflags |
| `research/halloy/src/widget/selectable_rich_text.rs` | Span-based display, selection math, link hover |
| **Email rendering** | |
| `/home/folk/Programs/litehtml-rs/` | Separate HTML email viewer — handles arbitrary wild HTML. The editor does NOT need to handle complex HTML; `html_parse.rs` only round-trips its own output. |
