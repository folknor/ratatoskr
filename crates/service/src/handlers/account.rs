//! Account write handlers (Phase 6a).
//!
//! `account.update` and `account.reorder` are the small / non-envelope
//! surfaces. The bigger account operations live in their own modules:
//!
//! - `account.create` (Plaintext | Encrypted credential envelope)
//! - `account.delete` (cancel-and-await runner orchestration)
//!
//! Credentials encryption: `account.create` and `account.update_tokens`
//! encrypt at the handler boundary using `BootSharedState`'s key. The
//! UI ships `Plaintext` and the handler routes it through
//! `common::crypto::encrypt_value` before calling the DB-layer write.
//! The `AccountCredentials::Encrypted` variant exists for the (rare)
//! case where the caller already holds ciphertext; both shapes hit the
//! same DB column with the same `enc:base64iv:base64ct` form.

use std::sync::Arc;

use serde_json::Value;
use service_api::{
    AccountCreateAck, AccountCreateParams, AccountCredentials, AccountDeleteAck,
    AccountDeleteParams, AccountReorderAck, AccountReorderParams, AccountUpdateAck,
    AccountUpdateParams, AccountUpdateTokensAck, AccountUpdateTokensParams, ServiceError,
};

use crate::boot::BootSharedState;

/// Encrypt up to four optional credential fields with the Service's
/// loaded key. Bundled into one `spawn_blocking` so the AES-GCM CPU
/// work doesn't sit on the dispatch executor and so we don't pay the
/// spawn overhead per field.
pub(crate) async fn encrypt_optional_credentials(
    key: [u8; 32],
    access_token: Option<String>,
    refresh_token: Option<String>,
    imap_password: Option<String>,
    smtp_password: Option<String>,
) -> Result<
    (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ),
    ServiceError,
> {
    tokio::task::spawn_blocking(move || {
        let enc = |v: Option<String>| -> Result<Option<String>, String> {
            v.map(|s| common::crypto::encrypt_value(&key, &s)).transpose()
        };
        Ok::<_, String>((
            enc(access_token)?,
            enc(refresh_token)?,
            enc(imap_password)?,
            enc(smtp_password)?,
        ))
    })
    .await
    .map_err(|e| ServiceError::Internal(format!("spawn_blocking encrypt: {e}")))?
    .map_err(ServiceError::Internal)
}

pub(crate) async fn handle_update(
    boot_state: &Arc<BootSharedState>,
    params: AccountUpdateParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;

    // Attachments roadmap review: if the patch flips
    // `cache_attachments_enabled` from 0 to 1, fire a backfill kick
    // so historical attachments inside the retention window populate
    // promptly. Without this, the only triggers for an enabled
    // account are the next boot's recovery kick or a window-extend.
    let toggle_request = params.cache_attachments_enabled;
    let account_id_for_kick = params.id.clone();
    let was_enabled_before: Option<bool> = if toggle_request.is_some() {
        let aid = account_id_for_kick.clone();
        write_db
            .with_conn(move |conn| {
                let v: Option<i64> = conn
                    .query_row(
                        "SELECT cache_attachments_enabled FROM accounts WHERE id = ?1",
                        rusqlite::params![aid],
                        |r| r.get(0),
                    )
                    .ok();
                Ok(v.map(|n| n != 0))
            })
            .await
            .ok()
            .flatten()
    } else {
        None
    };

    write_db
        .with_conn(move |conn| {
            let id = params.id;
            let update = db::db::queries_extra::UpdateAccountParams {
                account_name: params.account_name,
                display_name: params.display_name,
                account_color: params.account_color,
                caldav_url: params.caldav_url,
                caldav_username: params.caldav_username,
                caldav_password: params.caldav_password,
                cache_attachments_enabled: params.cache_attachments_enabled,
            };
            db::db::queries_extra::update_account_sync(conn, &id, update)
        })
        .await
        .map_err(ServiceError::Internal)?;

    if let (Some(true), Some(false)) = (toggle_request, was_enabled_before) {
        let boot_state = Arc::clone(boot_state);
        tokio::spawn(async move {
            kick_cache_reenable(boot_state, account_id_for_kick).await;
        });
    }

    serde_json::to_value(AccountUpdateAck)
        .map_err(|e| ServiceError::Internal(e.to_string()))
}

/// Post-commit kick fired when `cache_attachments_enabled` flips
/// from disabled to enabled. Detached from the IPC ack via
/// `tokio::spawn` so a long backfill doesn't stall `account.update`.
/// Phase 7: every provider with a known type kicks, not just JMAP.
async fn kick_cache_reenable(boot_state: Arc<BootSharedState>, account_id: String) {
    let Some(prefetch) = boot_state.prefetch_runtime() else {
        log::debug!(
            "account.update reenable-kick: PrefetchRuntime not installed; skipping for {account_id}",
        );
        return;
    };
    let Ok(write_db) = boot_state.write_db_state() else { return };
    let aid = account_id.clone();
    let provider: String = write_db
        .with_conn(move |conn| {
            conn.query_row(
                "SELECT COALESCE(provider, '') FROM accounts WHERE id = ?1",
                rusqlite::params![aid],
                |r| r.get::<_, String>(0),
            )
            .or(Ok(String::new()))
        })
        .await
        .unwrap_or_default();
    if provider.is_empty() {
        return;
    }
    let window_days = match write_db
        .with_conn(|conn| Ok(sync::config::get_sync_period_days(conn)))
        .await
    {
        Ok(v) => v,
        Err(e) => {
            log::debug!("account.update reenable-kick: sync_period_days read failed: {e}");
            return;
        }
    };
    let window_start = chrono::Utc::now().timestamp() - window_days.saturating_mul(86_400);
    if let Err(e) = prefetch
        .kick_backfill_account(&account_id, &provider, window_start)
        .await
    {
        log::debug!("account.update reenable-kick {account_id}: {e}");
    }
}

pub(crate) async fn handle_reorder(
    boot_state: &Arc<BootSharedState>,
    params: AccountReorderParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    write_db
        .with_conn(move |conn| {
            let updates: Vec<(String, i64)> = params
                .orders
                .into_iter()
                .map(|e| (e.account_id, e.sort_order))
                .collect();
            db::db::queries_extra::update_account_sort_order_sync(conn, &updates)
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(AccountReorderAck)
        .map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_update_tokens(
    boot_state: &Arc<BootSharedState>,
    params: Box<AccountUpdateTokensParams>,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    let key = boot_state.encryption_key().ok_or_else(|| {
        ServiceError::Internal(
            "encryption key not loaded; UI must wait for boot.ready before calling \
             account.update_tokens"
                .into(),
        )
    })?;
    let params = *params;
    let id = params.account_id.clone();
    let id_for_push = id.clone();
    let (access_token, refresh_token, imap_password, smtp_password) =
        encrypt_optional_credentials(
            key,
            params.access_token.map(service_api::RedactedString::into_inner),
            params.refresh_token.map(service_api::RedactedString::into_inner),
            params.imap_password.map(service_api::RedactedString::into_inner),
            params.smtp_password.map(service_api::RedactedString::into_inner),
        )
        .await?;
    let reauth = db::db::queries_extra::ReauthAccountParams {
        access_token,
        refresh_token,
        token_expires_at: params.token_expires_at,
        imap_password,
        smtp_password,
    };
    write_db
        .with_conn(move |conn| db::db::queries_extra::update_account_tokens_sync(conn, &id, reauth))
        .await
        .map_err(ServiceError::Internal)?;

    // Phase 8-3: re-arm push for the re-authed account. Without this,
    // a JMAP token-revocation kills the websocket bridge and password
    // re-entry leaves push dead until the Service restarts.
    // `start_account` silently skips non-JMAP accounts; `fresh_start:
    // true` clears the persisted `push_state` because the new session
    // may not honour the old session's cursor.
    if let Some(push_runtime) = boot_state.push_runtime() {
        tokio::spawn(async move {
            if let Err(e) = push_runtime
                .start_account(id_for_push.clone(), true)
                .await
            {
                log::warn!("[push] re-auth start_account({id_for_push}) failed: {e}");
            }
        });
    }

    serde_json::to_value(AccountUpdateTokensAck)
        .map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_create(
    boot_state: &Arc<BootSharedState>,
    params: Box<AccountCreateParams>,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    let key = boot_state.encryption_key().ok_or_else(|| {
        ServiceError::Internal(
            "encryption key not loaded; UI must wait for boot.ready before calling \
             account.create"
                .into(),
        )
    })?;
    let p = *params;
    // Plaintext routes through `encrypt_value` here; Encrypted is the
    // pre-encrypted-blob variant for re-auth / recovery paths and
    // passes through verbatim. Both shapes hit the same DB column in
    // the same `enc:base64iv:base64ct` form. The Phase 6b OAuth
    // two-step (`oauth.exchange_code`) lands tokens via this same
    // path on the initial-create branch (UI ships the returned
    // tokens back as `Plaintext`).
    let (access_token, refresh_token, imap_password, smtp_password) = match p.credentials {
        AccountCredentials::Plaintext {
            access_token,
            refresh_token,
            imap_password,
            smtp_password,
        } => {
            encrypt_optional_credentials(
                key,
                access_token,
                refresh_token,
                imap_password,
                smtp_password,
            )
            .await?
        }
        AccountCredentials::Encrypted {
            access_token,
            refresh_token,
            imap_password,
            smtp_password,
        } => (access_token, refresh_token, imap_password, smtp_password),
    };
    let create = db::db::queries_extra::CreateAccountParams {
        email: p.email,
        provider: p.provider,
        display_name: p.display_name,
        account_name: p.account_name,
        account_color: p.account_color,
        auth_method: p.auth_method,
        access_token,
        refresh_token,
        token_expires_at: p.token_expires_at,
        oauth_provider: p.oauth_provider,
        oauth_client_id: p.oauth_client_id,
        oauth_token_url: None,
        imap_host: p.imap_host,
        imap_port: p.imap_port,
        imap_security: p.imap_security,
        imap_username: p.imap_username,
        imap_password,
        smtp_host: p.smtp_host,
        smtp_port: p.smtp_port,
        smtp_security: p.smtp_security,
        smtp_username: p.smtp_username,
        smtp_password,
        jmap_url: p.jmap_url,
        accept_invalid_certs: p.accept_invalid_certs,
    };
    let id = crate::accounts::create_account_inner(&write_db, create).await?;
    serde_json::to_value(AccountCreateAck { id })
        .map_err(|e| ServiceError::Internal(e.to_string()))
}

/// Phase 6a-part-2: orchestrated account deletion.
///
/// Folds three concerns into one IPC: cancel-and-await per-account
/// runners (sync, push, calendar) so the runner-quiescence invariant
/// closes Service-side; orchestrated DB delete via
/// `delete_account_orchestrate`; external-store cleanup
/// (body / inline / search / attachment cache).
///
/// Failure policy: each external-store cleanup logs and continues
/// on error, mirroring the pre-relocation UI behaviour. The IPC
/// ack reports the cleanup counts; per-cache-file errors come back
/// as a flat `Vec<String>`. The cancel-and-await branches log + proceed
/// even if the supervisor join surfaces an error - the cancellation
/// token is already cancelled, so the runner will observe it on its
/// next checkpoint, and the alternative (block the delete) is worse.
pub(crate) async fn handle_delete(
    boot_state: &Arc<BootSharedState>,
    params: AccountDeleteParams,
) -> Result<Value, ServiceError> {
    let account_id = params.account_id;

    // Phase 8-5: flip `accounts.is_deleting = 1` BEFORE the cancel-
    // and-await flow so any concurrent SyncTick / start_account
    // request observes the flag and skips. Without this gate, a
    // SyncTick firing between the cancel-ack and the row-delete
    // could re-kick a sync against the disappearing account; the
    // cancel races the start, and the Service-side defense-in-depth
    // check in `SyncRuntime::start_account` would catch it but we'd
    // still pay the round-trip cost.
    let aid_for_flag = account_id.clone();
    if let Ok(write_db) = boot_state.write_db_state()
        && let Err(e) = write_db
            .with_conn(move |conn| {
                conn.execute(
                    "UPDATE accounts SET is_deleting = 1 WHERE id = ?1",
                    rusqlite::params![aid_for_flag],
                )
                .map(|_| ())
                .map_err(|e| format!("set is_deleting: {e}"))
            })
            .await
    {
        log::warn!(
            "account.delete: failed to set is_deleting for {account_id}: {e}; proceeding"
        );
    }

    if let Some(sync) = boot_state.sync_runtime()
        && let Err(e) = sync.cancel_account_and_await(&account_id).await
    {
        log::warn!(
            "account.delete: sync cancel-and-await({account_id}) returned error: {e}; proceeding",
        );
    }
    // Attachments roadmap Phase 4 + review: mark the account
    // cancelling and drop queued prefetch work. In-flight provider
    // calls cannot be aborted, but `run_pipeline` rechecks the
    // cancelling-set immediately before `PackStore::put` and again
    // before the `attachments.content_hash` UPDATE. A late-arriving
    // fetch either drops its bytes pre-put or tombstones the blob
    // post-put, so it can no longer outrun the `AttachmentCache`
    // step's snapshot of cached hashes.
    if let Some(prefetch) = boot_state.prefetch_runtime() {
        prefetch.cancel_account(&account_id).await;
    }
    if let Some(push) = boot_state.push_runtime() {
        let _ = push.cancel_account(&account_id).await;
    }
    if let Some(cal) = boot_state.calendar_runtime()
        && let Err(e) = cal.cancel_account_and_await(&account_id).await
    {
        log::warn!(
            "account.delete: calendar cancel-and-await({account_id}) returned error: {e}; proceeding",
        );
    }

    // Phase 6b: drive cleanup through the marker-backed path so a
    // crash mid-cleanup resumes from the next un-completed step on
    // boot. Runner-quiescence (above) doesn't need a marker because
    // there's no DB state to lose if the cancel is interrupted; the
    // markered work begins with the data-gather + cleanup steps and
    // ends with the SQLite CASCADE.
    let write_db = boot_state.write_db_state()?;
    let app_data = boot_state.app_data_dir().to_path_buf();
    let Some(sync) = boot_state.sync_runtime() else {
        return Err(ServiceError::Internal(
            "account.delete: sync runtime not installed; cannot run external-store cleanup"
                .into(),
        ));
    };
    let body_write = sync.body_write();
    let inline_write = sync.inline_write();
    let search_write = sync.search_write();

    let report = crate::accounts::delete_with_marker(
        &write_db,
        &body_write,
        &inline_write,
        &search_write,
        boot_state.pack_store(),
        &app_data,
        account_id.clone(),
    )
    .await
    .map_err(ServiceError::Internal)?;

    let ack = AccountDeleteAck {
        bodies_deleted: report.bodies_deleted,
        inline_images_deleted: report.inline_images_deleted,
        cache_files_deleted: report.cache_files_deleted,
        cache_file_errors: report.cache_file_errors,
        search_cleaned: report.search_cleaned,
    };

    log::info!(
        "account.delete({account_id}): {} bodies, {} inline images, {} cache files; \
         search_cleaned={}",
        ack.bodies_deleted,
        ack.inline_images_deleted,
        ack.cache_files_deleted,
        ack.search_cleaned,
    );
    if !ack.cache_file_errors.is_empty() {
        log::warn!(
            "account.delete({account_id}): {} cache file errors",
            ack.cache_file_errors.len(),
        );
    }

    serde_json::to_value(ack).map_err(|e| ServiceError::Internal(e.to_string()))
}
