# Command Palette: Implementation Roadmap

Phased implementation plan for the command palette backend. Each slice builds on the previous one and is independently shippable.

## Slice 1: Registry, Context, Fuzzy Search ✅

**Status: Complete** (`5e06755`)

- `CommandId` enum (55 variants)
- `CommandContext` struct with `ProviderKind`, `ViewType`, and `FocusedRegion` enums
- `CommandDescriptor` with availability predicates and toggle label support
- `CommandRegistry` with nucleo-matcher fuzzy search
- `CommandMatch` returned with `available: bool` (UI decides hide vs grey-out)
- 18 unit tests

**Files:** `crates/command-palette/src/` (`id.rs`, `context.rs`, `descriptor.rs`, `registry.rs`)

## Slice 2: Parameterized Commands — Input Schema & Resolver Trait ✅

**Status: Complete (infrastructure scaffolding)**

This slice adds the typed input model and resolver trait. The four parameterized commands gain schema metadata but the resolver stub returns empty results — real option resolution (querying DbState) is future work. The execution contract (`CommandArgs`, dispatch endpoint) is deferred to avoid defining a payload without a consumer.

### What was built

**Command palette crate (`crates/command-palette/src/`):**
- `input.rs` — `EnumOption` (value/label separation), `ParamDef` (ListPicker, DateTime, Enum, Text), `InputSchema` (Single, Sequence), `InputMode` (Direct, Parameterized) — all `Copy`, zero-allocation
- `input.rs` — `OptionItem` (flat, allocating runtime data from DB), `OptionMatch` (scored), `search_options()` with nucleo-matcher (searches label + path + keywords)
- `resolver.rs` — `CommandInputResolver` trait with sequence-aware `prior_selections: &[String]` on both `get_options()` and `validate_option()`
- `descriptor.rs` — `input_schema: Option<InputSchema>` on `CommandDescriptor`, `input_mode: InputMode` on `CommandMatch`
- Four commands registered as parameterized: `EmailMoveToFolder`, `EmailAddLabel`, `EmailRemoveLabel`, `EmailSnooze`
- 9 new tests (7 for search_options, 2 for input_mode)

### Design decisions made during implementation

- **`CommandArgs` deferred** — introducing the execution payload without a dispatch endpoint would leave the hardest contract implicit
- **`prior_selections: &[String]`** on both trait methods — enables sequence-aware resolution and cross-field validation from day one, no breaking change when multi-step commands arrive
- **`EnumOption { value, label }`** instead of raw `&[&str]` — separates machine identifier from display text, no localization hazard
- **`search_options()` includes `path` in search text** — ancestor names are searchable ("q2" finds "Projects / Q2 / Reviews") without the resolver duplicating ancestors into keywords
- **`get_options()` only for ListPicker steps** — DateTime/Text/Enum are UI-only input steps; the schema carries all info the UI needs

### What remains (future slices)

- **`CommandArgs` enum and the execution endpoint that consumes it** — this is now the most critical missing piece. The typed execution contract is what makes the command system real rather than just searchable metadata. The problem statement (§ Parameterized command execution contract) defines the full payload shape. Without this, the palette can find commands but cannot execute parameterized ones.
- **Real `CommandInputResolver` implementation** in the app crate, querying `DbState` for folders, labels, accounts, templates
- UI changes to honor `input_mode`

### Ownership boundary

```
CommandRegistry (command-palette crate, static, immutable):
  - CommandId, descriptors, availability, input schema declarations, search

CommandInputResolver (command-palette crate trait, app-implemented, live state):
  - Resolving dynamic options for ListPicker parameter steps
  - Validating selected values (any step type) against current state
  - Sequence-aware: prior_selections flows through both methods
  - Future: previews, step transitions

App layer (crates/app/, iced Elm architecture):
  - Constructing CommandContext from app model state
  - Holding the concrete CommandInputResolver implementation
  - Orchestrating: registry -> schema -> resolver -> user input -> typed args -> execute
  - Mapping CommandId to Message variants in update()
```

### Commands with input schemas

| Command | Schema | ParamDef |
|---------|--------|----------|
| `EmailMoveToFolder` | `Single` | `ListPicker { label: "Folder" }` |
| `EmailAddLabel` | `Single` | `ListPicker { label: "Label" }` |
| `EmailRemoveLabel` | `Single` | `ListPicker { label: "Label" }` |
| `EmailSnooze` | `Single` | `DateTime { label: "Snooze until" }` |
| `NavigateToLabel` | `Single` | `ListPicker { label: "Label" }` |

`NavigateToLabel` is a cross-account parameterized command: the resolver populates options from all accounts when in unified scope, with account name in `OptionItem.path` for disambiguation. This command is a prerequisite for the sidebar's unified view (see `docs/sidebar/problem-statement.md` and the "Cross-Account Label/Folder Disambiguation" section in the command palette problem statement). Additional parameterized commands (templates, filters, etc.) will be added incrementally.

**Files:** `crates/command-palette/src/input.rs`, `crates/command-palette/src/resolver.rs`

**Depends on:** Slice 1

## Slice 3: Keybinding Model ✅

**Status: Complete (backend-only groundwork)**

This slice is backend-only — no user-visible behavior changes until slice 6 wires keybinding resolution into the iced event loop.

### What was built

**Command palette crate (`crates/command-palette/src/keybinding.rs`):**
- Structured keybinding model: `Key` (Char/Named), `NamedKey` (26 variants matching DOM `KeyboardEvent.key`), `Modifiers` (with `CmdOrCtrl` abstraction), `Chord` (key + modifiers), `KeyBinding` (single chord or two-chord sequence), `Platform` (Mac/Windows/Linux)
- Const constructors: `KeyBinding::key('j')`, `::named(Escape)`, `::cmd_or_ctrl('a')`, `::cmd_or_ctrl_shift('e')`, `::seq('g', 'i')`
- Parse/display with canonical string format (`"CmdOrCtrl+Shift+E"`, `"g then i"`) and platform-resolved display (`"Ctrl"` on Linux, `"Cmd"` on Mac)
- Custom serde `Serialize`/`Deserialize` using canonical string format
- `BindingTable` — defaults + overrides (`Option<KeyBinding>` for explicit unbind) + O(1) reverse index + sequence-aware resolution (`resolve_chord`/`resolve_sequence` with `Pending` state) + conflict detection (chord vs chord, chord vs sequence first, sequence vs single) + primitive mutations (`set_override`, `unbind`, `remove_override`, `reset_all`)
- 27 tests (parse/display, serde, resolution, overrides, conflicts, display binding)

**Changes to existing types:**
- `CommandDescriptor::keybinding`: `Option<&'static str>` -> `Option<KeyBinding>`
- `CommandMatch::keybinding`: `Option<&'static str>` -> `Option<String>` (platform-resolved display)
- `CommandRegistry::command_for_key()` removed -> replaced by `BindingTable::resolve_chord()`/`resolve_sequence()`
- `CommandRegistry::default_bindings()` added for `BindingTable` construction
- All 55 command registrations updated to use `KeyBinding` constructors
- `query()` produces platform-resolved display strings via `cfg!`-detected platform
- `CommandId::as_str()` documented as canonical stable external identifier
- TaskViewAll duplicate binding ("g then k", same as NavGoTasks) removed

### Design decisions

- **Backend-only**: No binding management API exposed yet, no override persistence. The keybinding model is ready but not yet wired into the iced event loop.
- **`CmdOrCtrl` abstraction**: A single modifier that resolves per-platform at display time. Storage/wire format is always `"CmdOrCtrl"`, never platform-specific.
- **Sequences modeled properly**: `KeyBinding::Sequence(Chord, Chord)` with two-level resolution (`Pending` -> `resolve_sequence`). Not removed — the UI layer handles timeout/pending state.
- **`Option<KeyBinding>` overrides**: `None` = explicitly unbound (prevents default fallback when a conflict forced unbinding). Absent = use default.
- **Primitive mutations only**: Core provides `set_override` (rejects conflicts), `unbind`, `remove_override`. No `force_override` — the app layer decides conflict resolution policy.
- **Conflict rules**: single vs single, single vs sequence-first, sequence-first vs single, duplicate sequence. Different sequences sharing a first chord is allowed (they coexist via the pending state).

### What remains (slice 6)

- Wire `BindingTable` into the iced event subscription for keyboard dispatch
- Override persistence to settings
- `query()` signature change to accept `&BindingTable` for effective bindings

**Files:** `crates/command-palette/src/keybinding.rs` (new), `descriptor.rs`, `registry.rs`, `id.rs`, `lib.rs` (modified)

**Depends on:** Slice 1

## Slice 4: Ranking Signals

Move beyond pure fuzzy score to context-aware ranking.

**What needs to be built:**
- Recency tracking: store last-used timestamp or use count per command, persisted across sessions
- Context boost: commands relevant to current context rank higher (email commands when viewing email, task commands when viewing tasks)
- Enabled-over-disabled: available commands always outrank unavailable ones with equal fuzzy scores
- Exact/alias hits: "delete" matches "Move to Trash" via alias, ranks at top
- Empty-query ordering: replace alphabetical with recency-weighted default order

**Practical dependency on dispatch integration:** Recency tracking requires that command execution is recorded. Until keyboard, palette, and menu dispatch all route through the command system (slice 6), recency data will be partial — only commands invoked through whichever surface is integrated first will be tracked. The ranking infrastructure can be built in parallel with slice 1, but **recency becomes accurate only after slice 6 completes dispatch unification.**

The non-recency ranking signals (context boost, enabled-over-disabled, alias hits) are useful immediately and don't depend on dispatch integration.

**Depends on:** Slice 1. Recency accuracy depends on slice 6.

## Slice 5: Undo

Wire undo support into the command dispatch layer.

**What needs to be built:**
- Undo token type: serializable compensation payload capturing state needed to reverse an action (previous folder, prior read/starred state, old label set, etc.)
- Commands declare whether they're undoable at registration time
- Execution returns `Option<UndoToken>` alongside the result
- Undo stack: short bounded queue of recent tokens
- Token invalidation: tokens referencing deleted/changed entities become invalid

**Ownership boundary:** The undo stack and token execution are both app-layer concerns, not registry concerns. Core defines the `UndoToken` types and declares which commands are undoable. The app layer:
1. Receives tokens from command execution
2. Maintains the stack
3. Executes compensation when "Undo" is invoked

The registry contains an `Undo` command ID so it appears in the palette and can be bound to a key (Ctrl+Z). But the registry does not own the stack or execute compensations — it dispatches to the app layer like any other command. The app's undo handler pops the stack and runs the reversal.

**Scope:** Archive, trash, move, star, pin, mute, mark-read, add/remove label are undoable. Send, permanent delete, compose are not.

**Depends on:** Slice 1. Benefits from slice 2 (parameterized commands may produce undo tokens with parameter context). Requires an app-side execution contract to actually run compensations.

## Slice 6: Iced App Integration

Wire the command palette into the iced app's Elm architecture. The command palette core APIs (`crates/command-palette/`) are framework-agnostic; this slice is purely app-layer work in `crates/app/`.

**Three integration paths (not one):**

1. **Palette UI** — Build a palette overlay widget (text_input + scrollable results list). On keystroke, call `CommandRegistry::query()` and render `CommandMatch` results. On selection, map the `CommandId` to a `Message` variant and feed it into `update()`.

2. **Keyboard dispatch** — Subscribe to iced keyboard events. On keypress, call `BindingTable::resolve_chord()` / `resolve_sequence()` from slice 3 to get a `CommandId`. Map it to a `Message` variant and dispatch through `update()`. This replaces any ad-hoc key handling. Keyboard dispatch is a direct key-to-command lookup, not a search operation.

3. **Context menus / toolbars** — Any other UI surface that triggers commands (right-click menus, toolbar buttons) queries the registry for command metadata (label, icon, availability, keybinding hint) and invokes by `CommandId`. This uses `CommandRegistry::query()` with an empty query or a direct `get()`, not the fuzzy search path.

**What needs to be built in `crates/app/`:**

- **`CommandContext` assembly** — A function that snapshots current app model state (selected threads, current view, active account, thread flags, online status, focused region) into a `CommandContext` struct for each registry query. This is a thin adapter over the app's model fields.
- **`CommandInputResolver` implementation** — A concrete resolver that queries `DbState` for folders, labels, accounts, and templates to populate ListPicker options.
- **`CommandId -> Message` dispatch map** — A single function mapping command IDs to iced `Message` variants. The registry tells the app *what* to run; `update()` knows *how* to run it.
- **Palette overlay widget** — Text input with filtered results, keyboard navigation (arrow keys, Enter, Escape), availability-aware rendering. Built with iced primitives (text_input, scrollable, column, container).
- **Keyboard event subscription** — An iced subscription that captures key events, runs them through `BindingTable` resolution, and emits the corresponding `Message`.
- **Binding management** — Functions for set/reset/unbind exposed through a settings UI. Override persistence to app settings.
- **Pending chord indicator** — When a two-chord sequence's first chord matches, show a transient indicator (e.g., "g..." in the status bar) with a timeout before clearing.

**Migration strategy:** Incremental. Start with the palette UI (lowest risk, most visible payoff). Then keyboard dispatch. Then context menus.

**Depends on:** Slices 1 + 3 at minimum. Slice 2 needed for parameterized commands to work in the palette. Slice 4 (ranking) is nice-to-have. Slice 5 (undo) is independent.

## Dependency Graph

```
Slice 1 (registry) ✅
  ├── Slice 2 (parameterized commands) ✅ (schema + trait; CommandArgs & real resolver deferred)
  │     └── needs real CommandInputResolver impl in app crate (queries DbState) and CommandArgs/dispatch
  ├── Slice 3 (keybindings) ✅ (model complete; iced integration + persistence deferred to slice 6)
  │     └── needs iced event subscription, override persistence, query() integration
  ├── Slice 4 (ranking)
  │     └── recency accuracy requires slice 6 dispatch unification
  └── Slice 5 (undo)
        └── needs app-side execution contract
                │
                ▼
        Slice 6 (iced app integration)
          ├── palette UI widget (needs 1, 2)
          ├── keyboard dispatch via iced subscription (needs 1, 3)
          ├── context menus (needs 1)
          ├── binding management + persistence (needs 3)
          ├── CommandContext assembly from app model
          └── CommandId -> Message dispatch map
```

Slice 5 can be worked in parallel with anything. Slice 4's ranking infrastructure can be built in parallel, but recency tracking becomes useful only after slice 6. Slice 6 is incremental and can begin as soon as slices 1-3 are done. Slice 2's remaining work (real resolver, CommandArgs, execution dispatch) is needed before the palette UI in slice 6 can use parameterized commands. Slice 3's remaining work (iced event wiring, override persistence, query() integration) lands as part of slice 6's keyboard dispatch integration.

## Ecosystem Patterns

How patterns from the [iced ecosystem survey](../iced-ecosystem-survey.md) map to each roadmap slice. Sourced from the [cross-reference](../iced-ecosystem-cross-reference.md).

### Requirements to Survey Matches

| Roadmap Slice | Primary Source | How It Applies |
|---|---|---|
| Slice 4 (Ranking) | raffi MRU | `MruEntry` data model (count + timestamp, persisted to disk) for per-command recency tracking |
| Slice 5 (Undo) | cedilla patch history | Circular buffer concept (bounded queue, oldest evicted), but domain differs: action-compensation vs text-diff |
| Slice 6 (Palette UI) | shadcn-rs overlay + raffi query routing | Overlay placement math via `place_overlay_centered()`; prefix-based mode switching between command search and option picking |
| Slice 6 (Keyboard dispatch) | feu/cedilla keyboard subscriptions | `subscription::events_with` for global key capture before widget processing; HashMap-based shortcut lookup |
| Slice 6 (App architecture) | trebuchet Component trait | Palette as Component emitting `CommandSelected(CommandId)` and `Dismissed` events via `(Task, ComponentEvent)` tuples |
| Slice 6 (Event wiring) | rustcast `Subscription::batch()` | Combines keyboard, palette-internal, and timer subscriptions into a single batched subscription |
| Slice 6 (Resolver races) | bloom generational tracking | Guards against stale async option results from `CommandInputResolver::get_options()` when user switches commands rapidly |

### Gaps

- **Slices 1-3 (already complete)**: Backend-only, framework-agnostic -- no survey matches needed
- **Two-chord pending indicator with timeout**: No surveyed project handles pending chord state with timeouts or displays a transient chord indicator
- **User-customizable keybindings with conflict detection**: Beyond anything in the survey; the `BindingTable` architecture is original to Ratatoskr
- **The core registry architecture** (`CommandId` enum, `CommandContext` predicates, `CommandInputResolver` trait, typed `CommandArgs`): Original to Ratatoskr, no analogues in surveyed projects
