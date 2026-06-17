//! MDN (read receipt) support.
//!
//! Policy lookup and sent-flag tracking live in `db::queries_extra::mdn`.
//! This module keeps MIME message building and IMAP/JMAP protocol calls.

// ---------------------------------------------------------------------------
// MDN message builder (RFC 8098)
// ---------------------------------------------------------------------------

/// Build a `multipart/report; report-type=disposition-notification` message
/// (RFC 8098) and return the raw MIME bytes ready to send.
///
/// * `original_from`       - the address that asked for the receipt (goes in `To:`)
/// * `original_message_id` - the `Message-ID` of the original message
/// * `recipient_email`     - our email address (the one confirming reading)
/// * `recipient_name`      - display name for our `From:` header
/// * `is_manual`           - `true` → `manual-action/MDN-sent-manually`,
///   `false` → `automatic-action/MDN-sent-automatically`
pub fn build_mdn_message(
    original_from: &str,
    original_message_id: &str,
    recipient_email: &str,
    recipient_name: &str,
    is_manual: bool,
) -> Vec<u8> {
    let boundary = format!("mdn-boundary-{}", uuid::Uuid::new_v4());
    let message_id = format!("<{}.mdn@ratatoskr>", uuid::Uuid::new_v4());
    let date = chrono::Utc::now().format("%a, %d %b %Y %H:%M:%S +0000");

    let disposition_mode = if is_manual {
        "manual-action/MDN-sent-manually"
    } else {
        "automatic-action/MDN-sent-automatically"
    };

    // Human‑readable part
    let human_part = format!(
        "This is a return receipt for the message you sent to {recipient_email}.\r\n\
         \r\n\
         Note: This return receipt only acknowledges that the message was displayed \
         on the recipient's computer. There is no guarantee that the recipient has \
         read or understood the message contents.\r\n"
    );

    // Machine‑readable disposition notification (RFC 8098 §3.2.6)
    let machine_part = format!(
        "Reporting-UA: Ratatoskr\r\n\
         Final-Recipient: rfc822;{recipient_email}\r\n\
         Original-Message-ID: {original_message_id}\r\n\
         Disposition: {disposition_mode};displayed\r\n"
    );

    let from_header = if recipient_name.is_empty() {
        format!("<{recipient_email}>")
    } else {
        format!("{recipient_name} <{recipient_email}>")
    };

    // Assemble the full MIME message
    let raw = format!(
        "From: {from_header}\r\n\
         To: <{original_from}>\r\n\
         Subject: Read: Return receipt\r\n\
         Date: {date}\r\n\
         Message-ID: {message_id}\r\n\
         MIME-Version: 1.0\r\n\
         Content-Type: multipart/report; report-type=disposition-notification; boundary=\"{boundary}\"\r\n\
         \r\n\
         --{boundary}\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         Content-Transfer-Encoding: 7bit\r\n\
         \r\n\
         {human_part}\r\n\
         --{boundary}\r\n\
         Content-Type: message/disposition-notification\r\n\
         Content-Transfer-Encoding: 7bit\r\n\
         \r\n\
         {machine_part}\r\n\
         --{boundary}--\r\n"
    );

    raw.into_bytes()
}

// Server-side `$MDNSent` / `$mdnsent` keyword sync now lives behind
// `ProviderOps::mark_mdn_sent` in each provider's `ops.rs`. Gmail and
// Graph have no equivalent (Graph's `isReadReceiptRequested` is
// read-only); their default trait impl is a no-op.

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // DB policy + MDN-sent tracking tests now live in db::queries_extra::mdn.

    #[test]
    fn test_build_mdn_manual() {
        let raw = build_mdn_message(
            "sender@example.com",
            "<original-id@example.com>",
            "me@myhost.com",
            "My Name",
            true,
        );
        let text = String::from_utf8(raw).expect("valid utf8");
        assert!(text.contains("From: My Name <me@myhost.com>"));
        assert!(text.contains("To: <sender@example.com>"));
        assert!(text.contains("multipart/report"));
        assert!(text.contains("report-type=disposition-notification"));
        assert!(text.contains("Reporting-UA: Ratatoskr"));
        assert!(text.contains("Final-Recipient: rfc822;me@myhost.com"));
        assert!(text.contains("Original-Message-ID: <original-id@example.com>"));
        assert!(text.contains("manual-action/MDN-sent-manually;displayed"));
    }

    #[test]
    fn test_build_mdn_automatic() {
        let raw = build_mdn_message(
            "sender@example.com",
            "<original-id@example.com>",
            "me@myhost.com",
            "",
            false,
        );
        let text = String::from_utf8(raw).expect("valid utf8");
        assert!(text.contains("From: <me@myhost.com>"));
        assert!(text.contains("automatic-action/MDN-sent-automatically;displayed"));
    }
}
