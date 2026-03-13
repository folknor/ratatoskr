# Command Palette: Problem Statement

## Overview

Ratatoskr needs a centralized command system — a command palette — that serves as **the** action dispatch layer for the entire application. Every user-initiated action, whether triggered by keyboard shortcut, button click, menu item, or the palette UI itself, must be a registered command. There is no way to create an action without it being part of the palette.

This document describes the backend: the command registry, search, and dispatch infrastructure. It does not cover the UI.

## Current State

The app already has a command palette, but it's split across multiple systems that overlap and diverge:

### Two Separate Registries

1. **`src/constants/shortcuts.ts`** — 37 commands defined as `ShortcutItem[]` with `id`, `keys`, and `desc`. Three categories: Navigation (17), Actions (15), App (6).

2. **`src/components/search/CommandPalette.tsx`** — ~20 commands defined inline as `Command[]` with `id`, `label`, `category`, and `action` closures. Five categories: Navigation, Actions, Tasks, AI, Settings.

These overlap but don't match. The shortcuts file has commands the palette doesn't (pin, mute, select all, move to folder, arrow navigation). The palette has commands the shortcuts don't (theme switching, task panel toggle, AI chat, templates).

### Two Separate Execution Paths

1. **`src/hooks/useKeyboardShortcuts.ts`** — A 270-line `executeAction()` switch statement that maps shortcut IDs to imperative logic (store calls, navigation, custom events).

2. **`CommandPalette.tsx`** — Each command carries an inline `action: () => void` closure that does the same work independently.

The same action (e.g., "go to inbox") is implemented twice in different ways. These will inevitably drift.

### Other Limitations

- **Search**: `string.includes()` substring matching — no fuzzy scoring, no word-boundary weighting.
- **No second stage**: "Move to Folder" fires a `CustomEvent('ratatoskr-move-to-folder')` to punt the problem to another component. There's no general mechanism for parameterized commands.
- **No account awareness**: Everything is hardcoded. Templates (fetched per-account) are the only dynamic entries.
- **No context filtering**: Commands that require a selected thread (archive, star, reply) are always shown, even when nothing is selected.

### What Needs to Change

The Rust backend must become the single source of truth. Both keyboard dispatch and the palette UI should query the same registry, get the same commands, and invoke the same execution path. The TypeScript side becomes a thin consumer — it asks core "what commands are available given this context?" and "execute this command ID."

## Core Requirements

### 1. Exhaustive Command Registry

Every action the user can perform must be a registered command with a unique identity. This includes:

- Email thread actions (archive, trash, mark read, star, snooze, mute, pin, etc.)
- Compose actions (new email, reply, reply all, forward)
- Navigation (go to inbox, go to sent, go to a specific label/folder)
- UI layout (toggle sidebar, change reading pane position, change density)
- Search and filtering
- Account management
- Sync operations
- Contact, task, and calendar operations
- Settings and preferences

The current codebase has ~500+ operations at the Tauri command layer, but the palette surface is the ~80-100 user-facing actions that a human would invoke.

### 2. Hierarchical Command Organization

Commands are organized in a tree, primarily two levels deep:

```
Category > Command
```

Examples:
- `Email > Archive`
- `Email > Move to Folder`
- `View > Reading Pane > Bottom` (rare three-level case)
- `Navigate > Inbox`
- `Navigate > [Gmail Label]` (dynamic)

The tree structure is used for display and for narrowing search context, not as a dispatch mechanism.

### 3. Fuzzy Search with Word-Boundary Weighting

The palette's primary interaction mode is typing fragments and getting scored matches. The algorithm must:

- Match characters non-contiguously in order (typing "ear" matches "**E**mail > **A**rchive" — the `r` in "A**r**chive" confirms the match)
- Heavily weight matches at word boundaries (capital letters, after spaces/separators)
- Support the first-letter-of-each-word pattern: "ea" → "**E**mail > **A**rchive", "ts" → "**T**oggle **S**idebar"
- Score consecutive character matches higher
- Prefer shorter overall match spans
- Be case-insensitive

The `nucleo-matcher` crate (used by Helix editor and Walker launcher) implements this algorithm and is a strong candidate.

#### Ranking Beyond Fuzzy Score

Fuzzy match score alone is not sufficient for good ranking. The registry must support additional ranking signals, combined with the fuzzy score into a final sort order:

- **Recency**: Recently executed commands rank higher. The registry tracks a usage-count or last-used timestamp per command. This is what makes the palette learn the user's habits — "Archive" floats to the top for someone who archives constantly.
- **Context boost**: Commands relevant to the current context rank higher than commands that happen to match but aren't applicable. An enabled command should always outrank a disabled one with a better fuzzy score. Within enabled commands, commands whose context predicate is a tight match (e.g., "Star" when a thread is selected) rank above loosely relevant ones (e.g., "Toggle Sidebar").
- **Static commands over second-stage entities**: When the palette shows both top-level commands and second-stage options (if it ever does mixed results), static commands should generally rank above dynamic entities to avoid folder names drowning out commands.
- **Exact and alias hits**: An exact match on a command name or a defined alias (e.g., "delete" matching "Trash") should rank at or near the top regardless of fuzzy score.

The ranking model is part of the registry's query API, not something the app layer implements ad hoc. The weights and combination strategy are deferred to implementation, but the signals themselves must be designed into the registry from the start.

### 4. Three Command Tiers

#### Tier 1: Universal Commands
Always available, require no context.

- Compose new email
- Open settings
- Toggle sidebar
- Switch theme
- Trigger sync
- Open search

#### Tier 2: Context-Sensitive Static Commands
The commands are statically known, but their availability depends on application state.

- **Archive / Trash / Star / Mark Read** — require a selected thread
- **Reply / Reply All / Forward** — require a selected thread with messages
- **Delete Draft** — requires a selected draft

These commands exist in the registry at all times but may be disabled/hidden based on context.

#### Tier 3: Context-Sensitive Dynamic Commands (Parameterized)
The command itself is static, but selecting it opens a **second stage** populated with options fetched at runtime.

- **Move to Folder** → shows the folder tree for the active account
- **Add Label** → shows available labels for the active account
- **Navigate to Label** → shows labels/folders across accounts
- **Switch Account** → shows configured accounts

The second stage uses the same fuzzy search over the dynamic options.

### 5. Account-Aware Context

Ratatoskr supports multiple accounts across four providers (Gmail API, JMAP, Microsoft Graph, IMAP). The command palette must be account-aware because the available options differ per account and per provider:

| Universal (all accounts) | Account-Specific (data-driven) |
|---|---|
| Inbox, Sent, Drafts, Trash, Spam | Gmail labels, Exchange folder hierarchies, JMAP mailboxes |
| Starred/Flagged (same concept, different provider names) | Gmail categories (Primary, Social, Promotions) |
| Snoozed (local feature) | Shared mailboxes, distribution lists |
| "Unread" as a filter concept | Custom mailbox structures per account |

When a thread is selected, the active account is implied. When no thread is selected, the palette must either use the currently viewed account as context or show options from all accounts (disambiguated).

#### CommandContext

For the registry to determine command availability without leaking logic back into the app layer, it needs a concrete context snapshot. The app layer is responsible for assembling this struct from its own state (Zustand stores, Tauri state, iced model — whatever the framework uses) and passing it to the registry on each query. Core defines the shape; the app fills it in.

The context must include at minimum:

- **Selection**: selected thread IDs (zero, one, or many), active/focused message ID within a thread
- **Route/View**: current view type (inbox, label, smart folder, settings, calendar, tasks, attachments, compose), current label/folder ID if applicable
- **Account**: active account ID, provider type for that account (Gmail, JMAP, Graph, IMAP)
- **Provider capabilities**: what the active account's provider supports (labels vs folders, categories, shared mailboxes, server-side search). This avoids showing "Add Label" for an IMAP account that only has folders.
- **Entity state**: whether the selected thread is read/unread, starred, muted, pinned, in trash, is a draft — so toggle commands show the right label ("Star" vs "Unstar") and destructive commands resolve correctly ("Delete" in trash = permanent delete)
- **App state**: online/offline, whether the composer is open, whether multi-select is active, selection count
- **Focused UI region**: which panel has focus (thread list, reading pane, sidebar, composer) — some commands only apply in certain panels

Each command declares its context requirements as a predicate over this struct. The registry evaluates predicates to determine availability and filters the command list accordingly. This keeps all enablement logic in core, co-located with the command definitions, rather than scattered across app-layer UI code.

### 6. Framework Agnosticism

The command registry, search, and metadata live in `ratatoskr-core` (the framework-agnostic crate). The actual command *execution* is framework-specific:

- The current Tauri app dispatches commands through `#[tauri::command]` handlers
- The future iced app will dispatch through its own `Message` / `update` cycle

Core owns: command identity, metadata, hierarchy, search, context requirements.
The app layer owns: binding command IDs to concrete handler implementations.

## Constraints

- **Rust edition 2024, strict clippy** — no `unwrap`, max 7 args, max 100 lines per function, no cognitive complexity
- **Core crate has no UI dependencies** — no Tauri, no iced, no GTK
- Commands must be cheaply cloneable and searchable (the palette is invoked frequently and must feel instant)
- The fuzzy search must handle ~100-200 entries (static commands + dynamic options for one account) with sub-millisecond response times

### 7. Keybinding Model

Every command has a default keybinding (or none). Keybindings are a property of the command registration, not a separate system. The registry is the single source of truth for what key triggers what command.

- **Default bindings** are declared per-command in the registry (e.g., `Email > Archive` defaults to `e`).
- **User rebinding**: Users can override defaults. Overrides are stored as a `CommandId → KeyBinding` map, persisted in settings. The registry resolves bindings by checking user overrides first, then falling back to defaults.
- **Per-platform defaults**: Some bindings differ across platforms (e.g., `Cmd` vs `Ctrl`). The registry accepts platform-specific defaults.
- **Sequences**: Two-key sequences like `g then i` (Go to Inbox) are supported. The key dispatch layer handles the timing/state machine for pending keys.
- **Conflicts**: If a user rebinding collides with an existing binding, the registry detects and reports it. Resolution is the app layer's concern (show a warning, force the user to pick).

The palette UI displays the resolved keybinding next to each command. This is derived from the registry, never hardcoded in the UI.

### 8. Error Handling

Command execution can fail (network errors, permission denied on shared mailboxes, account not synced). The dispatch layer surfaces errors to the caller as structured results, not panics. Each command execution returns a `Result` — the app layer decides how to present failures (toast, inline error, retry prompt). Core does not swallow errors or log-and-ignore.

### 9. Undo

Commands can be undoable or not. This is declared at registration time — not every command supports undo (send, permanent delete), and that's explicit.

However, undo is not simply "run another command." Most real undos need runtime state captured from the original execution: the previous folder (for move), the prior read/starred/pinned state (for toggles), the old label set (for label changes), the previous pane position (for UI changes). A static `undo: Option<CommandId>` is insufficient because it doesn't carry this payload.

The backend framing: command execution may return an **undo token** — an opaque, serializable compensation payload that captures everything needed to reverse the action. The app layer maintains a short stack of these tokens. "Undo" pops the stack and executes the compensation. The token model must be framework-agnostic (core defines the token types; the app layer executes them), and tokens that reference ephemeral state (e.g., a thread that was permanently deleted) must be invalidated.

The full design of undo tokens, stack depth, expiration, and multi-step undo is deferred to implementation planning.

## Decisions

1. **Enum for command IDs, runtime for descriptors**: Command identity is a Rust enum (`CommandId`) — the compiler enforces that every command is handled. The ~80-100 user-facing commands today are small enough for this. Dynamic data (folder lists, label lists, account lists) is not command identity — it's Tier 3 second-stage parameters, fetched at runtime.

   However, the enum will grow as compound variants, view-local operations, and future features are added. To contain churn, the design must separate two layers:

   - **`CommandId` enum** — stable, top-level intents only. `Archive`, `MoveToFolder`, `ToggleSidebar`, `ComposeNew`. This changes infrequently. Adding a new user-facing action means adding a variant here, which is intentional friction — it forces you to handle it everywhere.
   - **`CommandDescriptor` (runtime)** — the context-resolved, display-ready representation of a command. Carries the resolved display label (possibly localized), resolved keybinding (user overrides applied), current availability (enabled/disabled given context), category path, and parameter schema. Built from the enum + current app state. This is what the palette UI and keyboard dispatch consume.

   The enum is the identity; the descriptor is the materialized view. The registry maps one to the other given a context snapshot.

2. **Command sequences, not composition**: "Archive and Next" is a registered compound command (`email.archive_and_next`) with its own ID, not a user-defined pipeline. This keeps the enum exhaustive and behavior predictable. If user-definable macros are needed later, they can be added as a separate feature on top.

## Open Questions

1. **Parameterized command execution contract**: The current description of Tier 3 commands ("second stage populated with options") understates the problem. "Pick from a list" is only one input shape. Real commands have diverse input requirements:

   - **List picker**: Move to Folder (pick one folder), Add Label (pick one label), Switch Account (pick one account)
   - **Date/time picker**: Snooze Until (pick a datetime)
   - **Multi-parameter**: Compose with Template (pick template + pick account), Create Filter from Sender (pick criteria + pick action)
   - **Enum/toggle**: Switch Theme (pick from fixed set of light/dark/system)
   - **Free text**: Rename Folder (type a name), Search (type a query)

   The backend needs a typed model for this. At minimum, each parameterized command must declare:

   - **Input schema**: What parameters it needs and their types (list selection, date, text, multi-step)
   - **Option identity vs display**: A folder has an internal ID and a display path — the registry must distinguish these so fuzzy search operates on the display label while execution uses the ID
   - **Validation**: Can the user submit this combination of parameters? (e.g., can't move a thread to the folder it's already in)
   - **Execution payload**: The final structured input passed to the command handler — not just "which option was picked" but the full typed arguments

   This is the hardest part of the backend design and must be resolved during implementation planning. The solution must be framework-agnostic (Tauri and iced will render these input shapes differently) while keeping the type contracts in core.

2. **Disabled command visibility**: Should commands that are currently unavailable (e.g., "Archive" with no thread selected) appear in palette results as greyed-out entries, or be hidden entirely? Showing them is discoverable (users learn what's possible), hiding them is less noisy (shorter, more relevant results). This is a product decision that affects the registry's query API — "return all commands with availability flags" vs "return only enabled commands."

3. **Palette visibility vs bindability**: Must every registered command be searchable in the palette, or can some be keyboard-only? Examples: `nav.next` / `nav.prev` (j/k) are essential keybindings but arguably noise in the palette — no one opens the palette to move down one thread. If commands can opt out of palette visibility, the registry needs a `palette_visible: bool` (or a visibility enum) on the command metadata.

4. **Scope of "single source of truth"**: The document states that keyboard dispatch and the palette UI consume the registry. But what about context menus, toolbars, right-click menus, and touch/mobile surfaces? If those also consume command metadata (label, icon, enabled state, keybinding hint), the registry is the app's entire action layer, not just the palette's backend. This is probably the right answer, but it expands the contract — the registry must serve any UI surface that can trigger or display a command, not just two.
