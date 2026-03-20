//! Calendar view skeleton: layout, state, and messages.
//!
//! When the app is in Calendar mode, this module renders the two-panel
//! calendar layout: a sidebar (mini-month, view switcher, calendar list)
//! and a main content area (placeholder for now, month view coming via
//! `calendar_month`).

use chrono::{Datelike, Local, NaiveDate, Weekday};
use iced::widget::{button, column, container, row, text, Space};
use iced::{Alignment, Element, Length};

use super::layout::*;
use super::theme;
use crate::icon;

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
}

impl CalendarState {
    pub fn new() -> Self {
        let today = Local::now().date_naive();
        Self {
            selected_date: today,
            active_view: CalendarView::Month,
            mini_month_year: today.year(),
            mini_month_month: today.month(),
            week_start: Weekday::Mon,
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
    }

    /// Navigate the mini-month to the next month.
    pub fn next_month(&mut self) {
        if self.mini_month_month == 12 {
            self.mini_month_month = 1;
            self.mini_month_year += 1;
        } else {
            self.mini_month_month += 1;
        }
    }

    /// Jump to today, updating both selected date and mini-month.
    pub fn go_to_today(&mut self) {
        let today = Local::now().date_naive();
        self.selected_date = today;
        self.mini_month_year = today.year();
        self.mini_month_month = today.month();
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

/// Calendar main content area: placeholder for now.
fn calendar_main_view<'a>(state: &'a CalendarState) -> Element<'a, CalendarMessage> {
    let view_label = match state.active_view {
        CalendarView::Day => "Day View",
        CalendarView::WorkWeek => "Work Week View",
        CalendarView::Week => "Week View",
        CalendarView::Month => "Month View",
    };

    let date_str = state.selected_date.format("%B %d, %Y").to_string();

    let placeholder = column![
        text(view_label)
            .size(TEXT_HEADING)
            .style(text::primary),
        text(date_str)
            .size(TEXT_LG)
            .style(text::secondary),
        Space::new().height(SPACE_LG),
        text("Calendar view coming soon")
            .size(TEXT_MD)
            .style(theme::TextClass::Muted.style()),
    ]
    .spacing(SPACE_XS)
    .align_x(Alignment::Center);

    container(placeholder)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .into()
}
