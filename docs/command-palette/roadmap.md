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

## Slice 2: Parameterized Commands

The hardest backend slice. Commands like "Move to Folder" need a typed model for their input — not just "pick from a list." Design is fully specified in `problem-statement.md` decision #3.

### Design Summary

**Stage separation:** `query()` stays a command search API. Parameterized commands appear in results with `input_mode: InputMode::Parameterized { schema }`. Parameter resolution uses a separate `OptionProvider` trait.

**Caller flow:**
1. `registry.query(ctx, query)` → user picks a `CommandMatch`
2. If `input_mode == Direct` → execute with `CommandId` alone
3. If `Parameterized` → call `OptionProvider::get_options()` for each step in the schema, user picks, build `CommandArgs`, execute

### What needs to be built in core

- `InputMode` enum (`Direct`, `Parameterized`) on `CommandMatch`
- `InputSchema` enum (`Single(ParamDef)`, `Sequence(&'static [ParamDef])`)
- `ParamDef` enum (`ListPicker`, `DateTime`, `Enum`, `Text`)
- `OptionItem` struct (flat: `id`, `label`, `path`, `keywords`, `disabled`)
- `CommandArgs` enum — one variant per parameterized command family, typed fields
- `OptionProvider` trait — `fn get_options(command_id, param_index, ctx) -> Result<Vec<OptionItem>, String>`
- Register `InputSchema` on parameterized `CommandDescriptor`s
- Fuzzy search over `OptionItem` lists (reuse nucleo-matcher)

### What needs to be built in the app crate

- `TauriOptionProvider` implementing `OptionProvider` — queries `DbState` for folders, labels, accounts, templates
- Registration of the provider in app state alongside the `CommandRegistry`
- Tauri command for parameter resolution: `command_palette_get_options(command_id, param_index, ctx) -> Vec<OptionItem>`

### Ownership boundary

Core owns the `OptionProvider` trait, `InputSchema`, `ParamDef`, `OptionItem`, and `CommandArgs` types. The app layer implements the trait. This is the same pattern as `ProgressReporter`: core defines the contract, the app provides the concrete implementation backed by `DbState`, account state, and provider clients.

The `OptionProvider` is a separate object from the `CommandRegistry`. The registry is immutable static data. The option provider needs live DB/state access.

### Commands that become parameterized

| Command | Schema | Args variant |
|---------|--------|-------------|
| `EmailMoveToFolder` | `Single(ListPicker)` | `MoveToFolder { folder_id }` |
| `EmailAddLabel` | `Single(ListPicker)` | `AddLabel { label_id }` |
| `EmailRemoveLabel` | `Single(ListPicker)` | `RemoveLabel { label_id }` |
| `EmailSnooze` | `Single(DateTime)` | `Snooze { until }` |

Additional parameterized commands (templates, filters, etc.) will be added incrementally.

**Depends on:** Slice 1

## Slice 3: Keybinding Model

Move keybinding ownership from the frontend into the registry.

**What needs to be built:**
- Default keybindings are already on `CommandDescriptor` (slice 1). This slice adds:
- User override storage (`CommandId → KeyBinding` map, persisted in settings DB)
- Resolved binding lookup: check user overrides first, fall back to defaults
- Per-platform defaults (`Cmd` vs `Ctrl`)
- Conflict detection: warn when a rebinding collides with an existing binding
- Sequence support formalization: the `"g then i"` pattern needs a state machine in the dispatch layer, currently implemented ad-hoc in `useKeyboardShortcuts.ts`
- **Binding resolution API** (`resolve_binding(&self, key: &str) -> Option<CommandId>`): a direct lookup path that works without running a palette query. This is what keyboard dispatch uses — it does not go through the search/filter pipeline.

**What this replaces:** `src/constants/shortcuts.ts` and the keybinding resolution logic in `useKeyboardShortcuts.ts`.

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
  ├── Slice 2 (parameterized commands)
  │     └── needs app-side OptionProvider adapter
  ├── Slice 3 (keybindings)
  │     └── binding resolution API for keyboard dispatch
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
          └── CommandContext assembly adapter
```

Slices 2, 3, and 5 can be worked in parallel after slice 1. Slice 4's ranking infrastructure can be built in parallel, but recency tracking becomes useful only after slice 6. Slice 6 is incremental and can begin as soon as slices 1 + 3 are done.
