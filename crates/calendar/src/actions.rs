//! Calendar event write path — create, update, delete through providers.
//!
//! These action functions live in the `calendar` crate (not `core::actions`)
//! because the calendar provider write APIs use typed clients (`GmailClient`,
//! `GraphClient`, `JmapClient`) that are not on the `ProviderOps` trait.
//! The `calendar` crate already depends on `core` (for `ActionContext`,
//! `ActionOutcome`, `DbState`) and has access to all provider write functions.
//! Adding `calendar` as a dependency of `core` would create a circular dep.

use ratatoskr_core::actions::{ActionContext, ActionOutcome};
use ratatoskr_core::db::DbState;
use ratatoskr_core::gmail::client::GmailClient;
use ratatoskr_core::graph::client::GraphClient;
use ratatoskr_core::jmap::client::JmapClient;

use super::google::{
    google_calendar_create_event_impl, google_calendar_delete_event_impl,
    google_calendar_update_event_impl,
};
use super::graph::{
    graph_calendar_create_event_impl, graph_calendar_delete_event_impl,
    graph_calendar_update_event_impl,
};
use super::caldav::{
    caldav_create_event_impl, caldav_delete_event_impl, caldav_update_event_impl,
};
use super::types::CalendarEventDto;

// ── Public types ─────────────────────────────────────────

/// Provider-agnostic input for calendar event create/update.
#[derive(Debug, Clone)]
pub struct CalendarEventInput {
    pub title: String,
    pub description: String,
    pub location: String,
    pub start_time: i64,
    pub end_time: i64,
    pub is_all_day: bool,
    pub timezone: Option<String>,
    pub recurrence_rule: Option<String>,
    pub availability: Option<String>,
    pub visibility: Option<String>,
}

// ── Provider resolution ──────────────────────────────────

enum CalendarProvider {
    Google(GmailClient),
    Graph(GraphClient),
    Jmap(JmapClient),
    CalDav { account_id: String },
}

/// Resolve the calendar provider for an account.
///
/// Same routing logic as `calendar_sync_account_impl`: checks
/// `calendar_provider` column first, falls back to `provider`.
async fn create_calendar_provider(
    db: &DbState,
    account_id: &str,
    encryption_key: [u8; 32],
) -> Result<CalendarProvider, String> {
    let aid = account_id.to_string();
    let db_clone = db.clone();
    let (provider, calendar_provider) = tokio::task::spawn_blocking(move || {
        let conn = db_clone.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        conn.query_row(
            "SELECT provider, calendar_provider FROM accounts WHERE id = ?1",
            rusqlite::params![aid],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .map_err(|e| format!("account lookup: {e}"))
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))??;

    let effective = calendar_provider.as_deref().unwrap_or(provider.as_str());

    match effective {
        "google_api" | "gmail_api" => {
            let client =
                GmailClient::from_account(db, account_id, encryption_key).await?;
            Ok(CalendarProvider::Google(client))
        }
        "graph" => {
            let client =
                GraphClient::from_account(db, account_id, encryption_key).await?;
            Ok(CalendarProvider::Graph(client))
        }
        "jmap" => {
            let client =
                JmapClient::from_account(db, account_id, &encryption_key).await?;
            Ok(CalendarProvider::Jmap(client))
        }
        "caldav" => Ok(CalendarProvider::CalDav {
            account_id: account_id.to_string(),
        }),
        other => Err(format!(
            "No calendar provider for account type: {other}"
        )),
    }
}

// ── JSON serialization for Google/Graph/CalDAV ───────────

/// Build a `serde_json::Value` payload from `CalendarEventInput`.
///
/// Uses field names that Google, Graph (`json_to_graph_event_create`),
/// and CalDAV (`parse_caldav_event_input`) all understand.
fn input_to_json(input: &CalendarEventInput) -> serde_json::Value {
    serde_json::json!({
        "summary": input.title,
        "description": input.description,
        "location": input.location,
        "start": input.start_time,
        "end": input.end_time,
        "isAllDay": input.is_all_day,
        "timezone": input.timezone,
        "recurrenceRule": input.recurrence_rule,
    })
}

// ── Provider dispatch helpers ────────────────────────────

async fn dispatch_create(
    provider: &CalendarProvider,
    ctx: &ActionContext,
    calendar_remote_id: &str,
    input: &CalendarEventInput,
) -> Result<CalendarEventDto, String> {
    let json = input_to_json(input);
    match provider {
        CalendarProvider::Google(client) => {
            google_calendar_create_event_impl(client, &ctx.db, calendar_remote_id, json).await
        }
        CalendarProvider::Graph(client) => {
            graph_calendar_create_event_impl(client, &ctx.db, calendar_remote_id, json).await
        }
        CalendarProvider::Jmap(client) => {
            let remote_id = ratatoskr_core::jmap::calendar_sync::create_event_remote(
                client,
                calendar_remote_id,
                &input.title,
                &input.description,
                &input.location,
                input.start_time,
                input.end_time,
                input.is_all_day,
            )
            .await?;
            Ok(CalendarEventDto {
                remote_event_id: remote_id,
                summary: Some(input.title.clone()),
                title: Some(input.title.clone()),
                description: Some(input.description.clone()),
                location: Some(input.location.clone()),
                start_time: input.start_time,
                end_time: input.end_time,
                is_all_day: input.is_all_day,
                status: "confirmed".to_string(),
                ..CalendarEventDto::default()
            })
        }
        CalendarProvider::CalDav { account_id } => {
            caldav_create_event_impl(
                &ctx.db, &ctx.encryption_key, account_id, calendar_remote_id, json,
            )
            .await
        }
    }
}

async fn dispatch_update(
    provider: &CalendarProvider,
    ctx: &ActionContext,
    calendar_remote_id: &str,
    remote_event_id: &str,
    input: &CalendarEventInput,
    etag: Option<&str>,
) -> Result<CalendarEventDto, String> {
    let json = input_to_json(input);
    match provider {
        CalendarProvider::Google(client) => {
            google_calendar_update_event_impl(
                client, &ctx.db, calendar_remote_id, remote_event_id, json,
            )
            .await
        }
        CalendarProvider::Graph(client) => {
            graph_calendar_update_event_impl(client, &ctx.db, remote_event_id, json).await
        }
        CalendarProvider::Jmap(client) => {
            ratatoskr_core::jmap::calendar_sync::update_event_remote(
                client,
                remote_event_id,
                &input.title,
                &input.description,
                &input.location,
                input.start_time,
                input.end_time,
                input.is_all_day,
            )
            .await?;
            Ok(CalendarEventDto {
                remote_event_id: remote_event_id.to_string(),
                summary: Some(input.title.clone()),
                title: Some(input.title.clone()),
                description: Some(input.description.clone()),
                location: Some(input.location.clone()),
                start_time: input.start_time,
                end_time: input.end_time,
                is_all_day: input.is_all_day,
                status: "confirmed".to_string(),
                ..CalendarEventDto::default()
            })
        }
        CalendarProvider::CalDav { account_id } => {
            caldav_update_event_impl(
                &ctx.db,
                &ctx.encryption_key,
                account_id,
                remote_event_id,
                json,
                etag.map(String::from),
            )
            .await
        }
    }
}

async fn dispatch_delete(
    provider: &CalendarProvider,
    ctx: &ActionContext,
    calendar_remote_id: &str,
    remote_event_id: &str,
    etag: Option<&str>,
) -> Result<(), String> {
    match provider {
        CalendarProvider::Google(client) => {
            google_calendar_delete_event_impl(
                client, &ctx.db, calendar_remote_id, remote_event_id,
            )
            .await
        }
        CalendarProvider::Graph(client) => {
            graph_calendar_delete_event_impl(client, &ctx.db, remote_event_id).await
        }
        CalendarProvider::Jmap(client) => {
            ratatoskr_core::jmap::calendar_sync::delete_event_remote(client, remote_event_id)
                .await
        }
        CalendarProvider::CalDav { account_id } => {
            caldav_delete_event_impl(
                &ctx.db,
                &ctx.encryption_key,
                account_id,
                remote_event_id,
                etag.map(String::from),
            )
            .await
        }
    }
}

// ── DB helpers ───────────────────────────────────────────

/// Look up a calendar's `remote_id` from its local `calendar_id`.
fn lookup_calendar_remote_id(
    conn: &rusqlite::Connection,
    account_id: &str,
    calendar_id: &str,
) -> Result<String, String> {
    conn.query_row(
        "SELECT remote_id FROM calendars WHERE id = ?1 AND account_id = ?2",
        rusqlite::params![calendar_id, account_id],
        |row| row.get::<_, String>(0),
    )
    .map_err(|e| format!("calendar remote_id lookup: {e}"))
}

/// Look up event metadata needed for provider dispatch.
struct EventMeta {
    remote_event_id: Option<String>,
    etag: Option<String>,
    calendar_id: Option<String>,
}

fn lookup_event_meta(
    conn: &rusqlite::Connection,
    event_id: &str,
) -> Result<EventMeta, String> {
    conn.query_row(
        "SELECT remote_event_id, etag, calendar_id FROM calendar_events WHERE id = ?1",
        rusqlite::params![event_id],
        |row| {
            Ok(EventMeta {
                remote_event_id: row.get(0)?,
                etag: row.get(1)?,
                calendar_id: row.get(2)?,
            })
        },
    )
    .map_err(|e| format!("event meta lookup: {e}"))
}

// ── Action functions ─────────────────────────────────────

/// Create a calendar event: local-first (instant feedback), then provider dispatch.
///
/// Returns `Success` if both local and provider succeeded.
/// Returns `LocalOnly` if local succeeded but provider failed — the event is
/// visible locally with `remote_event_id = NULL`, no automatic retry.
/// Returns `Failed` if local insert failed.
pub async fn create_calendar_event(
    ctx: &ActionContext,
    account_id: &str,
    calendar_id: &str,
    input: CalendarEventInput,
) -> ActionOutcome {
    // 1. Look up calendar_remote_id + local insert in one spawn_blocking
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let cid = calendar_id.to_string();
    let input_clone = input.clone();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;

        let calendar_remote_id = lookup_calendar_remote_id(&conn, &aid, &cid)?;

        let params = ratatoskr_core::db::queries_extra::calendars::LocalCalendarEventParams {
            account_id: aid.clone(),
            summary: input_clone.title,
            description: input_clone.description,
            location: input_clone.location,
            start_time: input_clone.start_time,
            end_time: input_clone.end_time,
            is_all_day: input_clone.is_all_day,
            calendar_id: Some(cid),
            timezone: input_clone.timezone,
            recurrence_rule: input_clone.recurrence_rule,
            availability: input_clone.availability,
            visibility: input_clone.visibility,
        };
        let event_id = ratatoskr_core::db::queries_extra::calendars::create_calendar_event_sync(
            &conn, &params,
        )?;

        Ok((event_id, calendar_remote_id))
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))
    .and_then(|r| r);

    let (event_id, calendar_remote_id) = match local_result {
        Ok(ids) => ids,
        Err(e) => return ActionOutcome::Failed { error: e },
    };

    // 2. Provider dispatch (best-effort for create)
    let provider = match create_calendar_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(p) => p,
        Err(e) => {
            log::warn!("Calendar create local-only (provider create failed): {e}");
            return ActionOutcome::LocalOnly { remote_error: e };
        }
    };

    match dispatch_create(&provider, ctx, &calendar_remote_id, &input).await {
        Ok(dto) => {
            // Store provider-assigned remote_event_id and etag
            let db = ctx.db.clone();
            let eid = event_id.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let conn = db.conn();
                if let Ok(conn) = conn.lock() {
                    let _ = conn.execute(
                        "UPDATE calendar_events SET remote_event_id = ?1, etag = ?2 WHERE id = ?3",
                        rusqlite::params![dto.remote_event_id, dto.etag, eid],
                    );
                }
            })
            .await;
            ActionOutcome::Success
        }
        Err(e) => {
            log::warn!("Calendar create provider failed for {account_id}: {e}");
            ActionOutcome::LocalOnly { remote_error: e }
        }
    }
}

/// Update a calendar event. Provider-first for synced events, local-only for unsynced.
pub async fn update_calendar_event(
    ctx: &ActionContext,
    account_id: &str,
    event_id: &str,
    input: CalendarEventInput,
) -> ActionOutcome {
    // 1. Look up event metadata
    let db = ctx.db.clone();
    let eid = event_id.to_string();
    let aid = account_id.to_string();
    let aid_outer = aid.clone();
    let meta_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        let meta = lookup_event_meta(&conn, &eid)?;

        // Also look up calendar_remote_id if we have a calendar_id
        let calendar_remote_id = meta
            .calendar_id
            .as_deref()
            .and_then(|cid| lookup_calendar_remote_id(&conn, &aid, cid).ok());

        Ok((meta, calendar_remote_id))
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))
    .and_then(|r| r);

    let (meta, calendar_remote_id) = match meta_result {
        Ok(m) => m,
        Err(e) => return ActionOutcome::Failed { error: e },
    };

    // 2. If no remote_event_id, this is a local-only event — update locally
    let Some(ref remote_event_id) = meta.remote_event_id else {
        let db = ctx.db.clone();
        let eid = event_id.to_string();
        let local_result = tokio::task::spawn_blocking(move || {
            let conn = db.conn();
            let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
            let params = ratatoskr_core::db::queries_extra::calendars::LocalCalendarEventParams {
                account_id: aid_outer,
                summary: input.title,
                description: input.description,
                location: input.location,
                start_time: input.start_time,
                end_time: input.end_time,
                is_all_day: input.is_all_day,
                calendar_id: meta.calendar_id,
                timezone: input.timezone,
                recurrence_rule: input.recurrence_rule,
                availability: input.availability,
                visibility: input.visibility,
            };
            ratatoskr_core::db::queries_extra::calendars::update_calendar_event_sync(
                &conn, &eid, &params,
            )
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))
        .and_then(|r| r);

        return match local_result {
            Ok(()) => ActionOutcome::Success,
            Err(e) => ActionOutcome::Failed { error: e },
        };
    };

    // 3. Synced event — provider-first
    let provider = match create_calendar_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(p) => p,
        Err(e) => return ActionOutcome::Failed { error: e },
    };

    let cal_remote = calendar_remote_id.unwrap_or_default();
    match dispatch_update(
        &provider,
        ctx,
        &cal_remote,
        remote_event_id,
        &input,
        meta.etag.as_deref(),
    )
    .await
    {
        Ok(dto) => {
            // Update local DB with provider-returned metadata
            let db = ctx.db.clone();
            let eid = event_id.to_string();
            let _ = tokio::task::spawn_blocking(move || {
                let conn = db.conn();
                if let Ok(conn) = conn.lock() {
                    let _ = conn.execute(
                        "UPDATE calendar_events SET \
                         summary = ?1, description = ?2, location = ?3, \
                         start_time = ?4, end_time = ?5, is_all_day = ?6, \
                         etag = ?7 \
                         WHERE id = ?8",
                        rusqlite::params![
                            dto.summary,
                            dto.description,
                            dto.location,
                            dto.start_time,
                            dto.end_time,
                            dto.is_all_day,
                            dto.etag,
                            eid,
                        ],
                    );
                }
            })
            .await;
            ActionOutcome::Success
        }
        Err(e) => {
            log::warn!("Calendar update failed for {account_id}/{event_id}: {e}");
            ActionOutcome::Failed { error: e }
        }
    }
}

/// Delete a calendar event. Provider-first for synced events, local-only for unsynced.
pub async fn delete_calendar_event(
    ctx: &ActionContext,
    account_id: &str,
    event_id: &str,
) -> ActionOutcome {
    // 1. Look up event metadata
    let db = ctx.db.clone();
    let eid = event_id.to_string();
    let aid = account_id.to_string();
    let meta_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        let meta = lookup_event_meta(&conn, &eid)?;
        let calendar_remote_id = meta
            .calendar_id
            .as_deref()
            .and_then(|cid| lookup_calendar_remote_id(&conn, &aid, cid).ok());
        Ok((meta, calendar_remote_id))
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))
    .and_then(|r| r);

    let (meta, calendar_remote_id) = match meta_result {
        Ok(m) => m,
        Err(e) => return ActionOutcome::Failed { error: e },
    };

    // 2. If no remote_event_id, local-only delete
    let Some(ref remote_event_id) = meta.remote_event_id else {
        let db = ctx.db.clone();
        let eid = event_id.to_string();
        let local_result = tokio::task::spawn_blocking(move || {
            let conn = db.conn();
            let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
            ratatoskr_core::db::queries_extra::calendars::delete_calendar_event_sync(&conn, &eid)
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))
        .and_then(|r| r);

        return match local_result {
            Ok(()) => ActionOutcome::Success,
            Err(e) => ActionOutcome::Failed { error: e },
        };
    };

    // 3. Synced event — provider-first
    let provider = match create_calendar_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(p) => p,
        Err(e) => return ActionOutcome::Failed { error: e },
    };

    let cal_remote = calendar_remote_id.unwrap_or_default();
    if let Err(e) = dispatch_delete(
        &provider,
        ctx,
        &cal_remote,
        remote_event_id,
        meta.etag.as_deref(),
    )
    .await
    {
        log::warn!("Calendar delete failed for {account_id}/{event_id}: {e}");
        return ActionOutcome::Failed { error: e };
    }

    // 4. Provider succeeded — delete locally
    let db = ctx.db.clone();
    let eid = event_id.to_string();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        ratatoskr_core::db::queries_extra::calendars::delete_calendar_event_sync(&conn, &eid)
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))
    .and_then(|r| r);

    if let Err(e) = local_result {
        log::warn!("Calendar delete local cleanup failed (provider succeeded): {e}");
    }

    ActionOutcome::Success
}
