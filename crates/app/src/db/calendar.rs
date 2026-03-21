use ratatoskr_core::db::queries_extra::calendars::{
    create_calendar_event_sync, delete_calendar_event_sync,
    get_calendar_event_sync, load_calendar_events_for_view_sync,
    update_calendar_event_sync,
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
            }))
        })
        .await
    }

    /// Create a new calendar event. Returns the new event's id.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_calendar_event(
        &self,
        account_id: String,
        title: String,
        description: String,
        location: String,
        start_time: i64,
        end_time: i64,
        is_all_day: bool,
        calendar_id: Option<String>,
    ) -> Result<String, String> {
        self.with_write_conn(move |conn| {
            create_calendar_event_sync(
                conn,
                &account_id,
                &title,
                &description,
                &location,
                start_time,
                end_time,
                is_all_day,
                calendar_id.as_deref(),
            )
        })
        .await
    }

    /// Update an existing calendar event.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_calendar_event(
        &self,
        event_id: String,
        title: String,
        description: String,
        location: String,
        start_time: i64,
        end_time: i64,
        is_all_day: bool,
        calendar_id: Option<String>,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            update_calendar_event_sync(
                conn,
                &event_id,
                &title,
                &description,
                &location,
                start_time,
                end_time,
                is_all_day,
                calendar_id.as_deref(),
            )
        })
        .await
    }

    /// Load all calendar events as TimeGridEvent for view rendering.
    pub async fn load_calendar_events_for_view(
        &self,
    ) -> Result<Vec<crate::ui::calendar_time_grid::TimeGridEvent>, String> {
        self.with_conn(|conn| {
            let core_events = load_calendar_events_for_view_sync(conn)?;
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
                })
                .collect())
        })
        .await
    }

    /// Delete a calendar event by id.
    pub async fn delete_calendar_event(
        &self,
        event_id: String,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            delete_calendar_event_sync(conn, &event_id)
        })
        .await
    }
}
