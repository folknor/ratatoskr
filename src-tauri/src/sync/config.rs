use rusqlite::Connection;

/// Account record read from the DB (minimal fields needed for sync).
pub struct SyncAccount {
    pub provider: String,
    pub calendar_provider: Option<String>,
    pub caldav_url: Option<String>,
}

pub struct AutoSyncConfig {
    pub provider: String,
    pub has_history: bool,
    pub sync_period_days: i64,
}

/// Read an account from the DB.
pub fn get_account(conn: &Connection, account_id: &str) -> Result<SyncAccount, String> {
    conn.query_row(
        "SELECT provider, calendar_provider, caldav_url FROM accounts WHERE id = ?1",
        rusqlite::params![account_id],
        |row| {
            Ok(SyncAccount {
                provider: row.get(0)?,
                calendar_provider: row.get(1)?,
                caldav_url: row.get(2)?,
            })
        },
    )
    .map_err(|e| format!("get account {account_id}: {e}"))
}

pub fn should_sync_calendar(account: &SyncAccount) -> bool {
    calendar_provider_kind(account).is_some()
}

pub fn calendar_provider_kind(account: &SyncAccount) -> Option<&'static str> {
    if account.provider == "caldav" {
        return Some("caldav");
    }

    if account.provider == "gmail_api" {
        return Some("google_api");
    }

    if account.calendar_provider.as_deref() == Some("caldav")
        && account
            .caldav_url
            .as_ref()
            .is_some_and(|url| !url.trim().is_empty())
    {
        return Some("caldav");
    }

    None
}

/// Read the `sync_period_days` setting from DB, defaulting to 365.
pub fn get_sync_period_days(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT value FROM settings WHERE key = 'sync_period_days'",
        [],
        |row| {
            let val: String = row.get(0)?;
            Ok(val.parse::<i64>().unwrap_or(365))
        },
    )
    .unwrap_or(365)
}

pub fn get_auto_sync_config(conn: &Connection, account_id: &str) -> Result<AutoSyncConfig, String> {
    let provider = conn
        .query_row(
            "SELECT provider, history_id FROM accounts WHERE id = ?1",
            rusqlite::params![account_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?.is_some(),
                ))
            },
        )
        .map_err(|e| format!("Failed to read sync state for account {account_id}: {e}"))?;

    Ok(AutoSyncConfig {
        provider: provider.0,
        has_history: provider.1,
        sync_period_days: get_sync_period_days(conn),
    })
}

pub fn get_active_account_ids(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM accounts WHERE is_active = 1 ORDER BY created_at ASC")
        .map_err(|e| format!("prepare active account query: {e}"))?;
    stmt.query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("query active accounts: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect active accounts: {e}"))
}
