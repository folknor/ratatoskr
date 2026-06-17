//! IT-distributable admin config: pre-fill values for the add-account wizard.
//!
//! Lives at `~/.config/ratatoskr/config.toml` (or platform equivalent via
//! `dirs::config_dir()`). All fields optional; the user can override
//! anything at the wizard. Best-effort load: a missing or malformed file
//! never blocks app start, just degrades to no pre-fill.

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AdminConfig {
    #[serde(default)]
    pub oidc: Option<OidcDefaults>,
    #[serde(default)]
    pub imap: Option<ServerDefaults>,
    #[serde(default)]
    pub smtp: Option<ServerDefaults>,
    #[serde(default)]
    pub jmap: Option<JmapDefaults>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct OidcDefaults {
    pub issuer_url: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    /// Space-separated extra OAuth scopes appended to the negotiated set.
    pub extra_scopes: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ServerDefaults {
    pub host: Option<String>,
    pub port: Option<u16>,
    /// `tls` / `starttls` / `none`. Tolerantly parsed at pre-fill time;
    /// unknown values log a warning and pre-fill the security field as
    /// the wizard's default (StartTls).
    pub security: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct JmapDefaults {
    pub url: Option<String>,
}

/// Resolved path: `~/.config/ratatoskr/config.toml` on Linux, the
/// platform equivalent on macOS / Windows. `None` when no config dir is
/// resolvable for the current user (rare).
pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("ratatoskr").join("config.toml"))
}

/// Load the admin config from disk. Returns `None` for any failure mode
/// (missing file, permission error, parse error). Parse errors log a
/// warning so admins can debug malformed TOML; missing files are silent.
pub fn load() -> Option<AdminConfig> {
    let path = config_path()?;
    let contents = std::fs::read_to_string(&path).ok()?;
    match toml::from_str::<AdminConfig>(&contents) {
        Ok(config) => {
            log::info!("Loaded admin config from {}", path.display());
            Some(config)
        }
        Err(e) => {
            log::warn!(
                "Failed to parse admin config at {}: {e}; ignoring",
                path.display()
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_complete_config() {
        let toml_str = r#"
[oidc]
issuer_url = "https://auth.corp.example"
client_id = "ratatoskr-corp"
client_secret = "shh"
extra_scopes = "imap-access offline_access"

[imap]
host = "imap.corp.example"
port = 993
security = "tls"

[smtp]
host = "smtp.corp.example"
port = 587
security = "starttls"

[jmap]
url = "https://jmap.corp.example/.well-known/jmap"
"#;
        let cfg: AdminConfig = toml::from_str(toml_str).expect("valid TOML");
        let oidc = cfg.oidc.expect("oidc table");
        assert_eq!(
            oidc.issuer_url.as_deref(),
            Some("https://auth.corp.example")
        );
        assert_eq!(oidc.client_id.as_deref(), Some("ratatoskr-corp"));
        assert_eq!(oidc.client_secret.as_deref(), Some("shh"));
        assert_eq!(
            oidc.extra_scopes.as_deref(),
            Some("imap-access offline_access")
        );

        let imap = cfg.imap.expect("imap table");
        assert_eq!(imap.host.as_deref(), Some("imap.corp.example"));
        assert_eq!(imap.port, Some(993));
        assert_eq!(imap.security.as_deref(), Some("tls"));

        let smtp = cfg.smtp.expect("smtp table");
        assert_eq!(smtp.security.as_deref(), Some("starttls"));

        let jmap = cfg.jmap.expect("jmap table");
        assert_eq!(
            jmap.url.as_deref(),
            Some("https://jmap.corp.example/.well-known/jmap")
        );
    }

    #[test]
    fn parses_partial_config() {
        // Only OIDC; the other sub-tables are absent.
        let toml_str = r#"
[oidc]
issuer_url = "https://auth.corp.example"
"#;
        let cfg: AdminConfig = toml::from_str(toml_str).expect("valid TOML");
        assert!(cfg.imap.is_none());
        assert!(cfg.smtp.is_none());
        assert!(cfg.jmap.is_none());
        let oidc = cfg.oidc.expect("oidc table");
        assert_eq!(
            oidc.issuer_url.as_deref(),
            Some("https://auth.corp.example")
        );
        assert!(oidc.client_id.is_none());
        assert!(oidc.client_secret.is_none());
        assert!(oidc.extra_scopes.is_none());
    }

    #[test]
    fn parses_empty_string() {
        let cfg: AdminConfig = toml::from_str("").expect("empty is valid");
        assert!(cfg.oidc.is_none());
        assert!(cfg.imap.is_none());
        assert!(cfg.smtp.is_none());
        assert!(cfg.jmap.is_none());
    }

    #[test]
    fn rejects_malformed_toml() {
        // load() swallows this; here we exercise the parser layer
        // directly to make sure malformed input is an Err, not a panic.
        assert!(toml::from_str::<AdminConfig>("[oidc\nissuer_url =").is_err());
    }

    #[test]
    fn config_path_returns_some_on_supported_platforms() {
        // dirs::config_dir() returns None on platforms with no
        // convention; otherwise the joined path includes "ratatoskr".
        if let Some(path) = config_path() {
            assert!(path.to_string_lossy().contains("ratatoskr"));
            assert!(path.ends_with("config.toml"));
        }
    }
}
