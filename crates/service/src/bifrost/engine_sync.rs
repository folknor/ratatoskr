use std::time::Duration;

use bifrost_types::AccountId;
use db::db::ReadDbState;
use service_state::{
    BodyStoreWriteState, InlineImageStoreWriteState, SearchWriteHandle, WriteDbState,
};
use tokio_util::sync::CancellationToken;

use super::{
    BifrostConsumerStores, BifrostProviderKind, BifrostSyncEngine, ChangeStreamConsumer,
    build_account_factory,
};

const LAG_BACKOFF_DELAYS: [Duration; 3] = [
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_millis(1000),
];

#[allow(clippy::too_many_arguments)]
pub async fn sync_jmap_account(
    engine: &BifrostSyncEngine,
    stores: BifrostConsumerStores,
    read_db: &ReadDbState,
    write_db: &WriteDbState,
    account_id: &str,
    encryption_key: [u8; 32],
    cancellation_token: &CancellationToken,
) -> Result<(), String> {
    let initial_sync_completed = {
        let aid = account_id.to_string();
        write_db
            .with_write(move |conn| {
                conn.query_row(
                    "SELECT initial_sync_completed FROM accounts WHERE id = ?1",
                    rusqlite::params![aid],
                    |row| row.get::<_, i64>(0),
                )
                .map(|value| value != 0)
                .map_err(|error| format!("read initial_sync_completed: {error}"))
            })
            .await?
    };
    // One connected legacy JmapClient per kick, shared by BOTH the
    // mailbox/folder-map prepare and the post-drive auxiliary passes. Each
    // stage previously built its own client via `from_account` + connect,
    // re-fetching the JMAP Session document over the network - two redundant
    // network round-trips per kick that the legacy single-connection delta
    // path never paid. Threading a single connected client collapses them to
    // one. The bifrost engine attach in `drive_once` keeps its own connection
    // by design and is unaffected.
    //
    // NOTE: this trims real wire round-trips (Session GETs) but does NOT move
    // the steady-state-delta gate's `meta.provider_requests` metric: saehrimnir
    // only records JSON-RPC method POSTs (Mailbox/changes, Email/changes,
    // Email/get, ...), not the Session-document GET that `connect()` issues, so
    // that count stays at its baseline.
    let aux_client = jmap::client::JmapClient::from_account(
        read_db,
        write_db.writer_pool(),
        account_id,
        &encryption_key,
    )
    .await
    .map_err(|error| error.clone())?;
    aux_client
        .ensure_valid_token()
        .await
        .map_err(|error| error.clone())?;

    let jmap_folder_map =
        prepare_jmap_mailboxes(&aux_client, account_id, read_db, write_db).await?;

    let mut attempt = 0usize;
    loop {
        let report = drive_once(
            engine,
            stores.clone(),
            read_db,
            write_db,
            account_id,
            encryption_key,
            cancellation_token,
            BifrostProviderKind::Jmap,
            jmap_folder_map.clone(),
        )
        .await?;
        if !report.lagged {
            if !initial_sync_completed {
                let aid = account_id.to_string();
                write_db
                    .with_write(move |conn| sync::pipeline::mark_initial_sync_completed(conn, &aid))
                    .await?;
            }
            run_auxiliary_sync(
                &aux_client,
                read_db,
                write_db,
                account_id,
                initial_sync_completed,
            )
            .await?;
            return Ok(());
        }
        if attempt >= LAG_BACKOFF_DELAYS.len() {
            return Err("bifrost JMAP consumer lagged after bounded reattach attempts".to_string());
        }
        tokio::select! {
            () = tokio::time::sleep(LAG_BACKOFF_DELAYS[attempt]) => {}
            () = cancellation_token.cancelled() => return Err("sync cancelled".to_string()),
        }
        attempt = attempt.saturating_add(1);
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn sync_graph_account(
    engine: &BifrostSyncEngine,
    stores: BifrostConsumerStores,
    read_db: &ReadDbState,
    write_db: &WriteDbState,
    account_id: &str,
    encryption_key: [u8; 32],
    cancellation_token: &CancellationToken,
) -> Result<(), String> {
    let initial_sync_completed = {
        let aid = account_id.to_string();
        write_db
            .with_write(move |conn| {
                conn.query_row(
                    "SELECT initial_sync_completed FROM accounts WHERE id = ?1",
                    rusqlite::params![aid],
                    |row| row.get::<_, i64>(0),
                )
                .map(|value| value != 0)
                .map_err(|error| format!("read initial_sync_completed: {error}"))
            })
            .await?
    };

    let aux_client = graph::client::GraphClient::from_account(
        read_db,
        write_db.writer_pool(),
        account_id,
        encryption_key,
    )
    .await?;

    let graph_folder_map =
        prepare_graph_folders(&aux_client, account_id, read_db, write_db).await?;

    let mut attempt = 0usize;
    loop {
        let report = drive_once(
            engine,
            stores.clone(),
            read_db,
            write_db,
            account_id,
            encryption_key,
            cancellation_token,
            BifrostProviderKind::Graph,
            graph_folder_map.clone(),
        )
        .await?;
        if !report.lagged {
            if !initial_sync_completed {
                let aid = account_id.to_string();
                write_db
                    .with_write(move |conn| sync::pipeline::mark_initial_sync_completed(conn, &aid))
                    .await?;
            }
            provider_sync::consumer_support::run_graph_auxiliary_sync(
                &aux_client,
                account_id,
                read_db,
                write_db,
                initial_sync_completed,
            )
            .await;
            return Ok(());
        }
        if attempt >= LAG_BACKOFF_DELAYS.len() {
            return Err(
                "bifrost Graph consumer lagged after bounded reattach attempts".to_string(),
            );
        }
        tokio::select! {
            () = tokio::time::sleep(LAG_BACKOFF_DELAYS[attempt]) => {}
            () = cancellation_token.cancelled() => return Err("sync cancelled".to_string()),
        }
        attempt = attempt.saturating_add(1);
    }
}

async fn run_auxiliary_sync(
    client: &jmap::client::JmapClient,
    read_db: &ReadDbState,
    write_db: &WriteDbState,
    account_id: &str,
    initial_sync_completed_before_run: bool,
) -> Result<(), String> {
    provider_sync::consumer_support::run_jmap_auxiliary_sync(
        client,
        account_id,
        read_db,
        write_db,
        initial_sync_completed_before_run,
    )
    .await;
    Ok(())
}

async fn prepare_jmap_mailboxes(
    client: &jmap::client::JmapClient,
    account_id: &str,
    read_db: &ReadDbState,
    write_db: &WriteDbState,
) -> Result<std::collections::HashMap<String, common::types::FolderKind>, String> {
    provider_sync::consumer_support::sync_jmap_mailbox_folder_map(
        client, account_id, read_db, write_db,
    )
    .await
}

async fn prepare_graph_folders(
    client: &graph::client::GraphClient,
    account_id: &str,
    read_db: &ReadDbState,
    write_db: &WriteDbState,
) -> Result<std::collections::HashMap<String, common::types::FolderKind>, String> {
    provider_sync::consumer_support::sync_graph_folder_map(client, account_id, read_db, write_db)
        .await
}

#[allow(clippy::too_many_arguments)]
async fn drive_once(
    engine: &BifrostSyncEngine,
    stores: BifrostConsumerStores,
    read_db: &ReadDbState,
    write_db: &WriteDbState,
    account_id: &str,
    encryption_key: [u8; 32],
    cancellation_token: &CancellationToken,
    provider: BifrostProviderKind,
    folder_map: std::collections::HashMap<String, common::types::FolderKind>,
) -> Result<super::ConsumerDriveReport, String> {
    let factory =
        build_account_factory(read_db, write_db.writer_pool(), account_id, encryption_key)
            .await
            .map_err(|error| error.to_string())?;
    let account = AccountId(account_id.to_string());
    engine
        .engine()
        .attach(account.clone(), factory)
        .await
        .map_err(|error| format!("{error:?}"))?;
    let drive_result = async {
        let mut consumer =
            ChangeStreamConsumer::new(engine.engine(), account.clone(), provider, stores)
                .with_folder_map(folder_map)
                .with_checkpoint_store(engine.checkpoints())
                // Honor the test hook registry on the PRODUCTION kick so the
                // production-lag-backoff gate (B3a-cut-jmap 6.4) can arm a ForceLag
                // hook against the real drive loop. The registry is empty in
                // production, so this is a no-op outside the harness.
                .with_hooks(crate::handlers::test_helpers::bifrost_hooks());
        tokio::select! {
            result = consumer.drive_to_caught_up() => result.map_err(|error| format!("{error:?}")),
            () = cancellation_token.cancelled() => Err("sync cancelled".to_string()),
        }
    }
    .await;
    // The consumer's lag arm (`drive_receiver`) already detaches on
    // `RecvError::Lagged` before returning a `lagged` report, so this
    // explicit detach can race that teardown and observe the account as
    // already gone. `AccountNotAttached` therefore is NOT a failure here -
    // treating it as one would convert every lagged report into a hard
    // error and defeat the bounded lag-backoff loop in `sync_jmap_account`
    // (the re-kick would never run).
    let detach_result = match engine.engine().detach(&account).await {
        Ok(()) | Err(bifrost_sync::Error::AccountNotAttached(_)) => Ok(()),
        Err(error) => Err(format!("{error:?}")),
    };
    match (drive_result, detach_result) {
        (Ok(report), Ok(())) => Ok(report),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) | (Err(_), Err(error)) => Err(error),
    }
}

#[allow(dead_code)]
fn _stores_type_check(
    _db: WriteDbState,
    _body: BodyStoreWriteState,
    _inline: InlineImageStoreWriteState,
    _search: SearchWriteHandle,
) {
}
