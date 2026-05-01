use iced::Element;

use crate::ui::dialog::{DialogAction, alert_dialog};

use super::messages::CalendarMessage;

/// Confirmation dialog before deleting an event.
pub(super) fn delete_confirm_card(title: &str) -> Element<'_, CalendarMessage> {
    let display_title = if title.is_empty() {
        "(Untitled)"
    } else {
        title
    };
    alert_dialog(
        "Delete event?",
        format!("Delete \"{display_title}\"? This cannot be undone."),
        vec![
            DialogAction::default_action("Cancel", CalendarMessage::CloseModal),
            DialogAction::destructive("Delete", CalendarMessage::DeleteEvent),
        ],
        None,
    )
}

/// Confirmation dialog before discarding unsaved editor changes.
pub(super) fn discard_confirm_card(_title: &str) -> Element<'_, CalendarMessage> {
    alert_dialog(
        "Discard unsaved changes?",
        "Your edits to this event will be lost.",
        vec![
            DialogAction::default_action("Keep editing", CalendarMessage::CloseModal),
            DialogAction::destructive("Discard", CalendarMessage::DiscardChanges),
        ],
        None,
    )
}
