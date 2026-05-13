use std::fmt;

/// Typed error enum for provider operations.
///
/// Replaces `Result<T, String>` across the provider layer so that callers can
/// distinguish auth failures from network errors, rate limits, etc.
#[derive(Debug, Clone)]
pub enum ProviderError {
    /// Authentication failed (expired token, invalid credentials, OAuth error).
    Auth(String),
    /// Network or connection error.
    Network(String),
    /// Rate limited by server (includes Retry-After detail if available).
    RateLimit(String),
    /// Resource not found (message, folder, mailbox, etc.).
    NotFound(String),
    /// Server returned an error we don't specifically classify.
    Server(String),
    /// Client-side error (invalid input, serialization, encoding, etc.).
    Client(String),
    /// Database error.
    Db(String),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auth(msg) => write!(f, "auth error: {msg}"),
            Self::Network(msg) => write!(f, "network error: {msg}"),
            Self::RateLimit(msg) => write!(f, "rate limited: {msg}"),
            Self::NotFound(msg) => write!(f, "not found: {msg}"),
            Self::Server(msg) => write!(f, "server error: {msg}"),
            Self::Client(msg) => write!(f, "client error: {msg}"),
            Self::Db(msg) => write!(f, "database error: {msg}"),
        }
    }
}

impl std::error::Error for ProviderError {}

/// Classification of `ProviderError` for retry / backoff decisions.
///
/// Phase 7 of the attachments roadmap introduces this split so the
/// prefetch pipeline can distinguish errors that will likely succeed on
/// a later attempt (network blip, 5xx, rate limit) from errors that
/// will keep failing without external intervention (token expired,
/// 4xx, blob not on server).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorKind {
    /// Likely to succeed on a later retry. Network blips, 5xx, rate
    /// limits, transient DB locks.
    Transient,
    /// Will keep failing until external state changes. Expired auth,
    /// 4xx, missing blob, client-side validation errors.
    Permanent,
}

impl ProviderError {
    /// Classify the error for retry / breaker decisions.
    pub fn kind(&self) -> ProviderErrorKind {
        match self {
            Self::Network(_) | Self::RateLimit(_) | Self::Server(_) | Self::Db(_) => {
                ProviderErrorKind::Transient
            }
            Self::Auth(_) | Self::NotFound(_) | Self::Client(_) => ProviderErrorKind::Permanent,
        }
    }
}

/// Fallback conversion: any bare `String` error maps to `Client`.
///
/// This keeps existing `.map_err(|e| e.to_string())?` and `?` on
/// `Result<T, String>` compiling during the migration.
impl From<String> for ProviderError {
    fn from(msg: String) -> Self {
        Self::Client(msg)
    }
}

/// Convenience: `&str` also converts to `Client`.
impl From<&str> for ProviderError {
    fn from(msg: &str) -> Self {
        Self::Client(msg.to_string())
    }
}

impl From<rusqlite::Error> for ProviderError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Db(err.to_string())
    }
}
