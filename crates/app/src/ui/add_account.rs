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

// ── Messages ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum AddAccountMessage {
    // Step 1: Email
    EmailChanged(String),
    SubmitEmail,

    // Step 2: Discovery result
    DiscoveryComplete(u64, Result<(), String>),

    // Manual config
    ManualImapHostChanged(String),
    ManualImapPortChanged(String),
    ManualSmtpHostChanged(String),
    ManualSmtpPortChanged(String),
    SubmitManualConfig,

    // Step 3: Authentication
    // OAuth
    CancelOAuth,

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

    // Step 4: Identity
    AccountNameChanged(String),
    SelectColor(usize),
    SubmitIdentity,

    // Step 5: Creation
    AccountCreated(u64, Result<String, String>),

    // General
    Cancel,
    Back,
}

/// Events emitted to App.
#[derive(Debug, Clone)]
pub enum AddAccountEvent {
    /// Wizard completed successfully. Carry the new account ID.
    AccountAdded(String),
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
    // Auth state
    pub auth_state: AuthState,
    // Identity
    pub identity: AccountIdentity,
    /// Colors already assigned to existing accounts (hex strings).
    pub used_colors: Vec<String>,
    /// DB handle for account creation (writable).
    db: Arc<Db>,
}

impl AddAccountWizard {
    pub fn new_first_launch(db: Arc<Db>) -> Self {
        Self::new(true, Vec::new(), db)
    }

    pub fn new_add_account(used_colors: Vec<String>, db: Arc<Db>) -> Self {
        Self::new(false, used_colors, db)
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
            auth_state: AuthState::default(),
            identity: AccountIdentity {
                name: String::new(),
                selected_color_index: Some(first_unused),
            },
            used_colors,
            db,
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
            AddAccountMessage::DiscoveryComplete(_, Ok(())) => {
                // Discovery succeeded — go to password auth for now
                // TODO: Wire real discovery to determine OAuth vs password
                self.prefill_from_email();
                self.step = AddAccountStep::PasswordAuth;
                (Task::none(), None)
            }
            AddAccountMessage::DiscoveryComplete(_, Err(e)) => {
                self.error = Some(format!(
                    "We couldn't auto-detect your mail server. {e}"
                ));
                self.step = AddAccountStep::ManualConfiguration;
                (Task::none(), None)
            }
            AddAccountMessage::SubmitManualConfig => {
                self.handle_submit_manual_config()
            }
            AddAccountMessage::SubmitCredentials => {
                self.handle_submit_credentials()
            }
            AddAccountMessage::SubmitIdentity => {
                self.handle_submit_identity()
            }
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
            AddAccountMessage::Cancel | AddAccountMessage::CancelOAuth => {
                (Task::none(), Some(AddAccountEvent::Cancelled))
            }
            AddAccountMessage::Back => {
                self.handle_back();
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
            AddAccountStep::ProtocolSelection => view_protocol_selection(),
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
        self.email = email;
        self.step = AddAccountStep::Discovering;
        self.error = None;
        self.generation += 1;
        let generation = self.generation;

        // TODO: Wire real discovery via ratatoskr_core::discovery::discover().
        // For now, simulate discovery completing after a short delay by
        // immediately returning success — the state machine structure is
        // correct and the real discovery call can be dropped in later.
        let task = Task::perform(
            async move {
                // Placeholder: real discovery would run here
                (generation, Ok(()))
            },
            |(g, result)| AddAccountMessage::DiscoveryComplete(g, result),
        );
        (task, None)
    }

    fn handle_submit_manual_config(
        &mut self,
    ) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        // Copy manual config fields to auth state and proceed to password auth
        self.prefill_from_email();
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
        // TODO: Wire credential validation (test IMAP connection).
        // For now, skip validation and go straight to identity step.
        self.prefill_identity_name();
        self.step = AddAccountStep::Identity;
        self.error = None;
        (Task::none(), None)
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

        // Build the account creation params
        let color = self.selected_color_hex();
        let account_name = self.identity.name.trim().to_string();
        let email = self.email.clone();
        let auth = self.auth_state.clone();
        let db = Arc::clone(&self.db);

        let task = Task::perform(
            async move {
                let account_id = uuid::Uuid::new_v4().to_string();
                let aid = account_id.clone();
                db.with_write_conn(move |conn| {
                    // SMTP credentials: separate if the user toggled the
                    // checkbox, otherwise same as IMAP.
                    let (smtp_user, smtp_pass) = if auth.use_separate_smtp_credentials {
                        (
                            Some(auth.smtp_username.clone()),
                            Some(auth.smtp_password.clone()),
                        )
                    } else {
                        (None, None)
                    };

                    conn.execute(
                        "INSERT INTO accounts (
                            id, email, display_name, provider, auth_method,
                            imap_host, imap_port, imap_security,
                            imap_username, imap_password,
                            smtp_host, smtp_port, smtp_security,
                            smtp_username, smtp_password,
                            accept_invalid_certs,
                            account_name, account_color
                        ) VALUES (
                            ?1, ?2, NULL, 'imap', 'password',
                            ?3, ?4, ?5, ?6, ?7,
                            ?8, ?9, ?10, ?11, ?12,
                            ?13, ?14, ?15
                        )",
                        rusqlite::params![
                            aid,
                            email,
                            auth.imap_host,
                            auth.imap_port,
                            auth.imap_security.to_db_string(),
                            auth.username,
                            auth.password,
                            auth.smtp_host,
                            auth.smtp_port,
                            auth.smtp_security.to_db_string(),
                            smtp_user,
                            smtp_pass,
                            if auth.accept_invalid_certs { 1 } else { 0 },
                            account_name,
                            color,
                        ],
                    )
                    .map_err(|e| format!("Failed to create account: {e}"))?;
                    Ok(aid)
                })
                .await
                .map(|id| (generation, Ok(id)))
                .unwrap_or_else(|e| (generation, Err(e)))
            },
            |(g, result): (u64, Result<String, String>)| {
                AddAccountMessage::AccountCreated(g, result)
            },
        );
        (task, None)
    }

    fn handle_back(&mut self) {
        match self.step {
            AddAccountStep::PasswordAuth | AddAccountStep::ManualConfiguration => {
                self.step = AddAccountStep::EmailInput;
                self.error = None;
            }
            AddAccountStep::Identity => {
                self.step = AddAccountStep::PasswordAuth;
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

// ── View: Protocol Selection (placeholder) ───────────────

fn view_protocol_selection<'a>() -> Element<'a, AddAccountMessage> {
    // TODO: Render discovered protocol options as selectable cards.
    // For now this step is skipped — discovery goes straight to auth.
    column![
        text("Choose your email provider")
            .size(TEXT_HEADING)
            .style(text::base)
            .font(iced::Font {
                weight: iced::font::Weight::Bold,
                ..font::text()
            }),
        Space::new().height(SPACE_MD),
        text("Protocol selection coming soon.")
            .size(TEXT_LG)
            .style(text::secondary),
        Space::new().height(SPACE_LG),
        ghost_button("Cancel", AddAccountMessage::Cancel),
    ]
    .spacing(SPACE_XS)
    .align_x(Alignment::Center)
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

        col = col.push(
            text("Complete sign-in in your browser")
                .size(TEXT_HEADING)
                .style(text::base)
                .font(iced::Font {
                    weight: iced::font::Weight::Bold,
                    ..font::text()
                }),
        );
        col = col.push(Space::new().height(SPACE_MD));
        col = col.push(
            text("Waiting for authorization...")
                .size(TEXT_LG)
                .style(text::secondary),
        );

        if let Some(ref err) = self.error {
            col = col.push(Space::new().height(SPACE_SM));
            col = col.push(text(err.as_str()).size(TEXT_SM).style(text::danger));
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

        col = col.push(
            text("Sign In")
                .size(TEXT_HEADING)
                .style(text::base)
                .font(iced::Font {
                    weight: iced::font::Weight::Bold,
                    ..font::text()
                }),
        );

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

        vec![frame.into_geometry()]
    }
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
