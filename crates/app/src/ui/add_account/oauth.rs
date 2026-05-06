use iced::widget::{Space, column, text};
use iced::{Alignment, Element, Length, Task};

use crate::font;
use crate::ui::layout::*;

use super::state::{
    AddAccountEvent, AddAccountMessage, AddAccountStep, AddAccountWizard, OAuthSuccess,
};
use super::views::{ghost_button, primary_button};

impl AddAccountWizard {
    /// Start the OAuth flow for re-auth, using the stored provider info.
    pub(super) fn start_reauth_oauth(
        &mut self,
        oauth_provider: Option<&str>,
        oauth_client_id: Option<&str>,
    ) -> Task<AddAccountMessage> {
        let provider_id = oauth_provider.unwrap_or("").to_string();
        let client_id = if oauth_client_id.is_some_and(|c| !c.is_empty()) {
            oauth_client_id.expect("checked").to_string()
        } else {
            resolve_client_id(&provider_id)
        };

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
        let client_id_clone = client_id.clone();

        Task::perform(
            async move {
                // Resolve endpoints: either from registry or OIDC discovery.
                let (auth_url, token_url, scopes, use_pkce) = if let Some(r) = resolved {
                    r
                } else if let Some(issuer) = oidc_issuer {
                    let endpoints = rtsk::discovery::oidc::probe_issuer(&issuer)
                        .await
                        .ok_or_else(|| {
                            format!(
                                "OIDC discovery failed for issuer \
                                         '{issuer}'"
                            )
                        })?;
                    (
                        endpoints.auth_url,
                        endpoints.token_url,
                        endpoints.scopes,
                        endpoints.supports_pkce_s256,
                    )
                } else {
                    return Err("No OAuth configuration available".into());
                };

                let request = rtsk::oauth::OAuthProviderAuthorizationRequest {
                    provider_id: provider_id_clone.clone(),
                    auth_url,
                    token_url,
                    scopes,
                    user_info_url: None,
                    use_pkce,
                    client_id: client_id_clone.clone(),
                    client_secret: None,
                };

                let provider = rtsk::oauth::GenericOAuthProvider::from_request(request);
                let open_url = |url: &str| -> Result<(), String> { open_browser_url(url) };
                let result = rtsk::oauth::authorize_with_provider(&provider, &open_url).await;
                let mapped = result.map(|bundle| {
                    #[allow(clippy::cast_possible_wrap)]
                    let expires_at =
                        chrono::Utc::now().timestamp() + bundle.tokens.expires_in as i64;
                    OAuthSuccess {
                        access_token: bundle.tokens.access_token,
                        refresh_token: bundle.tokens.refresh_token,
                        token_expires_at: Some(expires_at),
                        user_email: bundle.user_info.email,
                        user_name: bundle.user_info.name,
                        oauth_provider: provider_id_clone,
                        oauth_client_id: client_id_clone,
                    }
                });
                Ok(mapped)
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
        // Re-auth mode: save tokens directly via
        // `account.update_tokens` IPC, skip identity step.
        if let Some(ref account_id) = self.reauth_account_id {
            let Some(client) = self.service_client.as_ref().cloned() else {
                self.error = Some("Service not ready".into());
                return (Task::none(), None);
            };
            let params = service_api::AccountUpdateTokensParams {
                account_id: account_id.clone(),
                access_token: Some(service_api::RedactedString::new(success.access_token.clone())),
                refresh_token: success
                    .refresh_token
                    .clone()
                    .map(service_api::RedactedString::new),
                token_expires_at: success.token_expires_at,
                imap_password: None,
                smtp_password: None,
            };
            let generation = self.generation.next();
            let task = Task::perform(
                async move {
                    let result = client
                        .update_account_tokens(params)
                        .await
                        .map_err(|e| e.to_string());
                    (generation, result)
                },
                |(g, result)| AddAccountMessage::ReauthTokensSaved(g, result),
            );
            return (task, None);
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

pub(super) fn resolve_client_id(provider_id: &str) -> String {
    match provider_id {
        "microsoft" | "microsoft_graph" => rtsk::oauth::MICROSOFT_DEFAULT_CLIENT_ID.to_string(),
        // For Google, the client_id is typically embedded in the app.
        // If not available, the OAuth flow will use the discovery registry value.
        _ => String::new(),
    }
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
