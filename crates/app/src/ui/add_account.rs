//! Add Account wizard — multi-step state machine and views.
//!
//! Phases 2-3 of the accounts implementation spec. The wizard handles
//! first-launch onboarding and subsequent account additions.

use std::sync::Arc;

use iced::widget::{button, column, container, row, scrollable, text, text_input, Space};
use iced::{Alignment, Element, Length, Task};

use crate::component::Component;
use crate::db::Db;
use crate::font;
use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;

use ratatoskr_core::db::queries_extra::{
    CreateAccountParams, ReauthAccountParams, account_exists_by_email_sync,
    create_account_sync, get_account_auth_info_sync, update_account_tokens_sync,
};
use ratatoskr_core::discovery::types::{
    AuthMethod, DiscoveredConfig, DiscoverySource, Protocol, ProtocolOption, Security,
};

// ── Step enum ────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddAccountStep {
    /// Email input + Continue button.
    EmailInput,
    /// Discovery is running. Spinner shown.
    Discovering,
    /// Discovery returned multiple options. User must choose.
    ProtocolSelection,
    /// Discovery failed. Manual configuration form.
    ManualConfiguration,
    /// OAuth: waiting for browser callback.
    OAuthWaiting,
    /// Password auth: IMAP/SMTP credential form.
    PasswordAuth,
    /// Validating credentials (connecting to server).
    Validating,
    /// Account identity: name + color picker.
    Identity,
    /// Account creation in progress.
    Creating,
}

// ── Security option ──────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityOption {
    Tls,
    StartTls,
    None,
}

impl SecurityOption {
    fn label(self) -> &'static str {
        match self {
            Self::Tls => "SSL/TLS",
            Self::StartTls => "STARTTLS",
            Self::None => "None",
        }
    }

    fn to_db_string(self) -> &'static str {
        match self {
            Self::Tls => "tls",
            Self::StartTls => "starttls",
            Self::None => "none",
        }
    }
}

// ── Auth state ───────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AuthState {
    pub username: String,
    pub password: String,
    pub smtp_username: String,
    pub smtp_password: String,
    pub use_separate_smtp_credentials: bool,
    pub accept_invalid_certs: bool,
    pub imap_host: String,
    pub imap_port: String,
    pub imap_security: SecurityOption,
    pub smtp_host: String,
    pub smtp_port: String,
    pub smtp_security: SecurityOption,
}

impl Default for AuthState {
    fn default() -> Self {
        Self {
            username: String::new(),
            password: String::new(),
            smtp_username: String::new(),
            smtp_password: String::new(),
            use_separate_smtp_credentials: false,
            accept_invalid_certs: false,
            imap_host: String::new(),
            imap_port: "993".to_string(),
            imap_security: SecurityOption::Tls,
            smtp_host: String::new(),
            smtp_port: "587".to_string(),
            smtp_security: SecurityOption::StartTls,
        }
    }
}

// ── Account identity ─────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AccountIdentity {
    pub name: String,
    pub selected_color_index: Option<usize>,
}

// ── OAuth success ────────────────────────────────────────

/// Successful OAuth result carrying the tokens and user info.
#[derive(Debug, Clone)]
pub struct OAuthSuccess {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_expires_at: Option<i64>,
    pub user_email: String,
    pub user_name: String,
    pub oauth_provider: String,
    pub oauth_client_id: String,
}

// ── Messages ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum AddAccountMessage {
    // Step 1: Email
    EmailChanged(String),
    SubmitEmail,

    // Step 2: Discovery result
    DiscoveryComplete(u64, Result<DiscoveredConfig, String>),
    SelectProtocol(usize),
    ConfirmProtocol,

    // Manual config
    ManualImapHostChanged(String),
    ManualImapPortChanged(String),
    ManualSmtpHostChanged(String),
    ManualSmtpPortChanged(String),
    SubmitManualConfig,

    // Step 3: Authentication
    // OAuth
    OAuthComplete(u64, Result<OAuthSuccess, String>),
    CancelOAuth,
    RetryOAuth,

    // Password
    UsernameChanged(String),
    PasswordChanged(String),
    SmtpUsernameChanged(String),
    SmtpPasswordChanged(String),
    ToggleSeparateSmtpCredentials(bool),
    ToggleAcceptInvalidCerts(bool),
    AuthImapHostChanged(String),
    AuthImapPortChanged(String),
    AuthImapSecurityChanged(SecurityOption),
    AuthSmtpHostChanged(String),
    AuthSmtpPortChanged(String),
    AuthSmtpSecurityChanged(SecurityOption),
    SubmitCredentials,
    ValidationComplete(u64, Result<(), String>),

    // Step 4: Identity
    AccountNameChanged(String),
    SelectColor(usize),
    SubmitIdentity,

    // Step 5: Creation
    AccountCreated(u64, Result<String, String>),

    // Re-auth: token/credential update
    ReauthTokensSaved(u64, Result<(), String>),

    // General
    Cancel,
    Back,
    DismissError,
}

/// Events emitted to App.
#[derive(Debug, Clone)]
pub enum AddAccountEvent {
    /// Wizard completed successfully. Carry the new account ID.
    AccountAdded(String),
    /// Re-authentication completed successfully. Carry the account ID.
    ReauthComplete(String),
    /// Wizard cancelled.
    Cancelled,
}

// ── Wizard state ─────────────────────────────────────────

pub struct AddAccountWizard {
    pub step: AddAccountStep,
    pub is_first_launch: bool,
    pub email: String,
    pub error: Option<String>,
    pub generation: u64,
    // Discovery result
    pub discovery: Option<DiscoveredConfig>,
    pub selected_option: Option<usize>,
    // Auth state
    pub auth_state: AuthState,
    // OAuth result (stored when OAuth completes for account creation)
    pub oauth_success: Option<OAuthSuccess>,
    // Provider determined from discovery/selection
    pub resolved_provider: String,
    pub resolved_auth_method: String,
    // Identity
    pub identity: AccountIdentity,
    /// Colors already assigned to existing accounts (hex strings).
    pub used_colors: Vec<String>,
    /// DB handle for account creation (writable).
    db: Arc<Db>,
    /// Re-auth mode: when set, the wizard skips email/discovery/identity
    /// and goes straight to the auth step for this existing account.
    reauth_account_id: Option<String>,
}

impl AddAccountWizard {
    pub fn new_first_launch(db: Arc<Db>) -> Self {
        Self::new(true, Vec::new(), db)
    }

    pub fn new_add_account(used_colors: Vec<String>, db: Arc<Db>) -> Self {
        Self::new(false, used_colors, db)
    }

    /// Create a re-auth wizard for an existing account. Looks up the
    /// account's auth method and skips straight to the appropriate
    /// auth step (OAuth or password).
    pub fn new_reauth(
        account_id: String,
        email: String,
        db: Arc<Db>,
    ) -> Result<(Self, Task<AddAccountMessage>), String> {
        let auth_info = db.with_conn_sync(|conn| {
            get_account_auth_info_sync(conn, &account_id)
        })?;

        let mut wizard = Self::new(false, Vec::new(), db);
        wizard.email = email;
        wizard.reauth_account_id = Some(account_id);
        wizard.resolved_provider = auth_info.provider;
        wizard.resolved_auth_method = auth_info.auth_method.clone();

        let task = if auth_info.auth_method == "oauth" {
            wizard.start_reauth_oauth(
                auth_info.oauth_provider.as_deref(),
                auth_info.oauth_client_id.as_deref(),
            )
        } else {
            // Pre-populate server fields for password re-auth
            if let Some(host) = auth_info.imap_host {
                wizard.auth_state.imap_host = host;
            }
            if let Some(port) = auth_info.imap_port {
                wizard.auth_state.imap_port = port.to_string();
            }
            if let Some(sec) = auth_info.imap_security {
                wizard.auth_state.imap_security = match sec.as_str() {
                    "starttls" => SecurityOption::StartTls,
                    "none" => SecurityOption::None,
                    _ => SecurityOption::Tls,
                };
            }
            if let Some(host) = auth_info.smtp_host {
                wizard.auth_state.smtp_host = host;
            }
            if let Some(port) = auth_info.smtp_port {
                wizard.auth_state.smtp_port = port.to_string();
            }
            if let Some(sec) = auth_info.smtp_security {
                wizard.auth_state.smtp_security = match sec.as_str() {
                    "tls" => SecurityOption::Tls,
                    "none" => SecurityOption::None,
                    _ => SecurityOption::StartTls,
                };
            }
            if let Some(username) = auth_info.imap_username {
                wizard.auth_state.username = username;
            }
            wizard.step = AddAccountStep::PasswordAuth;
            Task::none()
        };

        Ok((wizard, task))
    }

    fn new(is_first_launch: bool, used_colors: Vec<String>, db: Arc<Db>) -> Self {
        let presets = ratatoskr_label_colors::category_colors::all_presets();
        let first_unused = presets
            .iter()
            .enumerate()
            .find(|(_, (_, bg, _))| !used_colors.iter().any(|uc| uc == *bg))
            .map(|(i, _)| i)
            .unwrap_or(0);

        Self {
            step: AddAccountStep::EmailInput,
            is_first_launch,
            email: String::new(),
            error: None,
            generation: 0,
            discovery: None,
            selected_option: None,
            auth_state: AuthState::default(),
            oauth_success: None,
            resolved_provider: "imap".to_string(),
            resolved_auth_method: "password".to_string(),
            identity: AccountIdentity {
                name: String::new(),
                selected_color_index: Some(first_unused),
            },
            used_colors,
            db,
            reauth_account_id: None,
        }
    }

    /// Get the selected color hex, or fallback to first preset.
    fn selected_color_hex(&self) -> String {
        let presets = ratatoskr_label_colors::category_colors::all_presets();
        self.identity
            .selected_color_index
            .and_then(|i| presets.get(i))
            .map(|(_, bg, _)| (*bg).to_string())
            .unwrap_or_else(|| presets[0].1.to_string())
    }
}

// ── Component impl ───────────────────────────────────────

impl Component for AddAccountWizard {
    type Message = AddAccountMessage;
    type Event = AddAccountEvent;

    fn update(
        &mut self,
        message: AddAccountMessage,
    ) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        match message {
            AddAccountMessage::EmailChanged(email) => {
                self.email = email;
                self.error = None;
                (Task::none(), None)
            }
            AddAccountMessage::SubmitEmail => self.handle_submit_email(),
            AddAccountMessage::DiscoveryComplete(g, _) if g != self.generation => {
                (Task::none(), None)
            }
            AddAccountMessage::DiscoveryComplete(_, Ok(config)) => {
                self.handle_discovery_result(config)
            }
            AddAccountMessage::DiscoveryComplete(_, Err(e)) => {
                // Duplicate account errors go back to email input.
                // Discovery failures go to manual configuration.
                if e.contains("already configured") || e.contains("Database error") {
                    self.error = Some(e);
                    self.step = AddAccountStep::EmailInput;
                } else {
                    self.error = Some(format!(
                        "We couldn't auto-detect your mail server. {e}"
                    ));
                    self.step = AddAccountStep::ManualConfiguration;
                }
                (Task::none(), None)
            }
            AddAccountMessage::SelectProtocol(idx) => {
                self.selected_option = Some(idx);
                (Task::none(), None)
            }
            AddAccountMessage::ConfirmProtocol => self.handle_confirm_protocol(),
            AddAccountMessage::SubmitManualConfig => self.handle_submit_manual_config(),
            AddAccountMessage::SubmitCredentials => self.handle_submit_credentials(),
            AddAccountMessage::ValidationComplete(g, _) if g != self.generation => {
                (Task::none(), None)
            }
            AddAccountMessage::ValidationComplete(_, Ok(())) => {
                // Re-auth mode: save password credentials directly.
                if let Some(ref account_id) = self.reauth_account_id {
                    let reauth_params = ReauthAccountParams {
                        access_token: None,
                        refresh_token: None,
                        token_expires_at: None,
                        imap_password: Some(self.auth_state.password.clone()),
                        smtp_password: if self.auth_state.use_separate_smtp_credentials {
                            Some(self.auth_state.smtp_password.clone())
                        } else {
                            None
                        },
                    };
                    self.generation += 1;
                    let generation = self.generation;
                    let db = Arc::clone(&self.db);
                    let aid = account_id.clone();
                    let task = Task::perform(
                        async move {
                            let result = db.with_write_conn(move |conn| {
                                update_account_tokens_sync(conn, &aid, reauth_params)
                            }).await;
                            (generation, result)
                        },
                        |(g, result)| AddAccountMessage::ReauthTokensSaved(g, result),
                    );
                    return (task, None);
                }

                self.prefill_identity_name();
                self.step = AddAccountStep::Identity;
                self.error = None;
                (Task::none(), None)
            }
            AddAccountMessage::ValidationComplete(_, Err(e)) => {
                self.error = Some(e);
                self.step = AddAccountStep::PasswordAuth;
                (Task::none(), None)
            }
            AddAccountMessage::OAuthComplete(g, _) if g != self.generation => {
                (Task::none(), None)
            }
            AddAccountMessage::OAuthComplete(_, Ok(success)) => {
                self.handle_oauth_success(success)
            }
            AddAccountMessage::OAuthComplete(_, Err(e)) => {
                self.error = Some(e);
                (Task::none(), None)
            }
            AddAccountMessage::RetryOAuth => self.handle_retry_oauth(),
            AddAccountMessage::SubmitIdentity => self.handle_submit_identity(),
            AddAccountMessage::AccountCreated(g, _) if g != self.generation => {
                (Task::none(), None)
            }
            AddAccountMessage::AccountCreated(_, Ok(account_id)) => {
                (Task::none(), Some(AddAccountEvent::AccountAdded(account_id)))
            }
            AddAccountMessage::AccountCreated(_, Err(e)) => {
                self.error = Some(e);
                self.step = AddAccountStep::Identity;
                (Task::none(), None)
            }
            AddAccountMessage::ReauthTokensSaved(g, _) if g != self.generation => {
                (Task::none(), None)
            }
            AddAccountMessage::ReauthTokensSaved(_, Ok(())) => {
                let account_id = self.reauth_account_id.clone()
                    .unwrap_or_default();
                (Task::none(), Some(AddAccountEvent::ReauthComplete(account_id)))
            }
            AddAccountMessage::ReauthTokensSaved(_, Err(e)) => {
                self.error = Some(format!("Failed to save credentials: {e}"));
                (Task::none(), None)
            }
            AddAccountMessage::Cancel | AddAccountMessage::CancelOAuth => {
                (Task::none(), Some(AddAccountEvent::Cancelled))
            }
            AddAccountMessage::Back => {
                self.handle_back();
                (Task::none(), None)
            }
            AddAccountMessage::DismissError => {
                self.error = None;
                (Task::none(), None)
            }
            _ => {
                self.handle_field_update(message);
                (Task::none(), None)
            }
        }
    }

    fn view(&self) -> Element<'_, AddAccountMessage> {
        match self.step {
            AddAccountStep::EmailInput => self.view_email_input(),
            AddAccountStep::Discovering => view_discovering(),
            AddAccountStep::ProtocolSelection => self.view_protocol_selection(),
            AddAccountStep::ManualConfiguration => self.view_manual_config(),
            AddAccountStep::OAuthWaiting => self.view_oauth_waiting(),
            AddAccountStep::PasswordAuth => self.view_password_auth(),
            AddAccountStep::Validating => view_validating(),
            AddAccountStep::Identity => self.view_identity(),
            AddAccountStep::Creating => view_creating(),
        }
    }
}

// ── Update helpers ───────────────────────────────────────

impl AddAccountWizard {
    fn handle_submit_email(
        &mut self,
    ) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        let email = self.email.trim().to_lowercase();
        if email.is_empty() || !email.contains('@') {
            self.error = Some("Please enter a valid email address.".to_string());
            return (Task::none(), None);
        }
        self.email = email.clone();
        self.step = AddAccountStep::Discovering;
        self.error = None;
        self.generation += 1;
        let generation = self.generation;
        let db = Arc::clone(&self.db);

        let task = Task::perform(
            async move {
                // Duplicate check — run synchronously inside spawn_blocking
                let email_for_dup = email.clone();
                let dup = db.with_conn(move |conn| {
                    account_exists_by_email_sync(conn, &email_for_dup)
                }).await;
                match dup {
                    Ok(true) => {
                        return (
                            generation,
                            Err("This account is already configured.".to_string()),
                        );
                    }
                    Err(e) => {
                        return (generation, Err(format!("Database error: {e}")));
                    }
                    Ok(false) => {}
                }

                // Run real discovery with 15s timeout
                let result = ratatoskr_core::discovery::discover(&email).await;
                (generation, result)
            },
            |(g, result)| AddAccountMessage::DiscoveryComplete(g, result),
        );
        (task, None)
    }

    fn handle_discovery_result(
        &mut self,
        config: DiscoveredConfig,
    ) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        if config.options.is_empty() {
            self.error = Some(
                "We couldn't auto-detect your mail server.".to_string(),
            );
            self.step = AddAccountStep::ManualConfiguration;
            return (Task::none(), None);
        }

        self.discovery = Some(config.clone());

        // Auto-proceed when exactly one high-confidence option
        let auto_proceed = config.options.len() == 1
            && config.options[0].source.is_high_confidence();

        if auto_proceed {
            self.selected_option = Some(0);
            return self.proceed_to_auth(&config.options[0]);
        }

        // Multiple options or lower confidence: show selection
        self.selected_option = Some(0);
        self.step = AddAccountStep::ProtocolSelection;
        (Task::none(), None)
    }

    fn handle_confirm_protocol(
        &mut self,
    ) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        let config = match &self.discovery {
            Some(c) => c.clone(),
            None => return (Task::none(), None),
        };
        let idx = self.selected_option.unwrap_or(0);
        let Some(option) = config.options.get(idx) else {
            return (Task::none(), None);
        };
        self.proceed_to_auth(option)
    }

    fn proceed_to_auth(
        &mut self,
        option: &ProtocolOption,
    ) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        // Set the provider string for account creation
        self.resolved_provider = protocol_to_db_provider(&option.protocol);

        match &option.auth.method {
            AuthMethod::OAuth2 {
                provider_id,
                auth_url,
                token_url,
                scopes,
                use_pkce,
            } => {
                self.resolved_auth_method = "oauth".to_string();
                self.step = AddAccountStep::OAuthWaiting;
                self.error = None;
                self.generation += 1;
                let generation = self.generation;

                let request = ratatoskr_core::oauth::OAuthProviderAuthorizationRequest {
                    provider_id: provider_id.clone(),
                    auth_url: auth_url.clone(),
                    token_url: token_url.clone(),
                    scopes: scopes.clone(),
                    user_info_url: None,
                    use_pkce: *use_pkce,
                    client_id: resolve_client_id(provider_id),
                    client_secret: None,
                };

                let provider_id_clone = provider_id.clone();
                let client_id_clone = resolve_client_id(provider_id);

                let task = Task::perform(
                    async move {
                        let provider =
                            ratatoskr_core::oauth::GenericOAuthProvider::from_request(request);
                        let open_url = |url: &str| -> Result<(), String> {
                            open_browser_url(url)
                        };
                        let result = ratatoskr_core::oauth::authorize_with_provider(
                            &provider, &open_url,
                        )
                        .await;
                        let mapped = result.map(|bundle| OAuthSuccess {
                            access_token: bundle.tokens.access_token,
                            refresh_token: bundle.tokens.refresh_token,
                            token_expires_at: Some(
                                chrono::Utc::now().timestamp()
                                    + bundle.tokens.expires_in as i64,
                            ),
                            user_email: bundle.user_info.email,
                            user_name: bundle.user_info.name,
                            oauth_provider: provider_id_clone,
                            oauth_client_id: client_id_clone,
                        });
                        (generation, mapped)
                    },
                    |(g, result)| AddAccountMessage::OAuthComplete(g, result),
                );
                (task, None)
            }

            AuthMethod::Password => {
                self.resolved_auth_method = "password".to_string();
                self.prefill_auth_from_option(option);
                self.step = AddAccountStep::PasswordAuth;
                self.error = None;
                (Task::none(), None)
            }

            AuthMethod::OAuth2Unsupported { provider_domain } => {
                self.resolved_auth_method = "password".to_string();
                self.prefill_auth_from_option(option);
                self.step = AddAccountStep::PasswordAuth;
                self.error = Some(format!(
                    "This provider requires an app-specific password. \
                     Check {provider_domain} for setup instructions."
                ));
                (Task::none(), None)
            }
        }
    }

    /// Start the OAuth flow for re-auth, using the stored provider info.
    fn start_reauth_oauth(
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

        // Look up the full OAuth config from the discovery registry
        let oauth_config =
            ratatoskr_core::discovery::registry::oauth_config_for_provider(&provider_id);
        let Some(auth_method) = oauth_config else {
            self.step = AddAccountStep::PasswordAuth;
            self.error = Some(format!(
                "No OAuth configuration found for provider '{provider_id}'. \
                 Please enter credentials manually."
            ));
            return Task::none();
        };

        let AuthMethod::OAuth2 {
            auth_url,
            token_url,
            scopes,
            use_pkce,
            ..
        } = auth_method
        else {
            self.step = AddAccountStep::PasswordAuth;
            return Task::none();
        };

        self.step = AddAccountStep::OAuthWaiting;
        self.error = None;
        self.generation += 1;
        let generation = self.generation;
        let provider_id_clone = provider_id.clone();
        let client_id_clone = client_id.clone();

        let request = ratatoskr_core::oauth::OAuthProviderAuthorizationRequest {
            provider_id,
            auth_url,
            token_url,
            scopes,
            user_info_url: None,
            use_pkce,
            client_id,
            client_secret: None,
        };

        Task::perform(
            async move {
                let provider =
                    ratatoskr_core::oauth::GenericOAuthProvider::from_request(request);
                let open_url = |url: &str| -> Result<(), String> {
                    open_browser_url(url)
                };
                let result = ratatoskr_core::oauth::authorize_with_provider(
                    &provider, &open_url,
                )
                .await;
                let mapped = result.map(|bundle| OAuthSuccess {
                    access_token: bundle.tokens.access_token,
                    refresh_token: bundle.tokens.refresh_token,
                    token_expires_at: Some(
                        chrono::Utc::now().timestamp()
                            + bundle.tokens.expires_in as i64,
                    ),
                    user_email: bundle.user_info.email,
                    user_name: bundle.user_info.name,
                    oauth_provider: provider_id_clone,
                    oauth_client_id: client_id_clone,
                });
                (generation, mapped)
            },
            |(g, result)| AddAccountMessage::OAuthComplete(g, result),
        )
    }

    fn prefill_auth_from_option(&mut self, option: &ProtocolOption) {
        if let Protocol::Imap {
            ref incoming,
            ref outgoing,
        } = option.protocol
        {
            self.auth_state.imap_host = incoming.hostname.clone();
            self.auth_state.imap_port = incoming.port.to_string();
            self.auth_state.imap_security = match incoming.security {
                Security::Tls => SecurityOption::Tls,
                Security::StartTls => SecurityOption::StartTls,
                Security::None => SecurityOption::None,
            };
            self.auth_state.smtp_host = outgoing.hostname.clone();
            self.auth_state.smtp_port = outgoing.port.to_string();
            self.auth_state.smtp_security = match outgoing.security {
                Security::Tls => SecurityOption::Tls,
                Security::StartTls => SecurityOption::StartTls,
                Security::None => SecurityOption::None,
            };
            self.auth_state.username = self.email.clone();
        }
    }

    fn handle_oauth_success(
        &mut self,
        success: OAuthSuccess,
    ) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        // Re-auth mode: save tokens directly, skip identity step.
        if let Some(ref account_id) = self.reauth_account_id {
            let reauth_params = ReauthAccountParams {
                access_token: Some(success.access_token.clone()),
                refresh_token: success.refresh_token.clone(),
                token_expires_at: success.token_expires_at,
                imap_password: None,
                smtp_password: None,
            };
            self.generation += 1;
            let generation = self.generation;
            let db = Arc::clone(&self.db);
            let aid = account_id.clone();
            let task = Task::perform(
                async move {
                    let result = db.with_write_conn(move |conn| {
                        update_account_tokens_sync(conn, &aid, reauth_params)
                    }).await;
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

    fn handle_retry_oauth(
        &mut self,
    ) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        // Re-auth mode: re-run using stored provider info
        if self.reauth_account_id.is_some() {
            self.error = None;
            // Look up auth info again for the retry
            let aid = self.reauth_account_id.clone().unwrap_or_default();
            let auth_info = self.db.with_conn_sync(|conn| {
                get_account_auth_info_sync(conn, &aid)
            });
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

    fn handle_submit_manual_config(
        &mut self,
    ) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        // Copy manual config fields to auth state and proceed to password auth
        self.prefill_from_email();
        self.resolved_provider = "imap".to_string();
        self.resolved_auth_method = "password".to_string();
        self.step = AddAccountStep::PasswordAuth;
        self.error = None;
        (Task::none(), None)
    }

    fn handle_submit_credentials(
        &mut self,
    ) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        if self.auth_state.username.trim().is_empty() {
            self.error = Some("Username is required.".to_string());
            return (Task::none(), None);
        }
        if self.auth_state.password.is_empty() {
            self.error = Some("Password is required.".to_string());
            return (Task::none(), None);
        }

        // Wire credential validation — test IMAP connection
        self.step = AddAccountStep::Validating;
        self.error = None;
        self.generation += 1;
        let generation = self.generation;

        let host = self.auth_state.imap_host.clone();
        let port_str = self.auth_state.imap_port.clone();
        let security = self.auth_state.imap_security;
        let username = self.auth_state.username.clone();
        let password = self.auth_state.password.clone();
        let accept_invalid_certs = self.auth_state.accept_invalid_certs;

        let task = Task::perform(
            async move {
                let result = validate_imap_connection(
                    &host,
                    &port_str,
                    security,
                    &username,
                    &password,
                    accept_invalid_certs,
                )
                .await;
                (generation, result)
            },
            |(g, result)| AddAccountMessage::ValidationComplete(g, result),
        );
        (task, None)
    }

    fn handle_submit_identity(
        &mut self,
    ) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        if self.identity.name.trim().is_empty() {
            self.error = Some("Please enter an account name.".to_string());
            return (Task::none(), None);
        }
        self.step = AddAccountStep::Creating;
        self.error = None;
        self.generation += 1;
        let generation = self.generation;

        let create_params = self.build_create_params();
        let db = Arc::clone(&self.db);

        let task = Task::perform(
            async move {
                let result = db
                    .with_write_conn(move |conn| {
                        create_account_sync(conn, create_params)
                    })
                    .await;
                match result {
                    Ok(id) => (generation, Ok(id)),
                    Err(e) => (generation, Err(e)),
                }
            },
            |(g, result): (u64, Result<String, String>)| {
                AddAccountMessage::AccountCreated(g, result)
            },
        );
        (task, None)
    }

    fn build_create_params(&self) -> CreateAccountParams {
        let color = self.selected_color_hex();
        let account_name = self.identity.name.trim().to_string();

        // SMTP credentials: separate if the user toggled the checkbox
        let (smtp_user, smtp_pass) = if self.auth_state.use_separate_smtp_credentials {
            (
                Some(self.auth_state.smtp_username.clone()),
                Some(self.auth_state.smtp_password.clone()),
            )
        } else {
            (None, None)
        };

        // Build params based on auth method (password vs OAuth)
        if let Some(ref oauth) = self.oauth_success {
            CreateAccountParams {
                email: self.email.clone(),
                provider: self.resolved_provider.clone(),
                display_name: Some(oauth.user_name.clone()),
                account_name,
                account_color: color,
                auth_method: self.resolved_auth_method.clone(),
                access_token: Some(oauth.access_token.clone()),
                refresh_token: oauth.refresh_token.clone(),
                token_expires_at: oauth.token_expires_at,
                oauth_provider: Some(oauth.oauth_provider.clone()),
                oauth_client_id: Some(oauth.oauth_client_id.clone()),
                imap_host: None,
                imap_port: None,
                imap_security: None,
                imap_username: None,
                imap_password: None,
                smtp_host: None,
                smtp_port: None,
                smtp_security: None,
                smtp_username: None,
                smtp_password: None,
                jmap_url: None,
                accept_invalid_certs: false,
            }
        } else {
            let imap_port = self.auth_state.imap_port.parse::<i64>().ok();
            let smtp_port = self.auth_state.smtp_port.parse::<i64>().ok();
            CreateAccountParams {
                email: self.email.clone(),
                provider: self.resolved_provider.clone(),
                display_name: None,
                account_name,
                account_color: color,
                auth_method: self.resolved_auth_method.clone(),
                access_token: None,
                refresh_token: None,
                token_expires_at: None,
                oauth_provider: None,
                oauth_client_id: None,
                imap_host: Some(self.auth_state.imap_host.clone()),
                imap_port,
                imap_security: Some(
                    self.auth_state.imap_security.to_db_string().to_string(),
                ),
                imap_username: Some(self.auth_state.username.clone()),
                imap_password: Some(self.auth_state.password.clone()),
                smtp_host: Some(self.auth_state.smtp_host.clone()),
                smtp_port,
                smtp_security: Some(
                    self.auth_state.smtp_security.to_db_string().to_string(),
                ),
                smtp_username: smtp_user,
                smtp_password: smtp_pass,
                jmap_url: None,
                accept_invalid_certs: self.auth_state.accept_invalid_certs,
            }
        }
    }

    fn handle_back(&mut self) {
        // In re-auth mode, Back is equivalent to Cancel — there's no
        // previous step to go back to.
        if self.reauth_account_id.is_some() {
            return;
        }

        match self.step {
            AddAccountStep::PasswordAuth | AddAccountStep::ManualConfiguration => {
                self.step = AddAccountStep::EmailInput;
                self.error = None;
            }
            AddAccountStep::Identity => {
                // Go back to auth step, depending on method
                if self.oauth_success.is_some() {
                    self.step = AddAccountStep::OAuthWaiting;
                } else {
                    self.step = AddAccountStep::PasswordAuth;
                }
                self.error = None;
            }
            AddAccountStep::ProtocolSelection => {
                self.step = AddAccountStep::EmailInput;
                self.error = None;
            }
            _ => {}
        }
    }

    fn handle_field_update(&mut self, message: AddAccountMessage) {
        match message {
            AddAccountMessage::UsernameChanged(v) => self.auth_state.username = v,
            AddAccountMessage::PasswordChanged(v) => self.auth_state.password = v,
            AddAccountMessage::SmtpUsernameChanged(v) => {
                self.auth_state.smtp_username = v;
            }
            AddAccountMessage::SmtpPasswordChanged(v) => {
                self.auth_state.smtp_password = v;
            }
            AddAccountMessage::ToggleSeparateSmtpCredentials(v) => {
                self.auth_state.use_separate_smtp_credentials = v;
            }
            AddAccountMessage::ToggleAcceptInvalidCerts(v) => {
                self.auth_state.accept_invalid_certs = v;
            }
            AddAccountMessage::AuthImapHostChanged(v) => {
                self.auth_state.imap_host = v;
            }
            AddAccountMessage::AuthImapPortChanged(v) => {
                self.auth_state.imap_port = v;
            }
            AddAccountMessage::AuthImapSecurityChanged(v) => {
                self.auth_state.imap_security = v;
            }
            AddAccountMessage::AuthSmtpHostChanged(v) => {
                self.auth_state.smtp_host = v;
            }
            AddAccountMessage::AuthSmtpPortChanged(v) => {
                self.auth_state.smtp_port = v;
            }
            AddAccountMessage::AuthSmtpSecurityChanged(v) => {
                self.auth_state.smtp_security = v;
            }
            AddAccountMessage::ManualImapHostChanged(v) => {
                self.auth_state.imap_host = v;
            }
            AddAccountMessage::ManualImapPortChanged(v) => {
                self.auth_state.imap_port = v;
            }
            AddAccountMessage::ManualSmtpHostChanged(v) => {
                self.auth_state.smtp_host = v;
            }
            AddAccountMessage::ManualSmtpPortChanged(v) => {
                self.auth_state.smtp_port = v;
            }
            AddAccountMessage::AccountNameChanged(v) => self.identity.name = v,
            AddAccountMessage::SelectColor(i) => {
                self.identity.selected_color_index = Some(i);
            }
            _ => {}
        }
    }

    fn prefill_from_email(&mut self) {
        if self.auth_state.username.is_empty() {
            self.auth_state.username = self.email.clone();
        }
        let domain = self.email.split('@').nth(1).unwrap_or("");
        if self.auth_state.imap_host.is_empty() {
            self.auth_state.imap_host = format!("imap.{domain}");
        }
        if self.auth_state.smtp_host.is_empty() {
            self.auth_state.smtp_host = format!("smtp.{domain}");
        }
    }

    fn prefill_identity_name(&mut self) {
        if self.identity.name.is_empty() {
            let domain = self.email.split('@').nth(1).unwrap_or("");
            let name = domain.split('.').next().unwrap_or(domain);
            self.identity.name = titlecase(name);
        }
    }
}

fn titlecase(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => {
            let mut result = c.to_uppercase().to_string();
            result.extend(chars);
            result
        }
    }
}

// ── Protocol helpers ─────────────────────────────────────

fn protocol_to_db_provider(protocol: &Protocol) -> String {
    match protocol {
        Protocol::GmailApi => "gmail_api".to_string(),
        Protocol::MicrosoftGraph => "graph".to_string(),
        Protocol::Jmap { .. } => "jmap".to_string(),
        Protocol::Imap { .. } => "imap".to_string(),
    }
}

fn protocol_display_name(protocol: &Protocol, provider_name: Option<&str>) -> String {
    match (protocol, provider_name) {
        (_, Some(name)) => name.to_string(),
        (Protocol::GmailApi, _) => "Gmail".to_string(),
        (Protocol::MicrosoftGraph, _) => "Microsoft 365".to_string(),
        (Protocol::Jmap { .. }, _) => "JMAP".to_string(),
        (Protocol::Imap { .. }, _) => "IMAP".to_string(),
    }
}

fn protocol_detail(protocol: &Protocol) -> String {
    match protocol {
        Protocol::GmailApi => "Gmail API (recommended)".to_string(),
        Protocol::MicrosoftGraph => "Microsoft Graph API".to_string(),
        Protocol::Jmap { session_url } => format!("JMAP: {session_url}"),
        Protocol::Imap { incoming, outgoing } => {
            format!(
                "IMAP: {}:{} / SMTP: {}:{}",
                incoming.hostname, incoming.port, outgoing.hostname, outgoing.port
            )
        }
    }
}

fn source_display(source: &DiscoverySource) -> &str {
    match source {
        DiscoverySource::Registry => "Known provider",
        DiscoverySource::AutoconfigXml { .. } => "Auto-detected",
        DiscoverySource::MxLookup { .. } => "MX lookup",
        DiscoverySource::JmapWellKnown => "JMAP discovery",
        DiscoverySource::PortProbe => "Port scan",
    }
}

fn resolve_client_id(provider_id: &str) -> String {
    match provider_id {
        "microsoft" | "microsoft_graph" => {
            ratatoskr_core::oauth::MICROSOFT_DEFAULT_CLIENT_ID.to_string()
        }
        // For Google, the client_id is typically embedded in the app.
        // If not available, the OAuth flow will use the discovery registry value.
        _ => String::new(),
    }
}

fn open_browser_url(url: &str) -> Result<(), String> {
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

/// Test IMAP connection to validate credentials.
async fn validate_imap_connection(
    host: &str,
    port_str: &str,
    security: SecurityOption,
    username: &str,
    password: &str,
    accept_invalid_certs: bool,
) -> Result<(), String> {
    let port: u16 = port_str
        .parse()
        .map_err(|_| "Invalid port number".to_string())?;

    let security_str = security.to_db_string().to_string();

    let config = ratatoskr_core::imap::types::ImapConfig {
        host: host.to_string(),
        port,
        security: security_str,
        username: username.to_string(),
        password: password.to_string(),
        auth_method: "password".to_string(),
        accept_invalid_certs,
    };

    // Connect and immediately close — we just want to verify credentials
    let session = ratatoskr_core::imap::connection::connect(&config).await?;
    drop(session);
    Ok(())
}

// ── DiscoverySource extension ────────────────────────────

trait HighConfidence {
    fn is_high_confidence(&self) -> bool;
}

impl HighConfidence for DiscoverySource {
    fn is_high_confidence(&self) -> bool {
        matches!(
            self,
            DiscoverySource::Registry | DiscoverySource::JmapWellKnown
        )
    }
}

// ── View: Email Input ────────────────────────────────────

impl AddAccountWizard {
    fn view_email_input(&self) -> Element<'_, AddAccountMessage> {
        let mut col = column![]
            .spacing(SPACE_LG)
            .align_x(Alignment::Center)
            .width(Length::Fill);

        if self.is_first_launch {
            col = col.push(
                container(icon::mail().size(48.0).style(text::primary))
                    .align_x(Alignment::Center),
            );
            col = col.push(Space::new().height(SPACE_SM));
            col = col.push(
                text("Welcome to Ratatoskr")
                    .size(TEXT_HEADING)
                    .style(text::base)
                    .font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..font::text()
                    }),
            );
            col = col.push(
                text("Enter your email address to get started")
                    .size(TEXT_LG)
                    .style(text::secondary),
            );
        } else {
            col = col.push(
                text("Add Account")
                    .size(TEXT_HEADING)
                    .style(text::base)
                    .font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..font::text()
                    }),
            );
        }

        col = col.push(Space::new().height(SPACE_SM));

        col = col.push(
            text_input("alice@example.com", &self.email)
                .on_input(AddAccountMessage::EmailChanged)
                .on_submit(AddAccountMessage::SubmitEmail)
                .size(TEXT_LG)
                .padding(PAD_INPUT)
                .style(theme::TextInputClass::Settings.style())
                .width(Length::Fill),
        );

        if let Some(ref err) = self.error {
            col = col.push(text(err.as_str()).size(TEXT_SM).style(text::danger));
        }

        col = col.push(primary_button("Continue", AddAccountMessage::SubmitEmail));

        if !self.is_first_launch {
            col = col.push(ghost_button("Cancel", AddAccountMessage::Cancel));
        }

        col.width(Length::Fill).into()
    }
}

// ── View: Discovering ────────────────────────────────────

fn view_discovering<'a>() -> Element<'a, AddAccountMessage> {
    column![
        text("Looking up your email provider...")
            .size(TEXT_LG)
            .style(text::secondary),
        Space::new().height(SPACE_MD),
        text("Please wait...").size(TEXT_SM).style(text::secondary),
        Space::new().height(SPACE_LG),
        ghost_button("Cancel", AddAccountMessage::Cancel),
    ]
    .spacing(SPACE_XS)
    .align_x(Alignment::Center)
    .width(Length::Fill)
    .into()
}

// ── View: Protocol Selection ─────────────────────────────

impl AddAccountWizard {
    fn view_protocol_selection(&self) -> Element<'_, AddAccountMessage> {
        let config = match &self.discovery {
            Some(c) => c,
            None => return column![].into(),
        };

        let mut col = column![
            text("Choose your email provider")
                .size(TEXT_HEADING)
                .style(text::base)
                .font(iced::Font {
                    weight: iced::font::Weight::Bold,
                    ..font::text()

                }),
            Space::new().height(SPACE_XS),
            text(&self.email).size(TEXT_LG).style(text::secondary),
        ]
        .spacing(SPACE_XS)
        .width(Length::Fill);

        col = col.push(Space::new().height(SPACE_MD));

        for (i, option) in config.options.iter().enumerate() {
            let selected = self.selected_option == Some(i);
            col = col.push(protocol_card_view(option, i, selected));
        }

        col = col.push(Space::new().height(SPACE_MD));
        col = col.push(primary_button(
            "Continue",
            AddAccountMessage::ConfirmProtocol,
        ));
        col = col.push(ghost_button("Back", AddAccountMessage::Back));

        col.into()
    }
}

fn protocol_card_view(
    option: &ProtocolOption,
    index: usize,
    selected: bool,
) -> Element<'_, AddAccountMessage> {
    let name = protocol_display_name(
        &option.protocol,
        option.provider_name.as_deref(),
    );
    let detail = protocol_detail(&option.protocol);
    let source_label = source_display(&option.source);

    let content = row![
        container(
            column![
                text(name)
                    .size(TEXT_LG)
                    .style(text::base)
                    .font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..font::text()
                    }),
                text(detail).size(TEXT_SM).style(text::secondary),
            ]
            .spacing(SPACE_XXXS),
        )
        .width(Length::Fill)
        .align_y(Alignment::Center),
        container(text(source_label).size(TEXT_XS).style(text::secondary))
            .align_y(Alignment::Center),
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Center);

    let style = if selected {
        theme::ButtonClass::Chip { active: true }
    } else {
        theme::ButtonClass::Action
    };

    button(
        container(content)
            .padding(PAD_CARD)
            .width(Length::Fill)
            .height(PROTOCOL_CARD_HEIGHT),
    )
    .on_press(AddAccountMessage::SelectProtocol(index))
    .padding(0)
    .style(style.style())
    .width(Length::Fill)
    .into()
}

// ── View: Manual Configuration ───────────────────────────

impl AddAccountWizard {
    fn view_manual_config(&self) -> Element<'_, AddAccountMessage> {
        let mut col = column![].spacing(SPACE_MD).width(Length::Fill);

        col = col.push(
            text("Manual Configuration")
                .size(TEXT_HEADING)
                .style(text::base)
                .font(iced::Font {
                    weight: iced::font::Weight::Bold,
                    ..font::text()
                }),
        );

        if let Some(ref err) = self.error {
            col = col.push(text(err.as_str()).size(TEXT_SM).style(text::danger));
        }

        col = col.push(text("Incoming (IMAP)").size(TEXT_XL).style(text::base));
        col = col.push(server_port_row(
            "imap.example.com",
            &self.auth_state.imap_host,
            "993",
            &self.auth_state.imap_port,
            AddAccountMessage::ManualImapHostChanged,
            AddAccountMessage::ManualImapPortChanged,
        ));

        col = col.push(text("Outgoing (SMTP)").size(TEXT_XL).style(text::base));
        col = col.push(server_port_row(
            "smtp.example.com",
            &self.auth_state.smtp_host,
            "587",
            &self.auth_state.smtp_port,
            AddAccountMessage::ManualSmtpHostChanged,
            AddAccountMessage::ManualSmtpPortChanged,
        ));

        col = col.push(Space::new().height(SPACE_SM));
        col = col.push(primary_button(
            "Continue",
            AddAccountMessage::SubmitManualConfig,
        ));
        col = col.push(ghost_button("Back", AddAccountMessage::Back));

        scrollable(col).spacing(SCROLLBAR_SPACING).into()
    }
}

// ── View: OAuth Waiting ──────────────────────────────────

impl AddAccountWizard {
    fn view_oauth_waiting(&self) -> Element<'_, AddAccountMessage> {
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
            col = col.push(
                text(&self.email).size(TEXT_LG).style(text::secondary),
            );
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

// ── View: Password Auth ──────────────────────────────────

impl AddAccountWizard {
    fn view_password_auth(&self) -> Element<'_, AddAccountMessage> {
        let mut col = column![].spacing(SPACE_MD).width(Length::Fill);

        let heading = if self.reauth_account_id.is_some() {
            "Re-authenticate"
        } else {
            "Sign In"
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
            col = col.push(
                text(&self.email).size(TEXT_LG).style(text::secondary),
            );
        }

        // IMAP section
        col = col.push(text("Incoming (IMAP)").size(TEXT_XL).style(text::base));
        col = col.push(server_port_row(
            "imap.example.com",
            &self.auth_state.imap_host,
            "993",
            &self.auth_state.imap_port,
            AddAccountMessage::AuthImapHostChanged,
            AddAccountMessage::AuthImapPortChanged,
        ));
        col = col.push(security_selector(
            self.auth_state.imap_security,
            AddAccountMessage::AuthImapSecurityChanged,
        ));
        col = col.push(labeled_input(
            "Username",
            "alice@example.com",
            &self.auth_state.username,
            AddAccountMessage::UsernameChanged,
        ));
        // INTENTIONAL: Password field is plaintext — no .secure(true).
        // This is a deliberate product decision per problem-statement.md.
        // Users need to see what they type for app-specific passwords.
        col = col.push(labeled_input(
            "Password",
            "",
            &self.auth_state.password,
            AddAccountMessage::PasswordChanged,
        ));

        // SMTP section
        col = col.push(Space::new().height(SPACE_SM));
        col = col.push(text("Outgoing (SMTP)").size(TEXT_XL).style(text::base));
        col = col.push(server_port_row(
            "smtp.example.com",
            &self.auth_state.smtp_host,
            "587",
            &self.auth_state.smtp_port,
            AddAccountMessage::AuthSmtpHostChanged,
            AddAccountMessage::AuthSmtpPortChanged,
        ));
        col = col.push(security_selector(
            self.auth_state.smtp_security,
            AddAccountMessage::AuthSmtpSecurityChanged,
        ));

        col = col.push(self.view_password_auth_options());

        if let Some(ref err) = self.error {
            col = col.push(text(err.as_str()).size(TEXT_SM).style(text::danger));
        }

        col = col.push(Space::new().height(SPACE_SM));
        col = col.push(primary_button(
            "Sign In",
            AddAccountMessage::SubmitCredentials,
        ));
        col = col.push(ghost_button("Back", AddAccountMessage::Back));

        scrollable(col).spacing(SCROLLBAR_SPACING).into()
    }

    fn view_password_auth_options(&self) -> Element<'_, AddAccountMessage> {
        let mut col = column![].spacing(SPACE_SM).width(Length::Fill);

        col = col.push(
            row![
                iced::widget::checkbox(self.auth_state.use_separate_smtp_credentials)
                    .on_toggle(AddAccountMessage::ToggleSeparateSmtpCredentials)
                    .size(RADIO_SIZE),
                text("Use different credentials for SMTP")
                    .size(TEXT_LG)
                    .style(text::base),
            ]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center),
        );

        if self.auth_state.use_separate_smtp_credentials {
            col = col.push(labeled_input(
                "SMTP Username",
                "",
                &self.auth_state.smtp_username,
                AddAccountMessage::SmtpUsernameChanged,
            ));
            col = col.push(labeled_input(
                "SMTP Password",
                "",
                &self.auth_state.smtp_password,
                AddAccountMessage::SmtpPasswordChanged,
            ));
        }

        col = col.push(
            row![
                iced::widget::checkbox(self.auth_state.accept_invalid_certs)
                    .on_toggle(AddAccountMessage::ToggleAcceptInvalidCerts)
                    .size(RADIO_SIZE),
                text("Accept self-signed certificates")
                    .size(TEXT_LG)
                    .style(text::base),
            ]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center),
        );

        col.into()
    }
}

// ── View: Validating ─────────────────────────────────────

fn view_validating<'a>() -> Element<'a, AddAccountMessage> {
    column![
        text("Validating credentials...")
            .size(TEXT_LG)
            .style(text::secondary),
        Space::new().height(SPACE_MD),
        text("Connecting to your mail server...")
            .size(TEXT_SM)
            .style(text::secondary),
    ]
    .spacing(SPACE_XS)
    .align_x(Alignment::Center)
    .width(Length::Fill)
    .into()
}

// ── View: Identity ───────────────────────────────────────

impl AddAccountWizard {
    fn view_identity(&self) -> Element<'_, AddAccountMessage> {
        let mut col = column![].spacing(SPACE_MD).width(Length::Fill);

        col = col.push(
            text(&self.email).size(TEXT_LG).style(text::secondary),
        );
        col = col.push(Space::new().height(SPACE_XS));
        col = col.push(labeled_input(
            "Account name",
            "e.g. Work, Personal",
            &self.identity.name,
            AddAccountMessage::AccountNameChanged,
        ));

        col = col.push(Space::new().height(SPACE_SM));
        col = col.push(
            text("Pick a color").size(TEXT_SM).style(text::secondary),
        );
        col = col.push(color_palette_grid(
            self.identity.selected_color_index,
            &self.used_colors,
        ));

        if let Some(ref err) = self.error {
            col = col.push(text(err.as_str()).size(TEXT_SM).style(text::danger));
        }

        col = col.push(Space::new().height(SPACE_LG));
        col = col.push(primary_button("Done", AddAccountMessage::SubmitIdentity));

        col.into()
    }
}

// ── View: Creating ───────────────────────────────────────

fn view_creating<'a>() -> Element<'a, AddAccountMessage> {
    column![
        text("Creating account...")
            .size(TEXT_LG)
            .style(text::secondary),
        Space::new().height(SPACE_MD),
        text("Please wait...").size(TEXT_SM).style(text::secondary),
    ]
    .spacing(SPACE_XS)
    .align_x(Alignment::Center)
    .width(Length::Fill)
    .into()
}

// ── Color palette grid ───────────────────────────────────

fn color_palette_grid<'a>(
    selected: Option<usize>,
    used_colors: &[String],
) -> Element<'a, AddAccountMessage> {
    let presets = ratatoskr_label_colors::category_colors::all_presets();
    let mut grid = column![].spacing(SPACE_XS);
    let mut current_row = row![].spacing(SPACE_XS);

    for (i, &(_name, bg_hex, _fg_hex)) in presets.iter().enumerate() {
        let is_selected = selected == Some(i);
        let is_used = used_colors.iter().any(|c| c == bg_hex);
        let color = theme::hex_to_color(bg_hex);

        let swatch = iced::widget::Canvas::new(SwatchPainter {
            color,
            selected: is_selected,
            used: is_used,
        })
        .width(COLOR_SWATCH_SIZE)
        .height(COLOR_SWATCH_SIZE);

        let style = if is_selected {
            theme::ButtonClass::Chip { active: true }
        } else {
            theme::ButtonClass::BareTransparent
        };

        let swatch_btn = button(swatch)
            .on_press(AddAccountMessage::SelectColor(i))
            .padding(2)
            .style(style.style());

        current_row = current_row.push(swatch_btn);

        if (i + 1) % COLOR_PALETTE_COLUMNS == 0 {
            grid = grid.push(current_row);
            current_row = row![].spacing(SPACE_XS);
        }
    }

    // Push remaining swatches
    if presets.len() % COLOR_PALETTE_COLUMNS != 0 {
        grid = grid.push(current_row);
    }

    grid.into()
}

// ── Swatch canvas painter ────────────────────────────────

struct SwatchPainter {
    color: iced::Color,
    selected: bool,
    used: bool,
}

impl<M> iced::widget::canvas::Program<M> for SwatchPainter {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<iced::widget::canvas::Geometry<iced::Renderer>> {
        let mut frame =
            iced::widget::canvas::Frame::new(renderer, bounds.size());
        let radius = bounds.width.min(bounds.height) / 2.0;
        let center =
            iced::Point::new(bounds.width / 2.0, bounds.height / 2.0);

        let circle = iced::widget::canvas::path::Path::circle(center, radius);

        let draw_color = if self.used && !self.selected {
            // Dim used colors
            iced::Color {
                a: 0.35,
                ..self.color
            }
        } else {
            self.color
        };

        frame.fill(&circle, draw_color);

        // Draw a small check for already-used colors
        if self.used {
            draw_check_mark(&mut frame, bounds, radius);
        }

        vec![frame.into_geometry()]
    }
}

fn draw_check_mark(
    frame: &mut iced::widget::canvas::Frame<iced::Renderer>,
    bounds: iced::Rectangle,
    radius: f32,
) {
    let check_color = iced::Color::WHITE;
    let check = iced::widget::canvas::path::Path::new(|b| {
        let cx = bounds.width / 2.0;
        let cy = bounds.height / 2.0;
        let s = radius * 0.35;
        b.move_to(iced::Point::new(cx - s * 0.5, cy));
        b.line_to(iced::Point::new(cx - s * 0.1, cy + s * 0.4));
        b.line_to(iced::Point::new(cx + s * 0.5, cy - s * 0.3));
    });
    frame.stroke(
        &check,
        iced::widget::canvas::Stroke::default()
            .with_color(check_color)
            .with_width(2.0),
    );
}

// ── Shared view helpers ──────────────────────────────────

fn primary_button<'a>(
    label: &'a str,
    on_press: AddAccountMessage,
) -> Element<'a, AddAccountMessage> {
    button(
        container(text(label).size(TEXT_LG).color(theme::ON_AVATAR))
            .center_x(Length::Fill),
    )
    .on_press(on_press)
    .padding(PAD_BUTTON)
    .style(theme::ButtonClass::Primary.style())
    .width(Length::Fill)
    .into()
}

fn ghost_button<'a>(
    label: &'a str,
    on_press: AddAccountMessage,
) -> Element<'a, AddAccountMessage> {
    button(
        container(text(label).size(TEXT_LG).style(text::secondary))
            .center_x(Length::Fill),
    )
    .on_press(on_press)
    .padding(PAD_BUTTON)
    .style(theme::ButtonClass::Ghost.style())
    .width(Length::Fill)
    .into()
}

fn labeled_input<'a>(
    label: &'a str,
    placeholder: &'a str,
    value: &'a str,
    on_input: impl Fn(String) -> AddAccountMessage + 'a,
) -> Element<'a, AddAccountMessage> {
    column![
        text(label).size(TEXT_SM).style(text::secondary),
        text_input(placeholder, value)
            .on_input(on_input)
            .size(TEXT_LG)
            .padding(PAD_INPUT)
            .style(theme::TextInputClass::Settings.style()),
    ]
    .spacing(SPACE_XXXS)
    .width(Length::Fill)
    .into()
}

fn server_port_row<'a>(
    server_placeholder: &'a str,
    server_value: &'a str,
    port_placeholder: &'a str,
    port_value: &'a str,
    on_server: impl Fn(String) -> AddAccountMessage + 'a,
    on_port: impl Fn(String) -> AddAccountMessage + 'a,
) -> Element<'a, AddAccountMessage> {
    row![
        column![
            text("Server").size(TEXT_SM).style(text::secondary),
            text_input(server_placeholder, server_value)
                .on_input(on_server)
                .size(TEXT_LG)
                .padding(PAD_INPUT)
                .style(theme::TextInputClass::Settings.style()),
        ]
        .spacing(SPACE_XXXS)
        .width(Length::FillPortion(3)),
        column![
            text("Port").size(TEXT_SM).style(text::secondary),
            text_input(port_placeholder, port_value)
                .on_input(on_port)
                .size(TEXT_LG)
                .padding(PAD_INPUT)
                .style(theme::TextInputClass::Settings.style()),
        ]
        .spacing(SPACE_XXXS)
        .width(Length::FillPortion(1)),
    ]
    .spacing(SPACE_SM)
    .into()
}

fn security_selector<'a>(
    current: SecurityOption,
    on_change: impl Fn(SecurityOption) -> AddAccountMessage + 'a + Copy,
) -> Element<'a, AddAccountMessage> {
    row![
        iced::widget::radio(
            SecurityOption::Tls.label(),
            SecurityOption::Tls,
            Some(current),
            on_change,
        )
        .size(RADIO_SIZE)
        .text_size(TEXT_LG)
        .spacing(SPACE_XXS)
        .style(theme::RadioClass::Settings.style()),
        iced::widget::radio(
            SecurityOption::StartTls.label(),
            SecurityOption::StartTls,
            Some(current),
            on_change,
        )
        .size(RADIO_SIZE)
        .text_size(TEXT_LG)
        .spacing(SPACE_XXS)
        .style(theme::RadioClass::Settings.style()),
        iced::widget::radio(
            SecurityOption::None.label(),
            SecurityOption::None,
            Some(current),
            on_change,
        )
        .size(RADIO_SIZE)
        .text_size(TEXT_LG)
        .spacing(SPACE_XXS)
        .style(theme::RadioClass::Settings.style()),
    ]
    .spacing(SPACE_MD)
    .into()
}
