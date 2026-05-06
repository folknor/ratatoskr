use std::sync::Arc;

use iced::{Element, Task};

use crate::component::Component;
use crate::db::Db;

use rtsk::db::queries_extra::{
    ReauthAccountParams, update_account_tokens_sync,
};
use rtsk::discovery::types::{
    AuthMethod, DiscoveredConfig, ProtocolOption, Security,
};

// Step enum

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

// Security option

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityOption {
    Tls,
    StartTls,
    None,
}

impl SecurityOption {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Tls => "SSL/TLS",
            Self::StartTls => "STARTTLS",
            Self::None => "None",
        }
    }

    pub(super) fn to_db_string(self) -> &'static str {
        match self {
            Self::Tls => "tls",
            Self::StartTls => "starttls",
            Self::None => "none",
        }
    }
}

// Manual provider

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualProvider {
    Gmail,
    Microsoft365,
    Jmap,
    Imap,
}

impl ManualProvider {
    pub(super) const ALL: &[ManualProvider] = &[
        ManualProvider::Gmail,
        ManualProvider::Microsoft365,
        ManualProvider::Jmap,
        ManualProvider::Imap,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Gmail => "Gmail",
            Self::Microsoft365 => "Microsoft 365",
            Self::Jmap => "JMAP",
            Self::Imap => "IMAP",
        }
    }

    pub(super) fn to_provider_string(self) -> &'static str {
        match self {
            Self::Gmail => "gmail_api",
            Self::Microsoft365 => "graph",
            Self::Jmap => "jmap",
            Self::Imap => "imap",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualAuthMethod {
    OAuth,
    Password,
}

impl ManualAuthMethod {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::OAuth => "OAuth",
            Self::Password => "Password",
        }
    }
}

// Manual config state

#[derive(Debug, Clone)]
pub struct ManualConfig {
    pub selected_provider: Option<ManualProvider>,
    pub jmap_url: String,
    pub auth_method: ManualAuthMethod,
}

impl Default for ManualConfig {
    fn default() -> Self {
        Self {
            selected_provider: None,
            jmap_url: String::new(),
            auth_method: ManualAuthMethod::OAuth,
        }
    }
}

// Auth state

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

// Account identity

#[derive(Debug, Clone)]
pub struct AccountIdentity {
    pub name: String,
    pub selected_color_index: Option<usize>,
}

// OAuth success

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

// Messages

#[derive(Debug, Clone)]
pub enum AddAccountMessage {
    // Step 1: Email
    EmailChanged(String),
    SubmitEmail,

    // Step 2: Discovery result
    DiscoveryComplete(
        rtsk::generation::GenerationToken<rtsk::generation::AddAccount>,
        Result<DiscoveredConfig, String>,
    ),
    SelectProtocol(usize),
    ConfirmProtocol,

    // Manual config
    SelectManualProvider(ManualProvider),
    ManualImapHostChanged(String),
    ManualImapPortChanged(String),
    ManualImapSecurityChanged(SecurityOption),
    ManualSmtpHostChanged(String),
    ManualSmtpPortChanged(String),
    ManualSmtpSecurityChanged(SecurityOption),
    ManualJmapUrlChanged(String),
    ManualAuthMethodChanged(ManualAuthMethod),
    SubmitManualConfig,

    // Step 3: Authentication
    // OAuth
    OAuthComplete(
        rtsk::generation::GenerationToken<rtsk::generation::AddAccount>,
        Result<OAuthSuccess, String>,
    ),
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
    ValidationComplete(
        rtsk::generation::GenerationToken<rtsk::generation::AddAccount>,
        Result<(), String>,
    ),

    // Step 4: Identity
    AccountNameChanged(String),
    SelectColor(usize),
    SubmitIdentity,

    // Step 5: Creation
    AccountCreated(
        rtsk::generation::GenerationToken<rtsk::generation::AddAccount>,
        Result<String, String>,
    ),

    // Re-auth: token/credential update
    ReauthTokensSaved(
        rtsk::generation::GenerationToken<rtsk::generation::AddAccount>,
        Result<(), String>,
    ),

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

// Wizard state

pub struct AddAccountWizard {
    pub step: AddAccountStep,
    pub is_first_launch: bool,
    pub email: String,
    pub error: Option<String>,
    pub generation: rtsk::generation::GenerationCounter<rtsk::generation::AddAccount>,
    // Discovery result
    pub discovery: Option<DiscoveredConfig>,
    pub selected_option: Option<usize>,
    // Manual configuration state (used when discovery fails)
    pub manual_config: ManualConfig,
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
    /// DB handle for account creation (writable). Phase 6a:
    /// `account.create` flows through `service_client` instead;
    /// `db` stays for read paths and the OAuth re-auth token write
    /// (the latter relocates in Phase 6b).
    pub(super) db: Arc<Db>,
    /// ServiceClient for the `account.create` IPC. Optional because
    /// the wizard can outlive a Service respawn; if `None`, the
    /// submit handler surfaces "Service not ready" to the user.
    pub(super) service_client: Option<Arc<crate::service_client::ServiceClient>>,
    /// Re-auth mode: when set, the wizard skips email/discovery/identity
    /// and goes straight to the auth step for this existing account.
    pub(super) reauth_account_id: Option<String>,
}

impl AddAccountWizard {
    pub fn new_first_launch(
        db: Arc<Db>,
        service_client: Option<Arc<crate::service_client::ServiceClient>>,
    ) -> Self {
        Self::new(true, Vec::new(), db, service_client)
    }

    pub fn new_add_account(
        used_colors: Vec<String>,
        db: Arc<Db>,
        service_client: Option<Arc<crate::service_client::ServiceClient>>,
    ) -> Self {
        Self::new(false, used_colors, db, service_client)
    }

    /// Create a re-auth wizard for an existing account. Looks up the
    /// account's auth method and skips straight to the appropriate
    /// auth step (OAuth or password).
    pub fn new_reauth(
        account_id: String,
        email: String,
        db: Arc<Db>,
        service_client: Option<Arc<crate::service_client::ServiceClient>>,
    ) -> Result<(Self, Task<AddAccountMessage>), String> {
        let auth_info = db.get_account_auth_info(&account_id)?;

        let mut wizard = Self::new(false, Vec::new(), db, service_client);
        wizard.email = email;
        wizard.reauth_account_id = Some(account_id);
        wizard.resolved_provider = auth_info.provider;
        wizard.resolved_auth_method = auth_info.auth_method.clone();

        let task = if auth_info.auth_method == "oauth2" {
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

    fn new(
        is_first_launch: bool,
        used_colors: Vec<String>,
        db: Arc<Db>,
        service_client: Option<Arc<crate::service_client::ServiceClient>>,
    ) -> Self {
        let presets = label_colors::preset_colors::all_presets();
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
            generation: rtsk::generation::GenerationCounter::new(),
            discovery: None,
            selected_option: None,
            manual_config: ManualConfig::default(),
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
            service_client,
            reauth_account_id: None,
        }
    }

    /// Get the selected color hex, or fallback to first preset.
    pub(super) fn selected_color_hex(&self) -> String {
        let presets = label_colors::preset_colors::all_presets();
        self.identity
            .selected_color_index
            .and_then(|i| presets.get(i))
            .map(|(_, bg, _)| (*bg).to_string())
            .unwrap_or_else(|| presets[0].1.to_string())
    }
}

// Component impl

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
            AddAccountMessage::DiscoveryComplete(g, _) if !self.generation.is_current(g) => {
                (Task::none(), None)
            }
            AddAccountMessage::DiscoveryComplete(_, Ok(config)) => {
                self.handle_discovery_result(&config)
            }
            AddAccountMessage::DiscoveryComplete(_, Err(e)) => {
                // Duplicate account errors go back to email input.
                // Discovery failures go to manual configuration.
                if e.contains("already configured") || e.contains("Database error") {
                    self.error = Some(e);
                    self.step = AddAccountStep::EmailInput;
                } else {
                    self.error = Some(format!("We couldn't auto-detect your mail server. {e}"));
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
            AddAccountMessage::ValidationComplete(g, _) if !self.generation.is_current(g) => {
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
                    let generation = self.generation.next();
                    let db = Arc::clone(&self.db);
                    let aid = account_id.clone();
                    let task = Task::perform(
                        async move {
                            let result = db
                                .with_write_conn(move |conn| {
                                    update_account_tokens_sync(conn, &aid, reauth_params)
                                })
                                .await;
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
            AddAccountMessage::OAuthComplete(g, _) if !self.generation.is_current(g) => {
                (Task::none(), None)
            }
            AddAccountMessage::OAuthComplete(_, Ok(success)) => self.handle_oauth_success(success),
            AddAccountMessage::OAuthComplete(_, Err(e)) => {
                self.error = Some(e);
                (Task::none(), None)
            }
            AddAccountMessage::RetryOAuth => self.handle_retry_oauth(),
            AddAccountMessage::SubmitIdentity => self.handle_submit_identity(),
            AddAccountMessage::AccountCreated(g, _) if !self.generation.is_current(g) => {
                (Task::none(), None)
            }
            AddAccountMessage::AccountCreated(_, Ok(account_id)) => (
                Task::none(),
                Some(AddAccountEvent::AccountAdded(account_id)),
            ),
            AddAccountMessage::AccountCreated(_, Err(e)) => {
                self.error = Some(e);
                self.step = AddAccountStep::Identity;
                (Task::none(), None)
            }
            AddAccountMessage::ReauthTokensSaved(g, _) if !self.generation.is_current(g) => {
                (Task::none(), None)
            }
            AddAccountMessage::ReauthTokensSaved(_, Ok(())) => {
                let account_id = self.reauth_account_id.clone().unwrap_or_default();
                (
                    Task::none(),
                    Some(AddAccountEvent::ReauthComplete(account_id)),
                )
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
            AddAccountStep::Discovering => super::discovery::view_discovering(),
            AddAccountStep::ProtocolSelection => self.view_protocol_selection(),
            AddAccountStep::ManualConfiguration => self.view_manual_config(),
            AddAccountStep::OAuthWaiting => self.view_oauth_waiting(),
            AddAccountStep::PasswordAuth => self.view_password_auth(),
            AddAccountStep::Validating => super::password_auth::view_validating(),
            AddAccountStep::Identity => self.view_identity(),
            AddAccountStep::Creating => super::identity::view_creating(),
        }
    }
}

// Update helpers

impl AddAccountWizard {
    pub(super) fn proceed_to_auth(
        &mut self,
        option: &ProtocolOption,
    ) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        // Set the provider string for account creation
        self.resolved_provider = super::discovery::protocol_to_db_provider(&option.protocol);

        match &option.auth.method {
            AuthMethod::OAuth2 {
                provider_id,
                auth_url,
                token_url,
                scopes,
                use_pkce,
            } => {
                self.resolved_auth_method = "oauth2".to_string();
                self.step = AddAccountStep::OAuthWaiting;
                self.error = None;
                let generation = self.generation.next();

                let request = rtsk::oauth::OAuthProviderAuthorizationRequest {
                    provider_id: provider_id.clone(),
                    auth_url: auth_url.clone(),
                    token_url: token_url.clone(),
                    scopes: scopes.clone(),
                    user_info_url: None,
                    use_pkce: *use_pkce,
                    client_id: super::oauth::resolve_client_id(provider_id),
                    client_secret: None,
                };

                let provider_id_clone = provider_id.clone();
                let client_id_clone = super::oauth::resolve_client_id(provider_id);

                let task = Task::perform(
                    async move {
                        let provider = rtsk::oauth::GenericOAuthProvider::from_request(request);
                        let open_url = |url: &str| -> Result<(), String> {
                            super::oauth::open_browser_url(url)
                        };
                        let result =
                            rtsk::oauth::authorize_with_provider(&provider, &open_url).await;
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
                let help_url = option.help_url.clone();
                self.prefill_auth_from_option(option);
                self.step = AddAccountStep::PasswordAuth;
                self.error = Some(if let Some(url) = help_url {
                    format!(
                        "This provider requires an app-specific password. \
                         See: {url}"
                    )
                } else {
                    format!(
                        "This provider requires an app-specific password. \
                         Check {provider_domain} for setup instructions."
                    )
                });
                (Task::none(), None)
            }
        }
    }

    pub(super) fn handle_back(&mut self) {
        // In re-auth mode, Back is equivalent to Cancel - there's no
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

    pub(super) fn handle_field_update(&mut self, message: AddAccountMessage) {
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
            AddAccountMessage::SelectManualProvider(provider) => {
                self.manual_config.selected_provider = Some(provider);
            }
            AddAccountMessage::ManualImapHostChanged(v) => {
                self.auth_state.imap_host = v;
            }
            AddAccountMessage::ManualImapPortChanged(v) => {
                self.auth_state.imap_port = v;
            }
            AddAccountMessage::ManualImapSecurityChanged(v) => {
                self.auth_state.imap_security = v;
            }
            AddAccountMessage::ManualSmtpHostChanged(v) => {
                self.auth_state.smtp_host = v;
            }
            AddAccountMessage::ManualSmtpPortChanged(v) => {
                self.auth_state.smtp_port = v;
            }
            AddAccountMessage::ManualSmtpSecurityChanged(v) => {
                self.auth_state.smtp_security = v;
            }
            AddAccountMessage::ManualJmapUrlChanged(v) => {
                self.manual_config.jmap_url = v;
            }
            AddAccountMessage::ManualAuthMethodChanged(v) => {
                self.manual_config.auth_method = v;
            }
            AddAccountMessage::AccountNameChanged(v) => self.identity.name = v,
            AddAccountMessage::SelectColor(i) => {
                self.identity.selected_color_index = Some(i);
            }
            _ => {}
        }
    }

    pub(super) fn prefill_auth_from_option(&mut self, option: &ProtocolOption) {
        if let rtsk::discovery::types::Protocol::Imap {
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
}
