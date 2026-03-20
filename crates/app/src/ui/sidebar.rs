use iced::widget::{column, container, scrollable, Space};
use iced::{Element, Length, Task};

use crate::component::Component;
use crate::db::{Account, Label};
use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets::{self, DropdownEntry, DropdownIcon, NavItem};

// ── Messages & Events ──────────────────────────────────

#[derive(Debug, Clone)]
pub enum SidebarMessage {
    SelectAccount(usize),
    SelectAllAccounts,
    CycleAccount,
    SelectLabel(Option<String>),
    ToggleScopeDropdown,
    ToggleLabelsSection,
    ToggleSmartFoldersSection,
    Compose,
    ToggleSettings,
    Noop,
}

/// Events the sidebar emits upward to the App.
#[derive(Debug, Clone)]
pub enum SidebarEvent {
    AccountSelected(usize),
    AllAccountsSelected,
    CycleAccount,
    LabelSelected(Option<String>),
    Compose,
    ToggleSettings,
}

// ── State ──────────────────────────────────────────────

pub struct Sidebar {
    pub accounts: Vec<Account>,
    pub labels: Vec<Label>,
    pub selected_account: Option<usize>,
    pub selected_label: Option<String>,
    pub scope_dropdown_open: bool,
    pub labels_expanded: bool,
    pub smart_folders_expanded: bool,
}

impl Sidebar {
    pub fn new() -> Self {
        Self {
            accounts: Vec::new(),
            labels: Vec::new(),
            selected_account: None,
            selected_label: None,
            scope_dropdown_open: false,
            labels_expanded: true,
            smart_folders_expanded: true,
        }
    }

    fn is_all_accounts(&self) -> bool {
        self.selected_account.is_none()
    }
}

// ── Component impl ─────────────────────────────────────

impl Component for Sidebar {
    type Message = SidebarMessage;
    type Event = SidebarEvent;

    fn update(
        &mut self,
        message: SidebarMessage,
    ) -> (Task<SidebarMessage>, Option<SidebarEvent>) {
        match message {
            SidebarMessage::SelectAccount(idx) => {
                self.selected_account = Some(idx);
                self.selected_label = None;
                self.scope_dropdown_open = false;
                (Task::none(), Some(SidebarEvent::AccountSelected(idx)))
            }
            SidebarMessage::SelectAllAccounts => {
                self.selected_account = None;
                self.selected_label = None;
                self.scope_dropdown_open = false;
                (Task::none(), Some(SidebarEvent::AllAccountsSelected))
            }
            SidebarMessage::CycleAccount => {
                if self.accounts.len() > 1 {
                    let next = match self.selected_account {
                        Some(idx) => (idx + 1) % self.accounts.len(),
                        None => 0,
                    };
                    self.update(SidebarMessage::SelectAccount(next))
                } else {
                    (Task::none(), None)
                }
            }
            SidebarMessage::SelectLabel(label_id) => {
                self.selected_label = label_id.clone();
                (Task::none(), Some(SidebarEvent::LabelSelected(label_id)))
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
            SidebarMessage::Compose => {
                (Task::none(), Some(SidebarEvent::Compose))
            }
            SidebarMessage::ToggleSettings => {
                (Task::none(), Some(SidebarEvent::ToggleSettings))
            }
            SidebarMessage::Noop => (Task::none(), None),
        }
    }

    fn view(&self) -> Element<'_, SidebarMessage> {
        let mut col = column![
            scope_dropdown(self),
            Space::new().height(SPACE_XXS),
            widgets::compose_button(SidebarMessage::Compose),
            Space::new().height(SPACE_XS),
            nav_items(self),
            widgets::section_break(),
            smart_folders(self.smart_folders_expanded),
        ]
        .spacing(0)
        .width(Length::Fill);

        if !self.is_all_accounts() {
            col = col.push(widgets::section_break::<SidebarMessage>());
            col = col.push(labels(self));
        }

        container(
            column![
                scrollable(col).spacing(SCROLLBAR_SPACING).height(Length::Fill),
                widgets::settings_button(SidebarMessage::ToggleSettings),
            ]
            .spacing(SPACE_XS),
        )
        .padding(PAD_SIDEBAR)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::sidebar_container)
        .into()
    }
}

// ── Scope dropdown ──────────────────────────────────────

fn scope_dropdown(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let (trigger_icon, trigger_label): (DropdownIcon<'_>, &str) =
        match sidebar.selected_account {
            Some(idx) if sidebar.accounts.get(idx).is_some() => {
                let acc = &sidebar.accounts[idx];
                let name = acc.display_name.as_deref().unwrap_or(&acc.email);
                (DropdownIcon::Avatar(name), name)
            }
            _ => (DropdownIcon::Icon(icon::INBOX_CODEPOINT), "All Accounts"),
        };

    let mut entries: Vec<DropdownEntry<'_, SidebarMessage>> = Vec::new();

    entries.push(DropdownEntry {
        icon: DropdownIcon::Icon(icon::INBOX_CODEPOINT),
        label: "All Accounts",
        selected: sidebar.selected_account.is_none(),
        on_press: SidebarMessage::SelectAllAccounts,
    });

    for (idx, acc) in sidebar.accounts.iter().enumerate() {
        let name = acc.display_name.as_deref().unwrap_or(&acc.email);
        entries.push(DropdownEntry {
            icon: DropdownIcon::Avatar(name),
            label: name,
            selected: sidebar.selected_account == Some(idx),
            on_press: SidebarMessage::SelectAccount(idx),
        });
    }

    widgets::dropdown(
        trigger_icon,
        trigger_label,
        sidebar.scope_dropdown_open,
        SidebarMessage::ToggleScopeDropdown,
        entries,
    )
}

// ── Nav items ───────────────────────────────────────────

fn nav_items(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let mut items = vec![
        NavItem { label: "Inbox",   id: "INBOX",     unread: 12 },
        NavItem { label: "Starred", id: "__starred",  unread: 0 },
        NavItem { label: "Snoozed", id: "__snoozed",  unread: 2 },
        NavItem { label: "Sent",    id: "__sent",     unread: 0 },
        NavItem { label: "Drafts",  id: "__drafts",   unread: 3 },
        NavItem { label: "Trash",   id: "__trash",    unread: 0 },
    ];
    if !sidebar.is_all_accounts() {
        items.push(NavItem { label: "Spam",     id: "__spam",     unread: 0 });
        items.push(NavItem { label: "All Mail", id: "__all_mail", unread: 0 });
    }
    widgets::nav_group(&items, &sidebar.selected_label, SidebarMessage::SelectLabel)
}

fn smart_folders(expanded: bool) -> Element<'static, SidebarMessage> {
    widgets::collapsible_section(
        "SMART FOLDERS",
        expanded,
        SidebarMessage::ToggleSmartFoldersSection,
        vec![
            widgets::nav_button(None, "VIP", false, widgets::NavSize::Compact, Some(3), SidebarMessage::Noop),
            widgets::nav_button(None, "Newsletters", false, widgets::NavSize::Compact, Some(0), SidebarMessage::Noop),
        ],
    )
}

fn labels(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let children = sidebar
        .labels
        .iter()
        .filter(|l| !is_system_label(&l.name))
        .take(12)
        .map(|l| {
            let active = sidebar.selected_label.as_deref() == Some(&l.id);
            widgets::label_nav_item(
                &l.name,
                &l.id,
                theme::avatar_color(&l.name),
                active,
                SidebarMessage::SelectLabel(Some(l.id.clone())),
            )
        })
        .collect();

    widgets::collapsible_section(
        "LABELS",
        sidebar.labels_expanded,
        SidebarMessage::ToggleLabelsSection,
        children,
    )
}

fn is_system_label(name: &str) -> bool {
    matches!(
        name,
        "INBOX" | "Sent" | "Drafts" | "Trash" | "Spam" | "Archive"
            | "Junk" | "Templates" | "Calendar" | "Contacts"
            | "Journal" | "Notes" | "Outbox" | "Deleted Items"
            | "Sent Items" | "Conversation History"
    )
}
