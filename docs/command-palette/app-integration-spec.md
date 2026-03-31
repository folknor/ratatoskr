# Command Palette: App Integration Spec

Implementation specification for wiring the command palette into the iced app's Elm architecture. This covers everything from the roadmap's Slice 6: palette overlay UI, keyboard dispatch, command-backed UI surfaces, and the supporting infrastructure that makes them work.

**Preconditions:** Slices 1-3 are complete. The `cmdk` crate provides `CommandRegistry`, `BindingTable`, `CommandContext`, `CommandInputResolver` trait, `InputSchema`/`ParamDef`/`InputMode`, and `search_options()`. All 55 commands are registered with fuzzy search, availability predicates, toggle labels, input schemas, and keybinding resolution. This spec is primarily `crates/app/` work, with one required addition to the command palette crate: `CommandArgs` (placed there for type-level guarantees and compile-time exhaustive matching in the dispatch layer). It also adds `AppOpenPalette` to the `CommandId` enum.

**App architecture prerequisites:** Several parts of this spec reference app-state accessors and component boundaries (`reading_pane.focused_message_id()`, `thread_list.selected_thread`, per-panel `Component` implementations) that may not exist in their final form yet. Where this spec references these, it is defining the target interface the app should expose — not assuming it already exists. Some of this work is app-state normalization that must happen alongside or slightly ahead of command integration.

## Table of Contents

1. [Core Infrastructure](#1-core-infrastructure)
2. [Path 1: Palette Overlay UI](#2-path-1-palette-overlay-ui)
3. [Path 2: Keyboard Dispatch](#3-path-2-keyboard-dispatch)
4. [Path 3: Command-Backed UI Surfaces](#4-path-3-command-backed-ui-surfaces)
5. [Phasing](#5-phasing)
6. [Ecosystem Patterns](#6-ecosystem-patterns)

---

## 1. Core Infrastructure

Four pieces of infrastructure that all three integration paths depend on. These are built first.

### 1.1 `CommandArgs` Enum

**File:** `crates/cmdk/src/args.rs` (new), re-exported from `crates/cmdk/src/lib.rs`

The typed execution payload for parameterized commands. One variant per command or command family. Non-parameterized commands execute with `CommandId` alone — no `CommandArgs` needed.

```rust
/// Typed execution payload for parameterized commands.
///
/// Each variant carries exactly the fields that command needs.
/// The app layer matches on the variant and dispatches to the
/// appropriate handler in `update()`.
#[derive(Debug, Clone)]
pub enum CommandArgs {
    /// EmailMoveToFolder — folder_id from ListPicker selection
    MoveToFolder { folder_id: String },
    /// EmailAddLabel — label_id from ListPicker selection
    AddLabel { label_id: String },
    /// EmailRemoveLabel — label_id from ListPicker selection
    RemoveLabel { label_id: String },
    /// EmailSnooze — unix timestamp from DateTime picker
    Snooze { until: i64 },
    /// NavigateToLabel — label_id from ListPicker selection.
    /// Includes account_id because cross-account navigation needs
    /// to know which account the label belongs to.
    NavigateToLabel { label_id: String, account_id: String },
}
```

**Design notes:**
- No `serde_json::Value`. No "one giant struct with every field optional." Each variant carries exactly what's needed.
- `NavigateToLabel` includes `account_id` because when navigating from an "All Accounts" context, the palette needs to know which account owns the selected label. The `OptionItem.id` for labels in cross-account context encodes both (e.g., `"gmail-abc:Label_42"`), and the resolver splits this into the two fields when building args.
- New parameterized commands add new variants here. The compiler enforces exhaustive matching in the dispatch function.

### 1.2 `CommandContext` Assembly

**File:** `crates/app/src/command_dispatch.rs` (new)

A function that snapshots the current `App` model state into a `CommandContext`. Called on every registry query (palette keystroke, keyboard event, UI surface render). Must be cheap — it reads fields, does not allocate except for `selected_thread_ids`.

```rust
use cmdk::{
    CommandContext, FocusedRegion, ProviderKind, ViewType,
};

/// Snapshot the app model into a CommandContext for registry queries.
///
/// Called frequently (every keystroke in the palette, every key event).
/// Must not perform DB access or block.
pub fn build_context(app: &App) -> CommandContext {
    let selected_thread_ids = selected_thread_ids(app);
    let active_message_id = app.reading_pane.focused_message_id();
    let (current_view, current_label_id) = current_view_and_label(app);
    let (active_account_id, provider_kind) = active_account_info(app);
    let thread_state = selected_thread_state(app);

    CommandContext {
        selected_thread_ids,
        active_message_id,
        current_view,
        current_label_id,
        active_account_id,
        provider_kind,
        thread_is_read: thread_state.is_read,
        thread_is_starred: thread_state.is_starred,
        thread_is_muted: thread_state.is_muted,
        thread_is_pinned: thread_state.is_pinned,
        thread_is_draft: thread_state.is_draft,
        thread_in_trash: thread_state.in_trash,
        thread_in_spam: thread_state.in_spam,
        is_online: app.is_online,
        composer_is_open: app.composer_is_open,
        focused_region: app.focused_region,
    }
}
```

**Required state additions to `App` (in `crates/app/src/main.rs`):**

```rust
struct App {
    // ... existing fields ...

    /// Which panel currently has focus. Updated on click/tab.
    focused_region: Option<FocusedRegion>,
    /// Network connectivity state.
    is_online: bool,
    /// Whether the compose window/panel is open.
    composer_is_open: bool,

    // Command palette infrastructure
    registry: CommandRegistry,
    binding_table: BindingTable,
    palette: PaletteState,
}
```

**Where each field comes from:**

| `CommandContext` field | Source |
|---|---|
| `selected_thread_ids` | `thread_list.selected_thread` (single selection for now; multi-select is future) — wrapped in `vec![id.clone()]` or `vec![]` |
| `active_message_id` | `reading_pane.focused_message_id()` (new accessor on `ReadingPane`) |
| `current_view` | Derived from sidebar selection state (inbox/starred/sent/etc.) — needs a `ViewType` enum mapping function |
| `current_label_id` | `sidebar.selected_label.clone()` |
| `active_account_id` | `sidebar.accounts.get(sidebar.selected_account?).map(|a| a.id.clone())` |
| `provider_kind` | `sidebar.accounts.get(sidebar.selected_account?).map(|a| a.provider_kind())` — needs a mapping from account DB type to `ProviderKind` |
| `thread_is_read` etc. | From the selected thread's flags in `thread_list.threads[selected_idx]` |
| `is_online` | New field on `App`, initially `true`, updated by a network check subscription (future) |
| `composer_is_open` | New field on `App`, toggled when compose is opened/closed |
| `focused_region` | New field on `App`, updated on panel click/focus events |

**`ViewType` mapping.** The sidebar currently tracks selection via `selected_account` index and `selected_label` ID. **The app should own an explicit `current_view: ViewType` field** on `App` rather than deriving view type heuristically from scattered sidebar fields. Heuristic derivation (e.g., "no label selected + account selected → Inbox") is fragile and will break as more navigation surfaces are added (search results, pinned searches, calendar mode). The `current_view` field should be set explicitly by navigation actions — clicking a sidebar item, executing a `NavigateTo` command, activating a search, etc. This is an app-state normalization that should happen as part of this integration work, not deferred.

The `ViewType` values:
- `ViewType::Inbox`, `Starred`, `Sent`, `Drafts`, `Snoozed`, `Trash`, `Spam`, `AllMail` — universal folders
- `ViewType::Label` — a user label/folder selected
- `ViewType::SmartFolder` — a smart folder selected
- `ViewType::Search` — active search results
- `ViewType::PinnedSearch` — a pinned search active
- `ViewType::Settings` — settings open
- `ViewType::Tasks`, `Attachments` — palette-first destinations

### 1.3 `CommandId` → `Message` Dispatch

**File:** `crates/app/src/command_dispatch.rs`

The central function that maps a command execution (either direct or parameterized) to an iced `Message` and feeds it into `update()`. This is the single point where "what to do" (from the registry) connects to "how to do it" (the app's message handling).

```rust
use cmdk::{CommandArgs, CommandId};

/// Map a direct (non-parameterized) command to an iced Message.
///
/// Returns `None` for commands that are not yet implemented,
/// allowing incremental rollout.
pub fn dispatch_command(id: CommandId, app: &App) -> Option<Message> {
    match id {
        // Navigation
        CommandId::NavNext => Some(Message::ThreadList(ThreadListMessage::SelectNext)),
        CommandId::NavPrev => Some(Message::ThreadList(ThreadListMessage::SelectPrev)),
        CommandId::NavOpen => Some(Message::ThreadList(ThreadListMessage::OpenSelected)),
        CommandId::NavMsgNext => Some(Message::ReadingPane(ReadingPaneMessage::NextMessage)),
        CommandId::NavMsgPrev => Some(Message::ReadingPane(ReadingPaneMessage::PrevMessage)),
        CommandId::NavGoInbox => Some(Message::NavigateTo(NavigationTarget::Inbox)),
        CommandId::NavGoStarred => Some(Message::NavigateTo(NavigationTarget::Starred)),
        CommandId::NavGoSent => Some(Message::NavigateTo(NavigationTarget::Sent)),
        CommandId::NavGoDrafts => Some(Message::NavigateTo(NavigationTarget::Drafts)),
        CommandId::NavGoSnoozed => Some(Message::NavigateTo(NavigationTarget::Snoozed)),
        CommandId::NavGoTrash => Some(Message::NavigateTo(NavigationTarget::Trash)),
        CommandId::NavGoAllMail => Some(Message::NavigateTo(NavigationTarget::AllMail)),
        CommandId::NavGoPrimary => Some(Message::NavigateTo(NavigationTarget::Primary)),
        CommandId::NavGoUpdates => Some(Message::NavigateTo(NavigationTarget::Updates)),
        CommandId::NavGoPromotions => Some(Message::NavigateTo(NavigationTarget::Promotions)),
        CommandId::NavGoSocial => Some(Message::NavigateTo(NavigationTarget::Social)),
        CommandId::NavGoNewsletters => Some(Message::NavigateTo(NavigationTarget::Newsletters)),
        CommandId::NavGoTasks => Some(Message::NavigateTo(NavigationTarget::Tasks)),
        CommandId::NavGoAttachments => Some(Message::NavigateTo(NavigationTarget::Attachments)),
        CommandId::NavEscape => Some(Message::Escape),

        // Email actions
        CommandId::EmailArchive => Some(Message::EmailAction(EmailAction::Archive)),
        CommandId::EmailTrash => Some(Message::EmailAction(EmailAction::Trash)),
        CommandId::EmailPermanentDelete => Some(Message::EmailAction(EmailAction::PermanentDelete)),
        CommandId::EmailSpam => Some(Message::EmailAction(EmailAction::ToggleSpam)),
        CommandId::EmailMarkRead => Some(Message::EmailAction(EmailAction::ToggleRead)),
        CommandId::EmailStar => Some(Message::EmailAction(EmailAction::ToggleStar)),
        CommandId::EmailPin => Some(Message::EmailAction(EmailAction::TogglePin)),
        CommandId::EmailMute => Some(Message::EmailAction(EmailAction::ToggleMute)),
        CommandId::EmailUnsubscribe => Some(Message::EmailAction(EmailAction::Unsubscribe)),
        CommandId::EmailSelectAll => Some(Message::ThreadList(ThreadListMessage::SelectAll)),
        CommandId::EmailSelectFromHere => {
            Some(Message::ThreadList(ThreadListMessage::SelectFromHere))
        }

        // Parameterized commands — these open the palette's stage 2,
        // not dispatched directly. Handled via dispatch_parameterized.
        CommandId::EmailMoveToFolder
        | CommandId::EmailAddLabel
        | CommandId::EmailRemoveLabel
        | CommandId::NavigateToLabel
        | CommandId::EmailSnooze => None,

        // Compose
        CommandId::ComposeNew => Some(Message::Compose),
        CommandId::ComposeReply => Some(Message::ComposeAction(ComposeAction::Reply)),
        CommandId::ComposeReplyAll => Some(Message::ComposeAction(ComposeAction::ReplyAll)),
        CommandId::ComposeForward => Some(Message::ComposeAction(ComposeAction::Forward)),

        // Tasks
        CommandId::TaskCreate => Some(Message::TaskAction(TaskAction::Create)),
        CommandId::TaskCreateFromEmail => Some(Message::TaskAction(TaskAction::CreateFromEmail)),
        CommandId::TaskTogglePanel => Some(Message::TaskAction(TaskAction::TogglePanel)),
        CommandId::TaskViewAll => Some(Message::NavigateTo(NavigationTarget::Tasks)),

        // View
        CommandId::ViewToggleSidebar => Some(Message::ToggleSidebar),
        CommandId::ViewSetThemeLight => Some(Message::SetTheme("Light".to_string())),
        CommandId::ViewSetThemeDark => Some(Message::SetTheme("Dark".to_string())),
        CommandId::ViewSetThemeSystem => Some(Message::SetTheme("System".to_string())),
        CommandId::ViewToggleTaskPanel => Some(Message::TaskAction(TaskAction::TogglePanel)),
        CommandId::ViewReadingPaneRight => {
            Some(Message::SetReadingPanePosition(ReadingPanePosition::Right))
        }
        CommandId::ViewReadingPaneBottom => {
            Some(Message::SetReadingPanePosition(ReadingPanePosition::Bottom))
        }
        CommandId::ViewReadingPaneHidden => {
            Some(Message::SetReadingPanePosition(ReadingPanePosition::Hidden))
        }

        // App
        CommandId::AppSearch => Some(Message::FocusSearch),
        CommandId::AppAskAi => None, // Not yet implemented
        CommandId::AppHelp => Some(Message::ShowHelp),
        CommandId::AppSyncFolder => Some(Message::SyncCurrentFolder),
    }
}

/// Map a parameterized command + resolved args to an iced Message.
pub fn dispatch_parameterized(id: CommandId, args: CommandArgs) -> Option<Message> {
    match (id, args) {
        (CommandId::EmailMoveToFolder, CommandArgs::MoveToFolder { folder_id }) => {
            Some(Message::EmailAction(EmailAction::MoveToFolder { folder_id }))
        }
        (CommandId::EmailAddLabel, CommandArgs::AddLabel { label_id }) => {
            Some(Message::EmailAction(EmailAction::AddLabel { label_id }))
        }
        (CommandId::EmailRemoveLabel, CommandArgs::RemoveLabel { label_id }) => {
            Some(Message::EmailAction(EmailAction::RemoveLabel { label_id }))
        }
        (CommandId::EmailSnooze, CommandArgs::Snooze { until }) => {
            Some(Message::EmailAction(EmailAction::Snooze { until }))
        }
        (CommandId::NavigateToLabel, CommandArgs::NavigateToLabel { label_id, account_id }) => {
            Some(Message::NavigateTo(NavigationTarget::Label { label_id, account_id }))
        }
        _ => None,
    }
}
```

**New `Message` variants required** (added to the existing `Message` enum in `crates/app/src/main.rs`):

```rust
#[derive(Debug, Clone)]
pub enum Message {
    // ... existing variants ...

    // Command palette
    Palette(PaletteMessage),

    // Command dispatch (new)
    NavigateTo(NavigationTarget),
    Escape,
    EmailAction(EmailAction),
    ComposeAction(ComposeAction),
    TaskAction(TaskAction),
    SetTheme(String),
    ToggleSidebar,
    FocusSearch,
    ShowHelp,
    SyncCurrentFolder,
    SetReadingPanePosition(ReadingPanePosition),
    ExecuteCommand(CommandId),
    ExecuteParameterized(CommandId, CommandArgs),

    // Keyboard dispatch
    KeyEvent(KeyEventMessage),
}
```

**Supporting enums:**

```rust
#[derive(Debug, Clone)]
pub enum NavigationTarget {
    Inbox,
    Starred,
    Sent,
    Drafts,
    Snoozed,
    Trash,
    AllMail,
    Primary,
    Updates,
    Promotions,
    Social,
    Newsletters,
    Tasks,
    Attachments,
    Label { label_id: String, account_id: String },
}

#[derive(Debug, Clone)]
pub enum EmailAction {
    Archive,
    Trash,
    PermanentDelete,
    ToggleSpam,
    ToggleRead,
    ToggleStar,
    TogglePin,
    ToggleMute,
    Unsubscribe,
    MoveToFolder { folder_id: String },
    AddLabel { label_id: String },
    RemoveLabel { label_id: String },
    Snooze { until: i64 },
}

#[derive(Debug, Clone)]
pub enum ComposeAction {
    Reply,
    ReplyAll,
    Forward,
}

#[derive(Debug, Clone)]
pub enum TaskAction {
    Create,
    CreateFromEmail,
    TogglePanel,
}

#[derive(Debug, Clone, Copy)]
pub enum ReadingPanePosition {
    Right,
    Bottom,
    Hidden,
}
```

**`ExecuteCommand` handler in `update()`:**

```rust
Message::ExecuteCommand(id) => {
    self.registry.usage.record_usage(id);
    match dispatch_command(id, self) {
        Some(msg) => self.update(msg),
        None => Task::none(),
    }
}
Message::ExecuteParameterized(id, args) => {
    self.registry.usage.record_usage(id);
    match dispatch_parameterized(id, args) {
        Some(msg) => self.update(msg),
        None => Task::none(),
    }
}
// Note: The recursive self.update(msg) calls above are idiomatic in iced's
// Elm architecture but require care. The dispatched message must not itself
// produce another ExecuteCommand (which would recurse unboundedly). This is
// safe because dispatch_command/dispatch_parameterized produce concrete
// action messages (NavigateTo, EmailAction, etc.), not meta-commands.
// If the update() function grows very large, consider extracting dispatch
// into a separate method to keep the recursion shallow and auditable.
```

### 1.4 `CommandInputResolver` Implementation

**File:** `crates/app/src/command_resolver.rs` (new)

A concrete implementation of the `CommandInputResolver` trait that queries `DbState` for folders, labels, and accounts. This provides the option lists for parameterized commands' stage 2.

```rust
use cmdk::{
    CommandContext, CommandId, CommandInputResolver, OptionItem,
};
use std::sync::Arc;
use crate::db::Db;

pub struct AppInputResolver {
    db: Arc<Db>,
}

impl AppInputResolver {
    pub fn new(db: Arc<Db>) -> Self {
        Self { db }
    }
}

impl CommandInputResolver for AppInputResolver {
    fn get_options(
        &self,
        command_id: CommandId,
        param_index: usize,
        _prior_selections: &[String],
        ctx: &CommandContext,
    ) -> Result<Vec<OptionItem>, String> {
        match (command_id, param_index) {
            (CommandId::EmailMoveToFolder, 0) => {
                self.get_folder_options(ctx)
            }
            (CommandId::EmailAddLabel, 0) => {
                self.get_label_options(ctx, false)
            }
            (CommandId::EmailRemoveLabel, 0) => {
                self.get_label_options(ctx, true)
            }
            (CommandId::NavigateToLabel, 0) => {
                // Cross-account: all user labels from all accounts,
                // with account name in path for disambiguation
                self.get_all_label_options_cross_account(ctx)
            }
            _ => Ok(vec![]),
        }
    }

    fn validate_option(
        &self,
        _command_id: CommandId,
        _param_index: usize,
        _value: &str,
        _prior_selections: &[String],
        _ctx: &CommandContext,
    ) -> Result<(), String> {
        // Validation is lenient for now — accept any non-empty value.
        // Tighten per-command as real email actions are wired up.
        Ok(())
    }
}
```

**Option resolution methods.** Each queries the DB via `db.conn()` (the `Arc<Mutex<Connection>>` pattern from CLAUDE.md). **Resolution must be async** — even though result sets are small (tens to low hundreds of items), the palette is a high-frequency UI path and mutex contention could cause jank. The palette's `Confirm` handler dispatches a `Task::perform` wrapping the resolver call, and the result arrives via `OptionsLoaded`. Each resolver call is tagged with a generation counter (`option_load_generation: u64` on `PaletteState`) to discard stale results if the user switches commands or types quickly between resolver calls.

```rust
impl AppInputResolver {
    fn get_folder_options(
        &self,
        ctx: &CommandContext,
    ) -> Result<Vec<OptionItem>, String> {
        let account_id = ctx.active_account_id.as_deref()
            .ok_or_else(|| "no active account".to_string())?;
        // Query labels/folders for this account, excluding system labels
        // (INBOX, SENT, TRASH, etc. — those have dedicated nav commands).
        // Build OptionItem with path segments for hierarchical folders.
        // Implementation queries the labels table via Db.
        self.db.get_user_folders_for_palette(account_id)
    }

    fn get_label_options(
        &self,
        ctx: &CommandContext,
        current_thread_only: bool,
    ) -> Result<Vec<OptionItem>, String> {
        let account_id = ctx.active_account_id.as_deref()
            .ok_or_else(|| "no active account".to_string())?;
        if current_thread_only {
            // RemoveLabel: only show labels currently on the selected thread
            let thread_id = ctx.selected_thread_ids.first()
                .ok_or_else(|| "no thread selected".to_string())?;
            self.db.get_thread_labels_for_palette(account_id, thread_id)
        } else {
            // AddLabel: show all user labels for the account
            self.db.get_user_labels_for_palette(account_id)
        }
    }
}
```

**New `Db` methods needed** (in `crates/app/src/db.rs`):

| Method | Returns | Description |
|---|---|---|
| `get_user_folders_for_palette(&self, account_id: &str) -> Result<Vec<OptionItem>, String>` | Flat list of user-visible folders/labels with `path` segments for hierarchy | Excludes system labels (INBOX, SENT, etc.). For Gmail, splits `/`-delimited labels into path segments. For Exchange/JMAP/IMAP, walks the folder hierarchy. |
| `get_user_labels_for_palette(&self, account_id: &str) -> Result<Vec<OptionItem>, String>` | All user labels for the account | Same as folders but may include non-folder labels (Gmail labels that aren't folders). |
| `get_thread_labels_for_palette(&self, account_id: &str, thread_id: &str) -> Result<Vec<OptionItem>, String>` | Labels currently applied to the thread | For "Remove Label" — only shows what can be removed. |

These methods query the existing `labels` table in `ratatoskr.db` and return `OptionItem` structs. The `OptionItem.id` is the label's provider-side ID (e.g., Gmail label ID, Exchange folder ID). The `OptionItem.path` is built from the label's name, splitting Gmail `/`-separated labels or following Exchange parent_id chains.

**Cross-account resolution.** When `ctx.active_account_id` is `None` (All Accounts view), the resolver queries all accounts and prefixes `OptionItem.path` with the account display name:

```rust
// All accounts: query each, prefix path with account name
for account in self.db.get_all_accounts_sync()? {
    let account_name = account.display_name
        .unwrap_or_else(|| account.email.clone());
    let mut items = self.db.get_user_folders_for_palette(&account.id)?;
    for item in &mut items {
        let mut new_path = vec![account_name.clone()];
        if let Some(existing) = item.path.take() {
            new_path.extend(existing);
        }
        item.path = Some(new_path);
        // Encode account_id into item.id for disambiguation
        item.id = format!("{}:{}", account.id, item.id);
    }
    all_items.extend(items);
}
```

### 1.5 Initialization

**File:** `crates/app/src/main.rs`, in `App::boot()`

```rust
fn boot() -> (Self, Task<Message>) {
    let db = Arc::clone(DB.get().expect("DB not initialized"));
    let registry = CommandRegistry::new();
    let binding_table = BindingTable::new(&registry, current_platform());
    // TODO (Slice 6c): load overrides from settings DB
    // binding_table.load_overrides(load_keybinding_overrides(&db));
    let resolver = Arc::new(AppInputResolver::new(Arc::clone(&db)));

    let app = Self {
        // ... existing fields ...
        registry,
        binding_table,
        resolver,
        palette: PaletteState::new(),
        focused_region: None,
        is_online: true,
        composer_is_open: false,
    };
    // ... existing boot task ...
}
```

---

## 2. Path 1: Palette Overlay UI

### 2.1 State Model

**File:** `crates/app/src/ui/palette.rs` (new)

The palette is a `Component` (per the existing `Component` trait in `crates/app/src/component.rs`) that manages its own state and emits events to the parent `App`.

```rust
/// Palette lifecycle states.
#[derive(Debug, Clone)]
pub enum PaletteStage {
    /// Stage 1: searching commands via CommandRegistry::query().
    CommandSearch {
        query: String,
        results: Vec<CommandMatch>,
        selected_index: usize,
    },
    /// Stage 2: picking an option for a parameterized command.
    /// V1 constraint: this handles single-step parameterized commands only
    /// (InputSchema::Single). The underlying command system supports
    /// InputSchema::Sequence (multi-step), but the palette UI does not
    /// implement sequential step navigation yet. All current parameterized
    /// commands are Single. Sequence support is deferred until a command
    /// actually needs it (e.g., ComposeWithTemplate which needs template
    /// + account selection).
    OptionPick {
        command_id: CommandId,
        command_label: &'static str,
        param_def: ParamDef,
        query: String,
        options: Vec<OptionMatch>,
        selected_index: usize,
    },
}

pub struct PaletteState {
    /// None = palette closed. Some = palette open at given stage.
    pub stage: Option<PaletteStage>,
}

impl PaletteState {
    pub fn new() -> Self {
        Self { stage: None }
    }

    pub fn is_open(&self) -> bool {
        self.stage.is_some()
    }

    pub fn open(&mut self, results: Vec<CommandMatch>) {
        self.stage = Some(PaletteStage::CommandSearch {
            query: String::new(),
            results,
            selected_index: 0,
        });
    }

    pub fn close(&mut self) {
        self.stage = None;
    }
}
```

### 2.2 Messages and Events

```rust
#[derive(Debug, Clone)]
pub enum PaletteMessage {
    /// Open the palette (triggered by Ctrl+K or palette button).
    Open,
    /// Close the palette (Escape, click outside, or command executed).
    Close,
    /// Text input changed in the search field.
    QueryChanged(String),
    /// Arrow key navigation within the results list.
    SelectNext,
    SelectPrev,
    /// Enter pressed — confirm the currently highlighted item.
    Confirm,
    /// Stage 2: option list loaded from resolver.
    OptionsLoaded(CommandId, Result<Vec<OptionItem>, String>),
}

/// Events emitted upward to the App.
#[derive(Debug, Clone)]
pub enum PaletteEvent {
    /// A direct command was selected — dispatch it.
    ExecuteCommand(CommandId),
    /// A parameterized command completed stage 2 — dispatch with args.
    ExecuteParameterized(CommandId, CommandArgs),
    /// Palette was dismissed without executing anything.
    Dismissed,
}
```

### 2.3 Update Logic

The palette's `update()` follows the `Component` trait pattern: `(Task<PaletteMessage>, Option<PaletteEvent>)`.

**`Open`:** Build a `CommandContext` from the app, call `registry.query(&ctx, "")`, populate `PaletteStage::CommandSearch` with results, return a `Task` that focuses the text input widget (`widget::operation::focus("palette-input")`).

**`QueryChanged(query)`:**
- In `CommandSearch` stage: call `registry.query(&ctx, &query)`, update results and reset `selected_index` to 0.
- In `OptionPick` stage: call `search_options(&cached_options, &query)`, update filtered results and reset index.

**`SelectNext` / `SelectPrev`:** Increment/decrement `selected_index`, clamped to `0..results.len()`.

**`Confirm`:**
- In `CommandSearch` stage:
  - If the selected `CommandMatch` has `input_mode == InputMode::Direct`:
    - Emit `PaletteEvent::ExecuteCommand(id)`.
    - Close the palette.
  - If `input_mode == InputMode::Parameterized { schema }`:
    - Transition to `PaletteStage::OptionPick`.
    - Return a `Task` that calls the resolver's `get_options()` (via `Task::perform` wrapping a blocking call).
    - Show "Loading..." placeholder until `OptionsLoaded` arrives.
- In `OptionPick` stage:
  - Build `CommandArgs` from the selected `OptionItem.id` and the command ID.
  - Emit `PaletteEvent::ExecuteParameterized(id, args)`.
  - Close the palette.

**`OptionsLoaded(id, generation, result)`:**
- If `generation < self.option_load_generation`: discard (stale result from a previous command selection). This prevents displaying options for a command the user has already navigated away from.
- On `Ok(items)`: populate `PaletteStage::OptionPick` with the items, apply the current query filter via `search_options()`.
- On `Err(msg)`: show error in the palette UI (replace results area with error text). Do not close.

**`Close`:** Set `stage = None`, emit `PaletteEvent::Dismissed`.

**Building `CommandArgs` from stage 2 selection.** When the user confirms an option in stage 2, the palette must construct the correct `CommandArgs` variant:

```rust
fn build_args(
    command_id: CommandId,
    selected_item: &OptionItem,
) -> Option<CommandArgs> {
    match command_id {
        CommandId::EmailMoveToFolder => {
            Some(CommandArgs::MoveToFolder {
                folder_id: selected_item.id.clone(),
            })
        }
        CommandId::EmailAddLabel => {
            Some(CommandArgs::AddLabel {
                label_id: selected_item.id.clone(),
            })
        }
        CommandId::EmailRemoveLabel => {
            Some(CommandArgs::RemoveLabel {
                label_id: selected_item.id.clone(),
            })
        }
        CommandId::EmailSnooze => {
            // DateTime picker returns a stringified unix timestamp
            selected_item.id.parse::<i64>().ok().map(|ts| {
                CommandArgs::Snooze { until: ts }
            })
        }
        CommandId::NavigateToLabel => {
            // Cross-account: item.id is "account_id:label_id"
            let (account_id, label_id) = selected_item.id
                .split_once(':')
                .map(|(a, l)| (a.to_string(), l.to_string()))
                .unwrap_or_else(|| (String::new(), selected_item.id.clone()));
            Some(CommandArgs::NavigateToLabel { label_id, account_id })
        }
        _ => None,
    }
}
```

### 2.4 View / Widget Tree

**File:** `crates/app/src/ui/palette.rs`

The palette renders as an overlay centered horizontally, near the top of the window (vertically offset ~20% from top). It does NOT live inside the normal layout flow — it floats above everything.

**Widget tree:**

```
container (centered overlay backdrop — full window, semi-transparent)
  └── mouse_area (captures clicks on backdrop → Close)
      └── container (palette card — fixed width 600px, max-height 400px)
          ├── text_input (search field, id="palette-input")
          │     placeholder: stage 1 → "Type a command..."
          │                  stage 2 → "Search {param_label}..."
          │     on_input: QueryChanged
          │     on_submit: Confirm
          └── scrollable (results list, max-height 340px)
              └── column (one row per result)
                  └── palette_result_row(match, is_selected)
```

**`palette_result_row` for stage 1 (command search):**

```
mouse_area (on_press → select + confirm)
  └── container (row height 36px, background highlight if selected)
      └── row
          ├── container (category badge — dimmed text, fixed width)
          │     text(match.category).size(TEXT_SM).style(text_muted)
          ├── container (label — fills remaining space)
          │     text(match.label).size(TEXT_MD)
          │     // If !match.available: style(text_disabled)
          └── container (keybinding hint — right-aligned, fixed width)
                text(match.keybinding.unwrap_or_default())
                  .size(TEXT_SM).style(text_muted)
                // Rendered in a subtle badge/pill style
```

**`palette_result_row` for stage 2 (option pick):**

```
mouse_area (on_press → select + confirm)
  └── container (row height 36px, background highlight if selected)
      └── row
          ├── container (label — fills remaining space)
          │     text(option.item.label).size(TEXT_MD)
          │     // If option.item.disabled: style(text_disabled)
          └── container (path breadcrumb — right-aligned, dimmed)
                text(path_display(&option.item.path))
                  .size(TEXT_SM).style(text_muted)
```

**Rendering the overlay.** iced does not have a built-in overlay/modal primitive. The palette is rendered using `iced::widget::stack`:

```rust
// In App::view(), after building the normal layout:
fn view(&self) -> Element<'_, Message> {
    let main_layout = self.build_main_layout();

    if self.palette.is_open() {
        let backdrop = mouse_area(
            container("")
                .width(Length::Fill)
                .height(Length::Fill)
                .style(palette_backdrop_style),
        )
        .on_press(Message::Palette(PaletteMessage::Close));

        let palette_widget = self.palette.view(&self.registry)
            .map(Message::Palette);

        let palette_positioned = container(palette_widget)
            .width(Length::Fill)
            .padding([80, 0, 0, 0])  // 80px from top
            .align_x(iced::Alignment::Center);

        stack![main_layout, backdrop, palette_positioned].into()
    } else {
        main_layout
    }
}
```

**Important:** Per UI.md, `iced::widget::stack` does not block events on lower layers. The `mouse_area` backdrop between the main layout and the palette widget is essential — it captures all clicks that miss the palette, preventing interaction with the mail UI underneath. The `on_press` handler sends `PaletteMessage::Close`.

### 2.5 Focus Management

When the palette opens, the text input must receive focus immediately. This is done via `widget::operation::focus("palette-input".to_string())` returned as a `Task` from the `Open` handler.

When the palette is open, keyboard events must be intercepted by the palette, not by the normal keyboard dispatch system. The keyboard subscription (Path 2) checks `palette.is_open()` and routes differently:

- **Palette open:** Only `Escape` (close palette), `ArrowUp`/`ArrowDown` (navigate), `Enter` (confirm) are intercepted. All other key events flow to the `text_input` normally.
- **Palette closed:** Normal keyboard dispatch via `BindingTable`.

### 2.6 Styling Constants

**File:** `crates/app/src/ui/layout.rs` (additions)

```rust
// Palette overlay
pub const PALETTE_WIDTH: f32 = 600.0;
pub const PALETTE_MAX_HEIGHT: f32 = 400.0;
pub const PALETTE_TOP_OFFSET: f32 = 80.0;
pub const PALETTE_RESULT_HEIGHT: f32 = 36.0;
pub const PALETTE_INPUT_HEIGHT: f32 = 44.0;
pub const PALETTE_CATEGORY_WIDTH: f32 = 80.0;
pub const PALETTE_KEYBINDING_WIDTH: f32 = 100.0;
```

**File:** `crates/app/src/ui/theme.rs` (additions)

```rust
// Palette styles
pub fn palette_backdrop_style(theme: &Theme) -> container::Style {
    let palette = theme.palette();
    container::Style {
        background: Some(Color { a: 0.5, ..palette.background.base.color }.into()),
        ..Default::default()
    }
}

pub fn palette_card_style(theme: &Theme) -> container::Style {
    let palette = theme.palette();
    container::Style {
        background: Some(palette.background.weak.color.into()),
        border: Border {
            radius: RADIUS_LG.into(),
            width: 1.0,
            color: palette.background.strong.color,
        },
        shadow: Shadow {
            color: Color { a: 0.3, ..Color::BLACK },
            offset: iced::Vector::new(0.0, 4.0),
            blur_radius: 16.0,
        },
        ..Default::default()
    }
}

pub fn palette_selected_row_style(theme: &Theme) -> container::Style {
    let palette = theme.palette();
    container::Style {
        background: Some(palette.primary.weak.color.into()),
        border: Border::default().rounded(RADIUS_SM),
        ..Default::default()
    }
}
```

---

## 3. Path 2: Keyboard Dispatch

### 3.1 Key Event Subscription

**File:** `crates/app/src/main.rs`, in `App::subscription()`

iced provides `iced::event::listen_with` which receives all events before widget processing. We use this to capture keyboard events and route them through `BindingTable`.

```rust
fn subscription(&self) -> iced::Subscription<Message> {
    let mut subs = vec![
        // ... existing subscriptions ...

        // Global keyboard dispatch
        iced::event::listen_with(|event, status, _id| {
            // Only intercept key presses. Let key releases and
            // other events pass through.
            if let iced::Event::Keyboard(
                iced::keyboard::Event::KeyPressed { key, modifiers, .. }
            ) = &event {
                Some(Message::KeyEvent(KeyEventMessage::KeyPressed {
                    key: key.clone(),
                    modifiers: *modifiers,
                    status: *status,
                }))
            } else {
                None
            }
        }),
    ];
    // ... rest of subscription ...
}
```

### 3.2 Key Event Message

```rust
#[derive(Debug, Clone)]
pub enum KeyEventMessage {
    KeyPressed {
        key: iced::keyboard::Key,
        modifiers: iced::keyboard::Modifiers,
        /// Whether the event was already handled by a widget (e.g., text_input).
        status: iced::event::Status,
    },
    /// Pending chord timed out — cancel the sequence.
    PendingChordTimeout,
}
```

### 3.3 Pending Chord State

**File:** `crates/app/src/main.rs` (new field on `App`)

```rust
struct App {
    // ... existing fields ...

    /// When a two-chord sequence's first chord matches, we enter pending
    /// state and wait for the second chord (or a timeout).
    pending_chord: Option<PendingChord>,
}

struct PendingChord {
    first: Chord,
    /// When the first chord was pressed. Used for timeout.
    started: iced::time::Instant,
}

/// How long to wait for the second chord of a sequence.
const CHORD_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1000);
```

**Timeout subscription.** When `pending_chord` is `Some`, add a time subscription:

```rust
if self.pending_chord.is_some() {
    subs.push(
        iced::time::every(CHORD_TIMEOUT)
            .map(|_| Message::KeyEvent(KeyEventMessage::PendingChordTimeout)),
    );
}
```

### 3.4 Key Event Handler

**File:** `crates/app/src/main.rs` (or `command_dispatch.rs`)

The handler converts iced's `keyboard::Key` + `keyboard::Modifiers` into the command palette's `Chord` type, then routes through the `BindingTable`.

```rust
fn handle_key_event(&mut self, msg: KeyEventMessage) -> Task<Message> {
    match msg {
        KeyEventMessage::KeyPressed { key, modifiers, status } => {
            self.handle_key_pressed(key, modifiers, status)
        }
        KeyEventMessage::PendingChordTimeout => {
            self.pending_chord = None;
            Task::none()
        }
    }
}

fn handle_key_pressed(
    &mut self,
    key: iced::keyboard::Key,
    modifiers: iced::keyboard::Modifiers,
    status: iced::event::Status,
) -> Task<Message> {
    // 1. If palette is open, route to palette
    if self.palette.is_open() {
        return self.handle_palette_key(key, modifiers);
    }

    // 2. If a text input or other widget captured the event, skip
    //    (unless it's a modifier-chord like Ctrl+K)
    if status == iced::event::Status::Captured
        && !has_command_modifier(&modifiers)
    {
        return Task::none();
    }

    // 3. Convert iced key to cmdk Chord
    let Some(chord) = iced_key_to_chord(&key, &modifiers) else {
        return Task::none();
    };

    // 4. If we're in pending chord state, resolve the sequence
    if let Some(pending) = self.pending_chord.take() {
        if let Some(id) = self.binding_table.resolve_sequence(
            &pending.first, &chord
        ) {
            return self.update(Message::ExecuteCommand(id));
        }
        // Second chord didn't match any sequence — discard
        return Task::none();
    }

    // 5. Resolve single chord
    match self.binding_table.resolve_chord(&chord) {
        ResolveResult::Command(id) => {
            self.update(Message::ExecuteCommand(id))
        }
        ResolveResult::Pending => {
            self.pending_chord = Some(PendingChord {
                first: chord,
                started: iced::time::Instant::now(),
            });
            Task::none()
        }
        ResolveResult::NoMatch => Task::none(),
    }
}
```

### 3.5 iced Key → Command Palette Chord Conversion

```rust
/// Convert iced keyboard types to cmdk Chord.
///
/// Returns None for keys we don't handle (modifiers alone, etc.)
fn iced_key_to_chord(
    key: &iced::keyboard::Key,
    modifiers: &iced::keyboard::Modifiers,
) -> Option<Chord> {
    let cp_key = match key {
        iced::keyboard::Key::Character(c) => {
            let ch = c.chars().next()?;
            Key::Char(ch.to_ascii_lowercase())
        }
        iced::keyboard::Key::Named(named) => {
            let cp_named = iced_named_to_cp(*named)?;
            Key::Named(cp_named)
        }
        _ => return None,
    };

    let cp_modifiers = Modifiers {
        cmd_or_ctrl: modifiers.command() || modifiers.control(),
        shift: modifiers.shift(),
        alt: modifiers.alt(),
    };

    Some(Chord {
        key: cp_key,
        modifiers: cp_modifiers,
    })
}

/// Map iced named keys to cmdk NamedKey.
fn iced_named_to_cp(named: iced::keyboard::key::Named) -> Option<NamedKey> {
    use iced::keyboard::key::Named as I;
    match named {
        I::Escape => Some(NamedKey::Escape),
        I::ArrowUp => Some(NamedKey::ArrowUp),
        I::ArrowDown => Some(NamedKey::ArrowDown),
        I::ArrowLeft => Some(NamedKey::ArrowLeft),
        I::ArrowRight => Some(NamedKey::ArrowRight),
        I::Enter => Some(NamedKey::Enter),
        I::Tab => Some(NamedKey::Tab),
        I::Space => Some(NamedKey::Space),
        I::Backspace => Some(NamedKey::Backspace),
        I::Delete => Some(NamedKey::Delete),
        I::Home => Some(NamedKey::Home),
        I::End => Some(NamedKey::End),
        I::PageUp => Some(NamedKey::PageUp),
        I::PageDown => Some(NamedKey::PageDown),
        I::F1 => Some(NamedKey::F1),
        I::F2 => Some(NamedKey::F2),
        I::F3 => Some(NamedKey::F3),
        I::F4 => Some(NamedKey::F4),
        I::F5 => Some(NamedKey::F5),
        I::F6 => Some(NamedKey::F6),
        I::F7 => Some(NamedKey::F7),
        I::F8 => Some(NamedKey::F8),
        I::F9 => Some(NamedKey::F9),
        I::F10 => Some(NamedKey::F10),
        I::F11 => Some(NamedKey::F11),
        I::F12 => Some(NamedKey::F12),
        _ => None,
    }
}

fn has_command_modifier(modifiers: &iced::keyboard::Modifiers) -> bool {
    modifiers.command() || modifiers.control()
}
```

### 3.6 Palette-Specific Key Handling

When the palette is open, arrow keys, Enter, and Escape are routed to the palette instead of the normal dispatch:

```rust
fn handle_palette_key(
    &mut self,
    key: iced::keyboard::Key,
    _modifiers: iced::keyboard::Modifiers,
) -> Task<Message> {
    match key {
        iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape) => {
            self.update(Message::Palette(PaletteMessage::Close))
        }
        iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowDown) => {
            self.update(Message::Palette(PaletteMessage::SelectNext))
        }
        iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowUp) => {
            self.update(Message::Palette(PaletteMessage::SelectPrev))
        }
        iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter) => {
            self.update(Message::Palette(PaletteMessage::Confirm))
        }
        _ => Task::none(),
    }
}
```

### 3.7 Pending Chord Indicator

When `pending_chord` is `Some`, the status bar (once implemented) shows a transient indicator. Until the status bar exists, the pending chord can be shown as a small floating badge in the bottom-right corner of the window.

**Widget tree for pending indicator:**

```
container (bottom-right positioned via stack)
  └── container (pill badge)
      └── text("g...").size(TEXT_SM).style(text_muted)
```

The indicator displays the first chord's display string followed by `"..."`. It disappears when:
- The second chord resolves a command
- The second chord doesn't match (no-op)
- The timeout fires

### 3.8 Ctrl+K: Opening the Palette

The palette open trigger is a keyboard shortcut. It should be registered as a `CommandId` in the registry. However, the current `CommandId` enum does not have a variant for "open palette." Two options:

**Option A (recommended):** Add `CommandId::AppOpenPalette` to the enum, with default binding `CmdOrCtrl+K`. The dispatch function maps it to `Message::Palette(PaletteMessage::Open)`. This keeps the palette trigger discoverable and rebindable like every other command.

**Option B:** Hardcode `Ctrl+K` in the key handler before `BindingTable` resolution. This is simpler but breaks the "every action is a registered command" principle.

**This spec recommends Option A.** Required changes:
- Add `AppOpenPalette` to `CommandId` enum in `crates/cmdk/src/id.rs`
- Add to `ALL_IDS` and `TABLE`
- Register in `register_app()` with binding `KeyBinding::cmd_or_ctrl('k')` and `is_available: always`
- Map in `dispatch_command`: `CommandId::AppOpenPalette => Some(Message::Palette(PaletteMessage::Open))`

---

## 4. Path 3: Command-Backed UI Surfaces

### 4.1 Principle

Every button, toolbar item, and context menu entry that triggers an action should consume the registry for its metadata (label, availability, keybinding hint) and invoke via `CommandId`. This ensures:

- Labels are consistent between the palette and UI surfaces
- Availability logic is centralized (no duplicate `if has_selection` checks in view code)
- Keybinding hints are always accurate (derived from `BindingTable`, not hardcoded)
- Actions route through the same `ExecuteCommand` path

### 4.2 Registry Query API for UI Surfaces

UI surfaces don't need fuzzy search. They need direct lookups by `CommandId`:

```rust
// Already exists:
registry.get(CommandId::EmailArchive) -> Option<&CommandDescriptor>

// For availability + resolved label:
let ctx = build_context(app);
let desc = registry.get(CommandId::EmailArchive)?;
let label = desc.resolved_label(&ctx);
let available = (desc.is_available)(&ctx);
let keybinding = binding_table.display_binding(CommandId::EmailArchive);
```

### 4.3 Command Button Widget

**File:** `crates/app/src/ui/widgets.rs` (addition)

A helper that builds a button from a `CommandId`, pulling metadata from the registry:

```rust
/// Build a button for a registered command.
///
/// Pulls label, availability, and keybinding hint from the registry.
/// Disabled buttons are greyed out but visible. Emits ExecuteCommand
/// on click.
pub fn command_button<'a>(
    id: CommandId,
    registry: &CommandRegistry,
    binding_table: &BindingTable,
    ctx: &CommandContext,
) -> Element<'a, Message> {
    let desc = registry.get(id);
    let (label, available) = desc.map_or(("???", false), |d| {
        (d.resolved_label(ctx), (d.is_available)(ctx))
    });
    let keybinding = binding_table.display_binding(id);

    let label_text = text(label).size(TEXT_SM);
    let mut btn = button(label_text);

    if available {
        btn = btn.on_press(Message::ExecuteCommand(id));
    }

    // Optionally add keybinding hint as tooltip
    if let Some(kb) = keybinding {
        tooltip(btn, kb, tooltip::Position::Bottom).into()
    } else {
        btn.into()
    }
}
```

### 4.4 Toolbar Integration

The reading pane toolbar currently has placeholder buttons. With the command system:

```rust
// In reading_pane.rs view function:
let toolbar = row![
    command_button(CommandId::ComposeReply, &app.registry, &app.binding_table, &ctx),
    command_button(CommandId::ComposeReplyAll, &app.registry, &app.binding_table, &ctx),
    command_button(CommandId::ComposeForward, &app.registry, &app.binding_table, &ctx),
    Space::with_width(Length::Fill),
    command_button(CommandId::EmailArchive, &app.registry, &app.binding_table, &ctx),
    command_button(CommandId::EmailTrash, &app.registry, &app.binding_table, &ctx),
    command_button(CommandId::EmailStar, &app.registry, &app.binding_table, &ctx),
]
.spacing(SPACE_XS);
```

This replaces hardcoded label strings and manual availability checks. The toggle label resolution (Star/Unstar) happens automatically via `resolved_label()`.

### 4.5 Context Menu Integration (Future)

Context menus (right-click on thread, right-click on message) will be implemented as popover overlays (using the existing `crates/app/src/ui/popover.rs`). Each menu item is a `CommandId`:

```rust
let context_menu_commands = [
    CommandId::ComposeReply,
    CommandId::ComposeReplyAll,
    CommandId::ComposeForward,
    // separator
    CommandId::EmailArchive,
    CommandId::EmailTrash,
    CommandId::EmailStar,
    CommandId::EmailPin,
    CommandId::EmailMute,
    // separator
    CommandId::EmailMoveToFolder,
    CommandId::EmailAddLabel,
    CommandId::EmailRemoveLabel,
];
```

Each renders as a menu row with label (from registry), keybinding hint (from binding table), and availability (from context predicate). Parameterized commands in the context menu open the palette at stage 2 when clicked.

---

## 5. Phasing

Six independently shippable slices with a clear dependency graph. Each slice produces a working increment.

### Slice 6a: Infrastructure + Keyboard Dispatch

**Goal:** Every existing keyboard shortcut works via the command system. No visible UI changes except behavior.

**What's built:**
1. `CommandArgs` enum in `crates/cmdk/src/args.rs`
2. `command_dispatch.rs` — `build_context()`, `dispatch_command()`, `dispatch_parameterized()`
3. `CommandRegistry` and `BindingTable` initialization in `boot()`
4. `KeyEventMessage`, `iced_key_to_chord()`, `iced_named_to_cp()`
5. Keyboard event subscription in `App::subscription()`
6. `handle_key_event()` with `BindingTable` resolution
7. Pending chord state + timeout
8. New `Message` variants: `ExecuteCommand`, `KeyEvent`, `NavigateTo`, `EmailAction`, `Escape`
9. Handlers for `NavigateTo` and `EmailAction` in `update()` (delegate to existing sidebar/thread logic)
10. `AppOpenPalette` command ID (binding only — palette UI is next slice)

**Files changed:**
- `crates/cmdk/src/args.rs` (new)
- `crates/cmdk/src/lib.rs` (re-export `CommandArgs`)
- `crates/cmdk/src/id.rs` (add `AppOpenPalette`)
- `crates/cmdk/src/registry.rs` (register `AppOpenPalette`)
- `crates/app/Cargo.toml` (add `cmdk` dependency)
- `crates/app/src/command_dispatch.rs` (new)
- `crates/app/src/main.rs` (new fields, new `Message` variants, subscription, key handler)

**Depends on:** Slices 1-3 (done).

**Does not depend on:** Palette UI, resolver implementation, UI surface integration.

**Acceptance criteria:** Pressing `e` on a selected thread triggers archive. `g then i` navigates to inbox. `Ctrl+Shift+E` toggles sidebar. All 55 commands' keybindings work. Pending chord indicator shows `"g..."` after pressing `g`. Timeout clears it after 1 second.

### Slice 6b: Palette Overlay UI (Stage 1 — Command Search)

**Goal:** `Ctrl+K` opens a floating palette. User types, sees fuzzy-matched commands, selects one, and it executes. Parameterized commands are listed but cannot be executed yet (stage 2 is next slice).

**What's built:**
1. `PaletteState`, `PaletteStage::CommandSearch`, `PaletteMessage`, `PaletteEvent`
2. Palette view function with text input + scrollable results
3. Stack-based overlay rendering in `App::view()`
4. Backdrop mouse_area for click-outside-to-close
5. Focus management (auto-focus text input on open)
6. Keyboard navigation within palette (arrows, Enter, Escape)
7. Style constants and theme functions for palette
8. `QueryChanged` handler calling `registry.query()`
9. `Confirm` handler for `InputMode::Direct` commands — emits `ExecuteCommand`
10. `Confirm` for `InputMode::Parameterized` — shows "Not yet available" toast or placeholder

**Files changed:**
- `crates/app/src/ui/palette.rs` (new)
- `crates/app/src/ui/mod.rs` (add `pub mod palette;`)
- `crates/app/src/main.rs` (add `PaletteState` field, `Palette` message variant, overlay in `view()`, palette key routing)
- `crates/app/src/ui/layout.rs` (palette constants)
- `crates/app/src/ui/theme.rs` (palette styles)

**Depends on:** Slice 6a (keyboard dispatch, `ExecuteCommand` message).

**Acceptance criteria:** `Ctrl+K` opens palette. Typing "arch" shows "Archive" as top result. Arrow keys navigate. Enter on "Archive" archives the selected thread and closes the palette. Escape closes. Clicking backdrop closes. Category badges and keybinding hints visible. Unavailable commands shown greyed out.

### Slice 6c: Palette Stage 2 — Parameterized Commands

**Goal:** Selecting a parameterized command (Move to Folder, Add Label, Remove Label) transitions to stage 2 with a searchable option list. Snooze (DateTime) shows a basic date/time input.

**What's built:**
1. `AppInputResolver` implementation in `command_resolver.rs`
2. `PaletteStage::OptionPick` state and transitions
3. `OptionsLoaded` message and async resolver call
4. Option list rendering with label + path breadcrumbs
5. `build_args()` function — stage 2 selection → `CommandArgs`
6. `dispatch_parameterized()` wiring
7. `Db` methods: `get_user_folders_for_palette()`, `get_user_labels_for_palette()`, `get_thread_labels_for_palette()`
8. Cross-account option resolution (account name in path)
9. Snooze: basic DateTime input (preset times: "1 hour", "Tomorrow 9am", "Next week", or a manual date picker)

**Files changed:**
- `crates/app/src/command_resolver.rs` (new)
- `crates/app/src/ui/palette.rs` (stage 2 rendering, `OptionPick` handling)
- `crates/app/src/db.rs` (new palette query methods)
- `crates/app/src/main.rs` (resolver initialization, `ExecuteParameterized` handler)

**Depends on:** Slice 6b (palette UI).

**Acceptance criteria:** Selecting "Move to Folder" in the palette transitions to a folder list. Typing "proj" filters to folders containing "proj" in name or path. Selecting a folder executes the move (once the email action backend is wired). "Add Label" shows account labels. "Remove Label" shows only labels on the selected thread. Cross-account view shows account names in path breadcrumbs.

### Slice 6d: Command-Backed UI Surfaces

**Goal:** Toolbar buttons and future context menus consume the registry for metadata and invoke via `CommandId`.

**What's built:**
1. `command_button()` helper widget
2. Reading pane toolbar refactored to use `command_button()`
3. Thread list action buttons (if any) refactored
4. Sidebar action buttons refactored
5. `CommandContext` passed through component views (or components query it themselves)

**Files changed:**
- `crates/app/src/ui/widgets.rs` (command_button helper)
- `crates/app/src/ui/reading_pane.rs` (toolbar refactor)
- `crates/app/src/ui/thread_list.rs` (action buttons if applicable)
- `crates/app/src/ui/sidebar.rs` (action buttons if applicable)

**Depends on:** Slice 6a (dispatch infrastructure, `ExecuteCommand` message).

**Can be done in parallel with** Slices 6b and 6c.

**Acceptance criteria:** Reading pane toolbar buttons show labels from the registry. Star button shows "Unstar" when thread is starred. Buttons are greyed out when unavailable. Keybinding hints appear in tooltips. All button clicks route through `ExecuteCommand`.

### Slice 6e: Override Persistence

**Goal:** User keybinding overrides are saved to the settings database and restored on app launch.

**What's built:**
1. Serialization of `binding_table.overrides()` to a JSON string
2. New settings key in the app's settings DB for keybinding overrides
3. Load overrides in `boot()` via `binding_table.load_overrides()`
4. Save overrides on change (when a settings UI for keybindings is eventually built)
5. `UsageTracker` persistence — save/load usage counts to settings DB

**Files changed:**
- `crates/app/src/main.rs` (load overrides in boot, save on close)
- `crates/app/src/db.rs` (settings get/set for keybinding overrides and usage counts)

**Depends on:** Slice 6a.

**Acceptance criteria:** User overrides set via the `BindingTable` API persist across app restarts. Usage counts persist across restarts and influence palette ranking.

### Slice 6f: Keybinding Management UI

**Goal:** A settings panel where users can view, search, and rebind keyboard shortcuts.

**What's built:**
1. Keybinding settings panel (accessible from Settings)
2. Searchable list of all commands with current bindings
3. Click-to-rebind: captures next keypress as new binding
4. Conflict detection UI: "This key is already bound to X. Unbind X?"
5. Reset-to-default button per command
6. Reset-all button

**Files changed:**
- `crates/app/src/ui/settings.rs` (keybinding management section)
- `crates/app/src/main.rs` (new settings messages)

**Depends on:** Slice 6e (override persistence).

**This slice is lower priority** — the default bindings work out of the box, and power users can wait for rebinding UI. It can be deferred past V1 if needed.

### Dependency Graph

```
Slice 6a (keyboard dispatch + infrastructure)
  ├── Slice 6b (palette UI — stage 1)
  │     └── Slice 6c (palette — stage 2, parameterized)
  ├── Slice 6d (command-backed UI surfaces)  [parallel with 6b/6c]
  └── Slice 6e (override persistence)
        └── Slice 6f (keybinding management UI)
```

Slices 6b and 6d can be worked in parallel after 6a. Slice 6c depends on 6b. Slice 6f depends on 6e but is low priority.

---

## 6. Ecosystem Patterns

How patterns from the [iced ecosystem survey](../iced-ecosystem-survey.md) and [cross-reference](../iced-ecosystem-cross-reference.md) apply to this spec.

### Patterns Used

| Pattern | Source | Application in This Spec |
|---|---|---|
| Stack-based overlay | shadcn-rs `place_overlay_centered()` | Palette rendered via `iced::widget::stack` over main layout, with backdrop mouse_area for event blocking (per UI.md gotcha about stack not blocking events) |
| Stage routing | raffi `route_query()` | `PaletteStage` enum dispatch: stage 1 calls `CommandRegistry::query()`, stage 2 calls `search_options()` over resolver results |
| Component trait | trebuchet `(Task, ComponentEvent)` | Palette implements the existing `Component` trait, emitting `PaletteEvent::ExecuteCommand` and `Dismissed` |
| Subscription batching | rustcast `Subscription::batch()` | Keyboard subscription, pending chord timeout, and palette-internal subscriptions batched in `App::subscription()` |
| Raw keyboard interception | feu `subscription::events_with` | `iced::event::listen_with` captures `KeyPressed` before widget processing for global shortcut dispatch |
| Focus management | iced `widget::operation::focus(id)` | Palette text input auto-focused on open via `Task` returning `focus("palette-input")` |
| Generational tracking | bloom load_generation | Palette option loads tagged with command ID to discard stale results when user switches commands rapidly |
| Shadow editing | bloom config/editing_config | Keybinding override edits applied to `BindingTable` immediately but only persisted on explicit save |

### Gaps (Original to This Spec)

- **Two-chord pending indicator with timeout**: No surveyed project handles this. The pending chord state machine, `iced::time::every` timeout subscription, and transient `"g..."` indicator are entirely custom.
- **`CommandId` → `Message` dispatch map**: The central function mapping 55+ command IDs to iced Message variants has no analogue in surveyed projects. It's the bridge between the framework-agnostic registry and iced's Elm architecture.
- **Stage 2 typed execution payload (`CommandArgs`)**: The transition from `OptionItem.id` to a typed `CommandArgs` variant, matched exhaustively in dispatch, is original.
- **Widget-aware keyboard routing**: The interaction between `iced::event::Status::Captured` (widget already handled the event), palette-open state, and `BindingTable` resolution requires careful precedence logic not found in surveyed projects.

---

## Appendix A: File Inventory

New files:
- `crates/cmdk/src/args.rs` — `CommandArgs` enum
- `crates/app/src/command_dispatch.rs` — context assembly, dispatch map, key conversion
- `crates/app/src/command_resolver.rs` — `AppInputResolver` implementation
- `crates/app/src/ui/palette.rs` — palette state, messages, view

Modified files:
- `crates/cmdk/src/lib.rs` — re-export `CommandArgs`
- `crates/cmdk/src/id.rs` — add `AppOpenPalette`
- `crates/cmdk/src/registry.rs` — register `AppOpenPalette`
- `crates/app/Cargo.toml` — add `cmdk` dependency
- `crates/app/src/main.rs` — `App` fields, `Message` variants, `boot()`, `subscription()`, `update()`, `view()`
- `crates/app/src/ui/mod.rs` — add `pub mod palette;`
- `crates/app/src/ui/layout.rs` — palette sizing constants
- `crates/app/src/ui/theme.rs` — palette style functions
- `crates/app/src/ui/widgets.rs` — `command_button()` helper
- `crates/app/src/ui/reading_pane.rs` — toolbar refactor to command buttons
- `crates/app/src/db.rs` — palette query methods

## Appendix B: Message Enum After Integration

The complete `Message` enum after all slices, showing both existing and new variants:

```rust
#[derive(Debug, Clone)]
pub enum Message {
    // Existing component messages
    Sidebar(SidebarMessage),
    ThreadList(ThreadListMessage),
    ReadingPane(ReadingPaneMessage),
    Settings(SettingsMessage),

    // Existing data loading
    AccountsLoaded(u64, Result<Vec<db::Account>, String>),
    LabelsLoaded(u64, Result<Vec<Label>, String>),
    ThreadsLoaded(u64, Result<Vec<Thread>, String>),
    ThreadMessagesLoaded(u64, Result<Vec<db::ThreadMessage>, String>),
    ThreadAttachmentsLoaded(u64, Result<Vec<db::ThreadAttachment>, String>),

    // Existing UI
    Compose,
    Noop,
    ToggleSettings,
    AppearanceChanged(appearance::Mode),
    DividerDragStart(Divider),
    DividerDragMove(Point),
    DividerDragEnd,
    DividerHover(Divider),
    DividerUnhover,
    WindowResized(Size),
    WindowMoved(Point),
    ToggleRightSidebar,
    SetDateDisplay(db::DateDisplay),
    WindowCloseRequested(iced::window::Id),

    // New: Command system (Slice 6a)
    KeyEvent(KeyEventMessage),
    ExecuteCommand(CommandId),
    ExecuteParameterized(CommandId, CommandArgs),
    NavigateTo(NavigationTarget),
    Escape,
    EmailAction(EmailAction),
    ComposeAction(ComposeAction),
    TaskAction(TaskAction),
    SetTheme(String),
    ToggleSidebar,
    FocusSearch,
    ShowHelp,
    SyncCurrentFolder,
    SetReadingPanePosition(ReadingPanePosition),

    // New: Palette (Slice 6b)
    Palette(PaletteMessage),
}
```
