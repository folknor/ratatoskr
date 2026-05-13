//! `settings.set` request handler (Phase 6a).
//!
//! Writes one or more settings rows in a single atomic transaction.
//! Per-variant `key()` and `render_for_storage()` live on the wire
//! type itself; the handler's job is the boundary crossing + the
//! transaction.
//!
//! Attachments roadmap Phase 6: when `sync_period_days` is written
//! and the new value strictly increases the window, fire
//! `prefetch.kick_backfill_account` for every active non-deleting
//! JMAP account so the slider extension takes immediate effect
//! instead of waiting for the next boot's recovery kick. The kick
//! runs after the transaction commits so a write failure can't trigger
//! a backfill on stale state.

use std::sync::Arc;

use serde_json::Value;
use service_api::{ServiceError, SettingValue, SettingsSetAck, SettingsSetParams};

use crate::boot::BootSharedState;

pub(crate) async fn handle_set(
    boot_state: &Arc<BootSharedState>,
    params: SettingsSetParams,
) -> Result<Value, ServiceError> {
    // Attachments roadmap Phase 6: capture the incoming
    // `sync_period_days` (if any) before the write so we can compare
    // it against the existing stored value after commit and fire a
    // backfill kick on extend.
    let new_window_days: Option<i64> = params.values.iter().find_map(|v| match v {
        SettingValue::SyncPeriodDays(s) => s.parse::<i64>().ok(),
        _ => None,
    });

    let write_db = boot_state.write_db_state()?;
    let old_window_days: Option<i64> = if new_window_days.is_some() {
        write_db
            .with_conn(|conn| {
                Ok(rtsk::db::queries::get_setting(conn, "sync_period_days")
                    .ok()
                    .flatten()
                    .and_then(|s| s.parse::<i64>().ok()))
            })
            .await
            .map_err(ServiceError::Internal)?
    } else {
        None
    };

    write_db
        .with_conn(move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("settings.set begin tx: {e}"))?;
            for value in &params.values {
                let key = value.key();
                let storage_value = value.render_for_storage();
                rtsk::db::queries::set_setting(&tx, key, &storage_value)
                    .map_err(|e| format!("settings.set {key}: {e}"))?;
            }
            tx.commit()
                .map_err(|e| format!("settings.set commit: {e}"))?;
            Ok(())
        })
        .await
        .map_err(ServiceError::Internal)?;

    if let (Some(new_days), Some(old_days)) = (new_window_days, old_window_days)
        && new_days > old_days
    {
        kick_window_extend(boot_state, new_days).await;
    }

    serde_json::to_value(SettingsSetAck)
        .map_err(|e| ServiceError::Internal(e.to_string()))
}

/// Walk every active non-deleting JMAP account with caching enabled
/// and fire `prefetch.kick_backfill_account` against the newly-
/// extended window. Errors are logged but do not surface to the
/// `settings.set` ack - the write itself succeeded and the next
/// boot's recovery kick is the backstop.
async fn kick_window_extend(boot_state: &Arc<BootSharedState>, window_days: i64) {
    let Some(prefetch) = boot_state.prefetch_runtime() else {
        log::debug!("settings.set: PrefetchRuntime not installed; skipping window-extend kick");
        return;
    };
    let Ok(write_db) = boot_state.write_db_state() else {
        return;
    };
    let accounts: Vec<String> = match write_db
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id FROM accounts \
                     WHERE provider = 'jmap' \
                       AND COALESCE(is_active, 1) = 1 \
                       AND COALESCE(is_deleting, 0) = 0 \
                       AND COALESCE(cache_attachments_enabled, 1) = 1",
                )
                .map_err(|e| format!("prepare window-extend account enum: {e}"))?;
            let it = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| format!("query window-extend account enum: {e}"))?;
            let mut out = Vec::new();
            for r in it {
                out.push(r.map_err(|e| format!("row window-extend account enum: {e}"))?);
            }
            Ok(out)
        })
        .await
    {
        Ok(v) => v,
        Err(e) => {
            log::warn!("settings.set window-extend account enum failed: {e}");
            return;
        }
    };
    let window_start_unix =
        chrono::Utc::now().timestamp() - window_days.saturating_mul(86_400);
    for account_id in accounts {
        if let Err(e) = prefetch
            .kick_backfill_account(&account_id, window_start_unix)
            .await
        {
            log::debug!("settings.set window-extend kick {account_id} failed: {e}");
        }
    }
}
