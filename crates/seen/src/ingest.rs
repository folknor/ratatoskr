use db_read::ReadConn;
use rusqlite::params;

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
pub fn direction_counters(d: Direction) -> (i64, i64, i64, i64) {
    match d {
        Direction::SentTo => (1, 0, 0, 0),
        Direction::SentCc => (0, 1, 0, 0),
        Direction::ReceivedFrom => (0, 0, 1, 0),
        Direction::ReceivedCc => (0, 0, 0, 1),
    }
}

pub fn direction_source(d: Direction) -> &'static str {
    match d {
        Direction::SentTo | Direction::SentCc => "sent",
        Direction::ReceivedFrom | Direction::ReceivedCc => "observed",
    }
}

/// Look up all email addresses belonging to this account (primary + aliases).
pub fn get_self_emails(conn: &ReadConn<'_>, account_id: &str) -> Result<Vec<String>, String> {
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
/// Pre-extract address fields from messages (without direction, since we need
/// self_emails from DB to determine direction).
pub struct DeferredObservation {
    from_address: Option<String>,
    from_name: Option<String>,
    to_addresses: Option<String>,
    cc_addresses: Option<String>,
    bcc_addresses: Option<String>,
    date_ms: i64,
}

pub fn collect_observations_deferred<T: MessageAddresses>(messages: &[T]) -> Vec<DeferredObservation> {
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

pub fn resolve_observations(
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

pub fn resolve_message_observations<T: MessageAddresses>(
    messages: &[T],
    self_emails: &[String],
) -> Vec<AddressObservation> {
    let deferred = collect_observations_deferred(messages);
    resolve_observations(&deferred, self_emails)
}
