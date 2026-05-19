# Command Palette: Design

The command palette is **the** action dispatch layer for the entire app. Every user-initiated action - keyboard shortcut, button click, menu item, palette selection - is a registered command. There is no way to create an action that isn't part of the registry.

This file is the load-bearing design rationale. For per-slice status and outstanding work, see `status.md`. For day-to-day code: registry in `crates/cmdk/`, app integration in `crates/app/src/{command_dispatch,command_resolver,ui/palette}.rs`.

## Three command tiers

The palette has to handle three distinct shapes of action:

1. **Universal** - always available, no context required (compose, open settings, toggle sidebar).
2. **Context-sensitive static** - statically known but availability depends on app state (archive, reply, delete-draft).
3. **Parameterized (two-stage)** - static command, selecting it opens a second stage populated at runtime (move-to-folder, add-label, navigate-to-label, snooze).

Tier 3 drives the registry/resolver split: the static command list is a `CommandRegistry`; the runtime option list comes from a separate `CommandInputResolver` trait the app implements with DB access. Core defines the contract; the app layer holds live state.

## Cross-account label/folder disambiguation

Ratatoskr is multi-account across four providers. The unified ("All Accounts") sidebar deliberately omits per-account labels - they're provider-specific noise in a cross-account context. **The palette becomes the primary way to navigate to a label or folder when in unified view.** The unified sidebar's removability of per-account labels is contingent on the palette doing this well.

The mechanism is `OptionItem`: a flat list (no trees) with `id`, `label`, optional `path: Vec<String>`, optional `keywords`, and `disabled`. In a single-account scope `path` carries hierarchy. In All Accounts the resolver prefixes `path` with the account display name. Fuzzy search runs over `label`, joined `path` segments, and `keywords`, so "work clients" finds "Clients" under "Work Exchange" without the user knowing the exact hierarchy. The UI reconstructs hierarchical display from `path`. Execution uses `id`. System folders (`INBOX`, `SENT`, etc.) are excluded - those have universal sidebar destinations and dedicated palette commands.

## Stage separation in the execution contract

`CommandRegistry::query()` is stage-1 only - it returns top-level commands, never second-stage options. Parameterized commands appear in query results with `input_mode: InputMode::Parameterized { schema }`; stage 2 is driven separately via the resolver. This keeps the search APIs cleanly typed and lets non-palette surfaces (context menus, toolbars) execute commands without going through stage 2 themselves.

The execution payload is a typed `CommandArgs` enum (one variant per parameterized command), not `serde_json::Value`. Each variant carries exactly the typed fields that command needs (`FolderId`, `LabelGroupId`, `i64` timestamp, `String`). The compiler enforces exhaustive matching in the dispatch function.

## Identity vs descriptor

`CommandId` is a Rust enum (~71 variants today, growing). The compiler enforces that every command is handled. Adding a new user-facing action means adding a variant - intentional friction, since you have to wire it in `dispatch_command`, register a descriptor, and consider a default keybinding.

`CommandDescriptor` is the runtime, context-resolved view: resolved label (with toggle text like Star/Unstar), resolved keybinding (user overrides applied), current availability (predicate evaluated against `CommandContext`), category, parameter schema. The palette UI and keyboard dispatch consume descriptors, not raw IDs.

## CommandContext

The registry needs a context snapshot to evaluate availability and resolve toggle labels. The app builds this on every query. It carries:

- selected thread IDs and active message ID
- current view type and label/folder ID
- active account ID and provider kind
- entity flags (read/starred/muted/pinned/draft/in-trash/in-spam)
- app state (online, composer open)
- focused UI region (thread list, reading pane, sidebar, composer)

`current_view` is an explicit `ViewType` field on `App`, set by navigation actions - not derived heuristically from sidebar state. This was an app-state normalization required by the integration.

## Keybindings as a property of the command

Every command has a default keybinding (or none) declared at registration. User overrides live in a `CommandId -> KeyBinding` map, persisted to `keybindings.json`. The registry resolves bindings (overrides -> defaults), supports two-key sequences (`g then i`), platform-specific defaults (`Cmd` vs `Ctrl`), and conflict detection.

The palette UI displays the resolved binding next to each command, derived from the `BindingTable` rather than hardcoded.

## Ranking beyond fuzzy score

Fuzzy match score alone isn't sufficient. The registry combines:

- **Recency** - `UsageTracker` records per-command usage counts, persisted to `command_usage.json`. Folded into both empty-query ordering and the fuzzy score (log-scaled `recency_bonus` so high-frequency commands tip ties without dominating).
- **Context boost** - categories aligned with the current view get a score bump; focused-region alignment adds more.
- **Availability bonus** - enabled commands always outrank disabled ones with similar fuzzy scores (`+1000` to the score).
- **Aliases / keywords** - folded into the fuzzy haystack via `build_command_haystack`, so alias hits surface naturally.

Disabled commands are returned with an `available: bool` flag; the caller (palette, context menu, toolbar) decides whether to hide or grey them. Every registered command is palette-visible; there is no keyboard-only tier.

## Undo

Some commands are undoable, declared via `is_undoable` on the descriptor. Undo isn't "run another command" - most real undos need runtime state captured from the original execution (previous folder, prior toggle state, old label set). The mechanism is an **undo token**: an opaque, serializable compensation payload pushed to a bounded stack at execution time. `Ctrl+Z` pops the stack and dispatches the inverse plan. Tokens that reference ephemeral state (e.g. a permanently-deleted thread) must be invalidated.

## App integration rationale

### Why dispatch is split in two

`dispatch_command(id) -> Option<Message>` handles direct (non-parameterized) commands. `dispatch_parameterized(id, args) -> Option<Message>` handles parameterized ones once stage 2 has produced typed `CommandArgs`. Both live in `crates/app/src/command_dispatch.rs`.

Parameterized commands return `None` from `dispatch_command` because they have nothing to dispatch *until* stage 2 completes. That `None` is what triggers the palette to enter `OptionPick` instead of executing immediately. Don't try to unify the two functions - the `None` is load-bearing signal, not a missing case.

### Why the palette UI talks to the resolver via `Task::perform`

Resolver calls touch the DB through `with_conn_sync` (`Arc<Mutex<Connection>>`). The palette is a high-frequency UI path; even small mutex contention could cause keystroke jank. So `Confirm` returning a parameterized command schedules an async resolver call, and the result arrives via `OptionsLoaded`.

Each resolver call carries a generation counter so stale results are discarded if the user switches commands or types between calls. Matters more than it looks: `get_options` for "All Accounts" navigation can return hundreds of items across multiple accounts, and a slow-account straggler landing after the user has moved on would otherwise overwrite the visible list.

### Why keyboard dispatch lives in a global subscription

The palette subscribes via `iced::event::listen_with`, which receives events before widgets process them. Necessary for two reasons:

1. **Modifier chords like `Ctrl+K` must work even when a `text_input` has focus.** Letting iced's normal widget event flow handle them would mean Ctrl+K gets eaten by whatever input is focused.
2. **Two-key sequences (`g then i`) need a state machine that survives between key events.** That state (`pending_chord`) lives on `App`, and the timeout subscription is conditionally added when `pending_chord.is_some()`.

The flow inside `handle_key_pressed`: if the palette is open, route to the palette's own key handler (only Escape/arrows/Enter intercepted; everything else flows to the text input). Otherwise, if a widget already captured the event AND there's no command modifier, skip. Otherwise convert to a `cmdk::Chord`, then resolve as second-of-sequence, single chord, or pending-first.

### Why `CommandArgs` is in `cmdk` rather than `app`

The natural place would be `crates/app/` - it's used by the app's dispatch. It lives in `crates/cmdk/` instead because it's part of the parameterized command contract: the registry says "this command takes these parameters," the resolver provides options, the app builds typed `CommandArgs`, and dispatch consumes them. Putting `CommandArgs` in `cmdk` lets the type system enforce that the variants match the parameterized command IDs at the contract layer, not just inside the app.

Trade-off: `cmdk` ends up depending on `crates/types/` for `FolderId` and `LabelId`. Accepted - `types` is the lightweight shared-IDs crate (serde only) precisely so other crates can use typed IDs without pulling in heavy deps.

### Why `PaletteStage` is a flat enum with state on the parent

The original spec had `PaletteStage::CommandSearch { query, results, selected_index }` as data-carrying. The implementation has a bare unit enum (today `CommandSearch`, `OptionPick`, and `TextInput`) with the fields stored on `Palette` directly. Deliberate shape choice:

- Most state (query text, results vec, option items, selected index, generation counter) is shared between stages and would have to be duplicated or moved through `Option`s in a data-carrying variant.
- The flat fields let stage-2-specific data (`option_items`, `option_matches`) coexist with stage-1 data (`results`) without `match` arms in every accessor.
- Equivalence is preserved: `is_option_pick()` plus the fields acts as the same state machine.

Don't refactor this back to data-carrying without good reason - it'll grow boilerplate.

### Escape behavior in stage 2

The original spec said Escape always closes. The implementation makes Escape in `OptionPick` go back to `CommandSearch` instead, only closing from `CommandSearch`. Intentional UX: backing out of a wrong command into the search list is the common case; closing entirely is a less common intent that another Escape (or click-outside) handles.

### Cross-account undo wart

`dispatch_plan_with_undo` (`crates/app/src/handlers/commands.rs`) splits cross-account plans (one journal row per account) into one plan per account; each split pushes its own undo-stack entry. An N-account bulk action therefore takes N `Ctrl+Z` presses to fully undo. Documented in code comments. When a real user complains, fold the splits into a single composite undo entry.

## Constraints

- Rust edition 2024, strict clippy.
- The `cmdk` crate has zero UI dependencies - no iced, no GTK.
- Commands must be cheaply cloneable; the palette must feel instant.
- Fuzzy search must handle ~100-200 entries (commands + dynamic options) in sub-millisecond.
- Every action is a registered command - no exceptions, including `nav.next` / `nav.prev` (they appear in the palette alongside everything else, even though they're low-frequency searches).
