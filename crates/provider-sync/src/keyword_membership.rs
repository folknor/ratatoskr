use std::collections::HashSet;

use db::db::queries_extra::{LabelWriteRow, upsert_labels};

#[derive(Clone, Copy, Debug)]
pub(crate) enum KeywordProvider {
    Imap,
    Jmap,
}

impl KeywordProvider {
    fn label_id(self, keyword: &str) -> Result<String, String> {
        match self {
            Self::Imap => common::types::LabelKind::imap_keyword(keyword),
            Self::Jmap => common::types::LabelKind::jmap_keyword(keyword),
        }
        .map(|label| label.storage_id())
    }

    fn name(self) -> &'static str {
        match self {
            Self::Imap => "IMAP",
            Self::Jmap => "JMAP",
        }
    }
}

pub(crate) fn replace_message_keywords(
    tx: &rusqlite::Transaction,
    provider: KeywordProvider,
    account_id: &str,
    message_id: &str,
    keywords: &[String],
) -> Result<(), String> {
    let label_pairs = upsert_keyword_labels(tx, provider, account_id, keywords)?;

    tx.execute(
        "DELETE FROM message_keywords WHERE account_id = ?1 AND message_id = ?2",
        rusqlite::params![account_id, message_id],
    )
    .map_err(|e| format!("delete {} message keywords: {e}", provider.name()))?;

    for (keyword, label_id) in label_pairs {
        tx.execute(
            "INSERT OR IGNORE INTO message_keywords (account_id, message_id, keyword, label_id) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![account_id, message_id, keyword, label_id],
        )
        .map_err(|e| format!("insert {} message keyword: {e}", provider.name()))?;
    }

    Ok(())
}

/// Recompute thread-level keyword labels from per-message keyword rows.
///
/// IMAP and JMAP account threads only carry keyword rows in `thread_labels`;
/// provider mailboxes are folder-shaped and local label groups live in
/// `thread_label_groups`. The destructive replace is therefore scoped to
/// the whole thread_labels set for the thread.
pub(crate) fn recompute_thread_keyword_labels(
    tx: &rusqlite::Transaction,
    provider: KeywordProvider,
    account_id: &str,
    thread_id: &str,
) -> Result<(), String> {
    tx.execute(
        "DELETE FROM thread_labels \
         WHERE account_id = ?1 AND thread_id = ?2",
        rusqlite::params![account_id, thread_id],
    )
    .map_err(|e| format!("delete {} thread keyword labels: {e}", provider.name()))?;

    tx.execute(
        "INSERT OR IGNORE INTO thread_labels (account_id, thread_id, label_id) \
         SELECT DISTINCT m.account_id, m.thread_id, mk.label_id \
         FROM messages m \
         JOIN message_keywords mk ON mk.account_id = m.account_id AND mk.message_id = m.id \
         WHERE m.account_id = ?1 AND m.thread_id = ?2",
        rusqlite::params![account_id, thread_id],
    )
    .map_err(|e| format!("insert {} thread keyword labels: {e}", provider.name()))?;

    Ok(())
}

fn upsert_keyword_labels(
    tx: &rusqlite::Transaction,
    provider: KeywordProvider,
    account_id: &str,
    keywords: &[String],
) -> Result<Vec<(String, String)>, String> {
    let mut unique = HashSet::new();
    let mut label_pairs = Vec::new();

    for keyword in keywords {
        if !common::folder_roles::is_user_visible_keyword(keyword) {
            continue;
        }
        if unique.insert(keyword.clone()) {
            let label_id = provider.label_id(keyword)?;
            label_pairs.push((keyword.clone(), label_id));
        }
    }

    if label_pairs.is_empty() {
        return Ok(label_pairs);
    }

    let rows: Vec<LabelWriteRow> = label_pairs
        .iter()
        .map(|(keyword, label_id)| LabelWriteRow {
            id: label_id.clone(),
            account_id: account_id.to_string(),
            name: keyword.clone(),
            visible: None,
            sort_order: None,
            server_color_bg: None,
            server_color_fg: None,
            user_color_bg: None,
            user_color_fg: None,
            is_undeletable: false,
        })
        .collect();
    upsert_labels(tx, &rows)?;
    Ok(label_pairs)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{KeywordProvider, recompute_thread_keyword_labels, replace_message_keywords};

    fn setup_conn(provider: &str) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");
        db::db::migrations::run_all(&conn).expect("migrations");
        conn.execute(
            "INSERT INTO accounts (id, email, provider) VALUES ('acc', 'a@example.com', ?1)",
            [provider],
        )
        .expect("insert account");
        conn.execute(
            "INSERT INTO threads (account_id, id, message_count) VALUES ('acc', 'thread', 2)",
            [],
        )
        .expect("insert thread");
        for message_id in ["m1", "m2"] {
            conn.execute(
                "INSERT INTO messages (account_id, id, thread_id, date, is_read) \
                 VALUES ('acc', ?1, 'thread', 1, 1)",
                [message_id],
            )
            .expect("insert message");
        }
        conn
    }

    fn thread_label_count(conn: &rusqlite::Connection, label_id: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM thread_labels \
             WHERE account_id = 'acc' AND thread_id = 'thread' AND label_id = ?1",
            [label_id],
            |row| row.get(0),
        )
        .expect("count thread label")
    }

    #[test]
    fn recompute_removes_keyword_absent_from_message_union() {
        let conn = setup_conn("jmap");
        let tx = conn.unchecked_transaction().unwrap();
        replace_message_keywords(
            &tx,
            KeywordProvider::Jmap,
            "acc",
            "m1",
            &[String::from("todo")],
        )
        .unwrap();
        recompute_thread_keyword_labels(&tx, KeywordProvider::Jmap, "acc", "thread").unwrap();
        tx.commit().unwrap();
        assert_eq!(thread_label_count(&conn, "kw:todo"), 1);

        let tx = conn.unchecked_transaction().unwrap();
        replace_message_keywords(&tx, KeywordProvider::Jmap, "acc", "m1", &[]).unwrap();
        recompute_thread_keyword_labels(&tx, KeywordProvider::Jmap, "acc", "thread").unwrap();
        tx.commit().unwrap();
        assert_eq!(thread_label_count(&conn, "kw:todo"), 0);
    }

    #[test]
    fn recompute_keeps_keyword_present_on_sibling_message() {
        let conn = setup_conn("jmap");
        let tx = conn.unchecked_transaction().unwrap();
        replace_message_keywords(
            &tx,
            KeywordProvider::Jmap,
            "acc",
            "m1",
            &[String::from("todo")],
        )
        .unwrap();
        replace_message_keywords(
            &tx,
            KeywordProvider::Jmap,
            "acc",
            "m2",
            &[String::from("todo")],
        )
        .unwrap();
        recompute_thread_keyword_labels(&tx, KeywordProvider::Jmap, "acc", "thread").unwrap();
        tx.commit().unwrap();
        assert_eq!(thread_label_count(&conn, "kw:todo"), 1);

        let tx = conn.unchecked_transaction().unwrap();
        replace_message_keywords(&tx, KeywordProvider::Jmap, "acc", "m1", &[]).unwrap();
        recompute_thread_keyword_labels(&tx, KeywordProvider::Jmap, "acc", "thread").unwrap();
        tx.commit().unwrap();
        assert_eq!(thread_label_count(&conn, "kw:todo"), 1);
    }

    #[test]
    fn imap_keywords_use_the_same_recompute_path() {
        let conn = setup_conn("imap");
        let tx = conn.unchecked_transaction().unwrap();
        replace_message_keywords(
            &tx,
            KeywordProvider::Imap,
            "acc",
            "m1",
            &[String::from("todo")],
        )
        .unwrap();
        recompute_thread_keyword_labels(&tx, KeywordProvider::Imap, "acc", "thread").unwrap();
        tx.commit().unwrap();

        assert_eq!(thread_label_count(&conn, "kw:todo"), 1);
    }
}
