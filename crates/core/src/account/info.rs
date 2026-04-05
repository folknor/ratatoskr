use crate::db::Connection;
use crate::db::queries_extra::{get_account_sync, get_all_accounts_sync};
use crate::db::types::DbAccount;
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
    let Some(account) = get_account_sync(conn, account_id)?
    else {
        return Ok(None);
    };

    let server_url = account
        .caldav_url
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "CalDAV credentials not configured".to_string())?;
    let password_raw = account
        .caldav_password
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "CalDAV credentials not configured".to_string())?;
    let password = if is_encrypted(&password_raw) {
        decrypt_value(encryption_key, &password_raw).unwrap_or(password_raw)
    } else {
        password_raw
    };
    let username = account
        .caldav_username
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(account.email);

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
    get_account_sync(conn, account_id).map(|account| account.map(map_basic_info))
}

pub fn list_basic_info(conn: &Connection) -> Result<Vec<AccountBasicInfo>, String> {
    get_all_accounts_sync(conn).map(|accounts| accounts.into_iter().map(map_basic_info).collect())
}

pub fn get_caldav_settings_info(
    conn: &Connection,
    account_id: &str,
    encryption_key: &[u8; 32],
) -> Result<Option<AccountCaldavSettingsInfo>, String> {
    get_account_sync(conn, account_id).map(|account| {
        account.map(|account| {
            let caldav_password = account.caldav_password.map(|raw| {
                if is_encrypted(&raw) {
                    decrypt_value(encryption_key, &raw).unwrap_or(raw)
                } else {
                    raw
                }
            });

            AccountCaldavSettingsInfo {
                id: account.id,
                email: account.email,
                caldav_url: account.caldav_url,
                caldav_username: account.caldav_username,
                caldav_password,
                calendar_provider: account.calendar_provider,
            }
        })
    })
}

pub fn get_oauth_credentials(
    conn: &Connection,
    account_id: &str,
    encryption_key: &[u8; 32],
) -> Result<Option<AccountOAuthCredentials>, String> {
    get_account_sync(conn, account_id).map(|account| {
        account.and_then(|account| {
            if account.provider != "gmail_api" && account.provider != "graph" {
                return None;
            }

            let client_id = account
                .oauth_client_id
                .filter(|value| !value.trim().is_empty())
                .map(|value| {
                    if is_encrypted(&value) {
                        decrypt_value(encryption_key, &value).unwrap_or(value)
                    } else {
                        value
                    }
                })?;
            let client_secret = account
                .oauth_client_secret
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

fn map_basic_info(account: DbAccount) -> AccountBasicInfo {
    AccountBasicInfo {
        id: account.id,
        email: account.email,
        display_name: account.display_name,
        avatar_url: account.avatar_url,
        provider: account.provider,
        is_active: account.is_active != 0,
    }
}
