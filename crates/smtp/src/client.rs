use lettre::{
    AsyncSmtpTransport, AsyncTransport, Tokio1Executor,
    transport::smtp::{
        authentication::{Credentials, Mechanism},
        client::{Tls, TlsParametersBuilder},
    },
};

use super::types::{SmtpConfig, SmtpSendResult};

/// Decode a base64url-encoded string (Gmail format) to raw bytes.
fn decode_base64url(input: &str) -> Result<Vec<u8>, String> {
    common::encoding::decode_base64url_nopad(input).map_err(|e| format!("Base64 decode error: {e}"))
}

/// Build an async SMTP transport from the given config.
fn build_transport(config: &SmtpConfig) -> Result<AsyncSmtpTransport<Tokio1Executor>, String> {
    log::debug!(
        "Building SMTP transport: host={}:{}, security={}, auth={}",
        config.host,
        config.port,
        config.security,
        config.auth_method
    );
    let credentials = Credentials::new(config.username.clone(), config.password.clone());

    // For OAuth2, force XOAUTH2 mechanism; for password, use default mechanisms
    let auth_mechanisms = if config.auth_method == "oauth2" {
        vec![Mechanism::Xoauth2]
    } else {
        vec![Mechanism::Plain, Mechanism::Login]
    };

    let transport = match config.security.as_str() {
        "tls" => {
            // Implicit TLS (typically port 465)
            let mut builder = AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host)
                .map_err(|e| format!("SMTP relay error: {e}"))?
                .port(config.port)
                .credentials(credentials)
                .authentication(auth_mechanisms);

            if config.accept_invalid_certs {
                let tls_params = TlsParametersBuilder::new(config.host.clone())
                    .dangerous_accept_invalid_certs(true)
                    .build()
                    .map_err(|e| format!("SMTP TLS params error: {e}"))?;
                builder = builder.tls(Tls::Required(tls_params));
            }

            builder.build()
        }
        "starttls" => {
            // STARTTLS (typically port 587)
            let mut builder = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)
                .map_err(|e| format!("SMTP STARTTLS error: {e}"))?
                .port(config.port)
                .credentials(credentials)
                .authentication(auth_mechanisms);

            if config.accept_invalid_certs {
                let tls_params = TlsParametersBuilder::new(config.host.clone())
                    .dangerous_accept_invalid_certs(true)
                    .build()
                    .map_err(|e| format!("SMTP TLS params error: {e}"))?;
                builder = builder.tls(Tls::Required(tls_params));
            }

            builder.build()
        }
        _ => {
            // Plain / no encryption (typically port 25) - not recommended
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host)
                .port(config.port)
                .credentials(credentials)
                .authentication(auth_mechanisms)
                .build()
        }
    };

    Ok(transport)
}

/// Extract an SMTP envelope (sender + recipients) from raw RFC 2822 bytes.
///
/// The envelope tells the SMTP server who the mail is from and who to deliver
/// it to, which is separate from the header fields visible to the recipient.
fn extract_envelope(raw: &[u8]) -> Result<lettre::address::Envelope, String> {
    let message = mail_parser::MessageParser::default()
        .parse(raw)
        .ok_or("Failed to parse email for envelope extraction")?;

    // Extract From address
    let from = message
        .from()
        .and_then(|list| list.first())
        .and_then(|addr| addr.address())
        .ok_or("No From address found in email")?;

    let from_addr: lettre::Address = from
        .parse()
        .map_err(|e| format!("Invalid From address '{from}': {e}"))?;

    // Collect all recipient addresses (To, Cc, Bcc)
    let mut recipients: Vec<lettre::Address> = Vec::new();

    if let Some(to_list) = message.to() {
        for addr in to_list.iter() {
            if let Some(email) = addr.address()
                && let Ok(a) = email.parse::<lettre::Address>()
            {
                recipients.push(a);
            }
        }
    }

    if let Some(cc_list) = message.cc() {
        for addr in cc_list.iter() {
            if let Some(email) = addr.address()
                && let Ok(a) = email.parse::<lettre::Address>()
            {
                recipients.push(a);
            }
        }
    }

    if let Some(bcc_list) = message.bcc() {
        for addr in bcc_list.iter() {
            if let Some(email) = addr.address()
                && let Ok(a) = email.parse::<lettre::Address>()
            {
                recipients.push(a);
            }
        }
    }

    if recipients.is_empty() {
        return Err("No recipients found in email".to_string());
    }

    lettre::address::Envelope::new(Some(from_addr), recipients)
        .map_err(|e| format!("Envelope error: {e}"))
}

/// Send a pre-built RFC 2822 email via SMTP.
///
/// The `raw_email_base64url` parameter is the full email message encoded as
/// base64url (the same encoding Gmail uses: `+` → `-`, `/` → `_`, no padding).
/// The function decodes it, extracts the envelope from headers, and sends it.
pub async fn send_raw_email(
    config: &SmtpConfig,
    raw_email_base64url: &str,
) -> Result<SmtpSendResult, String> {
    let raw_bytes = decode_base64url(raw_email_base64url)?;
    let envelope = extract_envelope(&raw_bytes)?;
    log::info!(
        "Sending email via SMTP {}:{} from={:?} to={} recipients",
        config.host,
        config.port,
        envelope.from(),
        envelope.to().len()
    );
    let transport = build_transport(config)?;

    transport
        .send_raw(&envelope, &raw_bytes)
        .await
        .map(|_response| {
            log::info!("Email sent successfully via SMTP");
            SmtpSendResult {
                success: true,
                message: "Email sent successfully".to_string(),
            }
        })
        .map_err(|e| {
            log::error!("SMTP send failed: {e}");
            format!("SMTP send error: {e}")
        })
}

/// Test SMTP connectivity by connecting, authenticating, and disconnecting.
pub async fn test_connection(config: &SmtpConfig) -> Result<SmtpSendResult, String> {
    log::info!(
        "Testing SMTP connection to {}:{} (security={})",
        config.host,
        config.port,
        config.security
    );
    let transport = build_transport(config)?;

    transport
        .test_connection()
        .await
        .map(|success| {
            if success {
                log::info!("SMTP connection test successful");
            } else {
                log::error!("SMTP connection test failed");
            }
            SmtpSendResult {
                success,
                message: if success {
                    "Connection successful".to_string()
                } else {
                    "Connection failed".to_string()
                },
            }
        })
        .map_err(|e| {
            log::error!("SMTP connection test error: {e}");
            format!("SMTP test error: {e}")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_base64url_valid() {
        // "Hello" in base64url
        let encoded = "SGVsbG8";
        let decoded = decode_base64url(encoded).expect("valid base64url should decode");
        assert_eq!(decoded, b"Hello");
    }

    #[test]
    fn test_decode_base64url_invalid() {
        let result = decode_base64url("!!!invalid!!!");
        assert!(result.is_err());
        assert!(
            result
                .expect_err("invalid base64url should fail")
                .contains("Base64 decode error")
        );
    }

    #[test]
    fn test_extract_envelope_valid() {
        let raw = b"From: alice@example.com\r\nTo: bob@example.com\r\nCc: carol@example.com\r\nSubject: Test\r\n\r\nBody";
        let envelope = extract_envelope(raw).expect("valid envelope should parse");
        // Envelope should have from and 2 recipients (To + Cc)
        assert!(envelope.from().is_some());
        assert_eq!(envelope.to().len(), 2);
    }

    #[test]
    fn test_extract_envelope_no_from() {
        let raw = b"To: bob@example.com\r\nSubject: Test\r\n\r\nBody";
        let result = extract_envelope(raw);
        assert!(result.is_err());
        assert!(
            result
                .expect_err("missing from should fail")
                .contains("No From address")
        );
    }

    #[test]
    fn test_extract_envelope_no_recipients() {
        let raw = b"From: alice@example.com\r\nSubject: Test\r\n\r\nBody";
        let result = extract_envelope(raw);
        assert!(result.is_err());
        assert!(
            result
                .expect_err("missing recipients should fail")
                .contains("No recipients found")
        );
    }

    #[test]
    fn test_extract_envelope_with_bcc() {
        let raw = b"From: alice@example.com\r\nTo: bob@example.com\r\nBcc: secret@example.com\r\nSubject: Test\r\n\r\nBody";
        let envelope = extract_envelope(raw).expect("bcc recipient should parse");
        assert_eq!(envelope.to().len(), 2);
    }
}
