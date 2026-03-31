use super::super::DbState;
use super::super::sql_fragments::LATEST_MESSAGE_SUBQUERY;
use super::super::types::{
    BundleSummary, BundleSummarySingle, DbBundleRule, ThreadCategoryWithManual, ThreadInfoRow,
};
use super::load_recent_rule_categorized_threads;
use crate::db::from_row::FromRow;
use crate::db::{query_as, query_one};
use rusqlite::params;

pub async fn db_set_thread_category(
    db: &DbState,
    account_id: String,
    thread_id: String,
    category: String,
    is_manual: bool,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO thread_categories (account_id, thread_id, category, is_manual)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(account_id, thread_id) DO UPDATE SET category = ?3, is_manual = ?4",
            params![account_id, thread_id, category, is_manual as i64],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_bundle_rules(
    db: &DbState,
    account_id: String,
) -> Result<Vec<DbBundleRule>, String> {
    db.with_conn(move |conn| {
        query_as::<DbBundleRule>(
            conn,
            "SELECT * FROM bundle_rules WHERE account_id = ?1",
            &[&account_id],
        )
    })
    .await
}

pub async fn db_get_bundle_summaries(
    db: &DbState,
    account_id: String,
    categories: Vec<String>,
) -> Result<Vec<BundleSummary>, String> {
    if categories.is_empty() {
        return Ok(Vec::new());
    }
    db.with_conn(move |conn| {
        let placeholders = categories
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 2))
            .collect::<Vec<_>>()
            .join(", ");
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        param_values.push(Box::new(account_id.clone()));
        for category in &categories {
            param_values.push(Box::new(category.clone()));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(AsRef::as_ref).collect();

        let count_sql = format!(
            "SELECT tc.category, COUNT(DISTINCT t.id) as count
                 FROM threads t
                 JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id AND tl.label_id = 'INBOX'
                 JOIN thread_categories tc ON tc.account_id = t.account_id AND tc.thread_id = t.id AND tc.category IN ({placeholders})
                 WHERE t.account_id = ?1
                 GROUP BY tc.category"
        );
        let mut stmt = conn.prepare(&count_sql).map_err(|e| e.to_string())?;
        let count_rows: Vec<(String, i64)> = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok((row.get::<_, String>("category")?, row.get::<_, i64>("count")?))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        let latest_sql = format!(
            "SELECT tc.category, t.subject, m.from_name
                 FROM threads t
                 JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id AND tl.label_id = 'INBOX'
                 JOIN thread_categories tc ON tc.account_id = t.account_id AND tc.thread_id = t.id AND tc.category IN ({placeholders})
                 JOIN messages m ON m.account_id = t.account_id AND m.thread_id = t.id
                 WHERE t.account_id = ?1
                 GROUP BY tc.category
                 HAVING t.last_message_at = MAX(t.last_message_at)"
        );
        let mut stmt2 = conn.prepare(&latest_sql).map_err(|e| e.to_string())?;
        let latest_rows: Vec<(String, Option<String>, Option<String>)> = stmt2
            .query_map(param_refs.as_slice(), |row| {
                Ok((
                    row.get::<_, String>("category")?,
                    row.get::<_, Option<String>>("subject")?,
                    row.get::<_, Option<String>>("from_name")?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        let mut results = Vec::with_capacity(categories.len());
        for category in &categories {
            let count = count_rows
                .iter()
                .find(|(c, _)| c == category)
                .map(|(_, n)| *n)
                .unwrap_or(0);
            let (latest_subject, latest_sender) = latest_rows
                .iter()
                .find(|(c, _, _)| c == category)
                .map(|(_, s, f)| (s.clone(), f.clone()))
                .unwrap_or((None, None));
            results.push(BundleSummary {
                category: category.clone(),
                count,
                latest_subject,
                latest_sender,
            });
        }
        Ok(results)
    })
    .await
}

pub async fn db_get_held_thread_ids(
    db: &DbState,
    account_id: String,
) -> Result<Vec<String>, String> {
    db.with_conn(move |conn| {
        let now = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_secs(),
        )
        .map_err(|_| "current time exceeds i64 range".to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT thread_id FROM bundled_threads WHERE account_id = ?1 AND held_until > ?2",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id, now], |row| {
            row.get::<_, String>("thread_id")
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_bundle_rule(
    db: &DbState,
    account_id: String,
    category: String,
) -> Result<Option<DbBundleRule>, String> {
    db.with_conn(move |conn| {
        query_one::<DbBundleRule>(
            conn,
            "SELECT * FROM bundle_rules WHERE account_id = ?1 AND category = ?2",
            &[&account_id, &category],
        )
    })
    .await
}

pub async fn db_set_bundle_rule(
    db: &DbState,
    account_id: String,
    category: String,
    is_bundled: bool,
    delivery_enabled: bool,
    schedule: Option<String>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO bundle_rules (id, account_id, category, is_bundled, delivery_enabled, delivery_schedule)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(account_id, category) DO UPDATE SET
                   is_bundled = ?4, delivery_enabled = ?5, delivery_schedule = ?6",
            params![id, account_id, category, is_bundled as i64, delivery_enabled as i64, schedule],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_hold_thread(
    db: &DbState,
    account_id: String,
    thread_id: String,
    category: String,
    held_until: Option<i64>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO bundled_threads (account_id, thread_id, category, held_until)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(account_id, thread_id) DO UPDATE SET
                   category = ?3, held_until = ?4",
            params![account_id, thread_id, category, held_until],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_is_thread_held(
    db: &DbState,
    account_id: String,
    thread_id: String,
    now: i64,
) -> Result<bool, String> {
    db.with_conn(move |conn| {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) AS cnt FROM bundled_threads WHERE account_id = ?1 AND thread_id = ?2 AND held_until > ?3",
                params![account_id, thread_id, now],
                |row| row.get("cnt"),
            )
            .map_err(|e| e.to_string())?;
        Ok(count > 0)
    })
    .await
}

pub async fn db_release_held_threads(
    db: &DbState,
    account_id: String,
    category: String,
) -> Result<i64, String> {
    db.with_conn(move |conn| {
        let affected = conn
            .execute(
                "DELETE FROM bundled_threads WHERE account_id = ?1 AND category = ?2 AND held_until IS NOT NULL",
                params![account_id, category],
            )
            .map_err(|e| e.to_string())?;
        i64::try_from(affected).map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_update_last_delivered(
    db: &DbState,
    account_id: String,
    category: String,
    now: i64,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE bundle_rules SET last_delivered_at = ?1 WHERE account_id = ?2 AND category = ?3",
            params![now, account_id, category],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_bundle_summary(
    db: &DbState,
    account_id: String,
    category: String,
) -> Result<BundleSummarySingle, String> {
    db.with_conn(move |conn| {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(DISTINCT t.id) AS cnt
                     FROM threads t
                     JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id AND tl.label_id = 'INBOX'
                     JOIN thread_categories tc ON tc.account_id = t.account_id AND tc.thread_id = t.id AND tc.category = ?2
                     WHERE t.account_id = ?1",
                params![account_id, category],
                |row| row.get("cnt"),
            )
            .map_err(|e| e.to_string())?;
        let latest = conn
            .query_row(
                "SELECT t.subject, m.from_name
                     FROM threads t
                     JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id AND tl.label_id = 'INBOX'
                     JOIN thread_categories tc ON tc.account_id = t.account_id AND tc.thread_id = t.id AND tc.category = ?2
                     JOIN messages m ON m.account_id = t.account_id AND m.thread_id = t.id
                     WHERE t.account_id = ?1
                     ORDER BY t.last_message_at DESC LIMIT 1",
                params![account_id, category],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>("subject")?,
                        row.get::<_, Option<String>>("from_name")?,
                    ))
                },
            )
            .ok();
        let (latest_subject, latest_sender) = latest.unwrap_or((None, None));
        Ok(BundleSummarySingle {
            count,
            latest_subject,
            latest_sender,
        })
    })
    .await
}

pub async fn db_get_thread_category(
    db: &DbState,
    account_id: String,
    thread_id: String,
) -> Result<Option<String>, String> {
    db.with_conn(move |conn| {
        let result = conn.query_row(
            "SELECT category FROM thread_categories WHERE account_id = ?1 AND thread_id = ?2",
            params![account_id, thread_id],
            |row| row.get::<_, String>("category"),
        );
        match result {
            Ok(category) => Ok(Some(category)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    })
    .await
}

pub async fn db_get_thread_category_with_manual(
    db: &DbState,
    account_id: String,
    thread_id: String,
) -> Result<Option<ThreadCategoryWithManual>, String> {
    db.with_conn(move |conn| {
        let result = conn.query_row(
            "SELECT category, is_manual FROM thread_categories WHERE account_id = ?1 AND thread_id = ?2",
            params![account_id, thread_id],
            ThreadCategoryWithManual::from_row,
        );
        match result {
            Ok(tc) => Ok(Some(tc)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    })
    .await
}

pub async fn db_get_recent_rule_categorized_thread_ids(
    db: &DbState,
    account_id: String,
    limit: Option<i64>,
) -> Result<Vec<ThreadInfoRow>, String> {
    db.with_conn(move |conn| {
        let lim = limit.unwrap_or(20);
        load_recent_rule_categorized_threads(conn, &account_id, lim)
    })
    .await
}

pub async fn db_set_thread_categories_batch(
    db: &DbState,
    account_id: String,
    categories: Vec<(String, String)>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for (thread_id, category) in &categories {
            tx.execute(
                "INSERT INTO thread_categories (account_id, thread_id, category, is_manual)
                     VALUES (?1, ?2, ?3, 0)
                     ON CONFLICT(account_id, thread_id) DO UPDATE SET
                       category = ?3
                     WHERE is_manual = 0",
                params![account_id, thread_id, category],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_uncategorized_inbox_thread_ids(
    db: &DbState,
    account_id: String,
    limit: Option<i64>,
) -> Result<Vec<ThreadInfoRow>, String> {
    db.with_conn(move |conn| {
        let lim = limit.unwrap_or(20);
        let sql = format!(
            "SELECT t.id, t.subject, t.snippet, m.from_address
                 FROM threads t
                 INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
                 LEFT JOIN ({LATEST_MESSAGE_SUBQUERY}
                 ) m ON m.account_id = t.account_id AND m.thread_id = t.id
                 LEFT JOIN thread_categories tc ON tc.account_id = t.account_id AND tc.thread_id = t.id
                 WHERE t.account_id = ?1 AND tl.label_id = 'INBOX' AND tc.thread_id IS NULL
                 ORDER BY t.last_message_at DESC
                 LIMIT ?2"
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id, lim], ThreadInfoRow::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}
