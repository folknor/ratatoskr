use ratatoskr_command_palette::{CommandArgs, CommandContext, CommandId, ViewType};
use ratatoskr_core::scope::ViewScope;

use crate::App;
use crate::Message;

// ── Supporting enums ────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavigationTarget {
    Inbox,
    Starred,
    Sent,
    Drafts,
    Snoozed,
    Trash,
    Spam,
    AllMail,
    Primary,
    Updates,
    Promotions,
    Social,
    Newsletters,
    Tasks,
    Attachments,
    SmartFolder { id: String },
    Label { label_id: String, account_id: String },
    Search { query: String },
    PinnedSearch { id: i64 },
    Chat { email: String },
}

impl NavigationTarget {
    /// Map a `NavigationTarget` to the sidebar `selected_label` string.
    ///
    /// Universal folders map to well-known IDs (INBOX, STARRED, etc.).
    /// Smart folders use their ID directly.
    /// Labels use `label_id`.
    /// Search and PinnedSearch return `None` (they don't select a sidebar label).
    pub fn to_label_id(&self) -> Option<String> {
        match self {
            Self::Inbox => Some("INBOX".to_string()),
            Self::Starred => Some("STARRED".to_string()),
            Self::Sent => Some("SENT".to_string()),
            Self::Drafts => Some("DRAFT".to_string()),
            Self::Snoozed => Some("SNOOZED".to_string()),
            Self::Trash => Some("TRASH".to_string()),
            Self::Spam => Some("SPAM".to_string()),
            Self::AllMail => Some("ALL_MAIL".to_string()),
            Self::Primary => Some("CATEGORY_PRIMARY".to_string()),
            Self::Updates => Some("CATEGORY_UPDATES".to_string()),
            Self::Promotions => Some("CATEGORY_PROMOTIONS".to_string()),
            Self::Social => Some("CATEGORY_SOCIAL".to_string()),
            Self::Newsletters => Some("CATEGORY_NEWSLETTERS".to_string()),
            Self::Tasks => Some("TASKS".to_string()),
            Self::Attachments => Some("ATTACHMENTS".to_string()),
            Self::SmartFolder { id } => Some(id.clone()),
            Self::Label { label_id, .. } => Some(label_id.clone()),
            Self::Search { .. } | Self::PinnedSearch { .. } | Self::Chat { .. } => None,
        }
    }
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
    MoveToFolder { folder_id: ratatoskr_core::actions::FolderId },
    AddLabel { label_id: ratatoskr_core::actions::TagId },
    RemoveLabel { label_id: ratatoskr_core::actions::TagId },
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

// PaletteMessage is defined in `ui::palette` as part of the Palette component.

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

    let search_query = if app.search_query.text().trim().is_empty() {
        None
    } else {
        Some(app.search_query.text().to_string())
    };

    let (may_remove_items, may_set_seen, may_set_keywords, may_submit) =
        current_mailbox_rights(app);

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
        composer_is_open: app.composer_is_open(),
        focused_region: app.focused_region,
        search_query,
        may_remove_items,
        may_set_seen,
        may_set_keywords,
        may_submit,
    }
}

/// Extract mailbox rights from the currently selected navigation folder.
///
/// Returns `(may_remove_items, may_set_seen, may_set_keywords, may_submit)`.
/// All `None` when the folder has no rights data (provider doesn't report ACL,
/// or we're in a universal/smart folder view).
fn current_mailbox_rights(
    app: &App,
) -> (Option<bool>, Option<bool>, Option<bool>, Option<bool>) {
    let rights = app
        .sidebar
        .selected_label
        .as_deref()
        .and_then(|label_id| {
            app.sidebar.nav_state.as_ref()?.folders.iter().find_map(|f| {
                if f.id == label_id {
                    f.rights.as_ref()
                } else {
                    None
                }
            })
        });

    match rights {
        Some(r) => (r.may_remove_items, r.may_set_seen, r.may_set_keywords, r.may_submit),
        None => (None, None, None, None),
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
    if app.sidebar.active_pinned_search.is_some() {
        return (ViewType::PinnedSearch, None);
    }

    // Active search (thread list in search mode)
    if app.thread_list.mode == crate::ui::thread_list::ThreadListMode::Search {
        return (ViewType::Search, None);
    }

    // Derive from navigation target if set
    if let Some(target) = &app.navigation_target {
        return view_type_from_target(app, target);
    }

    match &app.sidebar.selected_label {
        Some(label_id) => view_type_from_label(app, label_id),
        None => (ViewType::Inbox, None),
    }
}

/// Map a `NavigationTarget` to `ViewType` and optional label ID.
fn view_type_from_target(
    _app: &App,
    target: &NavigationTarget,
) -> (ViewType, Option<String>) {
    match target {
        NavigationTarget::Inbox => (ViewType::Inbox, None),
        NavigationTarget::Starred => (ViewType::Starred, None),
        NavigationTarget::Sent => (ViewType::Sent, None),
        NavigationTarget::Drafts => (ViewType::Drafts, None),
        NavigationTarget::Snoozed => (ViewType::Snoozed, None),
        NavigationTarget::Trash => (ViewType::Trash, None),
        NavigationTarget::Spam => (ViewType::Spam, None),
        NavigationTarget::AllMail => (ViewType::AllMail, None),
        NavigationTarget::Primary
        | NavigationTarget::Updates
        | NavigationTarget::Promotions
        | NavigationTarget::Social
        | NavigationTarget::Newsletters => (ViewType::Category, None),
        NavigationTarget::Tasks => (ViewType::Tasks, None),
        NavigationTarget::Attachments => (ViewType::Attachments, None),
        NavigationTarget::SmartFolder { id } => {
            (ViewType::SmartFolder, Some(id.clone()))
        }
        NavigationTarget::Label { label_id, .. } => {
            (ViewType::Label, Some(label_id.clone()))
        }
        NavigationTarget::Search { .. } => (ViewType::Search, None),
        NavigationTarget::PinnedSearch { .. } => (ViewType::PinnedSearch, None),
        NavigationTarget::Chat { .. } => (ViewType::Chat, None),
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
    // 1. Derive account from the current view scope.
    let scope_account: Option<&str> = match &app.sidebar.selected_scope {
        ViewScope::Account(id) => Some(id.as_str()),
        ViewScope::SharedMailbox { account_id, .. }
        | ViewScope::PublicFolder { account_id, .. } => Some(account_id.as_str()),
        ViewScope::AllAccounts => None,
    };
    if let Some(aid) = scope_account {
        if let Some(account) = app.sidebar.accounts.iter().find(|a| a.id == aid) {
            let pk = provider_str_to_kind(&account.provider);
            return (Some(account.id.clone()), pk);
        }
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
/// `is_muted` and `is_pinned` are read from the app-layer `Thread` struct.
/// `in_trash`, `in_spam`, and `is_draft` are derived from the current
/// navigation context (sidebar label or `NavigationTarget`).
fn selected_thread_state(app: &App) -> ThreadState {
    let thread = app
        .thread_list
        .selected_thread
        .and_then(|idx| app.thread_list.threads.get(idx));

    match thread {
        Some(t) => {
            // Derive trash/spam/draft from navigation target if available,
            // otherwise fall back to sidebar selected_label.
            let (in_trash, in_spam, is_draft) =
                if let Some(target) = &app.navigation_target {
                    (
                        Some(matches!(target, NavigationTarget::Trash)),
                        Some(matches!(target, NavigationTarget::Spam)),
                        Some(matches!(target, NavigationTarget::Drafts)),
                    )
                } else {
                    let current_label = app.sidebar.selected_label.as_deref();
                    (
                        Some(current_label == Some("TRASH")),
                        Some(current_label == Some("SPAM")),
                        Some(current_label == Some("DRAFT")),
                    )
                };

            ThreadState {
                is_read: Some(t.is_read),
                is_starred: Some(t.is_starred),
                is_muted: Some(t.is_muted),
                is_pinned: Some(t.is_pinned),
                is_draft,
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
/// Returns `None` for parameterized commands (handled by
/// `dispatch_parameterized`) and for `AppAskAi` (not yet implemented).
///
/// **No wildcard catch-all.** Every `CommandId` variant has an explicit arm
/// so that adding a new variant without wiring dispatch is a compiler error.
pub fn dispatch_command(id: CommandId, app: &App) -> Option<Message> {
    match id {
        // Navigation — direct
        CommandId::NavNext => dispatch_nav_next(app),
        CommandId::NavPrev => dispatch_nav_prev(app),
        CommandId::NavOpen => dispatch_nav_open(app),
        CommandId::NavMsgNext => Some(Message::ReadingPane(
            crate::ui::reading_pane::ReadingPaneMessage::NextMessage,
        )),
        CommandId::NavMsgPrev => Some(Message::ReadingPane(
            crate::ui::reading_pane::ReadingPaneMessage::PrevMessage,
        )),

        // Navigation — folder/view targets
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
        CommandId::EmailSelectAll => Some(Message::ThreadList(
            crate::ui::thread_list::ThreadListMessage::SelectAll,
        )),
        CommandId::EmailSelectFromHere => Some(Message::ThreadList(
            crate::ui::thread_list::ThreadListMessage::SelectFromHere,
        )),

        // Parameterized — handled by dispatch_parameterized, not here
        CommandId::EmailMoveToFolder
        | CommandId::EmailAddLabel
        | CommandId::EmailRemoveLabel
        | CommandId::EmailSnooze
        | CommandId::NavigateToLabel
        | CommandId::SmartFolderSave => None,

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
        CommandId::ViewReadingPaneRight => Some(Message::SetReadingPanePosition(ReadingPanePosition::Right)),
        CommandId::ViewReadingPaneBottom => Some(Message::SetReadingPanePosition(ReadingPanePosition::Bottom)),
        CommandId::ViewReadingPaneHidden => Some(Message::SetReadingPanePosition(ReadingPanePosition::Hidden)),

        // Calendar
        CommandId::CalendarToggle => Some(Message::ToggleAppMode),
        CommandId::SwitchToCalendar => Some(Message::SetAppMode(crate::AppMode::Calendar)),
        CommandId::SwitchToMail => Some(Message::SetAppMode(crate::AppMode::Mail)),
        CommandId::CalendarViewDay => Some(Message::SetCalendarView(crate::ui::calendar::CalendarView::Day)),
        CommandId::CalendarViewWorkWeek => Some(Message::SetCalendarView(crate::ui::calendar::CalendarView::WorkWeek)),
        CommandId::CalendarViewWeek => Some(Message::SetCalendarView(crate::ui::calendar::CalendarView::Week)),
        CommandId::CalendarViewMonth => Some(Message::SetCalendarView(crate::ui::calendar::CalendarView::Month)),
        CommandId::CalendarToday => Some(Message::CalendarToday),
        CommandId::CalendarCreateEvent => Some(Message::Calendar(Box::new(crate::ui::calendar::CalendarMessage::CreateEvent))),
        CommandId::CalendarPopOut => Some(Message::Calendar(Box::new(crate::ui::calendar::CalendarMessage::PopOutCalendar))),

        // App
        CommandId::AppSearch => Some(Message::FocusSearch),
        CommandId::AppAskAi => None, // not yet implemented
        CommandId::AppHelp => Some(Message::ShowHelp),
        CommandId::AppSyncFolder => Some(Message::SyncCurrentFolder),
        CommandId::AppOpenPalette => Some(Message::Palette(
            crate::ui::palette::PaletteMessage::Open(
                ratatoskr_command_palette::CommandContext::default(),
            ),
        )),

        // Undo
        CommandId::Undo => Some(Message::Undo),
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
            folder_id: ratatoskr_core::actions::FolderId::from(folder_id),
        })),
        (CommandId::EmailAddLabel, CommandArgs::AddLabel { label_id }) => {
            Some(Message::EmailAction(EmailAction::AddLabel {
                label_id: ratatoskr_core::actions::TagId::from(label_id),
            }))
        }
        (
            CommandId::EmailRemoveLabel,
            CommandArgs::RemoveLabel { label_id },
        ) => {
            Some(Message::EmailAction(EmailAction::RemoveLabel {
                label_id: ratatoskr_core::actions::TagId::from(label_id),
            }))
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
        (
            CommandId::SmartFolderSave,
            CommandArgs::SmartFolderSave { name },
        ) => Some(Message::SaveAsSmartFolder(name)),
        (other_id, other_args) => {
            log::warn!("unhandled parameterized dispatch: {other_id:?} with {other_args:?}");
            None
        }
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
