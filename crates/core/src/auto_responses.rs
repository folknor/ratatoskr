//! Auto-response (vacation/out-of-office) read/write across providers.
//!
//! Unified model maps to:
//! - Exchange (Graph): `GET/PATCH /me/mailboxSettings` → `automaticRepliesSetting`
//! - Gmail: `GET/PUT /users/me/settings/vacation`
//! - JMAP: `VacationResponse/get` / `VacationResponse/set` (RFC 8621 §7)

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::db::DbState;

// ── Unified model ──────────────────────────────────────

/// Provider-agnostic auto-response configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoResponseConfig {
    pub enabled: bool,
    /// ISO 8601 datetime string (nullable = no schedule).
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    /// Exchange supports separate internal/external messages.
    /// Gmail and JMAP only use `external_message_html`.
    pub internal_message_html: Option<String>,
    pub external_message_html: Option<String>,
    /// Who receives auto-replies: "none", "contacts_only", "all".
    pub external_audience: ExternalAudience,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExternalAudience {
    None,
    ContactsOnly,
    All,
}

impl Default for ExternalAudience {
    fn default() -> Self {
        Self::All
    }
}

impl ExternalAudience {
    fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ContactsOnly => "contacts_only",
            Self::All => "all",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "none" => Self::None,
            "contacts_only" => Self::ContactsOnly,
            _ => Self::All,
        }
    }
}

// ── DB queries ─────────────────────────────────────────

/// Get the cached auto-response config for an account.
pub async fn db_get_auto_response(
    db: &DbState,
    account_id: String,
) -> Result<Option<AutoResponseConfig>, String> {
    db.with_conn(move |conn| {
        conn.query_row(
            "SELECT enabled, start_date, end_date, internal_message_html, \
             external_message_html, external_audience \
             FROM auto_responses WHERE account_id = ?1",
            params![account_id],
            |row| {
                Ok(AutoResponseConfig {
                    enabled: row.get::<_, i32>(0)? != 0,
                    start_date: row.get(1)?,
                    end_date: row.get(2)?,
                    internal_message_html: row.get(3)?,
                    external_message_html: row.get(4)?,
                    external_audience: ExternalAudience::parse(
                        &row.get::<_, String>(5).unwrap_or_default(),
                    ),
                })
            },
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            _ => Err(format!("get auto_response: {e}")),
        })
    })
    .await
}

/// Cache an auto-response config for an account (upsert).
pub async fn db_upsert_auto_response(
    db: &DbState,
    account_id: String,
    config: AutoResponseConfig,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO auto_responses \
             (account_id, enabled, start_date, end_date, \
              internal_message_html, external_message_html, \
              external_audience, last_synced_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, unixepoch()) \
             ON CONFLICT(account_id) DO UPDATE SET \
               enabled = ?2, start_date = ?3, end_date = ?4, \
               internal_message_html = ?5, external_message_html = ?6, \
               external_audience = ?7, last_synced_at = unixepoch()",
            params![
                account_id,
                config.enabled as i32,
                config.start_date,
                config.end_date,
                config.internal_message_html,
                config.external_message_html,
                config.external_audience.as_str(),
            ],
        )
        .map_err(|e| format!("upsert auto_response: {e}"))?;
        Ok(())
    })
    .await
}

// ── Exchange (Graph) ───────────────────────────────────

/// Fetch auto-response settings from Microsoft Graph.
/// `GET /me/mailboxSettings/automaticRepliesSetting`
pub async fn fetch_graph_auto_response(
    client: &ratatoskr_graph::client::GraphClient,
    db: &DbState,
) -> Result<AutoResponseConfig, String> {
    let resp: serde_json::Value = client
        .get_json("/me/mailboxSettings/automaticRepliesSetting", db)
        .await
        .map_err(|e| format!("Graph automaticRepliesSetting: {e}"))?;

    let status = resp["status"].as_str().unwrap_or("disabled");
    let enabled = status == "alwaysEnabled" || status == "scheduled";

    let audience = match resp["externalAudience"].as_str() {
        Some("none") => ExternalAudience::None,
        Some("contactsOnly") => ExternalAudience::ContactsOnly,
        _ => ExternalAudience::All,
    };

    Ok(AutoResponseConfig {
        enabled,
        start_date: resp["scheduledStartDateTime"]["dateTime"]
            .as_str()
            .map(str::to_string),
        end_date: resp["scheduledEndDateTime"]["dateTime"]
            .as_str()
            .map(str::to_string),
        internal_message_html: resp["internalReplyMessage"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        external_message_html: resp["externalReplyMessage"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        external_audience: audience,
    })
}

/// Update auto-response settings on Microsoft Graph.
/// `PATCH /me/mailboxSettings`
pub async fn push_graph_auto_response(
    client: &ratatoskr_graph::client::GraphClient,
    db: &DbState,
    config: &AutoResponseConfig,
) -> Result<(), String> {
    let status = if !config.enabled {
        "disabled"
    } else if config.start_date.is_some() && config.end_date.is_some() {
        "scheduled"
    } else {
        "alwaysEnabled"
    };

    let audience = match config.external_audience {
        ExternalAudience::None => "none",
        ExternalAudience::ContactsOnly => "contactsOnly",
        ExternalAudience::All => "all",
    };

    let mut setting = serde_json::json!({
        "status": status,
        "externalAudience": audience,
    });

    if let Some(ref msg) = config.internal_message_html {
        setting["internalReplyMessage"] = serde_json::json!({ "message": msg });
    }
    if let Some(ref msg) = config.external_message_html {
        setting["externalReplyMessage"] = serde_json::json!({ "message": msg });
    }
    if let Some(ref start) = config.start_date {
        setting["scheduledStartDateTime"] =
            serde_json::json!({ "dateTime": start, "timeZone": "UTC" });
    }
    if let Some(ref end) = config.end_date {
        setting["scheduledEndDateTime"] =
            serde_json::json!({ "dateTime": end, "timeZone": "UTC" });
    }

    let body = serde_json::json!({ "automaticRepliesSetting": setting });
    client
        .patch("/me/mailboxSettings", &body, db)
        .await
        .map_err(|e| format!("Graph PATCH mailboxSettings: {e}"))
}

// ── Gmail ──────────────────────────────────────────────

/// Fetch vacation settings from Gmail API.
/// `GET /users/me/settings/vacation`
pub async fn fetch_gmail_auto_response(
    client: &ratatoskr_gmail::client::GmailClient,
    db: &DbState,
) -> Result<AutoResponseConfig, String> {
    let resp: serde_json::Value = client
        .get("/settings/vacation", db)
        .await
        .map_err(|e| format!("Gmail vacation settings: {e}"))?;

    let enabled = resp["enableAutoReply"].as_bool().unwrap_or(false);

    // Gmail uses epoch milliseconds for start/end
    let start_date = resp["startTime"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .map(|ms| {
            chrono::DateTime::from_timestamp(ms / 1000, 0)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default()
        })
        .filter(|s| !s.is_empty());

    let end_date = resp["endTime"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .map(|ms| {
            chrono::DateTime::from_timestamp(ms / 1000, 0)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default()
        })
        .filter(|s| !s.is_empty());

    let audience = if resp["restrictToContacts"].as_bool().unwrap_or(false) {
        ExternalAudience::ContactsOnly
    } else if resp["restrictToDomain"].as_bool().unwrap_or(false) {
        // Domain restriction maps closest to ContactsOnly
        ExternalAudience::ContactsOnly
    } else {
        ExternalAudience::All
    };

    Ok(AutoResponseConfig {
        enabled,
        start_date,
        end_date,
        internal_message_html: None, // Gmail doesn't distinguish
        external_message_html: resp["responseBodyHtml"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                resp["responseBodyPlainText"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            }),
        external_audience: audience,
    })
}

/// Update vacation settings on Gmail.
/// `PUT /users/me/settings/vacation`
pub async fn push_gmail_auto_response(
    client: &ratatoskr_gmail::client::GmailClient,
    db: &DbState,
    config: &AutoResponseConfig,
) -> Result<(), String> {
    let mut body = serde_json::json!({
        "enableAutoReply": config.enabled,
    });

    if let Some(ref msg) = config.external_message_html {
        body["responseBodyHtml"] = serde_json::Value::String(msg.clone());
    }

    if let Some(ref start) = config.start_date {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(start) {
            body["startTime"] =
                serde_json::Value::String((dt.timestamp() * 1000).to_string());
        }
    }
    if let Some(ref end) = config.end_date {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(end) {
            body["endTime"] =
                serde_json::Value::String((dt.timestamp() * 1000).to_string());
        }
    }

    body["restrictToContacts"] =
        serde_json::Value::Bool(config.external_audience == ExternalAudience::ContactsOnly);

    let _resp: serde_json::Value = client
        .put("/settings/vacation", &body, db)
        .await
        .map_err(|e| format!("Gmail PUT vacation: {e}"))?;
    Ok(())
}

// ── JMAP ───────────────────────────────────────────────

/// Fetch vacation response from JMAP server.
/// Uses `jmap-client` VacationResponse/get with the singleton ID.
pub async fn fetch_jmap_auto_response(
    client: &ratatoskr_jmap::client::JmapClient,
) -> Result<AutoResponseConfig, String> {
    client.ensure_valid_token().await?;
    let inner = client.inner();

    let vr = inner
        .vacation_response_get(None::<Vec<jmap_client::vacation_response::Property>>)
        .await
        .map_err(|e| format!("VacationResponse/get: {e}"))?
        .ok_or_else(|| "VacationResponse/get: no singleton returned".to_string())?;

    let start_date = vr.from_date().map(|ts| {
        chrono::DateTime::from_timestamp(ts, 0)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default()
    });
    let end_date = vr.to_date().map(|ts| {
        chrono::DateTime::from_timestamp(ts, 0)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default()
    });

    Ok(AutoResponseConfig {
        enabled: vr.is_enabled(),
        start_date,
        end_date,
        internal_message_html: None,
        external_message_html: vr
            .html_body()
            .map(str::to_string)
            .or_else(|| vr.text_body().map(str::to_string)),
        external_audience: ExternalAudience::All, // JMAP has no audience control
    })
}

/// Update vacation response on JMAP server.
/// Uses `jmap-client` VacationResponse/set to update the singleton.
pub async fn push_jmap_auto_response(
    client: &ratatoskr_jmap::client::JmapClient,
    config: &AutoResponseConfig,
) -> Result<(), String> {
    client.ensure_valid_token().await?;
    let inner = client.inner();

    if config.enabled {
        let from_ts = config.start_date.as_ref().and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|dt| dt.timestamp())
        });
        let to_ts = config.end_date.as_ref().and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|dt| dt.timestamp())
        });

        // Use vacation_response_enable for the full update
        inner
            .vacation_response_enable(
                "", // subject (unused by most servers)
                None::<String>,
                config.external_message_html.clone(),
            )
            .await
            .map_err(|e| format!("VacationResponse/set (enable): {e}"))?;

        // Set dates if provided
        if from_ts.is_some() || to_ts.is_some() {
            inner
                .vacation_response_set_dates(from_ts, to_ts)
                .await
                .map_err(|e| format!("VacationResponse/set (dates): {e}"))?;
        }
    } else {
        inner
            .vacation_response_disable()
            .await
            .map_err(|e| format!("VacationResponse/set (disable): {e}"))?;
    }

    log::info!("[JMAP] Updated VacationResponse (enabled={})", config.enabled);
    Ok(())
}
