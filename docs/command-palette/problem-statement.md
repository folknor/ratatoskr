# Command Palette: Problem Statement

The command palette is **the** action dispatch layer for the entire app. Every user-initiated action - keyboard shortcut, button click, menu item, palette selection - is a registered command. There is no way to create an action that isn't part of the registry.

This doc captures the load-bearing design decisions. For implementation status, see `roadmap.md`. For the current spec/code gaps, see `discrepancies.md`. For day-to-day code, the registry lives in `crates/cmdk/` and the app integration in `crates/app/src/{command_dispatch,command_resolver,ui/palette}.rs`.

## Three command tiers

The palette has to handle three distinct shapes of action:

1. **Universal** - always available, no context required (e.g. compose, open settings, toggle sidebar).
2. **Context-sensitive static** - the command is statically known but availability depends on app state (archive, reply, delete-draft).
3. **Parameterized (two-stage)** - the command is static but selecting it opens a second stage populated at runtime (move-to-folder, add-label, navigate-to-label, snooze).

Tier 3 drives the registry/resolver split: the static command list is a `CommandRegistry`; the runtime option list comes from a separate `CommandInputResolver` trait that the app implements with DB access. Core defines the contract; the app layer holds live state.

## Cross-account label/folder disambiguation

Ratatoskr is multi-account across four providers (Gmail, JMAP, Graph, IMAP). The unified ("All Accounts") sidebar deliberately omits per-account labels - they're provider-specific noise in a cross-account context. **The palette becomes the primary way to navigate to a label or folder when in unified view.** That removability of the unified sidebar's label list is contingent on the palette doing this well.

The mechanism is `OptionItem`: a flat list (no trees) with `id`, `label`, optional `path: Vec<String>`, optional `keywords`, and `disabled`. In a single-account scope `path` carries hierarchy. In All Accounts, the resolver prefixes `path` with the account display name. Fuzzy search runs over `label`, joined `path` segments, and `keywords`, so typing "work clients" finds "Clients" under "Work Exchange" without the user knowing the exact hierarchy. The UI reconstructs hierarchical display from `path`. Execution uses `id`. System labels (`INBOX`, `SENT`, etc.) are excluded - those have universal sidebar destinations and dedicated palette commands.

## Stage separation in the execution contract

`CommandRegistry::query()` is stage-1 only - it returns top-level commands, never second-stage options. Parameterized commands appear in query results with `input_mode: InputMode::Parameterized { schema }`; stage 2 is driven separately via the resolver. This keeps the search APIs cleanly typed and makes it easy for non-palette surfaces (context menus, toolbars) to execute commands without going through stage 2 themselves.

The execution payload is a typed `CommandArgs` enum (one variant per parameterized command), not `serde_json::Value`. Each variant carries exactly the typed fields that command needs (`FolderId`, `TagId`, `i64` timestamp, etc.). The compiler enforces exhaustive matching in the dispatch function.

## Identity vs descriptor

`CommandId` is a Rust enum (~70 variants today, growing). The compiler enforces that every command is handled. Adding a new user-facing action means adding a variant - intentional friction, since you have to wire it in `dispatch_command`, register a descriptor, and consider a default keybinding.

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

Every command has a default keybinding (or none) declared at registration. User overrides live in a `CommandId → KeyBinding` map, persisted to `keybindings.json`. The registry resolves bindings (overrides → defaults), supports two-key sequences (`g then i`), platform-specific defaults (`Cmd` vs `Ctrl`), and conflict detection.

The palette UI displays the resolved binding next to each command, derived from the `BindingTable` rather than hardcoded.

## Ranking beyond fuzzy score

Fuzzy match score alone isn't sufficient. The registry combines:

- **Recency** - `UsageTracker` records per-command usage counts, persisted to `command_usage.json`. Folded into both the empty-query ordering and the fuzzy score (log-scaled `recency_bonus` so high-frequency commands tip ties without dominating).
- **Context boost** - categories aligned with the current view get a score bump; focused-region alignment adds more.
- **Availability bonus** - enabled commands always outrank disabled ones with similar fuzzy scores (`+1000` to the score).
- **Aliases / keywords** - folded into the fuzzy haystack, so alias hits surface naturally.

Disabled commands are returned by `query()` with an `available: bool` flag - the caller (palette, context menu, toolbar) decides whether to hide or grey them out. Every registered command is palette-visible; there is no keyboard-only tier.

## Undo

Some commands are undoable, declared via `is_undoable` on the descriptor. Undo isn't "run another command" - most real undos need runtime state captured from the original execution (previous folder, prior toggle state, old label set). The mechanism is an **undo token**: an opaque, serializable compensation payload pushed to a bounded stack at execution time. `Ctrl+Z` pops the stack and dispatches the inverse plan. Tokens that reference ephemeral state (e.g. a permanently-deleted thread) must be invalidated.

## Constraints

- Rust edition 2024, strict clippy.
- The `cmdk` crate has zero UI dependencies - no iced, no GTK.
- Commands must be cheaply cloneable; the palette must feel instant.
- Fuzzy search must handle ~100-200 entries (commands + dynamic options) in sub-millisecond.
- Every action is a registered command - no exceptions, including `nav.next` / `nav.prev` (they appear in the palette alongside everything else, even though they're low-frequency searches).
