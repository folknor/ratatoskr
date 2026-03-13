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

When a thread is selected, the active account is implied. When no thread is selected, the palette must either:
- Use the currently viewed account as context, or
- Show options from all accounts (disambiguated)

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

Each command can declare an undo counterpart at registration time: archive → unarchive, move → move back to previous folder, add label → remove label. The registry tracks this as metadata (`undo: Option<CommandId>` or a reversal closure). Not every command is undoable (compose, send, permanent delete), and that's explicit — `undo: None`.

This is wired at the registry level from the start, even if not every undo is implemented immediately. The dispatch layer maintains a short undo stack so the app can offer "Undo" after mutations.

## Decisions

1. **Enum for command IDs, runtime for metadata**: Command identity is a Rust enum — the compiler enforces that every command is handled. The ~80-100 user-facing commands are small enough for this to be practical. Dynamic data (folder lists, label lists, account lists) is not command identity — it's Tier 3 second-stage parameters, fetched at runtime.

2. **Command sequences, not composition**: "Archive and Next" is a registered compound command (`email.archive_and_next`) with its own ID, not a user-defined pipeline. This keeps the enum exhaustive and behavior predictable. If user-definable macros are needed later, they can be added as a separate feature on top.

## Open Questions

1. **How to express context requirements**: Should each command declare what context it needs (e.g., "I need a selected thread", "I need an active account"), and the registry filters based on current state? Or should the app layer handle enablement logic?

2. **Second-stage data fetching**: When a parameterized command is selected, who provides the options? A callback/trait method on the command? A separate "parameter provider" registry? This must work across both Tauri and iced.
