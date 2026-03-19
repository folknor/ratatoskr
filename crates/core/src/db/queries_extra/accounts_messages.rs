use super::super::DbState;
use super::super::queries::row_to_message;
use super::super::types::{DbAccount, DbMessage};
use rusqlite::{Row, params};

fn row_to_account(row: &Row<'_>) -> rusqlite::Result<DbAccount> {
    Ok(DbAccount {
        id: row.get("id")?,
        email: row.get("email")?,
        display_name: row.get("display_name")?,
        avatar_url: row.get("avatar_url")?,
        access_token: row.get("access_token")?,
        refresh_token: row.get("refresh_token")?,
        token_expires_at: row.get("token_expires_at")?,
        history_id: row.get("history_id")?,
        initial_sync_completed: row.get("initial_sync_completed")?,
        last_sync_at: row.get("last_sync_at")?,
        is_active: row.get("is_active")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        provider: row.get("provider")?,
        imap_host: row.get("imap_host")?,
        imap_port: row.get("imap_port")?,
        imap_security: row.get("imap_security")?,
        smtp_host: row.get("smtp_host")?,
        smtp_port: row.get("smtp_port")?,
        smtp_security: row.get("smtp_security")?,
        auth_method: row.get("auth_method")?,
        imap_password: row.get("imap_password")?,
        oauth_provider: row.get("oauth_provider")?,
        oauth_client_id: row.get("oauth_client_id")?,
        oauth_client_secret: row.get("oauth_client_secret")?,
        imap_username: row.get("imap_username")?,
        caldav_url: row.get("caldav_url")?,
        caldav_username: row.get("caldav_username")?,
        caldav_password: row.get("caldav_password")?,
        caldav_principal_url: row.get("caldav_principal_url")?,
        caldav_home_url: row.get("caldav_home_url")?,
        calendar_provider: row.get("calendar_provider")?,
        accept_invalid_certs: row.get("accept_invalid_certs")?,
        jmap_url: row.get("jmap_url")?,
    })
}

pub async fn db_get_all_accounts(db: &DbState) -> Result<Vec<DbAccount>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare("SELECT * FROM accounts ORDER BY created_at ASC")
            .map_err(|e| e.to_string())?;
        stmt.query_map([], row_to_account)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_account(db: &DbState, id: String) -> Result<Option<DbAccount>, String> {
    db.with_conn(move |conn| {
        Ok(conn
            .query_row(
                "SELECT * FROM accounts WHERE id = ?1",
                params![id],
                row_to_account,
            )
            .ok())
    })
    .await
}

pub async fn db_get_account_by_email(
    db: &DbState,
    email: String,
) -> Result<Option<DbAccount>, String> {
    db.with_conn(move |conn| {
        Ok(conn
            .query_row(
                "SELECT * FROM accounts WHERE email = ?1",
                params![email],
                row_to_account,
            )
            .ok())
    })
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn db_insert_account(
    db: &DbState,
    id: String,
    email: String,
    display_name: Option<String>,
    avatar_url: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
    token_expires_at: Option<i64>,
    provider: String,
    auth_method: String,
    imap_host: Option<String>,
    imap_port: Option<i64>,
    imap_security: Option<String>,
    smtp_host: Option<String>,
    smtp_port: Option<i64>,
    smtp_security: Option<String>,
    imap_password: Option<String>,
    oauth_provider: Option<String>,
    oauth_client_id: Option<String>,
    oauth_client_secret: Option<String>,
    imap_username: Option<String>,
    accept_invalid_certs: Option<i64>,
    caldav_url: Option<String>,
    caldav_username: Option<String>,
    caldav_password: Option<String>,
    caldav_principal_url: Option<String>,
    caldav_home_url: Option<String>,
    calendar_provider: Option<String>,
    jmap_url: Option<String>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO accounts (id, email, display_name, avatar_url, access_token, \
                 refresh_token, token_expires_at, provider, auth_method, imap_host, imap_port, \
                 imap_security, smtp_host, smtp_port, smtp_security, imap_password, \
                 oauth_provider, oauth_client_id, oauth_client_secret, imap_username, \
                 accept_invalid_certs, caldav_url, caldav_username, caldav_password, \
                 caldav_principal_url, caldav_home_url, calendar_provider, jmap_url) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, \
                 ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28)",
            params![
                id,
                email,
                display_name,
                avatar_url,
                access_token,
                refresh_token,
                token_expires_at,
                provider,
                auth_method,
                imap_host,
                imap_port,
                imap_security,
                smtp_host,
                smtp_port,
                smtp_security,
                imap_password,
                oauth_provider,
                oauth_client_id,
                oauth_client_secret,
                imap_username,
                accept_invalid_certs.unwrap_or(0),
                caldav_url,
                caldav_username,
                caldav_password,
                caldav_principal_url,
                caldav_home_url,
                calendar_provider,
                jmap_url,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_update_account_tokens(
    db: &DbState,
    id: String,
    access_token: String,
    token_expires_at: i64,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE accounts SET access_token = ?1, token_expires_at = ?2, \
                 updated_at = unixepoch() WHERE id = ?3",
            params![access_token, token_expires_at, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_update_account_all_tokens(
    db: &DbState,
    id: String,
    access_token: String,
    refresh_token: String,
    token_expires_at: i64,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE accounts SET access_token = ?1, refresh_token = ?2, \
                 token_expires_at = ?3, updated_at = unixepoch() WHERE id = ?4",
            params![access_token, refresh_token, token_expires_at, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_delete_account(db: &DbState, id: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute("DELETE FROM accounts WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_update_account_caldav(
    db: &DbState,
    id: String,
    caldav_url: String,
    caldav_username: String,
    caldav_password: String,
    caldav_principal_url: Option<String>,
    caldav_home_url: Option<String>,
    calendar_provider: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE accounts SET caldav_url = ?1, caldav_username = ?2, caldav_password = ?3, \
                 caldav_principal_url = ?4, caldav_home_url = ?5, calendar_provider = ?6, \
                 updated_at = unixepoch() WHERE id = ?7",
            params![
                caldav_url,
                caldav_username,
                caldav_password,
                caldav_principal_url,
                caldav_home_url,
                calendar_provider,
                id
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_upsert_thread(
    db: &DbState,
    id: String,
    account_id: String,
    subject: Option<String>,
    snippet: Option<String>,
    last_message_at: Option<i64>,
    message_count: i64,
    is_read: bool,
    is_starred: bool,
    is_important: bool,
    has_attachments: bool,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO threads (id, account_id, subject, snippet, last_message_at, message_count, is_read, is_starred, is_important, has_attachments)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(account_id, id) DO UPDATE SET
                   subject = ?3, snippet = ?4, last_message_at = ?5, message_count = ?6,
                   is_read = ?7, is_starred = ?8, is_important = ?9, has_attachments = ?10",
            params![id, account_id, subject, snippet, last_message_at, message_count, is_read, is_starred, is_important, has_attachments],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_set_thread_labels(
    db: &DbState,
    account_id: String,
    thread_id: String,
    label_ids: Vec<String>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM thread_labels WHERE account_id = ?1 AND thread_id = ?2",
            params![account_id, thread_id],
        )
        .map_err(|e| e.to_string())?;
        for label_id in &label_ids {
            tx.execute(
                "INSERT OR IGNORE INTO thread_labels (account_id, thread_id, label_id) VALUES (?1, ?2, ?3)",
                params![account_id, thread_id, label_id],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_delete_all_threads_for_account(
    db: &DbState,
    account_id: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM threads WHERE account_id = ?1",
            params![account_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_muted_thread_ids(
    db: &DbState,
    account_id: String,
) -> Result<Vec<String>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare("SELECT id FROM threads WHERE account_id = ?1 AND is_muted = 1")
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id], |row| row.get::<_, String>("id"))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_unread_inbox_count(db: &DbState) -> Result<i64, String> {
    db.with_conn(move |conn| {
        conn.query_row(
            "SELECT COUNT(*) AS cnt FROM threads t
                 INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
                 WHERE tl.label_id = 'INBOX' AND t.is_read = 0",
            [],
            |row| row.get::<_, i64>("cnt"),
        )
        .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_messages_by_ids(
    db: &DbState,
    account_id: String,
    message_ids: Vec<String>,
) -> Result<Vec<DbMessage>, String> {
    if message_ids.is_empty() {
        return Ok(Vec::new());
    }
    db.with_conn(move |conn| {
        let mut all_results = Vec::new();
        for chunk in message_ids.chunks(500) {
            let placeholders = chunk
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 2))
                .collect::<Vec<_>>()
                .join(", ");
            let sql =
                format!("SELECT * FROM messages WHERE account_id = ?1 AND id IN ({placeholders})");
            let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            param_values.push(Box::new(account_id.clone()));
            for id in chunk {
                param_values.push(Box::new(id.clone()));
            }
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(AsRef::as_ref).collect();
            let rows = stmt
                .query_map(param_refs.as_slice(), row_to_message)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            all_results.extend(rows);
        }
        Ok(all_results)
    })
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn db_upsert_message(
    db: &DbState,
    id: String,
    account_id: String,
    thread_id: String,
    from_address: Option<String>,
    from_name: Option<String>,
    to_addresses: Option<String>,
    cc_addresses: Option<String>,
    bcc_addresses: Option<String>,
    reply_to: Option<String>,
    subject: Option<String>,
    snippet: Option<String>,
    date: i64,
    is_read: bool,
    is_starred: bool,
    body_cached: bool,
    raw_size: Option<i64>,
    internal_date: Option<i64>,
    list_unsubscribe: Option<String>,
    list_unsubscribe_post: Option<String>,
    auth_results: Option<String>,
    message_id_header: Option<String>,
    references_header: Option<String>,
    in_reply_to_header: Option<String>,
    imap_uid: Option<i64>,
    imap_folder: Option<String>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO messages (id, account_id, thread_id, from_address, from_name, to_addresses, cc_addresses, bcc_addresses, reply_to, subject, snippet, date, is_read, is_starred, body_cached, raw_size, internal_date, list_unsubscribe, list_unsubscribe_post, auth_results, message_id_header, references_header, in_reply_to_header, imap_uid, imap_folder)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)
                 ON CONFLICT(account_id, id) DO UPDATE SET
                   from_address = ?4, from_name = ?5, to_addresses = ?6, cc_addresses = ?7,
                   bcc_addresses = ?8, reply_to = ?9, subject = ?10, snippet = ?11,
                   date = ?12, is_read = ?13, is_starred = ?14,
                   body_cached = CASE WHEN ?15 = 1 THEN 1 ELSE body_cached END,
                   raw_size = ?16, internal_date = ?17, list_unsubscribe = ?18, list_unsubscribe_post = ?19,
                   auth_results = ?20, message_id_header = COALESCE(?21, message_id_header),
                   references_header = COALESCE(?22, references_header),
                   in_reply_to_header = COALESCE(?23, in_reply_to_header),
                   imap_uid = COALESCE(?24, imap_uid), imap_folder = COALESCE(?25, imap_folder)",
            params![id, account_id, thread_id, from_address, from_name, to_addresses, cc_addresses, bcc_addresses, reply_to, subject, snippet, date, is_read, is_starred, body_cached, raw_size, internal_date, list_unsubscribe, list_unsubscribe_post, auth_results, message_id_header, references_header, in_reply_to_header, imap_uid, imap_folder],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_delete_message(
    db: &DbState,
    account_id: String,
    message_id: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM messages WHERE account_id = ?1 AND id = ?2",
            params![account_id, message_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_update_message_thread_ids(
    db: &DbState,
    account_id: String,
    message_ids: Vec<String>,
    thread_id: String,
) -> Result<(), String> {
    if message_ids.is_empty() {
        return Ok(());
    }
    db.with_conn(move |conn| {
        for chunk in message_ids.chunks(500) {
            let placeholders = chunk
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 3))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "UPDATE messages SET thread_id = ?1 WHERE account_id = ?2 AND id IN ({placeholders})"
            );
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            param_values.push(Box::new(thread_id.clone()));
            param_values.push(Box::new(account_id.clone()));
            for id in chunk {
                param_values.push(Box::new(id.clone()));
            }
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(AsRef::as_ref).collect();
            conn.execute(&sql, param_refs.as_slice())
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .await
}

pub async fn db_delete_all_messages_for_account(
    db: &DbState,
    account_id: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM messages WHERE account_id = ?1",
            params![account_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_recent_sent_messages(
    db: &DbState,
    account_id: String,
    account_email: String,
    limit: Option<i64>,
) -> Result<Vec<DbMessage>, String> {
    db.with_conn(move |conn| {
        let lim = limit.unwrap_or(15);
        let mut stmt = conn
            .prepare(
                "SELECT * FROM messages
                     WHERE account_id = ?1 AND LOWER(from_address) = LOWER(?2)
                       AND body_cached = 1
                     ORDER BY date DESC LIMIT ?3",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id, account_email, lim], row_to_message)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}
