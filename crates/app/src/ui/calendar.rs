//! Calendar view skeleton: layout, state, and messages.
//!
//! When the app is in Calendar mode, this module renders the two-panel
//! calendar layout: a sidebar (mini-month, view switcher, calendar list)
//! and a main content area dispatched by the active view.
//! Includes event detail popover and creation/editing overlays.

use chrono::{Datelike, Local, NaiveDate, Weekday};
use iced::widget::{
    button, column, container, mouse_area, row, scrollable, text, text_input, Space,
};
use iced::{Alignment, Element, Length};

use super::calendar_month;
use super::calendar_time_grid;
use super::layout::*;
use super::theme;

// ── Calendar view enum ─────────────────────────────────

/// Which calendar view is active in the main content area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarView {
    Day,
    WorkWeek,
    Week,
    Month,
}

impl CalendarView {
    pub fn label(self) -> &'static str {
        match self {
            Self::Day => "D",
            Self::WorkWeek => "WW",
            Self::Week => "W",
            Self::Month => "M",
        }
    }
}

// ── Overlay state ─────────────────────────────────────

/// Which overlay is currently visible on the calendar.
#[derive(Debug, Clone)]
pub enum CalendarOverlay {
    /// No overlay.
    None,
    /// Viewing event details (read-only).
    EventDetail { event: CalendarEventData },
    /// Editing or creating an event.
    EventEditor {
        /// The event being edited. Fields are mutated as the user types.
        event: CalendarEventData,
        /// Whether this is a new event (true) or editing existing (false).
        is_new: bool,
    },
    /// Delete confirmation dialog.
    ConfirmDelete {
        event_id: String,
        title: String,
    },
}

/// Data for a calendar event in the UI layer.
///
/// Used for both detail display and editor form state.
#[derive(Debug, Clone)]
pub struct CalendarEventData {
    /// DB row id. `None` for new events that haven't been saved.
    pub id: Option<String>,
    pub title: String,
    pub start_date: NaiveDate,
    pub start_hour: u32,
    pub start_minute: u32,
    pub end_hour: u32,
    pub end_minute: u32,
    pub all_day: bool,
    pub location: String,
    pub description: String,
    pub calendar_id: Option<String>,
}

impl CalendarEventData {
    /// Build a blank event pre-filled with the given date/hour.
    pub fn new_at(date: NaiveDate, hour: u32) -> Self {
        // If starting at 23, end at 23:59 (can't wrap to next day in V1).
        let (end_hour, end_minute) = if hour >= 23 {
            (23, 59)
        } else {
            (hour + 1, 0)
        };
        Self {
            id: None,
            title: String::new(),
            start_date: date,
            start_hour: hour,
            start_minute: 0,
            end_hour,
            end_minute,
            all_day: false,
            location: String::new(),
            description: String::new(),
            calendar_id: None,
        }
    }
}

// ── Calendar state ─────────────────────────────────────

/// Persistent calendar state (survives mode switches).
pub struct CalendarState {
    /// The currently selected/focused date.
    pub selected_date: NaiveDate,
    /// The selected time slot hour (for event creation pre-fill).
    pub selected_hour: Option<u32>,
    /// The active view (day/work-week/week/month).
    pub active_view: CalendarView,
    /// Which month is displayed in the mini-month sidebar.
    pub mini_month_year: i32,
    pub mini_month_month: u32,
    /// Start-of-week preference.
    pub week_start: Weekday,
    /// Cached month grid data for the month view (rebuilt on state change).
    pub month_grid: calendar_month::MonthGridData,
    /// Cached time grid config for day/work-week/week views.
    pub time_grid_config: calendar_time_grid::TimeGridConfig,
    /// Current overlay (detail view, editor, delete confirm, or none).
    pub overlay: CalendarOverlay,
    /// Cached events from the DB. Reloaded after CRUD operations.
    pub events: Vec<calendar_time_grid::TimeGridEvent>,
}

impl CalendarState {
    pub fn new() -> Self {
        let today = Local::now().date_naive();
        let month_grid = calendar_month::build_month_grid(
            today.year(),
            today.month(),
            &[],
            Weekday::Mon,
            today,
        );
        let time_grid_config =
            calendar_time_grid::build_day_view(today, &[], today);
        Self {
            selected_date: today,
            selected_hour: None,
            active_view: CalendarView::Month,
            mini_month_year: today.year(),
            mini_month_month: today.month(),
            week_start: Weekday::Mon,
            month_grid,
            time_grid_config,
            overlay: CalendarOverlay::None,
            events: Vec::new(),
        }
    }

    /// Navigate the mini-month to the previous month.
    pub fn prev_month(&mut self) {
        if self.mini_month_month == 1 {
            self.mini_month_month = 12;
            self.mini_month_year -= 1;
        } else {
            self.mini_month_month -= 1;
        }
        self.rebuild_view_data();
    }

    /// Navigate the mini-month to the next month.
    pub fn next_month(&mut self) {
        if self.mini_month_month == 12 {
            self.mini_month_month = 1;
            self.mini_month_year += 1;
        } else {
            self.mini_month_month += 1;
        }
        self.rebuild_view_data();
    }

    /// Jump to today, updating both selected date and mini-month.
    pub fn go_to_today(&mut self) {
        let today = Local::now().date_naive();
        self.selected_date = today;
        self.mini_month_year = today.year();
        self.mini_month_month = today.month();
        self.rebuild_view_data();
    }

    /// Rebuild cached view data from `self.events`.
    /// Call after any state change (date, view, events loaded).
    pub fn rebuild_view_data(&mut self) {
        let events = &self.events;
        let today = Local::now().date_naive();

        // Convert to month events for the month grid.
        let month_events: Vec<calendar_month::MonthEvent> = events
            .iter()
            .map(|e| calendar_month::MonthEvent {
                id: e.id.clone(),
                title: e.title.clone(),
                start_time: e.start_time,
                end_time: e.end_time,
                all_day: e.all_day,
                color: e.color.clone(),
            })
            .collect();

        self.month_grid = calendar_month::build_month_grid(
            self.mini_month_year,
            self.mini_month_month,
            &month_events,
            self.week_start,
            today,
        );

        self.time_grid_config = match self.active_view {
            CalendarView::Day => {
                calendar_time_grid::build_day_view(self.selected_date, events, today)
            }
            CalendarView::WorkWeek => {
                calendar_time_grid::build_work_week_view(
                    self.selected_date,
                    events,
                    today,
                )
            }
            CalendarView::Week => calendar_time_grid::build_week_view(
                self.selected_date,
                events,
                today,
                self.week_start,
            ),
            CalendarView::Month => {
                // Keep existing config for non-grid views.
                calendar_time_grid::build_day_view(self.selected_date, events, today)
            }
        };
    }
}

// ── Messages ───────────────────────────────────────────

/// Which field changed in the event editor form.
#[derive(Debug, Clone)]
pub enum EventField {
    Title(String),
    Location(String),
    Description(String),
    StartHour(String),
    StartMinute(String),
    EndHour(String),
    EndMinute(String),
    AllDay(bool),
}

#[derive(Debug, Clone)]
pub enum CalendarMessage {
    /// A date was clicked in the mini-month or main view.
    SelectDate(NaiveDate),
    /// A time slot was clicked in day/week views (for event creation pre-fill).
    SelectSlot(NaiveDate, u32),
    /// Switch the active calendar view.
    SetView(CalendarView),
    /// Navigate mini-month backward.
    PrevMonth,
    /// Navigate mini-month forward.
    NextMonth,
    /// Jump to today.
    Today,
    /// An event was clicked (event ID).
    EventClicked(String),
    /// Close any open overlay.
    CloseOverlay,
    /// Open the event editor. `None` = create new event.
    OpenEventEditor(Option<CalendarEventData>),
    /// A field in the event editor changed.
    EventFieldChanged(EventField),
    /// Save the event (create or update).
    SaveEvent,
    /// Async save completed.
    EventSaved(Result<(), String>),
    /// Start deleting an event (shows confirmation).
    ConfirmDeleteEvent(String, String),
    /// User confirmed deletion.
    DeleteEvent(String),
    /// Async delete completed.
    EventDeleted(Result<(), String>),
    /// Create a new event (from command palette or UI action).
    CreateEvent,
    /// Event detail was loaded from DB after clicking an event.
    EventLoaded(Result<CalendarEventData, String>),
    /// Calendar events loaded from DB for view rendering.
    EventsLoaded(Result<Vec<calendar_time_grid::TimeGridEvent>, String>),
}

// ── View ───────────────────────────────────────────────

/// Render the full calendar layout (sidebar + main area + overlay).
///
/// Returns an `Element<CalendarMessage>` — the parent maps this to the
/// top-level app Message.
pub fn calendar_layout(state: &CalendarState) -> Element<'_, CalendarMessage> {
    let sidebar = calendar_sidebar(state);
    let main_view = calendar_main_view(state);

    let base = row![sidebar, main_view]
        .height(Length::Fill);

    // If an overlay is active, stack it on top with a backdrop.
    match &state.overlay {
        CalendarOverlay::None => base.into(),
        CalendarOverlay::EventDetail { event } => {
            overlay_stack(base.into(), event_detail_card(event))
        }
        CalendarOverlay::EventEditor { event, is_new } => {
            overlay_stack(base.into(), event_editor_card(event, *is_new))
        }
        CalendarOverlay::ConfirmDelete { event_id, title } => {
            overlay_stack(
                base.into(),
                delete_confirm_card(event_id, title),
            )
        }
    }
}

/// Wrap a base layout with a modal overlay (backdrop + centered card).
fn overlay_stack<'a>(
    base: Element<'a, CalendarMessage>,
    card: Element<'a, CalendarMessage>,
) -> Element<'a, CalendarMessage> {
    let backdrop = mouse_area(
        container("")
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::ContainerClass::ModalBackdrop.style()),
    )
    .on_press(CalendarMessage::CloseOverlay);

    let centered = container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill);

    iced::widget::stack![base, backdrop, centered].into()
}

// ── Event detail card ──────────────────────────────────

/// Read-only event detail popover (rendered as a centered modal).
fn event_detail_card(event: &CalendarEventData) -> Element<'_, CalendarMessage> {
    let mut content = column![].spacing(SPACE_SM);

    // Title row with expand/edit button
    let title_text = if event.title.is_empty() {
        "(Untitled event)"
    } else {
        &event.title
    };
    let title_row = row![
        container(
            text(title_text)
                .size(TEXT_HEADING)
                .font(crate::font::text_semibold()),
        )
        .width(Length::Fill),
    ]
    .align_y(Alignment::Center);
    content = content.push(title_row);

    // Time
    let time_label = format_event_time_range(event);
    content = content.push(
        text(time_label)
            .size(TEXT_MD)
            .style(text::secondary),
    );

    // Location (hidden if empty)
    if !event.location.is_empty() {
        content = content.push(
            text(&event.location)
                .size(TEXT_MD)
                .style(text::secondary),
        );
    }

    // Description (hidden if empty)
    if !event.description.is_empty() {
        content = content.push(
            text(&event.description)
                .size(TEXT_SM)
                .style(theme::TextClass::Muted.style()),
        );
    }

    // Action buttons: Edit, Delete
    let edit_btn = button(text("Edit").size(TEXT_SM))
        .on_press(CalendarMessage::OpenEventEditor(Some(event.clone())))
        .padding(PAD_BUTTON)
        .style(theme::ButtonClass::Ghost.style());

    let delete_btn = if let Some(ref id) = event.id {
        button(text("Delete").size(TEXT_SM))
            .on_press(CalendarMessage::ConfirmDeleteEvent(
                id.clone(),
                event.title.clone(),
            ))
            .padding(PAD_BUTTON)
            .style(theme::ButtonClass::Ghost.style())
    } else {
        button(text("Delete").size(TEXT_SM))
            .padding(PAD_BUTTON)
            .style(theme::ButtonClass::Ghost.style())
    };

    content = content.push(Space::new().height(SPACE_XS));
    content = content.push(
        row![edit_btn, delete_btn].spacing(SPACE_XS),
    );

    let scrollable_content = scrollable(content)
        .height(Length::Shrink);

    container(scrollable_content)
        .width(Length::Fixed(CALENDAR_OVERLAY_WIDTH))
        .max_height(CALENDAR_OVERLAY_MAX_HEIGHT)
        .padding(PAD_CARD)
        .style(theme::ContainerClass::Elevated.style())
        .into()
}

// ── Event editor card ──────────────────────────────────

/// Event creation/editing form (rendered as a centered modal).
fn event_editor_card(
    event: &CalendarEventData,
    is_new: bool,
) -> Element<'_, CalendarMessage> {
    let heading = if is_new { "New Event" } else { "Edit Event" };

    let mut content = column![].spacing(SPACE_SM);

    // Heading
    content = content.push(
        text(heading)
            .size(TEXT_HEADING)
            .font(crate::font::text_semibold()),
    );

    // Title
    content = content.push(form_field(
        "Title",
        text_input("Event title", &event.title)
            .on_input(|s| CalendarMessage::EventFieldChanged(EventField::Title(s)))
            .padding(PAD_INPUT)
            .size(TEXT_MD)
            .into(),
    ));

    // Date (read-only display for V1)
    let date_str = format!(
        "{}, {} {}, {}",
        weekday_short(event.start_date.weekday()),
        month_short(event.start_date.month()),
        event.start_date.day(),
        event.start_date.year(),
    );
    content = content.push(form_field(
        "Date",
        text(date_str).size(TEXT_MD).into(),
    ));

    // Time row (start hour:minute - end hour:minute)
    if !event.all_day {
        let time_row = time_input_row(event);
        content = content.push(form_field("Time", time_row));
    }

    // All-day toggle
    let all_day_label = if event.all_day { "All day: Yes" } else { "All day: No" };
    let all_day_btn = button(text(all_day_label).size(TEXT_SM))
        .on_press(CalendarMessage::EventFieldChanged(
            EventField::AllDay(!event.all_day),
        ))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());
    content = content.push(all_day_btn);

    // Location
    content = content.push(form_field(
        "Location",
        text_input("Location (optional)", &event.location)
            .on_input(|s| CalendarMessage::EventFieldChanged(EventField::Location(s)))
            .padding(PAD_INPUT)
            .size(TEXT_MD)
            .into(),
    ));

    // Description
    content = content.push(form_field(
        "Description",
        text_input("Description (optional)", &event.description)
            .on_input(|s| CalendarMessage::EventFieldChanged(EventField::Description(s)))
            .padding(PAD_INPUT)
            .size(TEXT_MD)
            .into(),
    ));

    // Action buttons
    let save_btn = button(text("Save").size(TEXT_SM))
        .on_press(CalendarMessage::SaveEvent)
        .padding(PAD_BUTTON)
        .style(theme::ButtonClass::Nav { active: true }.style());

    let cancel_btn = button(text("Cancel").size(TEXT_SM))
        .on_press(CalendarMessage::CloseOverlay)
        .padding(PAD_BUTTON)
        .style(theme::ButtonClass::Ghost.style());

    content = content.push(Space::new().height(SPACE_XS));
    content = content.push(
        row![save_btn, cancel_btn].spacing(SPACE_XS),
    );

    let scrollable_content = scrollable(content)
        .height(Length::Shrink);

    container(scrollable_content)
        .width(Length::Fixed(CALENDAR_OVERLAY_WIDTH))
        .max_height(CALENDAR_OVERLAY_MAX_HEIGHT)
        .padding(PAD_CARD)
        .style(theme::ContainerClass::Elevated.style())
        .into()
}

/// A labeled form field row: label on left, widget on right.
fn form_field<'a>(
    label: &'a str,
    widget: Element<'a, CalendarMessage>,
) -> Element<'a, CalendarMessage> {
    row![
        container(
            text(label)
                .size(TEXT_SM)
                .style(theme::TextClass::Muted.style()),
        )
        .width(Length::Fixed(CALENDAR_FORM_LABEL_WIDTH))
        .height(CALENDAR_FORM_ROW_HEIGHT)
        .align_y(Alignment::Center),
        container(widget)
            .width(Length::Fill)
            .align_y(Alignment::Center),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center)
    .into()
}

/// Time input row with start and end hour:minute text inputs.
fn time_input_row(event: &CalendarEventData) -> Element<'_, CalendarMessage> {
    let start_h = text_input("HH", &format!("{}", event.start_hour))
        .on_input(|s| CalendarMessage::EventFieldChanged(EventField::StartHour(s)))
        .padding(PAD_INPUT)
        .width(Length::Fixed(48.0))
        .size(TEXT_MD);

    let start_m = text_input("MM", &format!("{:02}", event.start_minute))
        .on_input(|s| CalendarMessage::EventFieldChanged(EventField::StartMinute(s)))
        .padding(PAD_INPUT)
        .width(Length::Fixed(48.0))
        .size(TEXT_MD);

    let end_h = text_input("HH", &format!("{}", event.end_hour))
        .on_input(|s| CalendarMessage::EventFieldChanged(EventField::EndHour(s)))
        .padding(PAD_INPUT)
        .width(Length::Fixed(48.0))
        .size(TEXT_MD);

    let end_m = text_input("MM", &format!("{:02}", event.end_minute))
        .on_input(|s| CalendarMessage::EventFieldChanged(EventField::EndMinute(s)))
        .padding(PAD_INPUT)
        .width(Length::Fixed(48.0))
        .size(TEXT_MD);

    row![
        start_h,
        text(":").size(TEXT_MD),
        start_m,
        text("\u{2013}").size(TEXT_MD),
        end_h,
        text(":").size(TEXT_MD),
        end_m,
    ]
    .spacing(SPACE_XXS)
    .align_y(Alignment::Center)
    .into()
}

// ── Delete confirmation card ───────────────────────────

/// Confirmation dialog before deleting an event.
fn delete_confirm_card<'a>(
    event_id: &str,
    title: &str,
) -> Element<'a, CalendarMessage> {
    let display_title = if title.is_empty() { "(Untitled)" } else { title };
    let id = event_id.to_string();

    let content = column![
        text("Delete Event")
            .size(TEXT_HEADING)
            .font(crate::font::text_semibold()),
        Space::new().height(SPACE_XS),
        text(format!("Delete \"{display_title}\"? This cannot be undone."))
            .size(TEXT_MD),
        Space::new().height(SPACE_MD),
        row![
            button(text("Delete").size(TEXT_SM))
                .on_press(CalendarMessage::DeleteEvent(id))
                .padding(PAD_BUTTON)
                .style(theme::ButtonClass::Nav { active: true }.style()),
            button(text("Cancel").size(TEXT_SM))
                .on_press(CalendarMessage::CloseOverlay)
                .padding(PAD_BUTTON)
                .style(theme::ButtonClass::Ghost.style()),
        ]
        .spacing(SPACE_XS),
    ]
    .spacing(SPACE_XXS);

    container(content)
        .width(Length::Fixed(CALENDAR_OVERLAY_WIDTH))
        .padding(PAD_CARD)
        .style(theme::ContainerClass::Elevated.style())
        .into()
}

// ── Calendar sidebar ───────────────────────────────────

/// Calendar sidebar: mini-month, view switcher, calendar list placeholder.
fn calendar_sidebar(state: &CalendarState) -> Element<'_, CalendarMessage> {
    let today = Local::now().date_naive();

    // Mini-month
    let mini = super::calendar_month::mini_month(
        state.mini_month_year,
        state.mini_month_month,
        Some(state.selected_date),
        today,
        state.week_start,
        |d| CalendarMessage::SelectDate(d),
        CalendarMessage::PrevMonth,
        CalendarMessage::NextMonth,
    );

    // View switcher buttons
    let view_switcher = view_switcher_row(state.active_view);

    // Today button
    let today_btn = button(
        text("Today")
            .size(TEXT_SM)
            .style(text::secondary),
    )
    .on_press(CalendarMessage::Today)
    .padding(PAD_ICON_BTN)
    .style(theme::ButtonClass::Ghost.style());

    // New event button
    let new_event_btn = button(
        text("+ New Event")
            .size(TEXT_SM)
            .style(text::primary),
    )
    .on_press(CalendarMessage::CreateEvent)
    .padding(PAD_ICON_BTN)
    .style(theme::ButtonClass::Ghost.style());

    // Calendar list placeholder
    let calendar_list = container(
        text("Calendars")
            .size(TEXT_SM)
            .style(theme::TextClass::Muted.style()),
    )
    .padding(SPACE_XS);

    let content = column![
        mini,
        Space::new().height(SPACE_XS),
        view_switcher,
        Space::new().height(SPACE_XXS),
        row![today_btn, new_event_btn].spacing(SPACE_XXS),
        Space::new().height(SPACE_SM),
        calendar_list,
    ]
    .spacing(0)
    .width(Length::Fill);

    container(content)
        .width(SIDEBAR_MIN_WIDTH)
        .height(Length::Fill)
        .padding(PAD_SIDEBAR)
        .style(theme::ContainerClass::Sidebar.style())
        .into()
}

/// Row of view switcher buttons: D, WW, W, M.
fn view_switcher_row(active: CalendarView) -> Element<'static, CalendarMessage> {
    let views = [
        CalendarView::Day,
        CalendarView::WorkWeek,
        CalendarView::Week,
        CalendarView::Month,
    ];

    let mut r = row![].spacing(SPACE_XXS);
    for v in views {
        let is_active = v == active;
        let style = if is_active {
            theme::ButtonClass::Nav { active: true }.style()
        } else {
            theme::ButtonClass::Ghost.style()
        };

        r = r.push(
            button(
                text(v.label())
                    .size(TEXT_SM)
                    .style(if is_active { text::primary } else { text::secondary }),
            )
            .on_press(CalendarMessage::SetView(v))
            .padding(PAD_ICON_BTN)
            .style(style),
        );
    }

    r.into()
}

/// Calendar main content area: dispatches to the appropriate view.
fn calendar_main_view(state: &CalendarState) -> Element<'_, CalendarMessage> {
    match state.active_view {
        CalendarView::Month => calendar_main_month(state),
        _ => calendar_main_time_grid(state),
    }
}

/// Month view main content area.
fn calendar_main_month(
    state: &CalendarState,
) -> Element<'_, CalendarMessage> {
    container(
        calendar_month::month_view(
            &state.month_grid,
            |d| CalendarMessage::SelectDate(d),
            |id| CalendarMessage::EventClicked(id.to_string()),
        ),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::ContainerClass::Content.style())
    .into()
}

/// Day / Work Week / Week time grid main content area.
fn calendar_main_time_grid(
    state: &CalendarState,
) -> Element<'_, CalendarMessage> {
    container(calendar_time_grid::time_grid_view(
        &state.time_grid_config,
        |id| CalendarMessage::EventClicked(id.to_string()),
        |date, hour| CalendarMessage::SelectSlot(date, hour),
    ))
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::ContainerClass::Content.style())
    .into()
}

// ── Helpers ────────────────────────────────────────────

/// Format time range like "10:00 - 11:30" or "All day".
fn format_event_time_range(event: &CalendarEventData) -> String {
    if event.all_day {
        return "All day".to_string();
    }
    format!(
        "{:02}:{:02} \u{2013} {:02}:{:02}",
        event.start_hour, event.start_minute, event.end_hour, event.end_minute,
    )
}

fn weekday_short(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "Mon",
        Weekday::Tue => "Tue",
        Weekday::Wed => "Wed",
        Weekday::Thu => "Thu",
        Weekday::Fri => "Fri",
        Weekday::Sat => "Sat",
        Weekday::Sun => "Sun",
    }
}

fn month_short(month: u32) -> &'static str {
    match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "???",
    }
}
