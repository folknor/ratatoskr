use regex::Regex;
use rusqlite::Connection;

/// A mention found in the HTML body that matches a known mention from the database.
#[derive(Debug, Clone)]
pub struct MentionAnnotation {
    /// The email address of the mentioned person.
    pub email: String,
    /// The display name from the mentions table, if available.
    pub name: Option<String>,
    /// Byte offset of the `<a ...>` tag start within the HTML string.
    pub byte_offset: usize,
    /// Byte length from the `<a` tag start to the closing `</a>` (inclusive).
    pub byte_length: usize,
}

/// Scan `html` for `<a href="mailto:...">` links and cross-reference them against
/// the `mentions` table for `(message_id, account_id)`. Returns an annotation for
/// every mailto link whose address matches a stored mention.
pub fn correlate_mentions_in_html(
    html: &str,
    conn: &Connection,
    message_id: &str,
    account_id: &str,
) -> Result<Vec<MentionAnnotation>, rusqlite::Error> {
    // 1. Query known mentions for this message.
    let mut stmt = conn.prepare(
        "SELECT mentioned_address, mentioned_name FROM mentions \
         WHERE message_id = ?1 AND account_id = ?2",
    )?;

    let mut known: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    let mut rows = stmt.query(rusqlite::params![message_id, account_id])?;
    while let Some(row) = rows.next()? {
        let addr: String = row.get("mentioned_address")?;
        let name: Option<String> = row.get("mentioned_name")?;
        known.insert(addr.to_lowercase(), name);
    }

    if known.is_empty() {
        return Ok(Vec::new());
    }

    // 2. Find all <a href="mailto:...">...</a> spans in the HTML.
    //    The regex captures the full <a ...>...</a> tag and the email address.
    #[allow(clippy::unwrap_in_result)]
    let re = Regex::new(r#"(?i)<a\s[^>]*href\s*=\s*"mailto:([^"?]+)[^"]*"[^>]*>.*?</a>"#)
        .expect("static regex is valid");

    let mut annotations = Vec::new();

    for cap in re.captures_iter(html) {
        let full_match = cap.get(0).expect("capture group 0 always exists");
        let email_raw = &cap[1];
        let email_lower = email_raw.to_lowercase();

        if let Some(name) = known.get(&email_lower) {
            annotations.push(MentionAnnotation {
                email: email_lower,
                name: name.clone(),
                byte_offset: full_match.start(),
                byte_length: full_match.len(),
            });
        }
    }

    Ok(annotations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "CREATE TABLE mentions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                message_id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                mention_id TEXT,
                mentioned_name TEXT,
                mentioned_address TEXT NOT NULL,
                created_by_name TEXT,
                created_by_address TEXT,
                created_at INTEGER,
                UNIQUE(message_id, account_id, mentioned_address)
            );",
        )
        .expect("create table");
        conn
    }

    #[test]
    fn matches_mailto_against_mentions() {
        let conn = setup_db();
        conn.execute(
            "INSERT INTO mentions (message_id, account_id, mentioned_address, mentioned_name) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["msg1", "acct1", "alice@example.com", "Alice"],
        )
        .expect("insert");

        let html = r#"<p>Hey <a href="mailto:alice@example.com">Alice</a>, check this.</p>"#;
        let result =
            correlate_mentions_in_html(html, &conn, "msg1", "acct1").expect("correlate");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].email, "alice@example.com");
        assert_eq!(result[0].name.as_deref(), Some("Alice"));
        // Verify offset points to the <a tag
        assert_eq!(&html[result[0].byte_offset..][..2], "<a");
        // Verify length covers through </a>
        let end = result[0].byte_offset + result[0].byte_length;
        assert!(html[..end].ends_with("</a>"));
    }

    #[test]
    fn ignores_unmentioned_mailto() {
        let conn = setup_db();
        conn.execute(
            "INSERT INTO mentions (message_id, account_id, mentioned_address) \
             VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg1", "acct1", "alice@example.com"],
        )
        .expect("insert");

        let html = r#"<a href="mailto:bob@example.com">Bob</a>"#;
        let result =
            correlate_mentions_in_html(html, &conn, "msg1", "acct1").expect("correlate");
        assert!(result.is_empty());
    }

    #[test]
    fn case_insensitive_matching() {
        let conn = setup_db();
        conn.execute(
            "INSERT INTO mentions (message_id, account_id, mentioned_address) \
             VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg1", "acct1", "Alice@Example.COM"],
        )
        .expect("insert");

        let html = r#"<a href="mailto:alice@example.com">Alice</a>"#;
        let result =
            correlate_mentions_in_html(html, &conn, "msg1", "acct1").expect("correlate");
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn no_mentions_returns_empty_fast() {
        let conn = setup_db();
        let html = r#"<a href="mailto:alice@example.com">Alice</a>"#;
        let result =
            correlate_mentions_in_html(html, &conn, "msg1", "acct1").expect("correlate");
        assert!(result.is_empty());
    }

    #[test]
    fn multiple_mentions_in_html() {
        let conn = setup_db();
        conn.execute_batch(
            "INSERT INTO mentions (message_id, account_id, mentioned_address, mentioned_name) \
             VALUES ('msg1', 'acct1', 'alice@example.com', 'Alice');
             INSERT INTO mentions (message_id, account_id, mentioned_address, mentioned_name) \
             VALUES ('msg1', 'acct1', 'bob@example.com', 'Bob');",
        )
        .expect("insert");

        let html = r#"<p><a href="mailto:alice@example.com">Alice</a> and <a href="mailto:bob@example.com">Bob</a></p>"#;
        let result =
            correlate_mentions_in_html(html, &conn, "msg1", "acct1").expect("correlate");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].email, "alice@example.com");
        assert_eq!(result[1].email, "bob@example.com");
        assert!(result[0].byte_offset < result[1].byte_offset);
    }

    #[test]
    fn mailto_with_query_params() {
        let conn = setup_db();
        conn.execute(
            "INSERT INTO mentions (message_id, account_id, mentioned_address) \
             VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg1", "acct1", "alice@example.com"],
        )
        .expect("insert");

        let html =
            r#"<a href="mailto:alice@example.com?subject=Hi&body=Hello">Alice</a>"#;
        let result =
            correlate_mentions_in_html(html, &conn, "msg1", "acct1").expect("correlate");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].email, "alice@example.com");
    }
}
