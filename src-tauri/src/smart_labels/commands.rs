#![allow(clippy::let_underscore_must_use)]

use std::collections::{HashMap, HashSet};

use tauri::{AppHandle, State};

use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::db::queries::row_to_message;
use crate::filters::{FilterCriteria, FilterableMessage, message_matches_filter};
use crate::gmail::client::GmailState;
use crate::graph::client::GraphState;
use crate::inline_image_store::InlineImageStoreState;
use crate::jmap::client::JmapState;
use crate::provider::router::{get_ops, get_provider_type};
use crate::provider::types::ProviderCtx;
use crate::search::SearchState;

use super::AppliedSmartLabelMatch;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn smart_labels_apply_criteria_to_new_message_ids_impl(
    account_id: &str,
    message_ids: &[String],
    db: &DbState,
    gmail: &GmailState,
    jmap: &JmapState,
    graph: &GraphState,
    body_store: &BodyStoreState,
    inline_images: &InlineImageStoreState,
    search: &SearchState,
    app_handle: &AppHandle,
) -> Result<Vec<AppliedSmartLabelMatch>, String> {
    if message_ids.is_empty() {
        return Ok(Vec::new());
    }

    let rules = load_enabled_criteria_rules(db, account_id).await?;
    if rules.is_empty() {
        return Ok(Vec::new());
    }

    let needs_body = rules.iter().any(|(_, criteria)| criteria.body.is_some());
    let messages = load_filterable_messages(db, body_store, account_id, message_ids, needs_body).await?;
    if messages.is_empty() {
        return Ok(Vec::new());
    }

    let matches = tokio::task::spawn_blocking(move || evaluate_criteria_matches(&rules, &messages))
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?;
    if matches.is_empty() {
        return Ok(Vec::new());
    }

    let provider = get_provider_type(db, account_id).await?;
    let ops = get_ops(
        &provider,
        account_id,
        gmail,
        jmap,
        graph,
        *gmail.encryption_key(),
    )
    .await?;
    let ctx = ProviderCtx {
        account_id,
        db,
        body_store,
        inline_images,
        search,
        app_handle,
    };

    let mut applied_matches = Vec::new();
    for (thread_id, label_ids) in matches {
        let mut applied_label_ids = Vec::new();
        for label_id in label_ids {
            match ops.add_tag(&ctx, &thread_id, &label_id).await {
                Ok(()) => applied_label_ids.push(label_id),
                Err(error) => log::warn!(
                    "Failed to apply smart label {label_id} to thread {thread_id}: {error}"
                ),
            }
        }

        if !applied_label_ids.is_empty() {
            applied_matches.push(AppliedSmartLabelMatch {
                thread_id,
                label_ids: applied_label_ids,
            });
        }
    }

    Ok(applied_matches)
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn smart_labels_apply_criteria_to_new_message_ids(
    account_id: String,
    message_ids: Vec<String>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<Vec<AppliedSmartLabelMatch>, String> {
    smart_labels_apply_criteria_to_new_message_ids_impl(
        &account_id,
        &message_ids,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &app_handle,
    )
    .await
}

async fn load_enabled_criteria_rules(
    db: &DbState,
    account_id: &str,
) -> Result<Vec<(String, FilterCriteria)>, String> {
    let account_id = account_id.to_string();
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT label_id, criteria_json FROM smart_label_rules
                 WHERE account_id = ?1 AND is_enabled = 1 AND criteria_json IS NOT NULL
                 ORDER BY sort_order, created_at",
            )
            .map_err(|e| format!("prepare smart label rules query: {e}"))?;

        let rows = stmt
            .query_map(rusqlite::params![account_id], |row| {
                let label_id: String = row.get(0)?;
                let criteria_json: String = row.get(1)?;
                Ok((label_id, criteria_json))
            })
            .map_err(|e| format!("query smart label rules: {e}"))?;

        let mut rules = Vec::new();
        for row in rows {
            let (label_id, criteria_json) =
                row.map_err(|e| format!("read smart label row: {e}"))?;
            let criteria: FilterCriteria = match serde_json::from_str(&criteria_json) {
                Ok(criteria) => criteria,
                Err(_) => continue,
            };
            if criteria.from.is_none()
                && criteria.to.is_none()
                && criteria.subject.is_none()
                && criteria.body.is_none()
                && criteria.has_attachment.is_none()
            {
                continue;
            }
            rules.push((label_id, criteria));
        }

        Ok(rules)
    })
    .await
}

async fn load_filterable_messages(
    db: &DbState,
    body_store: &BodyStoreState,
    account_id: &str,
    message_ids: &[String],
    needs_body: bool,
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
                    "SELECT * FROM messages WHERE account_id = ?1 AND id IN ({placeholders})"
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
                    .query_map(param_refs.as_slice(), row_to_message)
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?;
                all_results.extend(rows);
            }
            Ok(all_results)
        })
        .await?;

    if needs_body {
        let bodies = body_store.get_batch(message_ids.to_vec()).await?;
        let body_map: HashMap<String, crate::body_store::MessageBody> = bodies
            .into_iter()
            .map(|body| (body.message_id.clone(), body))
            .collect();

        for row in &mut rows {
            if let Some(body) = body_map.get(&row.id) {
                row.body_html = body.body_html.clone();
                row.body_text = body.body_text.clone();
            }
        }
    }

    Ok(rows
        .into_iter()
        .map(|row| FilterableMessage {
            thread_id: row.thread_id,
            from_name: row.from_name,
            from_address: row.from_address,
            to_addresses: row.to_addresses,
            subject: row.subject,
            body_text: row.body_text,
            body_html: row.body_html,
            has_attachments: false,
        })
        .collect())
}

fn evaluate_criteria_matches(
    rules: &[(String, FilterCriteria)],
    messages: &[FilterableMessage],
) -> Vec<(String, Vec<String>)> {
    let mut thread_map: HashMap<String, &FilterableMessage> = HashMap::new();
    for message in messages {
        thread_map
            .entry(message.thread_id.clone())
            .or_insert(message);
    }

    let mut matches: HashMap<String, HashSet<String>> = HashMap::new();
    for (thread_id, message) in thread_map {
        for (label_id, criteria) in rules {
            if message_matches_filter(message, criteria) {
                matches
                    .entry(thread_id.clone())
                    .or_default()
                    .insert(label_id.clone());
            }
        }
    }

    matches
        .into_iter()
        .map(|(thread_id, label_ids)| (thread_id, label_ids.into_iter().collect()))
        .collect()
}
