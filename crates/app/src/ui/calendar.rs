//! Calendar view skeleton: layout, state, and messages.
//!
//! When the app is in Calendar mode, this module renders the two-panel
//! calendar layout: a sidebar (mini-month, view switcher, calendar list)
//! and a main content area dispatched by the active view.

use chrono::{Datelike, Local, NaiveDate, Weekday};
use iced::widget::{button, column, container, row, text, Space};
use iced::{Element, Length};

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

// ── Calendar state ─────────────────────────────────────

/// Persistent calendar state (survives mode switches).
pub struct CalendarState {
    /// The currently selected/focused date.
    pub selected_date: NaiveDate,
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
            active_view: CalendarView::Month,
            mini_month_year: today.year(),
            mini_month_month: today.month(),
            week_start: Weekday::Mon,
            month_grid,
            time_grid_config,
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

    /// Rebuild cached view data after any state change.
    pub fn rebuild_view_data(&mut self) {
        let today = Local::now().date_naive();
        let events: &[calendar_time_grid::TimeGridEvent] = &[];

        self.month_grid = calendar_month::build_month_grid(
            self.mini_month_year,
            self.mini_month_month,
            &[],
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

#[derive(Debug, Clone)]
pub enum CalendarMessage {
    /// A date was clicked in the mini-month or main view.
    SelectDate(NaiveDate),
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
}

// ── View ───────────────────────────────────────────────

/// Render the full calendar layout (sidebar + main area).
///
/// Returns an `Element<CalendarMessage>` — the parent maps this to the
/// top-level app Message.
pub fn calendar_layout<'a>(state: &'a CalendarState) -> Element<'a, CalendarMessage> {
    let sidebar = calendar_sidebar(state);
    let main_view = calendar_main_view(state);

    row![sidebar, main_view]
        .height(Length::Fill)
        .into()
}

/// Calendar sidebar: mini-month, view switcher, calendar list placeholder.
fn calendar_sidebar<'a>(state: &'a CalendarState) -> Element<'a, CalendarMessage> {
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
        today_btn,
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
fn calendar_main_view<'a>(state: &'a CalendarState) -> Element<'a, CalendarMessage> {
    match state.active_view {
        CalendarView::Month => calendar_main_month(state),
        _ => calendar_main_time_grid(state),
    }
}

/// Month view main content area.
fn calendar_main_month<'a>(
    state: &'a CalendarState,
) -> Element<'a, CalendarMessage> {
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
fn calendar_main_time_grid<'a>(
    state: &'a CalendarState,
) -> Element<'a, CalendarMessage> {
    container(calendar_time_grid::time_grid_view(
        &state.time_grid_config,
        |id| CalendarMessage::EventClicked(id.to_string()),
        |date, _hour| CalendarMessage::SelectDate(date),
    ))
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::ContainerClass::Content.style())
    .into()
}
