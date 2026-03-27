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
    /// IANA timezone for schedule dates (Graph only; defaults to UTC).
    pub start_date_tz: Option<String>,
    pub end_date_tz: Option<String>,
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
    /// Gmail `restrictToDomain = true` — replies only to same-domain senders.
    DomainOnly,
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
            Self::DomainOnly => "domain_only",
            Self::All => "all",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "none" => Self::None,
            "contacts_only" => Self::ContactsOnly,
            "domain_only" => Self::DomainOnly,
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
                    start_date_tz: None,
                    end_date_tz: None,
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

// ── Helpers ───────────────────────────────────────────

/// Normalize a .NET-style datetime (e.g. `"2026-03-25T08:00:00.0000000"`)
/// into proper RFC 3339 so that `chrono::DateTime::parse_from_rfc3339` works
/// on the stored value when pushing to other providers.
fn normalize_dotnet_datetime(s: &str) -> String {
    // Already valid RFC 3339 — return as-is.
    if chrono::DateTime::parse_from_rfc3339(s).is_ok() {
        return s.to_string();
    }
    // .NET format without offset: try parsing as NaiveDateTime, emit as UTC.
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
        return naive.and_utc().to_rfc3339();
    }
    // Fallback: append Z and hope for the best.
    let mut owned = s.to_string();
    if !owned.ends_with('Z')
        && !owned.contains('+')
        && !owned.rfind('-').is_some_and(|i| i > 10)
    {
        owned.push('Z');
    }
    owned
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
            .map(normalize_dotnet_datetime),
        end_date: resp["scheduledEndDateTime"]["dateTime"]
            .as_str()
            .map(normalize_dotnet_datetime),
        start_date_tz: resp["scheduledStartDateTime"]["timeZone"]
            .as_str()
            .map(str::to_string),
        end_date_tz: resp["scheduledEndDateTime"]["timeZone"]
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
        ExternalAudience::ContactsOnly | ExternalAudience::DomainOnly => "contactsOnly",
        ExternalAudience::All => "all",
    };

    let mut setting = serde_json::json!({
        "status": status,
        "externalAudience": audience,
    });

    if let Some(ref msg) = config.internal_message_html {
        setting["internalReplyMessage"] = serde_json::Value::String(msg.clone());
    }
    if let Some(ref msg) = config.external_message_html {
        setting["externalReplyMessage"] = serde_json::Value::String(msg.clone());
    }
    if let Some(ref start) = config.start_date {
        let tz = config.start_date_tz.as_deref().unwrap_or("UTC");
        setting["scheduledStartDateTime"] =
            serde_json::json!({ "dateTime": start, "timeZone": tz });
    }
    if let Some(ref end) = config.end_date {
        let tz = config.end_date_tz.as_deref().unwrap_or("UTC");
        setting["scheduledEndDateTime"] =
            serde_json::json!({ "dateTime": end, "timeZone": tz });
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
        ExternalAudience::DomainOnly
    } else {
        ExternalAudience::All
    };

    Ok(AutoResponseConfig {
        enabled,
        start_date,
        end_date,
        start_date_tz: None,
        end_date_tz: None,
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
    body["restrictToDomain"] =
        serde_json::Value::Bool(config.external_audience == ExternalAudience::DomainOnly);

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
        start_date_tz: None,
        end_date_tz: None,
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
///
/// All fields are set in a single JMAP request to avoid partial-update
/// failures (e.g. enabling the vacation without schedule dates).
pub async fn push_jmap_auto_response(
    client: &ratatoskr_jmap::client::JmapClient,
    config: &AutoResponseConfig,
) -> Result<(), String> {
    use jmap_client::vacation_response::VacationResponseSet;

    client.ensure_valid_token().await?;
    let inner = client.inner();

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

    // Build a single VacationResponse/set request with all fields so that
    // enabling + body + dates are applied atomically.
    let mut request = inner.build();
    let account_id = request.default_account_id().to_string();
    let mut set = VacationResponseSet::new(&account_id);
    set.update("singleton")
        .is_enabled(config.enabled)
        .subject(Some(""))
        .text_body(None::<String>)
        .html_body(config.external_message_html.clone())
        .from_date(from_ts)
        .to_date(to_ts);

    let handle = request
        .call(set)
        .map_err(|e| format!("VacationResponse/set build: {e}"))?;
    request
        .send_single(&handle)
        .await
        .map_err(|e| format!("VacationResponse/set: {e}"))?
        .updated("singleton")
        .map_err(|e| format!("VacationResponse/set: {e}"))?;

    log::info!("[JMAP] Updated VacationResponse (enabled={})", config.enabled);
    Ok(())
}
