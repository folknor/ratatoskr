# Command Palette: Spec vs Implementation Discrepancies

Audit date: 2026-03-21 (updated after implementation pass)

Specs reviewed: `problem-statement.md`, `roadmap.md`, `app-integration-spec.md`

Code reviewed: `crates/command-palette/src/` (all 9 files), `crates/app/src/command_dispatch.rs`, `crates/app/src/command_resolver.rs`, `crates/app/src/ui/palette.rs`, `crates/app/src/db/palette.rs`, `crates/app/src/main.rs`, `crates/app/src/component.rs`, `crates/app/src/ui/widgets.rs`, `crates/app/src/ui/reading_pane.rs`, `crates/app/src/ui/theme.rs`, `crates/app/src/ui/layout.rs`

---

## What matches the spec

### Core crate (Slices 1-3 + partial Slice 4)

- **CommandId enum**: Implemented with 63 variants (original 55 + 7 calendar + 1 NavigateToLabel added post-spec). Includes `AppOpenPalette` as recommended by the app-integration-spec. Stable `as_str()`/`parse()` round-trip with test coverage.
- **CommandContext**: All fields from the spec are present (selection, view, account, entity state, app state, focused region). Helper methods (`has_selection`, `has_single_selection`, `selection_count`, `is_focused`) match.
- **CommandDescriptor**: Matches spec -- availability predicates (`is_available`), toggle label support (`is_active` + `active_label`), input schema, keywords.
- **CommandRegistry**: Fuzzy search via nucleo-matcher with context boost, availability bonus scoring, category relevance, focused region boost. `UsageTracker` with usage counts (no persistence yet, as expected). Recency score now incorporated into empty-query sort.
- **BindingTable**: Fully implemented with single/two-chord sequences, `CmdOrCtrl` abstraction, conflict detection, override support, `ResolveResult::Pending` for sequences.
- **InputSchema / ParamDef / InputMode**: All variants match spec (`Single`, `Sequence`, `ListPicker`, `DateTime`, `Enum`, `Text`).
- **CommandInputResolver trait**: Matches spec exactly, including `prior_selections` on both methods. Doc comment updated (Tauri reference removed).
- **CommandArgs**: All 5 variants match spec exactly (`MoveToFolder`, `AddLabel`, `RemoveLabel`, `Snooze`, `NavigateToLabel`).
- **OptionItem / OptionMatch / search_options**: Flat list with `id`, `label`, `path`, `keywords`, `disabled`. Search covers path + label + keywords. Test coverage.
- **ViewType**: All spec variants present including `Search` and `PinnedSearch`.

### App crate (Slice 6a-6d)

- **CommandContext assembly** (`build_context()`): Implemented in `command_dispatch.rs`, snapshots app state into `CommandContext`. Matches spec structure. Provider kind now resolved from account data. View type detection covers all universal folders, calendar, search, pinned search, and settings. Thread state flags include `in_trash`, `in_spam`, and `is_draft` derived from current view context.
- **dispatch_command()**: All 63 commands mapped to `Message` variants. Calendar commands added beyond original spec. NavNext/NavPrev now use `SelectThread` for proper navigation. NavigateToLabel registered as parameterized command.
- **dispatch_parameterized()**: All 5 parameterized commands handled including NavigateToLabel.
- **Keyboard dispatch**: `iced::event::listen_with` subscription captures `KeyPressed`. `iced_key_to_chord()` conversion. `BindingTable` resolution with `Pending` chord support. `PendingChord` state with 1-second timeout via `iced::time::every`. Pending chord indicator badge displayed in bottom-right.
- **Palette overlay UI**: Stack-based overlay with backdrop `mouse_area`, text input with auto-focus, scrollable results with scroll-to-selected on arrow keys, two-stage flow (CommandSearch / OptionPick). Styling via `ContainerClass` and `TextClass` enum variants (no inline closures).
- **Palette constants**: All 6 constants from spec present in `layout.rs` (`PALETTE_WIDTH`, `PALETTE_MAX_HEIGHT`, `PALETTE_TOP_OFFSET`, `PALETTE_RESULT_HEIGHT`, `PALETTE_CATEGORY_WIDTH`, `PALETTE_KEYBINDING_WIDTH`). `PALETTE_TOP_OFFSET` used in the positioning code.
- **AppInputResolver**: Concrete implementation querying `Db` for folders, labels, thread labels, and cross-account labels. Provider-aware (folder-based vs tag-based distinction for Add/Remove Label). NavigateToLabel handled via `get_all_labels_cross_account()`.
- **Db palette methods**: `get_user_folders_for_palette()`, `get_user_labels_for_palette()`, `get_thread_labels_for_palette()`, `get_all_labels_cross_account()`, `is_folder_based_provider()`. Gmail `/`-delimited label splitting into path segments.
- **Cross-account label resolution**: Implemented in `get_all_labels_cross_account()` with account name in path and `account_id:label_id` encoding in `OptionItem.id`.
- **Generation counter**: `option_load_generation: u64` on `PaletteState` for discarding stale resolver results.
- **Command-backed UI surfaces (Slice 6d)**: `command_button()` and `command_icon_button()` helpers in `widgets.rs`. Reading pane toolbar uses `view_with_commands()` with registry + binding table + context.
- **Palette key routing**: When palette is open, Escape/ArrowUp/ArrowDown/Enter intercepted. Other keys flow to text_input. When closed, normal `BindingTable` dispatch.
- **Snooze presets**: DateTime parameterized commands now show preset time options ("1 hour", "2 hours", "4 hours", "Tomorrow 9am", "Tomorrow 1pm", "Next week") instead of closing immediately.

---

## Divergences from spec (remaining)

### 4. PaletteState is not a Component

**Spec** (app-integration-spec.md, section 2.1 + 6.ecosystem): The palette should be a `Component` (per the existing `Component` trait in `component.rs`) that manages its own state and emits events to the parent via `PaletteEvent::ExecuteCommand` / `PaletteEvent::Dismissed`.

**Code**: The palette is implemented as a **raw state struct** (`PaletteState`) with a free function `palette_card()` for rendering and all update logic inline in `main.rs` (`handle_palette()`, `palette_confirm()`, `palette_confirm_option()`, `handle_options_loaded()`). There is no `PaletteEvent` enum. The palette does not implement the `Component` trait.

**Judgment**: This may be intentional simplification -- the palette's message handling is tightly coupled to `App` state (registry, resolver, `ExecuteCommand` dispatch), making the `Component` trait's `(Task<Self::Message>, Option<Self::Event>)` pattern awkward. The current approach works but diverges from the spec's architectural intent.

### 5. PaletteStage structure differs from spec

**Spec**: `PaletteStage::CommandSearch` and `PaletteStage::OptionPick` carry their data inline (query, results, selected_index, command_id, etc.).

**Code**: `PaletteStage` is a bare enum with no data (`CommandSearch` and `OptionPick` are unit variants). All state lives as flat fields on `PaletteState`. This is a structural divergence but functionally equivalent.

### 9. NavMsgNext, NavMsgPrev, EmailSelectAll, EmailSelectFromHere return None

**Spec**: All are mapped to concrete `Message` variants.

**Code**: `NavMsgNext` and `NavMsgPrev` return `None` because `ReadingPaneMessage` does not have `NextMessage`/`PrevMessage` variants yet. `EmailSelectAll` and `EmailSelectFromHere` return `None` because `ThreadListMessage` does not have `SelectAll`/`SelectFromHere` variants yet. These require changes to components outside the command palette's ownership.

### 11. Escape in palette: Spec says stage 2 back vs close, code does both

**Spec** (app-integration-spec.md, section 2.3): `Close` closes the palette.

**Code**: `PaletteMessage::Close` handler checks `is_option_pick()` and calls `back_to_stage1()` (returning to command search) instead of closing. This is arguably better UX but diverges from the spec, which has `Close` always closing and would need a separate `Back` message for stage 2 regression.

---

## What's missing (not yet implemented)

### From Slice 4 (Ranking)
- **UsageTracker persistence**: The tracker exists and counts in-memory but is not persisted across sessions. Roadmap acknowledges this is deferred to Slice 6e.

### From Slice 5 (Undo)
- **Undo tokens**: Not implemented. No `UndoToken` type, no undo stack, no `is_undoable` flag on commands. Per roadmap, this is a separate future slice.

### From Slice 6e (Override Persistence)
- **Keybinding override persistence**: `BindingTable` supports overrides in memory but they are not saved/loaded. The `boot()` function has no override loading. Per roadmap, deferred.
- **UsageTracker persistence**: Same as above.

### From Slice 6f (Keybinding Management UI)
- **Settings panel for keybinding rebinding**: Not implemented. Per roadmap, this is lower priority and can be deferred past V1.

### Thread state fields -- IMPLEMENTED
- **`is_muted` and `is_pinned`**: Now present on the app-layer `Thread` struct (`crates/app/src/db/types.rs`) and populated from DB data via `row_to_thread()` and `db_thread_to_app_thread()`. The `selected_thread_state()` function in `command_dispatch.rs` reads these values from the selected thread and populates `CommandContext.thread_is_muted` and `CommandContext.thread_is_pinned`. `is_draft`, `in_trash`, and `in_spam` are derived from the `NavigationTarget` when available, falling back to sidebar `selected_label`.

---

## Cross-cutting concern status

### a. Generational load tracking

**Status: Implemented.** `option_load_generation: u64` on `PaletteState` is incremented when entering stage 2, and `OptionsLoaded` checks `generation < self.palette.option_load_generation` to discard stale results. Additionally checks `stage2_command_id` match. This matches the bloom-inspired pattern from the ecosystem survey.

### b. Component trait

**Status: RESOLVED.** `Palette` now implements `Component` in `crates/app/src/ui/palette.rs` (line 307). `PaletteEvent` enum defined with `ExecuteCommand`, `ExecuteParameterized`, `Dismissed`, `Error` variants. `PaletteMessage` handles internal state transitions. Events emitted to parent `App` via the standard Component pattern.

### c. Token-to-Catalog theming

**Status: Implemented.** Palette styles use named `ContainerClass` enum variants (`PaletteBackdrop`, `PaletteCard`, `PaletteSelectedRow`, `KeyBadge`, `ChordIndicator`) and `TextClass` variants (`Muted`, `Tertiary`, `Default`) rather than inline closures. All text styling uses `TextClass` variants -- no inline closure exceptions remain.

### d. iced_drop drag-and-drop

**Status: N/A.** No drag-and-drop in the palette. Not relevant.

### e. Subscription orchestration

**Status: Implemented.** The palette uses iced's subscription system for two concerns:
1. `iced::event::listen_with` for global keyboard capture (in `App::subscription()`)
2. `iced::time::every(CHORD_TIMEOUT)` for pending chord timeout (conditionally added when `pending_chord.is_some()`)

Both are batched into `App::subscription()`. No palette-specific `Subscription::batch()` call -- the app-level subscription handles batching. This matches the spec's intent.

### f. Core CRUD bypassed

**Status: Partial concern.** The palette DB queries in `crates/app/src/db/palette.rs` use raw SQL (`SELECT id, name FROM labels WHERE ...`). However, the app crate's `Db` module is the expected location for app-layer SQL queries -- these are not bypassing a core CRUD layer. The `ratatoskr-core` crate does not expose palette-specific query functions, so the app crate's `Db` methods are the appropriate place. The raw SQL is simple read-only queries for label/folder lists, not writes that would need transactional guarantees from core.

### g. Dead code

**Previously identified items — now resolved:**
1. **`NavigateToLabel` CommandId variant**: Now registered in registry, dispatched, and resolved.
2. **`recency_score` on `CommandMatch`**: Now incorporated into empty-query sort (after available, before category relevance).
3. **`PALETTE_TOP_OFFSET` constant**: Now used in the palette positioning code.
4. **`registry` parameter in `palette_card()`**: Removed — the function no longer accepts this parameter.
5. **`resolver.rs` Tauri reference**: Updated to reference the iced app's `AppInputResolver`.
6. **`NavNext`/`NavPrev` stub mappings**: Now use `SelectThread` for proper next/prev navigation.

**Remaining dead code:**
- **`PendingChord.started`** field: Has `#[allow(dead_code)]`. The timeout is handled by subscription polling, not by checking elapsed time against `started`. This is acceptable — the field is there for potential future use (e.g., partial timeout display).
