use iced::widget::{column, container, scrollable, Space};
use iced::{Element, Length};

use crate::db::{Account, Label};
use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets::{self, DropdownEntry, DropdownIcon, NavItem};
use crate::Message;

pub struct SidebarModel<'a> {
    pub accounts: &'a [Account],
    pub selected_account: Option<usize>,
    pub labels: &'a [Label],
    pub selected_label: &'a Option<String>,
    pub scope_dropdown_open: bool,
    pub labels_expanded: bool,
    pub smart_folders_expanded: bool,
}

impl SidebarModel<'_> {
    fn is_all_accounts(&self) -> bool {
        self.selected_account.is_none()
    }
}

pub fn view<'a>(model: SidebarModel<'a>) -> Element<'a, Message> {
    let mut col = column![
        scope_dropdown(&model),
        Space::new().height(SPACE_XXS),
        widgets::compose_button(),
        Space::new().height(SPACE_XS),
        nav_items(&model),
        widgets::section_break(),
        smart_folders(model.smart_folders_expanded),
    ]
    .spacing(0)
    .width(Length::Fill);

    if !model.is_all_accounts() {
        col = col.push(widgets::section_break());
        col = col.push(labels(&model));
    }

    container(
        column![
            scrollable(col).height(Length::Fill),
            widgets::settings_button(),
        ]
        .spacing(SPACE_XS),
    )
    .padding(PAD_SIDEBAR)
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::sidebar_container)
    .into()
}

// ── Scope dropdown ──────────────────────────────────────

fn scope_dropdown<'a>(model: &SidebarModel<'a>) -> Element<'a, Message> {
    // Determine trigger icon + label from current selection
    let (trigger_icon, trigger_label): (DropdownIcon<'a>, &'a str) =
        match model.selected_account {
            Some(idx) if model.accounts.get(idx).is_some() => {
                let acc = &model.accounts[idx];
                let name = acc.display_name.as_deref().unwrap_or(&acc.email);
                (DropdownIcon::Avatar(name), name)
            }
            _ => (DropdownIcon::Icon(icon::INBOX_CODEPOINT), "All Accounts"),
        };

    // Build dropdown entries
    let mut entries: Vec<DropdownEntry<'a>> = Vec::new();

    entries.push(DropdownEntry {
        icon: DropdownIcon::Icon(icon::INBOX_CODEPOINT),
        label: "All Accounts",
        selected: model.selected_account.is_none(),
        on_press: Message::SelectAllAccounts,
    });

    for (idx, acc) in model.accounts.iter().enumerate() {
        let name = acc.display_name.as_deref().unwrap_or(&acc.email);
        entries.push(DropdownEntry {
            icon: DropdownIcon::Avatar(name),
            label: name,
            selected: model.selected_account == Some(idx),
            on_press: Message::SelectAccount(idx),
        });
    }

    widgets::dropdown(
        trigger_icon,
        trigger_label,
        model.scope_dropdown_open,
        Message::ToggleScopeDropdown,
        entries,
    )
}

// ── Nav items ───────────────────────────────────────────

fn nav_items<'a>(model: &SidebarModel<'a>) -> Element<'a, Message> {
    let mut items = vec![
        NavItem { label: "Inbox",   id: "INBOX",     unread: 12 },
        NavItem { label: "Starred", id: "__starred",  unread: 0 },
        NavItem { label: "Snoozed", id: "__snoozed",  unread: 2 },
        NavItem { label: "Sent",    id: "__sent",     unread: 0 },
        NavItem { label: "Drafts",  id: "__drafts",   unread: 3 },
        NavItem { label: "Trash",   id: "__trash",    unread: 0 },
    ];
    if !model.is_all_accounts() {
        items.push(NavItem { label: "Spam",     id: "__spam",     unread: 0 });
        items.push(NavItem { label: "All Mail", id: "__all_mail", unread: 0 });
    }
    widgets::nav_group(&items, model.selected_label)
}

fn smart_folders(expanded: bool) -> Element<'static, Message> {
    widgets::collapsible_section(
        "SMART FOLDERS",
        expanded,
        Message::ToggleSmartFoldersSection,
        vec![
            widgets::nav_button(None, "VIP", false, widgets::NavSize::Compact, Some(3), Message::Noop),
            widgets::nav_button(None, "Newsletters", false, widgets::NavSize::Compact, Some(0), Message::Noop),
        ],
    )
}

fn labels<'a>(model: &SidebarModel<'a>) -> Element<'a, Message> {
    let children = model
        .labels
        .iter()
        .filter(|l| !is_system_label(&l.name))
        .take(12)
        .map(|l| {
            let active = model.selected_label.as_deref() == Some(&l.id);
            widgets::label_nav_item(
                &l.name,
                &l.id,
                theme::avatar_color(&l.name),
                active,
                Message::SelectLabel(Some(l.id.clone())),
            )
        })
        .collect();

    widgets::collapsible_section("LABELS", model.labels_expanded, Message::ToggleLabelsSection, children)
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
