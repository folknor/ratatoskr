use rusqlite::params;

use super::connection::Db;
use super::types::*;

impl Db {
    /// Load a single calendar event by its DB id.
    pub async fn get_calendar_event(
        &self,
        event_id: String,
    ) -> Result<Option<CalendarEvent>, String> {
        self.with_conn(move |conn| {
            let result = conn.query_row(
                "SELECT id, summary, description, location,
                        start_time, end_time, is_all_day, calendar_id
                 FROM calendar_events WHERE id = ?1",
                params![event_id],
                |row| {
                    Ok(CalendarEvent {
                        id: row.get("id")?,
                        summary: row.get("summary")?,
                        description: row.get("description")?,
                        location: row.get("location")?,
                        start_time: row.get("start_time")?,
                        end_time: row.get("end_time")?,
                        is_all_day: row.get::<_, i64>("is_all_day")? != 0,
                        calendar_id: row.get("calendar_id")?,
                    })
                },
            );
            match result {
                Ok(event) => Ok(Some(event)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.to_string()),
            }
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
            let id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO calendar_events
                    (id, account_id, google_event_id, summary, description,
                     location, start_time, end_time, is_all_day, status,
                     calendar_id)
                 VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8, 'confirmed', ?9)",
                params![
                    id,
                    account_id,
                    title,
                    description,
                    location,
                    start_time,
                    end_time,
                    is_all_day as i64,
                    calendar_id,
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(id)
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
            conn.execute(
                "UPDATE calendar_events SET
                    summary = ?2, description = ?3, location = ?4,
                    start_time = ?5, end_time = ?6, is_all_day = ?7,
                    calendar_id = ?8, updated_at = unixepoch()
                 WHERE id = ?1",
                params![
                    event_id,
                    title,
                    description,
                    location,
                    start_time,
                    end_time,
                    is_all_day as i64,
                    calendar_id,
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
    }

    /// Load all calendar events as TimeGridEvent for view rendering.
    pub async fn load_calendar_events_for_view(
        &self,
    ) -> Result<Vec<crate::ui::calendar_time_grid::TimeGridEvent>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT e.id, e.summary, e.start_time, e.end_time,
                            e.is_all_day, COALESCE(c.color, '#3498db') AS color,
                            c.display_name AS calendar_name
                     FROM calendar_events e
                     LEFT JOIN calendars c
                       ON c.account_id = e.account_id AND c.id = e.calendar_id
                     ORDER BY e.start_time ASC",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(crate::ui::calendar_time_grid::TimeGridEvent {
                        id: row.get::<_, String>("id")?,
                        title: row.get::<_, Option<String>>("summary")?
                            .unwrap_or_default(),
                        start_time: row.get("start_time")?,
                        end_time: row.get("end_time")?,
                        all_day: row.get::<_, i64>("is_all_day")? != 0,
                        color: row.get::<_, Option<String>>("color")?
                            .unwrap_or_else(|| "#3498db".to_string()),
                        calendar_name: row.get("calendar_name")?,
                    })
                })
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
    }

    /// Delete a calendar event by id.
    pub async fn delete_calendar_event(
        &self,
        event_id: String,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            conn.execute(
                "DELETE FROM calendar_events WHERE id = ?1",
                params![event_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
    }
}
