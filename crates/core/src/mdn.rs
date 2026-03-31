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
    if let Ok(Some(value)) = crate::db::get_setting(conn, "default_read_receipt_policy") {
        return ReadReceiptPolicy::from_str(&value);
    }

    // 5. Hard‑coded fallback
    ReadReceiptPolicy::Never
}

/// Helper: look up a single policy row.
fn query_policy(conn: &Connection, account_id: &str, scope: &str) -> Option<ReadReceiptPolicy> {
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
// $MDNSent keyword management
// ---------------------------------------------------------------------------

/// Check whether an MDN has already been sent for a message, using the local
/// `mdn_sent` flag in the database.
///
/// This is provider-agnostic: regardless of whether the provider supports
/// the `$MDNSent` keyword natively, we always track it locally.
pub fn is_mdn_already_sent(conn: &Connection, account_id: &str, message_id: &str) -> bool {
    conn.query_row(
        "SELECT mdn_sent FROM messages WHERE account_id = ?1 AND id = ?2",
        params![account_id, message_id],
        |row| row.get::<_, bool>(0),
    )
    .unwrap_or(false)
}

/// Mark an MDN as sent in the local database.
///
/// This should be called after successfully sending the MDN (or after
/// setting the provider-side keyword). It sets the `mdn_sent` flag so
/// we never send a duplicate, even if the provider doesn't support
/// custom keywords.
pub fn mark_mdn_sent_local(
    conn: &Connection,
    account_id: &str,
    message_id: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE messages SET mdn_sent = 1 WHERE account_id = ?1 AND id = ?2",
        params![account_id, message_id],
    )
    .map_err(|e| format!("mark mdn_sent: {e}"))?;
    Ok(())
}

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
/// Checks PERMANENTFLAGS first — if the server does not support custom
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
/// returns `false` — the caller should fall back to the local DB check.
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
/// side — there is no server-side keyword to set. We only track MDN sent
/// status locally via [`mark_mdn_sent_local`].
///
/// This function checks whether the original message had
/// `isReadReceiptRequested` set, using the `mdn_requested` column that
/// was populated during sync.
pub fn is_mdn_requested_graph(conn: &Connection, account_id: &str, message_id: &str) -> bool {
    conn.query_row(
        "SELECT mdn_requested FROM messages WHERE account_id = ?1 AND id = ?2",
        params![account_id, message_id],
        |row| row.get::<_, bool>(0),
    )
    .unwrap_or(false)
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
            "CREATE TABLE IF NOT EXISTS accounts (
                id TEXT PRIMARY KEY,
                email TEXT NOT NULL,
                provider TEXT NOT NULL DEFAULT 'gmail_api'
            );
            CREATE TABLE IF NOT EXISTS messages (
                id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                thread_id TEXT,
                mdn_requested INTEGER NOT NULL DEFAULT 0,
                mdn_sent INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (account_id, id)
            );
            CREATE TABLE IF NOT EXISTS read_receipt_policy (
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

    // ── MDN sent tracking tests ──────────────────────────────────

    fn insert_test_message(conn: &Connection, account_id: &str, msg_id: &str, mdn_requested: bool) {
        conn.execute(
            "INSERT INTO messages (id, account_id, thread_id, mdn_requested, mdn_sent) \
             VALUES (?1, ?2, 'thread1', ?3, 0)",
            params![msg_id, account_id, mdn_requested],
        )
        .expect("insert message");
    }

    #[test]
    fn test_is_mdn_already_sent_false_by_default() {
        let conn = setup_db();
        insert_test_message(&conn, "acct1", "msg1", true);
        assert!(!is_mdn_already_sent(&conn, "acct1", "msg1"));
    }

    #[test]
    fn test_mark_mdn_sent_local() {
        let conn = setup_db();
        insert_test_message(&conn, "acct1", "msg1", true);
        assert!(!is_mdn_already_sent(&conn, "acct1", "msg1"));

        mark_mdn_sent_local(&conn, "acct1", "msg1").expect("mark sent");
        assert!(is_mdn_already_sent(&conn, "acct1", "msg1"));
    }

    #[test]
    fn test_is_mdn_already_sent_missing_message() {
        let conn = setup_db();
        // Nonexistent message should return false, not error
        assert!(!is_mdn_already_sent(&conn, "acct1", "nonexistent"));
    }

    #[test]
    fn test_is_mdn_requested_graph() {
        let conn = setup_db();
        insert_test_message(&conn, "acct1", "msg1", true);
        assert!(is_mdn_requested_graph(&conn, "acct1", "msg1"));

        insert_test_message(&conn, "acct1", "msg2", false);
        assert!(!is_mdn_requested_graph(&conn, "acct1", "msg2"));
    }

    #[test]
    fn test_mark_mdn_sent_idempotent() {
        let conn = setup_db();
        insert_test_message(&conn, "acct1", "msg1", true);
        mark_mdn_sent_local(&conn, "acct1", "msg1").expect("first mark");
        mark_mdn_sent_local(&conn, "acct1", "msg1").expect("second mark");
        assert!(is_mdn_already_sent(&conn, "acct1", "msg1"));
    }
}
