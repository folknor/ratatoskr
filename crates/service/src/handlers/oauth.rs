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
//! userinfo round-trip, then either updates the existing row's
//! tokens (re-auth, when `reauth_account_id` is set) or returns the
//! tokens + userinfo for the UI to ship to `account.create` after
//! the Identity step.
//!
//! Two-IPCs-pragmatic shape: the create flow keeps today's UI
//! ordering (OAuth -> Identity -> account.create) by having this
//! handler return the tokens + email + name without a DB write.
//! `service::accounts::create_account_inner` is then called by the
//! existing `account.create` handler. The re-auth flow folds the
//! token-persist into this handler because re-auth has no Identity
//! step; the ack omits token fields when `reauth_account_id` is set.

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
        // Re-auth: persist the new tokens onto the existing row,
        // omit token fields from the ack. Replaces the UI-side
        // with_write_conn callers in add_account/{state,oauth}.rs.
        // Tokens encrypt at the handler boundary (same key + path
        // as `account.update_tokens`).
        let write_db = boot_state.write_db_state()?;
        let key = boot_state.encryption_key().ok_or_else(|| {
            ServiceError::Internal(
                "encryption key not loaded; UI must wait for boot.ready before calling \
                 oauth.exchange_code"
                    .into(),
            )
        })?;
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
        write_db
            .with_write(move |conn| {
                db::db::queries_extra::update_account_tokens_sync(conn, &account_id, reauth)
            })
            .await
            .map_err(ServiceError::Internal)?;
        log::info!("oauth.exchange_code: re-auth tokens persisted for account {id_for_log}");

        // Phase 8-3: re-arm push for the re-authed account. Without
        // this, the dead websocket bridge produced by the
        // pre-re-auth token revocation stays dead until the Service
        // restarts. `start_account` silently skips non-JMAP accounts;
        // `fresh_start: true` clears the persisted `push_state`
        // because the new session may not honour the old session's
        // cursor. Spawned per-account so a slow handshake doesn't
        // block the OAuth exchange ack.
        if let Some(push_runtime) = boot_state.push_runtime() {
            let aid = id_for_log.clone();
            tokio::spawn(async move {
                if let Err(e) = push_runtime.start_account(aid.clone(), true).await {
                    log::warn!("[push] re-auth start_account({aid}) failed: {e}");
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
