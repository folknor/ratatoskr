use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarInfoInput {
    pub remote_id: String,
    pub display_name: String,
    pub color: Option<String>,
    pub is_primary: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarInfoDto {
    pub remote_id: String,
    pub display_name: String,
    pub color: Option<String>,
    pub is_primary: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEventDto {
    pub remote_event_id: String,
    pub uid: Option<String>,
    pub etag: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start_time: i64,
    pub end_time: i64,
    pub is_all_day: bool,
    pub status: String,
    pub organizer_email: Option<String>,
    pub attendees_json: Option<String>,
    pub html_link: Option<String>,
    pub ical_data: Option<String>,
}

/// Backward-compatible alias: `CalendarEventInput` was previously a separate
/// struct with identical fields. Code that receives events from the frontend
/// can continue to use this name.
pub type CalendarEventInput = CalendarEventDto;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarSyncResultDto {
    pub created: Vec<CalendarEventDto>,
    pub updated: Vec<CalendarEventDto>,
    pub deleted_remote_ids: Vec<String>,
    pub new_sync_token: Option<String>,
    pub new_ctag: Option<String>,
}
