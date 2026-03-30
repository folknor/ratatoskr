use rtsk::db::queries_extra::calendars::{
    create_calendar_event_sync, delete_calendar_event_sync,
    get_calendar_event_sync, load_calendar_events_for_view_sync,
    update_calendar_event_sync, LocalCalendarEventParams,
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
    pub async fn create_calendar_event(
        &self,
        params: LocalCalendarEventParams,
    ) -> Result<String, String> {
        self.with_write_conn(move |conn| {
            create_calendar_event_sync(conn, &params)
        })
        .await
    }

    /// Update an existing calendar event.
    pub async fn update_calendar_event(
        &self,
        event_id: String,
        params: LocalCalendarEventParams,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            update_calendar_event_sync(conn, &event_id, &params)
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

    /// Load attendees for a given event.
    pub async fn get_event_attendees(
        &self,
        account_id: String,
        event_id: String,
    ) -> Result<Vec<crate::ui::calendar::AttendeeEntry>, String> {
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT email, name, rsvp_status, is_organizer \
                 FROM calendar_attendees \
                 WHERE account_id = ?1 AND event_id = ?2 \
                 ORDER BY is_organizer DESC, email ASC",
            ).map_err(|e| e.to_string())?;
            let rows = stmt.query_map(
                rusqlite::params![account_id, event_id],
                |row| {
                    Ok(crate::ui::calendar::AttendeeEntry {
                        email: row.get("email")?,
                        name: row.get("name")?,
                        rsvp_status: row.get::<_, Option<String>>("rsvp_status")?
                            .unwrap_or_else(|| "needs-action".to_string()),
                        is_organizer: row.get::<_, i64>("is_organizer")? != 0,
                    })
                },
            ).map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
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
            let mut stmt = conn.prepare(
                "SELECT minutes_before, method \
                 FROM calendar_reminders \
                 WHERE account_id = ?1 AND event_id = ?2 \
                 ORDER BY minutes_before ASC",
            ).map_err(|e| e.to_string())?;
            let rows = stmt.query_map(
                rusqlite::params![account_id, event_id],
                |row| {
                    Ok(crate::ui::calendar::ReminderEntry {
                        minutes_before: row.get("minutes_before")?,
                        method: row.get::<_, Option<String>>("method")?
                            .unwrap_or_else(|| "popup".to_string()),
                    })
                },
            ).map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
        })
        .await
    }

    /// Load all calendars for the sidebar list.
    pub async fn load_calendars_for_sidebar(
        &self,
    ) -> Result<Vec<crate::ui::calendar::CalendarListEntry>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, account_id, display_name, color, is_visible \
                 FROM calendars \
                 ORDER BY account_id, sort_order ASC, is_primary DESC, display_name ASC",
            ).map_err(|e| e.to_string())?;
            let rows = stmt.query_map([], |row| {
                Ok(crate::ui::calendar::CalendarListEntry {
                    id: row.get::<_, String>("id")?,
                    account_id: row.get::<_, String>("account_id")?,
                    display_name: row.get::<_, Option<String>>("display_name")?
                        .unwrap_or_else(|| "(Unnamed)".to_string()),
                    color: row.get::<_, Option<String>>("color")?
                        .unwrap_or_else(|| "#3498db".to_string()),
                    is_visible: row.get::<_, i64>("is_visible")? != 0,
                })
            }).map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
        })
        .await
    }

    /// Set calendar visibility.
    pub async fn set_calendar_visibility(
        &self,
        calendar_id: String,
        visible: bool,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            conn.execute(
                "UPDATE calendars SET is_visible = ?1, updated_at = unixepoch() WHERE id = ?2",
                rusqlite::params![visible as i64, calendar_id],
            ).map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
    }
}
