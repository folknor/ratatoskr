use rusqlite::{Connection, params};

use ratatoskr_db::db::DbState;

use super::parse::extract_observations;
use super::types::{AddressObservation, Direction, ObservationParams};

/// Trait for extracting address fields from any provider's parsed message type.
pub trait MessageAddresses {
    fn sender_address(&self) -> Option<&str>;
    fn sender_name(&self) -> Option<&str>;
    fn to_addresses(&self) -> Option<&str>;
    fn cc_addresses(&self) -> Option<&str>;
    fn bcc_addresses(&self) -> Option<&str>;
    fn msg_date_ms(&self) -> i64;
}

/// Ingest address observations into the seen_addresses table.
///
/// Uses INSERT ON CONFLICT DO UPDATE to increment direction counters
/// and update timestamps. Display name precedence: 'sent' beats 'observed'.
pub fn ingest_observations(
    conn: &Connection,
    account_id: &str,
    observations: &[AddressObservation],
) -> Result<(), String> {
    if observations.is_empty() {
        return Ok(());
    }

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
        let (sent_to, sent_cc, recv_from, recv_cc) = direction_counters(obs.direction);
        let source = direction_source(obs.direction);

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

fn direction_counters(d: Direction) -> (i64, i64, i64, i64) {
    match d {
        Direction::SentTo => (1, 0, 0, 0),
        Direction::SentCc => (0, 1, 0, 0),
        Direction::ReceivedFrom => (0, 0, 1, 0),
        Direction::ReceivedCc => (0, 0, 0, 1),
    }
}

fn direction_source(d: Direction) -> &'static str {
    match d {
        Direction::SentTo | Direction::SentCc => "sent",
        Direction::ReceivedFrom | Direction::ReceivedCc => "observed",
    }
}

/// Look up all email addresses belonging to this account (primary + aliases).
pub(crate) fn get_self_emails(conn: &Connection, account_id: &str) -> Result<Vec<String>, String> {
    let primary: String = conn
        .query_row(
            "SELECT email FROM accounts WHERE id = ?1",
            params![account_id],
            |row| row.get("email"),
        )
        .map_err(|e| format!("get account email: {e}"))?;

    let mut aliases: Vec<String> = Vec::new();
    let mut stmt = conn
        .prepare("SELECT email FROM send_as_aliases WHERE account_id = ?1")
        .map_err(|e| format!("prepare aliases: {e}"))?;
    let rows = stmt
        .query_map(params![account_id], |row| row.get::<_, String>("email"))
        .map_err(|e| format!("query aliases: {e}"))?;
    for row in rows {
        aliases.push(row.map_err(|e| format!("read alias: {e}"))?);
    }

    let mut emails = vec![primary.to_lowercase()];
    for alias in aliases {
        let lower = alias.to_lowercase();
        if !emails.contains(&lower) {
            emails.push(lower);
        }
    }
    Ok(emails)
}

/// Fire-and-forget ingestion from a batch of parsed messages.
///
/// Looks up account email + aliases, extracts observations, and upserts.
/// Errors are logged but do not fail the sync.
pub async fn ingest_from_messages<T: MessageAddresses + Send + Sync + 'static>(
    db: &DbState,
    account_id: &str,
    messages: &[T],
) {
    if messages.is_empty() {
        return;
    }

    // Collect the data we need before moving into spawn_blocking
    let observations = collect_observations_deferred(account_id, messages);
    if observations.is_empty() {
        return;
    }

    let account_id = account_id.to_string();
    if let Err(e) = db
        .with_conn(move |conn| {
            let self_emails = get_self_emails(conn, &account_id)?;
            let resolved = resolve_observations(&observations, &self_emails);
            ingest_observations(conn, &account_id, &resolved)
        })
        .await
    {
        log::warn!("Failed to ingest seen addresses: {e}");
    }
}

/// Pre-extract address fields from messages (without direction, since we need
/// self_emails from DB to determine direction).
struct DeferredObservation {
    from_address: Option<String>,
    from_name: Option<String>,
    to_addresses: Option<String>,
    cc_addresses: Option<String>,
    bcc_addresses: Option<String>,
    date_ms: i64,
}

fn collect_observations_deferred<T: MessageAddresses>(
    _account_id: &str,
    messages: &[T],
) -> Vec<DeferredObservation> {
    messages
        .iter()
        .map(|m| DeferredObservation {
            from_address: m.sender_address().map(ToString::to_string),
            from_name: m.sender_name().map(ToString::to_string),
            to_addresses: m.to_addresses().map(ToString::to_string),
            cc_addresses: m.cc_addresses().map(ToString::to_string),
            bcc_addresses: m.bcc_addresses().map(ToString::to_string),
            date_ms: m.msg_date_ms(),
        })
        .collect()
}

fn resolve_observations(
    deferred: &[DeferredObservation],
    self_emails: &[String],
) -> Vec<AddressObservation> {
    let mut all = Vec::new();
    for d in deferred {
        let params = ObservationParams {
            self_emails,
            from_address: d.from_address.as_deref(),
            from_name: d.from_name.as_deref(),
            to_addresses: d.to_addresses.as_deref(),
            cc_addresses: d.cc_addresses.as_deref(),
            bcc_addresses: d.bcc_addresses.as_deref(),
            date_ms: d.date_ms,
        };
        all.extend(extract_observations(&params));
    }
    all
}
