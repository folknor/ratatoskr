//! Graph-only auxiliary metadata projection over Bifrost's unified engine.

use bifrost_sync::SyncEngine;
use bifrost_types::{AccountId, CategoryDefinition, MessageReactionState, ObjectId};
use common::types::{ImportanceLevel, LabelKind};
use db::db::ReadDbState;
use db::db::queries_extra::{
    LabelWriteRow, delete_message_reaction, upsert_labels, upsert_message_reaction_update_type,
};
use label_colors::preset_colors;
use service_state::WriteDbState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReactionRowOp {
    Upsert {
        message_id: String,
        reactor_email: String,
        reaction_type: String,
    },
    DeleteOwner {
        message_id: String,
        reactor_email: String,
    },
}

pub(crate) fn category_label_rows(
    defs: &[CategoryDefinition],
    account_id: &str,
) -> Result<Vec<LabelWriteRow>, String> {
    let mut rows = defs
        .iter()
        .enumerate()
        .map(|(index, definition)| {
            let preset = definition.color.as_deref().unwrap_or("None");
            let (server_color_bg, server_color_fg) = if preset == "None" {
                (None, None)
            } else {
                preset_colors::preset_to_hex(preset)
                    .map(|(bg, fg)| (Some(bg.to_string()), Some(fg.to_string())))
                    .unwrap_or((None, None))
            };
            let label = LabelKind::graph_category(&definition.name)?;
            Ok(LabelWriteRow {
                id: label.storage_id(),
                account_id: account_id.to_string(),
                name: definition.name.clone(),
                visible: None,
                sort_order: Some(i64::try_from(index).unwrap_or(0)),
                server_color_bg,
                server_color_fg,
                user_color_bg: None,
                user_color_fg: None,
                is_undeletable: false,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    rows.extend(importance_label_rows(account_id));
    Ok(rows)
}

fn importance_label_rows(account_id: &str) -> Vec<LabelWriteRow> {
    ImportanceLevel::ALL
        .into_iter()
        .map(|level| LabelWriteRow {
            id: level.label_id().to_string(),
            account_id: account_id.to_string(),
            name: level.display_name().to_string(),
            visible: None,
            sort_order: Some(level.sort_order()),
            server_color_bg: None,
            server_color_fg: None,
            user_color_bg: None,
            user_color_fg: None,
            is_undeletable: true,
        })
        .collect()
}

pub(crate) fn classify_reaction_updates(
    succeeded: &[MessageReactionState],
    owner_email: &str,
) -> Vec<ReactionRowOp> {
    let mut ops = Vec::with_capacity(succeeded.len() * 2);
    for state in succeeded {
        let message_id = state.id.0.clone();
        // Legacy trimmed the extended-property value before storing it and
        // treated a blank value as "no reaction"; keep both halves.
        match state
            .owner_reaction
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(reaction_type) => ops.push(ReactionRowOp::Upsert {
                message_id: message_id.clone(),
                reactor_email: owner_email.to_string(),
                reaction_type: reaction_type.to_string(),
            }),
            None => ops.push(ReactionRowOp::DeleteOwner {
                message_id: message_id.clone(),
                reactor_email: owner_email.to_string(),
            }),
        }
        // Faithful legacy behavior: an absent count does not delete its stale
        // `__count__` row. B15 pins this pre-existing semantic explicitly.
        if let Some(count) = state.reactions_count {
            ops.push(ReactionRowOp::Upsert {
                message_id,
                reactor_email: "__count__".to_string(),
                reaction_type: count.to_string(),
            });
        }
    }
    ops
}

pub(crate) async fn run_graph_auxiliary_sync(
    engine: &SyncEngine,
    account_id: &str,
    read_db: &ReadDbState,
    write_db: &WriteDbState,
    initial_sync_completed_before_run: bool,
) -> Result<(), String> {
    let account = AccountId(account_id.to_string());
    if !initial_sync_completed_before_run {
        return import_master_categories(engine, &account, write_db, account_id).await;
    }

    let cycle = sync::state::increment_graph_sync_cycle(&write_db.writer_pool(), account_id)
        .await
        .unwrap_or_else(|error| {
            log::warn!("Graph aux cadence counter failed for account {account_id}: {error}");
            1
        });
    // Legacy logged each half independently and always attempted the other,
    // so a transient reaction failure never suppressed the category refresh
    // that shares cycle 20. Keep that, and surface the first error to the
    // caller only after both halves have had their turn.
    let mut first_error = None;
    if cycle.is_multiple_of(5)
        && let Err(error) =
            refresh_reactions_for_recent_messages(engine, &account, read_db, write_db, account_id)
                .await
    {
        log::warn!("Graph reaction refresh failed for account {account_id}: {error}");
        first_error = Some(error);
    }
    if cycle.is_multiple_of(20)
        && let Err(error) = import_master_categories(engine, &account, write_db, account_id).await
    {
        log::warn!("Graph master category sync failed for account {account_id}: {error}");
        first_error = first_error.or(Some(error));
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

async fn import_master_categories(
    engine: &SyncEngine,
    account: &AccountId,
    write_db: &WriteDbState,
    account_id: &str,
) -> Result<(), String> {
    let definitions = engine
        .category_definitions_list(account)
        .await
        .map_err(|error| format!("Graph master category list: {error}"))?;
    log::info!(
        "[Graph] Label sync for account {account_id}: {} categories fetched",
        definitions.len()
    );
    let rows = category_label_rows(&definitions, account_id)?;
    write_db
        .with_write(move |conn| {
            let tx = conn
                .transaction()
                .map_err(|error| format!("label sync tx: {error}"))?;
            upsert_labels(&tx, &rows)?;
            tx.commit()
                .map_err(|error| format!("label sync commit: {error}"))
        })
        .await
}

async fn refresh_reactions_for_recent_messages(
    engine: &SyncEngine,
    account: &AccountId,
    read_db: &ReadDbState,
    write_db: &WriteDbState,
    account_id: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let message_ids = read_db
        .with_read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT DISTINCT m.id FROM messages m
                 LEFT JOIN message_reactions mr ON mr.message_id = m.id
                   AND mr.account_id = m.account_id AND mr.source = 'exchange_native'
                 WHERE m.account_id = ?1
                   AND (mr.message_id IS NOT NULL OR m.date >= strftime('%s','now','-14 days') * 1000)
                 ORDER BY m.date DESC LIMIT 60",
            ).map_err(|error| format!("prepare reaction refresh query: {error}"))?;
            let rows = stmt.query_map([aid], |row| row.get::<_, String>(0))
                .map_err(|error| format!("query reaction messages: {error}"))?;
            rows.collect::<Result<Vec<_>, _>>().map_err(|error| format!("read reaction message: {error}"))
        })
        .await?;
    if message_ids.is_empty() {
        return Ok(());
    }
    let owner_account = account_id.to_string();
    let owner_email: String = read_db
        .with_read(move |conn| {
            conn.query_row(
                "SELECT email FROM accounts WHERE id = ?1",
                [owner_account],
                |row| row.get(0),
            )
            .map_err(|error| format!("lookup account email: {error}"))
        })
        .await?;
    let ids = message_ids.into_iter().map(ObjectId).collect::<Vec<_>>();
    let outcome = engine
        .message_reactions(account, &ids)
        .await
        .map_err(|error| format!("Graph reaction refresh: {error}"))?;
    // Only the succeeded lane may drive row writes. A failed or uncertain
    // item is NOT a "no reaction" answer, and classifying it as one would
    // delete cached reactions on a transient Graph error (B15 spec 5.2).
    if !outcome.failed().is_empty() || !outcome.uncertain().is_empty() {
        log::warn!(
            "Graph reaction refresh for {account_id}: {} failed, {} uncertain item(s) skipped",
            outcome.failed().len(),
            outcome.uncertain().len(),
        );
    }
    let succeeded = outcome
        .succeeded()
        .iter()
        .map(|success| success.output.clone())
        .collect::<Vec<_>>();
    let ops = classify_reaction_updates(&succeeded, &owner_email);
    if ops.is_empty() {
        return Ok(());
    }
    log::debug!(
        "Graph reaction refresh for {account_id}: applying {} row op(s)",
        ops.len()
    );
    let aid = account_id.to_string();
    write_db
        .with_write(move |conn| {
            let tx = conn
                .transaction()
                .map_err(|error| format!("begin reaction refresh: {error}"))?;
            for op in ops {
                match op {
                    ReactionRowOp::Upsert {
                        message_id,
                        reactor_email,
                        reaction_type,
                    } => upsert_message_reaction_update_type(
                        &tx,
                        &message_id,
                        &aid,
                        &reactor_email,
                        &reaction_type,
                        "exchange_native",
                    )?,
                    ReactionRowOp::DeleteOwner {
                        message_id,
                        reactor_email,
                    } => delete_message_reaction(
                        &tx,
                        &message_id,
                        &aid,
                        &reactor_email,
                        "exchange_native",
                    )?,
                }
            }
            tx.commit()
                .map_err(|error| format!("commit reaction refresh: {error}"))
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_label_rows_include_categories_and_importance() {
        let defs = vec![CategoryDefinition {
            name: "Work".into(),
            color: Some("preset0".into()),
        }];
        let rows = category_label_rows(&defs, "account").expect("rows");
        assert_eq!(rows[0].id, "cat:Work");
        assert_eq!(rows[0].server_color_bg.as_deref(), Some("#e74c3c"));
        assert_eq!(rows.len(), 1 + ImportanceLevel::ALL.len());
    }

    #[test]
    fn category_label_rows_map_the_none_preset_to_null_colours() {
        let defs = vec![CategoryDefinition {
            name: "Uncategorised".into(),
            color: None,
        }];
        let rows = category_label_rows(&defs, "account").expect("rows");
        assert_eq!(rows[0].server_color_bg, None);
        assert_eq!(rows[0].server_color_fg, None);
        assert_eq!(rows[0].sort_order, Some(0));
        assert!(rows[1..].iter().all(|row| row.is_undeletable));
    }

    fn state(owner: Option<&str>, count: Option<i64>) -> MessageReactionState {
        MessageReactionState {
            id: ObjectId("message".into()),
            owner_reaction: owner.map(ToString::to_string),
            reactions_count: count,
        }
    }

    #[test]
    fn classify_reaction_updates_only_uses_succeeded_items() {
        let updates = vec![state(Some("like"), Some(2))];
        assert_eq!(
            classify_reaction_updates(&updates, "owner@example.test"),
            vec![
                ReactionRowOp::Upsert {
                    message_id: "message".into(),
                    reactor_email: "owner@example.test".into(),
                    reaction_type: "like".into(),
                },
                ReactionRowOp::Upsert {
                    message_id: "message".into(),
                    reactor_email: "__count__".into(),
                    reaction_type: "2".into(),
                },
            ],
        );
        // A failed or uncertain batch item never reaches this function, so an
        // outcome carrying only those lanes produces no ops at all - it must
        // NOT be mistaken for "the server reports no reactions".
        assert!(classify_reaction_updates(&[], "owner@example.test").is_empty());
    }

    #[test]
    fn classify_reaction_updates_deletes_owner_but_never_the_count_row() {
        // B15 spec 2.8 bug 1, ported faithfully: an absent owner property
        // deletes the owner row, an absent count leaves a stale `__count__`
        // row in place because legacy had no delete branch for it.
        assert_eq!(
            classify_reaction_updates(&[state(None, None)], "owner@example.test"),
            vec![ReactionRowOp::DeleteOwner {
                message_id: "message".into(),
                reactor_email: "owner@example.test".into(),
            }],
        );
        assert_eq!(
            classify_reaction_updates(&[state(Some("  "), Some(0))], "owner@example.test"),
            vec![
                ReactionRowOp::DeleteOwner {
                    message_id: "message".into(),
                    reactor_email: "owner@example.test".into(),
                },
                ReactionRowOp::Upsert {
                    message_id: "message".into(),
                    reactor_email: "__count__".into(),
                    reaction_type: "0".into(),
                },
            ],
        );
    }

    #[test]
    fn classify_reaction_updates_trims_the_owner_property_value() {
        assert_eq!(
            classify_reaction_updates(&[state(Some(" like "), None)], "owner@example.test"),
            vec![ReactionRowOp::Upsert {
                message_id: "message".into(),
                reactor_email: "owner@example.test".into(),
                reaction_type: "like".into(),
            }],
        );
    }
}
