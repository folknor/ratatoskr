//! `oauth.exchange_code` handler.
//!
//! Phase 6b moves the OAuth token-endpoint round-trip Service-side
//! via `oauth.exchange_code`. Service-side OAuth refresh predates
//! 6b - it has been Service-side since Phase 4 close-out, when the
//! planned `oauth.refresh_request` IPC was removed in favor of
//! per-provider `ensure_valid_token` helpers (jmap/graph/gmail/imap)
//! that read the row + call the token endpoint + write back. There
//! is no Phase-4 IPC for 6b to delete; refresh is already where it
//! should be. `oauth.exchange_code` joins
//! `RequestParams::bypasses_admission()` - the same admission-bypass
//! list as `health.ping` and `boot.ready` - so the OAuth round-trip
//! is not queued behind heavy traffic.
//!
//! OAuth code is a one-shot bearer credential. The wire-type wrapper
//! redacts both `Debug` and `Display`; logging frameworks reach for
//! both. After Phase 6b the auth code never reaches the UI beyond
//! the redirect handler that captures it; the IPC ships the code
//! straight to Service. The handler runs token-endpoint exchange +
//! userinfo round-trip, then either returns the tokens to the UI
//! (initial create) or verifies-and-persists them Service-side
//! (re-auth) without ever shipping them back.
//!
//! Two-IPCs-pragmatic shape: the create flow keeps today's UI
//! ordering (OAuth -> Identity -> account.create) by having this
//! handler return the tokens + email + name without a DB write.
//! `service::accounts::create_account_inner` is then called by the
//! existing `account.create` handler; the UI verifies mailbox
//! reachability (`account.verify`) before that create.
//!
//! Re-auth is different: it has no Identity step and must never let
//! the freshly-exchanged tokens cross the Service->app IPC (the
//! token-custody invariant). So the re-auth branch verifies the new
//! tokens Service-side (build the in-flight factory, `open` then
//! `close`), and only on success persists them via
//! `update_account_tokens_sync` and reattaches the resident account.
//! The ack omits every token field when `reauth_account_id` is set.

use std::sync::Arc;

use serde_json::Value;
use service_api::{OauthExchangeCodeAck, OauthExchangeCodeParams, RedactedString, ServiceError};

use crate::boot::BootSharedState;

pub(crate) async fn handle_exchange_code(
    boot_state: &Arc<BootSharedState>,
    params: Box<OauthExchangeCodeParams>,
) -> Result<Value, ServiceError> {
    let p = *params;

    // Build the provider config for the token-exchange + userinfo
    // round-trips. The UI shipped the same fields it would have used
    // locally for `authorize_with_provider`; we just construct the
    // trait impl Service-side. `provider_id` keys userinfo dispatch
    // (Microsoft hard-coded URL vs `user_info_url`) exactly as it
    // did pre-Phase-6b.
    let provider = rtsk::oauth::GenericOAuthProvider::from_request(
        rtsk::oauth::OAuthProviderAuthorizationRequest {
            provider_id: p.provider_id,
            // auth_url is unused on the Service side (we only token-
            // exchange + fetch_user_info); pass a placeholder.
            auth_url: String::new(),
            token_url: p.token_url,
            scopes: p.scopes,
            user_info_url: p.user_info_url,
            use_pkce: p.use_pkce,
            client_id: p.client_id,
            client_secret: p.client_secret.map(RedactedString::into_inner),
        },
    );

    let auth = rtsk::oauth::OAuthAuthorizationFlow {
        code: p.code.into_inner(),
        redirect_uri: p.redirect_uri,
        code_verifier: p.code_verifier,
    };

    let bundle = rtsk::oauth::exchange_code_with_provider(&provider, auth)
        .await
        .map_err(ServiceError::Internal)?;

    let email = bundle.user_info.email.clone();
    let display_name = if bundle.user_info.name.is_empty() {
        None
    } else {
        Some(bundle.user_info.name.clone())
    };
    #[allow(clippy::cast_possible_wrap)]
    let token_expires_at = chrono::Utc::now().timestamp() + bundle.tokens.expires_in as i64;

    if let Some(account_id) = p.reauth_account_id {
        // Re-auth (option b): verify-before-persist runs SERVICE-SIDE, and
        // the freshly-exchanged tokens NEVER cross the Service->app IPC. We
        // build an in-flight factory from the new tokens + the persisted row
        // config, prove it can open the mailbox, and only then persist +
        // reattach. On any verify failure we leave the old credentials
        // intact (no DB write) and surface an error. The ack below omits
        // every token field in all cases (token-custody invariant).
        let read_db = boot_state.read_db_state().ok_or_else(|| {
            ServiceError::Internal(
                "db not available; UI must wait for boot.ready before calling \
                 oauth.exchange_code"
                    .into(),
            )
        })?;
        let write_db = boot_state.write_db_state()?;
        let key = boot_state.encryption_key().ok_or_else(|| {
            ServiceError::Internal(
                "encryption key not loaded; UI must wait for boot.ready before calling \
                 oauth.exchange_code"
                    .into(),
            )
        })?;

        // Verify-before-persist: prove the NEW tokens open the mailbox.
        let factory = crate::bifrost::factory::build_reauth_verify_factory(
            &read_db,
            &account_id,
            key,
            bundle.tokens.access_token.clone(),
        )
        .await
        .map_err(|error| {
            ServiceError::Internal(
                crate::handlers::account_verify::build_error_message(&error).to_string(),
            )
        })?;
        let synthetic_id =
            bifrost_types::AccountId(format!("reauth-verify-{}", uuid::Uuid::new_v4()));
        match factory.open(synthetic_id).await {
            Ok(account) => {
                let _ = account.close().await;
            }
            Err(error) => {
                log::warn!(
                    "oauth.exchange_code: re-auth verify failed for account {account_id}; \
                     leaving existing credentials intact"
                );
                return Err(ServiceError::Internal(
                    crate::bifrost::error_map::account_error_to_action_error(&error).user_message(),
                ));
            }
        }

        // Verify succeeded: persist the new tokens onto the existing row.
        // Tokens encrypt at the handler boundary (same key + path as
        // `account.update_tokens`).
        let (access_token, refresh_token, _, _) =
            crate::handlers::account::encrypt_optional_credentials(
                key,
                Some(bundle.tokens.access_token),
                bundle.tokens.refresh_token,
                None,
                None,
            )
            .await?;
        let reauth = db::db::queries_extra::ReauthAccountParams {
            access_token,
            refresh_token,
            token_expires_at: Some(token_expires_at),
            imap_password: None,
            smtp_password: None,
        };
        let id_for_log = account_id.clone();
        let account_id_write = account_id.clone();
        write_db
            .with_write(move |conn| {
                db::db::queries_extra::update_account_tokens_sync(conn, &account_id_write, reauth)
            })
            .await
            .map_err(ServiceError::Internal)?;
        log::info!("oauth.exchange_code: re-auth tokens persisted for account {id_for_log}");

        if let Some(sync_runtime) = boot_state.sync_runtime() {
            let aid = id_for_log.clone();
            tokio::spawn(async move {
                if let Err(e) = sync_runtime.detach_resident_account(&aid).await {
                    log::debug!("oauth.exchange_code: resident detach before reattach {aid}: {e}");
                }
                if let Err(e) = sync_runtime.attach_resident_account(&aid).await {
                    log::warn!("oauth.exchange_code: resident reattach {aid} failed: {e}");
                }
            });
        }

        return serde_json::to_value(OauthExchangeCodeAck {
            email,
            display_name,
            access_token: None,
            refresh_token: None,
            token_expires_at: None,
        })
        .map_err(|e| ServiceError::Internal(e.to_string()));
    }

    // Initial create: ship the tokens back to the UI. UI runs
    // Identity step with `display_name` prefilled, then ships
    // account.create with the returned tokens.
    serde_json::to_value(OauthExchangeCodeAck {
        email,
        display_name,
        access_token: Some(RedactedString::new(bundle.tokens.access_token)),
        refresh_token: bundle.tokens.refresh_token.map(RedactedString::new),
        token_expires_at: Some(token_expires_at),
    })
    .map_err(|e| ServiceError::Internal(e.to_string()))
}
