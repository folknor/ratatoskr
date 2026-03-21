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

/// Derive `ViewType` and optional label ID from app state.
///
/// Uses explicit matching on sidebar `selected_label` IDs (which correspond
/// to `NavigationFolder.id` values like "INBOX", "STARRED", etc.) and checks
/// search/pinned search state for `Search`/`PinnedSearch` views.
fn current_view_and_label(app: &App) -> (ViewType, Option<String>) {
    if app.show_settings {
        return (ViewType::Settings, None);
    }

    // Calendar mode
    if app.app_mode == crate::AppMode::Calendar {
        return (ViewType::Calendar, None);
    }

    // Active pinned search
    if app.active_pinned_search.is_some() {
        return (ViewType::PinnedSearch, None);
    }

    // Active search (thread list in search mode)
    if app.thread_list.mode == crate::ui::thread_list::ThreadListMode::Search {
        return (ViewType::Search, None);
    }

    match &app.sidebar.selected_label {
        Some(label_id) => view_type_from_label(app, label_id),
        None => (ViewType::Inbox, None),
    }
}

/// Map a sidebar label ID to the appropriate `ViewType`.
///
/// Checks well-known universal folder IDs first, then consults the
/// navigation state to distinguish SmartFolders from account labels.
fn view_type_from_label(app: &App, label_id: &str) -> (ViewType, Option<String>) {
    match label_id {
        "INBOX" => (ViewType::Inbox, None),
        "STARRED" => (ViewType::Starred, None),
        "SENT" => (ViewType::Sent, None),
        "DRAFT" => (ViewType::Drafts, None),
        "SNOOZED" => (ViewType::Snoozed, None),
        "TRASH" => (ViewType::Trash, None),
        "SPAM" => (ViewType::Spam, None),
        "ALL_MAIL" => (ViewType::AllMail, None),
        other => {
            // Check nav_state to see if this is a SmartFolder
            let is_smart = app
                .sidebar
                .nav_state
                .as_ref()
                .and_then(|ns| ns.folders.iter().find(|f| f.id == other))
                .is_some_and(|f| {
                    matches!(
                        f.folder_kind,
                        ratatoskr_core::db::queries_extra::navigation::FolderKind::SmartFolder
                    )
                });
            if is_smart {
                (ViewType::SmartFolder, Some(other.to_string()))
            } else {
                (ViewType::Label, Some(other.to_string()))
            }
        }
    }
}

/// Resolve the active account ID and provider kind from sidebar state.
///
/// When scoped to a single account, also resolves `ProviderKind` from
/// the account's `provider` field so availability predicates work.
fn active_account_info(
    app: &App,
) -> (Option<String>, Option<ratatoskr_command_palette::ProviderKind>) {
    // 1. If sidebar is scoped to a single account, use that.
    if let Some(account) = app
        .sidebar
        .selected_account
        .and_then(|idx| app.sidebar.accounts.get(idx))
    {
        let pk = provider_str_to_kind(&account.provider);
        return (Some(account.id.clone()), pk);
    }
    // 2. If in unified view but a thread is selected, derive account
    //    from the selected thread. Look up provider from account list.
    if let Some(thread) = app
        .thread_list
        .selected_thread
        .and_then(|idx| app.thread_list.threads.get(idx))
    {
        let pk = app
            .sidebar
            .accounts
            .iter()
            .find(|a| a.id == thread.account_id)
            .map(|a| provider_str_to_kind(&a.provider))
            .unwrap_or(None);
        return (Some(thread.account_id.clone()), pk);
    }
    (None, None)
}

/// Map a provider string from the DB to a `ProviderKind` enum.
fn provider_str_to_kind(
    provider: &str,
) -> Option<ratatoskr_command_palette::ProviderKind> {
    match provider {
        "gmail_api" => Some(ratatoskr_command_palette::ProviderKind::Gmail),
        "jmap" => Some(ratatoskr_command_palette::ProviderKind::Jmap),
        "graph" => Some(ratatoskr_command_palette::ProviderKind::Graph),
        "imap" => Some(ratatoskr_command_palette::ProviderKind::Imap),
        _ => None,
    }
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

/// Populate thread state flags from the selected thread and current view.
///
/// The `Thread` struct currently only exposes `is_read` and `is_starred`.
/// `is_muted`, `is_pinned`, and `is_draft` are not on the app-layer Thread
/// type yet. We derive `in_trash` and `in_spam` from the current view
/// context (if viewing Trash/Spam, the selected thread is in that folder).
fn selected_thread_state(app: &App) -> ThreadState {
    let thread = app
        .thread_list
        .selected_thread
        .and_then(|idx| app.thread_list.threads.get(idx));

    match thread {
        Some(t) => {
            // Derive trash/spam from current view context
            let current_label = app.sidebar.selected_label.as_deref();
            let in_trash = Some(current_label == Some("TRASH"));
            let in_spam = Some(current_label == Some("SPAM"));

            ThreadState {
                is_read: Some(t.is_read),
                is_starred: Some(t.is_starred),
                // These fields are not yet on the app Thread type.
                // They will be populated once the Thread struct is extended.
                is_muted: None,
                is_pinned: None,
                is_draft: Some(current_label == Some("DRAFT")),
                in_trash,
                in_spam,
            }
        }
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
pub fn dispatch_command(id: CommandId, app: &App) -> Option<Message> {
    match id {
<<<<<<< HEAD
        // Navigation
        CommandId::NavNext => dispatch_nav_next(app),
        CommandId::NavPrev => dispatch_nav_prev(app),
        CommandId::NavOpen => dispatch_nav_open(app),
        CommandId::NavMsgNext | CommandId::NavMsgPrev => None,
        CommandId::NavGoInbox
        | CommandId::NavGoStarred
        | CommandId::NavGoSent
        | CommandId::NavGoDrafts
        | CommandId::NavGoSnoozed
        | CommandId::NavGoTrash
        | CommandId::NavGoAllMail
        | CommandId::NavGoPrimary
        | CommandId::NavGoUpdates
        | CommandId::NavGoPromotions
        | CommandId::NavGoSocial
        | CommandId::NavGoNewsletters
        | CommandId::NavGoTasks
        | CommandId::NavGoAttachments
        | CommandId::NavEscape => dispatch_navigation(id),

        // Email
        CommandId::EmailArchive
        | CommandId::EmailTrash
        | CommandId::EmailPermanentDelete
        | CommandId::EmailSpam
        | CommandId::EmailMarkRead
        | CommandId::EmailStar
        | CommandId::EmailPin
        | CommandId::EmailMute
        | CommandId::EmailUnsubscribe
        | CommandId::EmailSelectAll
        | CommandId::EmailSelectFromHere => dispatch_email(id),

        // Parameterized — stage 2
        CommandId::EmailMoveToFolder
        | CommandId::EmailAddLabel
        | CommandId::EmailRemoveLabel
        | CommandId::EmailSnooze
        | CommandId::NavigateToLabel => None,

        // Compose / Tasks / View / Calendar / App
        _ => dispatch_other(id),
    }
}

fn dispatch_navigation(id: CommandId) -> Option<Message> {
    match id {
=======
        // Navigation — thread list keyboard nav (j/k/Enter/Escape)
        CommandId::NavNext => {
            Some(Message::ThreadList(crate::ui::thread_list::ThreadListMessage::SelectNext))
        }
        CommandId::NavPrev => {
            Some(Message::ThreadList(crate::ui::thread_list::ThreadListMessage::SelectPrevious))
        }
        CommandId::NavOpen => {
            Some(Message::ThreadList(crate::ui::thread_list::ThreadListMessage::ActivateSelected))
        }
        CommandId::NavMsgNext => None,
        CommandId::NavMsgPrev => None,
>>>>>>> worktree-agent-a35d2dcb
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
        _ => None,
    }
}

fn dispatch_email(id: CommandId) -> Option<Message> {
    match id {
        CommandId::EmailArchive => Some(Message::EmailAction(EmailAction::Archive)),
        CommandId::EmailTrash => Some(Message::EmailAction(EmailAction::Trash)),
        CommandId::EmailPermanentDelete => Some(Message::EmailAction(EmailAction::PermanentDelete)),
        CommandId::EmailSpam => Some(Message::EmailAction(EmailAction::ToggleSpam)),
        CommandId::EmailMarkRead => Some(Message::EmailAction(EmailAction::ToggleRead)),
        CommandId::EmailStar => Some(Message::EmailAction(EmailAction::ToggleStar)),
        CommandId::EmailPin => Some(Message::EmailAction(EmailAction::TogglePin)),
        CommandId::EmailMute => Some(Message::EmailAction(EmailAction::ToggleMute)),
        CommandId::EmailUnsubscribe => Some(Message::EmailAction(EmailAction::Unsubscribe)),
        // Stubbed — ThreadListMessage::SelectAll / SelectFromHere don't exist yet
        CommandId::EmailSelectAll | CommandId::EmailSelectFromHere => None,
        _ => None,
    }
}

fn dispatch_other(id: CommandId) -> Option<Message> {
    match id {
        CommandId::ComposeNew => Some(Message::Compose),
        CommandId::ComposeReply => Some(Message::ComposeAction(ComposeAction::Reply)),
        CommandId::ComposeReplyAll => Some(Message::ComposeAction(ComposeAction::ReplyAll)),
        CommandId::ComposeForward => Some(Message::ComposeAction(ComposeAction::Forward)),
        CommandId::TaskCreate => Some(Message::TaskAction(TaskAction::Create)),
        CommandId::TaskCreateFromEmail => Some(Message::TaskAction(TaskAction::CreateFromEmail)),
        CommandId::TaskTogglePanel => Some(Message::TaskAction(TaskAction::TogglePanel)),
        CommandId::TaskViewAll => Some(Message::NavigateTo(NavigationTarget::Tasks)),
        CommandId::ViewToggleSidebar => Some(Message::ToggleSidebar),
        CommandId::ViewSetThemeLight => Some(Message::SetTheme("Light".to_string())),
        CommandId::ViewSetThemeDark => Some(Message::SetTheme("Dark".to_string())),
        CommandId::ViewSetThemeSystem => Some(Message::SetTheme("System".to_string())),
        CommandId::ViewToggleTaskPanel => Some(Message::TaskAction(TaskAction::TogglePanel)),
        CommandId::ViewReadingPaneRight => Some(Message::SetReadingPanePosition(ReadingPanePosition::Right)),
        CommandId::ViewReadingPaneBottom => Some(Message::SetReadingPanePosition(ReadingPanePosition::Bottom)),
        CommandId::ViewReadingPaneHidden => Some(Message::SetReadingPanePosition(ReadingPanePosition::Hidden)),
        CommandId::CalendarToggle => Some(Message::ToggleAppMode),
        CommandId::CalendarViewDay => Some(Message::SetCalendarView(crate::ui::calendar::CalendarView::Day)),
        CommandId::CalendarViewWorkWeek => Some(Message::SetCalendarView(crate::ui::calendar::CalendarView::WorkWeek)),
        CommandId::CalendarViewWeek => Some(Message::SetCalendarView(crate::ui::calendar::CalendarView::Week)),
        CommandId::CalendarViewMonth => Some(Message::SetCalendarView(crate::ui::calendar::CalendarView::Month)),
        CommandId::CalendarToday => Some(Message::CalendarToday),
        CommandId::CalendarCreateEvent => Some(Message::Calendar(crate::ui::calendar::CalendarMessage::CreateEvent)),
        CommandId::AppSearch => Some(Message::FocusSearch),
        CommandId::AppAskAi => None,
        CommandId::AppHelp => Some(Message::ShowHelp),
        CommandId::AppSyncFolder => Some(Message::SyncCurrentFolder),
        CommandId::AppOpenPalette => Some(Message::Palette(PaletteMessage::Open)),
        _ => None,
    }
}

/// NavNext: select the next thread in the list.
fn dispatch_nav_next(app: &App) -> Option<Message> {
    let current = app.thread_list.selected_thread.unwrap_or(0);
    let next = current.saturating_add(1);
    if next < app.thread_list.threads.len() {
        Some(Message::ThreadList(
            crate::ui::thread_list::ThreadListMessage::SelectThread(next),
        ))
    } else {
        None
    }
}

/// NavPrev: select the previous thread in the list.
fn dispatch_nav_prev(app: &App) -> Option<Message> {
    let current = app.thread_list.selected_thread?;
    if current > 0 {
        Some(Message::ThreadList(
            crate::ui::thread_list::ThreadListMessage::SelectThread(
                current.saturating_sub(1),
            ),
        ))
    } else {
        None
    }
}

/// NavOpen: open the currently selected thread.
///
/// Uses SelectThread to ensure the thread is selected (triggering detail load).
fn dispatch_nav_open(app: &App) -> Option<Message> {
    let idx = app.thread_list.selected_thread?;
    if idx < app.thread_list.threads.len() {
        Some(Message::ThreadList(
            crate::ui::thread_list::ThreadListMessage::SelectThread(idx),
        ))
    } else {
        None
    }
}

/// Map a parameterized command + resolved args to an iced Message.
pub fn dispatch_parameterized(
    id: CommandId,
    args: CommandArgs,
) -> Option<Message> {
    match (id, args) {
        (
            CommandId::EmailMoveToFolder,
            CommandArgs::MoveToFolder { folder_id },
        ) => Some(Message::EmailAction(EmailAction::MoveToFolder {
            folder_id,
        })),
        (CommandId::EmailAddLabel, CommandArgs::AddLabel { label_id }) => {
            Some(Message::EmailAction(EmailAction::AddLabel { label_id }))
        }
        (
            CommandId::EmailRemoveLabel,
            CommandArgs::RemoveLabel { label_id },
        ) => {
            Some(Message::EmailAction(EmailAction::RemoveLabel { label_id }))
        }
        (CommandId::EmailSnooze, CommandArgs::Snooze { until }) => {
            Some(Message::EmailAction(EmailAction::Snooze { until }))
        }
        (
            CommandId::NavigateToLabel,
            CommandArgs::NavigateToLabel {
                label_id,
                account_id,
            },
        ) => Some(Message::NavigateTo(NavigationTarget::Label {
            label_id,
            account_id,
        })),
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
