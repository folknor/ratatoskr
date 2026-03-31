# Command Palette: Problem Statement

## Overview

Ratatoskr needs a centralized command system — a command palette — that serves as **the** action dispatch layer for the entire application. Every user-initiated action, whether triggered by keyboard shortcut, button click, menu item, or the palette UI itself, must be a registered command. There is no way to create an action without it being part of the palette.

This document describes the backend: the command registry, search, and dispatch infrastructure. It does not cover the UI.

## Current State

The command palette backend is implemented in the `cmdk` crate (`crates/cmdk/`). The old TypeScript frontend (which had duplicate registries, duplicate execution paths, substring-only search, no parameterized commands, and no context filtering) has been removed entirely. The Rust crate is the single source of truth.

### What Exists (Slices 1-4 Complete)

1. **`CommandId` enum** (`crates/cmdk/src/id.rs`) — 55 commands across 6 categories (Navigation, Email, Compose, Tasks, View, App). Each variant has a stable `as_str()` / `parse()` round-trip for persistence.

2. **`CommandRegistry`** (`crates/cmdk/src/registry.rs`) — All 55 commands registered with labels, categories, default keybindings, context predicates (`is_available`), toggle labels (`is_active`), input schemas for parameterized commands, and keyword aliases. Fuzzy search via `nucleo-matcher` with context boost and availability bonus scoring.

3. **`CommandContext`** (`crates/cmdk/src/context.rs`) — Context snapshot struct with selection state, view type, account/provider info, entity state (read/starred/muted/pinned/draft/trash/spam), app state (online, composer open), and focused UI region.

4. **`BindingTable`** (`crates/cmdk/src/keybinding.rs`) — Keybinding resolution with single chords, two-key sequences (`g then i`), user override support, conflict detection, platform-aware display (`Cmd` vs `Ctrl`).

5. **`InputSchema` / `ParamDef`** (`crates/cmdk/src/input.rs`) — Parameterized command schemas (ListPicker, DateTime, Enum, Text), option items with hierarchical path display, and fuzzy search over options.

6. **`CommandInputResolver` trait** (`crates/cmdk/src/resolver.rs`) — Core-defined trait for resolving dynamic options and validating selections. The iced app layer provides the concrete implementation.

7. **`UsageTracker`** — Per-command usage counts for recency-based ranking. Persistence deferred to Slice 6.

### What Still Needs to Happen

The Rust backend is the single source of truth. The iced app layer (`crates/app/`) is the consumer — it assembles a `CommandContext` from its model state, queries the registry, and dispatches commands through its `Message` / `update()` cycle. Both keyboard dispatch (via iced's subscription/event system and `BindingTable`) and the palette UI query the same registry, get the same commands, and invoke the same execution path. The app passes either a `CommandId` alone (for direct commands) or a `CommandId` + `CommandArgs` (for parameterized commands that require stage-2 input resolution). See the Parameterized command execution contract for the full flow.

## Core Requirements

### 1. Exhaustive Command Registry

Every action the user can perform must be a registered command with a unique identity. This includes:

- Email thread actions (archive, trash, mark read, star, snooze, mute, pin, etc.)
- Compose actions (new email, reply, reply all, forward)
- Navigation (go to inbox, go to sent, go to a specific label/folder, navigate between threads)
- UI layout (toggle sidebar, toggle right sidebar)
- Search and filtering
- Account management
- Sync operations
- Contact, task, and calendar operations
- Settings and preferences

The current registry has 55 commands. As the app grows (compound variants, view-local operations, future features), the palette surface will expand to ~80-100 user-facing actions.

### 2. Hierarchical Command Organization

Commands are organized in a tree, primarily two levels deep:

```
Category > Command
```

Examples:
- `Email > Archive`
- `Email > Move to Folder`
- `View > Toggle Right Sidebar`
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

When a thread is selected, the active account is implied. When no thread is selected (or the sidebar is scoped to "All Accounts"), parameterized commands show options from all accounts, disambiguated via `OptionItem.path` (see § Cross-Account Label/Folder Disambiguation). This is consistent with the search and sidebar docs, which default to cross-account/additive behavior.

#### Cross-Account Label/Folder Disambiguation

The sidebar's unified ("All Accounts") view deliberately omits per-account labels — they're provider-specific noise in a cross-account context (see `docs/sidebar/problem-statement.md`). This means the command palette becomes the **primary way to navigate to a label or folder** when in unified view. The `Navigate to Label` command (Tier 3, parameterized) must handle this well enough that removing labels from the unified sidebar is not a discoverability regression.

The problem: a user with three accounts may have labels/folders with the same name across accounts ("Clients" on Gmail, "Clients" on Exchange). The second-stage option list must disambiguate without being noisy for the common case (unique names).

**Resolution via `OptionItem`**: The parameterized command infrastructure already provides the building blocks. When the `CommandInputResolver` resolves options for `Navigate to Label`:

- **Scoped to a single account**: Options come from that account only. No disambiguation needed — `OptionItem.label` is the label/folder name, `OptionItem.path` reflects hierarchy (Exchange folder tree, JMAP mailbox nesting), and the account is implicit from scope.

- **All Accounts scope (or no thread selected)**: Options come from all accounts. The resolver sets `OptionItem.path` to include the account name as the first segment:

  ```
  OptionItem { id: "gmail-abc:Label_42", label: "Clients", path: Some(["Foo Corp Gmail"]), ... }
  OptionItem { id: "graph-xyz:folder-99", label: "Clients", path: Some(["Work Exchange"]), ... }
  OptionItem { id: "graph-xyz:folder-77", label: "Reviews", path: Some(["Work Exchange", "Projects", "Q2"]), ... }
  ```

  The palette UI renders these as:
  ```
  Clients              Foo Corp Gmail
  Clients              Work Exchange
  Reviews              Work Exchange > Projects > Q2
  ```

  Fuzzy search covers both `label` and `path` segments, so typing "work clients" finds "Clients" under "Work Exchange" without the user needing to know the exact hierarchy.

- **Unique names across accounts**: When a label name is unique (only exists on one account), the account prefix is still shown in the "All Accounts" context for consistency — but it's visually secondary (right-aligned or dimmed, a UI concern). The user doesn't need to type the account name; the label name alone matches.

**Provider differences in the option list**: The resolver handles provider-specific structures transparently:

- **Gmail**: Flat label list. Labels that look nested in Gmail's UI (e.g., "Projects/Q2") are actually flat labels with `/` in the name. The resolver can split these into `path` segments for hierarchical display, or leave them flat — this is a display choice, not a data model issue.
- **Exchange/Graph**: Folder tree. The resolver walks the folder hierarchy and populates `path` with ancestor folders.
- **JMAP**: Mailbox hierarchy, similar to Exchange.
- **IMAP**: LSUB hierarchy, similar to Exchange.

**System labels/folders are excluded**: The option list only shows user-visible labels and folders. System labels (Gmail's `INBOX`, `SENT`, `TRASH`, etc.) and well-known Exchange folders are not included — those are universal folders with their own sidebar destinations and palette commands (`Navigate > Inbox`, etc.).

This design means the sidebar can safely omit labels from the unified view: the palette provides equivalent access with better search affordances. The prerequisite for removing labels from the unified sidebar is that this resolver is implemented and the `Navigate to Label` command works with cross-account options.

#### CommandContext

For the registry to determine command availability without leaking logic back into the app layer, it needs a concrete context snapshot. The iced app layer is responsible for assembling this struct from its model state and passing it to the registry on each query. Core defines the shape; the app fills it in.

The context must include at minimum:

- **Selection**: selected thread IDs (zero, one, or many), active/focused message ID within a thread
- **Route/View**: current view type (inbox, label, smart folder, settings, calendar, tasks, attachments, compose), current label/folder ID if applicable
- **Account**: active account ID, provider type for that account (Gmail, JMAP, Graph, IMAP)
- **Provider capabilities**: what the active account's provider supports (labels vs folders, categories, shared mailboxes, server-side search). This avoids showing "Add Label" for an IMAP account that only has folders.
- **Entity state**: whether the selected thread is read/unread, starred, muted, pinned, in trash, is a draft — so toggle commands show the right label ("Star" vs "Unstar") and destructive commands resolve correctly ("Delete" in trash = permanent delete)
- **App state**: online/offline, whether the composer is open, whether multi-select is active, selection count
- **Focused UI region**: which panel has focus (thread list, reading pane, sidebar, composer) — some commands only apply in certain panels

Each command declares its context requirements as a predicate over this struct. The registry evaluates predicates to determine availability and filters the command list accordingly. This keeps all enablement logic in core, co-located with the command definitions, rather than scattered across app-layer UI code.

### 6. Separation of Registry and Execution

The command registry, search, and metadata live in the `cmdk` crate (framework-agnostic, no UI dependencies). The actual command *execution* is app-specific:

- The iced app (`crates/app/`) dispatches commands through its `Message` enum / `update()` cycle

Core owns: command identity, metadata, hierarchy, search, context requirements.
The app layer owns: binding command IDs to concrete handler implementations.

## Constraints

- **Rust edition 2024, strict clippy** — no `unwrap`, max 7 args, max 100 lines per function, no cognitive complexity
- **Command palette crate has no UI dependencies** — no iced, no GTK
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

## Decisions (continued)

3. **Parameterized command execution contract**: Resolved. The design separates command search (stage 1) from parameter resolution (stage 2) into distinct APIs with typed contracts.

   ### Stage Separation

   `CommandRegistry::query()` remains a top-level command search API. It does not serve second-stage options. Parameterized commands appear in query results as normal commands, with metadata telling the caller "this requires input before execution."

   The caller flow:
   1. `registry.query(ctx, query)` → user picks a `CommandMatch`
   2. If `input_mode == InputMode::Direct` → execute immediately with `CommandId` alone
   3. If `input_mode == InputMode::Parameterized { schema }` → use a separate `OptionProvider` to get options for each step in the schema, user picks values, caller builds a typed `CommandArgs`, then executes

   ### Input Mode

   `CommandMatch` carries an `input_mode` field:

   ```
   InputMode::Direct                              — no parameters, execute immediately
   InputMode::Parameterized { schema: InputSchema } — requires parameter resolution before execution
   ```

   ### Input Schema

   Commands declare their input requirements as a typed schema. This is not a generic form builder — it's an enum of input flows, starting minimal:

   ```
   InputSchema::Single(ParamDef)                  — one parameter, then done
   InputSchema::Sequence(&'static [ParamDef])      — multiple parameters resolved sequentially
   ```

   `ParamDef` describes one parameter step:

   ```
   ParamDef::ListPicker { label }                 — pick one item from a dynamic list (folders, labels, accounts)
   ParamDef::DateTime { label }                   — pick a date/time (snooze)
   ParamDef::Enum { label, options: &[EnumOption] } — pick from a fixed set; EnumOption { value, label } separates machine ID from display text
   ParamDef::Text { label, placeholder }           — free text input (rename folder, search query)
   ```

   `EnumOption` separates machine identifier (`value`) from display text (`label`). The app layer sends `value` back, not `label` — no localization hazard.

   Examples:
   - **Move to Folder**: `Single { param: ListPicker { label: "Folder" } }`
   - **Snooze**: `Single { param: DateTime { label: "Snooze until" } }`
   - **Compose with Template**: `Sequence { params: &[ListPicker { label: "Template" }, ListPicker { label: "Account" }] }`
   - **Switch Theme**: `Single { param: Enum { label: "Theme", options: &[EnumOption { value: "light", label: "Light" }, ...] } }`
   - **Rename Folder**: `Single { param: Text { label: "New name", placeholder: "Folder name" } }`

   This leaves room to add richer branching later (conditional steps, validation between steps) without pretending slice 2 is a full declarative form engine.

   ### Command Input Resolver (Trait)

   Parameterized commands need runtime data — option lists, validation, step transitions. This is a trait defined in core, implemented by the app layer:

   ```
   trait CommandInputResolver: Send + Sync {
       /// Return available options for a ListPicker parameter step.
       /// Only called for ListPicker steps — DateTime, Text, and Enum are
       /// app-layer-only input steps (the schema carries all needed info).
       /// prior_selections contains values chosen in steps 0..param_index
       /// for Sequence schemas (empty for Single).
       fn get_options(
           &self,
           command_id: CommandId,
           param_index: usize,
           prior_selections: &[String],
           ctx: &CommandContext,
       ) -> Result<Vec<OptionItem>, String>;

       /// Validate a selected value for any step type.
       /// - ListPicker: value is OptionItem.id
       /// - DateTime: value is stringified unix timestamp
       /// - Enum: value is EnumOption.value
       /// - Text: value is the user's input string
       /// prior_selections enables cross-field validation for sequences.
       fn validate_option(
           &self,
           command_id: CommandId,
           param_index: usize,
           value: &str,
           prior_selections: &[String],
           ctx: &CommandContext,
       ) -> Result<(), String>;
   }
   ```

   The name `CommandInputResolver` rather than `OptionProvider` reflects that this trait handles more than just listing options — it also validates selections and may grow to handle step transitions and preview data as input shapes expand. The `prior_selections` parameter on both methods enables sequence-aware resolution and cross-field validation from day one.

   The `CommandInputResolver` is a **separate object from the registry**. The registry is immutable static data. The resolver needs DB access and live account state — things the command palette crate cannot depend on. This follows the same pattern as `ProgressReporter`: core defines the trait, the app layer provides a concrete implementation.

   - The iced app (`crates/app/`) provides its own resolver implementation that queries its model state.
   - The resolver is registered in the app layer at startup, alongside but separate from the `CommandRegistry`.

   ### Ownership Summary

   ```
   CommandRegistry (core, static, immutable):
     - CommandId enum
     - CommandDescriptor (label, category, availability predicate, input schema)
     - Top-level command search (query)
     - Input schema declarations (what parameters a command needs)

   CommandInputResolver (core trait, app-implemented, live state):
     - Resolving dynamic options for a parameterized command step
     - Validating selected option IDs against current state
     - Future: hydrating labels/paths/previews, step transitions

   App layer (framework-specific):
     - Constructing CommandContext from framework state
     - Holding the concrete resolver implementation
     - Orchestrating the flow: registry → schema → resolver → user input → typed args → execute
   ```

   ### Option Items (Flat, Not Trees)

   The option provider returns a flat list, not a tree:

   ```
   OptionItem {
       id: String,                    — stable identifier for execution (folder ID, label ID)
       label: String,                 — leaf display name ("Reviews")
       path: Option<Vec<String>>,     — breadcrumb path for hierarchical display (["Projects", "Q2", "Reviews"])
       keywords: Option<Vec<String>>, — additional search terms (aliases, alternative names)
       disabled: bool,                — greyed out but visible (e.g., can't move to current folder)
   }
   ```

   Search operates on `label`, `path` segments (joined with " > "), and `keywords`. Including `path` in the search text means ancestor names are searchable — typing "q2" finds "Projects / Q2 / Reviews" without the resolver duplicating ancestors into keywords. The UI reconstructs hierarchical display from `path`. Execution uses `id`. This gives:
   - Flat fuzzy search over all options (including hierarchy)
   - Hierarchical display when desired (folder trees, nested labels)
   - No tree traversal API in core
   - Reusable structure for folders, labels, accounts, templates, and any future option type

   ### Execution Payload (Typed)

   The final execution payload is a typed enum in core — one variant per parameterized command or per small family of commands:

   ```
   CommandArgs::MoveToFolder { folder_id: String }
   CommandArgs::AddLabel { label_id: String }
   CommandArgs::RemoveLabel { label_id: String }
   CommandArgs::Snooze { until: i64 }  // unix timestamp
   CommandArgs::ComposeWithTemplate { template_id: String, account_id: String }
   CommandArgs::SwitchTheme { theme: String }
   CommandArgs::RenameFolder { folder_id: String, new_name: String }
   ```

   No `serde_json::Value`. No "one giant struct with every field optional." Each variant carries exactly the typed fields that command needs. The app layer matches on the variant and dispatches to the appropriate handler.

   For non-parameterized commands, execution takes only a `CommandId` — no `CommandArgs` needed.

4. **Disabled command visibility**: Resolved. `query()` returns all commands with an `available: bool` flag. The app layer decides whether to hide unavailable commands or show them greyed out. This keeps the API flexible for palette, context menus, and other surfaces. **Every registered command is palette-visible** — there is no `palette_visible` metadata flag. Navigation commands like "Next Thread" (j/k) appear in the palette alongside everything else. This may be revisited after V1, but for now it's a hard rule: if it's a command, it's searchable.

## Resolved Questions

1. **Palette visibility vs bindability**: Resolved. Every registered command is palette-searchable. There is no keyboard-only tier. Navigation commands like `nav.next` / `nav.prev` (j/k) appear in the palette — they're low-frequency palette searches but they need to be discoverable and their keybindings visible. This may be revisited post-V1 if the palette becomes noisy, but for now the rule is: no command exists without showing in the palette.

2. **Scope of "single source of truth"**: Resolved. The registry is the app's entire action layer, not just the palette's backend. Context menus, toolbars, right-click menus, the "Search here" sidebar action, reading pane action buttons — any UI surface that triggers or displays a command consumes the registry's metadata (label, icon, enabled state, keybinding hint). This is a consequence of the overview's requirement that "every user-initiated action... must be a registered command."

## Ecosystem Patterns

Cross-reference of this spec's requirements against the [iced ecosystem survey](../iced-ecosystem-survey.md). See the [full cross-reference](../iced-ecosystem-cross-reference.md) for broader context.

### Requirements to Survey Matches

| Requirement | Primary Source | How It Applies |
|---|---|---|
| Overlay widget | shadcn-rs command palette + overlay positioning | `place_overlay_centered()` for placement; focus trapping; props-builder for `CommandMatch` descriptors |
| Stage-1 vs stage-2 search routing | raffi `route_query()` | Enum dispatch: stage-1 queries `CommandRegistry::query()`, stage-2 queries `CommandInputResolver::get_options()` |
| MRU/recency ranking | raffi `MruEntry` | `HashMap<CommandId, MruEntry>` with count+timestamp, persisted to disk |
| Keyboard subscription batching | rustcast `Subscription::batch()` | Batches hotkeys, keyboard, and palette-internal subscriptions |
| Raw keyboard interception | feu `subscription::events_with` | Intercept KeyPressed before widget processing; modal keybinds without text input interference |
| Panel-aware dispatch | trebuchet Component trait | Each panel returns `(Task, ComponentEvent)`; `FocusedRegion` routes keyboard to correct component |
| Binding registration | cedilla declarative key bindings | Macro + HashMap lookup decoupling menu structure from handlers |
| Stale option resolution | bloom generational tracking | Cancel stale `CommandInputResolver::get_options()` results when user switches commands |

### Gaps

- **Two-key chord sequences** (`g then i`): No surveyed project handles pending chord state with timeouts. The `BindingTable` two-key sequence support is entirely custom.
- **User-customizable keybindings with conflict detection**: Beyond anything in the survey. The override map, conflict reporting, and per-platform defaults are original to Ratatoskr.
- **The core registry architecture** (`CommandId` enum, `CommandContext` predicates, `CommandInputResolver` trait, typed `CommandArgs`): No analogues in the surveyed ecosystem. The separation of static registry from live resolver, the three-tier command model, and the typed execution payload are all original designs.
