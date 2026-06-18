use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bifrost_net::{AccessToken, Error, FinalResponse, TokenSource};
use bifrost_types::{AccountFuture, TransmissionState};
use bytes::Bytes;
use common::crypto;
use db::db::WriterPool;
use reqwest::StatusCode;
use serde::Deserialize;
use tokio::sync::RwLock;
use types::MailProviderKind;

#[derive(Clone)]
struct CachedToken {
    access_token: String,
    expires_at: Option<i64>,
}

pub struct DbWriteBackTokenSource {
    account_id: String,
    refresh_token: String,
    client_id: String,
    client_secret: Option<String>,
    provider: MailProviderKind,
    token_endpoint: String,
    encryption_key: [u8; 32],
    writer: WriterPool,
    http: reqwest::Client,
    cached: Arc<RwLock<CachedToken>>,
}

impl DbWriteBackTokenSource {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        account_id: String,
        access_token: String,
        expires_at: Option<i64>,
        refresh_token: String,
        client_id: String,
        client_secret: Option<String>,
        provider: MailProviderKind,
        token_endpoint: String,
        encryption_key: [u8; 32],
        writer: WriterPool,
        http: reqwest::Client,
    ) -> Self {
        Self {
            account_id,
            refresh_token,
            client_id,
            client_secret,
            provider,
            token_endpoint,
            encryption_key,
            writer,
            http,
            cached: Arc::new(RwLock::new(CachedToken {
                access_token,
                expires_at,
            })),
        }
    }
}

impl fmt::Debug for DbWriteBackTokenSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DbWriteBackTokenSource")
            .field("account_id", &self.account_id)
            .field("refresh_token", &"<redacted>")
            .field("client_id", &"<redacted>")
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "<redacted>"),
            )
            .field("provider", &self.provider)
            .field("token_endpoint", &self.token_endpoint)
            .field("encryption_key", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl TokenSource for DbWriteBackTokenSource {
    fn current(&self) -> AccountFuture<Result<AccessToken, Error>> {
        let cached = Arc::clone(&self.cached);
        Box::pin(async move {
            let cached = cached.read().await;
            Ok(AccessToken::new(
                cached.access_token.clone(),
                instant_from_unix(cached.expires_at),
            ))
        })
    }

    fn refresh(&self) -> AccountFuture<Result<AccessToken, Error>> {
        let account_id = self.account_id.clone();
        let refresh_token = self.refresh_token.clone();
        let client_id = self.client_id.clone();
        let client_secret = self.client_secret.clone();
        let token_endpoint = self.token_endpoint.clone();
        let key = self.encryption_key;
        let writer = self.writer.clone();
        let http = self.http.clone();
        let cached = Arc::clone(&self.cached);
        Box::pin(async move {
            let refreshed = refresh_oauth_token_typed(
                &http,
                &token_endpoint,
                &refresh_token,
                &client_id,
                client_secret.as_deref(),
            )
            .await?;
            let token = AccessToken::new(
                refreshed.access_token.clone(),
                instant_from_unix(Some(refreshed.expires_at)),
            );
            {
                let mut cached = cached.write().await;
                cached.access_token = refreshed.access_token.clone();
                cached.expires_at = Some(refreshed.expires_at);
            }
            persist_refreshed_token_best_effort(
                writer,
                account_id,
                refreshed.access_token,
                refreshed.expires_at,
                key,
            )
            .await;
            Ok(token)
        })
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: i64,
}

struct TokenRefreshResult {
    access_token: String,
    expires_at: i64,
}

async fn refresh_oauth_token_typed(
    http: &reqwest::Client,
    token_endpoint: &str,
    refresh_token: &str,
    client_id: &str,
    client_secret: Option<&str>,
) -> Result<TokenRefreshResult, Error> {
    let mut params = vec![
        ("refresh_token", refresh_token),
        ("client_id", client_id),
        ("grant_type", "refresh_token"),
    ];
    if let Some(secret) = client_secret
        && !secret.is_empty()
    {
        params.push(("client_secret", secret));
    }

    let response = http
        .post(token_endpoint)
        .form(&params)
        .send()
        .await
        .map_err(|error| Error::Network {
            message: error.to_string(),
            transmission_state: TransmissionState::Unsent,
            source: Some(Box::new(error)),
        })?;

    let status = response.status();
    let headers = response.headers().clone();
    if !status.is_success() {
        let body = response
            .bytes()
            .await
            .map(bifrost_net::error::cap_status_body)
            .unwrap_or_else(|_| Bytes::new());
        if status == StatusCode::UNAUTHORIZED
            || status == StatusCode::FORBIDDEN
            || body_contains_invalid_grant(&body)
        {
            return Err(Error::AuthLost {
                transmission_state: None,
                final_response: Some(FinalResponse {
                    status,
                    headers,
                    body,
                }),
            });
        }
        return Err(Error::Status {
            code: status,
            body,
            headers,
        });
    }

    let resp: TokenResponse = response.json().await.map_err(|error| Error::Network {
        message: format!("failed to parse token response: {error}"),
        transmission_state: TransmissionState::Acknowledged,
        source: Some(Box::new(error)),
    })?;
    let now = chrono::Utc::now().timestamp();
    Ok(TokenRefreshResult {
        access_token: resp.access_token,
        expires_at: now + resp.expires_in,
    })
}

fn body_contains_invalid_grant(body: &Bytes) -> bool {
    std::str::from_utf8(body)
        .map(|body| body.contains("\"invalid_grant\"") || body.contains("invalid_grant"))
        .unwrap_or(false)
}

async fn persist_refreshed_token_best_effort(
    writer: WriterPool,
    account_id: String,
    access_token: String,
    expires_at: i64,
    key: [u8; 32],
) {
    let encrypted = match crypto::encrypt_value(&key, &access_token) {
        Ok(encrypted) => encrypted,
        Err(error) => {
            log::warn!(
                "failed to encrypt refreshed bifrost access token for {account_id}: {error}"
            );
            return;
        }
    };
    if let Err(error) = writer
        .with_write(move |conn| {
            db::db::queries::persist_refreshed_token(conn, &account_id, &encrypted, expires_at)
        })
        .await
    {
        log::warn!("failed to persist refreshed bifrost access token: {error}");
    }
}

fn instant_from_unix(expires_at: Option<i64>) -> Option<Instant> {
    let expires_at = expires_at?;
    let now = chrono::Utc::now().timestamp();
    if expires_at <= now {
        return Some(Instant::now());
    }
    let seconds = u64::try_from(expires_at - now).ok()?;
    Instant::now().checked_add(Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use db::db::open_writer_pool;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const KEY: [u8; 32] = [7u8; 32];

    #[tokio::test]
    async fn bifrost_token_source_refresh_persists_encrypted() {
        let (writer, dir) = test_writer("persist");
        seed_account(&writer, "acct-persist", "old-token", 100).await;
        let endpoint = token_endpoint(
            "HTTP/1.1 200 OK",
            r#"{"access_token":"new-token","expires_in":3600}"#,
        )
        .await;
        let source = source(
            writer.clone(),
            "acct-persist",
            "old-token",
            Some(100),
            endpoint,
        );

        let token = source.refresh().await.expect("refresh succeeds");

        assert_eq!(token.as_str(), "new-token");
        let (encrypted, expires_at) = writer
            .with_read(|conn| {
                conn.query_row(
                    "SELECT access_token, token_expires_at FROM accounts WHERE id = 'acct-persist'",
                    [],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                )
                .map_err(|error| error.to_string())
            })
            .await
            .expect("read refreshed token");
        assert_ne!(encrypted, "new-token");
        let decrypted = crypto::StoredSecret::parse(encrypted)
            .expect("stored secret parses")
            .decrypt(&KEY)
            .expect("stored secret decrypts");
        assert_eq!(decrypted, "new-token");
        assert!(expires_at > chrono::Utc::now().timestamp());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn bifrost_token_source_refresh_writeback_failure_is_non_fatal() {
        let (writer, dir) = test_writer("writeback-fail");
        // Drop the table the write-back targets so `persist_refreshed_token`
        // errors. The wire refresh has already succeeded, so a failed
        // write-back must NOT fail the refresh - the deliberate divergence
        // from the legacy Gmail `do_refresh` that propagates the persist
        // error with `?`.
        writer
            .with_write(|conn| {
                conn.execute("DROP TABLE accounts", db::db::params![])
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            })
            .await
            .expect("drop accounts table");
        let endpoint = token_endpoint(
            "HTTP/1.1 200 OK",
            r#"{"access_token":"writeback-fail-token","expires_in":3600}"#,
        )
        .await;
        let source = source(
            writer,
            "acct-writeback-fail",
            "old-token",
            Some(100),
            endpoint,
        );

        let token = source
            .refresh()
            .await
            .expect("refresh succeeds despite write-back failure");

        assert_eq!(token.as_str(), "writeback-fail-token");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn bifrost_token_source_refresh_401_is_auth_lost() {
        let (writer, dir) = test_writer("auth-lost");
        seed_account(&writer, "acct-auth-lost", "old-token", 100).await;
        let endpoint =
            token_endpoint("HTTP/1.1 401 Unauthorized", r#"{"error":"invalid_grant"}"#).await;
        let source = source(writer, "acct-auth-lost", "old-token", Some(100), endpoint);

        let err = source
            .refresh()
            .await
            .expect_err("refresh token is rejected");

        assert!(matches!(err, Error::AuthLost { .. }));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn bifrost_token_source_debug_redacts_secret() {
        let (writer, dir) = test_writer("debug");
        let source = source(
            writer,
            "acct-debug",
            "visible-access",
            Some(100),
            "http://127.0.0.1:9/token".to_string(),
        );

        let debug = format!("{source:?}");

        assert!(!debug.contains("refresh-secret"));
        assert!(!debug.contains("client-secret"));
        assert!(!debug.contains("visible-access"));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn test_writer(name: &str) -> (WriterPool, std::path::PathBuf) {
        let dir = std::env::current_dir()
            .expect("current dir")
            .join("target")
            .join("bifrost-token-source-tests")
            .join(format!("{name}-{}", uuid::Uuid::new_v4()));
        let writer = open_writer_pool(&dir).expect("open writer pool");
        (writer, dir)
    }

    async fn seed_account(
        writer: &WriterPool,
        account_id: &str,
        access_token: &str,
        expires_at: i64,
    ) {
        let encrypted_access = crypto::encrypt_value(&KEY, access_token).expect("encrypt access");
        let encrypted_refresh =
            crypto::encrypt_value(&KEY, "refresh-secret").expect("encrypt refresh");
        let encrypted_client = crypto::encrypt_value(&KEY, "client-id").expect("encrypt client");
        writer
            .with_write({
                let account_id = account_id.to_string();
                move |conn| {
                    conn.execute(
                        "INSERT INTO accounts (
                            id, email, provider, auth_method, access_token,
                            refresh_token, token_expires_at, oauth_provider,
                            oauth_client_id, account_name, account_color
                         ) VALUES (
                            ?1, 'test@example.com', 'gmail_api', 'oauth2', ?2,
                            ?3, ?4, 'google', ?5, 'Test', '#000000'
                         )",
                        db::db::params![
                            account_id,
                            encrypted_access,
                            encrypted_refresh,
                            expires_at,
                            encrypted_client,
                        ],
                    )
                    .map_err(|error| error.to_string())?;
                    Ok(())
                }
            })
            .await
            .expect("seed account");
    }

    fn source(
        writer: WriterPool,
        account_id: &str,
        access_token: &str,
        expires_at: Option<i64>,
        endpoint: String,
    ) -> DbWriteBackTokenSource {
        DbWriteBackTokenSource::new(
            account_id.to_string(),
            access_token.to_string(),
            expires_at,
            "refresh-secret".to_string(),
            "client-id".to_string(),
            Some("client-secret".to_string()),
            MailProviderKind::Gmail,
            endpoint,
            KEY,
            writer,
            reqwest::Client::new(),
        )
    }

    async fn token_endpoint(status: &'static str, body: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind token endpoint");
        let addr = listener.local_addr().expect("token endpoint addr");
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept token request");
            let mut request = [0u8; 2048];
            let _ = stream.read(&mut request).await.expect("read request");
            let response = format!(
                "{status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write token response");
        });
        format!("http://{addr}/token")
    }
}
