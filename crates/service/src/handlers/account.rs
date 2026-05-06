//! Account write handlers (Phase 6a).
//!
//! `account.update` and `account.reorder` are the small / non-envelope
//! surfaces. The bigger account operations live in their own modules:
//!
//! - `account.create` (Plaintext | Encrypted credential envelope)
//! - `account.delete` (cancel-and-await runner orchestration)
//!
//! ...both will land alongside their own ack types and timeouts so
//! the handler doc here stays scoped to the simple-write path.
//!
//! Today's caldav_password column stores the value verbatim (no
//! encryption); when `internal.encrypt_for_storage` lands the wire
//! shape stays unchanged but this handler can route the value through
//! the cipher before writing.

use std::sync::Arc;

use serde_json::Value;
use service_api::{
    AccountCreateAck, AccountCreateParams, AccountDeleteAck, AccountDeleteParams, AccountReorderAck,
    AccountReorderParams, AccountUpdateAck, AccountUpdateParams, ServiceError,
};

use crate::boot::BootSharedState;

pub(crate) async fn handle_update(
    boot_state: &Arc<BootSharedState>,
    params: AccountUpdateParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
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
            };
            db::db::queries_extra::update_account_sync(conn, &id, update)
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(AccountUpdateAck)
        .map_err(|e| ServiceError::Internal(e.to_string()))
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

pub(crate) async fn handle_create(
    boot_state: &Arc<BootSharedState>,
    params: Box<AccountCreateParams>,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    let id = write_db
        .with_conn(move |conn| {
            // Today's behavior: both Plaintext and Encrypted variants
            // pass through to create_account_sync verbatim, because
            // the underlying DB column does not require ciphertext.
            // When `internal.encrypt_for_storage` lands the handler
            // will branch here on the variant tag and run Plaintext
            // through the cipher first.
            let p = *params;
            let (access_token, refresh_token, imap_password, smtp_password) =
                p.credentials.into_fields();
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
            db::db::queries_extra::create_account_sync(conn, &create)
        })
        .await
        .map_err(ServiceError::Internal)?;
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

    if let Some(sync) = boot_state.sync_runtime()
        && let Err(e) = sync.cancel_account_and_await(&account_id).await
    {
        log::warn!(
            "account.delete: sync cancel-and-await({account_id}) returned error: {e}; proceeding",
        );
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

    let write_db = boot_state.write_db_state()?;
    let plan_account_id = account_id.clone();
    let plan = write_db
        .with_conn(move |conn| {
            rtsk::account::delete::delete_account_orchestrate(conn, &plan_account_id)
        })
        .await
        .map_err(ServiceError::Internal)?;

    let mut ack = AccountDeleteAck::default();
    let message_ids = plan.data.message_ids;
    let cached_files = plan.data.cached_files;
    let inline_hashes = plan.data.inline_hashes;
    let shared_inline_hashes = plan.shared_inline_hashes;
    let shared_cache_hashes = plan.shared_cache_hashes;

    if let Some(sync) = boot_state.sync_runtime() {
        let body_write = sync.body_write();
        match body_write.delete(message_ids.clone()).await {
            Ok(n) => ack.bodies_deleted = n,
            Err(e) => log::error!("account.delete: body store cleanup: {e}"),
        }

        let to_delete: Vec<String> = inline_hashes
            .into_iter()
            .filter(|h| !shared_inline_hashes.contains(h))
            .collect();
        if !to_delete.is_empty() {
            let inline_write = sync.inline_write();
            match inline_write.delete_hashes(to_delete).await {
                Ok(n) => ack.inline_images_deleted = n,
                Err(e) => log::error!("account.delete: inline image cleanup: {e}"),
            }
        }

        let search_write = sync.search_write();
        match search_write.delete_messages_batch(message_ids).await {
            Ok(()) => ack.search_cleaned = true,
            Err(e) => log::error!("account.delete: search index cleanup: {e}"),
        }
    } else {
        log::warn!(
            "account.delete: sync runtime not installed; body / inline / search cleanup skipped \
             (boot-time invariant pass on next start will reconcile)",
        );
    }

    let app_data = boot_state.app_data_dir().to_path_buf();
    for (path, hash) in cached_files {
        if shared_cache_hashes.contains(&hash) {
            continue;
        }
        match rtsk::attachment_cache::remove_cached_relative(&app_data, &path) {
            Ok(()) => ack.cache_files_deleted += 1,
            Err(e) => ack.cache_file_errors.push(format!("{path}: {e}")),
        }
    }

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
