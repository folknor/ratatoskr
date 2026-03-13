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

The hardest backend slice. Commands like "Move to Folder" need a typed model for their input — not just "pick from a list."

**Input shapes to support:**
- **List picker**: Move to Folder, Add Label, Remove Label, Switch Account
- **Date/time**: Snooze Until
- **Enum/toggle**: Switch Theme (fixed set)
- **Free text**: Rename Folder, Search
- **Multi-parameter**: Compose with Template (template + account), Create Filter from Sender

**What needs to be built:**
- Input schema type that each parameterized command declares (what parameters, what types)
- Option provider trait — given a command ID and context, return the available options (folder list, label list, etc.)
- Option identity vs display label separation (fuzzy search on display, execute with ID)
- Validation — can this combination of parameters be submitted?
- Execution payload — the typed arguments passed to the handler

**Key constraint:** Framework-agnostic. Core defines the input schema and option providers. Tauri and iced render the appropriate UI (dropdown, date picker, text field) based on the schema.

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

**Depends on:** Slice 1. Independent of slices 2-3.

## Slice 5: Undo

Wire undo support into the command dispatch layer.

**What needs to be built:**
- Undo token type: serializable compensation payload capturing state needed to reverse an action (previous folder, prior read/starred state, old label set, etc.)
- Commands declare whether they're undoable at registration time
- Execution returns `Option<UndoToken>` alongside the result
- Undo stack: short bounded queue of recent tokens, maintained by the app layer
- Token invalidation: tokens referencing deleted/changed entities become invalid
- "Undo" command in the registry that pops and executes the top token

**Scope:** Archive, trash, move, star, pin, mute, mark-read, add/remove label are undoable. Send, permanent delete, compose are not.

**Depends on:** Slice 1. Benefits from slice 2 (parameterized commands may produce undo tokens with parameter context).

## Slice 6: Frontend Migration

Replace the three TypeScript files with thin consumers of the Rust registry.

**What gets replaced:**
- `src/constants/shortcuts.ts` → keybindings come from `command_palette_query` results
- `src/hooks/useKeyboardShortcuts.ts` → keyboard dispatch calls into the registry (via `command_for_key` or equivalent), then executes the returned `CommandId` through a single TS switch/map
- `src/components/search/CommandPalette.tsx` → UI queries the registry, renders results, invokes commands by ID

**What remains in TypeScript:** The execution handlers themselves (store mutations, navigation calls, Tauri invoke calls). These map `CommandId` → side effect. The registry tells the frontend *what* to run; the frontend knows *how* to run it in the current framework.

**Migration strategy:** Incremental. Start by having the palette UI call `command_palette_query` instead of maintaining its own command list. Then unify keyboard dispatch to use the same command IDs. Finally remove the old `shortcuts.ts` constants.

**Depends on:** Slices 1-3 at minimum. Slice 4 (ranking) is nice-to-have. Slice 5 (undo) is independent.

## Dependency Graph

```
Slice 1 (registry)
  ├── Slice 2 (parameterized) ──┐
  ├── Slice 3 (keybindings) ────┤
  ├── Slice 4 (ranking) ────────┤
  └── Slice 5 (undo) ──────────┤
                                └── Slice 6 (frontend migration)
```

Slices 2, 3, 4, and 5 can be worked in parallel after slice 1. Slice 6 requires at least 1-3 to be useful.
