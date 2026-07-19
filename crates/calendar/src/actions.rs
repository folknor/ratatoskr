//! Calendar event write path through the bifrost `Account` surface.

use action_types::{ActionError, ActionOutcome, CalendarActionContext, MutationLog};
use bifrost_types::{Account, ProtocolKind, RsvpStatus};
use db::db::WriteConn;
use db::db::queries_extra::{
    set_calendar_event_remote_id_and_etag, update_calendar_event_fields_and_etag,
};
use std::sync::Arc;

use super::idmap;

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

fn lookup_calendar_remote_id(
    conn: &WriteConn<'_>,
    account_id: &str,
    calendar_id: &str,
) -> Result<String, ActionError> {
    conn.query_row(
        "SELECT remote_id FROM calendars WHERE id = ?1 AND account_id = ?2",
        rusqlite::params![calendar_id, account_id],
        |row| row.get(0),
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => {
            ActionError::not_found(format!("calendar remote_id lookup: {e}"))
        }
        other => ActionError::db(format!("calendar remote_id lookup: {other}")),
    })
}

struct EventMeta {
    account_id: String,
    remote_event_id: Option<String>,
    calendar_id: Option<String>,
}

fn lookup_event_meta(conn: &WriteConn<'_>, event_id: &str) -> Result<EventMeta, ActionError> {
    conn.query_row(
        "SELECT account_id, remote_event_id, calendar_id FROM calendar_events WHERE id = ?1",
        rusqlite::params![event_id],
        |row| {
            Ok(EventMeta {
                account_id: row.get(0)?,
                remote_event_id: row.get(1)?,
                calendar_id: row.get(2)?,
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

async fn open_calendar_account(
    ctx: &CalendarActionContext,
    account_id: &str,
) -> Result<Option<(Arc<dyn Account>, ProtocolKind)>, ActionError> {
    ctx.opener.open(account_id).await
}

fn unsupported(operation: &str) -> ActionError {
    ActionError::not_implemented(format!(
        "account's calendar backend does not support event {operation}"
    ))
}

pub async fn create_calendar_event(
    ctx: &CalendarActionContext,
    account_id: &str,
    calendar_id: &str,
    input: CalendarEventInput,
) -> ActionOutcome {
    let mut mlog = MutationLog::begin("create_calendar_event", account_id, calendar_id);
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let cid = calendar_id.to_string();
    let local_input = input.clone();
    let local = db
        .with_write_mapped(
            move |conn| {
                let calendar_remote_id = lookup_calendar_remote_id(conn, &aid, &cid)?;
                let params = db::db::queries_extra::calendars::LocalCalendarEventParams {
                    account_id: aid.clone(),
                    summary: local_input.title,
                    description: local_input.description,
                    location: local_input.location,
                    start_time: local_input.start_time,
                    end_time: local_input.end_time,
                    is_all_day: local_input.is_all_day,
                    calendar_id: Some(cid),
                    timezone: local_input.timezone,
                    recurrence_rule: local_input.recurrence_rule,
                    availability: local_input.availability,
                    visibility: local_input.visibility,
                };
                let event_id =
                    db::db::queries_extra::calendars::create_calendar_event_sync(conn, &params)
                        .map_err(ActionError::db)?;
                Ok((event_id, calendar_remote_id))
            },
            ActionError::db,
        )
        .await;
    let (event_id, calendar_remote_id) = match local {
        Ok(v) => {
            mlog.set_local_id(&v.0);
            v
        }
        Err(error) => {
            let out = ActionOutcome::Failed { error };
            mlog.emit(&out);
            return out;
        }
    };
    let opened = match open_calendar_account(ctx, account_id).await {
        Ok(Some(v)) => v,
        Ok(None) => {
            let out = ActionOutcome::LocalOnly {
                reason: ActionError::remote("no calendar backend for account"),
                retryable: false,
            };
            mlog.emit(&out);
            return out;
        }
        Err(reason) => {
            let out = ActionOutcome::LocalOnly {
                reason,
                retryable: false,
            };
            mlog.emit(&out);
            return out;
        }
    };
    let (account, provider) = opened;
    let out = if !account.capabilities().pim_methods.event_create {
        ActionOutcome::LocalOnly {
            reason: unsupported("creation"),
            retryable: false,
        }
    } else {
        match account
            .event_create(idmap::input_to_event_create(&calendar_remote_id, &input))
            .await
        {
            Ok(remote_id) => {
                let native_id = idmap::strip_event_id(provider, &remote_id);
                mlog.set_remote_id(&native_id);
                let db = ctx.write_db.clone();
                let eid = event_id.clone();
                let _ = db
                    .with_write_mapped(
                        move |conn| {
                            set_calendar_event_remote_id_and_etag(conn, &eid, &native_id, None)
                                .map_err(ActionError::db)
                        },
                        ActionError::db,
                    )
                    .await;
                ActionOutcome::Success
            }
            Err(error) => ActionOutcome::LocalOnly {
                reason: ActionError::remote(error.to_string()),
                retryable: false,
            },
        }
    };
    mlog.emit(&out);
    out
}

pub async fn update_calendar_event(
    ctx: &CalendarActionContext,
    _account_id: &str,
    event_id: &str,
    input: CalendarEventInput,
) -> ActionOutcome {
    let mut mlog = MutationLog::begin("update_calendar_event", "", event_id);
    let db = ctx.write_db.clone();
    let eid = event_id.to_string();
    let got = db
        .with_write_mapped(
            move |conn| {
                let meta = lookup_event_meta(conn, &eid)?;
                let remote = meta
                    .calendar_id
                    .as_deref()
                    .and_then(|id| lookup_calendar_remote_id(conn, &meta.account_id, id).ok());
                Ok((meta, remote))
            },
            ActionError::db,
        )
        .await;
    let (meta, calendar_remote_id) = match got {
        Ok(v) => v,
        Err(error) => {
            let out = ActionOutcome::Failed { error };
            mlog.emit(&out);
            return out;
        }
    };
    mlog.set_account_id(&meta.account_id);
    let Some(remote_event_id) = meta.remote_event_id.as_deref() else {
        return local_update(ctx, event_id, meta, input, &mut mlog).await;
    };
    let Some(calendar_remote_id) = calendar_remote_id else {
        let out = ActionOutcome::Failed {
            error: ActionError::not_found("Synced event has no resolvable calendar remote ID"),
        };
        mlog.emit(&out);
        return out;
    };
    let (account, provider) = match open_calendar_account(ctx, &meta.account_id).await {
        Ok(Some(v)) => v,
        Ok(None) => {
            let out = ActionOutcome::Failed {
                error: ActionError::remote("no calendar backend for account"),
            };
            mlog.emit(&out);
            return out;
        }
        Err(error) => {
            let out = ActionOutcome::Failed { error };
            mlog.emit(&out);
            return out;
        }
    };
    if !account.capabilities().pim_methods.event_update {
        let out = ActionOutcome::Failed {
            error: unsupported("updates"),
        };
        mlog.emit(&out);
        return out;
    }
    let id =
        idmap::event_id_for_writeback(provider, remote_event_id, None, Some(&calendar_remote_id)).0;
    let out = match account
        .event_update(id, idmap::input_to_event_patch(&input))
        .await
    {
        Ok(()) => {
            let db = ctx.write_db.clone();
            let eid = event_id.to_string();
            let account_id = meta.account_id;
            let calendar_id = meta.calendar_id;
            match db
                .with_write_mapped(
                    move |conn| {
                        let p = params(account_id, calendar_id, input);
                        update_calendar_event_fields_and_etag(conn, &eid, &p, None)
                            .map_err(ActionError::db)
                    },
                    ActionError::db,
                )
                .await
            {
                Ok(()) => ActionOutcome::Success,
                Err(error) => ActionOutcome::Failed { error },
            }
        }
        Err(error) => ActionOutcome::Failed {
            error: ActionError::remote(error.to_string()),
        },
    };
    mlog.emit(&out);
    out
}

async fn local_update(
    ctx: &CalendarActionContext,
    event_id: &str,
    meta: EventMeta,
    input: CalendarEventInput,
    mlog: &mut MutationLog,
) -> ActionOutcome {
    let db = ctx.write_db.clone();
    let eid = event_id.to_string();
    let out = match db
        .with_write_mapped(
            move |conn| {
                db::db::queries_extra::calendars::update_calendar_event_sync(
                    conn,
                    &eid,
                    &params(meta.account_id, meta.calendar_id, input),
                )
                .map_err(ActionError::db)
            },
            ActionError::db,
        )
        .await
    {
        Ok(()) => ActionOutcome::Success,
        Err(error) => ActionOutcome::Failed { error },
    };
    mlog.emit(&out);
    out
}

fn params(
    account_id: String,
    calendar_id: Option<String>,
    input: CalendarEventInput,
) -> db::db::queries_extra::calendars::LocalCalendarEventParams {
    db::db::queries_extra::calendars::LocalCalendarEventParams {
        account_id,
        summary: input.title,
        description: input.description,
        location: input.location,
        start_time: input.start_time,
        end_time: input.end_time,
        is_all_day: input.is_all_day,
        calendar_id,
        timezone: input.timezone,
        recurrence_rule: input.recurrence_rule,
        availability: input.availability,
        visibility: input.visibility,
    }
}

pub async fn delete_calendar_event(
    ctx: &CalendarActionContext,
    _account_id: &str,
    event_id: &str,
) -> ActionOutcome {
    let mut mlog = MutationLog::begin("delete_calendar_event", "", event_id);
    let db = ctx.write_db.clone();
    let eid = event_id.to_string();
    let got = db
        .with_write_mapped(
            move |conn| {
                let meta = lookup_event_meta(conn, &eid)?;
                let remote = meta
                    .calendar_id
                    .as_deref()
                    .and_then(|id| lookup_calendar_remote_id(conn, &meta.account_id, id).ok());
                Ok((meta, remote))
            },
            ActionError::db,
        )
        .await;
    let (meta, calendar_remote_id) = match got {
        Ok(v) => v,
        Err(error) => {
            let out = ActionOutcome::Failed { error };
            mlog.emit(&out);
            return out;
        }
    };
    mlog.set_account_id(&meta.account_id);
    let Some(remote_event_id) = meta.remote_event_id.as_deref() else {
        return local_delete(ctx, event_id, &mut mlog).await;
    };
    let Some(calendar_remote_id) = calendar_remote_id else {
        let out = ActionOutcome::Failed {
            error: ActionError::not_found("Synced event has no resolvable calendar remote ID"),
        };
        mlog.emit(&out);
        return out;
    };
    let (account, provider) = match open_calendar_account(ctx, &meta.account_id).await {
        Ok(Some(v)) => v,
        Ok(None) => {
            let out = ActionOutcome::Failed {
                error: ActionError::remote("no calendar backend for account"),
            };
            mlog.emit(&out);
            return out;
        }
        Err(error) => {
            let out = ActionOutcome::Failed { error };
            mlog.emit(&out);
            return out;
        }
    };
    if !account.capabilities().pim_methods.event_delete {
        let out = ActionOutcome::Failed {
            error: unsupported("deletion"),
        };
        mlog.emit(&out);
        return out;
    }
    let id =
        idmap::event_id_for_writeback(provider, remote_event_id, None, Some(&calendar_remote_id)).0;
    if let Err(error) = account.event_delete(id).await {
        let out = ActionOutcome::Failed {
            error: ActionError::remote(error.to_string()),
        };
        mlog.emit(&out);
        return out;
    }
    local_delete(ctx, event_id, &mut mlog).await
}

async fn local_delete(
    ctx: &CalendarActionContext,
    event_id: &str,
    mlog: &mut MutationLog,
) -> ActionOutcome {
    let db = ctx.write_db.clone();
    let eid = event_id.to_string();
    if let Err(error) = db
        .with_write_mapped(
            move |conn| {
                db::db::queries_extra::calendars::delete_calendar_event_sync(conn, &eid)
                    .map_err(ActionError::db)
            },
            ActionError::db,
        )
        .await
    {
        log::warn!("Calendar delete local cleanup failed: {error}");
    }
    let out = ActionOutcome::Success;
    mlog.emit(&out);
    out
}

pub async fn rsvp_calendar_event(
    ctx: &CalendarActionContext,
    _account_id: &str,
    event_id: &str,
    status: RsvpStatus,
) -> ActionOutcome {
    let mut mlog = MutationLog::begin("rsvp_calendar_event", "", event_id);
    let db = ctx.write_db.clone();
    let eid = event_id.to_string();
    let got = db
        .with_write_mapped(
            move |conn| {
                let meta = lookup_event_meta(conn, &eid)?;
                let remote = meta
                    .calendar_id
                    .as_deref()
                    .and_then(|id| lookup_calendar_remote_id(conn, &meta.account_id, id).ok());
                Ok((meta, remote))
            },
            ActionError::db,
        )
        .await;
    let (meta, calendar_remote_id) = match got {
        Ok(v) => v,
        Err(error) => {
            let out = ActionOutcome::Failed { error };
            mlog.emit(&out);
            return out;
        }
    };
    mlog.set_account_id(&meta.account_id);
    let (Some(remote_event_id), Some(calendar_remote_id)) =
        (meta.remote_event_id.as_deref(), calendar_remote_id)
    else {
        let out = ActionOutcome::Failed {
            error: ActionError::not_found("cannot RSVP to an unsynced event"),
        };
        mlog.emit(&out);
        return out;
    };
    let (account, provider) = match open_calendar_account(ctx, &meta.account_id).await {
        Ok(Some(v)) => v,
        Ok(None) => {
            let out = ActionOutcome::Failed {
                error: ActionError::remote("no calendar backend for account"),
            };
            mlog.emit(&out);
            return out;
        }
        Err(error) => {
            let out = ActionOutcome::Failed { error };
            mlog.emit(&out);
            return out;
        }
    };
    if !account.capabilities().pim_methods.event_rsvp {
        let out = ActionOutcome::Failed {
            error: unsupported("RSVP"),
        };
        mlog.emit(&out);
        return out;
    }
    let id =
        idmap::event_id_for_writeback(provider, remote_event_id, None, Some(&calendar_remote_id)).0;
    let out = match account.event_rsvp(id, status).await {
        Ok(()) => {
            let db = ctx.write_db.clone();
            let eid = event_id.to_string();
            let value = idmap::rsvp(status);
            match db
                .with_write_mapped(
                    move |conn| {
                        db::db::queries_extra::calendars::set_calendar_event_rsvp_sync(
                            conn, &eid, &value,
                        )
                        .map_err(ActionError::db)
                    },
                    ActionError::db,
                )
                .await
            {
                Ok(()) => ActionOutcome::Success,
                Err(error) => ActionOutcome::Failed { error },
            }
        }
        Err(error) => ActionOutcome::Failed {
            error: ActionError::remote(error.to_string()),
        },
    };
    mlog.emit(&out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use action_types::CalendarAccountOpener;

    /// The unsynced-event RSVP branch resolves BEFORE opening any provider
    /// account, so a correct implementation must never call the opener. This
    /// stub panics to prove that invariant.
    struct PanicOpener;

    #[async_trait::async_trait]
    impl CalendarAccountOpener for PanicOpener {
        async fn open(
            &self,
            _account_id: &str,
        ) -> Result<Option<(Arc<dyn Account>, ProtocolKind)>, ActionError> {
            panic!("opener must not be opened for an unsynced-event RSVP");
        }
    }

    fn test_ctx(dir: &std::path::Path) -> CalendarActionContext {
        let pool = db::db::open_writer_pool(dir).expect("open writer pool");
        let write_db = service_state::WriteDbState::from_pool(pool);
        let read_db = db::db::open_reader_pool(dir).expect("open reader pool");
        CalendarActionContext {
            write_db,
            read_db,
            encryption_key: [0u8; 32],
            opener: Arc::new(PanicOpener),
        }
    }

    // Spec § 4.6 / § 6.3: RSVP has no local-only meaning, so RSVPing a purely
    // local (unsynced, `remote_event_id IS NULL`) event fails fast as
    // `not_found` without ever touching the provider.
    #[tokio::test]
    async fn rsvp_unsynced_event_fails_without_opening_provider() {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let ctx = test_ctx(tmp.path());
        let event_id = ctx
            .write_db
            .with_write_sync(|conn| {
                conn.execute(
                    "INSERT INTO accounts (id, email) VALUES ('acct', 'acct@example.test')",
                    [],
                )
                .map_err(|e| e.to_string())?;
                let params = db::db::queries_extra::calendars::LocalCalendarEventParams {
                    account_id: "acct".to_string(),
                    summary: "Local only".to_string(),
                    description: String::new(),
                    location: String::new(),
                    start_time: 0,
                    end_time: 0,
                    is_all_day: false,
                    calendar_id: None,
                    timezone: None,
                    recurrence_rule: None,
                    availability: None,
                    visibility: None,
                };
                db::db::queries_extra::calendars::create_calendar_event_sync(conn, &params)
            })
            .expect("seed unsynced event");

        let outcome = rsvp_calendar_event(&ctx, "acct", &event_id, RsvpStatus::Accepted).await;
        match outcome {
            ActionOutcome::Failed { error } => assert!(
                error.to_string().contains("unsynced"),
                "expected an unsynced-event failure, got: {error}"
            ),
            other => panic!("expected Failed for an unsynced RSVP, got {other:?}"),
        }
    }
}
