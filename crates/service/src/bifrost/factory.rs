use std::fmt;
use std::sync::Arc;

use bifrost_graph::account::{GraphAccountFactory, GraphClient};
use bifrost_imap::{
    AuthPolicy, Credentials, ImapAccountConfig, ImapAccountFactory, ImapConfig,
    SmtpSubmissionConfig, SubmissionCredentials, SubmissionTls,
};
use bifrost_jmap_new::sync::{JmapAccountFactory, JmapCredentials};
use bifrost_net::{OAuthRefresher, TokenSource};
use bifrost_types::AccountFactory;
use common::crypto::StoredSecret;
use db::db::{ReadConn, ReadDbState, WriterPool, params};
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
    match provider {
        MailProviderKind::Gmail => {
            let source = decrypted.oauth_token_source(provider, writer)?;
            let mut factory = match std::env::var("RATATOSKR_TEST_GMAIL_ENDPOINT")
                .ok()
                .and_then(|endpoint| gmail_api_base_from_test_endpoint(&endpoint))
            {
                Some(api_base) => {
                    bifrost_google::account::GoogleAccountFactory::from_token_source_with_api_base(
                        source, api_base,
                    )
                }
                None => bifrost_google::account::GoogleAccountFactory::from_token_source(source),
            };
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
            let graph_beta = std::env::var("RATATOSKR_TEST_GRAPH_ENDPOINT")
                .ok()
                .map(|base| format!("{}/beta", base.trim_end_matches('/')))
                .unwrap_or_else(|| "https://graph.microsoft.com/beta".to_string());
            let client = if std::env::var("RATATOSKR_TEST_GRAPH_ENDPOINT").is_ok() {
                let access_token =
                    decrypted.required_plain("access_token", decrypted.access_token.as_deref())?;
                GraphClient::with_api_bases(graph_base, graph_beta, access_token)
            } else {
                let source = decrypted.oauth_token_source(provider, writer)?;
                GraphClient::with_source(graph_base, graph_beta, source)
            };
            let mut factory = GraphAccountFactory::new(client);
            if let Ok(webhook_url) = std::env::var("RATATOSKR_GRAPH_PUSH_NOTIFICATION_URL") {
                factory = factory.with_push_endpoint(webhook_url);
            }
            Ok(Arc::new(factory))
        }
        MailProviderKind::Jmap => build_jmap_factory(&decrypted, provider, writer),
        MailProviderKind::Imap => build_imap_factory(decrypted, provider, writer),
    }
}

fn build_jmap_factory(
    account: &DecryptedAccountCredentials,
    provider: MailProviderKind,
    writer: WriterPool,
) -> Result<Arc<dyn AccountFactory>, BifrostBuildError> {
    let url = match std::env::var("RATATOSKR_TEST_JMAP_ENDPOINT") {
        Ok(url) => url,
        Err(_) => account.required_plain("jmap_url", account.row.jmap_url.as_deref())?,
    };
    let credentials = if account.is_oauth() {
        JmapCredentials::Bearer {
            token_source: account.oauth_token_source(provider, writer)?,
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
    account: DecryptedAccountCredentials,
    provider: MailProviderKind,
    writer: WriterPool,
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
                account_id: account.row.id,
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
        Some(account.oauth_token_source(provider, writer)?)
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
    accept_invalid_certs: bool,
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
            encryption_key,
            row: self,
        })
    }
}

struct DecryptedAccountCredentials {
    row: AccountCredentialsRow,
    access_token: Option<String>,
    refresh_token: Option<String>,
    oauth_client_id: Option<String>,
    oauth_client_secret: Option<String>,
    imap_password: Option<String>,
    smtp_password: Option<String>,
    encryption_key: [u8; 32],
}

impl DecryptedAccountCredentials {
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

    fn oauth_token_source(
        &self,
        provider: MailProviderKind,
        writer: WriterPool,
    ) -> Result<Arc<dyn TokenSource>, BifrostBuildError> {
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
            writer,
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
        let mut config =
            SmtpSubmissionConfig::new(host, tls, bifrost_types::Address::bare(&self.row.email));
        if let Some(port) = self.optional_port(self.row.smtp_port, "smtp_port")? {
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
    conn.query_row(
        "SELECT id, email, provider, auth_method, access_token, refresh_token,
                token_expires_at, oauth_provider, oauth_client_id,
                oauth_client_secret, oauth_token_url, imap_host, imap_port,
                imap_security, imap_username, imap_password, smtp_host, smtp_port,
                smtp_security, smtp_username, smtp_password, jmap_url,
                accept_invalid_certs
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
                accept_invalid_certs: row.get::<_, i64>("accept_invalid_certs")? != 0,
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
