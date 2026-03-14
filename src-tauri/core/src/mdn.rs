use rusqlite::{Connection, params};

// ---------------------------------------------------------------------------
// Read‑receipt policy resolution
// ---------------------------------------------------------------------------

/// Policy that governs whether an MDN (read receipt) should be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadReceiptPolicy {
    /// Always send a read receipt automatically.
    Always,
    /// Ask the user before sending.
    Ask,
    /// Never send a read receipt (default).
    Never,
}

impl ReadReceiptPolicy {
    fn from_str(s: &str) -> Self {
        match s {
            "always" => Self::Always,
            "ask" => Self::Ask,
            _ => Self::Never,
        }
    }
}

/// Resolve the effective read‑receipt policy for a given sender, using
/// most‑specific‑wins lookup order:
///
/// 1. `sender:{exact_email}`
/// 2. `domain:{domain}`
/// 3. Account‑level policy (scope = `account`)
/// 4. Global default from the `settings` table (`default_read_receipt_policy`)
/// 5. Hard‑coded fallback: `Never`
pub fn resolve_read_receipt_policy(
    conn: &Connection,
    account_id: &str,
    sender_email: &str,
) -> ReadReceiptPolicy {
    let sender_email_lower = sender_email.to_lowercase();

    // 1. Exact sender match
    let sender_scope = format!("sender:{sender_email_lower}");
    if let Some(policy) = query_policy(conn, account_id, &sender_scope) {
        return policy;
    }

    // 2. Domain match
    if let Some(domain) = sender_email_lower.split('@').nth(1) {
        let domain_scope = format!("domain:{domain}");
        if let Some(policy) = query_policy(conn, account_id, &domain_scope) {
            return policy;
        }
    }

    // 3. Account‑level policy
    if let Some(policy) = query_policy(conn, account_id, "account") {
        return policy;
    }

    // 4. Global default from settings table
    if let Ok(Some(value)) = conn.query_row(
        "SELECT value FROM settings WHERE key = 'default_read_receipt_policy'",
        [],
        |row| row.get::<_, String>(0),
    ).map(Some).or_else(|e| {
        if e == rusqlite::Error::QueryReturnedNoRows {
            Ok(None)
        } else {
            Err(e)
        }
    }) {
        return ReadReceiptPolicy::from_str(&value);
    }

    // 5. Hard‑coded fallback
    ReadReceiptPolicy::Never
}

/// Helper: look up a single policy row.
fn query_policy(
    conn: &Connection,
    account_id: &str,
    scope: &str,
) -> Option<ReadReceiptPolicy> {
    conn.query_row(
        "SELECT policy FROM read_receipt_policy WHERE account_id = ?1 AND scope = ?2",
        params![account_id, scope],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .map(|v| ReadReceiptPolicy::from_str(&v))
}

// ---------------------------------------------------------------------------
// MDN message builder (RFC 8098)
// ---------------------------------------------------------------------------

/// Build a `multipart/report; report-type=disposition-notification` message
/// (RFC 8098) and return the raw MIME bytes ready to send.
///
/// * `original_from`       – the address that asked for the receipt (goes in `To:`)
/// * `original_message_id` – the `Message-ID` of the original message
/// * `recipient_email`     – our email address (the one confirming reading)
/// * `recipient_name`      – display name for our `From:` header
/// * `is_manual`           – `true` → `manual-action/MDN-sent-manually`,
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// Set up an in‑memory DB with the required tables.
    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS read_receipt_policy (
                id TEXT PRIMARY KEY,
                account_id TEXT NOT NULL,
                scope TEXT NOT NULL,
                policy TEXT NOT NULL DEFAULT 'never',
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                UNIQUE(account_id, scope)
            );
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            INSERT INTO settings (key, value) VALUES ('default_read_receipt_policy', 'never');",
        )
        .expect("create tables");
        conn
    }

    #[test]
    fn test_default_policy_is_never() {
        let conn = setup_db();
        let policy = resolve_read_receipt_policy(&conn, "acct1", "sender@example.com");
        assert_eq!(policy, ReadReceiptPolicy::Never);
    }

    #[test]
    fn test_global_default_override() {
        let conn = setup_db();
        conn.execute(
            "UPDATE settings SET value = 'ask' WHERE key = 'default_read_receipt_policy'",
            [],
        )
        .expect("update setting");
        let policy = resolve_read_receipt_policy(&conn, "acct1", "sender@example.com");
        assert_eq!(policy, ReadReceiptPolicy::Ask);
    }

    #[test]
    fn test_account_level_policy() {
        let conn = setup_db();
        conn.execute(
            "INSERT INTO read_receipt_policy (id, account_id, scope, policy) VALUES ('1', 'acct1', 'account', 'always')",
            [],
        )
        .expect("insert");
        let policy = resolve_read_receipt_policy(&conn, "acct1", "sender@example.com");
        assert_eq!(policy, ReadReceiptPolicy::Always);
    }

    #[test]
    fn test_domain_beats_account() {
        let conn = setup_db();
        conn.execute_batch(
            "INSERT INTO read_receipt_policy (id, account_id, scope, policy) VALUES ('1', 'acct1', 'account', 'never');
             INSERT INTO read_receipt_policy (id, account_id, scope, policy) VALUES ('2', 'acct1', 'domain:example.com', 'ask');",
        )
        .expect("insert");
        let policy = resolve_read_receipt_policy(&conn, "acct1", "sender@example.com");
        assert_eq!(policy, ReadReceiptPolicy::Ask);
    }

    #[test]
    fn test_sender_beats_domain() {
        let conn = setup_db();
        conn.execute_batch(
            "INSERT INTO read_receipt_policy (id, account_id, scope, policy) VALUES ('1', 'acct1', 'domain:example.com', 'never');
             INSERT INTO read_receipt_policy (id, account_id, scope, policy) VALUES ('2', 'acct1', 'sender:boss@example.com', 'always');",
        )
        .expect("insert");
        let policy = resolve_read_receipt_policy(&conn, "acct1", "boss@example.com");
        assert_eq!(policy, ReadReceiptPolicy::Always);
    }

    #[test]
    fn test_case_insensitive_sender() {
        let conn = setup_db();
        conn.execute(
            "INSERT INTO read_receipt_policy (id, account_id, scope, policy) VALUES ('1', 'acct1', 'sender:boss@example.com', 'always')",
            [],
        )
        .expect("insert");
        let policy = resolve_read_receipt_policy(&conn, "acct1", "Boss@Example.COM");
        assert_eq!(policy, ReadReceiptPolicy::Always);
    }

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
