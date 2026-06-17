use iced::widget::{button, column, container, row, text};
use iced::{Alignment, Element, Length};

use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;

use super::{Sidebar, SidebarMessage, truncate_query};

// ── Chats ───────────────────────────────────────────────

pub(super) fn chats_section(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let children: Vec<Element<'_, SidebarMessage>> = sidebar
        .chat_contacts
        .iter()
        .map(|c| chat_entry_card(sidebar, c))
        .collect();

    widgets::collapsible_section(
        "CHATS",
        sidebar.chats_expanded,
        SidebarMessage::ToggleChatsSection,
        children,
    )
}

/// Maximum display lengths for the two chat-entry text lines.
const CHAT_NAME_MAX_CHARS: usize = 22;
const CHAT_PREVIEW_MAX_CHARS: usize = 32;

fn chat_entry_card<'a>(
    sidebar: &'a Sidebar,
    contact: &'a rtsk::chat::ChatContactSummary,
) -> Element<'a, SidebarMessage> {
    use iced::widget::text::Wrapping;

    let active = sidebar.active_chat.as_deref() == Some(contact.email.as_str());
    let unread = contact.unread_count > 0;

    let display_name = contact
        .display_name
        .clone()
        .unwrap_or_else(|| contact.email.clone());
    let name_display = truncate_query(&display_name, CHAT_NAME_MAX_CHARS);

    let time_label = contact
        .latest_message_at
        .map(format_relative_time_short)
        .unwrap_or_default();

    let preview_display = contact
        .latest_message_preview
        .as_deref()
        .map(|p| truncate_query(p, CHAT_PREVIEW_MAX_CHARS))
        .unwrap_or_else(|| "-".to_string());

    let avatar = container(widgets::avatar_circle::<SidebarMessage>(
        &display_name,
        AVATAR_DROPDOWN_TRIGGER,
    ))
    .align_y(Alignment::Center);

    let name_style: fn(&iced::Theme) -> text::Style =
        if active { text::primary } else { text::base };
    let name_font = if unread {
        crate::font::text_bold()
    } else {
        crate::font::text()
    };

    let name_widget = text(name_display)
        .size(TEXT_SM)
        .style(name_style)
        .font(name_font)
        .wrapping(Wrapping::None);

    let header_row = row![
        container(name_widget)
            .width(Length::Fill)
            .align_y(Alignment::Center),
        text(time_label)
            .size(TEXT_XS)
            .style(theme::TextClass::Muted.style())
            .wrapping(Wrapping::None),
    ]
    .spacing(SPACE_XXS)
    .align_y(Alignment::Center);

    let preview_widget = text(preview_display)
        .size(TEXT_SM)
        .style(theme::TextClass::Muted.style())
        .wrapping(Wrapping::None);

    let text_col = column![header_row, preview_widget]
        .spacing(SPACE_XXXS)
        .width(Length::Fill);

    let content = row![avatar, text_col]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center);

    button(container(content).padding(PAD_NAV_ITEM))
        .on_press(SidebarMessage::SelectChat(contact.email.clone()))
        .padding(0)
        .style(theme::ButtonClass::PinnedSearch { active }.style())
        .width(Length::Fill)
        .into()
}

/// Compact relative time label used by the CHATS section ("just now", "5m",
/// "2h", "3d"). Distinct from `format_relative_time` (used by pinned searches)
/// because the chat sidebar entries need shorter labels per the chats spec.
fn format_relative_time_short(timestamp: i64) -> String {
    let Some(dt) = chrono::DateTime::from_timestamp(timestamp, 0) else {
        return String::new();
    };
    let now = chrono::Utc::now();
    let delta = now.signed_duration_since(dt);

    if delta.num_seconds() < 60 {
        "now".to_string()
    } else if delta.num_minutes() < 60 {
        format!("{}m", delta.num_minutes())
    } else if delta.num_hours() < 24 {
        format!("{}h", delta.num_hours())
    } else {
        format!("{}d", delta.num_days())
    }
}
