use std::fmt;
use std::sync::Arc;

use bifrost_caldav::{CalDavAccountFactory, CalDavConfig, CalDavCredentials};
use bifrost_carddav::{CardDavConfig, CardDavCredentials};
use bifrost_graph::account::{GraphAccountFactory, GraphClient, PublicFolderScope};
use bifrost_imap::{
    AuthPolicy, Credentials, ImapAccountConfig, ImapAccountFactory, ImapConfig,
    SmtpSubmissionConfig, SubmissionCredentials, SubmissionTls,
};
use bifrost_jmap::sync::{JmapAccountFactory, JmapCredentials};
use bifrost_net::{OAuthRefresher, StaticTokenSource, TokenSource};
use bifrost_types::FolderId;
use bifrost_types::{AccountFactory, ProtocolKind};
use common::crypto::StoredSecret;
use db::db::{ReadConn, ReadDbState, WriterPool, params};
use service_api::VerifyAccountParams;
use service_api::actions::RemoteFailureKind;
use types::MailProviderKind;

use super::token_source::DbWriteBackTokenSource;

fn gmail_api_base_from_test_endpoint(endpoint: &str) -> Option<String> {
    common::test_endpoint::api_base_from_test_endpoint(endpoint, "gmail/v1/users/me")
}

#[derive(Debug, Clone)]
pub enum BifrostBuildError {
    UnknownProvider(String),
    MissingCredential {
        account_id: String,
        field: &'static str,
    },
    MissingEndpoint {
        account_id: String,
        provider: MailProviderKind,
    },
    Decrypt {
        account_id: String,
        field: &'static str,
        error: String,
    },
    Db(String),
    InvalidConfig {
        account_id: String,
        detail: String,
    },
}

impl BifrostBuildError {
    pub fn classify(&self) -> RemoteFailureKind {
        match self {
            Self::UnknownProvider(_)
            | Self::MissingCredential { .. }
            | Self::MissingEndpoint { .. }
            | Self::Decrypt { .. }
            | Self::InvalidConfig { .. } => RemoteFailureKind::Permanent,
            Self::Db(_) => RemoteFailureKind::Permanent,
        }
    }
}

impl fmt::Display for BifrostBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownProvider(provider) => write!(f, "unknown provider: {provider}"),
            Self::MissingCredential { account_id, field } => {
                write!(f, "missing credential {field} for account {account_id}")
            }
            Self::MissingEndpoint {
                account_id,
                provider,
            } => write!(
                f,
                "missing OAuth token endpoint for {provider:?} account {account_id}",
            ),
            Self::Decrypt {
                account_id,
                field,
                error,
            } => write!(
                f,
                "failed to decrypt credential {field} for account {account_id}: {error}",
            ),
            Self::Db(error) => write!(f, "database error: {error}"),
            Self::InvalidConfig { account_id, detail } => {
                write!(
                    f,
                    "invalid bifrost account config for {account_id}: {detail}"
                )
            }
        }
    }
}

impl std::error::Error for BifrostBuildError {}

pub async fn build_account_factory(
    db: &ReadDbState,
    writer: WriterPool,
    account_id: &str,
    encryption_key: [u8; 32],
) -> Result<Arc<dyn AccountFactory>, BifrostBuildError> {
    let account_id_for_read = account_id.to_string();
    let row = db
        .with_read(move |conn| read_bifrost_account_credentials(conn, &account_id_for_read))
        .await
        .map_err(BifrostBuildError::Db)??;
    let provider = MailProviderKind::parse(&row.provider)
        .map_err(|_| BifrostBuildError::UnknownProvider(row.provider.clone()))?;
    let decrypted = row.decrypt(encryption_key)?;
    factory_from_decrypted(&decrypted, provider, &TokenMode::WriteBack(writer))
}

/// Build an in-flight verify factory for an EXISTING account row (re-auth)
/// using freshly-exchanged plaintext tokens that have NOT been persisted.
///
/// Re-auth verify-before-persist (option b): `oauth.exchange_code` proves the
/// new tokens can open the mailbox BEFORE writing them to the row. Provider +
/// transport config comes from the persisted row, but the access token is
/// overridden with the freshly-exchanged one so verification exercises the NEW
/// credential rather than the stale persisted value. `TokenMode::Static`
/// (no refresh / no write-back) is used so the unpersisted token cannot trigger
/// a rotation DB write. The tokens are verified Service-side and never cross
/// the IPC - this reuses the same `factory_from_decrypted` + `open`/`close`
/// path as `handle_verify_account`.
pub(crate) async fn build_reauth_verify_factory(
    db: &ReadDbState,
    account_id: &str,
    encryption_key: [u8; 32],
    new_access_token: String,
) -> Result<Arc<dyn AccountFactory>, BifrostBuildError> {
    let account_id_for_read = account_id.to_string();
    let row = db
        .with_read(move |conn| read_bifrost_account_credentials(conn, &account_id_for_read))
        .await
        .map_err(BifrostBuildError::Db)??;
    let provider = MailProviderKind::parse(&row.provider)
        .map_err(|_| BifrostBuildError::UnknownProvider(row.provider.clone()))?;
    let mut decrypted = row.decrypt(encryption_key)?;
    // Verify the freshly-exchanged token, not the persisted one. The
    // test-endpoint factory arms read `decrypted.access_token` directly, while
    // the production arms read the Static token_mode below; overriding the
    // field keeps both paths on the new credential.
    decrypted.access_token = Some(new_access_token.clone());
    factory_from_decrypted(&decrypted, provider, &TokenMode::Static(new_access_token))
}

#[derive(Clone)]
pub(crate) enum TokenMode {
    WriteBack(WriterPool),
    Static(String),
}

pub(crate) fn factory_from_decrypted(
    decrypted: &DecryptedAccountCredentials,
    provider: MailProviderKind,
    token_mode: &TokenMode,
) -> Result<Arc<dyn AccountFactory>, BifrostBuildError> {
    match provider {
        MailProviderKind::Gmail => {
            let test_api_base = std::env::var("RATATOSKR_TEST_GMAIL_ENDPOINT")
                .ok()
                .and_then(|endpoint| gmail_api_base_from_test_endpoint(&endpoint));
            let mut factory = if let Some(api_base) = test_api_base {
                let access_token =
                    decrypted.required_plain("access_token", decrypted.access_token.as_deref())?;
                bifrost_google::account::GoogleAccountFactory::from_access_token_with_api_base(
                    access_token,
                    api_base,
                )
            } else if std::env::var("RATATOSKR_TEST_GCAL_ENDPOINT").is_ok() {
                let access_token =
                    decrypted.required_plain("access_token", decrypted.access_token.as_deref())?;
                bifrost_google::account::GoogleAccountFactory::from_access_token_with_api_base(
                    access_token,
                    "https://www.googleapis.com/gmail/v1/users/me",
                )
            } else {
                bifrost_google::account::GoogleAccountFactory::from_token_source(
                    decrypted.token_source(provider, token_mode)?,
                )
            };
            // Google's People (contacts + directory) API lives on a separate
            // host from Gmail mail, so it carries its own base override. The
            // harness redirects it to the saehrimnir People mock; bifrost's
            // default base ends in `/v1`, which the mock routes mirror.
            if let Ok(people_endpoint) = std::env::var("RATATOSKR_TEST_PEOPLE_ENDPOINT") {
                factory = factory
                    .with_people_api_base(format!("{}/v1", people_endpoint.trim_end_matches('/')));
            }
            if let Ok(topic) = std::env::var("RATATOSKR_GMAIL_PUBSUB_TOPIC") {
                factory =
                    factory.with_pubsub_config(bifrost_google::account::PubSubConfig::new(topic));
            }
            Ok(Arc::new(factory))
        }
        MailProviderKind::Graph => {
            let graph_base = std::env::var("RATATOSKR_TEST_GRAPH_ENDPOINT")
                .ok()
                .map(|base| format!("{}/v1.0", base.trim_end_matches('/')))
                .unwrap_or_else(|| "https://graph.microsoft.com/v1.0".to_string());
            // GraphClient takes a single v1.0 api base now and derives the
            // sibling bases from it internally, so the separately-constructed
            // beta base this used to thread through is gone rather than moved.
            let client = if std::env::var("RATATOSKR_TEST_GRAPH_ENDPOINT").is_ok() {
                let access_token =
                    decrypted.required_plain("access_token", decrypted.access_token.as_deref())?;
                GraphClient::with_api_base(graph_base, access_token)
            } else {
                let source = decrypted.token_source(provider, token_mode)?;
                GraphClient::with_source(graph_base, source)
            };
            let mut factory = GraphAccountFactory::new(client);
            // Graph webhook mode now requires the `clientState` secret up
            // front: bifrost dropped the URL-only constructor because it minted
            // a random per-resource secret and discarded it, leaving the
            // receiver nothing to validate against. A URL with no secret is
            // therefore a misconfiguration, not a degraded mode - it falls back
            // to the default (EWS streaming) push path rather than subscribing
            // with a secret nobody can check.
            if let Ok(webhook_url) = std::env::var("RATATOSKR_GRAPH_PUSH_NOTIFICATION_URL") {
                match std::env::var("RATATOSKR_GRAPH_PUSH_CLIENT_STATE") {
                    Ok(client_state) if !client_state.is_empty() => {
                        factory =
                            factory.with_push_endpoint_client_state(webhook_url, client_state);
                    }
                    _ => log::warn!(
                        "RATATOSKR_GRAPH_PUSH_NOTIFICATION_URL is set without a non-empty \
                         RATATOSKR_GRAPH_PUSH_CLIENT_STATE; Graph webhook subscriptions are \
                         disabled because the receiver could not validate them"
                    ),
                }
            }
            for mailbox in &decrypted.row.enabled_shared_mailboxes {
                factory = factory.with_shared_mailbox(mailbox.clone());
            }
            if decrypted.row.delegate_discovery_enabled {
                factory = factory.with_delegate_discovery();
            }
            if decrypted.row.public_folders_enabled {
                let pins = decrypted
                    .row
                    .enabled_public_folder_pins
                    .iter()
                    .cloned()
                    .map(FolderId);
                let scope = if decrypted.row.enabled_public_folder_pins.is_empty() {
                    PublicFolderScope::hierarchy_only()
                } else {
                    PublicFolderScope::pinned(pins)
                };
                factory = factory.with_public_folders(scope);
            }
            Ok(Arc::new(factory))
        }
        MailProviderKind::Jmap => build_jmap_factory(decrypted, provider, token_mode),
        MailProviderKind::Imap => build_imap_factory(decrypted, provider, token_mode),
    }
}

/// The calendar-provider precedence rule, resolved once. Calendar identity is
/// intentionally independent from mail identity (a Gmail/Graph mailbox can carry
/// a separate CalDAV calendar), so `calendar_provider = "caldav"` - or a
/// `provider = "caldav"` account with a non-empty `caldav_url` - routes to
/// CalDAV; otherwise the mail provider drives calendar identity. Returns `None`
/// for IMAP-only / unrecognised accounts (no calendar backend). This is the
/// single source of truth for the rule: both `build_calendar_account_factory`
/// (which needs the CalDAV/mail split) and the calendar action opener (which
/// needs the `ProtocolKind` for id translation) resolve it here rather than each
/// re-deriving the precedence and drifting.
pub(crate) fn calendar_protocol_kind(
    provider: &str,
    calendar_provider: Option<&str>,
    caldav_url: Option<&str>,
) -> Option<ProtocolKind> {
    let is_caldav = calendar_provider == Some("caldav")
        || (provider == "caldav" && caldav_url.is_some_and(|url| !url.trim().is_empty()));
    if is_caldav {
        return Some(ProtocolKind::CalDav);
    }
    match MailProviderKind::parse(provider) {
        Ok(MailProviderKind::Gmail) => Some(ProtocolKind::Gmail),
        Ok(MailProviderKind::Graph) => Some(ProtocolKind::Graph),
        Ok(MailProviderKind::Jmap) => Some(ProtocolKind::Jmap),
        Ok(MailProviderKind::Imap) | Err(_) => None,
    }
}

/// Build the calendar-facing factory. Calendar identity is intentionally
/// independent from mail identity: a Gmail/Graph mailbox can have a separate
/// CalDAV calendar configured on the same account row.
pub async fn build_calendar_account_factory(
    db: &ReadDbState,
    writer: WriterPool,
    account_id: &str,
    encryption_key: [u8; 32],
) -> Result<Option<Arc<dyn AccountFactory>>, BifrostBuildError> {
    let account_id_for_read = account_id.to_string();
    let row = db
        .with_read(move |conn| read_bifrost_account_credentials(conn, &account_id_for_read))
        .await
        .map_err(BifrostBuildError::Db)??;
    let calendar_is_caldav = matches!(
        calendar_protocol_kind(
            &row.provider,
            row.calendar_provider.as_deref(),
            row.caldav_url.as_deref(),
        ),
        Some(ProtocolKind::CalDav)
    );
    if calendar_is_caldav {
        let decrypted = row.decrypt(encryption_key)?;
        let url = decrypted.required_plain("caldav_url", decrypted.row.caldav_url.as_deref())?;
        let username =
            decrypted.required_plain("caldav_username", decrypted.caldav_username.as_deref())?;
        let password =
            decrypted.required_plain("caldav_password", decrypted.caldav_password.as_deref())?;
        return Ok(Some(Arc::new(CalDavAccountFactory::new(
            CalDavConfig::new(
                std::env::var("RATATOSKR_TEST_CALDAV_ENDPOINT").unwrap_or(url),
                CalDavCredentials::Basic { username, password },
            ),
        ))));
    }
    match MailProviderKind::parse(&row.provider) {
        Ok(MailProviderKind::Gmail | MailProviderKind::Graph | MailProviderKind::Jmap) => {
            build_account_factory(db, writer, account_id, encryption_key)
                .await
                .map(Some)
        }
        Ok(MailProviderKind::Imap) | Err(_) => Ok(None),
    }
}

fn build_jmap_factory(
    account: &DecryptedAccountCredentials,
    provider: MailProviderKind,
    token_mode: &TokenMode,
) -> Result<Arc<dyn AccountFactory>, BifrostBuildError> {
    let url = match std::env::var("RATATOSKR_TEST_JMAP_ENDPOINT") {
        Ok(url) => url,
        Err(_) => account.required_plain("jmap_url", account.row.jmap_url.as_deref())?,
    };
    let credentials = if account.is_oauth() {
        JmapCredentials::Bearer {
            token_source: account.token_source(provider, token_mode)?,
        }
    } else {
        JmapCredentials::Basic {
            username: account.username(),
            password: account.required_secret("imap_password", account.imap_password.as_deref())?,
        }
    };
    Ok(Arc::new(
        JmapAccountFactory::builder(url, credentials)
            .accept_invalid_certs(account.row.accept_invalid_certs)
            .build(),
    ))
}

fn build_imap_factory(
    account: &DecryptedAccountCredentials,
    provider: MailProviderKind,
    token_mode: &TokenMode,
) -> Result<Arc<dyn AccountFactory>, BifrostBuildError> {
    let (imap_host, imap_port, imap_security, allow_cleartext_auth) =
        if let Ok(endpoint) = std::env::var("RATATOSKR_TEST_IMAP_ENDPOINT") {
            let (host, port) =
                parse_host_port(&endpoint).ok_or_else(|| BifrostBuildError::InvalidConfig {
                    account_id: account.row.id.clone(),
                    detail: format!("invalid RATATOSKR_TEST_IMAP_ENDPOINT {endpoint}"),
                })?;
            (host, Some(port), "none".to_string(), true)
        } else {
            (
                account.required_plain("imap_host", account.row.imap_host.as_deref())?,
                account.optional_port(account.row.imap_port, "imap_port")?,
                account
                    .row
                    .imap_security
                    .as_deref()
                    .unwrap_or("tls")
                    .to_string(),
                false,
            )
        };
    let imap = match imap_security.as_str() {
        "tls" | "ssl" => ImapConfig::tls(imap_host),
        "starttls" => ImapConfig::starttls(imap_host),
        "none" => ImapConfig::plaintext(imap_host),
        other => {
            return Err(BifrostBuildError::InvalidConfig {
                account_id: account.row.id.clone(),
                detail: format!("unknown IMAP security mode {other}"),
            });
        }
    };
    let imap = if let Some(port) = imap_port {
        imap.with_port(port)
    } else {
        imap
    };
    // Build the OAuth refresher ONCE and share the same `Arc<dyn TokenSource>`
    // between IMAP auth and SMTP submission. Both flows refresh the same
    // account's access token, so a per-call refresher would give them
    // independent single-flight state and let them rotate/write-back the row
    // against each other. Sharing one refresher is the single generic
    // rotation path (spec 3.1/4) realized across both transports.
    let shared_source = if account.is_oauth() {
        Some(account.token_source(provider, token_mode)?)
    } else {
        None
    };
    let credentials = if let Some(source) = shared_source.clone() {
        Credentials::oauth2_source(account.username(), source)
    } else {
        Credentials::password(
            account.username(),
            account.required_secret("imap_password", account.imap_password.as_deref())?,
        )
    };
    let auth_policy = if allow_cleartext_auth {
        AuthPolicy::default()
            .with_login()
            .allow_cleartext_without_tls()
    } else {
        AuthPolicy::default().with_login()
    };
    let mut config = ImapAccountConfig::new(imap, credentials, auth_policy);
    // CardDAV composes beneath the IMAP-shaped account in bifrost.  The
    // existing CalDAV settings are the DAV discovery/configuration surface:
    // an IMAP account only gains contacts with an endpoint and usable DAV
    // credentials (the shared OAuth source or DAV basic credentials). This
    // deliberately leaves ordinary IMAP accounts Unsupported for contacts.
    if let Some(url) = account.row.caldav_url.as_deref() {
        let endpoint = std::env::var("RATATOSKR_TEST_CARDDAV_ENDPOINT")
            .or_else(|_| std::env::var("RATATOSKR_TEST_CALDAV_ENDPOINT"))
            .unwrap_or_else(|_| url.to_string());
        let carddav_credentials = match shared_source.clone() {
            Some(source) => Some(CardDavCredentials::bearer_source(source)),
            None => match (
                account.caldav_username.as_deref(),
                account.caldav_password.as_deref(),
            ) {
                (Some(username), Some(password)) => Some(CardDavCredentials::Basic {
                    username: username.to_string(),
                    password: password.to_string(),
                }),
                _ => None,
            },
        };
        if let Some(credentials) = carddav_credentials {
            config = config.with_carddav(CardDavConfig::new(endpoint, credentials));
        }
    }
    if let Some(submission) = account.smtp_submission(shared_source)? {
        config = config.with_submission(submission);
    }
    Ok(Arc::new(ImapAccountFactory::new(config)))
}

fn parse_host_port(endpoint: &str) -> Option<(String, u16)> {
    let (host, port) = endpoint.rsplit_once(':')?;
    if host.is_empty() {
        return None;
    }
    Some((host.to_string(), port.parse().ok()?))
}

#[derive(Clone)]
struct AccountCredentialsRow {
    id: String,
    email: String,
    provider: String,
    auth_method: String,
    access_token: Option<String>,
    refresh_token: Option<String>,
    token_expires_at: Option<i64>,
    oauth_provider: Option<String>,
    oauth_client_id: Option<String>,
    oauth_client_secret: Option<String>,
    oauth_token_url: Option<String>,
    imap_host: Option<String>,
    imap_port: Option<i64>,
    imap_security: Option<String>,
    imap_username: Option<String>,
    imap_password: Option<String>,
    smtp_host: Option<String>,
    smtp_port: Option<i64>,
    smtp_security: Option<String>,
    smtp_username: Option<String>,
    smtp_password: Option<String>,
    jmap_url: Option<String>,
    calendar_provider: Option<String>,
    caldav_url: Option<String>,
    caldav_username: Option<String>,
    caldav_password: Option<String>,
    accept_invalid_certs: bool,
    delegate_discovery_enabled: bool,
    public_folders_enabled: bool,
    enabled_shared_mailboxes: Vec<String>,
    enabled_public_folder_pins: Vec<String>,
}

impl AccountCredentialsRow {
    fn decrypt(
        self,
        encryption_key: [u8; 32],
    ) -> Result<DecryptedAccountCredentials, BifrostBuildError> {
        let account_id = self.id.clone();
        Ok(DecryptedAccountCredentials {
            access_token: decrypt_optional(
                &account_id,
                "access_token",
                self.access_token.as_ref(),
                &encryption_key,
            )?,
            refresh_token: decrypt_optional(
                &account_id,
                "refresh_token",
                self.refresh_token.as_ref(),
                &encryption_key,
            )?,
            oauth_client_id: decrypt_optional(
                &account_id,
                "oauth_client_id",
                self.oauth_client_id.as_ref(),
                &encryption_key,
            )?,
            oauth_client_secret: decrypt_optional(
                &account_id,
                "oauth_client_secret",
                self.oauth_client_secret.as_ref(),
                &encryption_key,
            )?,
            imap_password: decrypt_optional(
                &account_id,
                "imap_password",
                self.imap_password.as_ref(),
                &encryption_key,
            )?,
            smtp_password: decrypt_optional(
                &account_id,
                "smtp_password",
                self.smtp_password.as_ref(),
                &encryption_key,
            )?,
            caldav_username: decrypt_optional(
                &account_id,
                "caldav_username",
                self.caldav_username.as_ref(),
                &encryption_key,
            )?,
            caldav_password: decrypt_optional(
                &account_id,
                "caldav_password",
                self.caldav_password.as_ref(),
                &encryption_key,
            )?,
            encryption_key,
            row: self,
        })
    }
}

pub(crate) struct DecryptedAccountCredentials {
    row: AccountCredentialsRow,
    access_token: Option<String>,
    refresh_token: Option<String>,
    oauth_client_id: Option<String>,
    oauth_client_secret: Option<String>,
    imap_password: Option<String>,
    smtp_password: Option<String>,
    caldav_username: Option<String>,
    caldav_password: Option<String>,
    encryption_key: [u8; 32],
}

impl DecryptedAccountCredentials {
    pub(crate) fn row_provider(&self) -> String {
        self.row.provider.clone()
    }
    pub(crate) fn from_verify_params(params: VerifyAccountParams) -> Self {
        let auth_method = if params.access_token.is_some() {
            "oauth2".to_string()
        } else {
            "password".to_string()
        };
        Self {
            row: AccountCredentialsRow {
                id: format!("verify-{}", uuid::Uuid::new_v4()),
                email: params.email,
                provider: params.provider,
                auth_method,
                access_token: None,
                refresh_token: None,
                token_expires_at: None,
                oauth_provider: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                oauth_token_url: None,
                imap_host: params.imap_host,
                imap_port: params.imap_port.map(i64::from),
                imap_security: params.imap_security,
                imap_username: params.username,
                imap_password: None,
                smtp_host: None,
                smtp_port: None,
                smtp_security: None,
                smtp_username: None,
                smtp_password: None,
                jmap_url: params.jmap_url,
                calendar_provider: None,
                caldav_url: None,
                caldav_username: None,
                caldav_password: None,
                accept_invalid_certs: params.accept_invalid_certs,
                delegate_discovery_enabled: false,
                public_folders_enabled: false,
                enabled_shared_mailboxes: Vec::new(),
                enabled_public_folder_pins: Vec::new(),
            },
            access_token: params
                .access_token
                .map(service_api::RedactedString::into_inner),
            refresh_token: None,
            oauth_client_id: None,
            oauth_client_secret: None,
            imap_password: params
                .imap_password
                .map(service_api::RedactedString::into_inner),
            smtp_password: None,
            caldav_username: None,
            caldav_password: None,
            // Verify never decrypts or refreshes. Static TokenMode never
            // reaches this key, so it is intentionally unused zero data.
            encryption_key: [0; 32],
        }
    }
    fn is_oauth(&self) -> bool {
        matches!(self.row.auth_method.as_str(), "oauth2" | "oauth" | "bearer")
    }

    fn username(&self) -> String {
        self.row
            .imap_username
            .clone()
            .unwrap_or_else(|| self.row.email.clone())
    }

    fn required_plain(
        &self,
        field: &'static str,
        value: Option<&str>,
    ) -> Result<String, BifrostBuildError> {
        value
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .ok_or_else(|| BifrostBuildError::MissingCredential {
                account_id: self.row.id.clone(),
                field,
            })
    }

    fn required_secret(
        &self,
        field: &'static str,
        value: Option<&str>,
    ) -> Result<String, BifrostBuildError> {
        self.required_plain(field, value)
    }

    fn optional_port(
        &self,
        value: Option<i64>,
        field: &'static str,
    ) -> Result<Option<u16>, BifrostBuildError> {
        value
            .map(|port| {
                u16::try_from(port).map_err(|_| BifrostBuildError::InvalidConfig {
                    account_id: self.row.id.clone(),
                    detail: format!("{field} out of range: {port}"),
                })
            })
            .transpose()
    }

    fn token_source(
        &self,
        provider: MailProviderKind,
        mode: &TokenMode,
    ) -> Result<Arc<dyn TokenSource>, BifrostBuildError> {
        if let TokenMode::Static(token) = mode {
            let source: Arc<dyn TokenSource> =
                Arc::new(StaticTokenSource::new(token.clone(), None));
            return Ok(source);
        }
        let access_token = self.required_plain("access_token", self.access_token.as_deref())?;
        let refresh_token = self.required_plain("refresh_token", self.refresh_token.as_deref())?;
        let client_id = self.required_plain("oauth_client_id", self.oauth_client_id.as_deref())?;
        let endpoint = self.oauth_token_endpoint(provider)?;
        let source = DbWriteBackTokenSource::new(
            self.row.id.clone(),
            access_token,
            self.row.token_expires_at,
            refresh_token,
            client_id,
            self.oauth_client_secret.clone(),
            provider,
            endpoint,
            self.encryption_key,
            match mode {
                TokenMode::WriteBack(writer) => writer.clone(),
                TokenMode::Static(_) => unreachable!("static tokens returned above"),
            },
            reqwest::Client::new(),
        );
        let source: Arc<dyn TokenSource> = Arc::new(source);
        Ok(Arc::new(OAuthRefresher::new(source)))
    }

    fn oauth_token_endpoint(
        &self,
        provider: MailProviderKind,
    ) -> Result<String, BifrostBuildError> {
        if provider == MailProviderKind::Jmap
            && self
                .row
                .oauth_token_url
                .as_deref()
                .filter(|url| !url.is_empty())
                .is_none()
            && !matches!(
                self.row.oauth_provider.as_deref(),
                Some("fastmail" | "jmap")
            )
        {
            return Err(BifrostBuildError::MissingEndpoint {
                account_id: self.row.id.clone(),
                provider,
            });
        }
        common::token::oauth_token_endpoint(
            oauth_provider_id(provider, self.row.oauth_provider.as_deref()),
            self.row.oauth_token_url.as_deref(),
        )
        .map_err(|error| BifrostBuildError::InvalidConfig {
            account_id: self.row.id.clone(),
            detail: error,
        })
    }

    /// Build the SMTP submission config. `oauth_source` is the SAME
    /// refresher the IMAP credentials use (when the account is bearer-auth),
    /// so IMAP and SMTP share single-flight refresh + write-back state
    /// rather than constructing independent refreshers. `None` for
    /// password-auth accounts.
    fn smtp_submission(
        &self,
        oauth_source: Option<Arc<dyn TokenSource>>,
    ) -> Result<Option<SmtpSubmissionConfig>, BifrostBuildError> {
        // Harness redirect: when RATATOSKR_TEST_SMTP_ENDPOINT is set, the SMTP
        // submission transport must target the saehrimnir mock (host:port,
        // plaintext) instead of the persisted submission host, which under the
        // harness is a non-resolvable placeholder (e.g. smtp.example.test).
        // This mirrors the RATATOSKR_TEST_IMAP_ENDPOINT override in
        // `build_imap_factory`. Plaintext (not STARTTLS) is required: the mock's
        // self-signed cert would be rejected by `starttls_relay`'s native-tls
        // verifier, and the mock accepts cleartext AUTH.
        let test_endpoint = std::env::var("RATATOSKR_TEST_SMTP_ENDPOINT")
            .ok()
            .filter(|endpoint| !endpoint.is_empty());
        let (host, tls, port_override) = if let Some(endpoint) = &test_endpoint {
            let (host, port) =
                parse_host_port(endpoint).ok_or_else(|| BifrostBuildError::InvalidConfig {
                    account_id: self.row.id.clone(),
                    detail: format!("invalid RATATOSKR_TEST_SMTP_ENDPOINT {endpoint}"),
                })?;
            (host, SubmissionTls::Plaintext, Some(port))
        } else {
            let Some(host) = self.row.smtp_host.clone().filter(|host| !host.is_empty()) else {
                return Ok(None);
            };
            let tls = match self.row.smtp_security.as_deref().unwrap_or("starttls") {
                "tls" | "ssl" => SubmissionTls::Implicit,
                "starttls" => SubmissionTls::StartTls,
                "none" => SubmissionTls::Plaintext,
                other => {
                    return Err(BifrostBuildError::InvalidConfig {
                        account_id: self.row.id.clone(),
                        detail: format!("unknown SMTP security mode {other}"),
                    });
                }
            };
            (host, tls, None)
        };
        let mut config =
            SmtpSubmissionConfig::new(host, tls, bifrost_types::Address::bare(&self.row.email));
        let port = match port_override {
            Some(port) => Some(port),
            None => self.optional_port(self.row.smtp_port, "smtp_port")?,
        };
        if let Some(port) = port {
            config = config.with_port(port);
        }
        if let Some(username) = self
            .row
            .smtp_username
            .clone()
            .filter(|value| !value.is_empty())
        {
            let credentials =
                if let Some(source) = oauth_source.filter(|_| self.smtp_password.is_none()) {
                    SubmissionCredentials::OAuth2 {
                        identity: username,
                        token_source: source,
                    }
                } else {
                    SubmissionCredentials::Password {
                        username,
                        password: self
                            .required_plain("smtp_password", self.smtp_password.as_deref())?,
                    }
                };
            config = config.with_credentials(credentials);
        }
        Ok(Some(config))
    }
}

fn read_bifrost_account_credentials(
    conn: &ReadConn<'_>,
    account_id: &str,
) -> Result<Result<AccountCredentialsRow, BifrostBuildError>, String> {
    let enabled_shared_mailboxes = conn
        .prepare("SELECT mailbox_id FROM shared_mailboxes WHERE account_id = ?1 AND is_sync_enabled = 1 AND revoked_at IS NULL")
        .map_err(|error| format!("prepare enabled shared mailboxes: {error}"))?
        .query_map(params![account_id], |row| row.get(0))
        .map_err(|error| format!("query enabled shared mailboxes: {error}"))?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|error| format!("collect enabled shared mailboxes: {error}"))?;
    let enabled_public_folder_pins = conn
        .prepare("SELECT folder_id FROM public_folder_pins WHERE account_id = ?1 AND is_sync_enabled = 1")
        .map_err(|error| format!("prepare enabled public-folder pins: {error}"))?
        .query_map(params![account_id], |row| row.get(0))
        .map_err(|error| format!("query enabled public-folder pins: {error}"))?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|error| format!("collect enabled public-folder pins: {error}"))?;
    let enabled_public_folder_pins: Vec<String> = enabled_public_folder_pins
        .into_iter()
        .map(|storage_id| {
            storage_id
                .strip_prefix("public:")
                .unwrap_or(&storage_id)
                .to_string()
        })
        .collect();
    conn.query_row(
        "SELECT id, email, provider, auth_method, access_token, refresh_token,
                token_expires_at, oauth_provider, oauth_client_id,
                oauth_client_secret, oauth_token_url, imap_host, imap_port,
                imap_security, imap_username, imap_password, smtp_host, smtp_port,
                smtp_security, smtp_username, smtp_password, jmap_url,
                calendar_provider, caldav_url, caldav_username, caldav_password,
                accept_invalid_certs, delegate_discovery_enabled, public_folders_enabled
         FROM accounts
         WHERE id = ?1",
        params![account_id],
        |row| {
            Ok(AccountCredentialsRow {
                id: row.get("id")?,
                email: row.get("email")?,
                provider: row.get("provider")?,
                auth_method: row
                    .get::<_, Option<String>>("auth_method")?
                    .unwrap_or_else(|| "oauth2".to_string()),
                access_token: row.get("access_token")?,
                refresh_token: row.get("refresh_token")?,
                token_expires_at: row.get("token_expires_at")?,
                oauth_provider: row.get("oauth_provider")?,
                oauth_client_id: row.get("oauth_client_id")?,
                oauth_client_secret: row.get("oauth_client_secret")?,
                oauth_token_url: row.get("oauth_token_url")?,
                imap_host: row.get("imap_host")?,
                imap_port: row.get("imap_port")?,
                imap_security: row.get("imap_security")?,
                imap_username: row.get("imap_username")?,
                imap_password: row.get("imap_password")?,
                smtp_host: row.get("smtp_host")?,
                smtp_port: row.get("smtp_port")?,
                smtp_security: row.get("smtp_security")?,
                smtp_username: row.get("smtp_username")?,
                smtp_password: row.get("smtp_password")?,
                jmap_url: row.get("jmap_url")?,
                calendar_provider: row.get("calendar_provider")?,
                caldav_url: row.get("caldav_url")?,
                caldav_username: row.get("caldav_username")?,
                caldav_password: row.get("caldav_password")?,
                accept_invalid_certs: row.get::<_, i64>("accept_invalid_certs")? != 0,
                delegate_discovery_enabled: row.get::<_, i64>("delegate_discovery_enabled")? != 0,
                public_folders_enabled: row.get::<_, i64>("public_folders_enabled")? != 0,
                enabled_shared_mailboxes: enabled_shared_mailboxes.clone(),
                enabled_public_folder_pins: enabled_public_folder_pins.clone(),
            })
        },
    )
    .map(Ok)
    .or_else(|error| {
        if matches!(
            error,
            db::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)
        ) {
            Ok(Err(BifrostBuildError::MissingCredential {
                account_id: account_id.to_string(),
                field: "account",
            }))
        } else {
            Err(format!("read bifrost account credentials: {error}"))
        }
    })
}

fn decrypt_optional(
    account_id: &str,
    field: &'static str,
    encrypted: Option<&String>,
    key: &[u8; 32],
) -> Result<Option<String>, BifrostBuildError> {
    encrypted
        .map(|value| {
            StoredSecret::parse(value.clone())
                .and_then(|secret| secret.decrypt(key))
                .map_err(|error| BifrostBuildError::Decrypt {
                    account_id: account_id.to_string(),
                    field,
                    error,
                })
        })
        .transpose()
}

fn oauth_provider_id(provider: MailProviderKind, stored: Option<&str>) -> &str {
    stored
        .filter(|value| !value.is_empty())
        .unwrap_or(match provider {
            MailProviderKind::Gmail => "google",
            MailProviderKind::Graph => "microsoft",
            MailProviderKind::Jmap => "jmap",
            MailProviderKind::Imap => "imap",
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::crypto;
    use db::db::{open_reader_pool, open_writer_pool};

    const KEY: [u8; 32] = [9u8; 32];

    // B1 spec 2.4: prove `build_account_factory`'s output type is exactly
    // what `bifrost_sync::SyncEngine::attach` consumes, without standing up
    // an engine (out of scope until B3). This is a compile-time bound proof;
    // it also keeps the `bifrost-sync` dependency edge (spec 3.4) live.
    #[allow(dead_code)]
    async fn factory_output_satisfies_attach_bound(
        engine: &bifrost_sync::SyncEngine,
        account_id: bifrost_types::AccountId,
        factory: Arc<dyn AccountFactory>,
    ) {
        let _ = engine.attach(account_id, factory).await;
    }

    #[test]
    fn bifrost_factory_unknown_provider_is_permanent() {
        for error in [
            BifrostBuildError::UnknownProvider("harness-offline".to_string()),
            BifrostBuildError::MissingCredential {
                account_id: "acct".to_string(),
                field: "access_token",
            },
            BifrostBuildError::Decrypt {
                account_id: "acct".to_string(),
                field: "access_token",
                error: "decrypt credential: bad".to_string(),
            },
        ] {
            assert_eq!(error.classify(), RemoteFailureKind::Permanent);
        }
    }

    // Asserts each `MailProviderKind` dispatches to a working factory arm.
    // The spec (5, Brick 4) suggested asserting *which* provider's factory was
    // returned via a downcast or test-only tag; neither is available against
    // the frozen bifrost surface (ff56478): `AccountFactory` has no `Any`
    // supertrait / downcast hook, no `Debug`, and no provider tag, and 3.2
    // pins the return as a bare `Arc<dyn AccountFactory>` (wrapping it in a
    // tag type would deviate from that). So dispatch is proven implicitly: the
    // four seeded rows carry kind-specific, non-interchangeable credential
    // shapes (JMAP needs `jmap_url`; IMAP needs `imap_host` + password; the
    // OAuth kinds need decryptable tokens + a resolvable endpoint), so a build
    // that routed a row to the wrong arm would fail the required-column reads
    // rather than return `Ok`.
    #[tokio::test]
    async fn bifrost_factory_builds_each_provider_kind() {
        let (writer, reader, dir) = test_dbs("builds");
        seed_oauth(&writer, "gmail", "gmail_api", "google", None, None).await;
        seed_oauth(&writer, "graph", "graph", "microsoft", None, None).await;
        seed_oauth(
            &writer,
            "jmap",
            "jmap",
            "custom",
            Some("https://mail.example.test/jmap"),
            Some("https://issuer.example.test/token"),
        )
        .await;
        seed_password_imap(&writer, "imap").await;

        for account_id in ["gmail", "graph", "jmap", "imap"] {
            let factory = build_account_factory(&reader, writer.clone(), account_id, KEY)
                .await
                .expect("factory builds");
            let _: Arc<dyn AccountFactory> = factory;
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn factory_from_decrypted_accepts_inflight_credentials_for_each_provider() {
        let cases = [
            VerifyAccountParams {
                provider: "gmail_api".to_string(),
                email: "gmail@example.test".to_string(),
                imap_host: None,
                imap_port: None,
                imap_security: None,
                username: None,
                imap_password: None,
                accept_invalid_certs: false,
                access_token: Some(service_api::RedactedString::new("gmail-token")),
                jmap_url: None,
            },
            VerifyAccountParams {
                provider: "graph".to_string(),
                email: "graph@example.test".to_string(),
                imap_host: None,
                imap_port: None,
                imap_security: None,
                username: None,
                imap_password: None,
                accept_invalid_certs: false,
                access_token: Some(service_api::RedactedString::new("graph-token")),
                jmap_url: None,
            },
            VerifyAccountParams {
                provider: "jmap".to_string(),
                email: "jmap@example.test".to_string(),
                imap_host: None,
                imap_port: None,
                imap_security: None,
                username: None,
                imap_password: None,
                accept_invalid_certs: false,
                access_token: Some(service_api::RedactedString::new("jmap-token")),
                jmap_url: Some("https://mail.example.test/jmap".to_string()),
            },
            VerifyAccountParams {
                provider: "imap".to_string(),
                email: "imap@example.test".to_string(),
                imap_host: Some("imap.example.test".to_string()),
                imap_port: Some(993),
                imap_security: Some("tls".to_string()),
                username: Some("imap@example.test".to_string()),
                imap_password: Some(service_api::RedactedString::new("password")),
                accept_invalid_certs: false,
                access_token: None,
                jmap_url: None,
            },
        ];
        for params in cases {
            let decrypted = DecryptedAccountCredentials::from_verify_params(params);
            let provider = MailProviderKind::parse(&decrypted.row.provider).expect("provider");
            factory_from_decrypted(
                &decrypted,
                provider,
                &TokenMode::Static("token".to_string()),
            )
            .expect("in-flight factory builds");
        }
    }

    #[test]
    fn from_verify_params_derives_auth_method_from_token_presence() {
        // `is_oauth()` reads `row.auth_method`, not token presence (R1-1 /
        // R2-4); `from_verify_params` must set it so OAuth verify builds an
        // OAuth factory arm and password verify builds a password arm.
        let oauth = DecryptedAccountCredentials::from_verify_params(VerifyAccountParams {
            provider: "gmail_api".to_string(),
            email: "a@example.test".to_string(),
            imap_host: None,
            imap_port: None,
            imap_security: None,
            username: None,
            imap_password: None,
            accept_invalid_certs: false,
            access_token: Some(service_api::RedactedString::new("tok")),
            jmap_url: None,
        });
        assert!(oauth.is_oauth(), "token present must classify as OAuth");

        let password = DecryptedAccountCredentials::from_verify_params(VerifyAccountParams {
            provider: "imap".to_string(),
            email: "a@example.test".to_string(),
            imap_host: Some("imap.example.test".to_string()),
            imap_port: Some(993),
            imap_security: Some("tls".to_string()),
            username: Some("a@example.test".to_string()),
            imap_password: Some(service_api::RedactedString::new("pw")),
            accept_invalid_certs: false,
            access_token: None,
            jmap_url: None,
        });
        assert!(!password.is_oauth(), "no token must classify as password");
    }

    #[tokio::test]
    async fn build_calendar_account_factory_routes_each_backend() {
        let (writer, reader, dir) = test_dbs("calendar-builds");
        seed_oauth(&writer, "gmail", "gmail_api", "google", None, None).await;
        seed_oauth(&writer, "graph", "graph", "microsoft", None, None).await;
        seed_oauth(
            &writer,
            "jmap",
            "jmap",
            "custom",
            Some("https://mail.example.test/jmap"),
            Some("https://issuer.example.test/token"),
        )
        .await;
        seed_calendar_caldav(&writer, "caldav", "caldav", None).await;
        // A Gmail mail account with CalDAV configured proves calendar routing
        // gives the explicit calendar backend precedence over mail transport.
        seed_calendar_caldav(&writer, "gmail-caldav", "gmail_api", Some("caldav")).await;

        for account_id in ["gmail", "graph", "jmap", "caldav", "gmail-caldav"] {
            let factory = build_calendar_account_factory(&reader, writer.clone(), account_id, KEY)
                .await
                .expect("calendar factory builds")
                .expect("calendar backend is available");
            let _: Arc<dyn AccountFactory> = factory;
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn bifrost_factory_harness_strings_are_unknown() {
        let (writer, reader, dir) = test_dbs("harness");
        seed_provider_only(&writer, "harness-offline", "harness-offline").await;
        seed_provider_only(&writer, "harness-slow-sync", "harness-slow-sync").await;

        for account_id in ["harness-offline", "harness-slow-sync"] {
            let Err(err) = build_account_factory(&reader, writer.clone(), account_id, KEY).await
            else {
                panic!("harness provider is not ported to bifrost");
            };
            assert!(matches!(err, BifrostBuildError::UnknownProvider(_)));
            assert_eq!(err.classify(), RemoteFailureKind::Permanent);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    fn test_dbs(name: &str) -> (WriterPool, ReadDbState, std::path::PathBuf) {
        let dir = std::env::current_dir()
            .expect("current dir")
            .join("target")
            .join("bifrost-factory-tests")
            .join(format!("{name}-{}", uuid::Uuid::new_v4()));
        let writer = open_writer_pool(&dir).expect("open writer pool");
        let reader = open_reader_pool(&dir).expect("open reader pool");
        (writer, reader, dir)
    }

    async fn seed_provider_only(writer: &WriterPool, id: &str, provider: &str) {
        writer
            .with_write({
                let id = id.to_string();
                let provider = provider.to_string();
                move |conn| {
                    conn.execute(
                        "INSERT INTO accounts (
                            id, email, provider, auth_method, account_name, account_color
                         ) VALUES (?1, ?2, ?3, 'oauth2', 'Test', '#000000')",
                        db::db::params![id, format!("{id}@example.test"), provider],
                    )
                    .map_err(|error| error.to_string())?;
                    Ok(())
                }
            })
            .await
            .expect("seed provider row");
    }

    async fn seed_calendar_caldav(
        writer: &WriterPool,
        id: &str,
        provider: &str,
        calendar_provider: Option<&str>,
    ) {
        let username = encrypt("calendar-user");
        let password = encrypt("calendar-password");
        writer
            .with_write({
                let id = id.to_string();
                let provider = provider.to_string();
                let calendar_provider = calendar_provider.map(ToOwned::to_owned);
                move |conn| {
                    conn.execute(
                        "INSERT INTO accounts (id, email, provider, auth_method, account_name, account_color, calendar_provider, caldav_url, caldav_username, caldav_password) VALUES (?1, ?2, ?3, 'password', 'Test', '#000000', ?4, 'https://caldav.example.test', ?5, ?6)",
                        db::db::params![id, format!("{id}@example.test"), provider, calendar_provider, username, password],
                    )
                    .map_err(|error| error.to_string())?;
                    Ok(())
                }
            })
            .await
            .expect("seed calendar CalDAV row");
    }

    async fn seed_oauth(
        writer: &WriterPool,
        id: &str,
        provider: &str,
        oauth_provider: &str,
        jmap_url: Option<&str>,
        oauth_token_url: Option<&str>,
    ) {
        let access = encrypt("access");
        let refresh = encrypt("refresh");
        let client_id = encrypt("client");
        let client_secret = encrypt("secret");
        writer
            .with_write({
                let id = id.to_string();
                let provider = provider.to_string();
                let oauth_provider = oauth_provider.to_string();
                let jmap_url = jmap_url.map(ToOwned::to_owned);
                let oauth_token_url = oauth_token_url.map(ToOwned::to_owned);
                move |conn| {
                    conn.execute(
                        "INSERT INTO accounts (
                            id, email, provider, auth_method, access_token,
                            refresh_token, token_expires_at, oauth_provider,
                            oauth_client_id, oauth_client_secret, oauth_token_url,
                            jmap_url, account_name, account_color
                         ) VALUES (
                            ?1, ?2, ?3, 'oauth2', ?4,
                            ?5, 4102444800, ?6,
                            ?7, ?8, ?9,
                            ?10, 'Test', '#000000'
                         )",
                        db::db::params![
                            id,
                            format!("{id}@example.test"),
                            provider,
                            access,
                            refresh,
                            oauth_provider,
                            client_id,
                            client_secret,
                            oauth_token_url,
                            jmap_url,
                        ],
                    )
                    .map_err(|error| error.to_string())?;
                    Ok(())
                }
            })
            .await
            .expect("seed oauth row");
    }

    async fn seed_password_imap(writer: &WriterPool, id: &str) {
        let password = encrypt("password");
        writer
            .with_write({
                let id = id.to_string();
                move |conn| {
                    conn.execute(
                        "INSERT INTO accounts (
                            id, email, provider, auth_method, imap_host, imap_port,
                            imap_security, imap_username, imap_password, smtp_host,
                            smtp_port, smtp_security, account_name, account_color
                         ) VALUES (
                            ?1, ?2, 'imap', 'password', 'imap.example.test', 993,
                            'tls', ?2, ?3, 'smtp.example.test',
                            587, 'starttls', 'Test', '#000000'
                         )",
                        db::db::params![id, format!("{id}@example.test"), password],
                    )
                    .map_err(|error| error.to_string())?;
                    Ok(())
                }
            })
            .await
            .expect("seed imap row");
    }

    fn encrypt(value: &str) -> String {
        crypto::encrypt_value(&KEY, value).expect("encrypt test value")
    }
}
