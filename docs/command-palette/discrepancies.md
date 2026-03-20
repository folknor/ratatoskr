# Command Palette: Spec vs Implementation Discrepancies

Audit date: 2026-03-21

Specs reviewed: `problem-statement.md`, `roadmap.md`, `app-integration-spec.md`

Code reviewed: `crates/command-palette/src/` (all 9 files), `crates/app/src/command_dispatch.rs`, `crates/app/src/command_resolver.rs`, `crates/app/src/ui/palette.rs`, `crates/app/src/db/palette.rs`, `crates/app/src/main.rs`, `crates/app/src/component.rs`, `crates/app/src/ui/widgets.rs`, `crates/app/src/ui/reading_pane.rs`, `crates/app/src/ui/theme.rs`, `crates/app/src/ui/layout.rs`

---

## What matches the spec

### Core crate (Slices 1-3 + partial Slice 4)

- **CommandId enum**: Implemented with 62 variants (original 55 + 7 calendar commands added post-spec). Includes `AppOpenPalette` as recommended by the app-integration-spec. Stable `as_str()`/`parse()` round-trip with test coverage.
- **CommandContext**: All fields from the spec are present (selection, view, account, entity state, app state, focused region). Helper methods (`has_selection`, `has_single_selection`, `selection_count`, `is_focused`) match.
- **CommandDescriptor**: Matches spec -- availability predicates (`is_available`), toggle label support (`is_active` + `active_label`), input schema, keywords.
- **CommandRegistry**: Fuzzy search via nucleo-matcher with context boost, availability bonus scoring, category relevance, focused region boost. `UsageTracker` with usage counts (no persistence yet, as expected).
- **BindingTable**: Fully implemented with single/two-chord sequences, `CmdOrCtrl` abstraction, conflict detection, override support, `ResolveResult::Pending` for sequences.
- **InputSchema / ParamDef / InputMode**: All variants match spec (`Single`, `Sequence`, `ListPicker`, `DateTime`, `Enum`, `Text`).
- **CommandInputResolver trait**: Matches spec exactly, including `prior_selections` on both methods.
- **CommandArgs**: All 5 variants match spec exactly (`MoveToFolder`, `AddLabel`, `RemoveLabel`, `Snooze`, `NavigateToLabel`).
- **OptionItem / OptionMatch / search_options**: Flat list with `id`, `label`, `path`, `keywords`, `disabled`. Search covers path + label + keywords. Test coverage.

### App crate (Slice 6a-6d)

- **CommandContext assembly** (`build_context()`): Implemented in `command_dispatch.rs`, snapshots app state into `CommandContext`. Matches spec structure.
- **dispatch_command()**: All 62 commands mapped to `Message` variants. Calendar commands added beyond original spec. Matches spec pattern.
- **dispatch_parameterized()**: Implemented but incomplete (see divergences).
- **Keyboard dispatch**: `iced::event::listen_with` subscription captures `KeyPressed`. `iced_key_to_chord()` conversion. `BindingTable` resolution with `Pending` chord support. `PendingChord` state with 1-second timeout via `iced::time::every`.
- **Palette overlay UI**: Stack-based overlay with backdrop `mouse_area`, text input with auto-focus, scrollable results, two-stage flow (CommandSearch / OptionPick). Styling via `ContainerClass` enum variants.
- **Palette constants**: All 6 constants from spec present in `layout.rs` (`PALETTE_WIDTH`, `PALETTE_MAX_HEIGHT`, `PALETTE_TOP_OFFSET`, `PALETTE_RESULT_HEIGHT`, `PALETTE_CATEGORY_WIDTH`, `PALETTE_KEYBINDING_WIDTH`).
- **AppInputResolver**: Concrete implementation querying `Db` for folders, labels, thread labels. Provider-aware (folder-based vs tag-based distinction for Add/Remove Label).
- **Db palette methods**: `get_user_folders_for_palette()`, `get_user_labels_for_palette()`, `get_thread_labels_for_palette()`, `get_all_labels_cross_account()`, `is_folder_based_provider()`. Gmail `/`-delimited label splitting into path segments.
- **Cross-account label resolution**: Implemented in `get_all_labels_cross_account()` with account name in path and `account_id:label_id` encoding in `OptionItem.id`.
- **Generation counter**: `option_load_generation: u64` on `PaletteState` for discarding stale resolver results. Correctly incremented on stage 2 entry, checked on `OptionsLoaded`.
- **Command-backed UI surfaces (Slice 6d)**: `command_button()` and `command_icon_button()` helpers in `widgets.rs`. Reading pane toolbar uses `view_with_commands()` with registry + binding table + context.
- **Palette key routing**: When palette is open, Escape/ArrowUp/ArrowDown/Enter intercepted. Other keys flow to text_input. When closed, normal `BindingTable` dispatch.

---

## Divergences from spec

### 1. NavigateToLabel not registered in registry

**Spec** (roadmap.md, Slice 2): `NavigateToLabel` is listed as one of the 5 parameterized commands with input schema `Single { ListPicker { label: "Label" } }`.

**Code**: `NavigateToLabel` exists as a `CommandId` variant and has a `CommandArgs::NavigateToLabel` variant, but it is **not registered** in `register_navigation()` or any other registration function in `registry.rs`. It has no `CommandDescriptor`, so it will never appear in palette search results and has no availability predicate or keybinding.

**Impact**: The command palette cannot find or execute "Navigate to Label" -- the primary way to navigate to a label when in unified view, per the problem statement.

### 2. dispatch_parameterized missing NavigateToLabel arm

**Spec** (app-integration-spec.md, section 1.3): `dispatch_parameterized` should handle `(CommandId::NavigateToLabel, CommandArgs::NavigateToLabel { label_id, account_id })` mapping to `Message::NavigateTo(NavigationTarget::Label { label_id, account_id })`.

**Code**: The match in `dispatch_parameterized()` only handles 4 of 5 parameterized commands. `NavigateToLabel` falls through to the `_ => None` catch-all.

### 3. AppInputResolver missing NavigateToLabel handling

**Spec** (app-integration-spec.md, section 1.4): `get_options()` should handle `(CommandId::NavigateToLabel, 0)` by calling `get_all_label_options_cross_account(ctx)`.

**Code**: The `get_options()` match only handles `EmailMoveToFolder`, `EmailAddLabel`, `EmailRemoveLabel`. `NavigateToLabel` falls through to `Ok(vec![])`. The `get_all_labels_cross_account()` method exists on `Db` but is never called.

### 4. PaletteState is not a Component

**Spec** (app-integration-spec.md, section 2.1 + 6.ecosystem): The palette should be a `Component` (per the existing `Component` trait in `component.rs`) that manages its own state and emits events to the parent via `PaletteEvent::ExecuteCommand` / `PaletteEvent::Dismissed`.

**Code**: The palette is implemented as a **raw state struct** (`PaletteState`) with a free function `palette_card()` for rendering and all update logic inline in `main.rs` (`handle_palette()`, `palette_confirm()`, `palette_confirm_option()`, `handle_options_loaded()`). There is no `PaletteEvent` enum. The palette does not implement the `Component` trait.

**Judgment**: This may be intentional simplification -- the palette's message handling is tightly coupled to `App` state (registry, resolver, `ExecuteCommand` dispatch), making the `Component` trait's `(Task<Self::Message>, Option<Self::Event>)` pattern awkward. The current approach works but diverges from the spec's architectural intent.

### 5. PaletteStage structure differs from spec

**Spec**: `PaletteStage::CommandSearch` and `PaletteStage::OptionPick` carry their data inline (query, results, selected_index, command_id, etc.).

**Code**: `PaletteStage` is a bare enum with no data (`CommandSearch` and `OptionPick` are unit variants). All state lives as flat fields on `PaletteState`. This is a structural divergence but functionally equivalent.

### 6. ViewType missing Search and PinnedSearch variants

**Spec** (app-integration-spec.md, section 1.2): `ViewType` should include `Search` and `PinnedSearch` variants.

**Code**: `ViewType` has 15 variants but does not include `Search` or `PinnedSearch`. It does include `Category` and `Calendar` which are not in the spec's list.

**Judgment**: Likely intentional evolution -- Calendar was added post-spec, and Search/PinnedSearch are deferred features.

### 7. Resolver trait doc references Tauri

**Spec**: The problem statement says Tauri has been removed entirely.

**Code**: `crates/command-palette/src/resolver.rs` line 11 still says `"Tauri app: TauriInputResolver queries DbState..."`. This is a stale doc comment.

### 8. NavNext/NavPrev are stubs in dispatch

**Spec** (app-integration-spec.md, section 1.3): `NavNext` maps to `Message::ThreadList(ThreadListMessage::SelectNext)`, `NavPrev` maps to `SelectPrev`.

**Code**: Both `NavNext` and `NavPrev` map to `Message::NavigateTo(NavigationTarget::Inbox)` with a `// stub` comment. They do not perform next/prev navigation.

### 9. NavOpen, NavMsgNext, NavMsgPrev, EmailSelectAll, EmailSelectFromHere return None

**Spec**: All are mapped to concrete `Message` variants.

**Code**: All 5 return `None` from `dispatch_command()`, meaning these keybindings are registered but non-functional.

### 10. Pending chord indicator not implemented

**Spec** (app-integration-spec.md, section 3.7): When `pending_chord` is `Some`, a transient indicator (e.g., `"g..."` badge) should appear in the status bar or bottom-right corner.

**Code**: The pending chord state machine works (sequences resolve correctly), but there is no visual indicator. No search results for pending chord display code in the app crate.

### 11. Escape in palette: Spec says stage 2 back vs close, code does both

**Spec** (app-integration-spec.md, section 2.3): `Close` closes the palette.

**Code**: `PaletteMessage::Close` handler checks `is_option_pick()` and calls `back_to_stage1()` (returning to command search) instead of closing. This is arguably better UX but diverges from the spec, which has `Close` always closing and would need a separate `Back` message for stage 2 regression.

---

## What's missing (not yet implemented)

### From Slice 4 (Ranking)
- **UsageTracker persistence**: The tracker exists and counts in-memory but is not persisted across sessions. Roadmap acknowledges this is deferred to Slice 6e.
- **Recency-weighted empty-query ordering**: Empty query sorts by available > category relevance > alpha. Recency is tracked in `recency_score` but the empty-query sort does not incorporate it (the `recency_score` field is set but not used in the sort comparator of `query_empty()`).

### From Slice 5 (Undo)
- **Undo tokens**: Not implemented. No `UndoToken` type, no undo stack, no `is_undoable` flag on commands. Per roadmap, this is a separate future slice.

### From Slice 6e (Override Persistence)
- **Keybinding override persistence**: `BindingTable` supports overrides in memory but they are not saved/loaded. The `boot()` function has no override loading. Per roadmap, deferred.
- **UsageTracker persistence**: Same as above.

### From Slice 6f (Keybinding Management UI)
- **Settings panel for keybinding rebinding**: Not implemented. Per roadmap, this is lower priority and can be deferred past V1.

### From Slice 6c (Snooze)
- **DateTime input for Snooze**: The spec calls for preset times ("1 hour", "Tomorrow 9am", "Next week") or a manual date picker. The code detects `ParamDef::DateTime` in `palette_confirm()` and closes the palette (no-op), commenting that DateTime is not yet implemented.

---

## Cross-cutting concern status

### a. Generational load tracking

**Status: Implemented.** `option_load_generation: u64` on `PaletteState` is incremented when entering stage 2, and `OptionsLoaded` checks `generation < self.palette.option_load_generation` to discard stale results. Additionally checks `stage2_command_id` match. This matches the bloom-inspired pattern from the ecosystem survey.

### b. Component trait

**Status: Not used for palette.** The `Component` trait exists in `crates/app/src/component.rs` with the spec's `(Task<Self::Message>, Option<Self::Event>)` return signature. The palette does not implement it -- all logic is inline in `App::handle_palette()`. No `PaletteEvent` type exists. Other components in the app (sidebar, thread list, reading pane, settings) appear to use direct message passing rather than the Component trait as well, so this may reflect a broader architectural decision rather than a palette-specific omission.

### c. Token-to-Catalog theming

**Status: Implemented.** Palette styles use named `ContainerClass` enum variants (`PaletteBackdrop`, `PaletteCard`, `PaletteSelectedRow`, `KeyBadge`) and `TextClass` variants (`Muted`, `Tertiary`) rather than inline closures. Each has a `.style()` method returning a function pointer. This matches the token-to-catalog pattern.

**Exception**: `option_result_row()` uses an inline closure for the default (non-disabled) label style: `|_theme: &iced::Theme| text::Style { color: None }`. Same pattern in `palette_result_row()` for available label style.

### d. iced_drop drag-and-drop

**Status: N/A.** No drag-and-drop in the palette. Not relevant.

### e. Subscription orchestration

**Status: Implemented.** The palette uses iced's subscription system for two concerns:
1. `iced::event::listen_with` for global keyboard capture (in `App::subscription()`)
2. `iced::time::every(CHORD_TIMEOUT)` for pending chord timeout (conditionally added when `pending_chord.is_some()`)

Both are batched into `App::subscription()`. No palette-specific `Subscription::batch()` call -- the app-level subscription handles batching. This matches the spec's intent.

### f. Core CRUD bypassed

**Status: Partial concern.** The palette DB queries in `crates/app/src/db/palette.rs` use raw SQL (`SELECT id, name FROM labels WHERE ...`). However, the app crate's `Db` module is the expected location for app-layer SQL queries -- these are not bypassing a core CRUD layer. The `ratatoskr-core` crate does not expose palette-specific query functions, so the app crate's `Db` methods are the appropriate place. The raw SQL is simple read-only queries for label/folder lists, not writes that would need transactional guarantees from core.

**Note**: The spec (app-integration-spec.md, section 1.4) does call for `Db` methods like `get_user_folders_for_palette()`, which is exactly what was implemented. No bypass concern.

### g. Dead code

**Identified items:**
1. **`NavigateToLabel` CommandId variant**: Exists in the enum, has a `CommandArgs` variant, but is never registered in the registry, never dispatched, and the resolver never handles it. The `Db::get_all_labels_cross_account()` method exists but is unreachable.
2. **`recency_score` on `CommandMatch`**: Set in both `query_empty()` and `query_fuzzy()` but never used in sorting. The empty-query sort uses `available > category_relevance > category > label` but not recency.
3. **`PALETTE_TOP_OFFSET` constant** (`layout.rs`): Defined but the actual padding in `main.rs` uses an inline `[80, 0, 0, 0]` rather than referencing this constant.
4. **`PALETTE_INPUT_HEIGHT` constant**: Referenced in the spec but does not exist in `layout.rs`. The text input height is determined by padding + font size rather than an explicit constant.
5. **`registry` parameter in `palette_card()`**: Passed but immediately discarded with `let _ = registry;`. The comment says "Used for future enrichment; results are pre-queried." This is dead code today.
6. **`resolver.rs` Tauri reference**: Stale documentation referencing a removed architecture.
7. **`NavNext`/`NavPrev` stub mappings**: Map to `NavigateTo(Inbox)` which is incorrect behavior -- effectively dead as navigation commands.
