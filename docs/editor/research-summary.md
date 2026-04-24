# Rich Text Editor Research Summary

Surveyed 5 iced projects to find a starting point for the compose editor. None
have a WYSIWYG rich text editor. All fall into two camps: edit plain text, or
display rich text read-only. We build from scratch, stealing isolated pieces.

---

## Projects Surveyed

### cosmic-edit - Code editor (COSMIC desktop)

**What it is:** Syntax-highlighting code editor. Custom `TextBox` widget
implementing iced's `Widget` trait. Uses cosmic-text `Buffer`/`ViEditor` for text
storage and shaping.

**Architecture:** Per-line attributes only (syntax highlighting colors via
`AttrsList`). No per-character bold/italic/underline. Monospace throughout.
Renders via `renderer.fill_raw()` for glyphs + pixel buffer for line numbers.

**Relevance:** Reference for building custom iced widgets with keyboard/mouse/
scrolling/IME. The rendering approach (pixel buffer + glyph fill) is
over-engineered for our needs - we can use `Paragraph::with_spans` instead.

**Steal nothing.** Too tightly coupled to cosmic-text's code-editor assumptions.

### cedilla - Markdown editor (COSMIC desktop)

**What it is:** Dual-pane markdown editor. Plain text editing on the left,
rendered preview on the right via frostmark. Custom `TextEditor` widget (fork of
iced's) with line numbers and libcosmic integration.

**Key patterns:**
- **Patch-based undo/redo** (`src/app/core/history.rs`): Uses `dissimilar` crate
  to diff text snapshots, stores ~100-patch circular buffer. Avoids full-text
  cloning. Alternative to our operation-based approach - simpler but less
  granular.
- **Markdown formatting helpers** (`src/app/core/utils/markdown.rs`):
  `SelectionAction` enum with `format_selected_text()` - wraps/toggles markdown
  markers around selection. Smart cycling (bold on/off), list continuation on
  Enter. Pattern is directly applicable to our format toggle logic.

**Steal:** Undo concept (we chose op-based instead, but dissimilar is a fallback
if ops get complex). Markdown formatting toggle logic as reference for our
`ToggleInlineStyle`.

### frostmark - HTML/Markdown renderer for iced

**What it is:** Converts HTML (via html5ever) and Markdown (via comrak) into
native iced widgets. Used by cedilla for its preview pane.

**Architecture:**
- `RenderedSpan` enum: `Spans(Vec<Span>)` | `Elem(Element)` | `None`
- `ChildData` struct with bitflags: BOLD, ITALIC, UNDERLINE, STRIKETHROUGH,
  MONOSPACE, HIGHLIGHT, KEEP_WHITESPACE. Accumulates through recursive DOM
  traversal.
- Block/inline separation via `is_block_element()`. Inline content collected into
  `rich_text()` spans, block elements create new columns.
- Image handling: callback-driven (`on_drawing_image` closure), host app manages
  download/caching. `find_image_links()` pre-scans for batch loading.

**Steal:** The `ChildData` bitflags traversal pattern is exactly what our
`html_parse.rs` needs. The block/inline separation logic informs our DOM→Document
conversion. Image callback pattern is relevant for Phase 5.

### halloy - IRC client

**What it is:** Production IRC client with rich text display.

**Key widget: `selectable_rich_text.rs` (~1000 lines)**
- Generic: `Rich<'a, Message, Link, Entry, Theme, Renderer>`
- Span-based: uses iced's `Span<Link, Font>` with per-span color, font,
  underline, strikethrough, highlight, link
- Interactive: mouse hover for links, text selection (Idle→Selecting→Selected
  state machine), spoiler reveal, context menus
- Selection math: `Raw` points → `hit_test()` on `Paragraph` → character indices
  → grapheme-aware `Selection` struct

**Key data flow:**
```
Fragment enum (Text|Channel|User|Url|Formatted|...)
  → message_content.rs conversion layer
  → Span<Link, Font> array
  → selectable_rich_text widget
```

**Steal:** Selection rendering math (multi-line rectangles from grapheme
positions). Fragment→Span conversion pattern. Context menu integration. The
spoiler implementation (color == background) is clever but irrelevant.

### libcosmic - COSMIC widget toolkit

**What it is:** Pop-OS's iced wrapper. Provides `TextInput` (single-line) with
grapheme-aware cursor, DND, IME. Re-exports iced's `TextEditor` and optional
`markdown` widget.

**Relevant pieces:**
- `text_input/value.rs`: Grapheme-based text storage with word boundary detection
- `text_input/cursor.rs`: Cursor state with selection tracking
- `text_input/editor.rs`: Insert/paste/backspace/delete operations on Value

**Steal:** Grapheme-aware word navigation logic from `value.rs`. The cursor state
machine pattern.

---

## Gap Analysis

| Capability | cosmic-edit | cedilla | frostmark | halloy | libcosmic |
|------------|:-----------:|:-------:|:---------:|:------:|:---------:|
| Text input + cursor | yes | yes | - | - | yes |
| Per-character bold/italic | - | - | display | display | - |
| Underline/strikethrough | - | - | display | display | - |
| Block types (headings, lists) | - | markdown | display | - | - |
| Links | - | markdown | display | display | - |
| Inline images | - | preview | display | - | - |
| Selection | yes | yes | - | yes | yes |
| Undo/redo | cosmic-text | dissimilar | - | - | iced built-in |
| HTML output | - | - | - | - | - |
| HTML input | - | - | yes | - | - |
| **Editable rich text** | **no** | **no** | **no** | **no** | **no** |

The bottom row is the gap. Every project either edits plain text or displays rich
text. None combine both.

**Note on viewing vs editing:** Rendering arbitrary incoming HTML emails is handled
by `litehtml-rs` - a separate pure Rust pipeline (scraper → lightningcss → taffy
fork with table layout → cosmic-text → tiny-skia) that renders HTML to rasterized
images in 3-23ms. The editor's `html_parse.rs` does not need to handle wild HTML;
it only round-trips its own output (drafts, signatures, reply quotes). This
dramatically simplifies the parser - no CSS engine, no table layout, no marketing
email edge cases.

---

## JavaScript Editor Architectures

Studied ProseMirror, Slate.js, and Quill - three fundamentally different
approaches to the same problem. Source code in `research/prosemirror-*`,
`research/slate`, `research/quill`.

### ProseMirror - Schema + Transactions + Position Mapping

The most rigorous architecture. Four packages: model (document tree), transform
(editing operations), state (editor state + plugins), view (DOM rendering).

**Document model:** Immutable tree of `Node` objects with structural sharing.
Every position is a flat integer (text chars = 1, leaf nodes = 1, non-leaf = 2 +
content size for open/close tokens). `ResolvedPos` resolves flat integers to tree
context (depth, parent, text offset) via a walk-down algorithm with a 12-entry
per-node cache.

**Schema system:** `NodeSpec` and `MarkSpec` define valid document structure.
Content expressions (`"paragraph+ heading*"`) compile through tokenizer → AST →
NFA (Thompson's construction) → DFA (subset construction). The DFA validates
children incrementally and powers `fillBefore()` (auto-create required nodes) and
`findWrapping()` (find wrapper nodes to make an insertion valid). ~400 lines of
compiler code.

**Marks:** Sorted arrays by rank (defined by schema ordering). `addToSet()` and
`removeFromSet()` maintain sort order and handle mutual exclusion. Adjacent text
nodes with identical mark sets merge - this invariant is maintained everywhere.

**Steps and StepMap:** Each edit is a `Step` (ReplaceStep, AddMarkStep, etc.)
that produces a `StepMap` - an array of `[start, oldSize, newSize]` triples.
Positions are mapped through StepMaps to survive edits. The `Mapping` class
chains multiple StepMaps with a `mirror` system linking each step to its inverse,
enabling position recovery through undo.

**The Fitter** (`replace.ts`): The hardest algorithm. When pasting a `Slice`
(content with open start/end depths), the Fitter builds a valid replacement by
walking a `frontier` (open right side of the doc being built) and placing slice
content level by level, using the schema DFA to validate each placement. Falls
back to `openMore()` / `dropNode()` when content doesn't fit. ~300 lines.

**Slice:** `{ content, openStart, openEnd }`. When you copy from mid-paragraph
to mid-paragraph, the cut-through levels are recorded as open depths. This is
what makes cross-block paste work correctly.

**View layer:** Uses `contentEditable` as input mechanism. For most text input,
lets the browser handle the keystroke, observes DOM mutations via
`MutationObserver`, re-parses the changed DOM region, diffs against the model,
and dispatches a transaction. Only special cases (Enter, Backspace near nodes,
arrow keys, formatting shortcuts) are intercepted. **This entire layer is
browser-specific and irrelevant to our iced implementation.**

**What we take:**
- Position mapping through edits (StepMap concept, simplified for our 2-level model)
- Slice with open depths for clipboard
- Immutable nodes with structural sharing
- Operation invertibility (`step.invert(oldDoc)`)
- Mark normalization (merge adjacent same-mark text)

**What we skip:**
- Full schema with DFA content matching (overkill for email)
- The Fitter algorithm (we have a simple block set, not arbitrary nesting)
- The entire view layer (contentEditable, MutationObserver, DOM reconciliation)

### Slate.js - Normalization + Paths + Simplicity

Simpler model, different philosophy. Documents are plain JSON objects.

**Document model:** Three types - `Editor` (root), `Element` (has `children`),
`Text` (has `text` string). No schema. Formatting is properties directly on text
nodes (`{ text: "hello", bold: true }`), not separate mark objects. Elements have
arbitrary properties (`{ type: "paragraph", children: [...] }`).

**Addressing:** `Path` = `number[]` (array of child indices from root).
`Point` = `{ path, offset }`. `Range` = `{ anchor, focus }`. Paths are more
intuitive for tree operations but require path transformation logic for every
operation type.

**9 operations:** `insert_node`, `remove_node`, `set_node`, `split_node`,
`merge_node`, `move_node`, `insert_text`, `remove_text`, `set_selection`. Every
operation is trivially invertible (flip type, swap old/new). This makes undo
clean.

**Normalization** - the key innovation. After every operation, dirty paths are
computed and each affected node is normalized. Built-in rules:
1. Elements must have at least one child
2. Inline/block consistency within a parent
3. Adjacent identically-formatted text nodes merge
4. Inline nodes must be surrounded by text nodes

Users extend normalization with custom rules (override `editor.normalizeNode`).
Safety valve: max iterations = dirty_count × 42.

**History:** Operations batch into undo groups by merging consecutive same-type
ops at adjacent positions (typing, backspacing). Each group stores a selection
bookmark. Undo applies inverse operations in reverse order. Max 100 entries.

**Plugin model:** Functions that wrap editor methods (`withHistory(withReact(createEditor()))`).
Simple but unstructured - no lifecycle, no formal state management.

**What we take:**
- Formalized normalization pass with dirty tracking (not just "merge same runs")
- Operation invertibility as a first-class requirement
- History batching rules (consecutive same-type at adjacent positions)
- "Elements must have at least one child" and similar structural invariants

**What we skip:**
- Path-based addressing (our block_index + offset is simpler for 2-level docs)
- Schema-less design (our block types are fixed, validation is cheap)
- The React rendering layer

### Quill - Delta Algebra

Fundamentally different: documents are flat operation sequences, not trees.

**Delta format:** An array of `{ insert, retain, delete }` ops with optional
`attributes`. A document is a delta with only `insert` ops. A change is a delta
with all three op types. Same data structure for state and transitions.

**Block formatting hack:** Block-level formatting (headings, lists, quotes) is
encoded as attributes on the trailing `\n` character of each line. This keeps the
format flat but makes block operations awkward - inserting block embeds requires
"implicit newline" bookkeeping.

**Algebraic operations:** `compose(a, b)` = apply b after a. `transform(a, b)` =
adjust b assuming a happened first (OT). `invert(change, base)` = compute the
undo. `diff(a, b)` = compute the change from a to b. These give you
collaboration and undo for free.

**Dual representation:** Quill maintains both a flat delta (canonical) and a blot
tree (DOM-synced). `getDelta()` derives the flat form from the tree; `applyDelta()`
mutates the tree from the flat form. The `Editor` class is a bidirectional sync
engine between these representations.

**Tables expose the limit:** Quill has two table implementations - one using
line-level formatting (fragile) and one embedding sub-deltas inside block embeds
(essentially admitting the flat model can't handle 2D structure).

**History:** Stores inverted deltas. Consecutive text changes within a time window
(1s) compose into a single undo entry. External changes transform the history
stack via OT.

**What we take:**
- The insight that email is "flat-ish" - paragraphs, headings, lists, inline
  formatting. We don't need ProseMirror's full tree generality.
- History batching by time window (complement our "consecutive same-type" rule)

**What we skip:**
- The delta format itself (block-as-newline-attribute is a hack we don't want)
- OT/collaboration primitives (not needed for single-user email compose)
- The dual delta/blot representation

### Fleather - Native Flutter Editor (No ContentEditable)

The only reference project that solves the full stack on a declarative UI
framework without a browser. Flutter's `RenderBox` + `TextPainter` is analogous
to iced's `Widget` + `Paragraph`.

**Document model:** Quill Delta-based, in the `parchment` package. Tree of
`RootNode` → `LineNode`/`BlockNode` → `TextNode`/`EmbedNode`. Linked list
backing for O(1) split/merge. Each line's length includes a trailing `\n`.
Attributes split into inline scope (bold, italic, link) and line scope (heading,
block type, indent). Dual representation: node tree + flat Delta kept in sync.

**Heuristic rules engine (the key innovation for us):** 15 pure functions in a
chain-of-responsibility pattern. 9 insert rules, 3 delete rules, 3 format rules.
Every user action passes through the rules chain before becoming operations.
Rules encode all "what should happen when" behavior:
- Auto-exit block on double-Enter at end of list
- Heading resets to paragraph when splitting at end
- Inline style inheritance (typing in bold continues bold)
- Link boundary exclusivity (typing at edge doesn't extend link)
- Block style preservation when splitting lines in a list
- Embed isolation (block embeds must get their own line)

**Rendering:** Cascading `RenderBox` hierarchy mirroring the document tree.
`RenderEditor` → `RenderEditableTextBlock` → `RenderEditableTextLine` →
`RenderParagraphProxy` → Flutter's `RenderParagraph`. Each level subtracts its
layout offset and delegates for both hit testing and caret positioning.

**Text input:** Implements `DeltaTextInputClient`. On focus, connects to
platform IME and syncs current text + selection. Receives delta events
(insertions, deletions, replacements) back from the platform. Maps each to a
document operation. This is how native text input works without contentEditable.

**Run splitting:** `LeafNode.isolate(index, length)` splits a text node twice to
extract the affected range, applies the style change, then `optimize()` merges
adjacent same-style runs. This split-apply-merge pattern is universal across all
editors.

**What we take:**
- Heuristic rules as a formal system (`rules.rs`) - the most portable part
- Cascading hit test pattern for rendering ↔ document mapping
- Run splitting primitives (`split_at`, `isolate`)
- Text input strategy: sync text/selection to platform, receive deltas back
- Vec is fine for runs (email paragraphs rarely exceed ~10 styled runs)

**What we skip:**
- Quill Delta format (we use our own block tree)
- Linked list for node children (Vec is sufficient for our scale)
- Flutter-specific rendering and platform abstractions

### Architecture Comparison

| Aspect | ProseMirror | Slate | Quill | Fleather | **Ours** |
|--------|-------------|-------|-------|----------|----------|
| Document | Immutable tree | Mutable tree | Flat delta | Delta + node tree | **Immutable 2-level tree** |
| Schema | DFA content exprs | None (normalize) | None (blot) | None (heuristics) | **Assertions in normalize()** |
| Marks | Sorted arrays | Text node props | Delta attributes | Parchment attrs | **Bitflags on runs** |
| Positions | Flat integer | Path (indices) | Flat integer | Offset in tree | **block_index + char offset** |
| Edits | Steps | 9 operations | Delta compose | Delta compose | **EditOp enum** |
| Undo | Steps + mirror | Inverse ops | Inverted deltas | Delta history | **Inverse ops + PosMap** |
| Validation | Schema DFA | Normalization | Blot constraints | Heuristic rules | **Normalize + rules** |
| Edit behavior | Commands | Transforms | Modules | Heuristic rules | **rules.rs chain** |
| Rendering | contentEditable | React + cE | contentEditable | Flutter RenderBox | **iced Paragraph** |
| Input | MutationObserver | beforeinput | contentEditable | DeltaTextInputClient | **iced events + IME** |

---

## What We Steal

| Piece | Source | Used in |
|-------|--------|---------|
| Position mapping through edits (StepMap) | ProseMirror `prosemirror-transform/src/map.ts` | `operations.rs` (PosMap) |
| Slice with open depths for clipboard | ProseMirror `prosemirror-model/src/replace.ts` | `document.rs` (DocSlice) |
| Immutable nodes with structural sharing | ProseMirror `prosemirror-model/src/node.ts` | `document.rs` (Arc-wrapped blocks) |
| Operation invertibility pattern | Slate `packages/slate/src/interfaces/operation.ts` | `operations.rs` (EditOp::invert) |
| Normalization with dirty tracking | Slate `packages/slate/src/editor/normalize.ts` | `document.rs` (normalize pass) |
| History batching (consecutive same-type ops) | Slate `packages/slate-history/src/with-history.ts` | `operations.rs` (UndoStack) |
| Structural invariants (min 1 child, etc.) | Slate `packages/slate/src/core/normalize-node.ts` | `normalize.rs` (structural rules) |
| Heuristic rules engine (chain of responsibility) | fleather `packages/parchment/lib/src/heuristics.dart` | `rules.rs` (insert/delete/format rules) |
| Auto-exit block, heading reset, style inheritance | fleather `packages/parchment/lib/src/heuristics/insert_rules.dart` | `rules.rs` (insert rules) |
| Line merge style, embed protection, doc minimum | fleather `packages/parchment/lib/src/heuristics/delete_rules.dart` | `rules.rs` (delete rules) |
| Cascading hit test (editor → block → line → paragraph) | fleather `packages/fleather/lib/src/rendering/editor.dart` | `widget/cursor.rs` (position mapping) |
| Run split/isolate/optimize pattern | fleather `packages/parchment/lib/src/document/leaf.dart` | `operations.rs` (run splitting) |
| Platform IME text/selection sync | fleather `packages/fleather/lib/src/widgets/editor_input_client_mixin.dart` | `widget/input.rs` (text input strategy) |
| Selection rendering (grapheme positions) | halloy `selectable_rich_text.rs` | `widget/cursor.rs` |
| ChildData bitflags for DOM traversal | frostmark `renderer.rs` | `html_parse.rs` |
| Block/inline element classification | frostmark `renderer.rs` | `html_parse.rs` |
| Grapheme-aware word navigation | libcosmic `value.rs` | `widget/input.rs` |
| Key binding dispatch pattern | iced `text_editor.rs` | `widget/input.rs` |
| Paragraph::with_spans rendering | iced `rich.rs` | `widget/render.rs` |
| Focus/blink state machine | iced `text_editor.rs` | `widget/mod.rs` |
