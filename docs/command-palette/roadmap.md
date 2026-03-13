# Command Palette: Implementation Roadmap

Phased implementation plan for the command palette backend. Each slice builds on the previous one and is independently shippable.

## Slice 1: Registry, Context, Fuzzy Search ✅

**Status: Complete** (`5e06755`)

- `CommandId` enum (55 variants)
- `CommandContext` struct with `ProviderKind` and `ViewType` enums
- `CommandDescriptor` with availability predicates and toggle label support
- `CommandRegistry` with nucleo-matcher fuzzy search
- `CommandMatch` returned with `available: bool` (frontend decides hide vs grey-out)
- Tauri command wrapper (`command_palette_query`)
- 18 unit tests

**Files:** `src-tauri/core/src/command_palette/`, `src-tauri/src/command_palette/`

## Slice 2: Parameterized Commands — Input Schema & Resolver Trait ✅

**Status: Complete (infrastructure scaffolding)**

This slice adds the typed input model and resolver trait. The four parameterized commands gain schema metadata but the resolver stub returns empty results — real option resolution (querying DbState) is future work. The execution contract (`CommandArgs`, dispatch endpoint) is deferred to avoid defining a payload without a consumer.

### What was built

**Core crate (`src-tauri/core/src/command_palette/`):**
- `input.rs` — `EnumOption` (value/label separation), `ParamDef` (ListPicker, DateTime, Enum, Text), `InputSchema` (Single, Sequence), `InputMode` (Direct, Parameterized) — all `Copy`, zero-allocation
- `input.rs` — `OptionItem` (flat, allocating runtime data from DB), `OptionMatch` (scored), `search_options()` with nucleo-matcher (searches label + path + keywords)
- `resolver.rs` — `CommandInputResolver` trait with sequence-aware `prior_selections: &[String]` on both `get_options()` and `validate_option()`
- `descriptor.rs` — `input_schema: Option<InputSchema>` on `CommandDescriptor`, `input_mode: InputMode` on `CommandMatch`
- Four commands registered as parameterized: `EmailMoveToFolder`, `EmailAddLabel`, `EmailRemoveLabel`, `EmailSnooze`
- 9 new tests (7 for search_options, 2 for input_mode)

**App crate (`src-tauri/src/command_palette/`):**
- `resolver.rs` — `InputResolverState` newtype wrapper (for Tauri managed state), `TauriInputResolver` stub
- `commands.rs` — `command_palette_get_options` and `command_palette_validate_option` Tauri commands

### Design decisions made during implementation

- **`CommandArgs` deferred** — introducing the execution payload without a dispatch endpoint would leave the hardest contract implicit and risk duplicating argument assembly in TypeScript
- **`prior_selections: &[String]`** on both trait methods — enables sequence-aware resolution and cross-field validation from day one, no breaking change when multi-step commands arrive
- **`EnumOption { value, label }`** instead of raw `&[&str]` — separates machine identifier from display text, no localization hazard
- **`search_options()` includes `path` in search text** — ancestor names are searchable ("q2" finds "Projects / Q2 / Reviews") without the resolver duplicating ancestors into keywords
- **`get_options()` only for ListPicker steps** — DateTime/Text/Enum are frontend-only input steps; the schema carries all info the frontend needs
- **`InputResolverState` newtype** — Tauri's `State<'_>` lookup is concrete-type based; `Arc<dyn Trait>` directly is brittle

### What remains (future slices)

- `CommandArgs` enum and the execution endpoint that consumes it
- Real `TauriInputResolver` querying `DbState` for folders, labels, accounts, templates
- Frontend changes to honor `input_mode`

### Ownership boundary

```
CommandRegistry (core, static, immutable):
  - CommandId, descriptors, availability, input schema declarations, search

CommandInputResolver (core trait, app-implemented, live state):
  - Resolving dynamic options for ListPicker parameter steps
  - Validating selected values (any step type) against current state
  - Sequence-aware: prior_selections flows through both methods
  - Future: previews, step transitions

App layer (framework-specific):
  - Constructing CommandContext
  - Holding the concrete resolver (InputResolverState newtype)
  - Orchestrating: registry → schema → resolver → user input → typed args → execute
```

### Commands with input schemas

| Command | Schema | ParamDef |
|---------|--------|----------|
| `EmailMoveToFolder` | `Single` | `ListPicker { label: "Folder" }` |
| `EmailAddLabel` | `Single` | `ListPicker { label: "Label" }` |
| `EmailRemoveLabel` | `Single` | `ListPicker { label: "Label" }` |
| `EmailSnooze` | `Single` | `DateTime { label: "Snooze until" }` |

Additional parameterized commands (templates, filters, etc.) will be added incrementally.

**Files:** `src-tauri/core/src/command_palette/input.rs`, `src-tauri/core/src/command_palette/resolver.rs`, `src-tauri/src/command_palette/resolver.rs`

**Depends on:** Slice 1

## Slice 3: Keybinding Model ✅

**Status: Complete (backend-only groundwork)**

This slice is backend-only — no user-visible behavior changes until slice 6. The frontend's `shortcutStore.ts` and `useKeyboardShortcuts.ts` continue driving real keyboard behavior. The two systems remain independent until slice 6 unifies them.

### What was built

**Core crate (`src-tauri/core/src/command_palette/keybinding.rs`):**
- Structured keybinding model: `Key` (Char/Named), `NamedKey` (26 variants matching DOM `KeyboardEvent.key`), `Modifiers` (with `CmdOrCtrl` abstraction), `Chord` (key + modifiers), `KeyBinding` (single chord or two-chord sequence), `Platform` (Mac/Windows/Linux)
- Const constructors: `KeyBinding::key('j')`, `::named(Escape)`, `::cmd_or_ctrl('a')`, `::cmd_or_ctrl_shift('e')`, `::seq('g', 'i')`
- Parse/display with canonical string format (`"CmdOrCtrl+Shift+E"`, `"g then i"`) and platform-resolved display (`"Ctrl"` on Linux, `"Cmd"` on Mac)
- Custom serde `Serialize`/`Deserialize` using canonical string format
- `BindingTable` — defaults + overrides (`Option<KeyBinding>` for explicit unbind) + O(1) reverse index + sequence-aware resolution (`resolve_chord`/`resolve_sequence` with `Pending` state) + conflict detection (chord vs chord, chord vs sequence first, sequence vs single) + primitive mutations (`set_override`, `unbind`, `remove_override`, `reset_all`)
- 27 tests (parse/display, serde, resolution, overrides, conflicts, display binding)

**Changes to existing types:**
- `CommandDescriptor::keybinding`: `Option<&'static str>` → `Option<KeyBinding>`
- `CommandMatch::keybinding`: `Option<&'static str>` → `Option<String>` (platform-resolved display)
- `CommandRegistry::command_for_key()` removed → replaced by `BindingTable::resolve_chord()`/`resolve_sequence()`
- `CommandRegistry::default_bindings()` added for `BindingTable` construction
- All 55 command registrations updated to use `KeyBinding` constructors
- `query()` produces platform-resolved display strings via `cfg!`-detected platform
- `CommandId::as_str()` documented as canonical stable external identifier
- TaskViewAll duplicate binding ("g then k", same as NavGoTasks) removed

### Design decisions

- **Backend-only**: No Tauri commands for binding management, no override persistence, no `custom_shortcuts` interaction. The frontend still owns that setting. Prevents two writers with different schemas against the same persisted setting.
- **`CmdOrCtrl` abstraction**: A single modifier that resolves per-platform at display time. Storage/wire format is always `"CmdOrCtrl"`, never platform-specific.
- **Sequences modeled properly**: `KeyBinding::Sequence(Chord, Chord)` with two-level resolution (`Pending` → `resolve_sequence`). Not removed — the UI layer handles timeout/pending state.
- **`Option<KeyBinding>` overrides**: `None` = explicitly unbound (prevents default fallback when a conflict forced unbinding). Absent = use default.
- **Primitive mutations only**: Core provides `set_override` (rejects conflicts), `unbind`, `remove_override`. No `force_override` — the app layer decides conflict resolution policy.
- **Conflict rules**: single vs single, single vs sequence-first, sequence-first vs single, duplicate sequence. Different sequences sharing a first chord is allowed (they coexist via the pending state).

### What remains (slice 6)

- Tauri commands for binding management (set, reset, unbind)
- Override persistence to `custom_shortcuts` setting
- `query()` signature change to accept `&BindingTable` for effective bindings
- Frontend migration: replace `shortcutStore.ts` and `useKeyboardShortcuts.ts`

**Files:** `src-tauri/core/src/command_palette/keybinding.rs` (new), `descriptor.rs`, `registry.rs`, `id.rs`, `mod.rs` (modified)

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

## Slice 6: Frontend Migration (Tauri/React)

Replace the three TypeScript files with thin consumers of the Rust registry. This slice is specifically for the current Tauri/React frontend. Iced integration is a separate future effort that consumes the same core APIs.

**Three integration paths (not one):**

1. **Palette UI** — `CommandPalette.tsx` calls `command_palette_query` instead of maintaining its own command list. Renders `CommandMatch` results. Invokes commands by `CommandId`.

2. **Keyboard dispatch** — `useKeyboardShortcuts.ts` is replaced by a thin handler that calls the **binding resolution API** from slice 3 (`resolve_binding`), not `command_palette_query`. Keyboard dispatch is a direct key→command lookup, not a search operation. The resolved `CommandId` is executed through the same handler map as the palette.

3. **Context menus / toolbars** — Any other UI surface that triggers commands (right-click menus, toolbar buttons) queries the registry for command metadata (label, icon, availability, keybinding hint) and invokes by `CommandId`. This uses `command_palette_query` with an empty query or `registry.get()`, not the fuzzy search path.

**What gets replaced:**
- `src/constants/shortcuts.ts` → keybindings resolved from registry via binding resolution API
- `src/hooks/useKeyboardShortcuts.ts` → thin key handler using binding resolution, single `CommandId → handler` map
- `src/components/search/CommandPalette.tsx` → UI queries registry, renders results

**What remains in TypeScript:** The execution handlers. A single `executeCommand(id: CommandId)` function maps command IDs to side effects (store mutations, navigation, Tauri invocations). The registry tells the frontend *what* to run; the frontend knows *how* to run it.

**Also introduces: `CommandContext` assembly.** The app layer needs a function that snapshots current state (selected threads, current view, active account, thread flags, online status, etc.) into a `CommandContext` struct for each registry query. This is a thin adapter over existing Zustand stores and route state.

**Migration strategy:** Incremental. Start with the palette UI (lowest risk, most visible payoff). Then keyboard dispatch. Then context menus. Remove `shortcuts.ts` last.

**Depends on:** Slices 1 + 3 at minimum. Slice 2 needed for parameterized commands to work in the palette. Slice 4 (ranking) is nice-to-have. Slice 5 (undo) is independent.

## Future: Iced Integration

Not a numbered slice — this happens as part of the broader Tauri→iced migration. The command palette core APIs are framework-agnostic by design. The iced frontend will:

- Implement its own `OptionProvider` (slice 2's trait) backed by its own state
- Build `CommandContext` from its own model
- Map `CommandId` to iced `Message` variants in its `update()` function
- Render palette UI, keyboard dispatch, and menus using the same registry

No core changes needed. The work is entirely in the iced app layer.

## Dependency Graph

```
Slice 1 (registry) ✅
  ├── Slice 2 (parameterized commands) ✅ (schema + trait; CommandArgs & real resolver deferred)
  │     └── needs real TauriInputResolver (queries DbState) and CommandArgs/dispatch endpoint
  ├── Slice 3 (keybindings) ✅ (backend-only; Tauri commands + persistence + frontend migration deferred to slice 6)
  │     └── needs Tauri commands for binding management, override persistence, query() integration
  ├── Slice 4 (ranking)
  │     └── recency accuracy requires slice 6 dispatch unification
  └── Slice 5 (undo)
        └── needs app-side execution contract
                │
                ▼
        Slice 6 (frontend migration)
          ├── palette UI (needs 1, 2)
          ├── keyboard dispatch (needs 1, 3)
          ├── context menus (needs 1)
          ├── keybinding management Tauri commands + persistence (needs 3)
          └── CommandContext assembly adapter
```

Slice 5 can be worked in parallel with anything. Slice 4's ranking infrastructure can be built in parallel, but recency tracking becomes useful only after slice 6. Slice 6 is incremental and can begin as soon as slices 1-3 are done. Slice 2's remaining work (real resolver, CommandArgs, execution dispatch) is needed before the palette UI in slice 6 can use parameterized commands. Slice 3's remaining work (Tauri commands, override persistence, query() integration) lands as part of slice 6's keyboard dispatch migration.
