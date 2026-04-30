use iced::widget::{column, text};
use iced::{Element, Length};

use crate::icon;
use crate::ui::layout::*;
use crate::ui::settings::row_widgets::*;
use crate::ui::settings::types::*;

pub(super) fn mail_rules_tab(state: &Settings) -> Element<'_, SettingsMessage> {
    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section(
        "Labels",
        vec![editable_list(
            "labels",
            &state.demo_labels,
            "Add Label",
            &state.drag_state,
        )],
    ));
    col = col.push(section(
        "Filters",
        vec![action_row(
            "Create Filter",
            Some("Add a new mail filter rule"),
            Some(icon::filter()),
            ActionKind::InApp,
            SettingsMessage::OpenSheet(SettingsSheetPage::CreateFilter),
        )],
    ));
    if !state.demo_filters.is_empty() {
        col = col.push(section_untitled(vec![editable_list(
            "filters",
            &state.demo_filters,
            "Add Filter",
            &state.drag_state,
        )]));
    }
    col = col.push(section(
        "Smart Labels",
        vec![coming_soon_row("Smart label management")],
    ));
    col = col.push(section(
        "Smart Folders",
        vec![coming_soon_row("Smart folder management")],
    ));
    col = col.push(section(
        "Quick Steps",
        vec![coming_soon_row("Quick step management")],
    ));

    col.into()
}

pub(super) fn create_filter_sheet<'a>() -> Element<'a, SettingsMessage> {
    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(
        text("Create Filter")
            .size(TEXT_HEADING)
            .style(text::base)
            .font(iced::Font {
                weight: iced::font::Weight::Bold,
                ..crate::font::text()
            }),
    );

    col = col.push(section(
        "Conditions",
        vec![coming_soon_row("Match conditions")],
    ));

    col = col.push(section("Actions", vec![coming_soon_row("Filter actions")]));

    col.into()
}
