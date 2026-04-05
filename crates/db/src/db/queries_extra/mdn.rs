//! MDN (read receipt) policy lookup and sent-flag tracking.

use rusqlite::{Connection, params};

/// Policy that governs whether an MDN (read receipt) should be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadReceiptPolicy {
    Always,
    Ask,
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

/// Resolve the effective read-receipt policy for a given sender.
///
/// Most-specific-wins lookup order:
/// 1. `sender:{exact_email}`
/// 2. `domain:{domain}`
/// 3. Account-level policy (scope = `account`)
/// 4. Global default from the `settings` table (`default_read_receipt_policy`)
/// 5. Hard-coded fallback: `Never`
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

    // 3. Account-level policy
    if let Some(policy) = query_policy(conn, account_id, "account") {
        return policy;
    }

    // 4. Global default from settings table
    if let Ok(Some(value)) = super::super::queries::get_setting(conn, "default_read_receipt_policy")
    {
        return ReadReceiptPolicy::from_str(&value);
    }

    // 5. Hard-coded fallback
    ReadReceiptPolicy::Never
}

fn query_policy(conn: &Connection, account_id: &str, scope: &str) -> Option<ReadReceiptPolicy> {
    conn.query_row(
        "SELECT policy FROM read_receipt_policy WHERE account_id = ?1 AND scope = ?2",
        params![account_id, scope],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .map(|v| ReadReceiptPolicy::from_str(&v))
}

/// Check whether an MDN has already been sent for a message.
pub fn is_mdn_already_sent(conn: &Connection, account_id: &str, message_id: &str) -> bool {
    conn.query_row(
        "SELECT mdn_sent FROM messages WHERE account_id = ?1 AND id = ?2",
        params![account_id, message_id],
        |row| row.get::<_, bool>(0),
    )
    .unwrap_or(false)
}

/// Mark an MDN as sent in the local database.
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

/// Check whether `mdn_requested` is set for a Graph message.
pub fn is_mdn_requested_graph(conn: &Connection, account_id: &str, message_id: &str) -> bool {
    conn.query_row(
        "SELECT mdn_requested FROM messages WHERE account_id = ?1 AND id = ?2",
        params![account_id, message_id],
        |row| row.get::<_, bool>(0),
    )
    .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Tests (policy + MDN-sent tracking)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

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

    fn insert_test_message(
        conn: &Connection,
        account_id: &str,
        msg_id: &str,
        mdn_requested: bool,
    ) {
        conn.execute(
            "INSERT INTO messages (id, account_id, thread_id, mdn_requested, mdn_sent) \
             VALUES (?1, ?2, 'thread1', ?3, 0)",
            rusqlite::params![msg_id, account_id, mdn_requested],
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
        mark_mdn_sent_local(&conn, "acct1", "msg1").expect("mark sent");
        assert!(is_mdn_already_sent(&conn, "acct1", "msg1"));
    }

    #[test]
    fn test_is_mdn_already_sent_missing_message() {
        let conn = setup_db();
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
