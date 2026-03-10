use async_imap::{Authenticator, Client, Session};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_native_tls::TlsStream;

use super::types::*;

// ---------- Timeout constants ----------

pub(crate) const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const AUTH_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const IMAP_CMD_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const IMAP_FETCH_TIMEOUT: Duration = Duration::from_secs(120);
pub(crate) const IMAP_SEARCH_TIMEOUT: Duration = Duration::from_secs(60);
pub(crate) const OVERALL_CONNECT_TIMEOUT: Duration = Duration::from_secs(60);

/// Configure TCP keepalive and nodelay on a connected socket.
pub(crate) fn configure_tcp_socket(stream: &TcpStream) {
    // Set TCP nodelay via tokio's built-in API
    if let Err(e) = stream.set_nodelay(true) {
        log::warn!("Failed to set TCP_NODELAY: {e}");
    }

    // Set TCP keepalive via socket2
    let sock_ref = socket2::SockRef::from(stream);
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(Duration::from_secs(60))
        .with_interval(Duration::from_secs(60));
    if let Err(e) = sock_ref.set_tcp_keepalive(&keepalive) {
        log::warn!("Failed to set TCP keepalive: {e}");
    }
}

// ---------- XOAUTH2 authenticator ----------

struct XOAuth2 {
    response: Vec<u8>,
}

impl XOAuth2 {
    fn new(user: &str, access_token: &str) -> Self {
        // XOAUTH2 format: "user=" {user} "\x01auth=Bearer " {token} "\x01\x01"
        let s = format!("user={user}\x01auth=Bearer {access_token}\x01\x01");
        Self {
            response: s.into_bytes(),
        }
    }
}

impl Authenticator for XOAuth2 {
    type Response = Vec<u8>;
    fn process(&mut self, _challenge: &[u8]) -> Self::Response {
        // Return the initial XOAUTH2 string on the first (empty) challenge.
        // If the server sends a second challenge it means auth failed; we send
        // an empty response to let the server return a proper error.
        std::mem::take(&mut self.response)
    }
}

// ---------- Stream wrapper ----------

/// Wrapper to unify TLS / plain streams so Session can be generic.
pub(crate) enum ImapStream {
    Tls(TlsStream<TcpStream>),
    Plain(TcpStream),
}

impl tokio::io::AsyncRead for ImapStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            ImapStream::Tls(s) => std::pin::Pin::new(s).poll_read(cx, buf),
            ImapStream::Plain(s) => std::pin::Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl tokio::io::AsyncWrite for ImapStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match self.get_mut() {
            ImapStream::Tls(s) => std::pin::Pin::new(s).poll_write(cx, buf),
            ImapStream::Plain(s) => std::pin::Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            ImapStream::Tls(s) => std::pin::Pin::new(s).poll_flush(cx),
            ImapStream::Plain(s) => std::pin::Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            ImapStream::Tls(s) => std::pin::Pin::new(s).poll_shutdown(cx),
            ImapStream::Plain(s) => std::pin::Pin::new(s).poll_shutdown(cx),
        }
    }
}

impl std::fmt::Debug for ImapStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImapStream::Tls(_) => write!(f, "ImapStream::Tls"),
            ImapStream::Plain(_) => write!(f, "ImapStream::Plain"),
        }
    }
}

// ---------- TLS helper ----------

/// Build a TLS connector, optionally accepting invalid certificates
/// (for local mail bridges like ProtonMail Bridge with self-signed certs).
pub(crate) fn build_tls_connector(accept_invalid_certs: bool) -> Result<native_tls::TlsConnector, String> {
    let mut builder = native_tls::TlsConnector::builder();
    if accept_invalid_certs {
        builder.danger_accept_invalid_certs(true);
    }
    builder.build().map_err(|e| format!("Failed to create TLS connector: {e}"))
}

// ---------- Public API ----------

pub(crate) type ImapSession = Session<ImapStream>;

/// Establish an IMAP connection and authenticate.
///
/// Supports TLS (direct), STARTTLS (upgrade), and plain connections.
/// Auth methods: "password" (LOGIN) or "oauth2" (XOAUTH2).
///
/// Wraps the entire connection + auth sequence in a 60s overall timeout.
pub async fn connect(config: &ImapConfig) -> Result<ImapSession, String> {
    tokio::time::timeout(OVERALL_CONNECT_TIMEOUT, connect_inner(config))
        .await
        .map_err(|_| format!(
            "IMAP connection to {}:{} timed out after {}s — check your server settings or network connection",
            config.host, config.port, OVERALL_CONNECT_TIMEOUT.as_secs()
        ))?
}

async fn connect_inner(config: &ImapConfig) -> Result<ImapSession, String> {
    if config.security == "starttls" {
        return connect_starttls(config).await;
    }

    let stream = connect_stream(config).await?;
    let client = Client::new(stream);

    tokio::time::timeout(AUTH_TIMEOUT, authenticate(client, config))
        .await
        .map_err(|_| format!(
            "IMAP authentication timed out after {}s — check your server settings or network connection",
            AUTH_TIMEOUT.as_secs()
        ))?
}

/// Establish TCP + TLS or plain stream for "tls" and "none" security modes.
pub(crate) async fn connect_stream(config: &ImapConfig) -> Result<ImapStream, String> {
    let addr = (&*config.host, config.port);

    match config.security.as_str() {
        "tls" => {
            let native_connector = build_tls_connector(config.accept_invalid_certs)?;
            let tls_connector = tokio_native_tls::TlsConnector::from(native_connector);
            let tcp = tokio::time::timeout(TCP_CONNECT_TIMEOUT, TcpStream::connect(addr))
                .await
                .map_err(|_| format!(
                    "TCP connect to {}:{} timed out after {}s — check your server settings or network connection",
                    config.host, config.port, TCP_CONNECT_TIMEOUT.as_secs()
                ))?
                .map_err(|e| format!("TCP connect to {}:{} failed: {e}", config.host, config.port))?;
            configure_tcp_socket(&tcp);
            let tls = tokio::time::timeout(TLS_HANDSHAKE_TIMEOUT, tls_connector.connect(&config.host, tcp))
                .await
                .map_err(|_| format!(
                    "TLS handshake with {} timed out after {}s — check your server settings or network connection",
                    config.host, TLS_HANDSHAKE_TIMEOUT.as_secs()
                ))?
                .map_err(|e| format!("TLS handshake with {} failed: {e}", config.host))?;
            Ok(ImapStream::Tls(tls))
        }
        "none" => {
            let tcp = tokio::time::timeout(TCP_CONNECT_TIMEOUT, TcpStream::connect(addr))
                .await
                .map_err(|_| format!(
                    "TCP connect to {}:{} timed out after {}s — check your server settings or network connection",
                    config.host, config.port, TCP_CONNECT_TIMEOUT.as_secs()
                ))?
                .map_err(|e| format!("TCP connect to {}:{} failed: {e}", config.host, config.port))?;
            configure_tcp_socket(&tcp);
            Ok(ImapStream::Plain(tcp))
        }
        other => Err(format!(
            "Unknown security mode: {other}. Use \"tls\", \"starttls\", or \"none\"."
        )),
    }
}

/// Handle STARTTLS connection: connect plain, upgrade to TLS, then authenticate.
///
/// STARTTLS is special because we must issue the STARTTLS command on the plain
/// connection, upgrade the underlying TCP stream to TLS, and then create a new
/// Client on the TLS stream for authentication.
async fn connect_starttls(config: &ImapConfig) -> Result<ImapSession, String> {
    let addr = (&*config.host, config.port);
    let mut tcp = tokio::time::timeout(TCP_CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| format!(
            "TCP connect to {}:{} timed out after {}s — check your server settings or network connection",
            config.host, config.port, TCP_CONNECT_TIMEOUT.as_secs()
        ))?
        .map_err(|e| format!("TCP connect to {}:{} failed: {e}", config.host, config.port))?;
    configure_tcp_socket(&tcp);

    // Read the server greeting
    let mut buf = vec![0u8; 4096];
    let n = tokio::time::timeout(IMAP_CMD_TIMEOUT, tcp.read(&mut buf))
        .await
        .map_err(|_| format!(
            "Reading server greeting timed out after {}s — check your server settings or network connection",
            IMAP_CMD_TIMEOUT.as_secs()
        ))?
        .map_err(|e| format!("Failed to read server greeting: {e}"))?;
    let greeting = String::from_utf8_lossy(&buf[..n]);
    if !greeting.contains("OK") {
        return Err(format!("Unexpected server greeting: {greeting}"));
    }

    // Send STARTTLS command
    tcp.write_all(b"a001 STARTTLS\r\n")
        .await
        .map_err(|e| format!("Failed to send STARTTLS: {e}"))?;

    // Read STARTTLS response
    let n = tokio::time::timeout(IMAP_CMD_TIMEOUT, tcp.read(&mut buf))
        .await
        .map_err(|_| format!(
            "STARTTLS response timed out after {}s — check your server settings or network connection",
            IMAP_CMD_TIMEOUT.as_secs()
        ))?
        .map_err(|e| format!("Failed to read STARTTLS response: {e}"))?;
    let response = String::from_utf8_lossy(&buf[..n]);
    if !response.contains("OK") {
        return Err(format!("STARTTLS rejected: {response}"));
    }

    // Upgrade to TLS
    let native_connector = build_tls_connector(config.accept_invalid_certs)?;
    let tls_connector = tokio_native_tls::TlsConnector::from(native_connector);
    let tls = tokio::time::timeout(TLS_HANDSHAKE_TIMEOUT, tls_connector.connect(&config.host, tcp))
        .await
        .map_err(|_| format!(
            "TLS upgrade after STARTTLS timed out after {}s — check your server settings or network connection",
            TLS_HANDSHAKE_TIMEOUT.as_secs()
        ))?
        .map_err(|e| format!("TLS upgrade after STARTTLS failed: {e}"))?;

    // Create a new IMAP client on the TLS stream and authenticate
    let client = Client::new(ImapStream::Tls(tls));
    tokio::time::timeout(AUTH_TIMEOUT, authenticate(client, config))
        .await
        .map_err(|_| format!(
            "IMAP authentication timed out after {}s — check your server settings or network connection",
            AUTH_TIMEOUT.as_secs()
        ))?
}

/// Authenticate with the IMAP server (LOGIN or XOAUTH2).
async fn authenticate(
    client: Client<ImapStream>,
    config: &ImapConfig,
) -> Result<ImapSession, String> {
    match config.auth_method.as_str() {
        "oauth2" => {
            let auth = XOAuth2::new(&config.username, &config.password);
            client
                .authenticate("XOAUTH2", auth)
                .await
                .map_err(|(e, _)| format!("XOAUTH2 authentication failed: {e}"))
        }
        _ => client
            .login(&config.username, &config.password)
            .await
            .map_err(|(e, _)| format!("Login failed: {e}")),
    }
}
