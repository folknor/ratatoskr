use iced::widget::column;
use iced::{Element, Length};

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
    if !state.demo_filters.is_empty() {
        col = col.push(section(
            "Filters",
            vec![editable_list(
                "filters",
                &state.demo_filters,
                "Add Filter",
                &state.drag_state,
            )],
        ));
    }
    col.into()
}
