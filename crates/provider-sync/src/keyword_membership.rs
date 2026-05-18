use std::collections::HashSet;

use db::db::queries_extra::{LabelWriteRow, recompute_thread_labels_from_messages, upsert_labels};

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
    tx: &db::db::WriteTxn<'_>,
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
    tx: &db::db::WriteTxn<'_>,
    provider: KeywordProvider,
    account_id: &str,
    thread_id: &str,
) -> Result<(), String> {
    recompute_thread_labels_from_messages(tx, account_id, thread_id)
        .map_err(|e| format!("recompute {} thread keyword labels: {e}", provider.name()))
}

fn upsert_keyword_labels(
    tx: &db::db::WriteTxn<'_>,
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

    fn setup_pool(provider: &str) -> (db::db::WriterPool, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let pool = db::db::open_writer_pool(tmp.path()).expect("open writer pool");
        let provider = provider.to_string();
        pool.with_write_sync(move |conn| {
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
            Ok(())
        })
        .expect("seed db");
        (pool, tmp)
    }

    fn thread_label_count(pool: &db::db::WriterPool, label_id: &str) -> i64 {
        let label_id = label_id.to_string();
        pool.with_read_sync(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM thread_labels \
                 WHERE account_id = 'acc' AND thread_id = 'thread' AND label_id = ?1",
                [label_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())
        })
        .expect("count thread label")
    }

    #[test]
    fn recompute_removes_keyword_absent_from_message_union() {
        let (pool, _tmp) = setup_pool("jmap");
        pool.with_write_sync(|write| {
            let tx = write.transaction().unwrap();
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
            Ok(())
        })
        .unwrap();
        assert_eq!(thread_label_count(&pool, "kw:todo"), 1);

        pool.with_write_sync(|write| {
            let tx = write.transaction().unwrap();
            replace_message_keywords(&tx, KeywordProvider::Jmap, "acc", "m1", &[]).unwrap();
            recompute_thread_keyword_labels(&tx, KeywordProvider::Jmap, "acc", "thread").unwrap();
            tx.commit().unwrap();
            Ok(())
        })
        .unwrap();
        assert_eq!(thread_label_count(&pool, "kw:todo"), 0);
    }

    #[test]
    fn recompute_keeps_keyword_present_on_sibling_message() {
        let (pool, _tmp) = setup_pool("jmap");
        pool.with_write_sync(|write| {
            let tx = write.transaction().unwrap();
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
            Ok(())
        })
        .unwrap();
        assert_eq!(thread_label_count(&pool, "kw:todo"), 1);

        pool.with_write_sync(|write| {
            let tx = write.transaction().unwrap();
            replace_message_keywords(&tx, KeywordProvider::Jmap, "acc", "m1", &[]).unwrap();
            recompute_thread_keyword_labels(&tx, KeywordProvider::Jmap, "acc", "thread").unwrap();
            tx.commit().unwrap();
            Ok(())
        })
        .unwrap();
        assert_eq!(thread_label_count(&pool, "kw:todo"), 1);
    }

    #[test]
    fn imap_keywords_use_the_same_recompute_path() {
        let (pool, _tmp) = setup_pool("imap");
        pool.with_write_sync(|write| {
            let tx = write.transaction().unwrap();
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
            Ok(())
        })
        .unwrap();

        assert_eq!(thread_label_count(&pool, "kw:todo"), 1);
    }
}
