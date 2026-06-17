//! `oauth.exchange_code` wire types (Phase 6b).
//!
//! Two-step OAuth: UI binds a local listener, builds the
//! authorization URL, opens the browser, and waits for the OS to
//! redirect with `?code=...&state=...`. The bind / open / wait
//! steps stay UI-side because the listener has to live in the
//! visible app (firewall + focus). Once the UI has the code, this
//! IPC ships it Service-side; the Service runs the token-endpoint
//! round-trip + userinfo round-trip and returns the resulting
//! tokens + email + name (or, in re-auth mode, persists the new
//! tokens onto the existing account row before returning).
//!
//! The auth code is a one-shot bearer credential. The wire types
//! wrap it in `RedactedString` so a stray `format!("{:?}")` or
//! `format!("{}")` cannot leak it; both `Debug` and `Display`
//! redact.

use serde::{Deserialize, Serialize};

use crate::redacted::RedactedString;

/// `oauth.exchange_code` request body. Mirrors the existing
/// `OAuthProviderAuthorizationRequest` shape that the UI was
/// already constructing for `authorize_with_provider` - the
/// Service-side handler reuses the same `GenericOAuthProvider`
/// machinery, so userinfo dispatch (Microsoft hard-coded vs
/// `user_info_url`) keys on `provider_id` exactly as it did
/// pre-Phase-6b.
///
/// `reauth_account_id`:
/// - `None` (initial create): handler returns the tokens + userinfo
///   for the UI to feed into the existing `account.create` IPC after
///   the Identity step. No DB write happens inside this IPC.
/// - `Some(id)` (re-auth): handler runs the token-exchange + userinfo
///   round-trips and persists the new tokens via
///   `update_account_tokens_sync` on the named account row. Replaces
///   the UI-side `with_write_conn` token persists from
///   `add_account/{state,oauth}.rs` (deferred from 6a-part-2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OauthExchangeCodeParams {
    /// Provider id ("google" / "microsoft" / "fastmail" / "oidc:..."
    /// etc.). `GenericOAuthProvider::from_request` keys on this for
    /// userinfo dispatch (Microsoft hits a hard-coded endpoint;
    /// everything else uses `user_info_url`).
    pub provider_id: String,
    pub token_url: String,
    pub scopes: Vec<String>,
    pub user_info_url: Option<String>,
    pub use_pkce: bool,
    pub client_id: String,
    pub client_secret: Option<RedactedString>,
    pub redirect_uri: String,
    pub code: RedactedString,
    pub code_verifier: Option<String>,
    pub reauth_account_id: Option<String>,
}

/// `oauth.exchange_code` ack.
///
/// `email` and `display_name` always come from the userinfo
/// round-trip. `access_token` / `refresh_token` / `token_expires_at`
/// are populated only in initial-create mode (when
/// `reauth_account_id` was `None`); in re-auth mode the Service
/// already persisted those values via `update_account_tokens_sync`,
/// and re-shipping them to the UI would duplicate sensitive bytes
/// for no benefit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OauthExchangeCodeAck {
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token: Option<RedactedString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<RedactedString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_expires_at: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_params(reauth: bool) -> OauthExchangeCodeParams {
        OauthExchangeCodeParams {
            provider_id: "google".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            scopes: vec!["openid".into(), "email".into()],
            user_info_url: None,
            use_pkce: true,
            client_id: "client-id".into(),
            client_secret: None,
            redirect_uri: "http://127.0.0.1:54321/callback".into(),
            code: RedactedString::new("authcode-xyz"),
            code_verifier: Some("pkce-verifier".into()),
            reauth_account_id: if reauth { Some("acct-1".into()) } else { None },
        }
    }

    #[test]
    fn params_round_trip_create() {
        let original = sample_params(false);
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: OauthExchangeCodeParams = serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn params_round_trip_reauth() {
        let original = sample_params(true);
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: OauthExchangeCodeParams = serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn params_debug_does_not_leak_code() {
        let params = sample_params(false);
        let formatted = format!("{params:?}");
        assert!(
            !formatted.contains("authcode-xyz"),
            "Debug leaked auth code: {formatted}",
        );
    }

    #[test]
    fn ack_round_trips_create() {
        let ack = OauthExchangeCodeAck {
            email: "alice@example.com".into(),
            display_name: Some("Alice".into()),
            access_token: Some(RedactedString::new("at")),
            refresh_token: Some(RedactedString::new("rt")),
            token_expires_at: Some(1234567890),
        };
        let json = serde_json::to_value(&ack).expect("serialize");
        let recovered: OauthExchangeCodeAck = serde_json::from_value(json).expect("deserialize");
        assert_eq!(ack, recovered);
    }

    #[test]
    fn ack_round_trips_reauth_omits_tokens() {
        let ack = OauthExchangeCodeAck {
            email: "alice@example.com".into(),
            display_name: Some("Alice".into()),
            access_token: None,
            refresh_token: None,
            token_expires_at: None,
        };
        let json = serde_json::to_value(&ack).expect("serialize");
        let object = json.as_object().expect("object");
        assert!(!object.contains_key("access_token"));
        assert!(!object.contains_key("refresh_token"));
        assert!(!object.contains_key("token_expires_at"));
    }

    #[test]
    fn ack_debug_does_not_leak_tokens() {
        let ack = OauthExchangeCodeAck {
            email: "alice@example.com".into(),
            display_name: Some("Alice".into()),
            access_token: Some(RedactedString::new("supersecret-access-token")),
            refresh_token: Some(RedactedString::new("supersecret-refresh-token")),
            token_expires_at: Some(1234567890),
        };
        let formatted = format!("{ack:?}");
        assert!(
            !formatted.contains("supersecret"),
            "Debug leaked token bytes: {formatted}",
        );
    }
}
