use rtsk::db::queries_extra::calendars::{
    LocalCalendarEventParams, create_calendar_event_sync, delete_calendar_event_sync,
    expand_view_events, get_calendar_event_sync, get_event_attendees_sync,
    get_event_reminders_sync, load_calendars_for_sidebar_sync, load_view_event_rows_sync,
    set_calendar_visibility_sync, update_calendar_event_sync,
};

use super::connection::Db;
use super::types::*;

impl Db {
    /// Load a single calendar event by its DB id.
    pub async fn get_calendar_event(
        &self,
        event_id: String,
    ) -> Result<Option<CalendarEvent>, String> {
        self.with_conn(move |conn| {
            let core_event = get_calendar_event_sync(conn, &event_id)?;
            Ok(core_event.map(|ev| CalendarEvent {
                id: ev.id,
                summary: ev.summary,
                description: ev.description,
                location: ev.location,
                start_time: ev.start_time,
                end_time: ev.end_time,
                is_all_day: ev.is_all_day != 0,
                calendar_id: ev.calendar_id,
                account_id: ev.account_id,
                timezone: ev.timezone,
                recurrence_rule: ev.recurrence_rule,
                organizer_name: ev.organizer_name,
                organizer_email: ev.organizer_email,
                rsvp_status: ev.rsvp_status,
                availability: ev.availability,
                visibility: ev.visibility,
            }))
        })
        .await
    }

    /// Create a new calendar event. Returns the new event's id.
    #[allow(dead_code)] // Calendar CRUD wrappers; calendar UI calls core directly for now.
    pub async fn create_calendar_event(
        &self,
        params: LocalCalendarEventParams,
    ) -> Result<String, String> {
        self.with_write_conn(move |conn| create_calendar_event_sync(conn, &params))
            .await
    }

    /// Update an existing calendar event.
    #[allow(dead_code)] // see create_calendar_event above
    pub async fn update_calendar_event(
        &self,
        event_id: String,
        params: LocalCalendarEventParams,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| update_calendar_event_sync(conn, &event_id, &params))
            .await
    }

    /// Load calendar events for view rendering, clipped to a time window.
    ///
    /// Window is provided by the caller (the active view's date range) so
    /// the SQL filter actually bounds the result set. The split into
    /// "load rows under the connection mutex" + "expand outside" keeps the
    /// CPU-heavy RRULE walk off the lock - sync workers, IPC, search, and
    /// the body store no longer block while expansion runs. (Round 3 #3.)
    pub async fn load_calendar_events_for_view(
        &self,
        window_start: i64,
        window_end: i64,
    ) -> Result<Vec<crate::ui::calendar_time_grid::TimeGridEvent>, String> {
        let rows = self
            .with_conn(move |conn| load_view_event_rows_sync(conn, window_start, window_end))
            .await?;
        // Expand off the lock. Pure CPU work over the loaded Vec.
        let core_events = expand_view_events(rows, window_start, window_end);
        Ok(core_events
            .into_iter()
            .map(|ev| crate::ui::calendar_time_grid::TimeGridEvent {
                id: ev.id,
                title: ev.title,
                start_time: ev.start_time,
                end_time: ev.end_time,
                all_day: ev.all_day,
                color: ev.color,
                calendar_name: ev.calendar_name,
                location: ev.location,
                recurrence_rule: ev.recurrence_rule,
                calendar_id: ev.calendar_id,
                account_id: ev.account_id,
                organizer_name: ev.organizer_name,
                organizer_email: ev.organizer_email,
                rsvp_status: ev.rsvp_status,
                description: ev.description,
                availability: ev.availability,
                visibility: ev.visibility,
                timezone: ev.timezone,
            })
            .collect())
    }

    /// Delete a calendar event by id.
    #[allow(dead_code)] // see create_calendar_event above
    pub async fn delete_calendar_event(&self, event_id: String) -> Result<(), String> {
        self.with_write_conn(move |conn| delete_calendar_event_sync(conn, &event_id))
            .await
    }

    /// Load attendees for a given event.
    pub async fn get_event_attendees(
        &self,
        account_id: String,
        event_id: String,
    ) -> Result<Vec<crate::ui::calendar::AttendeeEntry>, String> {
        self.with_conn(move |conn| {
            Ok(get_event_attendees_sync(conn, &account_id, &event_id)?
                .into_iter()
                .map(|row| crate::ui::calendar::AttendeeEntry {
                    email: row.email,
                    name: row.name,
                    rsvp_status: row
                        .rsvp_status
                        .unwrap_or_else(|| "needs-action".to_string()),
                    is_organizer: row.is_organizer != 0,
                })
                .collect())
        })
        .await
    }

    /// Load reminders for a given event.
    pub async fn get_event_reminders(
        &self,
        account_id: String,
        event_id: String,
    ) -> Result<Vec<crate::ui::calendar::ReminderEntry>, String> {
        self.with_conn(move |conn| {
            Ok(get_event_reminders_sync(conn, &account_id, &event_id)?
                .into_iter()
                .map(|row| crate::ui::calendar::ReminderEntry {
                    minutes_before: row.minutes_before,
                    method: row.method.unwrap_or_else(|| "popup".to_string()),
                })
                .collect())
        })
        .await
    }

    /// Load all calendars for the sidebar list.
    pub async fn load_calendars_for_sidebar(
        &self,
    ) -> Result<Vec<crate::ui::calendar::CalendarListEntry>, String> {
        self.with_conn(|conn| {
            Ok(load_calendars_for_sidebar_sync(conn)?
                .into_iter()
                .map(|row| crate::ui::calendar::CalendarListEntry {
                    id: row.id,
                    account_id: row.account_id,
                    display_name: row.display_name.unwrap_or_else(|| "(Unnamed)".to_string()),
                    color: row.color.unwrap_or_else(|| "#3498db".to_string()),
                    is_visible: row.is_visible != 0,
                })
                .collect())
        })
        .await
    }

    /// Set calendar visibility.
    pub async fn set_calendar_visibility(
        &self,
        calendar_id: String,
        visible: bool,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| set_calendar_visibility_sync(conn, &calendar_id, visible))
        .await
    }
}
