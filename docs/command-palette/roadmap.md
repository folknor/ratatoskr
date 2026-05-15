# Command Palette: Implementation Roadmap

Status snapshot dated 2026-05-15. The backend (`crates/cmdk/`) and app integration (`crates/app/`) are largely shipped; this doc tracks what's left.

## Slice 1: Registry, Context, Fuzzy Search

**Status:** Done.

- `CommandId` enum, currently **70 variants** across Navigation, Email, Compose, Tasks, View, Calendar, App, Undo, Smart Folders, search-index rebuild. (Started at 55; grew through calendar + select-range + smart-folder + undo + rebuild work.)
- `CommandContext` with `ProviderKind`, `ViewType`, `FocusedRegion`.
- `CommandDescriptor` with availability predicates, toggle labels, palette labels, input mode.
- `CommandRegistry::query()` with nucleo-matcher fuzzy search.

Files: `crates/cmdk/src/{id,context,descriptor}.rs`, `crates/cmdk/src/registry/{core,scoring,nav,email,compose,tasks,view,calendar,app,smart_folders}.rs`.

## Slice 2: Parameterized Commands

**Status:** Done.

- Input model: `EnumOption`, `ParamDef`, `InputSchema`, `InputMode`, `OptionItem`, `OptionMatch`, `search_options()`.
- `CommandInputResolver` trait with sequence-aware `prior_selections`.
- `CommandArgs` enum (7 variants): `MoveToFolder`, `AddLabel`, `RemoveLabel`, `Snooze`, `NavigateToFolder`, `NavigateToTag`, `SmartFolderSave`. The spec's single `NavigateToLabel` split into `NavigateToFolder` (typed `FolderId`) + `NavigateToTag` (typed `TagId`) to match the typed-IDs convention; `CommandId::NavigateToLabel` dispatches to whichever the resolver produces.
- Concrete `AppInputResolver` in `crates/app/src/command_resolver.rs` queries DB for folders, labels, and cross-account options.

Files: `crates/cmdk/src/{input,resolver,args}.rs`, `crates/app/src/command_resolver.rs`, `crates/app/src/db/palette.rs`.

## Slice 3: Keybinding Model

**Status:** Done.

- `Key`, `NamedKey`, `Modifiers` (with `CmdOrCtrl`), `Chord`, `KeyBinding` (single + two-chord sequence), `Platform`.
- `BindingTable` with defaults, overrides (`Option<KeyBinding>` for explicit unbind), O(1) reverse index, sequence resolution, conflict detection.
- Canonical serde format; platform-resolved display strings.
- All 70 command registrations carry `KeyBinding` constructors.

Files: `crates/cmdk/src/keybinding.rs`.

## Slice 4: Ranking Signals

**Status:** Mostly done. One gap.

What's wired into `CommandRegistry::query()` today (`crates/cmdk/src/registry/core.rs`):

- **Empty query** (`query_empty`): sorts by availability → recency (`UsageTracker::usage_count`) → `category_relevance` → alphabetical.
- **Fuzzy query** (`query_fuzzy`): `score = raw_fuzzy + context_boost + availability_bonus(1000)`; sorted by score descending. `context_boost` combines category relevance and `focused_region_boost`.
- **Aliases / keywords**: folded into the haystack via `build_command_haystack(category, label, keywords)`, so alias hits surface through fuzzy score.

What's missing:

- **Recency in fuzzy results.** `query_fuzzy` reads `recency_score` into each `CommandMatch` but does not add it to the sort key — recency only orders the empty-query case. Folding usage count (or a log-scaled function of it) into the fuzzy score is the remaining work.

## Slice 5: Undo

**Status:** Done (backend + app dispatch wired).

- `UndoStack<T>` bounded FIFO (capacity 20) and `UndoEntry<T>` in `crates/cmdk/src/undo.rs`.
- `CommandId::Undo` registered with `Ctrl+Z`. `is_undoable` flag on descriptors; 13 email commands marked undoable.
- `App.undo_stack` real; `dispatch_plan_with_undo` (in `crates/app/src/handlers/commands.rs`) pushes inverse plans and runs them via `Message::UndoCompleted` with toast + nav + thread-list reload.
- Cross-account bulk undo splits per account and pushes one stack entry per split — minor UX wart (N presses for an N-account bulk action) noted in the dispatch comments.

## Slice 6a: Infrastructure + Keyboard Dispatch

**Status:** Done.

- `command_dispatch.rs`: `build_context()`, `dispatch_command()` (covers 64 commands; returns `None` for 5 parameterized + 1 unimplemented `AppAskAi`), `dispatch_parameterized()`, iced↔cmdk key conversion.
- `KeyEventMessage`, global keyboard subscription, pending-chord state with 1s timeout and transient `"g..."` indicator.
- Registry + `BindingTable` initialized in `App::boot()`.
- `AppOpenPalette` registered with `Ctrl+K`.

## Slice 6b/6c: Palette UI (Stages 1 + 2)

**Status:** Done.

- `PaletteState` as a `Component` (~750 lines, `crates/app/src/ui/palette.rs`).
- Stage 1 command search with availability indicators, category badges, keybinding hints.
- Stage 2 option pick with async resolver load, generation tracking for stale-result discard, label + path breadcrumb rendering, cross-account account-name prefix.
- Stack-based overlay with backdrop `mouse_area` for click-outside-to-close, auto-focus on open.
- Snooze preset DateTime options surfaced via the resolver.

Known divergences (intentional or blocked):
- `PaletteStage` is a bare unit enum; query/results/selection live as flat fields on `Palette`. Functionally equivalent to the spec's data-carrying variant.
- Escape in stage 2 calls `back_to_stage1()` instead of closing — better UX, kept on purpose.
- `scroll_to_selected()` is a no-op (`crates/app/src/ui/palette.rs:620`). Blocked on the iced fork not exposing `scrollable::scroll_to()`. Arrow keys move the selection index but the scrollable doesn't follow.

## Slice 6d: Command-Backed UI Surfaces

**Status:** Partial.

- `command_button` and `command_icon_button` helpers live in `crates/app/src/ui/widgets/buttons.rs`.
- Reading-pane toolbar uses `command_icon_button` for 5 actions (`crates/app/src/ui/reading_pane.rs`).
- No context-menu / right-click integration yet.
- Other toolbars (thread list, sidebar) have not been migrated.

## Slice 6e: Override + Usage Persistence

**Status:** Done.

- `keybindings.json` and `command_usage.json` under the app data dir.
- Loaded in `App::boot()`; saved on mutation in `crates/app/src/handlers/commands.rs`.

## Slice 6f: Keybinding Management UI

**Status:** Not started. Deferred past V1 per the original spec.

A settings panel for viewing/searching/rebinding shortcuts with conflict detection. No analogue exists in `crates/app/src/ui/settings/`. Default bindings work out of the box, so this is low-priority.

## Remaining Work Summary

1. **Fold recency into the fuzzy score** (Slice 4 gap).
2. **`scroll_to_selected()`** — needs `scrollable::scroll_to()` on the iced fork.
3. **Slice 6d expansion** — context menus, additional toolbars.
4. **Slice 6f** — keybinding management UI (deferred).
5. **`AppAskAi`** — dispatch stub returns `None`; waits on the Ask AI feature itself.
