use super::super::DbState;
use super::super::types::{DbAllowlistEntry, DbNotificationVip, DbPhishingAllowlistEntry};
use rusqlite::params;

pub async fn db_add_to_allowlist(
    db: &DbState,
    id: String,
    account_id: String,
    sender_address: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO image_allowlist (id, account_id, sender_address) VALUES (?1, ?2, ?3)",
            params![id, account_id, sender_address],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_allowlisted_senders(
    db: &DbState,
    account_id: String,
    sender_addresses: Vec<String>,
) -> Result<Vec<String>, String> {
    if sender_addresses.is_empty() {
        return Ok(Vec::new());
    }
    db.with_conn(move |conn| {
        let mut results = Vec::new();
        for chunk in sender_addresses.chunks(100) {
            let placeholders = chunk
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 2))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT sender_address FROM image_allowlist
                     WHERE account_id = ?1 AND sender_address IN ({placeholders})"
            );
            let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            param_values.push(Box::new(account_id.clone()));
            for address in chunk {
                param_values.push(Box::new(address.clone()));
            }
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(AsRef::as_ref).collect();
            let rows = stmt
                .query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            results.extend(rows);
        }
        Ok(results)
    })
    .await
}

pub async fn db_add_vip_sender(
    db: &DbState,
    id: String,
    account_id: String,
    email: String,
    display_name: Option<String>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO notification_vips (id, account_id, email_address, display_name)
                 VALUES (?1, ?2, ?3, ?4)",
            params![id, account_id, email, display_name],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_remove_vip_sender(
    db: &DbState,
    account_id: String,
    email: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM notification_vips WHERE account_id = ?1 AND email_address = ?2",
            params![account_id, email],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_is_vip_sender(
    db: &DbState,
    account_id: String,
    email: String,
) -> Result<bool, String> {
    db.with_conn(move |conn| {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM notification_vips WHERE account_id = ?1 AND email_address = ?2",
                params![account_id, email],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(count > 0)
    })
    .await
}

pub async fn db_get_vip_senders(db: &DbState, account_id: String) -> Result<Vec<String>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare("SELECT email_address FROM notification_vips WHERE account_id = ?1")
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_all_vip_senders(
    db: &DbState,
    account_id: String,
) -> Result<Vec<DbNotificationVip>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, account_id, email_address, display_name, created_at
                     FROM notification_vips WHERE account_id = ?1
                     ORDER BY display_name, email_address",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id], |row| {
            Ok(DbNotificationVip {
                id: row.get("id")?,
                account_id: row.get("account_id")?,
                email_address: row.get("email_address")?,
                display_name: row.get("display_name")?,
                created_at: row.get("created_at")?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_is_allowlisted(
    db: &DbState,
    account_id: String,
    sender_address: String,
) -> Result<bool, String> {
    let sender_address = sender_address.to_lowercase();
    db.with_conn(move |conn| {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM image_allowlist WHERE account_id = ?1 AND sender_address = ?2",
                params![account_id, sender_address],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(count > 0)
    })
    .await
}

pub async fn db_remove_from_allowlist(
    db: &DbState,
    account_id: String,
    sender_address: String,
) -> Result<(), String> {
    let sender_address = sender_address.to_lowercase();
    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM image_allowlist WHERE account_id = ?1 AND sender_address = ?2",
            params![account_id, sender_address],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_allowlist_for_account(
    db: &DbState,
    account_id: String,
) -> Result<Vec<DbAllowlistEntry>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, account_id, sender_address, created_at
                     FROM image_allowlist WHERE account_id = ?1
                     ORDER BY sender_address",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id], |row| {
            Ok(DbAllowlistEntry {
                id: row.get("id")?,
                account_id: row.get("account_id")?,
                sender_address: row.get("sender_address")?,
                created_at: row.get("created_at")?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_is_phishing_allowlisted(
    db: &DbState,
    account_id: String,
    sender_address: String,
) -> Result<bool, String> {
    let sender_address = sender_address.to_lowercase();
    db.with_conn(move |conn| {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM phishing_allowlist WHERE account_id = ?1 AND sender_address = ?2",
                params![account_id, sender_address],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(count > 0)
    })
    .await
}

pub async fn db_add_to_phishing_allowlist(
    db: &DbState,
    account_id: String,
    sender_address: String,
) -> Result<(), String> {
    let sender_address = sender_address.to_lowercase();
    let id = uuid::Uuid::new_v4().to_string();
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO phishing_allowlist (id, account_id, sender_address) VALUES (?1, ?2, ?3)",
            params![id, account_id, sender_address],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_remove_from_phishing_allowlist(
    db: &DbState,
    account_id: String,
    sender_address: String,
) -> Result<(), String> {
    let sender_address = sender_address.to_lowercase();
    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM phishing_allowlist WHERE account_id = ?1 AND sender_address = ?2",
            params![account_id, sender_address],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_phishing_allowlist(
    db: &DbState,
    account_id: String,
) -> Result<Vec<DbPhishingAllowlistEntry>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, sender_address, created_at
                     FROM phishing_allowlist WHERE account_id = ?1
                     ORDER BY sender_address",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id], |row| {
            Ok(DbPhishingAllowlistEntry {
                id: row.get("id")?,
                sender_address: row.get("sender_address")?,
                created_at: row.get("created_at")?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
    })
    .await
}
