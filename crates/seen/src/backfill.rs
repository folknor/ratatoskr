use rusqlite::params;

use db::db::DbState;

use super::ingest::{get_self_emails, ingest_observations};
use super::parse::extract_observations;
use super::types::ObservationParams;

const BATCH_SIZE: i64 = 1000;

/// Backfill seen_addresses from existing messages for one account.
///
/// Processes messages in batches ordered by date ASC. Tracks completion
/// with a settings key to avoid re-running.
///
/// Returns the number of addresses ingested.
pub async fn backfill_seen_addresses(
    db: &DbState,
    account_id: String,
) -> Result<u64, String> {
    let settings_key = format!("seen_addresses_backfill_{account_id}");

    db.with_conn(move |conn| {
        // Check if already done
        let done: bool = conn
            .query_row(
                "SELECT COUNT(*) AS cnt FROM settings WHERE key = ?1",
                params![settings_key],
                |row| row.get::<_, i64>("cnt"),
            )
            .unwrap_or(0)
            > 0;

        if done {
            log::info!("Seen addresses backfill already completed for {account_id}");
            return Ok(0);
        }

        let self_emails = get_self_emails(conn, &account_id)?;
        let mut total: u64 = 0;
        let mut offset: i64 = 0;

        loop {
            let batch = fetch_message_batch(conn, &account_id, offset)?;
            if batch.is_empty() {
                break;
            }

            let mut observations = Vec::new();
            for row in &batch {
                let params = ObservationParams {
                    self_emails: &self_emails,
                    from_address: row.from_address.as_deref(),
                    from_name: row.from_name.as_deref(),
                    to_addresses: row.to_addresses.as_deref(),
                    cc_addresses: row.cc_addresses.as_deref(),
                    bcc_addresses: row.bcc_addresses.as_deref(),
                    date_ms: row.date_ms,
                };
                observations.extend(extract_observations(&params));
            }

            let count = observations.len() as u64;
            ingest_observations(conn, &account_id, &observations)?;
            total += count;
            offset += BATCH_SIZE;

            if batch.len() < usize::try_from(BATCH_SIZE).unwrap_or(usize::MAX) {
                break;
            }
        }

        // Mark as done
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, '1')",
            params![settings_key],
        )
        .map_err(|e| format!("mark backfill done: {e}"))?;

        log::info!("Backfilled {total} seen address observations for {account_id}");
        Ok(total)
    })
    .await
}

struct MessageRow {
    from_address: Option<String>,
    from_name: Option<String>,
    to_addresses: Option<String>,
    cc_addresses: Option<String>,
    bcc_addresses: Option<String>,
    date_ms: i64,
}

fn fetch_message_batch(
    conn: &rusqlite::Connection,
    account_id: &str,
    offset: i64,
) -> Result<Vec<MessageRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT from_address, from_name, to_addresses, cc_addresses,
                    bcc_addresses, date
             FROM messages
             WHERE account_id = ?1
             ORDER BY date ASC
             LIMIT ?2 OFFSET ?3",
        )
        .map_err(|e| format!("prepare backfill query: {e}"))?;

    let rows = stmt
        .query_map(params![account_id, BATCH_SIZE, offset], |row| {
            Ok(MessageRow {
                from_address: row.get("from_address")?,
                from_name: row.get("from_name")?,
                to_addresses: row.get("to_addresses")?,
                cc_addresses: row.get("cc_addresses")?,
                bcc_addresses: row.get("bcc_addresses")?,
                date_ms: row.get("date")?,
            })
        })
        .map_err(|e| format!("query messages for backfill: {e}"))?;

    let mut batch = Vec::new();
    for row in rows {
        batch.push(row.map_err(|e| format!("read message row: {e}"))?);
    }
    Ok(batch)
}
