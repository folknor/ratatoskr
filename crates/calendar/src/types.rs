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
    /// Whether the authenticated user can write to this calendar.
    /// Defaults to `true` for providers that don't expose a permission
    /// flag (Google personal, JMAP, CalDAV) - we currently only learn
    /// `false` from Microsoft Graph's `canEdit`.
    #[serde(default = "default_can_edit")]
    pub can_edit: bool,
}

fn default_can_edit() -> bool {
    true
}
