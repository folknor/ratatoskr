#![allow(clippy::let_underscore_must_use)]

use std::collections::{HashMap, HashSet};

use tauri::{AppHandle, State};

use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::filters::commands::load_filterable_messages;
use crate::filters::{FilterCriteria, FilterableMessage, message_matches_filter};
use crate::gmail::client::GmailState;
use crate::graph::client::GraphState;
use crate::inline_image_store::InlineImageStoreState;
use crate::jmap::client::JmapState;
use crate::provider::router::{get_ops, get_provider_type};
use crate::provider::types::ProviderCtx;
use crate::search::SearchState;

use super::{AppliedSmartLabelMatch, SmartLabelAIRule, SmartLabelAIThread};

#[allow(clippy::too_many_arguments)]
pub(crate) async fn smart_labels_apply_criteria_to_new_message_ids_impl(
    account_id: &str,
    provider: &str,
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

    let ops = get_ops(
        provider,
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
pub(crate) async fn smart_labels_apply_matches_impl(
    account_id: &str,
    matches: &[AppliedSmartLabelMatch],
    db: &DbState,
    gmail: &GmailState,
    jmap: &JmapState,
    graph: &GraphState,
    body_store: &BodyStoreState,
    inline_images: &InlineImageStoreState,
    search: &SearchState,
    app_handle: &AppHandle,
) -> Result<(), String> {
    if matches.is_empty() {
        return Ok(());
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

    for applied in matches {
        for label_id in &applied.label_ids {
            if let Err(error) = ops.add_tag(&ctx, &applied.thread_id, label_id).await {
                log::warn!(
                    "Failed to apply smart label {label_id} to thread {}: {error}",
                    applied.thread_id
                );
            }
        }
    }

    Ok(())
}

pub(crate) async fn smart_labels_prepare_ai_remainder_impl(
    account_id: &str,
    message_ids: &[String],
    db: &DbState,
    body_store: &BodyStoreState,
    pre_applied_matches: &[AppliedSmartLabelMatch],
) -> Result<(Vec<SmartLabelAIThread>, Vec<SmartLabelAIRule>), String> {
    if message_ids.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let rules = load_enabled_rules_for_ai(db, account_id).await?;
    if rules.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let needs_body = rules.iter().any(|rule| rule.criteria.as_ref().and_then(|c| c.body.as_ref()).is_some());
    let messages = load_filterable_messages(db, body_store, account_id, message_ids, needs_body).await?;
    if messages.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    // Fetch thread snippets so AI gets ~100-char previews, not full body text
    let thread_ids: Vec<String> = messages.iter().map(|m| m.thread_id.clone()).collect();
    let snippets = load_thread_snippets(db, account_id, &thread_ids).await?;

    let pre_applied_matches = pre_applied_matches.to_vec();
    let prepared = tokio::task::spawn_blocking(move || {
        prepare_ai_remainder(messages, rules, &pre_applied_matches, snippets)
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))?;

    Ok(prepared)
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
        &get_provider_type(&db, &account_id).await?,
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

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn smart_labels_apply_matches(
    account_id: String,
    matches: Vec<AppliedSmartLabelMatch>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    smart_labels_apply_matches_impl(
        &account_id,
        &matches,
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

#[derive(Debug)]
struct EnabledSmartLabelRule {
    label_id: String,
    description: String,
    criteria: Option<FilterCriteria>,
}

async fn load_enabled_rules_for_ai(
    db: &DbState,
    account_id: &str,
) -> Result<Vec<EnabledSmartLabelRule>, String> {
    let account_id = account_id.to_string();
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT label_id, ai_description, criteria_json FROM smart_label_rules
                 WHERE account_id = ?1 AND is_enabled = 1
                 ORDER BY sort_order, created_at",
            )
            .map_err(|e| format!("prepare smart label ai query: {e}"))?;

        let rows = stmt
            .query_map(rusqlite::params![account_id], |row| {
                let label_id: String = row.get(0)?;
                let description: String = row.get(1)?;
                let criteria_json: Option<String> = row.get(2)?;
                Ok((label_id, description, criteria_json))
            })
            .map_err(|e| format!("query smart label ai rules: {e}"))?;

        let mut rules = Vec::new();
        for row in rows {
            let (label_id, description, criteria_json) =
                row.map_err(|e| format!("read smart label ai row: {e}"))?;
            let criteria = criteria_json
                .as_deref()
                .and_then(|json| serde_json::from_str::<FilterCriteria>(json).ok());
            rules.push(EnabledSmartLabelRule {
                label_id,
                description,
                criteria,
            });
        }

        Ok(rules)
    })
    .await
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

    let mut ordered_matches: Vec<(String, Vec<String>)> = matches
        .into_iter()
        .map(|(thread_id, label_ids)| {
            let mut label_ids: Vec<String> = label_ids.into_iter().collect();
            label_ids.sort();
            (thread_id, label_ids)
        })
        .collect();
    ordered_matches.sort_by(|left, right| left.0.cmp(&right.0));
    ordered_matches
}

async fn load_thread_snippets(
    db: &DbState,
    account_id: &str,
    thread_ids: &[String],
) -> Result<HashMap<String, String>, String> {
    if thread_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let account_id = account_id.to_string();
    let ids = thread_ids.to_vec();
    db.with_conn(move |conn| {
        let mut map = HashMap::new();
        for chunk in ids.chunks(500) {
            let placeholders: String = chunk
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 2))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT id, snippet FROM threads WHERE account_id = ?1 AND id IN ({placeholders})"
            );
            let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            params.push(Box::new(account_id.clone()));
            for id in chunk {
                params.push(Box::new(id.clone()));
            }
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
            let rows = stmt
                .query_map(param_refs.as_slice(), |row| {
                    let id: String = row.get(0)?;
                    let snippet: Option<String> = row.get(1)?;
                    Ok((id, snippet))
                })
                .map_err(|e| e.to_string())?;
            for row in rows {
                let (id, snippet) = row.map_err(|e| e.to_string())?;
                if let Some(s) = snippet {
                    map.insert(id, s);
                }
            }
        }
        Ok(map)
    })
    .await
}

fn prepare_ai_remainder(
    messages: Vec<FilterableMessage>,
    rules: Vec<EnabledSmartLabelRule>,
    pre_applied_matches: &[AppliedSmartLabelMatch],
    thread_snippets: HashMap<String, String>,
) -> (Vec<SmartLabelAIThread>, Vec<SmartLabelAIRule>) {
    let mut thread_map: HashMap<String, FilterableMessage> = HashMap::new();
    for message in messages {
        thread_map.entry(message.thread_id.clone()).or_insert(message);
    }

    let mut matched_labels: HashMap<String, HashSet<String>> = HashMap::new();
    for applied in pre_applied_matches {
        let existing = matched_labels.entry(applied.thread_id.clone()).or_default();
        for label_id in &applied.label_ids {
            existing.insert(label_id.clone());
        }
    }

    for (thread_id, message) in &thread_map {
        for rule in &rules {
            if matched_labels
                .get(thread_id)
                .is_some_and(|labels| labels.contains(&rule.label_id))
            {
                continue;
            }

            if let Some(criteria) = rule.criteria.as_ref() {
                if message_matches_filter(message, criteria) {
                    matched_labels
                        .entry(thread_id.clone())
                        .or_default()
                        .insert(rule.label_id.clone());
                }
            }
        }
    }

    let ai_rules: Vec<SmartLabelAIRule> = rules
        .iter()
        .map(|rule| SmartLabelAIRule {
            label_id: rule.label_id.clone(),
            description: rule.description.clone(),
        })
        .collect();

    let ai_threads = thread_map
        .into_iter()
        .filter_map(|(thread_id, message)| {
            let all_rules_matched = ai_rules.iter().all(|rule| {
                matched_labels
                    .get(&thread_id)
                    .is_some_and(|labels| labels.contains(&rule.label_id))
            });
            if all_rules_matched {
                return None;
            }

            Some(SmartLabelAIThread {
                id: thread_id.clone(),
                subject: message.subject.unwrap_or_default(),
                snippet: thread_snippets
                    .get(&thread_id)
                    .cloned()
                    .unwrap_or_default(),
                from_address: message.from_address.unwrap_or_default(),
            })
        })
        .collect();

    (ai_threads, ai_rules)
}
