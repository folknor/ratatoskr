use cmdk::{CommandArgs, CommandContext, CommandId, ViewType};
use rtsk::scope::ViewScope;
use types::SidebarSelection;

use crate::App;
use crate::Message;

// ── Supporting enums ────────────────────────────────────

/// Navigation targets — sidebar-backed destinations wrap `SidebarSelection`,
/// non-sidebar destinations (search, chat) have their own variants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavigationTarget {
    /// Navigate to a sidebar-backed destination.
    Sidebar {
        selection: SidebarSelection,
        /// Account to scope to (for cross-account navigation from the palette).
        account_id: Option<String>,
    },
    Search {
        query: String,
    },
    PinnedSearch {
        id: i64,
    },
    Chat {
        email: String,
    },
}

/// Derive `ViewType` and optional label ID from a `SidebarSelection`.
pub fn selection_to_view_type(sel: &SidebarSelection) -> (ViewType, Option<String>) {
    match sel {
        SidebarSelection::Inbox => (ViewType::Inbox, None),
        SidebarSelection::Folder(f) => {
            use types::SystemFolder;
            match f {
                SystemFolder::Starred => (ViewType::Starred, None),
                SystemFolder::Sent => (ViewType::Sent, None),
                SystemFolder::Draft => (ViewType::Drafts, None),
                SystemFolder::Snoozed => (ViewType::Snoozed, None),
                SystemFolder::Trash => (ViewType::Trash, None),
                SystemFolder::Spam => (ViewType::Spam, None),
                SystemFolder::AllMail => (ViewType::AllMail, None),
            }
        }
        SidebarSelection::Bundle(_) => (ViewType::Bundle, None),
        SidebarSelection::FeatureView(fv) => {
            use types::FeatureView;
            match fv {
                FeatureView::Tasks => (ViewType::Tasks, None),
                FeatureView::Attachments => (ViewType::Attachments, None),
            }
        }
        SidebarSelection::SmartFolder { id } => (ViewType::SmartFolder, Some(id.clone())),
        SidebarSelection::ProviderFolder(fid) => (ViewType::Label, Some(fid.0.clone())),
        SidebarSelection::Tag(tid) => (ViewType::Label, Some(tid.0.clone())),
    }
}

pub use crate::action_resolve::MailActionIntent;

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
fn current_mailbox_rights(app: &App) -> (Option<bool>, Option<bool>, Option<bool>, Option<bool>) {
    let rights = app
        .sidebar
        .selection
        .navigation_folder_id()
        .and_then(|nav_id| {
            app.sidebar
                .nav_state
                .as_ref()?
                .folders
                .iter()
                .find_map(|f| if f.id == nav_id { f.rights.as_ref() } else { None })
        });

    match rights {
        Some(r) => (
            r.may_remove_items,
            r.may_set_seen,
            r.may_set_keywords,
            r.may_submit,
        ),
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
/// Checks non-sidebar state first (settings, calendar, search, chat),
/// then derives from `sidebar.selection`.
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

    // Shared mailbox / public folder scope
    match &app.sidebar.selected_scope {
        ViewScope::SharedMailbox { .. } => return (ViewType::SharedMailbox, None),
        ViewScope::PublicFolder { .. } => return (ViewType::PublicFolder, None),
        _ => {}
    }

    // Chat view
    if app.active_chat.is_some() {
        return (ViewType::Chat, None);
    }

    // Derive from sidebar selection
    selection_to_view_type(&app.sidebar.selection)
}

/// Resolve the active account ID and provider kind from sidebar state.
///
/// When scoped to a single account, also resolves `ProviderKind` from
/// the account's `provider` field so availability predicates work.
fn active_account_info(app: &App) -> (Option<String>, Option<cmdk::ProviderKind>) {
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
fn provider_str_to_kind(provider: &str) -> Option<cmdk::ProviderKind> {
    match provider {
        "gmail_api" => Some(cmdk::ProviderKind::Gmail),
        "jmap" => Some(cmdk::ProviderKind::Jmap),
        "graph" => Some(cmdk::ProviderKind::Graph),
        "imap" => Some(cmdk::ProviderKind::Imap),
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
            let (in_trash, in_spam, is_draft) = (
                Some(app.sidebar.selection.is_trash()),
                Some(app.sidebar.selection.is_spam()),
                Some(app.sidebar.selection.is_draft()),
            );

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
/// Shorthand for sidebar-backed navigation messages.
fn nav_msg(selection: SidebarSelection) -> Message {
    Message::NavigateTo(NavigationTarget::Sidebar {
        selection,
        account_id: None,
    })
}

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
        CommandId::NavGoInbox => Some(nav_msg(SidebarSelection::Inbox)),
        CommandId::NavGoStarred => {
            Some(nav_msg(SidebarSelection::Folder(types::SystemFolder::Starred)))
        }
        CommandId::NavGoSent => {
            Some(nav_msg(SidebarSelection::Folder(types::SystemFolder::Sent)))
        }
        CommandId::NavGoDrafts => {
            Some(nav_msg(SidebarSelection::Folder(types::SystemFolder::Draft)))
        }
        CommandId::NavGoSnoozed => {
            Some(nav_msg(SidebarSelection::Folder(types::SystemFolder::Snoozed)))
        }
        CommandId::NavGoTrash => {
            Some(nav_msg(SidebarSelection::Folder(types::SystemFolder::Trash)))
        }
        CommandId::NavGoAllMail => {
            Some(nav_msg(SidebarSelection::Folder(types::SystemFolder::AllMail)))
        }
        CommandId::NavGoPrimary => {
            Some(nav_msg(SidebarSelection::Bundle(types::Bundle::Primary)))
        }
        CommandId::NavGoUpdates => {
            Some(nav_msg(SidebarSelection::Bundle(types::Bundle::Updates)))
        }
        CommandId::NavGoPromotions => {
            Some(nav_msg(SidebarSelection::Bundle(types::Bundle::Promotions)))
        }
        CommandId::NavGoSocial => {
            Some(nav_msg(SidebarSelection::Bundle(types::Bundle::Social)))
        }
        CommandId::NavGoNewsletters => {
            Some(nav_msg(SidebarSelection::Bundle(types::Bundle::Newsletters)))
        }
        CommandId::NavGoTasks => {
            Some(nav_msg(SidebarSelection::FeatureView(types::FeatureView::Tasks)))
        }
        CommandId::NavGoAttachments => {
            Some(nav_msg(SidebarSelection::FeatureView(types::FeatureView::Attachments)))
        }
        CommandId::NavEscape => Some(Message::Escape),

        // Email actions
        CommandId::EmailArchive => Some(Message::EmailAction(MailActionIntent::Archive)),
        CommandId::EmailTrash => Some(Message::EmailAction(MailActionIntent::Trash)),
        CommandId::EmailPermanentDelete => {
            Some(Message::EmailAction(MailActionIntent::PermanentDelete))
        }
        CommandId::EmailSpam => Some(Message::EmailAction(MailActionIntent::ToggleSpam)),
        CommandId::EmailMarkRead => Some(Message::EmailAction(MailActionIntent::ToggleRead)),
        CommandId::EmailStar => Some(Message::EmailAction(MailActionIntent::ToggleStar)),
        CommandId::EmailPin => Some(Message::EmailAction(MailActionIntent::TogglePin)),
        CommandId::EmailMute => Some(Message::EmailAction(MailActionIntent::ToggleMute)),
        CommandId::EmailUnsubscribe => Some(Message::EmailAction(MailActionIntent::Unsubscribe)),
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
        CommandId::TaskViewAll => {
            Some(nav_msg(SidebarSelection::FeatureView(types::FeatureView::Tasks)))
        }

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

        // Calendar
        CommandId::CalendarToggle => Some(Message::ToggleAppMode),
        CommandId::SwitchToCalendar => Some(Message::SetAppMode(crate::AppMode::Calendar)),
        CommandId::SwitchToMail => Some(Message::SetAppMode(crate::AppMode::Mail)),
        CommandId::CalendarViewDay => Some(Message::SetCalendarView(
            crate::ui::calendar::CalendarView::Day,
        )),
        CommandId::CalendarViewWorkWeek => Some(Message::SetCalendarView(
            crate::ui::calendar::CalendarView::WorkWeek,
        )),
        CommandId::CalendarViewWeek => Some(Message::SetCalendarView(
            crate::ui::calendar::CalendarView::Week,
        )),
        CommandId::CalendarViewMonth => Some(Message::SetCalendarView(
            crate::ui::calendar::CalendarView::Month,
        )),
        CommandId::CalendarToday => Some(Message::CalendarToday),
        CommandId::CalendarCreateEvent => Some(Message::Calendar(Box::new(
            crate::ui::calendar::CalendarMessage::CreateEvent,
        ))),
        CommandId::CalendarPopOut => Some(Message::Calendar(Box::new(
            crate::ui::calendar::CalendarMessage::PopOutCalendar,
        ))),

        // App
        CommandId::AppSearch => Some(Message::FocusSearch),
        CommandId::AppAskAi => None, // not yet implemented
        CommandId::AppHelp => Some(Message::ShowHelp),
        CommandId::AppSyncFolder => Some(Message::SyncCurrentFolder),
        CommandId::AppOpenPalette => Some(Message::Palette(
            crate::ui::palette::PaletteMessage::Open(cmdk::CommandContext::default()),
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
            crate::ui::thread_list::ThreadListMessage::SelectThread(current.saturating_sub(1)),
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
pub fn dispatch_parameterized(id: CommandId, args: CommandArgs) -> Option<Message> {
    match (id, args) {
        (CommandId::EmailMoveToFolder, CommandArgs::MoveToFolder { folder_id }) => {
            Some(Message::EmailAction(MailActionIntent::MoveToFolder {
                folder_id,
            }))
        }
        (CommandId::EmailAddLabel, CommandArgs::AddLabel { label_id }) => {
            Some(Message::EmailAction(MailActionIntent::AddLabel {
                label_id,
            }))
        }
        (CommandId::EmailRemoveLabel, CommandArgs::RemoveLabel { label_id }) => {
            Some(Message::EmailAction(MailActionIntent::RemoveLabel {
                label_id,
            }))
        }
        (CommandId::EmailSnooze, CommandArgs::Snooze { until }) => {
            Some(Message::EmailAction(MailActionIntent::Snooze { until }))
        }
        (
            CommandId::NavigateToLabel,
            CommandArgs::NavigateToFolder {
                folder_id,
                account_id,
            },
        ) => Some(Message::NavigateTo(NavigationTarget::Sidebar {
            selection: SidebarSelection::ProviderFolder(folder_id),
            account_id: Some(account_id),
        })),
        (
            CommandId::NavigateToLabel,
            CommandArgs::NavigateToTag {
                tag_id,
                account_id,
            },
        ) => Some(Message::NavigateTo(NavigationTarget::Sidebar {
            selection: SidebarSelection::Tag(tag_id),
            account_id: Some(account_id),
        })),
        (CommandId::SmartFolderSave, CommandArgs::SmartFolderSave { name }) => {
            Some(Message::SaveAsSmartFolder(name))
        }
        (other_id, other_args) => {
            log::warn!("unhandled parameterized dispatch: {other_id:?} with {other_args:?}");
            None
        }
    }
}

// ── iced key conversion ─────────────────────────────────

use cmdk::{Chord, Key, Modifiers, NamedKey};

/// Convert iced keyboard types to cmdk Chord.
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

/// Whether the modifiers include Ctrl or Cmd.
pub fn has_command_modifier(modifiers: &iced::keyboard::Modifiers) -> bool {
    modifiers.command() || modifiers.control()
}
