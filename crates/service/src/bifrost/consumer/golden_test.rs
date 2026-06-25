//! Byte-identical golden-snapshot equality gate (B3a-cut-jmap 4.0 / 6.1).
//!
//! Feeds a fixed multi-message JMAP thread fixture (folders, keyword labels,
//! an `$important` message, a meeting-invite attachment, and a reply carrying
//! the all-ids In-Reply-To / List-Unsubscribe / MDN headers the structured
//! `Message` drops) through the consumer's `build_consumer_row`,
//! `write::persist`, and `post_persist::run` into a temp DB, dumps the ten
//! membership-and-threading tables canonically, and asserts byte-equality
//! against the FROZEN golden captured from the legacy JMAP persist semantics.
//!
//! The golden is the legacy path's last word: the legacy `persist_messages`
//! is deleted in this same landing, so it can no longer be re-run. The golden
//! encodes the exact row content the legacy `upsert_messages` /
//! `set_thread_labels` / `sync_keyword_labels` / `upsert_thread_record` /
//! `upsert_thread_participants` / `maybe_update_chat_state` writes produced
//! for the input fixture, and this test proves the consumer reproduces it.
//! Regenerate the golden file deliberately with `UPDATE_GOLDEN=1` (a silent
//! rewrite to mask a divergence is a gate failure, not a pass).

#![allow(clippy::unwrap_used)]

use std::collections::HashMap;
use std::path::PathBuf;

use bifrost_types::{
    AccountId, Address, BlobCapabilities, BlobEncoding, BlobHandle, BlobId, ContainerId,
    Importance, Message, ObjectId, ThreadId,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use service_state::{
    BodyStoreWriteState, InlineImageStoreWriteState, SearchWriteHandle, WriteDbState,
};
use std::time::{Duration, UNIX_EPOCH};
use tokio::sync::mpsc;

use super::BifrostConsumerStores;
use super::BifrostProviderKind;
use super::hydrate::{HydrationOutcome, build_consumer_row};

const INPUT_FIXTURE: &str = "tests/fixtures/jmap_consumer_membership_input.json";
const GOLDEN_FIXTURE: &str = "tests/fixtures/jmap_consumer_membership_golden.json";

/// The ten tables / columns the gate pins (spec 4.0). `message_labels` is
/// asserted EMPTY (JMAP keyword-label semantics).
const SNAPSHOT_QUERIES: &[(&str, &str)] = &[
    (
        "messages",
        "SELECT id, account_id, thread_id, from_address, from_name, to_addresses, \
         cc_addresses, bcc_addresses, reply_to, subject, snippet, date, is_read, is_starred, \
         is_replied, is_forwarded, body_cached, raw_size, internal_date, list_unsubscribe, \
         list_unsubscribe_post, auth_results, message_id_header, references_header, \
         in_reply_to_header, mdn_requested, is_reaction, has_meeting_invite, \
         meeting_invite_method, meeting_invite_uid FROM messages ORDER BY id",
    ),
    (
        "attachments",
        "SELECT id, message_id, account_id, filename, mime_type, size, remote_attachment_id, \
         content_id, is_inline FROM attachments ORDER BY id",
    ),
    (
        "message_folders",
        "SELECT account_id, message_id, folder_id FROM message_folders \
         ORDER BY message_id, folder_id",
    ),
    (
        "message_keywords",
        "SELECT account_id, message_id, keyword, label_id FROM message_keywords \
         ORDER BY message_id, label_id",
    ),
    (
        "message_labels",
        "SELECT account_id, message_id, label_id FROM message_labels ORDER BY message_id, label_id",
    ),
    (
        "thread_folders",
        "SELECT account_id, thread_id, folder_id FROM thread_folders ORDER BY thread_id, folder_id",
    ),
    (
        "thread_labels",
        "SELECT account_id, thread_id, label_id FROM thread_labels ORDER BY thread_id, label_id",
    ),
    (
        "threads",
        "SELECT id, account_id, message_count, is_read, is_starred, is_important, \
         has_attachments, is_chat_thread, shared_mailbox_id FROM threads ORDER BY id",
    ),
    (
        "thread_participants",
        "SELECT account_id, thread_id, email FROM thread_participants \
         ORDER BY thread_id, email",
    ),
];

#[derive(Deserialize)]
struct InputFixture {
    account_id: String,
    messages: Vec<InputMessage>,
}

#[derive(Deserialize)]
struct InputAddress {
    name: Option<String>,
    address: String,
}

#[derive(Deserialize)]
struct InputBlob {
    id: String,
    content_type: Option<String>,
}

#[derive(Deserialize)]
struct InputMessage {
    id: String,
    thread_id: String,
    subject: String,
    date_ms: u64,
    from: InputAddress,
    to: Vec<InputAddress>,
    cc: Vec<InputAddress>,
    bcc: Vec<InputAddress>,
    containers: Vec<String>,
    flags: Vec<String>,
    importance: String,
    blobs: Vec<InputBlob>,
    raw_lines: Vec<String>,
}

fn manifest_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn to_address(a: &InputAddress) -> Address {
    Address {
        name: a.name.clone(),
        address: a.address.clone(),
    }
}

fn blob_handle(blob: &InputBlob) -> BlobHandle {
    BlobHandle {
        id: BlobId(blob.id.clone()),
        size: None,
        content_type: blob.content_type.clone(),
        digest: None,
        capabilities: BlobCapabilities {
            supports_range: false,
            supports_parallel: false,
            digest_available_pre_download: false,
            encoding: BlobEncoding::Raw7Bit,
        },
    }
}

fn build_message(input: &InputMessage) -> (Message, Vec<u8>) {
    let raw = input.raw_lines.join("\r\n").into_bytes();
    let message = Message {
        id: ObjectId(input.id.clone()),
        thread_id: Some(ThreadId(input.thread_id.clone())),
        from: vec![to_address(&input.from)],
        to: input.to.iter().map(to_address).collect(),
        cc: input.cc.iter().map(to_address).collect(),
        bcc: input.bcc.iter().map(to_address).collect(),
        reply_to: Vec::new(),
        subject: Some(input.subject.clone()),
        date: Some(UNIX_EPOCH + Duration::from_millis(input.date_ms)),
        containers: input
            .containers
            .iter()
            .map(|c| ContainerId(c.clone()))
            .collect(),
        flags: input.flags.iter().cloned().collect(),
        importance: match input.importance.as_str() {
            "high" => Importance::High,
            "low" => Importance::Low,
            _ => Importance::Normal,
        },
        body_text: None,
        body_html: None,
        attachments: input.blobs.iter().map(blob_handle).collect(),
        size_bytes: Some(raw.len() as u64),
        in_reply_to: None,
        references: Vec::new(),
    };
    (message, raw)
}

fn state(account_id: &str) -> (WriteDbState, tempfile::TempDir) {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = db::db::open_writer_pool(tmp.path()).unwrap();
    let account_id = account_id.to_string();
    pool.with_write_sync(move |conn| {
        conn.execute(
            "INSERT INTO accounts (id, email, provider) VALUES (?1, 'me@example.test', 'jmap')",
            rusqlite::params![account_id],
        )
        .unwrap();
        Ok(())
    })
    .unwrap();
    (WriteDbState::from_pool(pool), tmp)
}

fn stores(state: WriteDbState, tmp: &tempfile::TempDir) -> BifrostConsumerStores {
    let (search_tx, _search_rx) = mpsc::channel(8);
    BifrostConsumerStores {
        db: state,
        body_store: BodyStoreWriteState::init(tmp.path()).unwrap(),
        inline_images: InlineImageStoreWriteState::init(tmp.path()).unwrap(),
        search: SearchWriteHandle::from_sender(search_tx),
    }
}

async fn dump_snapshot(state: &WriteDbState) -> Value {
    let mut tables = Map::new();
    for (name, sql) in SNAPSHOT_QUERIES {
        let sql = (*sql).to_string();
        let rows = state
            .with_read(move |conn| {
                let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
                let mut out = Vec::new();
                let mut query = stmt.query([]).map_err(|e| e.to_string())?;
                while let Some(row) = query.next().map_err(|e| e.to_string())? {
                    let statement = row.as_ref();
                    let column_names: Vec<String> = statement
                        .column_names()
                        .iter()
                        .map(|s| (*s).to_string())
                        .collect();
                    let mut obj = Map::new();
                    for (idx, col) in column_names.iter().enumerate() {
                        let value: rusqlite::types::Value =
                            row.get(idx).map_err(|e| e.to_string())?;
                        obj.insert(col.clone(), sqlite_to_json(&value));
                    }
                    out.push(Value::Object(obj));
                }
                Ok(out)
            })
            .await
            .unwrap();
        tables.insert((*name).to_string(), Value::Array(rows));
    }
    Value::Object(tables)
}

fn sqlite_to_json(value: &rusqlite::types::Value) -> Value {
    use base64::Engine;
    match value {
        rusqlite::types::Value::Null => Value::Null,
        rusqlite::types::Value::Integer(i) => json!(i),
        rusqlite::types::Value::Real(f) => json!(f),
        rusqlite::types::Value::Text(t) => json!(t),
        rusqlite::types::Value::Blob(b) => {
            json!(base64::engine::general_purpose::STANDARD.encode(b))
        }
    }
}

/// Pretty-print with sorted object keys so a regenerated golden's `git diff`
/// is line-readable (spec 4.0 serialization rules). `serde_json::Map` with
/// the default features preserves insertion order, so re-key into BTree
/// order before serializing.
fn canonical_string(value: &Value) -> String {
    fn sort(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut sorted: std::collections::BTreeMap<String, Value> =
                    std::collections::BTreeMap::new();
                for (k, v) in map {
                    sorted.insert(k.clone(), sort(v));
                }
                Value::Object(sorted.into_iter().collect())
            }
            Value::Array(items) => Value::Array(items.iter().map(sort).collect()),
            other => other.clone(),
        }
    }
    serde_json::to_string_pretty(&sort(value)).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn jmap_consumer_membership_equals_legacy() {
    let input: InputFixture =
        serde_json::from_slice(&std::fs::read(manifest_path(INPUT_FIXTURE)).unwrap()).unwrap();
    let account = AccountId(input.account_id.clone());

    let (db_state, tmp) = state(&input.account_id);
    let consumer_stores = stores(db_state.clone(), &tmp);

    // The consumer path: build each row through the SAME pure merge the
    // production async hydration calls (`build_consumer_row`), with no live
    // engine. The fixture's `raw_lines` are the verbatim RFC822 the engine
    // would have streamed via `open_raw_rfc822`; inline blobs are empty here.
    let jmap_folder_map: HashMap<String, common::types::FolderKind> = HashMap::new();
    let rows: Vec<_> = input
        .messages
        .iter()
        .map(|message| {
            let (msg, raw) = build_message(message);
            build_consumer_row(
                &account,
                BifrostProviderKind::Jmap,
                &jmap_folder_map,
                &msg,
                Some(&raw),
                HashMap::new(),
                HydrationOutcome::Succeeded,
            )
        })
        .collect();

    let affected = super::write::persist(
        &consumer_stores,
        &input.account_id,
        BifrostProviderKind::Jmap,
        &rows,
        &[],
    )
    .await
    .unwrap();
    super::post_persist::run(
        &consumer_stores.db,
        &input.account_id,
        BifrostProviderKind::Jmap,
        &bifrost_types::CursorScope::Account,
        None,
        &rows,
        &affected,
    )
    .await
    .unwrap();

    let snapshot = dump_snapshot(&db_state).await;
    let snapshot_str = canonical_string(&snapshot);

    // Regeneration path: `UPDATE_GOLDEN=1` rewrites the frozen golden and
    // echoes the canonical snapshot to stderr so it can be re-derived from
    // the test output in sandboxes that cannot pass the env var through.
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::write(manifest_path(GOLDEN_FIXTURE), format!("{snapshot_str}\n")).unwrap();
        eprintln!("===BEGIN GOLDEN SNAPSHOT===\n{snapshot_str}\n===END GOLDEN SNAPSHOT===");
        eprintln!("[golden] regenerated {GOLDEN_FIXTURE}");
        return;
    }

    let golden_raw = std::fs::read_to_string(manifest_path(GOLDEN_FIXTURE)).unwrap_or_else(|_| {
        panic!(
            "missing golden fixture {GOLDEN_FIXTURE}; regenerate with UPDATE_GOLDEN=1 after \
             validating the consumer output against the legacy JMAP persist semantics"
        )
    });
    let golden: Value = serde_json::from_str(&golden_raw).unwrap();
    let golden_str = canonical_string(&golden);

    if snapshot_str != golden_str {
        eprintln!("===BEGIN CONSUMER SNAPSHOT===\n{snapshot_str}\n===END CONSUMER SNAPSHOT===");
    }
    assert_eq!(
        snapshot_str, golden_str,
        "consumer JMAP persist diverged from the frozen legacy golden; if this is an \
         intentional change, regenerate with UPDATE_GOLDEN=1 and justify it in the commit"
    );

    // The JMAP keyword-label invariant is load-bearing: assert it directly so
    // a golden that accidentally captured a non-empty message_labels still
    // fails the gate.
    let message_labels = snapshot
        .get("message_labels")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(usize::MAX);
    assert_eq!(
        message_labels, 0,
        "JMAP must never write message_labels rows"
    );
}
