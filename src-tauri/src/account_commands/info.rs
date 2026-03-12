use rusqlite::OptionalExtension;
use tauri::{AppHandle, Manager, State};

use crate::attachment_cache;
use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::gmail::client::GmailState;
use crate::graph::client::GraphState;
use crate::inline_image_store::InlineImageStoreState;
use crate::jmap::client::JmapState;
use crate::provider::crypto::{decrypt_value, is_encrypted};
use crate::sync::config;

use super::types::{
    AccountBasicInfo, AccountCaldavSettingsInfo, CaldavConnectionInfo, CalendarProviderInfo,
};

#[tauri::command]
pub async fn account_get_calendar_provider_info(
    db: State<'_, DbState>,
    account_id: String,
) -> Result<Option<CalendarProviderInfo>, String> {
    db.with_conn(move |conn| {
        let account = config::get_account(conn, &account_id)?;
        Ok(
            config::calendar_provider_kind(&account).map(|provider| CalendarProviderInfo {
                provider: provider.to_string(),
            }),
        )
    })
    .await
}

#[tauri::command]
pub async fn account_get_caldav_connection_info(
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    account_id: String,
) -> Result<Option<CaldavConnectionInfo>, String> {
    let encryption_key = *gmail.encryption_key();
    db.with_conn(move |conn| {
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
            decrypt_value(&encryption_key, &password_raw).unwrap_or(password_raw)
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
    })
    .await
}

#[tauri::command]
pub async fn account_get_basic_info(
    db: State<'_, DbState>,
    account_id: String,
) -> Result<Option<AccountBasicInfo>, String> {
    db.with_conn(move |conn| {
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
    })
    .await
}

#[tauri::command]
pub async fn account_list_basic_info(
    db: State<'_, DbState>,
) -> Result<Vec<AccountBasicInfo>, String> {
    db.with_conn(move |conn| {
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
    })
    .await
}

#[tauri::command]
pub async fn account_delete(
    app_handle: AppHandle,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    inline_images: State<'_, InlineImageStoreState>,
    account_id: String,
) -> Result<(), String> {
    let (message_ids, cached_files, inline_hashes) = db
        .with_conn({
            let account_id = account_id.clone();
            move |conn| {
                let message_ids = {
                    let mut stmt = conn
                        .prepare("SELECT id FROM messages WHERE account_id = ?1")
                        .map_err(|e| format!("prepare account message query: {e}"))?;
                    stmt.query_map(rusqlite::params![&account_id], |row| {
                        row.get::<_, String>(0)
                    })
                    .map_err(|e| format!("query account message ids: {e}"))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("collect account message ids: {e}"))?
                };

                let cached_files = {
                    let mut stmt = conn
                        .prepare(
                            "SELECT DISTINCT local_path, content_hash
                             FROM attachments
                             WHERE account_id = ?1
                               AND cached_at IS NOT NULL
                               AND local_path IS NOT NULL
                               AND content_hash IS NOT NULL",
                        )
                        .map_err(|e| format!("prepare account cached attachment query: {e}"))?;
                    stmt.query_map(rusqlite::params![&account_id], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .map_err(|e| format!("query account cached attachments: {e}"))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("collect account cached attachments: {e}"))?
                };

                let inline_hashes = {
                    let mut stmt = conn
                        .prepare(
                            "SELECT DISTINCT content_hash
                             FROM attachments
                             WHERE account_id = ?1
                               AND is_inline = 1
                               AND content_hash IS NOT NULL",
                        )
                        .map_err(|e| format!("prepare account inline hash query: {e}"))?;
                    stmt.query_map(rusqlite::params![&account_id], |row| {
                        row.get::<_, String>(0)
                    })
                    .map_err(|e| format!("query account inline hashes: {e}"))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("collect account inline hashes: {e}"))?
                };

                Ok((message_ids, cached_files, inline_hashes))
            }
        })
        .await?;

    body_store.delete(message_ids).await?;

    db.with_conn({
        let account_id = account_id.clone();
        move |conn| {
            conn.execute(
                "DELETE FROM accounts WHERE id = ?1",
                rusqlite::params![account_id],
            )
            .map_err(|e| format!("delete account: {e}"))?;
            Ok(())
        }
    })
    .await?;

    for (local_path, content_hash) in cached_files {
        let remaining_refs: i64 = db
            .with_conn({
                let content_hash = content_hash.clone();
                move |conn| {
                    conn.query_row(
                        "SELECT COUNT(*) FROM attachments
                         WHERE content_hash = ?1 AND cached_at IS NOT NULL",
                        rusqlite::params![content_hash],
                        |row| row.get(0),
                    )
                    .map_err(|e| format!("count remaining cached attachment refs: {e}"))
                }
            })
            .await?;
        if remaining_refs == 0 {
            let app_data_dir = app_handle
                .path()
                .app_data_dir()
                .map_err(|e| format!("resolve app data dir: {e}"))?;
            let _ = attachment_cache::remove_cached_relative(&app_data_dir, &local_path);
        }
    }

    inline_images
        .delete_unreferenced(&db, inline_hashes)
        .await?;

    gmail.remove(&account_id).await;
    jmap.remove(&account_id).await;
    graph.remove(&account_id).await;
    Ok(())
}

#[tauri::command]
pub async fn account_get_caldav_settings_info(
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    account_id: String,
) -> Result<Option<AccountCaldavSettingsInfo>, String> {
    let encryption_key = *gmail.encryption_key();
    db.with_conn(move |conn| {
        conn.query_row(
            "SELECT id, email, caldav_url, caldav_username, caldav_password, calendar_provider \
             FROM accounts WHERE id = ?1",
            rusqlite::params![account_id],
            |row| {
                let password_raw: Option<String> = row.get(4)?;
                let caldav_password = password_raw.map(|raw| {
                    if is_encrypted(&raw) {
                        decrypt_value(&encryption_key, &raw).unwrap_or(raw)
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
    })
    .await
}
