use iced::widget::{Space, column, text};
use iced::{Alignment, Element, Length, Task};

use crate::font;
use crate::ui::layout::*;

use super::state::{
    AddAccountEvent, AddAccountMessage, AddAccountStep, AddAccountWizard, OAuthSuccess,
};
use super::views::{ghost_button, primary_button};

/// Redirect URI passed to IdPs during RFC 7591 dynamic client registration.
///
/// The actual loopback listener bound by `rtsk::oauth::run_oauth_authorization_flow`
/// formats its redirect URI as `http://127.0.0.1:{actual_port}` (no path
/// suffix). Defaults to port 17248 - `bind_oauth_listener` tries that
/// first - so registering with the same string maximises IdP match
/// probability. If the IdP enforces an exact match and the runtime port
/// differs (17248 already taken), we fall through to the IdP's reject
/// path; in practice most loopback-friendly IdPs accept any 127.0.0.1
/// port per RFC 8252 §7.3.
const OAUTH_REDIRECT_URI: &str = "http://127.0.0.1:17248";

impl AddAccountWizard {
    /// Start the OAuth flow for re-auth, using the stored provider info.
    ///
    /// `oauth_extra_scopes` is the raw space-separated value from
    /// `accounts.oauth_extra_scopes`; parsed and merged with whatever
    /// scope set discovery or the registry produced for this provider.
    /// Pass `None` for new-account flows where no account row exists yet.
    pub(super) fn start_reauth_oauth(
        &mut self,
        oauth_provider: Option<&str>,
        oauth_client_id: Option<&str>,
        oauth_extra_scopes: Option<&str>,
    ) -> Task<AddAccountMessage> {
        let provider_id = oauth_provider.unwrap_or("").to_string();
        let client_id = if oauth_client_id.is_some_and(|c| !c.is_empty()) {
            oauth_client_id.expect("checked").to_string()
        } else {
            resolve_client_id(&provider_id)
        };
        let extras = parse_extra_scopes(oauth_extra_scopes);

        // Look up the full OAuth config from the discovery registry.
        let oauth_config = rtsk::discovery::registry::oauth_config_for_provider(&provider_id);

        // For built-in providers, we can resolve endpoints synchronously.
        // For generic OIDC providers (oidc:https://...), we need to
        // re-discover endpoints from the issuer URL at runtime.
        let resolved = match oauth_config {
            Some(rtsk::discovery::types::AuthMethod::OAuth2 {
                auth_url,
                token_url,
                scopes,
                use_pkce,
                ..
            }) => Some((auth_url, token_url, scopes, use_pkce)),
            _ => None,
        };

        // If not in registry and not an OIDC provider, fall back to password.
        let oidc_issuer = if resolved.is_none() {
            if let Some(issuer) = provider_id.strip_prefix("oidc:") {
                Some(issuer.to_string())
            } else {
                self.step = AddAccountStep::PasswordAuth;
                self.error = Some(format!(
                    "No OAuth configuration found for provider \
                     '{provider_id}'. Please enter credentials manually."
                ));
                return Task::none();
            }
        } else {
            None
        };

        self.step = AddAccountStep::OAuthWaiting;
        self.error = None;
        let generation = self.generation.next();
        let provider_id_clone = provider_id.clone();
        let initial_client_id = client_id.clone();

        let Some(client) = self.service_client.as_ref().cloned() else {
            self.error = Some("Service not ready".into());
            return Task::none();
        };
        let reauth_account_id = self.reauth_account_id.clone();
        Task::perform(
            async move {
                // Resolve endpoints: either from the registry (built-in
                // providers - Gmail / Microsoft 365) or from OIDC
                // discovery against a user-supplied issuer URL.
                //
                // We carry `registration_endpoint` through the OIDC
                // discovery path so a downstream RFC 7591 dynamic
                // registration attempt can use it. Registry-resolved
                // providers don't get dyn-registration; their client IDs
                // are baked in.
                let (
                    auth_url,
                    token_url,
                    scopes,
                    use_pkce,
                    registration_endpoint,
                    supports_public_client,
                ) = if let Some(r) = resolved {
                    (r.0, r.1, r.2, r.3, None, true)
                } else if let Some(issuer) = oidc_issuer {
                    let endpoints = rtsk::discovery::oidc::probe_issuer(&issuer)
                        .await
                        .ok_or_else(|| format!("OIDC discovery failed for issuer '{issuer}'"))?;
                    (
                        endpoints.auth_url,
                        endpoints.token_url,
                        endpoints.scopes,
                        endpoints.supports_pkce_s256,
                        endpoints.registration_endpoint,
                        endpoints.supports_public_client,
                    )
                } else {
                    return Err::<Result<OAuthSuccess, String>, String>(
                        "No OAuth configuration available".into(),
                    );
                };

                // RFC 7591 dynamic registration: when the user didn't
                // provide a client_id and the IdP advertised a
                // registration endpoint, register Ratatoskr as a public
                // client and use the resulting credentials.
                let (resolved_client_id, resolved_client_secret) = if initial_client_id.is_empty() {
                    if let Some(endpoint) = registration_endpoint.as_deref() {
                        if !supports_public_client {
                            return Err::<Result<OAuthSuccess, String>, String>(
                                "Provider does not support public clients; supply a Client ID."
                                    .into(),
                            );
                        }
                        let scope = scopes.join(" ");
                        let req = rtsk::discovery::dyn_registration::public_client_request(
                            OAUTH_REDIRECT_URI,
                            &scope,
                        );
                        match rtsk::discovery::dyn_registration::register(endpoint, &req).await {
                            Some(registered) => {
                                log::info!(
                                    "OAuth: dyn-registered client_id={} for {}",
                                    registered.client_id,
                                    provider_id_clone,
                                );
                                (registered.client_id, registered.client_secret)
                            }
                            None => {
                                return Err::<Result<OAuthSuccess, String>, String>(
                                    "Dynamic client registration failed; supply a Client ID \
                                     manually."
                                        .into(),
                                );
                            }
                        }
                    } else {
                        return Err::<Result<OAuthSuccess, String>, String>(
                            "No Client ID supplied and the provider does not advertise a \
                             registration endpoint."
                                .into(),
                        );
                    }
                } else {
                    (initial_client_id, None)
                };

                let merged_scopes = merge_scopes(&scopes, &extras);
                let provider_id_for_success = provider_id_clone.clone();
                let client_id_for_success = resolved_client_id.clone();
                let client_secret_for_success = resolved_client_secret.clone();
                let result = run_capture_then_exchange(
                    &client,
                    OauthCaptureConfig {
                        provider_id: provider_id_clone,
                        auth_url,
                        token_url,
                        scopes: merged_scopes,
                        user_info_url: None,
                        use_pkce,
                        client_id: resolved_client_id,
                        client_secret: resolved_client_secret,
                    },
                    reauth_account_id,
                )
                .await
                .map(|ack| {
                    let access_token = ack
                        .access_token
                        .map(service_api::RedactedString::into_inner)
                        .unwrap_or_default();
                    OAuthSuccess {
                        access_token,
                        refresh_token: ack
                            .refresh_token
                            .map(service_api::RedactedString::into_inner),
                        token_expires_at: ack.token_expires_at,
                        user_email: ack.email,
                        user_name: ack.display_name.unwrap_or_default(),
                        oauth_provider: provider_id_for_success,
                        oauth_client_id: client_id_for_success,
                        oauth_client_secret: client_secret_for_success,
                    }
                });
                Ok(result)
            },
            move |result: Result<Result<OAuthSuccess, String>, String>| match result {
                Ok(inner) => AddAccountMessage::OAuthComplete(generation, inner),
                Err(e) => AddAccountMessage::OAuthComplete(generation, Err(e)),
            },
        )
    }

    pub(super) fn handle_oauth_success(
        &mut self,
        success: OAuthSuccess,
    ) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        // Phase 6b: re-auth tokens now persist Service-side inside
        // `oauth.exchange_code` (when `reauth_account_id` is set).
        // The IPC ack carries empty token fields in re-auth mode and
        // run_capture_then_exchange surfaced this back as an
        // OAuthSuccess with empty access_token. Drop the empty
        // success and dispatch a synthetic ReauthTokensSaved.
        if self.reauth_account_id.is_some() {
            let generation = self.generation.next();
            let _ = success;
            return (
                Task::done(AddAccountMessage::ReauthTokensSaved(generation, Ok(()))),
                None,
            );
        }

        self.oauth_success = Some(success);
        self.prefill_identity_name();
        self.step = AddAccountStep::Identity;
        self.error = None;
        (Task::none(), None)
    }

    pub(super) fn handle_retry_oauth(
        &mut self,
    ) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        // Re-auth mode: re-run using stored provider info
        if self.reauth_account_id.is_some() {
            self.error = None;
            // Look up auth info again for the retry
            let aid = self.reauth_account_id.clone().unwrap_or_default();
            let auth_info = self.db.get_account_auth_info(&aid);
            match auth_info {
                Ok(info) => {
                    let task = self.start_reauth_oauth(
                        info.oauth_provider.as_deref(),
                        info.oauth_client_id.as_deref(),
                        info.oauth_extra_scopes.as_deref(),
                    );
                    return (task, None);
                }
                Err(e) => {
                    self.error = Some(format!("Failed to look up account: {e}"));
                    return (Task::none(), None);
                }
            }
        }

        // Re-run the OAuth flow using the stored discovery config
        let config = match &self.discovery {
            Some(c) => c.clone(),
            None => return (Task::none(), None),
        };
        let idx = self.selected_option.unwrap_or(0);
        let Some(option) = config.options.get(idx) else {
            return (Task::none(), None);
        };
        self.error = None;
        self.proceed_to_auth(option)
    }

    pub(super) fn view_oauth_waiting(&self) -> Element<'_, AddAccountMessage> {
        let mut col = column![]
            .spacing(SPACE_XS)
            .align_x(Alignment::Center)
            .width(Length::Fill);

        let heading = if self.reauth_account_id.is_some() {
            "Re-authenticate in your browser"
        } else {
            "Complete sign-in in your browser"
        };

        col = col.push(
            text(heading)
                .size(TEXT_HEADING)
                .style(text::base)
                .font(iced::Font {
                    weight: iced::font::Weight::Bold,
                    ..font::text()
                }),
        );

        // Show the account email for re-auth context
        if self.reauth_account_id.is_some() {
            col = col.push(text(&self.email).size(TEXT_LG).style(text::secondary));
        }
        col = col.push(Space::new().height(SPACE_MD));
        col = col.push(
            text("Waiting for authorization...")
                .size(TEXT_LG)
                .style(text::secondary),
        );

        if let Some(ref err) = self.error {
            col = col.push(Space::new().height(SPACE_SM));
            col = col.push(text(err.as_str()).size(TEXT_SM).style(text::danger));
            col = col.push(Space::new().height(SPACE_SM));
            col = col.push(primary_button("Retry", AddAccountMessage::RetryOAuth));
        }

        col = col.push(Space::new().height(SPACE_LG));
        col = col.push(ghost_button("Cancel", AddAccountMessage::CancelOAuth));

        col.into()
    }
}

/// Parse the space-separated `oauth_extra_scopes` DB column into a vector.
///
/// Empty / whitespace-only input returns an empty vector. RFC 6749 §3.3
/// scope names cannot contain whitespace, so splitting on whitespace is
/// safe and matches the OAuth wire format.
pub(super) fn parse_extra_scopes(raw: Option<&str>) -> Vec<String> {
    raw.map(|s| s.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default()
}

/// Merge negotiated OAuth scopes with extras, preserving order and
/// deduplicating. Negotiated scopes keep their original order; extras
/// not already present are appended in input order.
pub(super) fn merge_scopes(negotiated: &[String], extras: &[String]) -> Vec<String> {
    let mut merged: Vec<String> = negotiated.to_vec();
    for s in extras {
        if !merged.iter().any(|existing| existing == s) {
            merged.push(s.clone());
        }
    }
    merged
}

/// Return a usable client_id for an OIDC provider, registering one via
/// RFC 7591 dynamic client registration when none is provided and the
/// discovery document advertised a `registration_endpoint`.
///
/// Returns `None` when:
/// - the provided client_id is non-empty (caller already has one, no
///   registration needed) - caller should use the original.
/// - the discovery document has no `registration_endpoint`.
/// - the provider doesn't support public clients (no `none` auth method).
/// - the registration request itself failed (network, parse, etc.).
///
/// Returns `Some(client)` on a successful registration. Callers should
/// persist `client.client_id` (and `client.client_secret` if any) onto
/// the account row before continuing with the auth flow.
///
/// This helper is not invoked by any current code path - the wizard
/// surface for a "Custom OIDC" provider that would need it is gated on
/// the widget-definition work tracked in `docs/focus/problem-statement.md`.
/// It exists so the future wizard call site has one helper to reach for.
#[allow(dead_code)]
pub(super) async fn resolve_or_register_client(
    endpoints: &rtsk::discovery::oidc::OidcEndpoints,
    provided_client_id: &str,
    redirect_uri: &str,
) -> Option<rtsk::discovery::dyn_registration::RegisteredClient> {
    if !provided_client_id.is_empty() {
        return None;
    }
    let endpoint = endpoints.registration_endpoint.as_deref()?;
    if !endpoints.supports_public_client {
        log::debug!(
            "dyn_registration: skipping for {} - provider does not support public clients",
            endpoints.issuer_url
        );
        return None;
    }
    let scope = endpoints.scopes.join(" ");
    let request = rtsk::discovery::dyn_registration::public_client_request(redirect_uri, &scope);
    rtsk::discovery::dyn_registration::register(endpoint, &request).await
}

pub(super) fn resolve_client_id(provider_id: &str) -> String {
    match provider_id {
        "microsoft" | "microsoft_graph" => rtsk::oauth::MICROSOFT_DEFAULT_CLIENT_ID.to_string(),
        // For Google, the client_id is typically embedded in the app.
        // If not available, the OAuth flow will use the discovery registry value.
        _ => String::new(),
    }
}

/// Phase 6b: provider config the UI ships with the captured auth
/// code. Mirrors `OAuthProviderAuthorizationRequest` so today's
/// userinfo dispatch (Microsoft hard-coded URL vs `user_info_url`)
/// keys on `provider_id` exactly as it did pre-Phase-6b.
///
/// `client_secret` is sensitive; the manual `Debug` impl prints
/// `<redacted>` so log lines that capture this struct don't leak
/// it. Wire transport already wraps the value in `RedactedString`;
/// this keeps UI-side debug printing consistent with that policy.
#[derive(Clone)]
pub(super) struct OauthCaptureConfig {
    pub provider_id: String,
    pub auth_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
    pub user_info_url: Option<String>,
    pub use_pkce: bool,
    pub client_id: String,
    pub client_secret: Option<String>,
}

impl std::fmt::Debug for OauthCaptureConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OauthCaptureConfig")
            .field("provider_id", &self.provider_id)
            .field("auth_url", &self.auth_url)
            .field("token_url", &self.token_url)
            .field("scopes", &self.scopes)
            .field("user_info_url", &self.user_info_url)
            .field("use_pkce", &self.use_pkce)
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// Phase 6b: capture the OAuth auth code locally, then ship it
/// Service-side via `oauth.exchange_code`. Two-IPCs-pragmatic shape
/// (see `service/src/handlers/oauth.rs`): the Service runs the
/// token-endpoint + userinfo round-trips and either returns the
/// tokens (initial create; UI proceeds to Identity +
/// `account.create`) or persists the tokens onto the existing row
/// (re-auth, when `reauth_account_id` is set).
pub(super) async fn run_capture_then_exchange(
    client: &crate::service_client::ServiceClient,
    config: OauthCaptureConfig,
    reauth_account_id: Option<String>,
) -> Result<service_api::OauthExchangeCodeAck, String> {
    let request = rtsk::oauth::OAuthProviderAuthorizationRequest {
        provider_id: config.provider_id.clone(),
        auth_url: config.auth_url,
        token_url: config.token_url.clone(),
        scopes: config.scopes.clone(),
        user_info_url: config.user_info_url.clone(),
        use_pkce: config.use_pkce,
        client_id: config.client_id.clone(),
        client_secret: config.client_secret.clone(),
    };
    let provider = rtsk::oauth::GenericOAuthProvider::from_request(request);
    let open_url = |url: &str| -> Result<(), String> { open_browser_url(url) };
    let auth_request = <rtsk::oauth::GenericOAuthProvider as rtsk::oauth::OAuthIdentityProvider>::authorization_request(&provider);
    let auth = rtsk::oauth::run_oauth_authorization_flow(auth_request, &open_url).await?;

    let params = service_api::OauthExchangeCodeParams {
        provider_id: config.provider_id,
        token_url: config.token_url,
        scopes: config.scopes,
        user_info_url: config.user_info_url,
        use_pkce: config.use_pkce,
        client_id: config.client_id,
        client_secret: config.client_secret.map(service_api::RedactedString::new),
        redirect_uri: auth.redirect_uri,
        code: service_api::RedactedString::new(auth.code),
        code_verifier: auth.code_verifier,
        reauth_account_id,
    };
    client
        .exchange_oauth_code(params)
        .await
        .map_err(|e| e.to_string())
}

pub(super) fn open_browser_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("Failed to open browser: {e}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("Failed to open browser: {e}"))?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", url])
            .spawn()
            .map_err(|e| format!("Failed to open browser: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn parse_extra_scopes_none_or_empty() {
        assert!(parse_extra_scopes(None).is_empty());
        assert!(parse_extra_scopes(Some("")).is_empty());
        assert!(parse_extra_scopes(Some("   ")).is_empty());
    }

    #[test]
    fn parse_extra_scopes_splits_whitespace() {
        assert_eq!(parse_extra_scopes(Some("a b c")), v(&["a", "b", "c"]));
        // Multiple spaces / tabs / mixed whitespace
        assert_eq!(
            parse_extra_scopes(Some("a\tb  c\n d")),
            v(&["a", "b", "c", "d"])
        );
    }

    #[test]
    fn merge_scopes_empty_extras_returns_negotiated() {
        assert_eq!(
            merge_scopes(&v(&["openid", "email"]), &[]),
            v(&["openid", "email"])
        );
    }

    #[test]
    fn merge_scopes_appends_disjoint_extras() {
        assert_eq!(
            merge_scopes(&v(&["openid"]), &v(&["offline_access"])),
            v(&["openid", "offline_access"])
        );
    }

    #[test]
    fn merge_scopes_dedups_overlap() {
        // "openid" present in both - extras don't double it.
        assert_eq!(
            merge_scopes(&v(&["openid", "email"]), &v(&["openid", "profile"])),
            v(&["openid", "email", "profile"])
        );
    }

    #[test]
    fn merge_scopes_preserves_negotiated_order() {
        assert_eq!(
            merge_scopes(&v(&["c", "a", "b"]), &v(&["d", "a"])),
            v(&["c", "a", "b", "d"])
        );
    }

    #[test]
    fn merge_scopes_empty_negotiated_uses_extras() {
        assert_eq!(
            merge_scopes(&[], &v(&["custom:scope"])),
            v(&["custom:scope"])
        );
    }
}
