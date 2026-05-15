# Command Palette: Status

Snapshot dated 2026-05-15. The backend (`crates/cmdk/`) and app integration (`crates/app/`) are largely shipped; this doc tracks per-slice state and what's left.

For design rationale, see `design.md`.

## Per-slice status

### Slice 1: Registry, Context, Fuzzy Search - Done

- `CommandId` enum, currently **70 variants** across Navigation, Email, Compose, Tasks, View, Calendar, App, Undo, Smart Folders, search-index rebuild. (Started at 55; grew through calendar + select-range + smart-folder + undo + rebuild work.)
- `CommandContext` with `ProviderKind`, `ViewType`, `FocusedRegion`.
- `CommandDescriptor` with availability predicates, toggle labels, palette labels, input mode.
- `CommandRegistry::query()` with nucleo-matcher fuzzy search.

Files: `crates/cmdk/src/{id,context,descriptor}.rs`, `crates/cmdk/src/registry/{core,scoring,nav,email,compose,tasks,view,calendar,app,smart_folders}.rs`.

### Slice 2: Parameterized Commands - Done

- Input model: `EnumOption`, `ParamDef`, `InputSchema`, `InputMode`, `OptionItem`, `OptionMatch`, `search_options()`.
- `CommandInputResolver` trait with sequence-aware `prior_selections`.
- `CommandArgs` enum (7 variants): `MoveToFolder`, `AddLabel`, `RemoveLabel`, `Snooze`, `NavigateToFolder`, `NavigateToLabel`, `SmartFolderSave`. Folder navigation carries `FolderId`; label navigation carries `LabelId`; `CommandId::NavigateToLabel` dispatches to whichever the resolver produces.
- Concrete `AppInputResolver` in `crates/app/src/command_resolver.rs` queries DB for folders, labels, and cross-account options.

Files: `crates/cmdk/src/{input,resolver,args}.rs`, `crates/app/src/command_resolver.rs`, `crates/app/src/db/palette.rs`.

### Slice 3: Keybinding Model - Done

- `Key`, `NamedKey`, `Modifiers` (with `CmdOrCtrl`), `Chord`, `KeyBinding` (single + two-chord sequence), `Platform`.
- `BindingTable` with defaults, overrides (`Option<KeyBinding>` for explicit unbind), O(1) reverse index, sequence resolution, conflict detection.
- Canonical serde format; platform-resolved display strings.
- All 70 command registrations carry `KeyBinding` constructors.

Files: `crates/cmdk/src/keybinding.rs`.

### Slice 4: Ranking Signals - Done

Wired into `CommandRegistry::query()` (`crates/cmdk/src/registry/core.rs`):

- **Empty query** (`query_empty`): availability -> raw recency count -> `category_relevance` -> alphabetical.
- **Fuzzy query** (`query_fuzzy`): `score = raw_fuzzy + context_boost + availability_bonus(1000) + recency_bonus`. Recency log-scaled (`recency_bonus(count)` in `scoring.rs`): 1 use adds 16, 7 adds 32, 127 adds 64. Stays well under the 1000 availability bonus, so an enabled never-used command beats a disabled heavily-used one.
- **Context boost**: combines `category_relevance` and `focused_region_boost`.
- **Aliases / keywords**: folded into the haystack via `build_command_haystack(category, label, keywords)`.

### Slice 5: Undo - Done

- `UndoStack<T>` bounded FIFO (capacity 20) and `UndoEntry<T>` in `crates/cmdk/src/undo.rs`.
- `CommandId::Undo` registered with `Ctrl+Z`. `is_undoable` flag on descriptors; 13 email commands marked undoable.
- `App.undo_stack` real; `dispatch_plan_with_undo` (in `crates/app/src/handlers/commands.rs`) pushes inverse plans and runs them via `Message::UndoCompleted` with toast + nav + thread-list reload.
- Cross-account bulk undo splits per account and pushes one stack entry per split - minor UX wart (N presses for an N-account bulk action) noted in the dispatch comments and in `design.md`.

### Slice 6a: Infrastructure + Keyboard Dispatch - Done

- `command_dispatch.rs`: `build_context()`, `dispatch_command()` (covers 64 commands; returns `None` for 5 parameterized + 1 unimplemented `AppAskAi`), `dispatch_parameterized()`, iced<->cmdk key conversion.
- `KeyEventMessage`, global keyboard subscription, pending-chord state with 1s timeout and transient `"g..."` indicator.
- Registry + `BindingTable` initialized in `App::boot()`.
- `AppOpenPalette` registered with `Ctrl+K`.

### Slice 6b/6c: Palette UI (Stages 1 + 2) - Done

- `PaletteState` as a `Component` (~750 lines, `crates/app/src/ui/palette.rs`).
- Stage 1 command search with availability indicators, category badges, keybinding hints.
- Stage 2 option pick with async resolver load, generation tracking for stale-result discard, label + path breadcrumb rendering, cross-account account-name prefix.
- Stack-based overlay with backdrop `mouse_area` for click-outside-to-close, auto-focus on open.
- Snooze preset DateTime options surfaced via the resolver.
- `PaletteStage` is a bare unit enum (intentional, see `design.md`); Escape in stage 2 returns to stage 1 (intentional).

### Slice 6d: Command-Backed UI Surfaces - Partial

- `command_button` and `command_icon_button` helpers live in `crates/app/src/ui/widgets/buttons.rs`.
- Reading-pane toolbar uses `command_icon_button` for 5 actions (`crates/app/src/ui/reading_pane.rs`).
- No context-menu / right-click integration yet.
- Other toolbars (thread list, sidebar) have not been migrated.

### Slice 6e: Override + Usage Persistence - Done

- `keybindings.json` and `command_usage.json` under the app data dir.
- Loaded in `App::boot()`; saved on mutation in `crates/app/src/handlers/commands.rs`.

### Slice 6f: Keybinding Management UI - Not started

Settings panel for view/search/rebind with conflict detection. No analogue exists in `crates/app/src/ui/settings/`. Default bindings work out of the box; deferred past V1 per the original spec.

## Outstanding work

1. **`scroll_to_selected()`** - blocked on the iced fork not exposing `scrollable::scroll_to()`. Currently a no-op (`crates/app/src/ui/palette.rs:620`); arrow keys move the selection index but the scrollable doesn't follow when the highlight goes off-screen.
2. **Slice 6d expansion** - context menus (the primitive doesn't exist yet), thread-list and sidebar toolbar migrations.
3. **Slice 6f** - keybinding management UI.
4. **`AppAskAi`** - dispatch returns `None` until the Ask AI feature itself lands.
