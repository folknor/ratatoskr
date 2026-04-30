use chrono::NaiveDate;

use crate::ui::calendar_time_grid;

use super::types::{CalendarEventData, CalendarListEntry, CalendarView};

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
    /// Calendar selection carrying both calendar and account ownership.
    /// `account_id` comes from `CalendarListEntry.account_id` at selection
    /// time - not reconstructed from a later lookup.
    CalendarSelected {
        calendar_id: Option<String>,
        account_id: Option<String>,
    },
    Timezone(Option<String>),
    Availability(Option<String>),
    Visibility(Option<String>),
    RecurrenceRule(Option<String>),
}

/// Identifies a text field in the event editor for undo/redo.
#[derive(Debug, Clone, Copy)]
pub enum EventTextField {
    Title,
    Location,
    Description,
}

#[derive(Debug, Clone)]
pub enum CalendarMessage {
    /// A date was clicked in the mini-month or main view.
    SelectDate(NaiveDate),
    /// A time slot was clicked in day/week views (for event creation pre-fill).
    SelectSlot(NaiveDate, u32),
    /// A time slot was double-clicked - open event creation dialog.
    DoubleClickSlot(NaiveDate, u32),
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
    /// Close the active popover.
    ClosePopover,
    /// Close the active modal.
    CloseModal,
    /// Expand the event-detail popover into a full modal.
    ExpandPopoverToModal,
    /// Open the event editor. `None` = create new event.
    OpenEventEditor(Option<CalendarEventData>),
    /// A field in the event editor changed.
    EventFieldChanged(EventField),
    /// Undo the last edit to a text field in the event editor.
    EventFieldUndo(EventTextField),
    /// Redo a previously undone edit to a text field in the event editor.
    EventFieldRedo(EventTextField),
    /// Save the event (create or update).
    SaveEvent,
    /// Async save completed.
    EventSaved(Result<(), String>),
    /// Start deleting an event (shows confirmation).
    ConfirmDeleteEvent {
        event_id: String,
        title: String,
        account_id: Option<String>,
    },
    /// User confirmed deletion. Identity is read from workflow state
    /// (ConfirmingDelete), not from this message.
    DeleteEvent,
    /// User confirmed discarding unsaved editor changes.
    DiscardChanges,
    /// Async delete completed.
    EventDeleted(Result<(), String>),
    /// Create a new event (from command palette or UI action).
    CreateEvent,
    /// Event detail was loaded from DB after clicking an event.
    EventLoaded(Result<CalendarEventData, String>),
    /// Calendar events loaded from DB for view rendering.
    /// The token is a generation guard - stale results are discarded.
    EventsLoaded(
        rtsk::generation::GenerationToken<rtsk::generation::Calendar>,
        Result<Vec<calendar_time_grid::TimeGridEvent>, String>,
    ),
    /// Switch back to mail mode.
    SwitchToMail,
    /// No-op event sink for modal blocker (iced requires on_press to capture).
    Noop,
    /// Pop out the calendar into a separate window.
    PopOutCalendar,
    /// Toggle visibility of a calendar (checkbox in sidebar).
    ToggleCalendarVisibility(String, bool),
    /// Calendars loaded from DB for sidebar list.
    /// The token is a generation guard - stale results are discarded.
    CalendarsLoaded(
        rtsk::generation::GenerationToken<rtsk::generation::Calendar>,
        Result<Vec<CalendarListEntry>, String>,
    ),
}
