use async_imap::{Authenticator, Client, Session};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_native_tls::TlsStream;

use super::types::*;

// ---------- Timeout constants ----------

pub const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
pub const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
pub const AUTH_TIMEOUT: Duration = Duration::from_secs(30);
pub const IMAP_CMD_TIMEOUT: Duration = Duration::from_secs(30);
pub const IMAP_FETCH_TIMEOUT: Duration = Duration::from_secs(120);
pub const IMAP_SEARCH_TIMEOUT: Duration = Duration::from_secs(60);
pub const OVERALL_CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
/// Timeout for graceful IMAP LOGOUT before dropping the connection.
pub const IMAP_LOGOUT_TIMEOUT: Duration = Duration::from_secs(5);

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

// ---------- SASL authenticators ----------

/// XOAUTH2 authenticator (Google-style).
/// Format: `user={user}\x01auth=Bearer {token}\x01\x01`
struct XOAuth2 {
    response: Vec<u8>,
}

impl XOAuth2 {
    fn new(user: &str, access_token: &str) -> Self {
        let s = format!("user={user}\x01auth=Bearer {access_token}\x01\x01");
        Self {
            response: s.into_bytes(),
        }
    }
}

impl Authenticator for XOAuth2 {
    type Response = Vec<u8>;
    fn process(&mut self, _challenge: &[u8]) -> Self::Response {
        let resp = std::mem::take(&mut self.response);
        if resp.is_empty() {
            // Error acknowledgment: respond with \x01 per protocol spec
            vec![0x01]
        } else {
            resp
        }
    }
}

/// OAUTHBEARER authenticator (RFC 7628).
/// Format: `n,a={user},\x01auth=Bearer {token}\x01\x01`
///
/// `user` should be the full email address (authorization identity per
/// RFC 7628). The GS2 header escapes `=` → `=3D` and `,` → `=2C`
/// per RFC 5801 §4.
///
/// Preferred over XOAUTH2 for modern on-prem servers (Dovecot, Cyrus)
/// and standards-compliant OIDC deployments.
struct OAuthBearer {
    response: Vec<u8>,
}

impl OAuthBearer {
    fn new(user: &str, access_token: &str) -> Self {
        // RFC 5801 §4: the authzid in the GS2 header must encode '=' as '=3D'
        // and ',' as '=2C' to avoid conflicting with the GS2 header delimiters.
        let escaped_user = user.replace('=', "=3D").replace(',', "=2C");
        let s = format!("n,a={escaped_user},\x01auth=Bearer {access_token}\x01\x01");
        Self {
            response: s.into_bytes(),
        }
    }
}

impl Authenticator for OAuthBearer {
    type Response = Vec<u8>;
    fn process(&mut self, _challenge: &[u8]) -> Self::Response {
        let resp = std::mem::take(&mut self.response);
        if resp.is_empty() {
            // Error acknowledgment: respond with \x01 per RFC 7628 §3.2.3
            vec![0x01]
        } else {
            resp
        }
    }
}

// ---------- Stream wrapper ----------

/// Wrapper to unify TLS / plain streams so Session can be generic.
pub enum ImapStream {
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
    builder
        .build()
        .map_err(|e| format!("Failed to create TLS connector: {e}"))
}

// ---------- Public API ----------

pub type ImapSession = Session<ImapStream>;

/// Establish an IMAP connection and authenticate.
///
/// Supports TLS (direct), STARTTLS (upgrade), and plain connections.
/// Auth methods: "password" (LOGIN) or "oauth2" (XOAUTH2).
///
/// Wraps the entire connection + auth sequence in a 60s overall timeout.
pub async fn connect(config: &ImapConfig) -> Result<ImapSession, String> {
    log::info!("[IMAP] Connecting to {}:{} (security={}, auth={})", config.host, config.port, config.security, config.auth_method);
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

/// Authenticate with the IMAP server.
///
/// Auth methods:
/// - `"oauth2"` — XOAUTH2 (Google/Microsoft legacy)
/// - `"oauthbearer"` — OAUTHBEARER RFC 7628 (modern on-prem OIDC)
/// - anything else — LOGIN (password)
async fn authenticate(
    client: Client<ImapStream>,
    config: &ImapConfig,
) -> Result<ImapSession, String> {
    match config.auth_method.as_str() {
        "oauthbearer" => {
            log::debug!("[IMAP] Authenticating with OAUTHBEARER as {}", config.username);
            let auth = OAuthBearer::new(&config.username, &config.password);
            client
                .authenticate("OAUTHBEARER", auth)
                .await
                .map_err(|(e, _)| {
                    log::error!("[IMAP] OAUTHBEARER authentication failed for {}: {e}", config.username);
                    format!("OAUTHBEARER authentication failed: {e}")
                })
        }
        "oauth2" => {
            log::debug!("[IMAP] Authenticating with XOAUTH2 as {}", config.username);
            let auth = XOAuth2::new(&config.username, &config.password);
            client
                .authenticate("XOAUTH2", auth)
                .await
                .map_err(|(e, _)| {
                    log::error!("[IMAP] XOAUTH2 authentication failed for {}: {e}", config.username);
                    format!("XOAUTH2 authentication failed: {e}")
                })
        }
        _ => {
            log::debug!("[IMAP] Authenticating with LOGIN as {}", config.username);
            client
                .login(&config.username, &config.password)
                .await
                .map_err(|(e, _)| {
                    log::error!("[IMAP] LOGIN failed for {}: {e}", config.username);
                    format!("Login failed: {e}")
                })
        }
    }
}

/// Negotiated CONDSTORE/QRESYNC capability state for a session.
#[derive(Debug, Clone)]
pub struct ImapCapabilities {
    /// Server supports CONDSTORE (RFC 4551) — HIGHESTMODSEQ in SELECT and
    /// CHANGEDSINCE modifier for UID FETCH.
    pub condstore: bool,
    /// Server supports QRESYNC (RFC 7162) and successfully responded to
    /// `ENABLE QRESYNC`. False if the server advertises QRESYNC but has a
    /// broken implementation (e.g. iCloud).
    pub qresync: bool,
}

/// Probe the server's CONDSTORE/QRESYNC capabilities and negotiate QRESYNC
/// if available.
///
/// iCloud is known to advertise QRESYNC in CAPABILITY but not actually
/// respond with `ENABLED QRESYNC` after the `ENABLE` command. This function
/// detects that case and falls back to CONDSTORE-only mode.
pub async fn negotiate_condstore_qresync(
    session: &mut ImapSession,
) -> Result<ImapCapabilities, String> {
    let caps = tokio::time::timeout(IMAP_CMD_TIMEOUT, session.capabilities())
        .await
        .map_err(|_| format!(
            "CAPABILITY timed out after {}s",
            IMAP_CMD_TIMEOUT.as_secs()
        ))?
        .map_err(|e| format!("CAPABILITY failed: {e}"))?;

    let has_condstore = caps.has_str("CONDSTORE");
    let has_qresync_cap = caps.has_str("QRESYNC");

    if !has_condstore && !has_qresync_cap {
        log::info!("IMAP: server supports neither CONDSTORE nor QRESYNC");
        return Ok(ImapCapabilities {
            condstore: false,
            qresync: false,
        });
    }

    if !has_qresync_cap {
        log::info!("IMAP: server supports CONDSTORE but not QRESYNC");
        return Ok(ImapCapabilities {
            condstore: true,
            qresync: false,
        });
    }

    // Server advertises QRESYNC — try to ENABLE it and verify the ENABLED
    // response.  QRESYNC implicitly enables CONDSTORE (RFC 7162 §3.2.3).
    let qresync_enabled = tokio::time::timeout(IMAP_CMD_TIMEOUT, async {
        let tag = session
            .run_command("ENABLE QRESYNC")
            .await
            .map_err(|e| format!("ENABLE QRESYNC send failed: {e}"))?;

        // Read responses until the tagged OK. Look for an ENABLED response
        // containing QRESYNC. imap-proto parses `* ENABLED QRESYNC` as
        // `Response::Capabilities([Atom("QRESYNC")])`.
        let mut saw_enabled = false;
        loop {
            let resp = session
                .read_response()
                .await
                .map_err(|e| format!("ENABLE QRESYNC read failed: {e}"))?
                .ok_or_else(|| "Connection lost during ENABLE QRESYNC".to_string())?;

            match resp.parsed() {
                async_imap::imap_proto::Response::Capabilities(caps) => {
                    for cap in caps {
                        if let async_imap::imap_proto::types::Capability::Atom(name) = cap
                            && name.eq_ignore_ascii_case("QRESYNC")
                        {
                            saw_enabled = true;
                        }
                    }
                }
                async_imap::imap_proto::Response::Done { tag: resp_tag, .. } if *resp_tag == tag => {
                    break;
                }
                _ => {}
            }
        }

        Ok::<bool, String>(saw_enabled)
    })
    .await
    .map_err(|_| format!(
        "ENABLE QRESYNC timed out after {}s",
        IMAP_CMD_TIMEOUT.as_secs()
    ))??;

    if qresync_enabled {
        log::info!("IMAP: QRESYNC successfully enabled (implies CONDSTORE)");
        Ok(ImapCapabilities {
            condstore: true,
            qresync: true,
        })
    } else {
        log::warn!(
            "IMAP: server advertises QRESYNC in CAPABILITY but did not respond with \
             ENABLED QRESYNC — likely iCloud or similar broken implementation. \
             Falling back to CONDSTORE-only mode."
        );
        Ok(ImapCapabilities {
            condstore: has_condstore,
            qresync: false,
        })
    }
}

// ---------- NAMESPACE discovery (RFC 2342) ----------

/// Parse a NAMESPACE response line into a `NamespaceInfo`.
///
/// The response format is three sections (personal, other_users, shared),
/// each either `NIL` or a parenthesized list of `(prefix delimiter)` pairs:
/// ```text
/// (("" "/")) (("Other Users/" "/")) (("Shared/" "/"))
/// ```
pub(crate) fn parse_namespace_response(line: &str) -> Result<NamespaceInfo, String> {
    let line = line.trim();
    let mut info = NamespaceInfo::default();
    let mut pos = 0;
    let bytes = line.as_bytes();

    // Parse three sections in order: personal, other_users, shared
    let sections = [&mut info.personal, &mut info.other_users, &mut info.shared];

    for section in sections {
        // Skip whitespace
        while pos < bytes.len() && bytes[pos] == b' ' {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }

        // Check for NIL
        if bytes[pos..].starts_with(b"NIL") || bytes[pos..].starts_with(b"nil") {
            pos += 3;
            continue;
        }

        // Expect opening '(' for the section list
        if bytes[pos] != b'(' {
            return Err(format!(
                "Expected '(' or NIL at position {pos} in NAMESPACE response: {line}"
            ));
        }
        pos += 1;

        // Parse entries: each is (prefix delimiter)
        loop {
            // Skip whitespace
            while pos < bytes.len() && bytes[pos] == b' ' {
                pos += 1;
            }
            if pos >= bytes.len() || bytes[pos] == b')' {
                pos += 1; // consume closing ')'
                break;
            }

            // Expect '(' for entry
            if bytes[pos] != b'(' {
                return Err(format!(
                    "Expected '(' for entry at position {pos} in NAMESPACE response: {line}"
                ));
            }
            pos += 1;

            // Parse prefix (quoted string)
            let prefix = parse_ns_string(bytes, &mut pos)?;

            // Skip space
            while pos < bytes.len() && bytes[pos] == b' ' {
                pos += 1;
            }

            // Parse delimiter (quoted string or NIL)
            let delimiter = if bytes[pos..].starts_with(b"NIL") || bytes[pos..].starts_with(b"nil")
            {
                pos += 3;
                None
            } else {
                Some(parse_ns_string(bytes, &mut pos)?)
            };

            // Skip whitespace before closing ')'
            while pos < bytes.len() && bytes[pos] == b' ' {
                pos += 1;
            }

            // Consume optional extension data before closing ')'
            // Some servers include extra data like TRANSLATION after the delimiter
            while pos < bytes.len() && bytes[pos] != b')' {
                if bytes[pos] == b'"' {
                    // Skip quoted string
                    let _ = parse_ns_string(bytes, &mut pos)?;
                } else if bytes[pos] == b'(' {
                    // Skip parenthesized list
                    let mut depth = 1;
                    pos += 1;
                    while pos < bytes.len() && depth > 0 {
                        if bytes[pos] == b'(' {
                            depth += 1;
                        } else if bytes[pos] == b')' {
                            depth -= 1;
                        }
                        pos += 1;
                    }
                } else {
                    pos += 1;
                }
            }

            if pos < bytes.len() && bytes[pos] == b')' {
                pos += 1; // consume entry closing ')'
            }

            section.push(NamespaceEntry { prefix, delimiter });
        }
    }

    Ok(info)
}

/// Parse a quoted string from the NAMESPACE response at the given position.
fn parse_ns_string(bytes: &[u8], pos: &mut usize) -> Result<String, String> {
    if *pos >= bytes.len() || bytes[*pos] != b'"' {
        return Err(format!(
            "Expected '\"' at position {pos} in NAMESPACE string",
            pos = *pos
        ));
    }
    *pos += 1; // skip opening quote

    let start = *pos;
    while *pos < bytes.len() && bytes[*pos] != b'"' {
        if bytes[*pos] == b'\\' {
            *pos += 1; // skip escaped char
        }
        *pos += 1;
    }
    let s = String::from_utf8_lossy(&bytes[start..*pos]).to_string();
    if *pos < bytes.len() {
        *pos += 1; // skip closing quote
    }
    Ok(s)
}

/// Discover IMAP namespaces by sending the NAMESPACE command (RFC 2342).
///
/// Returns the personal, other-users, and shared namespace prefixes and
/// delimiters. This is used to identify shared/delegated mailbox prefixes
/// for Dovecot/Cyrus servers.
pub async fn discover_namespaces(
    session: &mut ImapSession,
) -> Result<NamespaceInfo, String> {
    tokio::time::timeout(IMAP_CMD_TIMEOUT, async {
        let tag = session
            .run_command("NAMESPACE")
            .await
            .map_err(|e| format!("NAMESPACE send failed: {e}"))?;

        let mut result = NamespaceInfo::default();
        loop {
            let resp = session
                .read_response()
                .await
                .map_err(|e| format!("NAMESPACE read failed: {e}"))?
                .ok_or_else(|| "Connection lost during NAMESPACE".to_string())?;

            match resp.parsed() {
                async_imap::imap_proto::Response::Done { tag: resp_tag, .. }
                    if *resp_tag == tag =>
                {
                    break;
                }
                _ => {
                    // Look for untagged NAMESPACE response in raw bytes.
                    // imap_proto does not have a NAMESPACE parser, so we
                    // inspect the raw response line.
                    let raw = String::from_utf8_lossy(resp.borrow_owner());
                    if let Some(ns_start) = raw.find("NAMESPACE ") {
                        let ns_data = &raw[ns_start + "NAMESPACE ".len()..];
                        // Strip trailing \r\n
                        let ns_data = ns_data.trim_end();
                        match parse_namespace_response(ns_data) {
                            Ok(info) => {
                                result = info;
                                log::info!(
                                    "IMAP NAMESPACE: personal={:?}, other_users={:?}, shared={:?}",
                                    result.personal,
                                    result.other_users,
                                    result.shared
                                );
                            }
                            Err(e) => {
                                log::warn!("Failed to parse NAMESPACE response: {e}");
                            }
                        }
                    }
                }
            }
        }

        Ok(result)
    })
    .await
    .map_err(|_| format!(
        "NAMESPACE timed out after {}s",
        IMAP_CMD_TIMEOUT.as_secs()
    ))?
}

// ---------- ACL discovery: MYRIGHTS (RFC 4314) ----------

/// Discover the current user's access rights on a folder via MYRIGHTS (RFC 4314).
///
/// Returns the rights string (e.g. `"lrswipcda"`). Common right characters:
/// - `l` lookup, `r` read, `s` seen, `w` write flags, `i` insert
/// - `p` post, `c`/`k` create subfolder, `d`/`t` delete, `e` expunge, `a` admin
pub async fn discover_myrights(
    session: &mut ImapSession,
    folder: &str,
) -> Result<String, String> {
    tokio::time::timeout(IMAP_CMD_TIMEOUT, async {
        let cmd = format!("MYRIGHTS \"{}\"", folder.replace('"', "\\\""));
        let tag = session
            .run_command(&cmd)
            .await
            .map_err(|e| format!("MYRIGHTS send failed: {e}"))?;

        let mut rights = String::new();
        loop {
            let resp = session
                .read_response()
                .await
                .map_err(|e| format!("MYRIGHTS read failed: {e}"))?
                .ok_or_else(|| "Connection lost during MYRIGHTS".to_string())?;

            match resp.parsed() {
                async_imap::imap_proto::Response::Done {
                    tag: resp_tag,
                    status,
                    information,
                    ..
                } if *resp_tag == tag => {
                    if !matches!(status, async_imap::imap_proto::Status::Ok) {
                        let info = information
                            .as_deref()
                            .unwrap_or("unknown error");
                        return Err(format!("MYRIGHTS failed: {info}"));
                    }
                    break;
                }
                async_imap::imap_proto::Response::MyRights(my_rights) => {
                    // imap_proto parses MYRIGHTS into AclRight variants;
                    // convert back to the compact character string.
                    rights = my_rights
                        .rights
                        .iter()
                        .map(|r| char::from(*r))
                        .collect();
                }
                _ => {}
            }
        }

        log::info!("IMAP MYRIGHTS \"{folder}\": {rights}");
        Ok(rights)
    })
    .await
    .map_err(|_| format!(
        "MYRIGHTS timed out after {}s",
        IMAP_CMD_TIMEOUT.as_secs()
    ))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_standard_namespace() {
        let input = r#"(("" "/")) (("Other Users/" "/")) (("Shared/" "/"))"#;
        let info = parse_namespace_response(input).unwrap();
        assert_eq!(info.personal.len(), 1);
        assert_eq!(info.personal[0].prefix, "");
        assert_eq!(info.personal[0].delimiter.as_deref(), Some("/"));
        assert_eq!(info.other_users.len(), 1);
        assert_eq!(info.other_users[0].prefix, "Other Users/");
        assert_eq!(info.other_users[0].delimiter.as_deref(), Some("/"));
        assert_eq!(info.shared.len(), 1);
        assert_eq!(info.shared[0].prefix, "Shared/");
        assert_eq!(info.shared[0].delimiter.as_deref(), Some("/"));
    }

    #[test]
    fn parse_namespace_with_nil_sections() {
        let input = r#"(("INBOX." ".")) NIL NIL"#;
        let info = parse_namespace_response(input).unwrap();
        assert_eq!(info.personal.len(), 1);
        assert_eq!(info.personal[0].prefix, "INBOX.");
        assert_eq!(info.personal[0].delimiter.as_deref(), Some("."));
        assert!(info.other_users.is_empty());
        assert!(info.shared.is_empty());
    }

    #[test]
    fn parse_namespace_multiple_entries() {
        let input =
            r##"(("" "/")("#mbox" "/")) NIL (("Public Folders/" "/"))"##;
        let info = parse_namespace_response(input).unwrap();
        assert_eq!(info.personal.len(), 2);
        assert_eq!(info.personal[0].prefix, "");
        assert_eq!(info.personal[1].prefix, "#mbox");
        assert!(info.other_users.is_empty());
        assert_eq!(info.shared.len(), 1);
        assert_eq!(info.shared[0].prefix, "Public Folders/");
    }

    #[test]
    fn parse_namespace_all_nil() {
        let input = "NIL NIL NIL";
        let info = parse_namespace_response(input).unwrap();
        assert!(info.personal.is_empty());
        assert!(info.other_users.is_empty());
        assert!(info.shared.is_empty());
    }

    #[test]
    fn parse_namespace_nil_delimiter() {
        let input = r#"(("" NIL)) NIL NIL"#;
        let info = parse_namespace_response(input).unwrap();
        assert_eq!(info.personal.len(), 1);
        assert_eq!(info.personal[0].prefix, "");
        assert!(info.personal[0].delimiter.is_none());
    }

    #[test]
    fn rights_string_has_read_access() {
        let rights = "lrswipcda";
        assert!(rights.contains('l'), "should have lookup");
        assert!(rights.contains('r'), "should have read");
        assert!(rights.contains('s'), "should have seen");
    }

    #[test]
    fn rights_string_read_only() {
        let rights = "lr";
        assert!(rights.contains('l'));
        assert!(rights.contains('r'));
        assert!(!rights.contains('w'), "should not have write");
        assert!(!rights.contains('i'), "should not have insert");
        assert!(!rights.contains('d'), "should not have delete");
    }

    #[test]
    fn oauthbearer_gs2_escapes_special_chars() {
        // RFC 5801 §4: '=' must be encoded as '=3D', ',' as '=2C' in authzid
        let auth = OAuthBearer::new("user=test,org@example.com", "tok123");
        let payload = String::from_utf8(auth.response).expect("valid utf-8");
        assert_eq!(
            payload,
            "n,a=user=3Dtest=2Corg@example.com,\x01auth=Bearer tok123\x01\x01"
        );
    }

    #[test]
    fn oauthbearer_plain_email_unchanged() {
        let auth = OAuthBearer::new("alice@example.com", "tok");
        let payload = String::from_utf8(auth.response).expect("valid utf-8");
        assert_eq!(payload, "n,a=alice@example.com,\x01auth=Bearer tok\x01\x01");
    }

    #[test]
    fn namespace_type_classification() {
        let info = parse_namespace_response(
            r#"(("" "/")) (("Other Users/" "/")) (("Shared/" "/"))"#,
        )
        .unwrap();

        // A folder path can be classified by checking which namespace prefix it matches
        let folder = "Shared/team-inbox";
        let ns_type = classify_folder_namespace(&info, folder);
        assert_eq!(ns_type, Some(NamespaceType::Shared));

        let folder = "Other Users/bob/INBOX";
        let ns_type = classify_folder_namespace(&info, folder);
        assert_eq!(ns_type, Some(NamespaceType::OtherUsers));

        let folder = "INBOX";
        let ns_type = classify_folder_namespace(&info, folder);
        assert_eq!(ns_type, Some(NamespaceType::Personal));
    }
}

/// Classify which namespace a folder path belongs to based on prefix matching.
pub(crate) fn classify_folder_namespace(info: &NamespaceInfo, folder_path: &str) -> Option<NamespaceType> {
    // Check other_users and shared first (they have non-empty prefixes),
    // then fall back to personal.
    for entry in &info.other_users {
        if !entry.prefix.is_empty() && folder_path.starts_with(&entry.prefix) {
            return Some(NamespaceType::OtherUsers);
        }
    }
    for entry in &info.shared {
        if !entry.prefix.is_empty() && folder_path.starts_with(&entry.prefix) {
            return Some(NamespaceType::Shared);
        }
    }
    // If the personal namespace has a non-empty prefix, check it; otherwise
    // treat everything not matched above as personal.
    for entry in &info.personal {
        if entry.prefix.is_empty() || folder_path.starts_with(&entry.prefix) {
            return Some(NamespaceType::Personal);
        }
    }
    None
}
