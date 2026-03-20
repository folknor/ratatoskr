use ratatoskr_command_palette::{CommandArgs, CommandContext, CommandId, OptionItem, ViewType};

use crate::App;
use crate::Message;

// ── Supporting enums ────────────────────────────────────

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

#[derive(Debug, Clone)]
pub enum PaletteMessage {
    /// Open the palette (triggered by Ctrl+K or palette button).
    Open,
    /// Close the palette (Escape, click outside, or command executed).
    Close,
    /// Text input changed in the search field.
    QueryChanged(String),
    /// Arrow down: select next result.
    SelectNext,
    /// Arrow up: select previous result.
    SelectPrev,
    /// Enter pressed: execute the currently selected command.
    Confirm,
    /// Mouse click on a result row.
    ClickResult(usize),
    /// Stage 2: option list loaded from resolver.
    /// The `u64` is the generation counter to discard stale results.
    OptionsLoaded(u64, CommandId, Result<Vec<OptionItem>, String>),
    /// Mouse click on a stage 2 option row.
    ClickOption(usize),
}

#[derive(Debug, Clone)]
pub enum KeyEventMessage {
    KeyPressed {
        key: iced::keyboard::Key,
        modifiers: iced::keyboard::Modifiers,
        status: iced::event::Status,
        window_id: iced::window::Id,
    },
    PendingChordTimeout,
}

// ── Context assembly ────────────────────────────────────

/// Snapshot the app model into a `CommandContext` for registry queries.
///
/// Called frequently (every key event). Must not perform DB access or block.
pub fn build_context(app: &App) -> CommandContext {
    let selected_thread_ids = selected_thread_ids(app);
    let active_message_id = None; // Reading pane doesn't expose this yet
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

fn selected_thread_ids(app: &App) -> Vec<String> {
    app.thread_list
        .selected_thread
        .and_then(|idx| app.thread_list.threads.get(idx))
        .map(|t| vec![t.id.clone()])
        .unwrap_or_default()
}

fn current_view_and_label(app: &App) -> (ViewType, Option<String>) {
    if app.show_settings {
        return (ViewType::Settings, None);
    }
    match &app.sidebar.selected_label {
        Some(label_id) => (ViewType::Label, Some(label_id.clone())),
        None => (ViewType::Inbox, None),
    }
}

fn active_account_info(app: &App) -> (Option<String>, Option<ratatoskr_command_palette::ProviderKind>) {
    // 1. If sidebar is scoped to a single account, use that.
    if let Some(account) = app
        .sidebar
        .selected_account
        .and_then(|idx| app.sidebar.accounts.get(idx))
    {
        return (Some(account.id.clone()), None);
    }
    // 2. If in unified view but a thread is selected, derive account
    //    from the selected thread. This is critical for parameterized
    //    commands (Move to Folder, Add/Remove Label) which need an
    //    account to resolve options in stage 2.
    if let Some(thread) = app
        .thread_list
        .selected_thread
        .and_then(|idx| app.thread_list.threads.get(idx))
    {
        return (Some(thread.account_id.clone()), None);
    }
    (None, None)
}

struct ThreadState {
    is_read: Option<bool>,
    is_starred: Option<bool>,
    is_muted: Option<bool>,
    is_pinned: Option<bool>,
    is_draft: Option<bool>,
    in_trash: Option<bool>,
    in_spam: Option<bool>,
}

fn selected_thread_state(app: &App) -> ThreadState {
    let thread = app
        .thread_list
        .selected_thread
        .and_then(|idx| app.thread_list.threads.get(idx));

    match thread {
        Some(t) => ThreadState {
            is_read: Some(t.is_read),
            is_starred: Some(t.is_starred),
            is_muted: None,
            is_pinned: None,
            is_draft: None,
            in_trash: None,
            in_spam: None,
        },
        None => ThreadState {
            is_read: None,
            is_starred: None,
            is_muted: None,
            is_pinned: None,
            is_draft: None,
            in_trash: None,
            in_spam: None,
        },
    }
}

// ── Command dispatch ────────────────────────────────────

/// Map a direct (non-parameterized) command to an iced Message.
///
/// Returns `None` for commands that are not yet implemented,
/// allowing incremental rollout.
pub fn dispatch_command(id: CommandId, _app: &App) -> Option<Message> {
    match id {
        // Navigation
        CommandId::NavNext => Some(Message::NavigateTo(NavigationTarget::Inbox)), // stub
        CommandId::NavPrev => Some(Message::NavigateTo(NavigationTarget::Inbox)), // stub
        CommandId::NavOpen => None,
        CommandId::NavMsgNext => None,
        CommandId::NavMsgPrev => None,
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
        CommandId::EmailPermanentDelete => {
            Some(Message::EmailAction(EmailAction::PermanentDelete))
        }
        CommandId::EmailSpam => Some(Message::EmailAction(EmailAction::ToggleSpam)),
        CommandId::EmailMarkRead => Some(Message::EmailAction(EmailAction::ToggleRead)),
        CommandId::EmailStar => Some(Message::EmailAction(EmailAction::ToggleStar)),
        CommandId::EmailPin => Some(Message::EmailAction(EmailAction::TogglePin)),
        CommandId::EmailMute => Some(Message::EmailAction(EmailAction::ToggleMute)),
        CommandId::EmailUnsubscribe => Some(Message::EmailAction(EmailAction::Unsubscribe)),
        CommandId::EmailSelectAll => None,
        CommandId::EmailSelectFromHere => None,

        // Parameterized -- these open the palette's stage 2
        CommandId::EmailMoveToFolder
        | CommandId::EmailAddLabel
        | CommandId::EmailRemoveLabel
        | CommandId::EmailSnooze => None,

        // Compose
        CommandId::ComposeNew => Some(Message::Compose),
        CommandId::ComposeReply => Some(Message::ComposeAction(ComposeAction::Reply)),
        CommandId::ComposeReplyAll => Some(Message::ComposeAction(ComposeAction::ReplyAll)),
        CommandId::ComposeForward => Some(Message::ComposeAction(ComposeAction::Forward)),

        // Tasks
        CommandId::TaskCreate => Some(Message::TaskAction(TaskAction::Create)),
        CommandId::TaskCreateFromEmail => {
            Some(Message::TaskAction(TaskAction::CreateFromEmail))
        }
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
        CommandId::AppAskAi => None,
        CommandId::AppHelp => Some(Message::ShowHelp),
        CommandId::AppSyncFolder => Some(Message::SyncCurrentFolder),
        CommandId::AppOpenPalette => Some(Message::Palette(PaletteMessage::Open)),
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
        _ => None,
    }
}

// ── iced key conversion ─────────────────────────────────

use ratatoskr_command_palette::{Chord, Key, Modifiers, NamedKey};

/// Convert iced keyboard types to command-palette Chord.
///
/// Returns `None` for keys we don't handle (modifiers alone, etc.)
pub fn iced_key_to_chord(
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

/// Map iced named keys to command-palette NamedKey.
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

/// Whether the modifiers include Ctrl or Cmd.
pub fn has_command_modifier(modifiers: &iced::keyboard::Modifiers) -> bool {
    modifiers.command() || modifiers.control()
}
