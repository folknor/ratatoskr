use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Element, Length, Theme};

use crate::db::{Account, Label};
use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;
use crate::Message;

#[allow(clippy::too_many_arguments)]
pub fn view<'a>(
    accounts: &'a [Account],
    selected_account: Option<usize>,
    labels: &'a [Label],
    selected_label: &'a Option<String>,
    scope_dropdown_open: bool,
    labels_expanded: bool,
    smart_folders_expanded: bool,
) -> Element<'a, Message> {
    let mut col = column![].spacing(SPACE_XXS).width(SIDEBAR_WIDTH);

    // ── Scope selector (dropdown) ────────────────────────
    col = col.push(scope_selector(
        accounts,
        selected_account,
        scope_dropdown_open,
    ));

    col = col.push(Space::new().height(SPACE_XXS));

    // ── Compose button ──────────────────────────────────
    col = col.push(compose_button());

    col = col.push(Space::new().height(SPACE_XS));

    // ── Navigation items with unread badges ──────────────
    col = col.push(nav_section(selected_label));

    col = col.push(Space::new().height(SPACE_XXS));
    col = col.push(widgets::divider());
    col = col.push(Space::new().height(SPACE_XXS));

    // ── Smart Folders (collapsible) ─────────────────────
    col = col.push(smart_folders_section(smart_folders_expanded));

    col = col.push(Space::new().height(SPACE_XXS));
    col = col.push(widgets::divider());
    col = col.push(Space::new().height(SPACE_XXS));

    // ── Labels (collapsible) ────────────────────────────
    col = col.push(labels_section(
        labels,
        selected_label,
        labels_expanded,
    ));

    // ── Bottom spacer + settings ────────────────────────
    col = col.push(Space::new().height(Length::Fill));
    col = col.push(settings_button());

    container(scrollable(col).height(Length::Fill))
        .padding(PAD_SIDEBAR)
        .width(SIDEBAR_WIDTH)
        .style(theme::sidebar_container)
        .into()
}

// ── Scope selector ──────────────────────────────────────

fn scope_selector<'a>(
    accounts: &'a [Account],
    selected_account: Option<usize>,
    dropdown_open: bool,
) -> Element<'a, Message> {
    let trigger_content: Element<'a, Message> = if let Some(idx) = selected_account {
        if let Some(acc) = accounts.get(idx) {
            let name = acc.display_name.as_deref().unwrap_or(&acc.email);
            row![
                widgets::avatar_circle(name, 24.0),
                text(name).size(12).style(text::base),
            ]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center)
            .into()
        } else {
            text("All Accounts").size(12).style(text::base).into()
        }
    } else {
        text("All Accounts").size(12).style(text::base).into()
    };

    let trigger = widgets::dropdown_trigger(trigger_content, Message::ToggleScopeDropdown);

    if !dropdown_open {
        return trigger;
    }

    // Build dropdown items
    let mut items: Vec<Element<'a, Message>> = Vec::new();

    // "All Accounts" option
    items.push(widgets::dropdown_item(
        row![
            icon::inbox().size(12).style(text::secondary),
            text("All Accounts").size(12).style(text::base),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center)
        .into(),
        selected_account.is_none(),
        Message::ToggleScopeDropdown,
    ));

    // Per-account items
    for (idx, acc) in accounts.iter().enumerate() {
        let name = acc.display_name.as_deref().unwrap_or(&acc.email);
        let is_selected = selected_account == Some(idx);
        items.push(widgets::dropdown_item(
            row![
                widgets::avatar_circle(name, 20.0),
                text(name).size(12).style(text::base),
            ]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center)
            .into(),
            is_selected,
            Message::SelectAccount(idx),
        ));
    }

    let menu = widgets::dropdown_menu(items);

    column![trigger, menu].spacing(SPACE_XXS).into()
}

// ── Compose button ──────────────────────────────────────

fn compose_button<'a>() -> Element<'a, Message> {
    button(
        container(
            row![
                icon::pencil().size(13).color(iced::Color::WHITE),
                text("Compose").size(13).color(iced::Color::WHITE),
            ]
            .spacing(SPACE_XXS)
            .align_y(Alignment::Center),
        )
        .center_x(Length::Fill)
        .center_y(Length::Fill),
    )
    .on_press(Message::Compose)
    .padding(PAD_BUTTON)
    .style(button::primary)
    .width(Length::Fill)
    .into()
}

// ── Navigation items ────────────────────────────────────

fn nav_section<'a>(selected_label: &'a Option<String>) -> Element<'a, Message> {
    let nav_items: Vec<(&str, &str, i64)> = vec![
        ("Inbox", "INBOX", 12),
        ("Starred", "__starred", 0),
        ("Snoozed", "__snoozed", 2),
        ("Sent", "__sent", 0),
        ("Drafts", "__drafts", 3),
        ("Trash", "__trash", 0),
    ];

    let mut col = column![].spacing(SPACE_XXS);

    for (display_name, id, unread) in nav_items {
        let is_active = match selected_label {
            Some(lid) => lid == id,
            None => id == "INBOX",
        };
        let on_press = if id == "INBOX" {
            Message::SelectLabel(None)
        } else {
            Message::SelectLabel(Some(id.to_string()))
        };
        col = col.push(widgets::nav_item_with_badge(
            display_name,
            id,
            is_active,
            unread,
            on_press,
        ));
    }

    col.into()
}

// ── Smart Folders (collapsible) ─────────────────────────

fn smart_folders_section(expanded: bool) -> Element<'static, Message> {
    let children: Vec<Element<'static, Message>> = vec![
        widgets::nav_item_with_badge("VIP", "__sf_vip", false, 3, Message::Noop),
        widgets::nav_item_with_badge("Newsletters", "__sf_news", false, 0, Message::Noop),
    ];

    widgets::collapsible_section(
        "SMART FOLDERS",
        expanded,
        Message::ToggleSmartFoldersSection,
        children,
    )
}

// ── Labels (collapsible) ────────────────────────────────

fn labels_section<'a>(
    labels: &'a [Label],
    selected_label: &'a Option<String>,
    expanded: bool,
) -> Element<'a, Message> {
    let user_labels: Vec<&Label> = labels
        .iter()
        .filter(|l| {
            !matches!(
                l.name.as_str(),
                "INBOX" | "Sent" | "Drafts" | "Trash" | "Spam" | "Archive"
                    | "Junk" | "Templates" | "Calendar" | "Contacts"
                    | "Journal" | "Notes" | "Outbox" | "Deleted Items"
                    | "Sent Items" | "Conversation History"
            )
        })
        .collect();

    let children: Vec<Element<'a, Message>> = user_labels
        .iter()
        .take(12)
        .map(|label| {
            let is_active = selected_label.as_deref() == Some(&label.id);
            let color_dot = widgets::color_dot(theme::avatar_color(&label.name));
            let lbl_style: fn(&Theme) -> text::Style = if is_active {
                text::primary
            } else {
                text::secondary
            };

            button(
                row![color_dot, text(&label.name).size(12).style(lbl_style)]
                    .spacing(SPACE_XS)
                    .align_y(Alignment::Center),
            )
            .on_press(Message::SelectLabel(Some(label.id.clone())))
            .padding(PAD_ICON_BTN)
            .style(theme::nav_button(is_active))
            .width(Length::Fill)
            .into()
        })
        .collect();

    widgets::collapsible_section(
        "LABELS",
        expanded,
        Message::ToggleLabelsSection,
        children,
    )
}

// ── Settings ────────────────────────────────────────────

fn settings_button<'a>() -> Element<'a, Message> {
    button(
        row![
            icon::settings().size(12).style(text::secondary),
            text("Settings").size(12).style(text::secondary),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center),
    )
    .on_press(Message::Noop)
    .style(theme::bare_button)
    .padding(PAD_NAV_ITEM)
    .width(Length::Fill)
    .into()
}
