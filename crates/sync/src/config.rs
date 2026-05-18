use db::db::from_row::{query_as, query_one, FromRow, QuerySource};

/// Account record read from the DB (minimal fields needed for sync).
pub struct SyncAccount {
    pub provider: String,
    pub calendar_provider: Option<String>,
    pub caldav_url: Option<String>,
}

impl FromRow for SyncAccount {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            provider: row.get("provider")?,
            calendar_provider: row.get("calendar_provider")?,
            caldav_url: row.get("caldav_url")?,
        })
    }
}

pub struct AutoSyncConfig {
    pub provider: String,
    pub initial_sync_completed: bool,
    pub sync_period_days: i64,
}

/// Read an account from the DB.
pub fn get_account(conn: &(impl QuerySource + ?Sized), account_id: &str) -> Result<SyncAccount, String> {
    query_one::<SyncAccount>(
        conn,
        "SELECT provider, calendar_provider, caldav_url FROM accounts WHERE id = ?1",
        &[&account_id],
    )
    .and_then(|row| row.ok_or_else(|| format!("account not found: {account_id}")))
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

    if account.provider == "graph" {
        return Some("graph");
    }

    // IMAP, JMAP, and any other mail provider can co-host CalDAV calendar:
    // the per-account override `calendar_provider = "caldav"` plus a
    // configured `caldav_url` is enough to enable CalDAV sync regardless of
    // the underlying mail protocol.
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
pub fn get_sync_period_days(conn: &(impl QuerySource + ?Sized)) -> i64 {
    db::db::queries::get_setting(conn, "sync_period_days")
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(365)
}

pub fn get_auto_sync_config(
    conn: &(impl QuerySource + ?Sized),
    account_id: &str,
) -> Result<AutoSyncConfig, String> {
    struct AutoSyncConfigRow {
        provider: String,
        initial_sync_completed: bool,
    }

    impl FromRow for AutoSyncConfigRow {
        fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
            Ok(Self {
                provider: row.get("provider")?,
                initial_sync_completed: row.get::<_, i64>("initial_sync_completed")? != 0,
            })
        }
    }

    let provider = query_one::<AutoSyncConfigRow>(
        conn,
        "SELECT provider, initial_sync_completed FROM accounts WHERE id = ?1",
        &[&account_id],
    )
    .map_err(|e| format!("Failed to read sync state for account {account_id}: {e}"))?
    .ok_or_else(|| format!("Failed to read sync state for account {account_id}: not found"))?;

    Ok(AutoSyncConfig {
        provider: provider.provider,
        initial_sync_completed: provider.initial_sync_completed,
        sync_period_days: get_sync_period_days(conn),
    })
}

pub fn get_active_account_ids(conn: &(impl QuerySource + ?Sized)) -> Result<Vec<String>, String> {
    struct AccountIdRow {
        id: String,
    }

    impl FromRow for AccountIdRow {
        fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
            Ok(Self { id: row.get("id")? })
        }
    }

    query_as::<AccountIdRow>(
        conn,
        "SELECT id FROM accounts WHERE is_active = 1 ORDER BY created_at ASC",
        &[],
    )
    .map(|rows| rows.into_iter().map(|row| row.id).collect())
    .map_err(|e| format!("collect active accounts: {e}"))
}
