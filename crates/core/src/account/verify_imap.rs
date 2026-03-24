//! IMAP credential verification for account setup.
//!
//! Wraps the IMAP crate's types so the app doesn't need direct provider access.

/// Verify IMAP credentials by connecting and immediately disconnecting.
///
/// Returns `Ok(())` if the connection succeeds, or an error message.
pub async fn verify_imap_credentials(
    host: &str,
    port: u16,
    security: &str,
    username: &str,
    password: &str,
    accept_invalid_certs: bool,
) -> Result<(), String> {
    let config = ratatoskr_imap::types::ImapConfig {
        host: host.to_string(),
        port,
        security: security.to_string(),
        username: username.to_string(),
        password: password.to_string(),
        auth_method: "password".to_string(),
        accept_invalid_certs,
    };

    let session = ratatoskr_imap::connection::connect(&config).await?;
    drop(session);
    Ok(())
}
