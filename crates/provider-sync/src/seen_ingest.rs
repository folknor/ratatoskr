use db::db::WriteConn;
use rusqlite::params;
use service_state::WriteDbState;

pub(crate) async fn ingest_from_messages<T: seen::MessageAddresses + Send + Sync + 'static>(
    write_db: &WriteDbState,
    account_id: &str,
    messages: &[T],
) {
    if messages.is_empty() {
        return;
    }

    let deferred = seen::collect_observations_deferred(messages);
    if deferred.is_empty() {
        return;
    }

    let account_id = account_id.to_string();
    if let Err(e) = write_db
        .with_write(move |conn| {
            let self_emails = seen::get_self_emails(&conn.as_read(), &account_id)?;
            let observations = seen::resolve_observations(&deferred, &self_emails);
            ingest_observations(conn, &account_id, &observations)
        })
        .await
    {
        log::warn!("Failed to ingest seen addresses: {e}");
    }
}

fn ingest_observations(
    conn: &WriteConn<'_>,
    account_id: &str,
    observations: &[seen::AddressObservation],
) -> Result<(), String> {
    if observations.is_empty() {
        return Ok(());
    }

    log::debug!(
        "Ingesting {} address observations for account {}",
        observations.len(),
        account_id
    );

    let mut stmt = conn
        .prepare_cached(
            "INSERT INTO seen_addresses
                (email, account_id, display_name, display_name_source,
                 times_sent_to, times_sent_cc, times_received_from, times_received_cc,
                 first_seen_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
             ON CONFLICT(account_id, email) DO UPDATE SET
                times_sent_to = times_sent_to + ?5,
                times_sent_cc = times_sent_cc + ?6,
                times_received_from = times_received_from + ?7,
                times_received_cc = times_received_cc + ?8,
                last_seen_at = MAX(last_seen_at, ?9),
                first_seen_at = MIN(first_seen_at, ?9),
                display_name = CASE
                    WHEN ?4 = 'sent' THEN COALESCE(?3, display_name)
                    WHEN display_name_source = 'sent' THEN display_name
                    ELSE COALESCE(?3, display_name)
                END,
                display_name_source = CASE
                    WHEN ?4 = 'sent' THEN 'sent'
                    WHEN display_name_source = 'sent' THEN display_name_source
                    ELSE ?4
                END",
        )
        .map_err(|e| format!("prepare seen_addresses upsert: {e}"))?;

    for obs in observations {
        let (sent_to, sent_cc, recv_from, recv_cc) = seen::direction_counters(obs.direction);
        let source = seen::direction_source(obs.direction);

        stmt.execute(params![
            obs.email,
            account_id,
            obs.display_name,
            source,
            sent_to,
            sent_cc,
            recv_from,
            recv_cc,
            obs.date_ms,
        ])
        .map_err(|e| format!("upsert seen_address: {e}"))?;
    }

    Ok(())
}
