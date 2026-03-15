use rusqlite::{Connection, OptionalExtension};

use crate::provider::crypto::{decrypt_value, is_encrypted};
use crate::sync::config;

use super::types::{
    AccountBasicInfo, AccountCaldavSettingsInfo, AccountOAuthCredentials, CaldavConnectionInfo,
    CalendarProviderInfo,
};

pub fn get_calendar_provider_info(
    conn: &Connection,
    account_id: &str,
) -> Result<Option<CalendarProviderInfo>, String> {
    let account = config::get_account(conn, account_id)?;
    Ok(
        config::calendar_provider_kind(&account).map(|provider| CalendarProviderInfo {
            provider: provider.to_string(),
        }),
    )
}

pub fn get_caldav_connection_info(
    conn: &Connection,
    account_id: &str,
    encryption_key: &[u8; 32],
) -> Result<Option<CaldavConnectionInfo>, String> {
    let Some(row) = conn
        .query_row(
            "SELECT email, caldav_url, caldav_username, caldav_password FROM accounts WHERE id = ?1",
            rusqlite::params![account_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|e| format!("query caldav account: {e}"))?
    else {
        return Ok(None);
    };

    let server_url = row
        .1
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "CalDAV credentials not configured".to_string())?;
    let password_raw = row
        .3
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "CalDAV credentials not configured".to_string())?;
    let password = if is_encrypted(&password_raw) {
        decrypt_value(encryption_key, &password_raw).unwrap_or(password_raw)
    } else {
        password_raw
    };
    let username = row
        .2
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(row.0);

    Ok(Some(CaldavConnectionInfo {
        server_url,
        username,
        password,
    }))
}

pub fn get_basic_info(
    conn: &Connection,
    account_id: &str,
) -> Result<Option<AccountBasicInfo>, String> {
    conn.query_row(
        "SELECT id, email, display_name, avatar_url, provider, is_active FROM accounts WHERE id = ?1",
        rusqlite::params![account_id],
        |row| {
            Ok(AccountBasicInfo {
                id: row.get(0)?,
                email: row.get(1)?,
                display_name: row.get(2)?,
                avatar_url: row.get(3)?,
                provider: row.get(4)?,
                is_active: row.get::<_, i64>(5)? != 0,
            })
        },
    )
    .optional()
    .map_err(|e| format!("query account basic info: {e}"))
}

pub fn list_basic_info(conn: &Connection) -> Result<Vec<AccountBasicInfo>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, email, display_name, avatar_url, provider, is_active \
             FROM accounts ORDER BY created_at ASC",
        )
        .map_err(|e| format!("prepare account list: {e}"))?;
    stmt.query_map([], |row| {
        Ok(AccountBasicInfo {
            id: row.get(0)?,
            email: row.get(1)?,
            display_name: row.get(2)?,
            avatar_url: row.get(3)?,
            provider: row.get(4)?,
            is_active: row.get::<_, i64>(5)? != 0,
        })
    })
    .map_err(|e| format!("query account list: {e}"))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| format!("collect account list: {e}"))
}

pub fn get_caldav_settings_info(
    conn: &Connection,
    account_id: &str,
    encryption_key: &[u8; 32],
) -> Result<Option<AccountCaldavSettingsInfo>, String> {
    conn.query_row(
        "SELECT id, email, caldav_url, caldav_username, caldav_password, calendar_provider \
         FROM accounts WHERE id = ?1",
        rusqlite::params![account_id],
        |row| {
            let password_raw: Option<String> = row.get(4)?;
            let caldav_password = password_raw.map(|raw| {
                if is_encrypted(&raw) {
                    decrypt_value(encryption_key, &raw).unwrap_or(raw)
                } else {
                    raw
                }
            });

            Ok(AccountCaldavSettingsInfo {
                id: row.get(0)?,
                email: row.get(1)?,
                caldav_url: row.get(2)?,
                caldav_username: row.get(3)?,
                caldav_password,
                calendar_provider: row.get(5)?,
            })
        },
    )
    .optional()
    .map_err(|e| format!("query account caldav settings info: {e}"))
}

pub fn get_oauth_credentials(
    conn: &Connection,
    account_id: &str,
    encryption_key: &[u8; 32],
) -> Result<Option<AccountOAuthCredentials>, String> {
    conn.query_row(
        "SELECT provider, oauth_client_id, oauth_client_secret
         FROM accounts
         WHERE id = ?1",
        rusqlite::params![account_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        },
    )
    .optional()
    .map_err(|e| format!("query account oauth credentials: {e}"))
    .map(|result| {
        result.and_then(|(provider, client_id, client_secret)| {
            if provider != "gmail_api" && provider != "graph" {
                return None;
            }

            let client_id =
                client_id
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| {
                        if is_encrypted(&value) {
                            decrypt_value(encryption_key, &value).unwrap_or(value)
                        } else {
                            value
                        }
                    })?;
            let client_secret =
                client_secret
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| {
                        if is_encrypted(&value) {
                            decrypt_value(encryption_key, &value).unwrap_or(value)
                        } else {
                            value
                        }
                    });

            Some(AccountOAuthCredentials {
                client_id,
                client_secret,
            })
        })
    })
}
