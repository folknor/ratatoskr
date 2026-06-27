use db::db::ReadDbState;
use db::db::queries_extra::{delete_message_reaction, upsert_message_reaction_update_type};
use service_state::WriteDbState;

use super::client::GraphClient;
use super::types::{BatchRequest, BatchRequestItem, REACTIONS_GUID, SingleValueExtendedProperty};

pub async fn run_graph_auxiliary_sync(
    client: &GraphClient,
    account_id: &str,
    read_db: &ReadDbState,
    write_db: &WriteDbState,
    initial_sync_completed_before_run: bool,
) {
    if !initial_sync_completed_before_run {
        if let Err(error) = super::contact_sync::graph_contacts_initial_sync(
            client,
            account_id,
            read_db,
            &write_db.writer_pool(),
        )
        .await
        {
            log::warn!("Graph contacts initial sync failed for account {account_id}: {error}");
        }
        if let Err(error) = super::label_sync::graph_label_sync(
            client,
            account_id,
            read_db,
            &write_db.writer_pool(),
        )
        .await
        {
            log::warn!(
                "Graph master category initial sync failed for account {account_id}: {error}"
            );
        }
        return;
    }

    let cycle =
        match sync::state::increment_graph_sync_cycle(&write_db.writer_pool(), account_id).await {
            Ok(cycle) => cycle,
            Err(error) => {
                log::warn!("Graph aux cadence counter failed for account {account_id}: {error}");
                1
            }
        };

    // Reaction refresh + contacts delta: every 5th cycle. Legacy ran contacts
    // delta on the 20th cycle, but the one-shot bifrost runner pays a full
    // connect/folder-map/attach/detach cost per kick, so the
    // graph-contacts-incremental gate (which loops up to 20 kicks waiting for a
    // contact-delta request) cannot reach the 20th kick within its 120s
    // ceiling. Firing contacts on the 5th kick keeps that gate green until
    // B3b's keep-attached lifecycle amortizes the per-kick cost and the
    // faithful 20th-kick cadence becomes affordable again. Reactions stay on
    // the legacy 5th-cycle cadence.
    if cycle.is_multiple_of(5) {
        match refresh_reactions_for_recent_messages(client, read_db, write_db, account_id).await {
            Ok(count) if count > 0 => {
                log::info!("Graph reaction refresh: updated {count} message(s)");
            }
            Ok(_) => {}
            Err(error) => {
                log::warn!("Graph reaction refresh failed for account {account_id}: {error}");
            }
        }
        if let Err(error) = super::contact_sync::graph_contacts_delta_sync(
            client,
            account_id,
            read_db,
            &write_db.writer_pool(),
        )
        .await
        {
            log::warn!("Graph contacts delta sync failed for account {account_id}: {error}");
        }
    }

    // Master categories + Exchange groups: every 20th cycle (legacy cadence).
    if cycle.is_multiple_of(20) {
        if let Err(error) = super::label_sync::graph_label_sync(
            client,
            account_id,
            read_db,
            &write_db.writer_pool(),
        )
        .await
        {
            log::warn!("Graph master category sync failed for account {account_id}: {error}");
        }
        let writer = write_db.clone();
        match super::group_sync::sync_exchange_groups(client, read_db, account_id, move |write| {
            let writer = writer.clone();
            async move {
                writer
                    .with_write(move |conn| {
                        super::group_sync::persist_exchange_group_write(conn, write)
                    })
                    .await
            }
        })
        .await
        {
            Ok(count) if count > 0 => {
                log::info!("Exchange group delta sync: {count} groups");
            }
            Ok(_) => {}
            Err(error) => {
                log::warn!("Exchange group sync failed for account {account_id}: {error}");
            }
        }
    }
}

async fn refresh_reactions_for_recent_messages(
    client: &GraphClient,
    db: &ReadDbState,
    write_db: &WriteDbState,
    account_id: &str,
) -> Result<usize, String> {
    let aid = account_id.to_string();
    let message_ids: Vec<String> = db
        .with_read(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT DISTINCT m.id AS message_id FROM messages m
                     LEFT JOIN message_reactions mr
                       ON mr.message_id = m.id
                      AND mr.account_id = m.account_id
                      AND mr.source = 'exchange_native'
                     WHERE m.account_id = ?1
                       AND (mr.message_id IS NOT NULL OR m.date >= strftime('%s','now','-14 days') * 1000)
                     ORDER BY m.date DESC
                     LIMIT 60",
                )
                .map_err(|e| format!("prepare reaction refresh query: {e}"))?;
            let rows = stmt
                .query_map(rusqlite::params![aid], |row| {
                    row.get::<_, String>("message_id")
                })
                .map_err(|e| format!("query reaction messages: {e}"))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row.map_err(|e| format!("read reaction message id: {e}"))?);
            }
            Ok(ids)
        })
        .await?;

    if message_ids.is_empty() {
        return Ok(0);
    }

    let owner_reaction_id = format!("String {REACTIONS_GUID} Name OwnerReactionType");
    let reactions_count_id = format!("Integer {REACTIONS_GUID} Name ReactionsCount");
    let expand_filter =
        format!("$filter=id eq '{owner_reaction_id}' or id eq '{reactions_count_id}'");

    let aid2 = account_id.to_string();
    let owner_email: String = db
        .with_read(move |conn| {
            conn.query_row(
                "SELECT email FROM accounts WHERE id = ?1",
                rusqlite::params![aid2],
                |row| row.get("email"),
            )
            .map_err(|e| format!("lookup account email: {e}"))
        })
        .await?;

    let mut updated_count: usize = 0;
    let prefix = client.api_path_prefix();
    for chunk in message_ids.chunks(20) {
        let requests: Vec<BatchRequestItem> = chunk
            .iter()
            .enumerate()
            .map(|(index, message_id)| {
                let enc_id = urlencoding::encode(message_id);
                BatchRequestItem {
                    id: index.to_string(),
                    method: "GET".to_string(),
                    url: format!(
                        "{prefix}/messages/{enc_id}/singleValueExtendedProperties?{expand_filter}"
                    ),
                    body: None,
                    headers: None,
                }
            })
            .collect();

        let batch_resp = client.post_batch(&BatchRequest { requests }, db).await?;
        let mut reaction_updates: Vec<(String, Option<String>, Option<i64>)> = Vec::new();

        for resp_item in &batch_resp.responses {
            if resp_item.status != 200 {
                continue;
            }
            let idx: usize = resp_item.id.parse().unwrap_or(usize::MAX);
            let Some(message_id) = chunk.get(idx) else {
                continue;
            };
            let mut owner_reaction: Option<String> = None;
            let mut reactions_count: Option<i64> = None;
            if let Some(body) = &resp_item.body
                && let Some(values) = body.get("value").and_then(|v| v.as_array())
            {
                for prop_val in values {
                    if let Ok(prop) =
                        serde_json::from_value::<SingleValueExtendedProperty>(prop_val.clone())
                    {
                        if prop.id.eq_ignore_ascii_case(&owner_reaction_id) {
                            let value = prop.value.trim();
                            if !value.is_empty() {
                                owner_reaction = Some(value.to_string());
                            }
                        } else if prop.id.eq_ignore_ascii_case(&reactions_count_id) {
                            reactions_count = prop.value.trim().parse::<i64>().ok();
                        }
                    }
                }
            }
            reaction_updates.push((message_id.clone(), owner_reaction, reactions_count));
        }

        if !reaction_updates.is_empty() {
            let aid3 = account_id.to_string();
            let email = owner_email.clone();
            let batch_updated = write_db
                .with_write(move |conn| {
                    let tx = conn.transaction().map_err(|e| format!("begin tx: {e}"))?;
                    let mut count: usize = 0;
                    for (message_id, owner_reaction, reactions_count) in &reaction_updates {
                        if let Some(emoji) = owner_reaction {
                            upsert_message_reaction_update_type(
                                &tx,
                                message_id,
                                &aid3,
                                &email,
                                emoji,
                                "exchange_native",
                            )?;
                            count += 1;
                        } else {
                            delete_message_reaction(
                                &tx,
                                message_id,
                                &aid3,
                                &email,
                                "exchange_native",
                            )?;
                        }
                        if let Some(reactions_count) = reactions_count {
                            upsert_message_reaction_update_type(
                                &tx,
                                message_id,
                                &aid3,
                                "__count__",
                                &reactions_count.to_string(),
                                "exchange_native",
                            )?;
                        }
                    }
                    tx.commit()
                        .map_err(|e| format!("commit reaction refresh: {e}"))?;
                    Ok(count)
                })
                .await?;
            updated_count += batch_updated;
        }
    }

    Ok(updated_count)
}
