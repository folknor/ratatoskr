use iced::widget::{container, mouse_area, row, text};
use iced::{Alignment, Element, Length};

use crate::ui::calendar_month;
use crate::ui::calendar_time_grid;
use crate::ui::layout::*;
use crate::ui::theme;

use super::dialogs::{delete_confirm_card, discard_confirm_card};
use super::event_detail::event_detail_popover;
use super::event_editor::event_editor_card;
use super::event_full_modal::event_full_modal;
use super::messages::CalendarMessage;
use super::sidebar::calendar_sidebar;
use super::types::{CalendarModal, CalendarPopover, CalendarState, CalendarView, CalendarWorkflow};

/// Render the full calendar layout (sidebar + main area + calendar surfaces).
///
/// Returns an `Element<CalendarMessage>` - the parent maps this to the
/// top-level app Message.
pub fn calendar_layout(state: &CalendarState) -> Element<'_, CalendarMessage> {
    let sidebar = calendar_sidebar(state);
    let main_view = calendar_main_view(state);

    let base = row![sidebar, main_view].height(Length::Fill);

    if let Some(modal) = &state.active_modal {
        let card = match modal {
            CalendarModal::EventFull { event } => event_full_modal(event, state),
            CalendarModal::EventEditor => {
                let (draft, is_creating) = match &state.workflow {
                    CalendarWorkflow::CreatingEvent { session, .. } => (&session.draft, true),
                    CalendarWorkflow::EditingEvent { session, .. } => (&session.draft, false),
                    other => {
                        debug_assert!(
                            false,
                            "EventEditor modal without editing workflow: {other:?}"
                        );
                        log::error!("EventEditor modal without editing workflow state");
                        return container(text("")).into();
                    }
                };
                event_editor_card(draft, is_creating, &state.calendars)
            }
            CalendarModal::ConfirmDelete { title, .. } => delete_confirm_card(title),
            CalendarModal::ConfirmDiscard { title } => discard_confirm_card(title),
        };
        crate::ui::modal_overlay::modal_overlay(
            base,
            card,
            crate::ui::modal_overlay::ModalSurface::Modal,
            CalendarMessage::Noop,
        )
    } else if let Some(popover) = &state.active_popover {
        match popover {
            CalendarPopover::EventDetail { event } => {
                popover_stack(base.into(), event_detail_popover(event))
            }
        }
    } else {
        base.into()
    }
}

/// Wrap a base layout with a lightweight popover (click-away backdrop, right-aligned).
fn popover_stack<'a>(
    base: Element<'a, CalendarMessage>,
    card: Element<'a, CalendarMessage>,
) -> Element<'a, CalendarMessage> {
    let backdrop = mouse_area(container("").width(Length::Fill).height(Length::Fill))
        .on_press(CalendarMessage::ClosePopover);

    let positioned = container(container(card).align_y(Alignment::Center).max_width(320.0))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::End)
        .align_y(Alignment::Center)
        .padding(iced::Padding::from([SPACE_LG, SPACE_LG]));

    iced::widget::stack![base, backdrop, positioned].into()
}

/// Calendar main content area: dispatches to the appropriate view.
fn calendar_main_view(state: &CalendarState) -> Element<'_, CalendarMessage> {
    match state.active_view {
        CalendarView::Month => calendar_main_month(state),
        _ => calendar_main_time_grid(state),
    }
}

/// Month view main content area.
fn calendar_main_month(state: &CalendarState) -> Element<'_, CalendarMessage> {
    container(calendar_month::month_view(
        &state.month_grid,
        CalendarMessage::SelectDate,
        |id| CalendarMessage::EventClicked(id.to_string()),
    ))
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::ContainerClass::Content.style())
    .into()
}

/// Day / Work Week / Week time grid main content area.
fn calendar_main_time_grid(state: &CalendarState) -> Element<'_, CalendarMessage> {
    container(calendar_time_grid::time_grid_view(
        &state.time_grid_config,
        |id| CalendarMessage::EventClicked(id.to_string()),
        CalendarMessage::SelectSlot,
    ))
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::ContainerClass::Content.style())
    .into()
}
