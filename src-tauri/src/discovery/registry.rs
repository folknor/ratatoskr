use crate::discovery::types::{
    AuthConfig, AuthMethod, DiscoverySource, Protocol, ProtocolOption, Security, ServerConfig,
    UsernameFormat,
};

struct RegistryEntry {
    domains: &'static [&'static str],
    name: &'static str,
    help_url: Option<&'static str>,
    /// Protocol options paired with optional auth overrides.
    /// If the auth override is None, the entry-level default auth is used.
    options: &'static [(RegistryProtocol, Option<RegistryAuthConfig>)],
    /// Default auth config for options that don't override.
    default_auth: RegistryAuthConfig,
}

enum RegistryProtocol {
    GmailApi,
    MicrosoftGraph,
    Jmap {
        session_url: &'static str,
    },
    Imap {
        imap_host: &'static str,
        imap_port: u16,
        imap_security: Security,
        smtp_host: &'static str,
        smtp_port: u16,
        smtp_security: Security,
    },
}

enum RegistryAuth {
    Password,
    OAuth2 {
        provider_id: &'static str,
        auth_url: &'static str,
        token_url: &'static str,
        scopes: &'static [&'static str],
        use_pkce: bool,
    },
}

struct RegistryAuthConfig {
    primary: RegistryAuth,
    alternatives: &'static [RegistryAuth],
}

const GOOGLE_OAUTH: RegistryAuth = RegistryAuth::OAuth2 {
    provider_id: "google",
    auth_url: "https://accounts.google.com/o/oauth2/v2/auth",
    token_url: "https://oauth2.googleapis.com/token",
    scopes: &[
        "https://www.googleapis.com/auth/gmail.modify",
        "https://www.googleapis.com/auth/gmail.send",
        "https://www.googleapis.com/auth/gmail.readonly",
        "openid",
        "profile",
        "email",
    ],
    use_pkce: true,
};

const MICROSOFT_IMAP_OAUTH: RegistryAuth = RegistryAuth::OAuth2 {
    provider_id: "microsoft",
    auth_url: "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize",
    token_url: "https://login.microsoftonline.com/consumers/oauth2/v2.0/token",
    scopes: &[
        "https://outlook.office.com/IMAP.AccessAsUser.All",
        "https://outlook.office.com/SMTP.Send",
        "offline_access",
        "openid",
        "profile",
        "email",
    ],
    use_pkce: true,
};

const YAHOO_OAUTH: RegistryAuth = RegistryAuth::OAuth2 {
    provider_id: "yahoo",
    auth_url: "https://api.login.yahoo.com/oauth2/request_auth",
    token_url: "https://api.login.yahoo.com/oauth2/get_token",
    scopes: &["mail-r", "mail-w", "openid", "sdps-r"],
    use_pkce: true,
};

static REGISTRY: &[RegistryEntry] = &[
    // Gmail
    RegistryEntry {
        domains: &["gmail.com", "googlemail.com"],
        name: "Gmail",
        help_url: None,
        options: &[
            (RegistryProtocol::GmailApi, None),
            (
                RegistryProtocol::Imap {
                    imap_host: "imap.gmail.com",
                    imap_port: 993,
                    imap_security: Security::Tls,
                    smtp_host: "smtp.gmail.com",
                    smtp_port: 465,
                    smtp_security: Security::Tls,
                },
                None,
            ),
        ],
        default_auth: RegistryAuthConfig {
            primary: RegistryAuth::OAuth2 {
                provider_id: "google",
                auth_url: "https://accounts.google.com/o/oauth2/v2/auth",
                token_url: "https://oauth2.googleapis.com/token",
                scopes: &[
                    "https://www.googleapis.com/auth/gmail.modify",
                    "https://www.googleapis.com/auth/gmail.send",
                    "https://www.googleapis.com/auth/gmail.readonly",
                    "openid",
                    "profile",
                    "email",
                ],
                use_pkce: true,
            },
            alternatives: &[],
        },
    },
    // Microsoft Outlook / Hotmail — Graph gets Graph scopes, IMAP gets IMAP scopes
    RegistryEntry {
        domains: &[
            "outlook.com",
            "hotmail.com",
            "live.com",
            "msn.com",
            "outlook.co.uk",
            "hotmail.co.uk",
        ],
        name: "Outlook",
        help_url: None,
        options: &[
            (
                RegistryProtocol::MicrosoftGraph,
                Some(RegistryAuthConfig {
                    primary: RegistryAuth::OAuth2 {
                        provider_id: "microsoft_graph",
                        auth_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
                        token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
                        scopes: &[
                            "Mail.ReadWrite",
                            "Mail.Send",
                            "MailboxSettings.ReadWrite",
                            "offline_access",
                            "openid",
                            "profile",
                            "email",
                        ],
                        use_pkce: true,
                    },
                    alternatives: &[],
                }),
            ),
            (
                RegistryProtocol::Imap {
                    imap_host: "imap-mail.outlook.com",
                    imap_port: 993,
                    imap_security: Security::Tls,
                    smtp_host: "smtp-mail.outlook.com",
                    smtp_port: 587,
                    smtp_security: Security::StartTls,
                },
                Some(RegistryAuthConfig {
                    primary: RegistryAuth::OAuth2 {
                        provider_id: "microsoft",
                        auth_url: "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize",
                        token_url: "https://login.microsoftonline.com/consumers/oauth2/v2.0/token",
                        scopes: &[
                            "https://outlook.office.com/IMAP.AccessAsUser.All",
                            "https://outlook.office.com/SMTP.Send",
                            "offline_access",
                            "openid",
                            "profile",
                            "email",
                        ],
                        use_pkce: true,
                    },
                    alternatives: &[],
                }),
            ),
        ],
        // Unused since both options have overrides, but required by the struct
        default_auth: RegistryAuthConfig {
            primary: RegistryAuth::Password,
            alternatives: &[],
        },
    },
    // Yahoo
    RegistryEntry {
        domains: &["yahoo.com", "yahoo.co.uk", "yahoo.co.jp", "ymail.com"],
        name: "Yahoo",
        help_url: None,
        options: &[(
            RegistryProtocol::Imap {
                imap_host: "imap.mail.yahoo.com",
                imap_port: 993,
                imap_security: Security::Tls,
                smtp_host: "smtp.mail.yahoo.com",
                smtp_port: 465,
                smtp_security: Security::Tls,
            },
            None,
        )],
        default_auth: RegistryAuthConfig {
            primary: RegistryAuth::OAuth2 {
                provider_id: "yahoo",
                auth_url: "https://api.login.yahoo.com/oauth2/request_auth",
                token_url: "https://api.login.yahoo.com/oauth2/get_token",
                scopes: &["mail-r", "mail-w", "openid", "sdps-r"],
                use_pkce: true,
            },
            alternatives: &[RegistryAuth::Password],
        },
    },
    // iCloud
    RegistryEntry {
        domains: &["icloud.com", "me.com", "mac.com"],
        name: "iCloud",
        help_url: Some("https://support.apple.com/102654"),
        options: &[(
            RegistryProtocol::Imap {
                imap_host: "imap.mail.me.com",
                imap_port: 993,
                imap_security: Security::Tls,
                smtp_host: "smtp.mail.me.com",
                smtp_port: 587,
                smtp_security: Security::StartTls,
            },
            None,
        )],
        default_auth: RegistryAuthConfig {
            primary: RegistryAuth::Password,
            alternatives: &[],
        },
    },
    // Fastmail
    RegistryEntry {
        domains: &["fastmail.com", "fastmail.fm", "messagingengine.com"],
        name: "Fastmail",
        help_url: None,
        options: &[
            (
                RegistryProtocol::Jmap {
                    session_url: "https://api.fastmail.com/jmap/session",
                },
                None,
            ),
            (
                RegistryProtocol::Imap {
                    imap_host: "imap.fastmail.com",
                    imap_port: 993,
                    imap_security: Security::Tls,
                    smtp_host: "smtp.fastmail.com",
                    smtp_port: 465,
                    smtp_security: Security::Tls,
                },
                None,
            ),
        ],
        default_auth: RegistryAuthConfig {
            primary: RegistryAuth::Password,
            alternatives: &[],
        },
    },
    // Zoho
    RegistryEntry {
        domains: &["zoho.com", "zohomail.com"],
        name: "Zoho",
        help_url: None,
        options: &[(
            RegistryProtocol::Imap {
                imap_host: "imap.zoho.com",
                imap_port: 993,
                imap_security: Security::Tls,
                smtp_host: "smtp.zoho.com",
                smtp_port: 465,
                smtp_security: Security::Tls,
            },
            None,
        )],
        default_auth: RegistryAuthConfig {
            primary: RegistryAuth::Password,
            alternatives: &[],
        },
    },
    // AOL
    RegistryEntry {
        domains: &["aol.com"],
        name: "AOL",
        help_url: None,
        options: &[(
            RegistryProtocol::Imap {
                imap_host: "imap.aol.com",
                imap_port: 993,
                imap_security: Security::Tls,
                smtp_host: "smtp.aol.com",
                smtp_port: 465,
                smtp_security: Security::Tls,
            },
            None,
        )],
        default_auth: RegistryAuthConfig {
            primary: RegistryAuth::Password,
            alternatives: &[],
        },
    },
    // GMX
    RegistryEntry {
        domains: &["gmx.com", "gmx.net", "gmx.de"],
        name: "GMX",
        help_url: None,
        options: &[(
            RegistryProtocol::Imap {
                imap_host: "imap.gmx.com",
                imap_port: 993,
                imap_security: Security::Tls,
                smtp_host: "mail.gmx.com",
                smtp_port: 465,
                smtp_security: Security::Tls,
            },
            None,
        )],
        default_auth: RegistryAuthConfig {
            primary: RegistryAuth::Password,
            alternatives: &[],
        },
    },
    // Mail.ru
    RegistryEntry {
        domains: &["mail.ru", "inbox.ru", "list.ru", "bk.ru"],
        name: "Mail.ru",
        help_url: None,
        options: &[(
            RegistryProtocol::Imap {
                imap_host: "imap.mail.ru",
                imap_port: 993,
                imap_security: Security::Tls,
                smtp_host: "smtp.mail.ru",
                smtp_port: 465,
                smtp_security: Security::Tls,
            },
            None,
        )],
        default_auth: RegistryAuthConfig {
            primary: RegistryAuth::Password,
            alternatives: &[],
        },
    },
    // Mailo
    RegistryEntry {
        domains: &["mailo.com", "net-c.com", "netc.fr"],
        name: "Mailo",
        help_url: None,
        options: &[(
            RegistryProtocol::Imap {
                imap_host: "mail.mailo.com",
                imap_port: 993,
                imap_security: Security::Tls,
                smtp_host: "mail.mailo.com",
                smtp_port: 465,
                smtp_security: Security::Tls,
            },
            None,
        )],
        default_auth: RegistryAuthConfig {
            primary: RegistryAuth::Password,
            alternatives: &[],
        },
    },
];

/// Known OAuth providers for autoconfig XML that declares OAuth2.
static OAUTH_PROVIDERS: &[(&str, &RegistryAuth)] = &[
    ("google.com", &GOOGLE_OAUTH),
    ("gmail.com", &GOOGLE_OAUTH),
    ("outlook.com", &MICROSOFT_IMAP_OAUTH),
    ("hotmail.com", &MICROSOFT_IMAP_OAUTH),
    ("live.com", &MICROSOFT_IMAP_OAUTH),
    ("yahoo.com", &YAHOO_OAUTH),
    ("ymail.com", &YAHOO_OAUTH),
];

fn auth_method_from_registry(auth: &RegistryAuth) -> AuthMethod {
    match auth {
        RegistryAuth::Password => AuthMethod::Password,
        RegistryAuth::OAuth2 {
            provider_id,
            auth_url,
            token_url,
            scopes,
            use_pkce,
        } => AuthMethod::OAuth2 {
            provider_id: (*provider_id).to_string(),
            auth_url: (*auth_url).to_string(),
            token_url: (*token_url).to_string(),
            scopes: scopes.iter().map(|s| (*s).to_string()).collect(),
            use_pkce: *use_pkce,
        },
    }
}

fn build_auth_config(cfg: &RegistryAuthConfig) -> AuthConfig {
    AuthConfig {
        method: auth_method_from_registry(&cfg.primary),
        alternatives: cfg
            .alternatives
            .iter()
            .map(auth_method_from_registry)
            .collect(),
    }
}

fn protocol_from_registry(proto: &RegistryProtocol) -> Protocol {
    match proto {
        RegistryProtocol::GmailApi => Protocol::GmailApi,
        RegistryProtocol::MicrosoftGraph => Protocol::MicrosoftGraph,
        RegistryProtocol::Jmap { session_url } => Protocol::Jmap {
            session_url: (*session_url).to_string(),
        },
        RegistryProtocol::Imap {
            imap_host,
            imap_port,
            imap_security,
            smtp_host,
            smtp_port,
            smtp_security,
        } => Protocol::Imap {
            incoming: ServerConfig {
                hostname: (*imap_host).to_string(),
                port: *imap_port,
                security: *imap_security,
                username: UsernameFormat::EmailAddress,
            },
            outgoing: ServerConfig {
                hostname: (*smtp_host).to_string(),
                port: *smtp_port,
                security: *smtp_security,
                username: UsernameFormat::EmailAddress,
            },
        },
    }
}

/// Stage 1: Look up the hardcoded provider registry.
pub fn lookup(domain: &str) -> Vec<ProtocolOption> {
    let lower = domain.to_lowercase();
    for entry in REGISTRY {
        if entry.domains.iter().any(|d| *d == lower) {
            return entry
                .options
                .iter()
                .map(|(proto, auth_override)| {
                    let auth = auth_override
                        .as_ref()
                        .map_or_else(|| build_auth_config(&entry.default_auth), build_auth_config);
                    ProtocolOption {
                        protocol: protocol_from_registry(proto),
                        auth,
                        provider_name: Some(entry.name.to_string()),
                        help_url: entry.help_url.map(str::to_string),
                        source: DiscoverySource::Registry,
                    }
                })
                .collect();
        }
    }
    Vec::new()
}

/// Look up OAuth endpoints for a domain (used by autoconfig when XML says OAuth2).
pub fn lookup_oauth_for_domain(domain: &str) -> Option<AuthMethod> {
    let lower = domain.to_lowercase();
    for &(pattern, auth) in OAUTH_PROVIDERS {
        if lower == pattern || lower.ends_with(&format!(".{pattern}")) {
            return Some(auth_method_from_registry(auth));
        }
    }
    None
}
