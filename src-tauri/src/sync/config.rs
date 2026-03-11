use rusqlite::Connection;

/// Account record read from the DB (minimal fields needed for sync).
pub struct SyncAccount {
    pub provider: String,
    pub calendar_provider: Option<String>,
    pub caldav_url: Option<String>,
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
    if account.provider == "caldav" || account.provider == "gmail_api" {
        return true;
    }

    if account.calendar_provider.as_deref() == Some("google_api") {
        return true;
    }

    account.calendar_provider.as_deref() == Some("caldav")
        && account
            .caldav_url
            .as_ref()
            .is_some_and(|url| !url.trim().is_empty())
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
