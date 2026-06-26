//! Drive-level Graph move-vs-purge reconcile gate (finding A / spec 4.4).
//!
//! Validates that a Graph `ScopeChange{Removed}` is reconciled across the WHOLE
//! drive, not per-batch. A folder MOVE surfaces as `Removed` in the source
//! folder's per-folder scope batch and as `Updated`/`Added` in the DESTINATION
//! folder's SEPARATE batch (`graph.md:255-258`); a true PURGE surfaces as
//! `Removed` only. The gate drives the real `ChangeStreamConsumer` over an
//! injected `MultiplexerEvent` stream carrying two distinct per-folder scope
//! batches and asserts the moved message survives while the purged one is
//! deleted - in BOTH processing orders.
//!
//! Why this is an in-process drive test rather than a `saehrimnir` sync-harness
//! lua gate: (1) `saehrimnir` is an external installed binary with no in-repo
//! source, and no existing Graph fixture exercises `[[change]]` move/destroy
//! deltas, so a fixture-driven Graph delta cannot be authored or verified here;
//! (2) the bifrost inject harness (`test.bifrost_inject_batch`) waits for each
//! batch to PERSIST and ACK before returning, but this fix DEFERS the deletion
//! to drive end, which that per-batch-wait model cannot observe. Driving the
//! production `drive_injected_stream` directly is the faithful, deterministic
//! gate for the deferred-reconcile path, and it exercises the exact drive loop
//! production uses.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use bifrost_sync::{CheckpointStore, InMemoryCheckpointStore, SyncEngine};
use bifrost_types::{
    AccountId, Batch, Change, CursorScope, FolderId, MembershipScope, ObjectChange,
    ObjectChangeKind, ObjectId, ObjectType, PageBoundary, ScopeChange, ScopeChangeKind, SyncEvent,
};
use service_state::{
    BodyStoreWriteState, InlineImageStoreWriteState, SearchWriteHandle, WriteDbState, WriterCommand,
};
use tokio::sync::{broadcast, mpsc};

use super::hydrate::{
    ConsumerMessageRow, HydratedChange, SyntheticMessage, encode_synthetic_message,
    hydrate_change_to_message_insert_row_offline,
};
use super::{BifrostConsumerStores, BifrostProviderKind, ChangeStreamConsumer};

fn state() -> (WriteDbState, tempfile::TempDir) {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = db::db::open_writer_pool(tmp.path()).unwrap();
    pool.with_write_sync(|conn| {
        conn.execute(
            "INSERT INTO accounts (id, email, provider) VALUES ('acc', 'me@example.test', 'graph')",
            [],
        )
        .unwrap();
        Ok(())
    })
    .unwrap();
    (WriteDbState::from_pool(pool), tmp)
}

fn stores(state: WriteDbState, tmp: &tempfile::TempDir) -> BifrostConsumerStores {
    // The drive calls `search.flush_now()`, which sends a `FlushNow` command
    // and awaits its ack oneshot - so the test needs a live writer task that
    // answers commands, not a dropped receiver. A minimal drain-and-ack loop
    // keeps the search lane satisfied (this test gates DB membership, not
    // index contents).
    let (search_tx, mut search_rx) = mpsc::channel::<WriterCommand>(64);
    tokio::spawn(async move {
        while let Some(command) = search_rx.recv().await {
            let ack = match command {
                WriterCommand::Index { ack, .. }
                | WriterCommand::Delete { ack, .. }
                | WriterCommand::Clear { ack }
                | WriterCommand::FlushNow { ack } => ack,
            };
            let _ = ack.send(Ok(()));
        }
    });
    BifrostConsumerStores {
        db: state,
        body_store: BodyStoreWriteState::init(tmp.path()).unwrap(),
        inline_images: InlineImageStoreWriteState::init(tmp.path()).unwrap(),
        search: SearchWriteHandle::from_sender(search_tx),
    }
}

fn synthetic(id: &str, thread: &str, folders: &[&str]) -> SyntheticMessage {
    SyntheticMessage {
        id: id.to_string(),
        thread_id: Some(thread.to_string()),
        subject: format!("subject {id}"),
        from_addr: "peer@example.com".to_string(),
        to_addrs: vec!["me@example.com".to_string()],
        folder_ids: folders.iter().map(|f| (*f).to_string()).collect(),
        label_ids: Vec::new(),
        keywords: Vec::new(),
        raw_body: b"body".to_vec(),
        degraded_body: false,
        forced_outcome: None,
        reaction_emoji: None,
    }
}

/// Build the persisted-row representation of a synthetic Graph message in the
/// given folders (the offline hydration the consumer's pure merge produces).
fn synthetic_row(id: &str, thread: &str, folders: &[&str]) -> ConsumerMessageRow {
    let change = Change::ObjectChange(ObjectChange {
        id: ObjectId(encode_synthetic_message(&synthetic(id, thread, folders)).unwrap()),
        kind: ObjectChangeKind::Created,
    });
    match hydrate_change_to_message_insert_row_offline("acc", BifrostProviderKind::Graph, &change) {
        HydratedChange::Message(row, _) => *row,
        other => panic!("expected synthetic message row, got {other:?}"),
    }
}

/// A live `ObjectChange{Updated}` carrying the synthetic message (so hydration
/// re-materializes the row in its new folders without a live engine fetch).
fn object_updated(id: &str, thread: &str, folders: &[&str]) -> Change {
    Change::ObjectChange(ObjectChange {
        id: ObjectId(encode_synthetic_message(&synthetic(id, thread, folders)).unwrap()),
        kind: ObjectChangeKind::Updated,
    })
}

fn scope_change(id: &str, folder: &str, kind: ScopeChangeKind) -> Change {
    Change::ScopeChange(ScopeChange {
        id: ObjectId(id.to_string()),
        membership: MembershipScope::Folder(FolderId(folder.to_string())),
        kind,
    })
}

fn folder_email_scope(folder: &str) -> CursorScope {
    CursorScope::FolderType {
        folder: FolderId(folder.to_string()),
        ty: ObjectType::Email,
    }
}

fn batch_event(
    scope: CursorScope,
    items: Vec<Change>,
) -> bifrost_sync::multiplexer::MultiplexerEvent {
    bifrost_sync::multiplexer::MultiplexerEvent {
        scope,
        event: Arc::new(SyncEvent::Batch(Batch {
            bytes_in: 0,
            checkpoint: None,
            items,
            page_boundary: PageBoundary::Page,
            server_latency: Duration::from_millis(0),
        })),
        checkpoint: None,
    }
}

async fn count_messages(state: &WriteDbState, id: &'static str) -> i64 {
    let id = id.to_string();
    state
        .with_read(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM messages WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| e.to_string())
        })
        .await
        .unwrap()
}

/// Run one move-and-purge drive. `destination_first` controls whether the
/// destination folder's batch (the move target, carrying the surviving
/// `Updated`/`Added`) is processed before or after the source folder's batch
/// (carrying the `Removed`). Returns `(moved_survives, purged_count)`.
async fn run_move_and_purge(destination_first: bool) -> (i64, i64) {
    let (db_state, tmp) = state();
    let consumer_stores = stores(db_state.clone(), &tmp);

    // Prior-sync state: both messages live in the inbox (a separate kick).
    super::write::persist(
        &consumer_stores,
        "acc",
        BifrostProviderKind::Graph,
        &[
            synthetic_row("msg-move", "thread-move", &["INBOX"]),
            synthetic_row("msg-purge", "thread-purge", &["INBOX"]),
        ],
        &[],
    )
    .await
    .unwrap();
    assert_eq!(count_messages(&db_state, "msg-move").await, 1);
    assert_eq!(count_messages(&db_state, "msg-purge").await, 1);

    // The delta drive: msg-move moves INBOX -> ARCHIVE (Removed in inbox's
    // batch, Updated + Added in archive's batch); msg-purge is hard-deleted
    // (Removed only).
    let archive_batch = batch_event(
        folder_email_scope("mbx-archive"),
        vec![
            object_updated("msg-move", "thread-move", &["ARCHIVE"]),
            scope_change("msg-move", "mbx-archive", ScopeChangeKind::Added),
        ],
    );
    let inbox_batch = batch_event(
        folder_email_scope("mbx-inbox"),
        vec![
            scope_change("msg-move", "mbx-inbox", ScopeChangeKind::Removed),
            scope_change("msg-purge", "mbx-inbox", ScopeChangeKind::Removed),
        ],
    );

    let checkpoints: Arc<dyn CheckpointStore> = Arc::new(InMemoryCheckpointStore::new());
    let engine = Arc::new(
        SyncEngine::builder()
            .checkpoints(checkpoints)
            .build()
            .unwrap(),
    );
    let mut consumer = ChangeStreamConsumer::new(
        engine,
        AccountId("acc".to_string()),
        BifrostProviderKind::Graph,
        consumer_stores,
    );

    let (tx, rx) = broadcast::channel(8);
    if destination_first {
        tx.send(archive_batch).unwrap();
        tx.send(inbox_batch).unwrap();
    } else {
        tx.send(inbox_batch).unwrap();
        tx.send(archive_batch).unwrap();
    }
    drop(tx);
    consumer.drive_injected_stream(rx).await.unwrap();

    (
        count_messages(&db_state, "msg-move").await,
        count_messages(&db_state, "msg-purge").await,
    )
}

/// The destination-first order is the one the OLD per-batch reconcile lost: it
/// would create the moved row from the archive batch, then delete it when the
/// inbox `Removed` arrived in a later batch whose local live-set did not see
/// the move. The drive-level reconcile keeps it.
#[tokio::test(flavor = "multi_thread")]
async fn graph_drive_reconciles_move_destination_first() {
    let (moved, purged) = run_move_and_purge(true).await;
    assert_eq!(
        moved, 1,
        "a moved message must survive the source-folder Removed"
    );
    assert_eq!(purged, 0, "a purged message must be deleted at drive end");
}

/// Order-independence: processing the source `Removed` batch before the
/// destination batch must reach the same outcome.
#[tokio::test(flavor = "multi_thread")]
async fn graph_drive_reconciles_move_source_first() {
    let (moved, purged) = run_move_and_purge(false).await;
    assert_eq!(
        moved, 1,
        "a moved message must survive regardless of batch order"
    );
    assert_eq!(purged, 0, "a purged message must be deleted at drive end");
}
