use serde::Serialize;

/// Complete discovery result for an email address.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredConfig {
    /// The email address that was queried.
    pub email: String,
    /// The domain extracted from the email.
    pub domain: String,
    /// Protocol options, ranked by preference (index 0 = best).
    pub options: Vec<ProtocolOption>,
    /// If MX lookup resolved to a different provider domain.
    pub resolved_domain: Option<String>,
    /// Per-stage diagnostics.
    pub diagnostics: Vec<StageDiagnostic>,
}

/// A single protocol option discovered for the email address.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolOption {
    pub protocol: Protocol,
    pub auth: AuthConfig,
    /// Display name for the provider (e.g., "Fastmail").
    pub provider_name: Option<String>,
    /// Where this option came from.
    pub source: DiscoverySource,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Protocol {
    GmailApi,
    MicrosoftGraph,
    #[serde(rename_all = "camelCase")]
    Jmap {
        session_url: String,
    },
    #[serde(rename_all = "camelCase")]
    Imap {
        incoming: ServerConfig,
        outgoing: ServerConfig,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    pub hostname: String,
    pub port: u16,
    pub security: Security,
    pub username: UsernameFormat,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Security {
    Tls,
    StartTls,
    None,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UsernameFormat {
    EmailAddress,
    LocalPart,
    Custom { value: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthConfig {
    pub method: AuthMethod,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub alternatives: Vec<AuthMethod>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AuthMethod {
    Password,
    #[serde(rename_all = "camelCase")]
    OAuth2 {
        provider_id: String,
        auth_url: String,
        token_url: String,
        scopes: Vec<String>,
        use_pkce: bool,
    },
    #[serde(rename_all = "camelCase")]
    OAuth2Unsupported {
        provider_domain: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DiscoverySource {
    Registry,
    #[serde(rename_all = "camelCase")]
    AutoconfigXml { url: String },
    #[serde(rename_all = "camelCase")]
    MxLookup { mx_domain: String },
    JmapWellKnown,
    PortProbe,
}

/// Diagnostics for a single discovery stage.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageDiagnostic {
    pub stage: &'static str,
    pub duration_ms: u64,
    pub outcome: StageOutcome,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum StageOutcome {
    Found { count: usize },
    NotFound,
    Error { message: String },
    Skipped,
}

impl Protocol {
    /// Priority for ranking (lower = better).
    pub fn priority(&self) -> u8 {
        match self {
            Self::GmailApi => 0,
            Self::MicrosoftGraph => 1,
            Self::Jmap { .. } => 2,
            Self::Imap { .. } => 3,
        }
    }
}

impl DiscoverySource {
    /// Confidence for ranking (lower = more confident).
    pub fn confidence(&self) -> u8 {
        match self {
            Self::Registry => 0,
            Self::AutoconfigXml { .. } => 1,
            Self::MxLookup { .. } => 2,
            Self::JmapWellKnown => 1,
            Self::PortProbe => 4,
        }
    }
}
