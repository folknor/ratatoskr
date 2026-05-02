use iced::Element;

use crate::icon;
use crate::ui::theme;
use crate::ui::widgets::{self, DropdownEntry, DropdownIcon};
use rtsk::scope::ViewScope;

use super::{Sidebar, SidebarMessage};

// ── Scope dropdown ──────────────────────────────────────

pub(super) fn scope_dropdown(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let (trigger_icon, trigger_label): (DropdownIcon<'_>, &str) = match &sidebar.selected_scope {
        ViewScope::Account(id) => {
            let acc = sidebar.accounts.iter().find(|a| a.id == *id);
            if let Some(acc) = acc {
                let name = acc
                    .account_name
                    .as_deref()
                    .or(acc.display_name.as_deref())
                    .unwrap_or(&acc.email);
                let icon = DropdownIcon::Avatar {
                    name,
                    color: acc.account_color.as_deref().map(theme::hex_to_color),
                };
                (icon, name)
            } else {
                (DropdownIcon::Icon(icon::INBOX_CODEPOINT), "All Accounts")
            }
        }
        ViewScope::SharedMailbox { mailbox_id, .. } => {
            let name = sidebar
                .shared_mailboxes
                .iter()
                .find(|sm| sm.mailbox_id == *mailbox_id)
                .and_then(|sm| sm.display_name.as_deref())
                .unwrap_or(mailbox_id.as_str());
            (DropdownIcon::Icon('\u{e1a4}'), name)
        }
        ViewScope::PublicFolder { folder_id, .. } => {
            let name = sidebar
                .pinned_public_folders
                .iter()
                .find(|pf| pf.folder_id == *folder_id)
                .map(|pf| pf.display_name.as_str())
                .unwrap_or(folder_id.as_str());
            (DropdownIcon::Icon('\u{e0d7}'), name)
        }
        ViewScope::AllAccounts => (DropdownIcon::Icon(icon::INBOX_CODEPOINT), "All Accounts"),
    };

    let mut entries: Vec<DropdownEntry<'_, SidebarMessage>> = Vec::new();

    entries.push(DropdownEntry {
        icon: DropdownIcon::Icon(icon::INBOX_CODEPOINT),
        label: "All Accounts",
        selected: matches!(sidebar.selected_scope, ViewScope::AllAccounts),
        on_press: SidebarMessage::SelectAllAccounts,
    });

    for (idx, acc) in sidebar.accounts.iter().enumerate() {
        let name = acc
            .account_name
            .as_deref()
            .or(acc.display_name.as_deref())
            .unwrap_or(&acc.email);
        entries.push(DropdownEntry {
            icon: DropdownIcon::Avatar {
                name,
                color: acc.account_color.as_deref().map(theme::hex_to_color),
            },
            label: name,
            selected: matches!(&sidebar.selected_scope, ViewScope::Account(id) if *id == acc.id),
            on_press: SidebarMessage::SelectAccount(idx),
        });
    }

    // Shared/delegated mailboxes
    for sm in &sidebar.shared_mailboxes {
        let name = sm.display_name.as_deref().unwrap_or(&sm.mailbox_id);
        let selected = matches!(
            &sidebar.selected_scope,
            ViewScope::SharedMailbox { account_id, mailbox_id }
                if *account_id == sm.account_id && *mailbox_id == sm.mailbox_id
        );
        entries.push(DropdownEntry {
            icon: DropdownIcon::Icon('\u{e1a4}'), // users icon
            label: name,
            selected,
            on_press: SidebarMessage::SelectSharedMailbox(
                sm.account_id.clone(),
                sm.mailbox_id.clone(),
            ),
        });
    }

    // Pinned public folders
    for pf in &sidebar.pinned_public_folders {
        let selected = matches!(
            &sidebar.selected_scope,
            ViewScope::PublicFolder { account_id, folder_id }
                if *account_id == pf.account_id && *folder_id == pf.folder_id
        );
        entries.push(DropdownEntry {
            icon: DropdownIcon::Icon('\u{e0d7}'),
            label: &pf.display_name,
            selected,
            on_press: SidebarMessage::SelectPublicFolder(
                pf.account_id.clone(),
                pf.folder_id.clone(),
            ),
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
