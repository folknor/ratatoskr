//! Calendar event write path — create, update, delete through providers.
//!
//! These action functions live in the `calendar` crate (not `core::actions`)
//! because the calendar provider write APIs use typed clients (`GmailClient`,
//! `GraphClient`, `JmapClient`) that are not on the `ProviderOps` trait.
//! The `calendar` crate already depends on `core` (for `ActionContext`,
//! `ActionOutcome`, `DbState`) and has access to all provider write functions.
//! Adding `calendar` as a dependency of `core` would create a circular dep.

use rtsk::actions::{ActionContext, ActionError, ActionOutcome, MutationLog};
use rtsk::db::DbState;
use gmail::client::GmailClient;
use graph::client::GraphClient;
use jmap::client::JmapClient;

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
) -> Result<CalendarProvider, ActionError> {
    let aid = account_id.to_string();
    let db_clone = db.clone();
    let (provider, calendar_provider) = tokio::task::spawn_blocking(move || {
        let conn = db_clone.conn();
        let conn = conn.lock().map_err(|e| ActionError::db(format!("db lock: {e}")))?;
        conn.query_row(
            "SELECT provider, calendar_provider FROM accounts WHERE id = ?1",
            rusqlite::params![aid],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                ActionError::not_found(format!("account lookup: {e}"))
            }
            other => ActionError::db(format!("account lookup: {other}")),
        })
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))?
    ?;

    let effective = calendar_provider.as_deref().unwrap_or(provider.as_str());

    match effective {
        "google_api" | "gmail_api" => {
            let client = GmailClient::from_account(db, account_id, encryption_key)
                .await
                .map_err(|e| ActionError::remote(e))?;
            Ok(CalendarProvider::Google(client))
        }
        "graph" => {
            let client = GraphClient::from_account(db, account_id, encryption_key)
                .await
                .map_err(|e| ActionError::remote(e))?;
            Ok(CalendarProvider::Graph(client))
        }
        "jmap" => {
            let client = JmapClient::from_account(db, account_id, &encryption_key)
                .await
                .map_err(|e| ActionError::remote(e))?;
            Ok(CalendarProvider::Jmap(client))
        }
        "caldav" => Ok(CalendarProvider::CalDav {
            account_id: account_id.to_string(),
        }),
        other => Err(ActionError::remote(format!(
            "No calendar provider for account type: {other}"
        ))),
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
        "availability": input.availability,
        "visibility": input.visibility,
    })
}

// ── Provider dispatch helpers ────────────────────────────

async fn dispatch_create(
    provider: &CalendarProvider,
    ctx: &ActionContext,
    calendar_remote_id: &str,
    input: &CalendarEventInput,
) -> Result<CalendarEventDto, ActionError> {
    let json = input_to_json(input);
    match provider {
        CalendarProvider::Google(client) => {
            google_calendar_create_event_impl(client, &ctx.db, calendar_remote_id, json)
                .await
                .map_err(|e| ActionError::remote(e))
        }
        CalendarProvider::Graph(client) => {
            graph_calendar_create_event_impl(client, &ctx.db, calendar_remote_id, json)
                .await
                .map_err(|e| ActionError::remote(e))
        }
        CalendarProvider::Jmap(client) => {
            let remote_id = jmap::calendar_sync::create_event_remote(
                client,
                calendar_remote_id,
                &input.title,
                &input.description,
                &input.location,
                input.start_time,
                input.end_time,
                input.is_all_day,
            )
            .await
            .map_err(|e| ActionError::remote(e))?;
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
            .map_err(|e| ActionError::remote(e))
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
) -> Result<CalendarEventDto, ActionError> {
    let json = input_to_json(input);
    match provider {
        CalendarProvider::Google(client) => {
            google_calendar_update_event_impl(
                client, &ctx.db, calendar_remote_id, remote_event_id, json,
            )
            .await
            .map_err(|e| ActionError::remote(e))
        }
        CalendarProvider::Graph(client) => {
            graph_calendar_update_event_impl(client, &ctx.db, remote_event_id, json)
                .await
                .map_err(|e| ActionError::remote(e))
        }
        CalendarProvider::Jmap(client) => {
            jmap::calendar_sync::update_event_remote(
                client,
                remote_event_id,
                &input.title,
                &input.description,
                &input.location,
                input.start_time,
                input.end_time,
                input.is_all_day,
            )
            .await
            .map_err(|e| ActionError::remote(e))?;
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
            .map_err(|e| ActionError::remote(e))
        }
    }
}

async fn dispatch_delete(
    provider: &CalendarProvider,
    ctx: &ActionContext,
    calendar_remote_id: &str,
    remote_event_id: &str,
    etag: Option<&str>,
) -> Result<(), ActionError> {
    match provider {
        CalendarProvider::Google(client) => {
            google_calendar_delete_event_impl(
                client, &ctx.db, calendar_remote_id, remote_event_id,
            )
            .await
            .map_err(|e| ActionError::remote(e))
        }
        CalendarProvider::Graph(client) => {
            graph_calendar_delete_event_impl(client, &ctx.db, remote_event_id)
                .await
                .map_err(|e| ActionError::remote(e))
        }
        CalendarProvider::Jmap(client) => {
            jmap::calendar_sync::delete_event_remote(client, remote_event_id)
                .await
                .map_err(|e| ActionError::remote(e))
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
            .map_err(|e| ActionError::remote(e))
        }
    }
}

// ── DB helpers ───────────────────────────────────────────

/// Look up a calendar's `remote_id` from its local `calendar_id`.
fn lookup_calendar_remote_id(
    conn: &rusqlite::Connection,
    account_id: &str,
    calendar_id: &str,
) -> Result<String, ActionError> {
    conn.query_row(
        "SELECT remote_id FROM calendars WHERE id = ?1 AND account_id = ?2",
        rusqlite::params![calendar_id, account_id],
        |row| row.get::<_, String>(0),
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => {
            ActionError::not_found(format!("calendar remote_id lookup: {e}"))
        }
        other => ActionError::db(format!("calendar remote_id lookup: {other}")),
    })
}

/// Look up event metadata needed for provider dispatch.
struct EventMeta {
    account_id: String,
    remote_event_id: Option<String>,
    etag: Option<String>,
    calendar_id: Option<String>,
}

fn lookup_event_meta(
    conn: &rusqlite::Connection,
    event_id: &str,
) -> Result<EventMeta, ActionError> {
    conn.query_row(
        "SELECT account_id, remote_event_id, etag, calendar_id FROM calendar_events WHERE id = ?1",
        rusqlite::params![event_id],
        |row| {
            Ok(EventMeta {
                account_id: row.get(0)?,
                remote_event_id: row.get(1)?,
                etag: row.get(2)?,
                calendar_id: row.get(3)?,
            })
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => {
            ActionError::not_found(format!("event meta lookup: {e}"))
        }
        other => ActionError::db(format!("event meta lookup: {other}")),
    })
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
    let mut mlog = MutationLog::begin("create_calendar_event", account_id, calendar_id);

    // 1. Look up calendar_remote_id + local insert in one spawn_blocking
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let cid = calendar_id.to_string();
    let input_clone = input.clone();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| ActionError::db(format!("db lock: {e}")))?;

        let calendar_remote_id = lookup_calendar_remote_id(&conn, &aid, &cid)?;

        let params = rtsk::db::queries_extra::calendars::LocalCalendarEventParams {
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
        let event_id = rtsk::db::queries_extra::calendars::create_calendar_event_sync(
            &conn, &params,
        )
        .map_err(|e| ActionError::db(e))?;

        Ok((event_id, calendar_remote_id))
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r);

    let (event_id, calendar_remote_id) = match local_result {
        Ok(ids) => {
            mlog.set_local_id(&ids.0);
            ids
        }
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };

    // 2. Provider dispatch (best-effort for create)
    let provider = match create_calendar_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(p) => p,
        Err(e) => {
            let outcome = ActionOutcome::LocalOnly { reason: e, retryable: false };
            mlog.emit(&outcome);
            return outcome;
        }
    };

    let outcome = match dispatch_create(&provider, ctx, &calendar_remote_id, &input).await {
        Ok(dto) => {
            mlog.set_remote_id(&dto.remote_event_id);
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
        Err(e) => ActionOutcome::LocalOnly { reason: e, retryable: false },
    };
    mlog.emit(&outcome);
    outcome
}

/// Update a calendar event. Provider-first for synced events, local-only for unsynced.
///
/// The `account_id` parameter is used as a fallback only. The event's own
/// `account_id` from the DB is authoritative for provider resolution, preventing
/// wrong-account dispatch in multi-account setups.
pub async fn update_calendar_event(
    ctx: &ActionContext,
    _account_id: &str,
    event_id: &str,
    input: CalendarEventInput,
) -> ActionOutcome {
    let mut mlog = MutationLog::begin("update_calendar_event", "", event_id);

    // 1. Look up event metadata — use the event's own account_id, not the caller's
    let db = ctx.db.clone();
    let eid = event_id.to_string();
    let meta_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| ActionError::db(format!("db lock: {e}")))?;
        let meta = lookup_event_meta(&conn, &eid)?;

        // Use the event's own account_id for calendar lookup
        let calendar_remote_id = meta
            .calendar_id
            .as_deref()
            .and_then(|cid| lookup_calendar_remote_id(&conn, &meta.account_id, cid).ok());

        Ok((meta, calendar_remote_id))
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r);

    let (meta, calendar_remote_id) = match meta_result {
        Ok(m) => m,
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };
    mlog.set_account_id(&meta.account_id);
    if let Some(ref rid) = meta.remote_event_id {
        mlog.set_remote_id(rid);
    }

    // 2. If no remote_event_id, this is a local-only event — update locally
    let Some(ref remote_event_id) = meta.remote_event_id else {
        let db = ctx.db.clone();
        let eid = event_id.to_string();
        let local_result = tokio::task::spawn_blocking(move || {
            let conn = db.conn();
            let conn = conn.lock().map_err(|e| ActionError::db(format!("db lock: {e}")))?;
            let params = rtsk::db::queries_extra::calendars::LocalCalendarEventParams {
                account_id: meta.account_id.clone(),
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
            rtsk::db::queries_extra::calendars::update_calendar_event_sync(
                &conn, &eid, &params,
            )
            .map_err(|e| ActionError::db(e))
        })
        .await
        .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
        .and_then(|r| r);

        let outcome = match local_result {
            Ok(()) => ActionOutcome::Success,
            Err(e) => ActionOutcome::Failed { error: e },
        };
        mlog.emit(&outcome);
        return outcome;
    };

    // 3. Synced event — provider-first (use event's own account_id)
    let provider =
        match create_calendar_provider(&ctx.db, &meta.account_id, ctx.encryption_key).await {
            Ok(p) => p,
            Err(e) => {
                let outcome = ActionOutcome::Failed { error: e };
                mlog.emit(&outcome);
                return outcome;
            }
        };

    let Some(cal_remote) = calendar_remote_id else {
        let outcome = ActionOutcome::Failed {
            error: ActionError::not_found(
                "Synced event has no resolvable calendar remote ID",
            ),
        };
        mlog.emit(&outcome);
        return outcome;
    };
    let outcome = match dispatch_update(
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
            // Update local DB with all edited fields + provider-returned etag.
            // Use input values (what the user edited) for all fields, plus etag
            // from the DTO for concurrency control.
            let db = ctx.db.clone();
            let eid = event_id.to_string();
            let event_account_id = meta.account_id.clone();
            let cal_id = meta.calendar_id.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let conn = db.conn();
                if let Ok(conn) = conn.lock() {
                    let params =
                        rtsk::db::queries_extra::calendars::LocalCalendarEventParams {
                            account_id: event_account_id,
                            summary: input.title,
                            description: input.description,
                            location: input.location,
                            start_time: input.start_time,
                            end_time: input.end_time,
                            is_all_day: input.is_all_day,
                            calendar_id: cal_id,
                            timezone: input.timezone,
                            recurrence_rule: input.recurrence_rule,
                            availability: input.availability,
                            visibility: input.visibility,
                        };
                    let _ = rtsk::db::queries_extra::calendars::update_calendar_event_sync(
                        &conn, &eid, &params,
                    );
                    // Also update etag from provider response
                    let _ = conn.execute(
                        "UPDATE calendar_events SET etag = ?1 WHERE id = ?2",
                        rusqlite::params![dto.etag, eid],
                    );
                }
            })
            .await;
            ActionOutcome::Success
        }
        Err(e) => ActionOutcome::Failed { error: e },
    };
    mlog.emit(&outcome);
    outcome
}

/// Delete a calendar event. Provider-first for synced events, local-only for unsynced.
pub async fn delete_calendar_event(
    ctx: &ActionContext,
    _account_id: &str,
    event_id: &str,
) -> ActionOutcome {
    let mut mlog = MutationLog::begin("delete_calendar_event", "", event_id);

    // 1. Look up event metadata — use the event's own account_id
    let db = ctx.db.clone();
    let eid = event_id.to_string();
    let meta_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| ActionError::db(format!("db lock: {e}")))?;
        let meta = lookup_event_meta(&conn, &eid)?;
        let calendar_remote_id = meta
            .calendar_id
            .as_deref()
            .and_then(|cid| lookup_calendar_remote_id(&conn, &meta.account_id, cid).ok());
        Ok((meta, calendar_remote_id))
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r);

    let (meta, calendar_remote_id) = match meta_result {
        Ok(m) => m,
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };
    mlog.set_account_id(&meta.account_id);
    if let Some(ref rid) = meta.remote_event_id {
        mlog.set_remote_id(rid);
    }

    // 2. If no remote_event_id, local-only delete
    let Some(ref remote_event_id) = meta.remote_event_id else {
        let db = ctx.db.clone();
        let eid = event_id.to_string();
        let local_result = tokio::task::spawn_blocking(move || {
            let conn = db.conn();
            let conn = conn.lock().map_err(|e| ActionError::db(format!("db lock: {e}")))?;
            rtsk::db::queries_extra::calendars::delete_calendar_event_sync(&conn, &eid)
                .map_err(|e| ActionError::db(e))
        })
        .await
        .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
        .and_then(|r| r);

        let outcome = match local_result {
            Ok(()) => ActionOutcome::Success,
            Err(e) => ActionOutcome::Failed { error: e },
        };
        mlog.emit(&outcome);
        return outcome;
    };

    // 3. Synced event — provider-first (use event's own account_id)
    let provider =
        match create_calendar_provider(&ctx.db, &meta.account_id, ctx.encryption_key).await {
            Ok(p) => p,
            Err(e) => {
                let outcome = ActionOutcome::Failed { error: e };
                mlog.emit(&outcome);
                return outcome;
            }
        };

    let Some(cal_remote) = calendar_remote_id else {
        let outcome = ActionOutcome::Failed {
            error: ActionError::not_found(
                "Synced event has no resolvable calendar remote ID",
            ),
        };
        mlog.emit(&outcome);
        return outcome;
    };
    if let Err(e) = dispatch_delete(
        &provider,
        ctx,
        &cal_remote,
        remote_event_id,
        meta.etag.as_deref(),
    )
    .await
    {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    // 4. Provider succeeded — delete locally
    let db = ctx.db.clone();
    let eid = event_id.to_string();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| ActionError::db(format!("db lock: {e}")))?;
        rtsk::db::queries_extra::calendars::delete_calendar_event_sync(&conn, &eid)
            .map_err(|e| ActionError::db(e))
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r);

    if let Err(e) = local_result {
        log::warn!("Calendar delete local cleanup failed (provider succeeded): {e}");
    }

    let outcome = ActionOutcome::Success;
    mlog.emit(&outcome);
    outcome
}
