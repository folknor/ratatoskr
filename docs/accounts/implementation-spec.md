# Accounts: Implementation Spec

## Overview

This spec covers the UI layer for account management in Ratatoskr. The backend is complete: auto-discovery (`crates/core/src/discovery/`), OAuth with PKCE (`crates/core/src/oauth.rs`), IMAP credential handling, and account storage (`crates/db/src/db/types.rs` `DbAccount`). This spec defines the app-side state, messages, views, and phasing for:

1. First-launch experience (centered modal over empty window)
2. Add Account wizard (multi-step state machine)
3. Account management in Settings (card list + slide-in editor)
4. Account selector wiring (sidebar dropdown enhancements)
5. Error states and recovery flows

Reference: `docs/accounts/problem-statement.md` for the full product spec.

---

## Phase 0: Data Model Extensions

### Account color column

The `DbAccount` type (`crates/db/src/db/types.rs`) does not currently have an `account_color` field. Before any UI work, add:

```sql
ALTER TABLE accounts ADD COLUMN account_color TEXT;
ALTER TABLE accounts ADD COLUMN account_name TEXT;
ALTER TABLE accounts ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0;
```

And extend `DbAccount`:

```rust
// In crates/db/src/db/types.rs
pub struct DbAccount {
    // ... existing fields ...
    pub account_color: Option<String>,   // hex like "#e74c3c"
    pub account_name: Option<String>,    // user-chosen label ("Work", "Personal")
    pub sort_order: i64,
}
```

The app-side `db::Account` type (`crates/app/src/db.rs`) gains corresponding fields:

```rust
pub struct Account {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub provider: String,
    pub account_name: Option<String>,
    pub account_color: Option<String>,
    pub last_sync_at: Option<i64>,
    pub sort_order: i64,
}
```

### Core DB functions needed

Add to `crates/core/src/db/queries_extra/`:

```rust
// accounts_crud.rs (new file)

pub async fn db_create_account(db: &DbState, params: CreateAccountParams) -> Result<String, String>;
pub async fn db_update_account(db: &DbState, id: String, params: UpdateAccountParams) -> Result<(), String>;
pub async fn db_update_account_color(db: &DbState, id: String, color: String) -> Result<(), String>;
pub async fn db_update_account_name(db: &DbState, id: String, name: String) -> Result<(), String>;
pub async fn db_update_account_sort_order(db: &DbState, updates: Vec<(String, i64)>) -> Result<(), String>;
pub async fn db_account_exists_by_email(db: &DbState, email: String) -> Result<bool, String>;

pub struct CreateAccountParams {
    pub email: String,
    pub provider: String,
    pub display_name: Option<String>,
    pub account_name: String,
    pub account_color: String,
    pub auth_method: String,
    // OAuth fields
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub token_expires_at: Option<i64>,
    pub oauth_provider: Option<String>,
    pub oauth_client_id: Option<String>,
    // IMAP fields
    pub imap_host: Option<String>,
    pub imap_port: Option<i64>,
    pub imap_security: Option<String>,
    pub imap_username: Option<String>,
    pub imap_password: Option<String>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i64>,
    pub smtp_security: Option<String>,
    // JMAP fields
    pub jmap_url: Option<String>,
    pub accept_invalid_certs: bool,
}

pub struct UpdateAccountParams {
    pub account_name: Option<String>,
    pub display_name: Option<String>,
    pub account_color: Option<String>,
    pub caldav_url: Option<String>,
    pub caldav_username: Option<String>,
    pub caldav_password: Option<String>,
}
```

### Account health type

Define in `crates/app/src/db.rs` (or a new `crates/app/src/account_health.rs`):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountHealth {
    Healthy,
    Warning,
    Error,
    Disabled,
}
```

Health is derived client-side from the account's token state and last sync time. No DB column -- computed on demand:

```rust
fn compute_health(account: &Account) -> AccountHealth {
    // token_expires_at in the past + no successful recent sync → Error
    // token_expires_at within 24h → Warning
    // is_active == 0 → Disabled
    // otherwise → Healthy
}
```

---

## Phase 1: First Launch Detection & Empty State

### Goal

When the app has zero accounts, show a centered modal instead of the normal three-panel layout. This is the entry point for the Add Account wizard.

### App state changes

In `crates/app/src/main.rs`, add to `App`:

```rust
struct App {
    // ... existing fields ...

    /// True when the app has no configured accounts.
    /// Set during boot, updated after account add/delete.
    no_accounts: bool,

    /// The add-account wizard state. Some when the modal is open.
    add_account_wizard: Option<AddAccountWizard>,
}
```

### Boot flow change

Currently `boot()` loads accounts and auto-selects the first one. Change to:

```rust
fn boot() -> (Self, Task<Message>) {
    // ... existing setup ...
    let app = Self {
        // ... existing fields ...
        no_accounts: false, // will be set by AccountsLoaded
        add_account_wizard: None,
    };
    // load accounts as before
}
```

In `handle_accounts_loaded`:

```rust
fn handle_accounts_loaded(&mut self, accounts: Vec<db::Account>) -> Task<Message> {
    self.sidebar.accounts = accounts;
    if self.sidebar.accounts.is_empty() {
        self.no_accounts = true;
        self.add_account_wizard = Some(AddAccountWizard::new_first_launch());
        self.status = "Welcome".to_string();
        return Task::none();
    }
    self.no_accounts = false;
    // ... existing logic: select first account, load labels/threads ...
}
```

### View routing

In `App::view()`:

```rust
fn view(&self) -> Element<'_, Message> {
    // Add-account wizard modal takes precedence over everything
    if let Some(ref wizard) = self.add_account_wizard {
        if self.no_accounts {
            // First launch: modal over empty window
            return self.view_first_launch_modal(wizard);
        }
        // Subsequent add: modal over existing layout
        return self.view_with_modal(wizard);
    }

    if self.show_settings {
        return self.settings.view().map(Message::Settings);
    }

    // ... existing three-panel layout ...
}
```

### First-launch modal view

```rust
fn view_first_launch_modal(&self, wizard: &AddAccountWizard) -> Element<'_, Message> {
    let modal_content = wizard.view().map(Message::AddAccount);

    let modal = container(modal_content)
        .width(Length::Fixed(ACCOUNT_MODAL_WIDTH))
        .max_height(ACCOUNT_MODAL_MAX_HEIGHT)
        .padding(PAD_SETTINGS_CONTENT)
        .style(theme::ContainerClass::Elevated.style());

    // Center the modal in the window
    container(modal)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(theme::ContainerClass::Content.style())
        .into()
}
```

### Layout constants

Add to `crates/app/src/ui/layout.rs`:

```rust
/// Add Account modal width
pub const ACCOUNT_MODAL_WIDTH: f32 = 520.0;
/// Add Account modal max height
pub const ACCOUNT_MODAL_MAX_HEIGHT: f32 = 640.0;
/// Color swatch size in the palette picker
pub const COLOR_SWATCH_SIZE: f32 = 28.0;
/// Color palette grid columns
pub const COLOR_PALETTE_COLUMNS: usize = 5;
/// Protocol selection card height
pub const PROTOCOL_CARD_HEIGHT: f32 = 64.0;
```

---

## Phase 2: Add Account Wizard — State Machine

### State types

New file: `crates/app/src/ui/add_account.rs`

```rust
use ratatoskr_core::discovery::types::{DiscoveredConfig, ProtocolOption, Protocol, Security};
use ratatoskr_label_colors::category_colors::all_presets;

/// The multi-step add-account wizard.
pub struct AddAccountWizard {
    pub step: AddAccountStep,
    pub is_first_launch: bool,
    /// Email entered in step 1, carried through all steps.
    pub email: String,
    /// Discovery results, populated after step 1.
    pub discovery: Option<DiscoveredConfig>,
    /// The selected protocol option (index into discovery.options).
    pub selected_option: Option<usize>,
    /// Manual configuration state (used when discovery fails).
    pub manual_config: ManualConfig,
    /// Authentication state.
    pub auth_state: AuthState,
    /// Account identity state (step 4).
    pub identity: AccountIdentity,
    /// Colors already assigned to existing accounts (hex strings).
    pub used_colors: Vec<String>,
    /// Error message to display (contextual to current step).
    pub error: Option<String>,
    /// Generation counter for async task staleness detection.
    pub generation: u64,
}

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

pub struct ManualConfig {
    pub selected_provider: Option<ManualProvider>,
    pub imap_host: String,
    pub imap_port: String,
    pub imap_security: SecurityOption,
    pub smtp_host: String,
    pub smtp_port: String,
    pub smtp_security: SecurityOption,
    pub jmap_url: String,
    pub auth_method: ManualAuthMethod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualProvider {
    Gmail,
    Microsoft365,
    Jmap,
    Imap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityOption {
    Tls,
    StartTls,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualAuthMethod {
    OAuth,
    Password,
}

pub struct AuthState {
    pub username: String,
    pub password: String,
    pub smtp_username: String,
    pub smtp_password: String,
    pub use_separate_smtp_credentials: bool,
    pub accept_invalid_certs: bool,
    /// Pre-filled server details from discovery (displayed alongside password fields).
    pub imap_host: String,
    pub imap_port: String,
    pub imap_security: SecurityOption,
    pub smtp_host: String,
    pub smtp_port: String,
    pub smtp_security: SecurityOption,
}

pub struct AccountIdentity {
    pub name: String,
    pub selected_color_index: Option<usize>,
}
```

### Constructor

```rust
impl AddAccountWizard {
    pub fn new_first_launch() -> Self {
        Self::new(true, Vec::new())
    }

    pub fn new_add_account(used_colors: Vec<String>) -> Self {
        Self::new(false, used_colors)
    }

    fn new(is_first_launch: bool, used_colors: Vec<String>) -> Self {
        // Pre-select the first unassigned color
        let presets = all_presets();
        let first_unused = presets.iter().enumerate().find(|(_, (_, bg, _))| {
            !used_colors.iter().any(|uc| uc == *bg)
        }).map(|(i, _)| i).unwrap_or(0);

        Self {
            step: AddAccountStep::EmailInput,
            is_first_launch,
            email: String::new(),
            discovery: None,
            selected_option: None,
            manual_config: ManualConfig::default(),
            auth_state: AuthState::default(),
            identity: AccountIdentity {
                name: String::new(),
                selected_color_index: Some(first_unused),
            },
            used_colors,
            error: None,
            generation: 0,
        }
    }
}
```

### Messages

```rust
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
    AccountCreated(u64, Result<String, String>), // Ok(account_id)

    // General
    Cancel,
    Back,
    DismissError,
}

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
```

### Events (emitted to App)

```rust
#[derive(Debug, Clone)]
pub enum AddAccountEvent {
    /// Wizard completed successfully. Carry the new account ID.
    AccountAdded(String),
    /// Wizard cancelled.
    Cancelled,
}
```

### Component implementation

```rust
impl Component for AddAccountWizard {
    type Message = AddAccountMessage;
    type Event = AddAccountEvent;

    fn update(&mut self, message: AddAccountMessage) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        match message {
            AddAccountMessage::EmailChanged(email) => {
                self.email = email;
                self.error = None;
                (Task::none(), None)
            }

            AddAccountMessage::SubmitEmail => {
                let email = self.email.trim().to_lowercase();
                if email.is_empty() || !email.contains('@') {
                    self.error = Some("Please enter a valid email address.".to_string());
                    return (Task::none(), None);
                }
                self.email = email.clone();
                self.step = AddAccountStep::Discovering;
                self.error = None;
                self.generation += 1;
                let gen = self.generation;

                // Check for duplicate account first, then run discovery
                let task = Task::perform(
                    async move {
                        // TODO: check db_account_exists_by_email here
                        let result = ratatoskr_core::discovery::discover(&email).await;
                        (gen, result)
                    },
                    |(gen, result)| AddAccountMessage::DiscoveryComplete(gen, result),
                );
                (task, None)
            }

            AddAccountMessage::DiscoveryComplete(gen, _) if gen != self.generation => {
                (Task::none(), None)
            }

            AddAccountMessage::DiscoveryComplete(_, Ok(config)) => {
                self.handle_discovery_result(config)
            }

            AddAccountMessage::DiscoveryComplete(_, Err(e)) => {
                // Discovery failed entirely — show manual config
                self.error = Some(format!("We couldn't auto-detect your mail server. {e}"));
                self.step = AddAccountStep::ManualConfiguration;
                (Task::none(), None)
            }

            AddAccountMessage::Cancel => {
                (Task::none(), Some(AddAccountEvent::Cancelled))
            }

            // ... other arms follow the same pattern ...

            _ => (Task::none(), None),
        }
    }

    fn view(&self) -> Element<'_, AddAccountMessage> {
        match self.step {
            AddAccountStep::EmailInput => self.view_email_input(),
            AddAccountStep::Discovering => self.view_discovering(),
            AddAccountStep::ProtocolSelection => self.view_protocol_selection(),
            AddAccountStep::ManualConfiguration => self.view_manual_config(),
            AddAccountStep::OAuthWaiting => self.view_oauth_waiting(),
            AddAccountStep::PasswordAuth => self.view_password_auth(),
            AddAccountStep::Validating => self.view_validating(),
            AddAccountStep::Identity => self.view_identity(),
            AddAccountStep::Creating => self.view_creating(),
        }
    }
}
```

### Discovery result handling

```rust
impl AddAccountWizard {
    fn handle_discovery_result(
        &mut self,
        config: DiscoveredConfig,
    ) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        if config.options.is_empty() {
            self.error = Some("We couldn't auto-detect your mail server.".to_string());
            self.step = AddAccountStep::ManualConfiguration;
            return (Task::none(), None);
        }

        self.discovery = Some(config.clone());

        // High confidence: single option from registry source
        let high_confidence = config.options.len() == 1
            && config.options[0].source.confidence() == 0;

        if high_confidence {
            self.selected_option = Some(0);
            return self.proceed_to_auth(&config.options[0]);
        }

        // Multiple options or lower confidence: show selection
        self.selected_option = Some(0); // pre-select top option
        self.step = AddAccountStep::ProtocolSelection;
        (Task::none(), None)
    }

    fn proceed_to_auth(
        &mut self,
        option: &ProtocolOption,
    ) -> (Task<AddAccountMessage>, Option<AddAccountEvent>) {
        match &option.auth.method {
            AuthMethod::OAuth2 { provider_id, auth_url, token_url, scopes, use_pkce } => {
                self.step = AddAccountStep::OAuthWaiting;
                self.generation += 1;
                let gen = self.generation;

                // Build OAuth provider and launch flow
                let request = OAuthProviderAuthorizationRequest {
                    provider_id: provider_id.clone(),
                    auth_url: auth_url.clone(),
                    token_url: token_url.clone(),
                    scopes: scopes.clone(),
                    user_info_url: None,
                    use_pkce: *use_pkce,
                    client_id: self.resolve_client_id(provider_id),
                    client_secret: None,
                };

                let task = Task::perform(
                    async move {
                        let provider = GenericOAuthProvider::from_request(request);
                        let open_url = |url: &str| -> Result<(), String> {
                            open::that(url).map_err(|e| format!("Failed to open browser: {e}"))
                        };
                        let result = authorize_with_provider(&provider, &open_url).await;
                        // Map to OAuthSuccess
                        let mapped = result.map(|bundle| OAuthSuccess {
                            access_token: bundle.tokens.access_token,
                            refresh_token: bundle.tokens.refresh_token,
                            token_expires_at: Some(
                                chrono::Utc::now().timestamp() + bundle.tokens.expires_in as i64
                            ),
                            user_email: bundle.user_info.email,
                            user_name: bundle.user_info.name,
                            oauth_provider: provider_id_clone,
                            oauth_client_id: client_id_clone,
                        });
                        (gen, mapped)
                    },
                    |(gen, result)| AddAccountMessage::OAuthComplete(gen, result),
                );
                (task, None)
            }

            AuthMethod::Password => {
                // Pre-fill server fields from discovery
                self.prefill_auth_from_option(option);
                self.step = AddAccountStep::PasswordAuth;
                (Task::none(), None)
            }

            AuthMethod::OAuth2Unsupported { provider_domain } => {
                // Show password form with help link
                self.prefill_auth_from_option(option);
                self.step = AddAccountStep::PasswordAuth;
                self.error = Some(format!(
                    "This provider requires an app-specific password. Check {provider_domain} for setup instructions."
                ));
                (Task::none(), None)
            }
        }
    }

    fn prefill_auth_from_option(&mut self, option: &ProtocolOption) {
        if let Protocol::Imap { ref incoming, ref outgoing } = option.protocol {
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
        // Pre-fill identity name from domain
        let domain = self.email.split('@').nth(1).unwrap_or("");
        let name = domain.split('.').next().unwrap_or(domain);
        self.identity.name = titlecase(name);
    }
}
```

---

## Phase 3: Wizard Views

### Step 1: Email Input

```rust
fn view_email_input(&self) -> Element<'_, AddAccountMessage> {
    let mut col = column![].spacing(SPACE_LG).align_x(Alignment::Center);

    if self.is_first_launch {
        // App icon placeholder (will be a real icon once assets are ready)
        col = col.push(
            container(icon::mail().size(48.0).style(text::primary))
                .align_x(Alignment::Center),
        );
        col = col.push(Space::new().height(SPACE_SM));
        col = col.push(
            text("Welcome to Ratatoskr")
                .size(TEXT_HEADING)
                .style(text::base)
                .font(Font { weight: Weight::Bold, ..font::text() }),
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
                .font(Font { weight: Weight::Bold, ..font::text() }),
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
        col = col.push(
            text(err.as_str())
                .size(TEXT_SM)
                .style(text::danger),
        );
    }

    col = col.push(
        button(
            container(text("Continue").size(TEXT_LG).color(theme::ON_AVATAR))
                .center_x(Length::Fill),
        )
        .on_press(AddAccountMessage::SubmitEmail)
        .padding(PAD_BUTTON)
        .style(theme::ButtonClass::Primary.style())
        .width(Length::Fill),
    );

    if !self.is_first_launch {
        col = col.push(
            button(text("Cancel").size(TEXT_LG).style(text::secondary))
                .on_press(AddAccountMessage::Cancel)
                .padding(PAD_BUTTON)
                .style(theme::ButtonClass::Ghost.style())
                .width(Length::Fill),
        );
    }

    col.width(Length::Fill).into()
}
```

### Step 1b: Discovering (spinner)

```rust
fn view_discovering(&self) -> Element<'_, AddAccountMessage> {
    let col = column![
        text("Looking up your email provider...")
            .size(TEXT_LG)
            .style(text::secondary),
        Space::new().height(SPACE_MD),
        // Use a simple "..." animation or a throbber widget
        text("Please wait...")
            .size(TEXT_SM)
            .style(text::secondary),
        Space::new().height(SPACE_LG),
        button(text("Cancel").size(TEXT_LG).style(text::secondary))
            .on_press(AddAccountMessage::Cancel)
            .padding(PAD_BUTTON)
            .style(theme::ButtonClass::Ghost.style()),
    ]
    .spacing(SPACE_XS)
    .align_x(Alignment::Center)
    .width(Length::Fill);

    col.into()
}
```

### Step 2: Protocol Selection

```rust
fn view_protocol_selection(&self) -> Element<'_, AddAccountMessage> {
    let config = self.discovery.as_ref().expect("discovery result present");
    let mut col = column![
        text("Choose your email provider")
            .size(TEXT_HEADING)
            .style(text::base)
            .font(Font { weight: Weight::Bold, ..font::text() }),
        Space::new().height(SPACE_XS),
        text(&self.email)
            .size(TEXT_LG)
            .style(text::secondary),
    ]
    .spacing(SPACE_XS)
    .width(Length::Fill);

    col = col.push(Space::new().height(SPACE_MD));

    for (i, option) in config.options.iter().enumerate() {
        let selected = self.selected_option == Some(i);
        col = col.push(protocol_card(option, i, selected));
    }

    col = col.push(Space::new().height(SPACE_MD));

    col = col.push(
        button(
            container(text("Continue").size(TEXT_LG).color(theme::ON_AVATAR))
                .center_x(Length::Fill),
        )
        .on_press(AddAccountMessage::ConfirmProtocol)
        .padding(PAD_BUTTON)
        .style(theme::ButtonClass::Primary.style())
        .width(Length::Fill),
    );

    col = col.push(
        button(text("Cancel").size(TEXT_LG).style(text::secondary))
            .on_press(AddAccountMessage::Cancel)
            .padding(PAD_BUTTON)
            .style(theme::ButtonClass::Ghost.style())
            .width(Length::Fill),
    );

    col.into()
}
```

#### Protocol card widget

Add to `crates/app/src/ui/widgets.rs`:

```rust
/// A selectable card for a discovered email protocol option.
pub fn protocol_card<'a, M: Clone + 'a>(
    option: &'a ProtocolOption,
    index: usize,
    selected: bool,
    on_select: impl Fn(usize) -> M,
) -> Element<'a, M> {
    let name = protocol_display_name(&option.protocol, option.provider_name.as_deref());
    let detail = protocol_detail(&option.protocol);
    let source_label = source_display(&option.source);

    let content = row![
        container(
            column![
                text(name).size(TEXT_LG).style(text::base)
                    .font(Font { weight: Weight::Bold, ..font::text() }),
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

    button(
        container(content)
            .padding(PAD_CARD)
            .width(Length::Fill)
            .height(PROTOCOL_CARD_HEIGHT),
    )
    .on_press(on_select(index))
    .padding(0)
    .style(if selected {
        theme::ButtonClass::ProtocolCardSelected.style()
    } else {
        theme::ButtonClass::ProtocolCard.style()
    })
    .width(Length::Fill)
    .into()
}
```

Helper functions:

```rust
fn protocol_display_name(protocol: &Protocol, provider_name: Option<&str>) -> &str {
    match (protocol, provider_name) {
        (_, Some(name)) => name,
        (Protocol::GmailApi, _) => "Gmail",
        (Protocol::MicrosoftGraph, _) => "Microsoft 365",
        (Protocol::Jmap { .. }, _) => "JMAP",
        (Protocol::Imap { .. }, _) => "IMAP",
    }
}

fn protocol_detail(protocol: &Protocol) -> String {
    match protocol {
        Protocol::GmailApi => "Gmail API (recommended)".to_string(),
        Protocol::MicrosoftGraph => "Microsoft Graph API".to_string(),
        Protocol::Jmap { session_url } => format!("JMAP: {session_url}"),
        Protocol::Imap { incoming, outgoing } => {
            format!("IMAP: {}:{} / SMTP: {}:{}", incoming.hostname, incoming.port, outgoing.hostname, outgoing.port)
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
```

### Step 3a: OAuth Waiting

```rust
fn view_oauth_waiting(&self) -> Element<'_, AddAccountMessage> {
    let mut col = column![
        text("Complete sign-in in your browser")
            .size(TEXT_HEADING)
            .style(text::base)
            .font(Font { weight: Weight::Bold, ..font::text() }),
        Space::new().height(SPACE_MD),
        text("Waiting for authorization...")
            .size(TEXT_LG)
            .style(text::secondary),
    ]
    .spacing(SPACE_XS)
    .align_x(Alignment::Center)
    .width(Length::Fill);

    if let Some(ref err) = self.error {
        col = col.push(Space::new().height(SPACE_SM));
        col = col.push(
            text(err.as_str())
                .size(TEXT_SM)
                .style(text::danger),
        );
        col = col.push(Space::new().height(SPACE_SM));
        col = col.push(
            button(
                container(text("Retry").size(TEXT_LG).color(theme::ON_AVATAR))
                    .center_x(Length::Fill),
            )
            .on_press(AddAccountMessage::RetryOAuth)
            .padding(PAD_BUTTON)
            .style(theme::ButtonClass::Primary.style())
            .width(Length::Fill),
        );
    }

    col = col.push(Space::new().height(SPACE_LG));
    col = col.push(
        button(text("Cancel").size(TEXT_LG).style(text::secondary))
            .on_press(AddAccountMessage::CancelOAuth)
            .padding(PAD_BUTTON)
            .style(theme::ButtonClass::Ghost.style()),
    );

    col.into()
}
```

### Step 3b: Password Auth

```rust
fn view_password_auth(&self) -> Element<'_, AddAccountMessage> {
    let mut col = column![].spacing(SPACE_MD).width(Length::Fill);

    col = col.push(
        text("Sign In")
            .size(TEXT_HEADING)
            .style(text::base)
            .font(Font { weight: Weight::Bold, ..font::text() }),
    );

    // IMAP section
    col = col.push(text("Incoming (IMAP)").size(TEXT_XL).style(text::base));
    col = col.push(
        row![
            column![
                text("Server").size(TEXT_SM).style(text::secondary),
                text_input("imap.example.com", &self.auth_state.imap_host)
                    .on_input(AddAccountMessage::AuthImapHostChanged)
                    .size(TEXT_LG).padding(PAD_INPUT)
                    .style(theme::TextInputClass::Settings.style()),
            ].spacing(SPACE_XXXS).width(Length::FillPortion(3)),
            column![
                text("Port").size(TEXT_SM).style(text::secondary),
                text_input("993", &self.auth_state.imap_port)
                    .on_input(AddAccountMessage::AuthImapPortChanged)
                    .size(TEXT_LG).padding(PAD_INPUT)
                    .style(theme::TextInputClass::Settings.style()),
            ].spacing(SPACE_XXXS).width(Length::FillPortion(1)),
        ].spacing(SPACE_SM),
    );

    // Security dropdown for IMAP (use radio buttons for simplicity)
    col = col.push(security_selector(
        "imap",
        self.auth_state.imap_security,
        AddAccountMessage::AuthImapSecurityChanged,
    ));

    col = col.push(
        column![
            text("Username").size(TEXT_SM).style(text::secondary),
            text_input("alice@example.com", &self.auth_state.username)
                .on_input(AddAccountMessage::UsernameChanged)
                .size(TEXT_LG).padding(PAD_INPUT)
                .style(theme::TextInputClass::Settings.style()),
        ].spacing(SPACE_XXXS).width(Length::Fill),
    );

    col = col.push(
        column![
            text("Password").size(TEXT_SM).style(text::secondary),
            text_input("", &self.auth_state.password)
                .on_input(AddAccountMessage::PasswordChanged)
                .size(TEXT_LG).padding(PAD_INPUT)
                .style(theme::TextInputClass::Settings.style()),
                // Note: plaintext per problem-statement.md, no .secure(true)
        ].spacing(SPACE_XXXS).width(Length::Fill),
    );

    // SMTP section
    col = col.push(Space::new().height(SPACE_SM));
    col = col.push(text("Outgoing (SMTP)").size(TEXT_XL).style(text::base));
    col = col.push(
        row![
            column![
                text("Server").size(TEXT_SM).style(text::secondary),
                text_input("smtp.example.com", &self.auth_state.smtp_host)
                    .on_input(AddAccountMessage::AuthSmtpHostChanged)
                    .size(TEXT_LG).padding(PAD_INPUT)
                    .style(theme::TextInputClass::Settings.style()),
            ].spacing(SPACE_XXXS).width(Length::FillPortion(3)),
            column![
                text("Port").size(TEXT_SM).style(text::secondary),
                text_input("587", &self.auth_state.smtp_port)
                    .on_input(AddAccountMessage::AuthSmtpPortChanged)
                    .size(TEXT_LG).padding(PAD_INPUT)
                    .style(theme::TextInputClass::Settings.style()),
            ].spacing(SPACE_XXXS).width(Length::FillPortion(1)),
        ].spacing(SPACE_SM),
    );

    col = col.push(security_selector(
        "smtp",
        self.auth_state.smtp_security,
        AddAccountMessage::AuthSmtpSecurityChanged,
    ));

    // Separate SMTP credentials toggle
    col = col.push(
        row![
            iced::widget::checkbox("Use different credentials for SMTP", self.auth_state.use_separate_smtp_credentials)
                .on_toggle(AddAccountMessage::ToggleSeparateSmtpCredentials)
                .size(RADIO_SIZE)
                .text_size(TEXT_LG),
        ],
    );

    if self.auth_state.use_separate_smtp_credentials {
        col = col.push(
            column![
                text("SMTP Username").size(TEXT_SM).style(text::secondary),
                text_input("", &self.auth_state.smtp_username)
                    .on_input(AddAccountMessage::SmtpUsernameChanged)
                    .size(TEXT_LG).padding(PAD_INPUT)
                    .style(theme::TextInputClass::Settings.style()),
            ].spacing(SPACE_XXXS).width(Length::Fill),
        );
        col = col.push(
            column![
                text("SMTP Password").size(TEXT_SM).style(text::secondary),
                text_input("", &self.auth_state.smtp_password)
                    .on_input(AddAccountMessage::SmtpPasswordChanged)
                    .size(TEXT_LG).padding(PAD_INPUT)
                    .style(theme::TextInputClass::Settings.style()),
            ].spacing(SPACE_XXXS).width(Length::Fill),
        );
    }

    // Accept self-signed certs
    col = col.push(
        row![
            iced::widget::checkbox("Accept self-signed certificates", self.auth_state.accept_invalid_certs)
                .on_toggle(AddAccountMessage::ToggleAcceptInvalidCerts)
                .size(RADIO_SIZE)
                .text_size(TEXT_LG),
        ],
    );

    if let Some(ref err) = self.error {
        col = col.push(
            text(err.as_str()).size(TEXT_SM).style(text::danger),
        );
    }

    col = col.push(Space::new().height(SPACE_SM));

    col = col.push(
        button(
            container(text("Sign In").size(TEXT_LG).color(theme::ON_AVATAR))
                .center_x(Length::Fill),
        )
        .on_press(AddAccountMessage::SubmitCredentials)
        .padding(PAD_BUTTON)
        .style(theme::ButtonClass::Primary.style())
        .width(Length::Fill),
    );

    col = col.push(
        button(text("Back").size(TEXT_LG).style(text::secondary))
            .on_press(AddAccountMessage::Back)
            .padding(PAD_BUTTON)
            .style(theme::ButtonClass::Ghost.style())
            .width(Length::Fill),
    );

    scrollable(col).spacing(SCROLLBAR_SPACING).into()
}
```

### Step 4: Account Identity (name + color picker)

```rust
fn view_identity(&self) -> Element<'_, AddAccountMessage> {
    let mut col = column![].spacing(SPACE_MD).width(Length::Fill);

    col = col.push(
        text(&self.email)
            .size(TEXT_LG)
            .style(text::secondary),
    );

    col = col.push(Space::new().height(SPACE_XS));

    col = col.push(
        column![
            text("Account name").size(TEXT_SM).style(text::secondary),
            text_input("e.g. Work, Personal", &self.identity.name)
                .on_input(AddAccountMessage::AccountNameChanged)
                .size(TEXT_LG)
                .padding(PAD_INPUT)
                .style(theme::TextInputClass::Settings.style())
                .width(Length::Fill),
        ].spacing(SPACE_XXXS),
    );

    col = col.push(Space::new().height(SPACE_SM));
    col = col.push(text("Pick a color").size(TEXT_SM).style(text::secondary));
    col = col.push(color_palette_grid(
        self.identity.selected_color_index,
        &self.used_colors,
    ));

    col = col.push(Space::new().height(SPACE_LG));

    col = col.push(
        button(
            container(text("Done").size(TEXT_LG).color(theme::ON_AVATAR))
                .center_x(Length::Fill),
        )
        .on_press(AddAccountMessage::SubmitIdentity)
        .padding(PAD_BUTTON)
        .style(theme::ButtonClass::Primary.style())
        .width(Length::Fill),
    );

    col.into()
}
```

#### Color palette grid widget

Add to `crates/app/src/ui/widgets.rs`:

```rust
/// A grid of 25 color swatches from the label-colors preset palette.
/// Already-used colors are dimmed with a checkmark overlay.
/// The selected swatch has a prominent ring.
pub fn color_palette_grid<'a, M: Clone + 'a>(
    selected: Option<usize>,
    used_colors: &'a [String],
    on_select: impl Fn(usize) -> M + 'a,
) -> Element<'a, M> {
    let presets = ratatoskr_label_colors::category_colors::all_presets();
    let mut grid = column![].spacing(SPACE_XS);
    let mut current_row = row![].spacing(SPACE_XS);

    for (i, &(_name, bg_hex, _fg_hex)) in presets.iter().enumerate() {
        let is_selected = selected == Some(i);
        let is_used = used_colors.iter().any(|c| c == bg_hex);
        let color = hex_to_color(bg_hex);

        let swatch = Canvas::new(SwatchPainter {
            color,
            selected: is_selected,
            used: is_used,
            size: COLOR_SWATCH_SIZE,
        })
        .width(COLOR_SWATCH_SIZE)
        .height(COLOR_SWATCH_SIZE);

        let swatch_btn = button(swatch)
            .on_press(on_select(i))
            .padding(2)
            .style(if is_selected {
                theme::ButtonClass::ColorSwatchSelected.style()
            } else {
                theme::ButtonClass::BareTransparent.style()
            });

        current_row = current_row.push(swatch_btn);

        if (i + 1) % COLOR_PALETTE_COLUMNS == 0 {
            grid = grid.push(current_row);
            current_row = row![].spacing(SPACE_XS);
        }
    }

    // Push remaining
    if presets.len() % COLOR_PALETTE_COLUMNS != 0 {
        grid = grid.push(current_row);
    }

    grid.into()
}
```

The `SwatchPainter` is a simple `canvas::Program` that draws a filled circle with the color. If `used`, it draws a small check icon or reduces alpha. If `selected`, the outer button style provides a ring.

---

## Phase 4: App-Level Message Wiring

### Message enum additions

In `crates/app/src/main.rs`:

```rust
#[derive(Debug, Clone)]
pub enum Message {
    // ... existing variants ...

    /// Messages from the add-account wizard.
    AddAccount(AddAccountMessage),
    /// Open the add-account modal (from Settings "Add Account" button).
    OpenAddAccount,
    /// Account CRUD operations completed.
    AccountDeleted(Result<(), String>),
    AccountUpdated(Result<(), String>),
}
```

### Handler in App

```rust
impl App {
    fn handle_add_account(&mut self, msg: AddAccountMessage) -> Task<Message> {
        let wizard = match self.add_account_wizard.as_mut() {
            Some(w) => w,
            None => return Task::none(),
        };

        let (task, event) = wizard.update(msg);
        let mut tasks = vec![task.map(Message::AddAccount)];

        if let Some(evt) = event {
            tasks.push(self.handle_add_account_event(evt));
        }
        Task::batch(tasks)
    }

    fn handle_add_account_event(&mut self, event: AddAccountEvent) -> Task<Message> {
        match event {
            AddAccountEvent::AccountAdded(account_id) => {
                self.add_account_wizard = None;
                self.no_accounts = false;
                // Reload accounts list
                let db = Arc::clone(&self.db);
                self.nav_generation += 1;
                let gen = self.nav_generation;
                Task::perform(
                    async move { (gen, load_accounts(db).await) },
                    |(g, result)| Message::AccountsLoaded(g, result),
                )
            }
            AddAccountEvent::Cancelled => {
                // Only allow cancel if there are existing accounts
                if !self.no_accounts {
                    self.add_account_wizard = None;
                }
                Task::none()
            }
        }
    }
}
```

### Modal overlay for subsequent adds

When adding an account while the app has existing accounts, the modal overlays the existing layout:

```rust
fn view_with_modal(&self, wizard: &AddAccountWizard) -> Element<'_, Message> {
    let base_layout = self.view_main_layout(); // existing three-panel layout

    let modal_content = wizard.view().map(Message::AddAccount);

    let modal = container(modal_content)
        .width(Length::Fixed(ACCOUNT_MODAL_WIDTH))
        .max_height(ACCOUNT_MODAL_MAX_HEIGHT)
        .padding(PAD_SETTINGS_CONTENT)
        .style(theme::ContainerClass::Elevated.style());

    let centered_modal = container(modal)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill);

    // Event blocker between base and modal
    let blocker = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::ContainerClass::ModalBackdrop.style()),
    )
    .on_press(Message::Noop) // block clicks
    .interaction(iced::mouse::Interaction::default());

    iced::widget::stack![
        base_layout,
        blocker,
        centered_modal,
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}
```

New container style needed: `ModalBackdrop` -- a semi-transparent dark overlay.

---

## Phase 5: Account Management in Settings

### Settings tab addition

Add `Accounts` to the `Tab` enum in `crates/app/src/ui/settings.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Accounts,    // NEW — first tab
    General,
    Theme,
    Notifications,
    Composing,
    MailRules,
    People,
    Shortcuts,
    Ai,
    About,
}

impl Tab {
    const ALL: &[Tab] = &[
        Tab::Accounts,  // First position
        Tab::General,
        // ... rest unchanged
    ];

    fn label(self) -> &'static str {
        match self {
            Tab::Accounts => "Accounts",
            // ... rest unchanged
        }
    }

    fn icon(self) -> iced::widget::Text<'static> {
        match self {
            Tab::Accounts => icon::users(), // or a new account icon
            // ... rest unchanged
        }
    }
}
```

### Account list state

Add to `Settings`:

```rust
pub struct Settings {
    // ... existing fields ...

    // Accounts tab
    pub managed_accounts: Vec<ManagedAccount>,
    pub editing_account: Option<AccountEditor>,
    pub account_overlay_anim: animation::Animation<bool>,
}

/// An account card in the settings list.
#[derive(Debug, Clone)]
pub struct ManagedAccount {
    pub id: String,
    pub email: String,
    pub provider: String,
    pub account_name: Option<String>,
    pub account_color: Option<String>,
    pub display_name: Option<String>,
    pub last_sync_at: Option<i64>,
    pub health: AccountHealth,
}

/// The slide-in editor state for a single account.
#[derive(Debug, Clone)]
pub struct AccountEditor {
    pub account_id: String,
    pub account_name: String,
    pub display_name: String,
    pub account_color_index: Option<usize>,
    pub caldav_url: String,
    pub caldav_username: String,
    pub caldav_password: String,
    pub show_delete_confirmation: bool,
    pub dirty: bool,
}
```

### Settings messages for accounts

Add to `SettingsMessage`:

```rust
// Accounts tab
AccountCardClicked(String),           // account_id
CloseAccountEditor,
SaveAccountEditor,
AccountNameEditorChanged(String),
DisplayNameEditorChanged(String),
AccountColorEditorChanged(usize),
CaldavUrlChanged(String),
CaldavUsernameChanged(String),
CaldavPasswordChanged(String),
ReauthenticateAccount(String),        // account_id
DeleteAccountRequested(String),       // shows confirmation
DeleteAccountConfirmed(String),       // executes deletion
DeleteAccountCancelled,
AddAccountFromSettings,
AccountDragGripPress(usize),
AccountDragMove(Point),
AccountDragEnd,
```

### Settings events for accounts

Add to `SettingsEvent`:

```rust
pub enum SettingsEvent {
    Closed,
    DateDisplayChanged(DateDisplay),
    // NEW
    OpenAddAccountWizard,
    DeleteAccount(String),
    ReauthenticateAccount(String),
    AccountsReordered(Vec<(String, i64)>),  // (id, new_sort_order)
}
```

### Accounts tab view

```rust
fn accounts_tab(state: &Settings) -> Element<'_, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    // Account cards
    let mut cards: Vec<Element<'_, SettingsMessage>> = Vec::new();
    for account in &state.managed_accounts {
        cards.push(account_card(account));
    }

    // Add Account button at the bottom
    cards.push(
        button(
            container(
                row![
                    icon::plus().size(ICON_MD).style(text::base),
                    text("Add Account").size(TEXT_LG).style(text::base)
                        .font(Font { weight: Weight::Bold, ..font::text() }),
                ]
                .spacing(SPACE_XS)
                .align_y(Alignment::Center),
            )
            .center_x(Length::Fill)
            .align_y(Alignment::Center),
        )
        .on_press(SettingsMessage::AddAccountFromSettings)
        .padding(PAD_SETTINGS_ROW)
        .style(theme::ButtonClass::Action.style())
        .width(Length::Fill)
        .height(SETTINGS_ROW_HEIGHT)
        .into(),
    );

    col = col.push(section("Accounts", cards));

    col.into()
}
```

#### Account card widget

```rust
fn account_card(account: &ManagedAccount) -> Element<'_, SettingsMessage> {
    let color = account.account_color.as_deref()
        .and_then(hex_to_color_opt)
        .unwrap_or(Color::from_rgb(0.5, 0.5, 0.5));

    let name = account.account_name.as_deref()
        .or(account.display_name.as_deref())
        .unwrap_or(&account.email);

    let provider_label = format_provider_label(&account.provider, None);
    let sync_label = format_last_sync(account.last_sync_at);
    let health_dot = health_indicator(account.health);

    let content = row![
        // Color indicator
        widgets::color_dot(color),
        // Main info
        column![
            text(name).size(TEXT_LG).style(text::base),
            text(&account.email).size(TEXT_SM).style(text::secondary),
        ]
        .spacing(SPACE_XXXS)
        .width(Length::Fill),
        // Right side: provider + sync + health
        column![
            text(provider_label).size(TEXT_SM).style(text::secondary),
            row![
                text(sync_label).size(TEXT_XS).style(text::secondary),
                Space::new().width(SPACE_XS),
                health_dot,
            ].align_y(Alignment::Center),
        ]
        .spacing(SPACE_XXXS)
        .align_x(Alignment::End),
        // Chevron
        container(icon::arrow_right().size(ICON_XL).style(text::base))
            .align_y(Alignment::Center),
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Center);

    let id = account.id.clone();
    button(
        container(content)
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill)
            .height(SETTINGS_TOGGLE_ROW_HEIGHT)
            .align_y(Alignment::Center),
    )
    .on_press(SettingsMessage::AccountCardClicked(id))
    .padding(0)
    .style(theme::ButtonClass::Action.style())
    .width(Length::Fill)
    .into()
}
```

### Account editor slide-in

Uses the same overlay pattern as the existing `SettingsOverlay::CreateFilter`. The account editor slides in from the right, covering the accounts tab content.

Add a new overlay variant:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsOverlay {
    CreateFilter,
    AccountEditor,  // NEW
}
```

The editor view:

```rust
fn account_editor_overlay(state: &Settings) -> Element<'_, SettingsMessage> {
    let editor = match &state.editing_account {
        Some(e) => e,
        None => return column![].into(),
    };

    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(
        text("Edit Account")
            .size(TEXT_HEADING)
            .style(text::base)
            .font(Font { weight: Weight::Bold, ..font::text() }),
    );

    // Account name
    col = col.push(section("Account Name", vec![
        input_row("acct-name", "Account Name", "e.g. Work", &editor.account_name,
            SettingsMessage::AccountNameEditorChanged),
    ]));

    // Display name
    col = col.push(section("Display Name", vec![
        input_row("display-name", "Display Name", "Your Name", &editor.display_name,
            SettingsMessage::DisplayNameEditorChanged),
    ]));

    // Account color
    let used_colors: Vec<String> = state.managed_accounts.iter()
        .filter(|a| a.id != editor.account_id)
        .filter_map(|a| a.account_color.clone())
        .collect();

    col = col.push(section("Account Color", vec![
        container(
            color_palette_grid(
                editor.account_color_index,
                &used_colors,
                SettingsMessage::AccountColorEditorChanged,
            ),
        )
        .padding(PAD_SETTINGS_ROW)
        .width(Length::Fill)
        .into(),
    ]));

    // Re-authenticate
    col = col.push(section("Authentication", vec![
        action_row("Re-authenticate", Some("Sign in again to refresh credentials"),
            None, ActionKind::InApp,
            SettingsMessage::ReauthenticateAccount(editor.account_id.clone())),
    ]));

    // CalDAV settings (only for IMAP/JMAP accounts)
    col = col.push(section("Calendar (CalDAV)", vec![
        input_row("caldav-url", "CalDAV URL", "https://", &editor.caldav_url,
            SettingsMessage::CaldavUrlChanged),
        input_row("caldav-user", "Username", "", &editor.caldav_username,
            SettingsMessage::CaldavUsernameChanged),
        input_row("caldav-pass", "Password", "", &editor.caldav_password,
            SettingsMessage::CaldavPasswordChanged),
    ]));

    // Delete
    if editor.show_delete_confirmation {
        col = col.push(section("Danger Zone", vec![
            container(
                column![
                    text("Are you sure you want to delete this account?")
                        .size(TEXT_LG).style(text::danger),
                    text("All data for this account will be permanently removed.")
                        .size(TEXT_SM).style(text::secondary),
                    Space::new().height(SPACE_SM),
                    row![
                        button(text("Delete Account").size(TEXT_LG).style(text::danger))
                            .on_press(SettingsMessage::DeleteAccountConfirmed(editor.account_id.clone()))
                            .padding(PAD_BUTTON)
                            .style(theme::ButtonClass::ExperimentSemantic { variant: 2 }.style()),
                        Space::new().width(SPACE_SM),
                        button(text("Cancel").size(TEXT_LG).style(text::secondary))
                            .on_press(SettingsMessage::DeleteAccountCancelled)
                            .padding(PAD_BUTTON)
                            .style(theme::ButtonClass::Ghost.style()),
                    ],
                ]
                .spacing(SPACE_XS),
            )
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill)
            .into(),
        ]));
    } else {
        col = col.push(section("Danger Zone", vec![
            action_row("Delete Account", Some("Remove this account and all its data"),
                None, ActionKind::InApp,
                SettingsMessage::DeleteAccountRequested(editor.account_id.clone())),
        ]));
    }

    col.into()
}
```

### Deletion edge cases

When an account is deleted, the `App` handles:

```rust
fn handle_account_deleted(&mut self, deleted_id: &str) -> Task<Message> {
    // 1. If this was the selected account, revert to All Accounts
    if let Some(idx) = self.sidebar.selected_account {
        if self.sidebar.accounts.get(idx).is_some_and(|a| a.id == deleted_id) {
            self.sidebar.selected_account = None;
        }
    }

    // 2. Reload accounts
    self.nav_generation += 1;
    let db = Arc::clone(&self.db);
    let gen = self.nav_generation;

    Task::perform(
        async move { (gen, load_accounts(db).await) },
        |(g, result)| Message::AccountsLoaded(g, result),
    )
    // AccountsLoaded handler will detect zero accounts and set no_accounts = true,
    // which triggers the first-launch modal.
}
```

The deletion cascade (labels, threads, messages, attachments, cached files) happens in `db_delete_account` in the core crate. The UI just needs to handle the state transitions.

For compose windows and pop-out windows: these are not yet implemented. When they are, the compose window must check account validity before sending, and pop-outs for deleted accounts must close. This is documented here as a future requirement.

---

## Phase 6: Account Selector Enhancements

### Current state

The sidebar dropdown (`crates/app/src/ui/sidebar.rs` `scope_dropdown()`) already shows All Accounts + individual accounts with avatars. What remains:

1. **Color indicators**: Replace avatar circles with account color dots in the dropdown.
2. **Account names**: Show `account_name` instead of email when available.
3. **Sort order**: Respect `sort_order` when listing accounts.

### Changes to sidebar dropdown

```rust
fn scope_dropdown(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let (trigger_icon, trigger_label): (DropdownIcon<'_>, &str) =
        match sidebar.selected_account {
            Some(idx) if sidebar.accounts.get(idx).is_some() => {
                let acc = &sidebar.accounts[idx];
                let name = acc.account_name.as_deref()
                    .or(acc.display_name.as_deref())
                    .unwrap_or(&acc.email);
                // Use account color dot instead of avatar
                (DropdownIcon::Avatar(name), name) // TODO: DropdownIcon::ColorDot
            }
            _ => (DropdownIcon::Icon(icon::INBOX_CODEPOINT), "All Accounts"),
        };

    // ... entries built as before, but using account_name where available ...
}
```

A new `DropdownIcon` variant may be needed:

```rust
pub enum DropdownIcon<'a> {
    Avatar(&'a str),
    Icon(char),
    ColorDot(Color),  // NEW
}
```

### Sort order

Accounts are already queried with `ORDER BY created_at ASC`. Change to `ORDER BY sort_order ASC, created_at ASC` once the `sort_order` column exists. Reordering in settings updates `sort_order` via `db_update_account_sort_order`.

---

## Phase 7: Error States & Recovery

### Token expiry

When the status bar (see `docs/status-bar/problem-statement.md`) detects a token expiry:

1. Status bar shows: "alice@corp.com needs re-authentication -- click to sign in"
2. Clicking triggers `Message::ReauthenticateAccount(account_id)`
3. App opens the OAuth flow (same as Add Account step 3a, but for an existing account)
4. On success, tokens are updated in DB, sync resumes

This requires a new `ReauthWizard` state in App (simpler than `AddAccountWizard` -- just OAuth waiting + success):

```rust
struct App {
    // ... existing fields ...
    reauth_wizard: Option<ReauthWizard>,
}

struct ReauthWizard {
    account_id: String,
    account_email: String,
    generation: u64,
    error: Option<String>,
}
```

### Connection failure

Handled by the sync pipeline (not the accounts UI). The status bar surfaces persistent failures. The accounts tab shows the health indicator. No special UI flow needed -- the user can see the error in the account card's health dot and optionally re-authenticate.

### Duplicate account detection

In the Add Account wizard, after email is submitted (step 1), before discovery runs:

```rust
// In AddAccountWizard::update, SubmitEmail arm:
// Check for existing account with same email
let exists = db_account_exists_by_email(&db, email.clone()).await?;
if exists {
    return Err("This account is already configured.".to_string());
}
```

This check runs as part of the discovery task. If the email already exists, the wizard shows the error and stays on the email input step.

---

## Theme Additions

New style classes needed in `crates/app/src/ui/theme.rs`:

```rust
pub enum ButtonClass {
    // ... existing ...
    ProtocolCard,           // Normal protocol selection card
    ProtocolCardSelected,   // Selected protocol card (primary border)
    ColorSwatchSelected,    // Color swatch with selection ring
}

pub enum ContainerClass {
    // ... existing ...
    ModalBackdrop,          // Semi-transparent dark overlay behind modals
}
```

**ProtocolCard**: `base` background, subtle border, hover brightens. **ProtocolCardSelected**: primary-colored border, subtle primary background tint. **ColorSwatchSelected**: 2px primary ring around the swatch. **ModalBackdrop**: `Color { r: 0, g: 0, b: 0, a: 0.5 }` background.

---

## Dependency Graph & Phasing

```
Phase 0: Data Model Extensions
  │  DB migration (account_color, account_name, sort_order columns)
  │  Core CRUD functions (create, update, delete, exists-by-email)
  │
Phase 1: First Launch Detection
  │  no_accounts flag, view routing, empty window with modal
  │  Depends on: Phase 0
  │
Phase 2: Add Account Wizard State Machine
  │  AddAccountWizard struct, AddAccountStep enum, all state types
  │  Depends on: Phase 0
  │
Phase 3: Wizard Views
  │  Email input, discovering spinner, protocol selection, OAuth waiting,
  │  password form, identity/color picker, creating spinner
  │  Depends on: Phase 2
  │  Subtasks (can be parallelized):
  │    3a: Email input + discovery views
  │    3b: Protocol selection cards + manual config
  │    3c: OAuth waiting view
  │    3d: Password auth form
  │    3e: Identity + color palette grid
  │
Phase 4: App-Level Wiring
  │  Message::AddAccount, modal overlay, event handling,
  │  account reload after add
  │  Depends on: Phase 1, Phase 3
  │
Phase 5: Account Management in Settings
  │  Accounts tab, account cards, slide-in editor,
  │  edit/delete/reorder, color picker reuse
  │  Depends on: Phase 0, Phase 4 (shared types)
  │  Subtasks:
  │    5a: Account list with cards
  │    5b: Slide-in editor (name, color, CalDAV)
  │    5c: Delete with confirmation + edge cases
  │    5d: Drag reordering
  │
Phase 6: Sidebar Enhancements
  │  Color dots in dropdown, account names, sort order
  │  Depends on: Phase 0
  │  Can be done in parallel with Phase 5
  │
Phase 7: Error States & Recovery
  │  Re-auth flow, health indicators, duplicate detection
  │  Depends on: Phase 4, Phase 5
  │  Pairs with status bar implementation
```

### Estimated complexity

| Phase | Files touched | New files | Complexity |
|-------|--------------|-----------|------------|
| 0 | `crates/db/src/db/types.rs`, `crates/core/src/db/queries_extra/`, `crates/app/src/db.rs` | `accounts_crud.rs` | Low |
| 1 | `crates/app/src/main.rs` | None | Low |
| 2 | None | `crates/app/src/ui/add_account.rs` | Medium |
| 3 | `crates/app/src/ui/add_account.rs`, `crates/app/src/ui/widgets.rs`, `crates/app/src/ui/layout.rs` | None | High |
| 4 | `crates/app/src/main.rs` | None | Medium |
| 5 | `crates/app/src/ui/settings.rs`, `crates/app/src/ui/theme.rs` | None | High |
| 6 | `crates/app/src/ui/sidebar.rs`, `crates/app/src/ui/widgets.rs` | None | Low |
| 7 | `crates/app/src/main.rs`, `crates/app/src/ui/add_account.rs` | None | Medium |

### Critical path

Phase 0 -> Phase 1 -> Phase 2 -> Phase 3 -> Phase 4 is the critical path. Everything else can be parallelized once Phase 4 lands. The first-launch experience (Phases 0-4) is the priority -- without it, the app requires a seeded database and has no onboarding story.

---

## Open Items

1. **Spinner widget**: iced does not have a built-in spinner/throbber. Options: use a canvas-based rotating arc (custom `Program`), use a text-based "..." animation with a subscription timer, or find a community crate. Decision deferred to implementation.

2. **`open` crate for browser launch**: The OAuth flow calls `open::that(url)` to open the system browser. Need to add the `open` crate as a dependency of the app crate. Already used implicitly via `ratatoskr-core`'s OAuth module, but the app needs to provide the `open_url` callback.

3. **DB write access**: The app's current `Db` opens the database with `PRAGMA query_only = ON`. Account creation and updates require write access. Either remove `query_only` or add a separate write connection. The core crate's `DbState` does not have this restriction, so the app should use `DbState` from `ratatoskr-core` instead of its own `Db` wrapper for write operations.

4. **Account reordering drag UX**: The existing drag-to-reorder in the Mail Rules tab (`editable_list()`) uses `mouse_area` `on_move` for position tracking. The same pattern applies to account cards. Reuse the `DragState` and `handle_drag_move` infrastructure.

5. **Credential validation**: Step 3b (password auth) should validate credentials before proceeding to step 4. This means attempting an IMAP connection. The core crate's IMAP provider has connection logic, but a lightweight "test connection" function may need to be exposed. If connection fails, show the error and stay on the password form.
