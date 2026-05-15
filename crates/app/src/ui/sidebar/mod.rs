use std::collections::HashSet;

use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Length, Task};

use crate::component::Component;
use crate::db::{Account, PinnedPublicFolder, PinnedSearch, SharedMailbox};
use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;
use rtsk::db::queries_extra::navigation::{FolderKind, NavigationState};
use rtsk::scope::ViewScope;
use types::{FolderId, SidebarSelection};

mod chats;
mod folders;
mod labels;
mod nav;
mod pinned_searches;
mod public_folders;
mod scope;
mod search_here;
mod smart;

use chats::chats_section;
use labels::labels_section;
use nav::nav_items;
use pinned_searches::pinned_searches_section;
use public_folders::pinned_public_folders_section;
use scope::scope_dropdown;
use smart::smart_folders;

// ── Messages & Events ──────────────────────────────────

#[derive(Debug, Clone)]
pub enum SidebarMessage {
    SelectAccount(usize),
    SelectAllAccounts,
    CycleAccount,
    Select(SidebarSelection),
    ToggleScopeDropdown,
    ToggleLabelsSection,
    ToggleSmartFoldersSection,
    ToggleFolderExpand(String),
    Compose,
    ToggleSettings,
    SelectPinnedSearch(i64),
    DismissPinnedSearch(i64),
    RefreshPinnedSearch(i64),
    ToggleChatsSection,
    SelectChat(String),
    ToggleMode,
    /// Right-click "Search here" on a label/folder.
    SearchHere(String),
    /// Click a smart folder - execute its query via the unified search pipeline.
    SelectSmartFolder {
        id: String,
        query: String,
    },
    /// Select a shared/delegated mailbox in the scope dropdown.
    /// Carries (account_id, mailbox_id).
    SelectSharedMailbox(String, String),
    /// Select a pinned public folder.
    /// Carries (account_id, folder_id).
    SelectPublicFolder(String, String),
}

/// Events the sidebar emits upward to the App.
#[derive(Debug, Clone)]
#[allow(dead_code)] // shared-mailbox / public-folder routing is wired one variant at a time
pub enum SidebarEvent {
    AccountSelected(usize),
    AllAccountsSelected,
    SelectionChanged(SidebarSelection),
    Compose,
    ToggleSettings,
    PinnedSearchSelected(i64),
    PinnedSearchDismissed(i64),
    PinnedSearchRefreshed(i64),
    ChatSelected(String),
    ModeToggled,
    /// "Search here" - prefill search bar with a scope query prefix.
    SearchHere {
        query_prefix: String,
    },
    /// Smart folder selected - execute its query via the unified search pipeline.
    SmartFolderSelected {
        id: String,
        query: String,
    },
    /// Shared mailbox selected in scope dropdown.
    /// Carries (account_id, mailbox_id).
    SharedMailboxSelected {
        account_id: String,
        mailbox_id: String,
    },
    /// Pinned public folder selected.
    /// Carries (account_id, folder_id).
    PublicFolderSelected {
        account_id: String,
        folder_id: String,
    },
}

// ── Sidebar layout constants ─────────────────────────────

/// Maximum display length for pinned search query text before truncation.
const PINNED_SEARCH_QUERY_MAX_CHARS: usize = 28;

// ── State ──────────────────────────────────────────────

pub struct Sidebar {
    pub accounts: Vec<Account>,
    pub nav_state: Option<NavigationState>,
    pub selected_scope: ViewScope,
    pub selection: SidebarSelection,
    pub scope_dropdown_open: bool,
    pub labels_expanded: bool,
    pub smart_folders_expanded: bool,
    /// Set of folder IDs whose children are collapsed (hidden).
    pub collapsed_folders: HashSet<String>,
    /// Pinned searches, set by parent App before each view.
    pub pinned_searches: Vec<PinnedSearch>,
    /// Currently selected pinned search, set by parent App.
    pub active_pinned_search: Option<i64>,
    /// Shared/delegated mailboxes discovered via Autodiscover.
    pub shared_mailboxes: Vec<SharedMailbox>,
    /// Pinned public folders.
    pub pinned_public_folders: Vec<PinnedPublicFolder>,
    /// Designated chat contacts, set by parent App after each load.
    pub chat_contacts: Vec<rtsk::chat::ChatContactSummary>,
    /// Email of the chat contact whose timeline is currently active, if any.
    /// Pushed by App::view before each Sidebar::view call.
    pub active_chat: Option<String>,
    /// Whether the CHATS section is expanded.
    pub chats_expanded: bool,
}

impl Sidebar {
    pub fn new() -> Self {
        Self {
            accounts: Vec::new(),
            nav_state: None,
            selected_scope: ViewScope::AllAccounts,
            selection: SidebarSelection::Inbox,
            scope_dropdown_open: false,
            labels_expanded: true,
            smart_folders_expanded: true,
            collapsed_folders: HashSet::new(),
            pinned_searches: Vec::new(),
            active_pinned_search: None,
            shared_mailboxes: Vec::new(),
            pinned_public_folders: Vec::new(),
            chat_contacts: Vec::new(),
            active_chat: None,
            chats_expanded: true,
        }
    }

    pub fn is_all_accounts(&self) -> bool {
        matches!(self.selected_scope, ViewScope::AllAccounts)
    }

    /// Return the selected account index, if scope is a single account.
    pub fn selected_account_index(&self) -> Option<usize> {
        if let ViewScope::Account(ref id) = self.selected_scope {
            self.accounts.iter().position(|a| a.id == *id)
        } else {
            None
        }
    }
}

// ── Component impl ─────────────────────────────────────

impl Component for Sidebar {
    type Message = SidebarMessage;
    type Event = SidebarEvent;

    fn update(&mut self, message: SidebarMessage) -> (Task<SidebarMessage>, Option<SidebarEvent>) {
        match message {
            SidebarMessage::SelectAccount(idx) => {
                let account_id = self
                    .accounts
                    .get(idx)
                    .map(|a| a.id.clone())
                    .unwrap_or_default();
                self.selected_scope = ViewScope::Account(account_id);
                self.selection = SidebarSelection::Inbox;
                self.scope_dropdown_open = false;
                (Task::none(), Some(SidebarEvent::AccountSelected(idx)))
            }
            SidebarMessage::SelectAllAccounts => {
                self.selected_scope = ViewScope::AllAccounts;
                self.selection = SidebarSelection::Inbox;
                self.scope_dropdown_open = false;
                (Task::none(), Some(SidebarEvent::AllAccountsSelected))
            }
            SidebarMessage::CycleAccount => {
                if self.accounts.len() > 1 {
                    let current_idx = self.selected_account_index();
                    let next = match current_idx {
                        Some(idx) => (idx + 1) % self.accounts.len(),
                        None => 0,
                    };
                    let account_id = self.accounts[next].id.clone();
                    self.selected_scope = ViewScope::Account(account_id);
                    self.selection = SidebarSelection::Inbox;
                    self.scope_dropdown_open = false;
                    (Task::none(), Some(SidebarEvent::AccountSelected(next)))
                } else {
                    (Task::none(), None)
                }
            }
            SidebarMessage::Select(sel) => {
                self.selection = sel.clone();
                (Task::none(), Some(SidebarEvent::SelectionChanged(sel)))
            }
            SidebarMessage::ToggleScopeDropdown => {
                self.scope_dropdown_open = !self.scope_dropdown_open;
                (Task::none(), None)
            }
            SidebarMessage::ToggleLabelsSection => {
                self.labels_expanded = !self.labels_expanded;
                (Task::none(), None)
            }
            SidebarMessage::ToggleSmartFoldersSection => {
                self.smart_folders_expanded = !self.smart_folders_expanded;
                (Task::none(), None)
            }
            SidebarMessage::ToggleFolderExpand(folder_id) => {
                if !self.collapsed_folders.remove(&folder_id) {
                    self.collapsed_folders.insert(folder_id);
                }
                (Task::none(), None)
            }
            SidebarMessage::Compose => (Task::none(), Some(SidebarEvent::Compose)),
            SidebarMessage::ToggleSettings => (Task::none(), Some(SidebarEvent::ToggleSettings)),
            SidebarMessage::SelectPinnedSearch(id) => {
                (Task::none(), Some(SidebarEvent::PinnedSearchSelected(id)))
            }
            SidebarMessage::DismissPinnedSearch(id) => {
                (Task::none(), Some(SidebarEvent::PinnedSearchDismissed(id)))
            }
            SidebarMessage::RefreshPinnedSearch(id) => {
                (Task::none(), Some(SidebarEvent::PinnedSearchRefreshed(id)))
            }
            SidebarMessage::ToggleChatsSection => {
                self.chats_expanded = !self.chats_expanded;
                (Task::none(), None)
            }
            SidebarMessage::SelectChat(email) => {
                (Task::none(), Some(SidebarEvent::ChatSelected(email)))
            }
            SidebarMessage::ToggleMode => (Task::none(), Some(SidebarEvent::ModeToggled)),
            SidebarMessage::SearchHere(query_prefix) => (
                Task::none(),
                Some(SidebarEvent::SearchHere { query_prefix }),
            ),
            SidebarMessage::SelectSmartFolder { id, query } => {
                self.selection = SidebarSelection::SmartFolder { id: id.clone() };
                (
                    Task::none(),
                    Some(SidebarEvent::SmartFolderSelected { id, query }),
                )
            }
            SidebarMessage::SelectSharedMailbox(account_id, mailbox_id) => {
                self.selected_scope = ViewScope::SharedMailbox {
                    account_id: account_id.clone(),
                    mailbox_id: mailbox_id.clone(),
                };
                self.selection = SidebarSelection::Inbox;
                self.scope_dropdown_open = false;
                (
                    Task::none(),
                    Some(SidebarEvent::SharedMailboxSelected {
                        account_id,
                        mailbox_id,
                    }),
                )
            }
            SidebarMessage::SelectPublicFolder(account_id, folder_id) => {
                self.selected_scope = ViewScope::PublicFolder {
                    account_id: account_id.clone(),
                    folder_id: folder_id.clone(),
                };
                self.selection = SidebarSelection::Inbox;
                self.scope_dropdown_open = false;
                (
                    Task::none(),
                    Some(SidebarEvent::PublicFolderSelected {
                        account_id,
                        folder_id,
                    }),
                )
            }
        }
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    fn view(&self) -> Element<'_, SidebarMessage> {
        let show_labels = matches!(
            self.selected_scope,
            ViewScope::Account(_) | ViewScope::SharedMailbox { .. }
        );

        // Mode toggle button (tall square spanning dropdown + compose height)
        let mode_btn = container(
            button(
                container(icon::calendar().size(ICON_HERO).style(text::primary))
                    .center_x(Length::Fill)
                    .center_y(Length::Fill),
            )
            .on_press(SidebarMessage::ToggleMode)
            .height(Length::Fill)
            .width(Length::Fill)
            .style(theme::ButtonClass::Experiment { variant: 10 }.style()),
        )
        .width(SIDEBAR_HEADER_HEIGHT) // square
        .height(Length::Fill);

        // Stack the dropdown and compose button vertically
        let right_stack = container(
            column![
                scope_dropdown(self),
                Space::new().height(SPACE_XXS),
                widgets::compose_button(SidebarMessage::Compose),
            ]
            .spacing(0)
            .width(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(Alignment::Center);

        // Fixed-height header so Fill children resolve correctly
        let header_row = container(
            row![mode_btn, right_stack]
                .spacing(SPACE_XXS)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .height(SIDEBAR_HEADER_HEIGHT)
        .width(Length::Fill);

        let mut scroll_content = column![Space::new().height(SPACE_XXS)]
            .spacing(0)
            .width(Length::Fill);

        // Pinned searches (only if non-empty)
        if !self.pinned_searches.is_empty() {
            scroll_content = scroll_content.push(pinned_searches_section(self));
            scroll_content = scroll_content.push(Space::new().height(SPACE_XXS));
        }

        // Chats (between pinned searches and universal folders, only if non-empty,
        // scope-independent)
        if !self.chat_contacts.is_empty() {
            if !self.pinned_searches.is_empty() {
                scroll_content = scroll_content.push(widgets::section_break::<SidebarMessage>());
            }
            scroll_content = scroll_content.push(chats_section(self));
            scroll_content = scroll_content.push(Space::new().height(SPACE_XXS));
        }

        scroll_content = scroll_content.push(Space::new().height(SPACE_XS));
        scroll_content = scroll_content.push(nav_items(self));
        scroll_content = scroll_content.push(widgets::section_break());
        scroll_content = scroll_content.push(smart_folders(self));

        let has_account_labels = self.nav_state.as_ref().is_some_and(|ns| {
            ns.folders
                .iter()
                .any(|f| matches!(f.folder_kind, FolderKind::AccountLabel))
        });
        if show_labels && has_account_labels {
            scroll_content = scroll_content.push(widgets::section_break::<SidebarMessage>());
            scroll_content = scroll_content.push(labels_section(self));
        }

        // Pinned public folders (if any)
        if !self.pinned_public_folders.is_empty() {
            scroll_content = scroll_content.push(widgets::section_break::<SidebarMessage>());
            scroll_content = scroll_content.push(pinned_public_folders_section(self));
        }

        container(
            column![
                header_row,
                scrollable(scroll_content)
                    .spacing(SCROLLBAR_SPACING)
                    .height(Length::Fill),
                widgets::settings_button(SidebarMessage::ToggleSettings),
            ]
            .spacing(SPACE_XS),
        )
        .padding(PAD_SIDEBAR)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::ContainerClass::Sidebar.style())
        .into()
    }
}

/// Truncates a query string for display, adding ellipsis if needed.
pub fn truncate_query(query: &str, max_chars: usize) -> String {
    if query.len() <= max_chars {
        query.to_string()
    } else {
        format!("{}...", &query[..query.floor_char_boundary(max_chars)])
    }
}

/// Map a universal folder's DB ID to the corresponding `SidebarSelection`.
pub(crate) fn universal_folder_selection(id: &str) -> SidebarSelection {
    use types::{Bundle, FeatureView, SystemFolder};
    match id {
        "INBOX" => SidebarSelection::Inbox,
        "STARRED" => SidebarSelection::Folder(SystemFolder::Starred),
        "SENT" => SidebarSelection::Folder(SystemFolder::Sent),
        "DRAFT" => SidebarSelection::Folder(SystemFolder::Draft),
        "SNOOZED" => SidebarSelection::Folder(SystemFolder::Snoozed),
        "TRASH" => SidebarSelection::Folder(SystemFolder::Trash),
        "SPAM" => SidebarSelection::Folder(SystemFolder::Spam),
        "all-mail" => SidebarSelection::Folder(SystemFolder::AllMail),
        "BUNDLE_PRIMARY" => SidebarSelection::Bundle(Bundle::Primary),
        "BUNDLE_UPDATES" => SidebarSelection::Bundle(Bundle::Updates),
        "BUNDLE_PROMOTIONS" => SidebarSelection::Bundle(Bundle::Promotions),
        "BUNDLE_SOCIAL" => SidebarSelection::Bundle(Bundle::Social),
        "BUNDLE_NEWSLETTERS" => SidebarSelection::Bundle(Bundle::Newsletters),
        "TASKS" => SidebarSelection::FeatureView(FeatureView::Tasks),
        "ATTACHMENTS" => SidebarSelection::FeatureView(FeatureView::Attachments),
        other => SidebarSelection::ProviderFolder(FolderId::from(other)),
    }
}
