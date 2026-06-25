use bifrost_sync::encode_envelope;
use bifrost_types::{Checkpoint, CursorScope};
use rusqlite::params;
use service_state::WriteDbState;

use super::BifrostProviderKind;
use super::hydrate::ConsumerMessageRow;
use super::write::PersistAffected;

struct SeenMessage {
    from_address: Option<String>,
    from_name: Option<String>,
    to_addresses: Option<String>,
    cc_addresses: Option<String>,
    bcc_addresses: Option<String>,
    date: i64,
}

impl seen::MessageAddresses for SeenMessage {
    fn sender_address(&self) -> Option<&str> {
        self.from_address.as_deref()
    }

    fn sender_name(&self) -> Option<&str> {
        self.from_name.as_deref()
    }

    fn to_addresses(&self) -> Option<&str> {
        self.to_addresses.as_deref()
    }

    fn cc_addresses(&self) -> Option<&str> {
        self.cc_addresses.as_deref()
    }

    fn bcc_addresses(&self) -> Option<&str> {
        self.bcc_addresses.as_deref()
    }

    fn msg_date_ms(&self) -> i64 {
        self.date
    }
}

pub async fn run(
    db: &WriteDbState,
    account_id: &str,
    provider: BifrostProviderKind,
    scope: &CursorScope,
    checkpoint: Option<&Checkpoint>,
    rows: &[ConsumerMessageRow],
    affected: &PersistAffected,
) -> Result<(), String> {
    match provider {
        BifrostProviderKind::Imap => {
            log::debug!(
                "bifrost IMAP post-persist touched {} threads",
                affected.thread_ids.len()
            );
        }
        BifrostProviderKind::Graph => {
            log::debug!(
                "bifrost Graph reaction refresh placeholder touched {} messages",
                affected.message_ids.len()
            );
        }
        BifrostProviderKind::Gmail | BifrostProviderKind::Jmap => {}
    }

    // The seen-ingest counter upsert is the only non-idempotent
    // post-persist effect, so it is gated by a durable marker keyed by
    // (scope, checkpoint). The marker MUST be written in the SAME txn as
    // the increment (B3-spec 4.1.3): if they split, a crash between them
    // double-counts on replay, defeating the marker entirely.
    let marker = checkpoint.map(|cp| MarkerKey::new(account_id, scope, cp));
    ingest_seen_with_marker(db, rows, marker).await
}

/// Resolve and apply the seen-address observations for `rows` AND insert
/// the replay-safety marker in one atomic `with_write` txn. If the marker
/// is already present the increment is skipped (the counters already
/// reflect this checkpoint). A `None` marker (non-advancing batch) runs
/// the increment unconditionally - there is no cursor to replay it from.
async fn ingest_seen_with_marker(
    db: &WriteDbState,
    rows: &[ConsumerMessageRow],
    marker: Option<MarkerKey>,
) -> Result<(), String> {
    let Some(first) = rows.first() else {
        // No message rows: still record the marker so an empty advancing
        // batch is not re-evaluated on replay.
        if let Some(marker) = marker {
            insert_marker(db, marker).await?;
        }
        return Ok(());
    };
    let account_id = first.message.account_id.clone();
    let deferred = seen::collect_observations_deferred(&seen_messages(rows));

    db.with_write(move |conn| {
        if let Some(marker) = &marker {
            let present: bool = conn
                .query_row(
                    "SELECT 1 FROM seen_ingest_markers \
                     WHERE account_id = ?1 AND scope_key = ?2 AND checkpoint_blob = ?3",
                    params![marker.account_id, marker.scope_key, marker.checkpoint_blob],
                    |_| Ok(()),
                )
                .map(|()| true)
                .or_else(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => Ok(false),
                    other => Err(other.to_string()),
                })?;
            if present {
                return Ok(());
            }
        }

        if !deferred.is_empty() {
            let self_emails = seen::get_self_emails(&conn.as_read(), &account_id)?;
            let observations = seen::resolve_observations(&deferred, &self_emails);
            upsert_seen_observations(conn, &account_id, &observations)?;
        }

        if let Some(marker) = &marker {
            conn.execute(
                "INSERT OR IGNORE INTO seen_ingest_markers \
                 (account_id, scope_key, checkpoint_blob) VALUES (?1, ?2, ?3)",
                params![marker.account_id, marker.scope_key, marker.checkpoint_blob],
            )
            .map_err(|error| error.to_string())?;
        }
        Ok(())
    })
    .await
}

fn seen_messages(rows: &[ConsumerMessageRow]) -> Vec<SeenMessage> {
    rows.iter()
        .map(|row| SeenMessage {
            from_address: row.message.from_address.clone(),
            from_name: row.message.from_name.clone(),
            to_addresses: row.message.to_addresses.clone(),
            cc_addresses: row.message.cc_addresses.clone(),
            bcc_addresses: row.message.bcc_addresses.clone(),
            date: row.message.date,
        })
        .collect()
}

fn upsert_seen_observations(
    conn: &db::db::WriteConn<'_>,
    account_id: &str,
    observations: &[seen::AddressObservation],
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

#[derive(Clone)]
struct MarkerKey {
    account_id: String,
    scope_key: String,
    checkpoint_blob: Vec<u8>,
}

impl MarkerKey {
    fn new(account_id: &str, scope: &CursorScope, checkpoint: &Checkpoint) -> Self {
        Self {
            account_id: account_id.to_string(),
            scope_key: format!("{scope:?}"),
            checkpoint_blob: encode_envelope(checkpoint),
        }
    }
}

async fn insert_marker(db: &WriteDbState, marker: MarkerKey) -> Result<(), String> {
    db.with_write(move |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO seen_ingest_markers \
             (account_id, scope_key, checkpoint_blob) VALUES (?1, ?2, ?3)",
            rusqlite::params![marker.account_id, marker.scope_key, marker.checkpoint_blob],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    })
    .await
}

/// Bound the marker table after an ack.
///
/// Deviation from the spec's "delete markers strictly below the acked
/// cursor": `Checkpoint` is an opaque envelope blob at `aa9172d` with no
/// cheap ordering, so we cannot compute "below the cursor" without
/// decoding and comparing every marker. Instead we keep a fixed recent
/// window per scope. This is sound because pruning is explicitly
/// best-effort (a leaked marker only costs one skipped, correct re-ingest,
/// never a double-count) and the in-flight un-acked checkpoint window is
/// tiny in practice. The window must stay comfortably larger than any
/// plausible un-acked backlog so a still-replayable checkpoint's marker is
/// never evicted early.
pub async fn prune_marker_window(
    db: &WriteDbState,
    account_id: &str,
    scope: &CursorScope,
) -> Result<(), String> {
    const MARKER_WINDOW: u32 = 256;
    let account_id = account_id.to_string();
    let scope_key = format!("{scope:?}");
    db.with_write(move |conn| {
        conn.execute(
            "DELETE FROM seen_ingest_markers
             WHERE account_id = ?1 AND scope_key = ?2 AND rowid NOT IN (
                 SELECT rowid FROM seen_ingest_markers
                 WHERE account_id = ?1 AND scope_key = ?2
                 ORDER BY created_at DESC, rowid DESC
                 LIMIT ?3
             )",
            rusqlite::params![account_id, scope_key, MARKER_WINDOW],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    })
    .await
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{MarkerKey, ingest_seen_with_marker, prune_marker_window};
    use crate::bifrost::consumer::hydrate::ConsumerMessageRow;
    use bifrost_types::{ChangeCursor, Checkpoint, CursorScope, OpaqueChangeState, ProtocolKind};
    use db::db::queries_extra::MessageInsertRow;
    use service_state::WriteDbState;

    fn state() -> (WriteDbState, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let pool = db::db::open_writer_pool(tmp.path()).unwrap();
        pool.with_write_sync(|conn| {
            conn.execute(
                "INSERT INTO accounts (id, email, provider) \
                 VALUES ('acc', 'me@example.com', 'jmap')",
                [],
            )
            .unwrap();
            Ok(())
        })
        .unwrap();
        (WriteDbState::from_pool(pool), tmp)
    }

    fn inbound_row() -> ConsumerMessageRow {
        let mut message = MessageInsertRow {
            id: "m1".to_string(),
            account_id: "acc".to_string(),
            thread_id: "m1".to_string(),
            snippet: String::new(),
            date: 1,
            is_read: true,
            ..minimal()
        };
        message.from_address = Some("peer@example.com".to_string());
        message.to_addresses = Some("me@example.com".to_string());
        ConsumerMessageRow {
            search_document: search::SearchDocument {
                message_id: message.id.clone(),
                account_id: message.account_id.clone(),
                thread_id: message.thread_id.clone(),
                subject: message.subject.clone(),
                from_name: message.from_name.clone(),
                from_address: message.from_address.clone(),
                to_addresses: message.to_addresses.clone(),
                body_text: None,
                snippet: Some(message.snippet.clone()),
                date: message.date,
                is_read: message.is_read,
                is_starred: message.is_starred,
                has_attachment: false,
                attachments: Vec::new(),
            },
            message,
            folders: Vec::new(),
            labels: Vec::new(),
            keywords: Vec::new(),
            attachments: Vec::new(),
            body_html: None,
            body_text: None,
            inline_images: Vec::new(),
            is_important: false,
        }
    }

    fn minimal() -> MessageInsertRow {
        // Reuse the hydrate stub shape so the test does not duplicate every
        // MessageInsertRow field.
        match crate::bifrost::consumer::hydrate::hydrate_change_to_message_insert_row_offline(
            "acc",
            crate::bifrost::consumer::BifrostProviderKind::Jmap,
            &bifrost_types::Change::ObjectChange(bifrost_types::ObjectChange {
                id: bifrost_types::ObjectId("m1".to_string()),
                kind: bifrost_types::ObjectChangeKind::Created,
            }),
        ) {
            crate::bifrost::consumer::hydrate::HydratedChange::Message(row, _) => row.message,
            _ => unreachable!(),
        }
    }

    fn checkpoint() -> Checkpoint {
        Checkpoint::Change(ChangeCursor {
            scope: CursorScope::Account,
            server_state: OpaqueChangeState {
                protocol: ProtocolKind::Jmap,
                envelope_version: 1,
                bytes: vec![1, 2, 3],
            },
            advanced_through: None,
            envelope_version: 1,
        })
    }

    async fn times_received_from(state: &WriteDbState) -> i64 {
        state
            .with_read(|conn| {
                conn.query_row(
                    "SELECT COALESCE(times_received_from, 0) FROM seen_addresses \
                     WHERE email = 'peer@example.com'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .or_else(|error| match error {
                    db::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
                    other => Err(other.to_string()),
                })
            })
            .await
            .unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prune_marker_table_is_empty_safe() {
        let (state, _tmp) = state();
        prune_marker_window(&state, "acc", &CursorScope::Account)
            .await
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn marker_suppresses_seen_ingest_replay_double_count() {
        let (state, _tmp) = state();
        let scope = CursorScope::Account;
        let cp = checkpoint();
        let rows = [inbound_row()];

        ingest_seen_with_marker(&state, &rows, Some(MarkerKey::new("acc", &scope, &cp)))
            .await
            .unwrap();
        let after_first = times_received_from(&state).await;
        assert_eq!(after_first, 1, "first ingest counts once");

        // Replay the same (scope, checkpoint): the marker must suppress the
        // re-increment so the counter stays at a single ingest.
        ingest_seen_with_marker(&state, &rows, Some(MarkerKey::new("acc", &scope, &cp)))
            .await
            .unwrap();
        let after_replay = times_received_from(&state).await;
        assert_eq!(after_replay, 1, "replay must not double-count");
    }
}
