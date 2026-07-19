//! Contact writes: local persistence plus the resident bifrost account.

use bifrost_types::{AccountId, ContactId, ContactOrganization, ContactPatch, ContactPhone};
use db::db::queries_extra::contacts::{
    UpsertContactParams, db_delete_contact, db_upsert_contact_full,
};

use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use crate::bifrost::resident::ResidentActionAccount;

#[derive(Debug, Clone)]
pub struct ContactSaveInput {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub email2: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub notes: Option<String>,
    pub account_id: Option<String>,
    pub source: Option<String>,
    pub server_id: Option<String>,
}

pub(crate) async fn save_contact(
    ctx: &ActionContext,
    action_account: Option<&ResidentActionAccount>,
    input: ContactSaveInput,
) -> ActionOutcome {
    let mlog = MutationLog::begin(
        "save_contact",
        input.account_id.as_deref().unwrap_or(""),
        &input.id,
    );
    let db = ctx.write_db.clone();
    let local = input.clone();
    if let Err(error) = db
        .with_write_mapped(
            move |conn| {
                db_upsert_contact_full(
                    conn,
                    UpsertContactParams {
                        id: &local.id,
                        email: &local.email,
                        display_name: local.display_name.as_deref(),
                        email2: local.email2.as_deref(),
                        phone: local.phone.as_deref(),
                        company: local.company.as_deref(),
                        notes: local.notes.as_deref(),
                        account_id: local.account_id.as_deref(),
                        source: local.source.as_deref().unwrap_or("user"),
                    },
                )
                .map_err(ActionError::db)
            },
            ActionError::db,
        )
        .await
    {
        let outcome = ActionOutcome::Failed { error };
        mlog.emit(&outcome);
        return outcome;
    }
    if input.source.as_deref().unwrap_or("user") == "user" {
        mlog.emit(&ActionOutcome::Success);
        return ActionOutcome::Success;
    }
    let outcome = match (&input.account_id, &input.server_id) {
        (Some(account_id), Some(server_id)) => match update_remote(
            action_account,
            account_id,
            server_id,
            input.phone.as_deref(),
            input.company.as_deref(),
            input.notes.as_deref(),
        )
        .await
        {
            Ok(()) => ActionOutcome::Success,
            Err(reason) => {
                let retryable = reason.is_retryable();
                ActionOutcome::LocalOnly { reason, retryable }
            }
        },
        _ => ActionOutcome::LocalOnly {
            reason: ActionError::remote("synced contact missing account_id or server_id"),
            retryable: false,
        },
    };
    mlog.emit(&outcome);
    outcome
}

pub(crate) async fn delete_contact(
    ctx: &ActionContext,
    action_account: Option<&ResidentActionAccount>,
    contact_id: &str,
) -> ActionOutcome {
    let mut mlog = MutationLog::begin("delete_contact", "", contact_id);
    let contact_id_owned = contact_id.to_string();
    let meta = match ctx
        .write_db
        .clone()
        .with_write_mapped(
            move |conn| {
                db::db::queries_extra::action_helpers::get_contact_meta_by_id_sync(
                    &conn.as_read(),
                    &contact_id_owned,
                )
                .map_err(ActionError::db)?
                .ok_or_else(|| ActionError::not_found("contact not found"))
            },
            ActionError::db,
        )
        .await
    {
        Ok(meta) => meta,
        Err(error) => {
            let outcome = ActionOutcome::Failed { error };
            mlog.emit(&outcome);
            return outcome;
        }
    };
    if let Some(account_id) = &meta.account_id {
        mlog.set_account_id(account_id);
    }
    if let Some(server_id) = &meta.server_id {
        mlog.set_remote_id(server_id);
    }
    if meta.source.as_deref().unwrap_or("user") != "user" {
        let (Some(account_id), Some(server_id)) =
            (meta.account_id.as_deref(), meta.server_id.as_deref())
        else {
            let outcome = ActionOutcome::Failed {
                error: ActionError::remote("synced contact missing account identity"),
            };
            mlog.emit(&outcome);
            return outcome;
        };
        if let Err(error) = delete_remote(action_account, account_id, server_id).await {
            let outcome = ActionOutcome::Failed { error };
            mlog.emit(&outcome);
            return outcome;
        }
    }
    if let Err(error) = db_delete_contact(&ctx.write_db.writer_pool(), contact_id.to_string()).await
    {
        let outcome = ActionOutcome::Failed {
            error: ActionError::db(error),
        };
        mlog.emit(&outcome);
        return outcome;
    }
    mlog.emit(&ActionOutcome::Success);
    ActionOutcome::Success
}

async fn update_remote(
    action_account: Option<&ResidentActionAccount>,
    account_id: &str,
    server_id: &str,
    phone: Option<&str>,
    company: Option<&str>,
    notes: Option<&str>,
) -> Result<(), ActionError> {
    let account =
        action_account.ok_or_else(|| ActionError::remote("resident account unavailable"))?;
    let patch = ContactPatch {
        phones: phone.map(|value| {
            vec![ContactPhone {
                value: value.into(),
                kind: None,
                is_primary: true,
            }]
        }),
        organizations: company.map(|value| {
            vec![ContactOrganization {
                name: value.into(),
                title: None,
            }]
        }),
        notes: notes.map(|value| Some(value.into())),
        ..Default::default()
    };
    account
        .engine
        .contact_update(
            &AccountId(account_id.to_string()),
            ContactId(server_id.to_string()),
            patch,
        )
        .await
        .map_err(map_engine_error)
}

/// Convert a bifrost engine error into an `ActionError` without leaking the
/// raw `AccountError` across the IPC wire: `AccountError`s route through the
/// shared mapper (stable `message_key` + retryable classification), and only
/// engine-internal errors (never attached, shutting down, ...) fall back to a
/// `Display` string.
fn map_engine_error(error: bifrost_sync::Error) -> ActionError {
    match error {
        bifrost_sync::Error::Account(ref account_error) => {
            crate::bifrost::account_error_to_action_error(account_error)
        }
        other => ActionError::remote(other.to_string()),
    }
}

async fn delete_remote(
    action_account: Option<&ResidentActionAccount>,
    account_id: &str,
    server_id: &str,
) -> Result<(), ActionError> {
    let account =
        action_account.ok_or_else(|| ActionError::remote("resident account unavailable"))?;
    account
        .engine
        .contact_delete(
            &AccountId(account_id.to_string()),
            ContactId(server_id.to_string()),
        )
        .await
        .map_err(map_engine_error)
}
