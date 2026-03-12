#![allow(clippy::let_underscore_must_use)]

use std::collections::HashMap;

use futures::stream::{self, StreamExt};
use tauri::{AppHandle, State};

use crate::body_store::BodyStoreState;
use crate::progress::{ProgressReporter, TauriProgressReporter};

use crate::db::DbState;
use crate::provider::registry::ProviderRegistry;
use crate::provider::router::get_provider_type;
use crate::provider::types::ProviderCtx;
use crate::state::AppState;

use super::{
    FilterActions, FilterCriteria, FilterResult, FilterableMessage, evaluate_filters,
    message_matches_filter_without_body,
};

const POST_SYNC_ACTION_CONCURRENCY: usize = 8;

/// Evaluate enabled filters for an account against a set of messages.
/// Reads filter rules from DB, runs matching in Rust, returns per-thread actions.
/// The caller (TS) is responsible for applying the actions via emailActions.
#[tauri::command]
pub async fn filters_evaluate(
    state: State<'_, DbState>,
    account_id: String,
    messages: Vec<FilterableMessage>,
) -> Result<HashMap<String, FilterResult>, String> {
    if messages.is_empty() {
        return Ok(HashMap::new());
    }

    // Read enabled filters from DB
    let filters = load_enabled_filters(&state, &account_id).await?;

    if filters.is_empty() {
        return Ok(HashMap::new());
    }

    // Run matching on blocking thread (CPU-bound for large filter sets)
    tokio::task::spawn_blocking(move || Ok(evaluate_filters(&filters, &messages)))
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn filters_apply_to_new_message_ids_impl(
    account_id: &str,
    provider: &str,
    message_ids: &[String],
    app_state: &AppState,
    progress: &dyn ProgressReporter,
) -> Result<(), String> {
    if message_ids.is_empty() {
        return Ok(());
    }

    let filters = load_enabled_filters(&app_state.db, account_id).await?;
    if filters.is_empty() {
        return Ok(());
    }

    let body_criteria: Vec<FilterCriteria> = filters
        .iter()
        .filter_map(|(criteria, _)| criteria.body.as_ref().map(|_| criteria.clone()))
        .collect();
    let messages = load_filterable_messages(
        &app_state.db,
        &app_state.body_store,
        account_id,
        message_ids,
        &body_criteria,
    )
    .await?;
    filters_apply_to_messages_impl(
        account_id, provider, &filters, &messages, app_state, progress,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn filters_apply_to_new_message_ids(
    account_id: String,
    message_ids: Vec<String>,
    app_state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    filters_apply_to_new_message_ids_impl(
        &account_id,
        &get_provider_type(&app_state.db, &account_id).await?,
        &message_ids,
        &app_state,
        &reporter,
    )
    .await
}

pub(crate) async fn load_enabled_filters(
    state: &DbState,
    account_id: &str,
) -> Result<Vec<(FilterCriteria, FilterActions)>, String> {
    let account_id = account_id.to_string();
    state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT criteria_json, actions_json FROM filter_rules \
                     WHERE account_id = ?1 AND is_enabled = 1 \
                     ORDER BY sort_order, created_at",
                )
                .map_err(|e| format!("prepare filters query: {e}"))?;

            let rows = stmt
                .query_map(rusqlite::params![account_id], |row| {
                    let criteria_json: String = row.get(0)?;
                    let actions_json: String = row.get(1)?;
                    Ok((criteria_json, actions_json))
                })
                .map_err(|e| format!("query filters: {e}"))?;

            let mut filters = Vec::new();
            for row in rows {
                let (criteria_json, actions_json) =
                    row.map_err(|e| format!("read filter row: {e}"))?;
                let criteria: FilterCriteria = match serde_json::from_str(&criteria_json) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let actions: FilterActions = match serde_json::from_str(&actions_json) {
                    Ok(a) => a,
                    Err(_) => continue,
                };
                filters.push((criteria, actions));
            }

            Ok(filters)
        })
        .await
}

#[derive(Debug)]
struct FilterableMessageRow {
    id: String,
    message: FilterableMessage,
}

pub(crate) async fn load_filterable_messages(
    db: &DbState,
    body_store: &BodyStoreState,
    account_id: &str,
    message_ids: &[String],
    body_criteria: &[FilterCriteria],
) -> Result<Vec<FilterableMessage>, String> {
    let account_id = account_id.to_string();
    let ids = message_ids.to_vec();
    let mut rows = db
        .with_conn(move |conn| {
            let mut all_results = Vec::new();
            for chunk in ids.chunks(500) {
                let placeholders: String = chunk
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", i + 2))
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "SELECT id, thread_id, from_name, from_address, to_addresses, subject, has_attachments
                     FROM messages WHERE account_id = ?1 AND id IN ({placeholders})"
                );
                let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
                let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
                param_values.push(Box::new(account_id.clone()));
                for id in chunk {
                    param_values.push(Box::new(id.clone()));
                }
                let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                    param_values.iter().map(AsRef::as_ref).collect();
                let rows = stmt
                    .query_map(param_refs.as_slice(), |row| {
                        Ok(FilterableMessageRow {
                            id: row.get("id")?,
                            message: FilterableMessage {
                                thread_id: row.get("thread_id")?,
                                from_name: row.get("from_name")?,
                                from_address: row.get("from_address")?,
                                to_addresses: row.get("to_addresses")?,
                                subject: row.get("subject")?,
                                body_text: None,
                                body_html: None,
                                has_attachments: row.get::<_, i64>("has_attachments")? != 0,
                            },
                        })
                    })
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?;
                all_results.extend(rows);
            }
            Ok(all_results)
        })
        .await?;

    if !body_criteria.is_empty() {
        let body_ids: Vec<String> = rows
            .iter()
            .filter(|row| {
                body_criteria
                    .iter()
                    .any(|criteria| message_matches_filter_without_body(&row.message, criteria))
            })
            .map(|row| row.id.clone())
            .collect();
        let bodies = body_store.get_batch(body_ids).await?;
        let body_map: HashMap<String, crate::body_store::MessageBody> = bodies
            .into_iter()
            .map(|body| (body.message_id.clone(), body))
            .collect();

        for row in &mut rows {
            if let Some(body) = body_map.get(&row.id) {
                row.message.body_html = body.body_html.clone();
                row.message.body_text = body.body_text.clone();
            }
        }
    }

    Ok(rows.into_iter().map(|row| row.message).collect())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn filters_apply_to_messages_impl(
    account_id: &str,
    provider: &str,
    filters: &[(FilterCriteria, FilterActions)],
    messages: &[FilterableMessage],
    app_state: &AppState,
    progress: &dyn ProgressReporter,
) -> Result<(), String> {
    if filters.is_empty() || messages.is_empty() {
        return Ok(());
    }

    let filters = filters.to_vec();
    let messages = messages.to_vec();
    let thread_actions = tokio::task::spawn_blocking(move || evaluate_filters(&filters, &messages))
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?;
    if thread_actions.is_empty() {
        return Ok(());
    }

    let ops = app_state.providers.get_ops(provider, account_id).await?;
    let ctx = ProviderCtx {
        account_id,
        db: &app_state.db,
        body_store: &app_state.body_store,
        inline_images: &app_state.inline_images,
        search: &app_state.search,
        progress,
    };
    let ops = &*ops;
    let ctx = &ctx;

    stream::iter(thread_actions)
        .for_each_concurrent(POST_SYNC_ACTION_CONCURRENCY, move |(thread_id, result)| {
            let ops = ops;
            let ctx = ctx;
            async move {
                apply_filter_result(ops, ctx, &thread_id, &result).await;
            }
        })
        .await;

    Ok(())
}

async fn apply_filter_result(
    ops: &dyn crate::provider::ops::ProviderOps,
    ctx: &ProviderCtx<'_>,
    thread_id: &str,
    result: &FilterResult,
) {
    for label_id in &result.add_label_ids {
        if let Err(e) = ops.add_tag(ctx, thread_id, label_id).await {
            log::warn!("Filter: add_tag {label_id} on {thread_id} failed: {e}");
        }
    }

    for label_id in &result.remove_label_ids {
        if let Err(e) = ops.remove_tag(ctx, thread_id, label_id).await {
            log::warn!("Filter: remove_tag {label_id} on {thread_id} failed: {e}");
        }
    }

    if result.mark_read {
        if let Err(e) = ops.mark_read(ctx, thread_id, true).await {
            log::warn!("Filter: mark_read on {thread_id} failed: {e}");
        }
    }

    if result.star {
        if let Err(e) = ops.star(ctx, thread_id, true).await {
            log::warn!("Filter: star on {thread_id} failed: {e}");
        }
    }
}
