//! MDN (read receipt) support.
//!
//! Policy lookup and sent-flag tracking live in `db::queries_extra::mdn`.
//! This module keeps MIME message building and IMAP/JMAP protocol calls.

// Re-export storage functions and types.
pub use crate::db::queries_extra::{
    ReadReceiptPolicy, is_mdn_already_sent, is_mdn_requested_graph, mark_mdn_sent_local,
    resolve_read_receipt_policy,
};

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
///                           `false` → `automatic-action/MDN-sent-automatically`
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

// ---------------------------------------------------------------------------
// $MDNSent keyword management
// ---------------------------------------------------------------------------


/// Set the `$mdnsent` keyword on a JMAP message via `Email/set`.
///
/// JMAP keywords are case-sensitive; the canonical form is lowercase `$mdnsent`.
pub async fn mark_mdn_sent_jmap(
    client: &jmap_client::client::Client,
    message_id: &str,
) -> Result<(), String> {
    let account_id = client.default_account_id().to_string();
    let mut email_set = jmap_client::email::EmailSet::new(&account_id);
    email_set.update(message_id).keyword("$mdnsent", true);
    let mut request = client.build();
    let handle = request
        .call(email_set)
        .map_err(|e| format!("JMAP Email/set $mdnsent build: {e}"))?;
    let mut response = request
        .send()
        .await
        .map_err(|e| format!("JMAP Email/set $mdnsent send: {e}"))?;
    response
        .get(&handle)
        .map_err(|e| format!("JMAP Email/set $mdnsent: {e}"))?;
    Ok(())
}

/// Set the `$MDNSent` keyword on an IMAP message via `UID STORE +FLAGS`.
///
/// Checks PERMANENTFLAGS first - if the server does not support custom
/// keywords (`\*` not in PERMANENTFLAGS), this is a silent no-op.
/// The caller should always also call [`mark_mdn_sent_local`] to ensure
/// local tracking regardless of server support.
pub async fn mark_mdn_sent_imap(
    session: &mut crate::imap::connection::ImapSession,
    folder: &str,
    uid: u32,
) -> Result<(), String> {
    crate::imap::client::set_keyword_if_supported(session, folder, uid, "+FLAGS", "$MDNSent").await
}

/// Check whether the `$MDNSent` keyword is already set on an IMAP message.
///
/// Performs a `UID SEARCH KEYWORD $MDNSent` scoped to the single UID.
/// Returns `true` if the server reports the keyword is present.
///
/// If the search fails (e.g. server doesn't support custom keywords),
/// returns `false` - the caller should fall back to the local DB check.
pub async fn is_mdn_sent_imap(
    session: &mut crate::imap::connection::ImapSession,
    folder: &str,
    uid: u32,
) -> bool {
    use crate::imap::connection::IMAP_CMD_TIMEOUT;

    // SELECT the folder first
    if tokio::time::timeout(IMAP_CMD_TIMEOUT, session.select(folder))
        .await
        .is_err()
    {
        return false;
    }

    let query = format!("UID {uid} KEYWORD $MDNSent");
    let result = tokio::time::timeout(IMAP_CMD_TIMEOUT, session.uid_search(&query)).await;
    match result {
        Ok(Ok(uids)) => uids.contains(&uid),
        _ => false,
    }
}

/// For Graph (Microsoft), `isReadReceiptRequested` is read-only on the API
/// side - there is no server-side keyword to set. We only track MDN sent
/// status locally via [`mark_mdn_sent_local`].
///

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
