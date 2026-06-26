//! Handlers for the `test-helpers` feature. Compiled out of release builds.
//!
//! Each handler maps to a `RequestParams::Test*` variant defined in
//! `service-api` under the same feature flag. They exist exclusively to give
//! integration tests a deterministic way to drive panic-safety, version-
//! mismatch, in-flight-cap, and stdio-corruption behaviors.

use crate::boot::BootSharedState;
use rusqlite::{OptionalExtension, params};
use serde_json::Value;
use service_api::{
    HealthPingResponse, ServiceError, TestBifrostArmHookAck, TestBifrostArmHookParams,
    TestBifrostAttachAck, TestBifrostAttachParams, TestBifrostDurableCursor,
    TestBifrostFactoryOpenAck, TestBifrostFactoryOpenParams, TestBifrostHook,
    TestBifrostInjectBatchAck, TestBifrostInjectBatchParams, TestBifrostItemOutcome,
    TestBifrostProbeAck, TestBifrostProbeParams, TestBifrostProviderKind, TestCounterReadAck,
    TestCrashAfterNWritesAck, TestCrashAfterNWritesParams, TestDbAccountRow, TestDbAttachmentRow,
    TestDbCalendarEventRow, TestDbCalendarRow, TestDbContactGroupRow, TestDbContactRow,
    TestDbFolderRow, TestDbLabelRow, TestDbLocalDraftRow, TestDbMessageRow, TestDbSignatureRow,
    TestDelayNextWriteAck, TestDelayNextWriteParams, TestPendingOpRow, TestPendingOpsReadAck,
    TestPendingOpsReadParams, TestQueryBlobTombstoneStateAck, TestQueryBlobTombstoneStateParams,
    TestQueryDbStateAck, TestQueryDbStateParams, TestRemoveCachedAttachmentBytesAck,
    TestRemoveCachedAttachmentBytesParams, TestRunDiscoveryParams, TestSearchIndexAck,
    TestSearchIndexParams, TestSearchIndexResult, TestSeedAccountAck, TestSeedAccountParams,
    TestSeedCachedAttachmentAck, TestSeedCachedAttachmentParams, TestSeedRemoteAttachmentAck,
    TestSeedRemoteAttachmentParams, TestSeedThreadAck, TestSeedThreadParams, TestStartSyncParams,
    TestThreadReadAck, TestThreadReadParams,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::broadcast;

static BIFROST_HOOKS: OnceLock<Arc<crate::bifrost::ConsumerHookRegistry>> = OnceLock::new();
static BIFROST_SESSIONS: OnceLock<std::sync::Mutex<HashMap<u64, Arc<BifrostTestSession>>>> =
    OnceLock::new();
static NEXT_BIFROST_SESSION_ID: AtomicU64 = AtomicU64::new(1);

struct BifrostTestSession {
    account_id: String,
    db: service_state::WriteDbState,
    inject_tx: broadcast::Sender<bifrost_sync::multiplexer::MultiplexerEvent>,
    _engine: crate::bifrost::BifrostSyncEngine,
}

/// Per-account latch of the most-recent attach session's one-shot
/// completion edge (spec 4.1.2), readable by `TestBifrostProbe`. The
/// empty-stream "completes immediately" edge has no durable side effect to
/// probe for (no batch, no cursor, no marker), so the driver's
/// `ConsumerDriveReport.completed` is surfaced through this latch instead.
/// Keyed by account id and installed by the attach handler so the probe can
/// read it without holding the session (the drive task may have detached).
static BIFROST_COMPLETION: OnceLock<
    std::sync::Mutex<HashMap<String, Arc<std::sync::atomic::AtomicBool>>>,
> = OnceLock::new();

fn bifrost_completion()
-> &'static std::sync::Mutex<HashMap<String, Arc<std::sync::atomic::AtomicBool>>> {
    BIFROST_COMPLETION.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

pub(crate) fn bifrost_hooks() -> Arc<crate::bifrost::ConsumerHookRegistry> {
    Arc::clone(BIFROST_HOOKS.get_or_init(|| Arc::new(Default::default())))
}

fn bifrost_sessions() -> &'static std::sync::Mutex<HashMap<u64, Arc<BifrostTestSession>>> {
    BIFROST_SESSIONS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

pub(super) async fn panic_handle() -> Result<Value, ServiceError> {
    panic!("test-helpers: TestPanic handler intentional panic");
}

pub(super) async fn version_handle(version: u32) -> Result<Value, ServiceError> {
    serde_json::to_value(HealthPingResponse {
        version,
        pid: std::process::id(),
        uptime_ms: 0,
    })
    .map_err(|error| ServiceError::Internal(error.to_string()))
}

pub(super) async fn slow_handle(millis: u64) -> Result<Value, ServiceError> {
    tokio::time::sleep(Duration::from_millis(millis)).await;
    Ok(Value::Null)
}

pub(super) async fn println_handle(message: String) -> Result<Value, ServiceError> {
    // Goes through the global stdout HANDLE; with the stdio-defense in place
    // this lands in /dev/null (unix) or NUL (windows) instead of corrupting
    // the JSON-RPC framing. The test asserts that the response on the
    // saved-FD stdout is still well-formed.
    println!("{message}");
    Ok(Value::Null)
}

pub(super) async fn seed_account_handle(
    boot_state: &Arc<BootSharedState>,
    params: TestSeedAccountParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    let unique = uuid::Uuid::new_v4().to_string();
    let email = params
        .email
        .unwrap_or_else(|| format!("harness-{unique}@example.test"));
    let provider = params.provider.unwrap_or_else(|| "imap".into());
    let caldav_url = params.caldav_url;
    let caldav_username = params.caldav_username;
    let caldav_password = params.caldav_password;
    let encryption_key = boot_state.encryption_key().ok_or_else(|| {
        ServiceError::Internal(
            "test.seed_account received before encryption key was available".into(),
        )
    })?;
    let encrypt_secret = |value: Option<String>| -> Result<Option<String>, ServiceError> {
        value
            .map(|secret| common::crypto::encrypt_value(&encryption_key, &secret))
            .transpose()
            .map_err(ServiceError::Internal)
    };
    let caldav_password = encrypt_secret(caldav_password)?;
    let requested_auth_method = params.auth_method;
    let default_oauth_provider = match provider.as_str() {
        "gmail_api" => Some("google".to_string()),
        "graph" => Some("microsoft".to_string()),
        _ => None,
    };
    let oauth_provider = params.oauth_provider.or(default_oauth_provider);
    let auth_requires_oauth = requested_auth_method
        .as_deref()
        .is_some_and(|method| matches!(method, "oauth2" | "bearer" | "oauthbearer"));
    if auth_requires_oauth && oauth_provider.is_none() {
        return Err(ServiceError::InvalidParams {
            method: "test.seed_account".into(),
            message: "auth_method requires oauth_provider".into(),
        });
    }
    let uses_oauth = auth_requires_oauth || oauth_provider.is_some();
    let create_params = db::db::queries_extra::CreateAccountParams {
        email: email.clone(),
        provider,
        display_name: params.display_name.or_else(|| Some("Harness".into())),
        account_name: params
            .account_name
            .unwrap_or_else(|| "Harness Account".into()),
        account_color: "#4285f4".into(),
        auth_method: requested_auth_method.unwrap_or_else(|| {
            if uses_oauth {
                "oauth2".into()
            } else {
                "password".into()
            }
        }),
        access_token: encrypt_secret(
            params
                .access_token
                .or_else(|| uses_oauth.then(|| "test-access-token".into())),
        )?,
        refresh_token: encrypt_secret(
            params
                .refresh_token
                .or_else(|| uses_oauth.then(|| "test-refresh-token".into())),
        )?,
        token_expires_at: params
            .token_expires_at
            .or_else(|| uses_oauth.then(|| chrono::Utc::now().timestamp() + 3_600)),
        oauth_provider,
        oauth_client_id: encrypt_secret(
            params
                .oauth_client_id
                .or_else(|| uses_oauth.then(|| "test-client-id".into())),
        )?,
        oauth_client_secret: None,
        oauth_token_url: params.oauth_token_url,
        oauth_extra_scopes: params.oauth_extra_scopes,
        imap_host: Some("imap.example.test".into()),
        imap_port: Some(993),
        imap_security: Some("tls".into()),
        imap_username: Some(email.clone()),
        imap_password: encrypt_secret(Some("test-password".into()))?,
        smtp_host: Some("smtp.example.test".into()),
        smtp_port: Some(587),
        smtp_security: Some("starttls".into()),
        smtp_username: Some(email.clone()),
        smtp_password: encrypt_secret(Some("test-password".into()))?,
        jmap_url: Some(
            params
                .jmap_url
                .unwrap_or_else(|| "https://jmap.example.test".into()),
        ),
        accept_invalid_certs: true,
    };
    let ack_email = email.clone();
    let (account_id, label_count) = write_db
        .with_write(move |conn| {
            let account_id = db::db::queries_extra::create_account_sync(conn, &create_params)?;
            if caldav_url.is_some() || caldav_username.is_some() || caldav_password.is_some() {
                conn.execute(
                    "UPDATE accounts
                     SET caldav_url = ?1,
                         caldav_username = ?2,
                         caldav_password = ?3
                     WHERE id = ?4",
                    params![caldav_url, caldav_username, caldav_password, &account_id],
                )
                .map_err(|e| format!("seed account caldav config: {e}"))?;
            }
            let label_count = insert_harness_account_rows(conn, &account_id, &email)?;
            Ok((account_id, label_count))
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(TestSeedAccountAck {
        account_id,
        email: ack_email,
        label_count,
    })
    .map_err(|error| ServiceError::Internal(error.to_string()))
}

pub(super) async fn bifrost_factory_open_handle(
    boot_state: &Arc<BootSharedState>,
    params: TestBifrostFactoryOpenParams,
) -> Result<Value, ServiceError> {
    let read_db = boot_state.read_db_state().ok_or_else(|| {
        ServiceError::Internal(
            "test.bifrost_factory_open received before read DB was available".into(),
        )
    })?;
    let write_db = boot_state.write_db_state()?;
    let encryption_key = boot_state.encryption_key().ok_or_else(|| {
        ServiceError::Internal(
            "test.bifrost_factory_open received before encryption key was available".into(),
        )
    })?;
    let factory = match crate::bifrost::build_account_factory(
        &read_db,
        write_db.writer_pool(),
        &params.account_id,
        encryption_key,
    )
    .await
    {
        Ok(factory) => factory,
        // A construction-time failure (unknown provider, missing
        // credential/endpoint, decrypt failure) is reported through the
        // ack rather than as a request error so the harness can assert on
        // the typed `BifrostBuildError::classify` mapping at the IO
        // boundary, alongside the open-path `AccountError` mapping below.
        Err(error) => {
            return serde_json::to_value(TestBifrostFactoryOpenAck {
                account_id: params.account_id,
                opened: false,
                capability_debug: None,
                failure_kind: Some(format!("{:?}", error.classify())),
                provider_message: Some(error.to_string()),
                diagnostic_debug: None,
            })
            .map_err(|error| ServiceError::Internal(error.to_string()));
        }
    };
    match factory
        .open(bifrost_types::AccountId(params.account_id.clone()))
        .await
    {
        Ok(account) => serde_json::to_value(TestBifrostFactoryOpenAck {
            account_id: params.account_id,
            opened: true,
            capability_debug: Some(format!("{:?}", account.capabilities())),
            failure_kind: None,
            provider_message: None,
            diagnostic_debug: None,
        }),
        Err(error) => {
            let service_api::actions::ActionError::Remote { kind, message } =
                crate::bifrost::account_error_to_action_error(&error)
            else {
                unreachable!("bifrost account errors map to remote action errors");
            };
            // `provider_message` carries the wire-safe `message_key()` (the
            // `account_error_to_action_error` message), matching the live
            // mapping convention. The raw cause-chain diagnostic goes on the
            // separate, clearly-internal `diagnostic_debug` field so the safe
            // key is not conflated with `support_internal()` dumps.
            serde_json::to_value(TestBifrostFactoryOpenAck {
                account_id: params.account_id,
                opened: false,
                capability_debug: None,
                failure_kind: Some(format!("{kind:?}")),
                provider_message: Some(message),
                diagnostic_debug: Some(format!("{:?}", error.support_internal())),
            })
        }
    }
    .map_err(|error| ServiceError::Internal(error.to_string()))
}

pub(super) async fn bifrost_arm_hook_handle(
    params: TestBifrostArmHookParams,
) -> Result<Value, ServiceError> {
    let hook = match params.hook {
        TestBifrostHook::StallConsumer { after_ms } => {
            crate::bifrost::ConsumerHook::StallConsumer { after_ms }
        }
        TestBifrostHook::CrashBeforeAck => crate::bifrost::ConsumerHook::CrashBeforeAck,
        TestBifrostHook::CrashAfterAckNoSentinel => {
            crate::bifrost::ConsumerHook::CrashAfterAckNoSentinel
        }
        TestBifrostHook::CrashBeforeDriveEndThreading => {
            crate::bifrost::ConsumerHook::CrashBeforeDriveEndThreading
        }
        TestBifrostHook::ForceLag => crate::bifrost::ConsumerHook::ForceLag,
    };
    bifrost_hooks().arm(params.account_id, hook).await;
    serde_json::to_value(TestBifrostArmHookAck { armed: true })
        .map_err(|error| ServiceError::Internal(error.to_string()))
}

pub(super) async fn bifrost_attach_handle(
    boot_state: &Arc<BootSharedState>,
    params: TestBifrostAttachParams,
) -> Result<Value, ServiceError> {
    let read_db = boot_state.read_db_state().ok_or_else(|| {
        ServiceError::Internal("test.bifrost_attach received before read DB was available".into())
    })?;
    let write_db = boot_state.write_db_state()?;
    let sync_runtime = boot_state.sync_runtime().ok_or_else(|| {
        ServiceError::Internal(
            "test.bifrost_attach received before SyncRuntime was installed".into(),
        )
    })?;
    let stores = sync_runtime.bifrost_consumer_stores();
    let checkpoint_store =
        crate::bifrost::SqliteCheckpointStore::new(write_db.writer_pool(), read_db.clone());
    let harness = crate::bifrost::BifrostSyncEngine::build(checkpoint_store, None)
        .map_err(|error| ServiceError::Internal(format!("build bifrost engine: {error}")))?;
    let account_id = bifrost_types::AccountId(params.account_id.clone());
    let provider = match params.provider_kind {
        TestBifrostProviderKind::Gmail => crate::bifrost::BifrostProviderKind::Gmail,
        TestBifrostProviderKind::Graph => crate::bifrost::BifrostProviderKind::Graph,
        TestBifrostProviderKind::Imap => crate::bifrost::BifrostProviderKind::Imap,
        TestBifrostProviderKind::Jmap => crate::bifrost::BifrostProviderKind::Jmap,
    };
    // Deliberate deviation from spec 4.4 "changes_capacity parity" (256):
    // a smaller standee capacity lets the lag-recovery gate overflow the
    // bounded broadcast with a manageable number of synthetic injects rather
    // than 256+. The gate asserts Lagged recovery, which any bounded
    // capacity smaller than the flood exercises identically; production
    // fidelity of the exact overflow threshold is not what the gate pins.
    const INJECT_CHANNEL_CAPACITY: usize = 16;
    let (inject_tx, inject_rx) = broadcast::channel(INJECT_CHANNEL_CAPACITY);
    let mut consumer = crate::bifrost::ChangeStreamConsumer::new(
        harness.engine(),
        account_id.clone(),
        provider,
        stores,
    )
    .with_checkpoint_store(harness.checkpoints())
    .with_hooks(bifrost_hooks());
    let completion_edge = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let task_edge = Arc::clone(&completion_edge);
        tokio::spawn(async move {
            match consumer.drive_injected_stream(inject_rx).await {
                Ok(report) => {
                    if report.completed {
                        // Surface the one-shot completion edge (spec 4.1.2)
                        // - this is the only observation of the empty-stream
                        // "completes immediately" case, which has no durable
                        // side effect for a probe to read otherwise.
                        task_edge.store(true, Ordering::Relaxed);
                    }
                }
                Err(error) => log::warn!("test bifrost injected consumer exited: {error}"),
            }
        });
    }
    bifrost_completion()
        .lock()
        .map_err(|error| ServiceError::Internal(format!("bifrost completion lock: {error}")))?
        .insert(params.account_id.clone(), Arc::clone(&completion_edge));
    let session_id = NEXT_BIFROST_SESSION_ID.fetch_add(1, Ordering::Relaxed);
    let session = Arc::new(BifrostTestSession {
        account_id: params.account_id,
        db: write_db.clone(),
        inject_tx,
        _engine: harness,
    });
    bifrost_sessions()
        .lock()
        .map_err(|error| ServiceError::Internal(format!("bifrost session lock: {error}")))?
        .insert(session_id, session);
    serde_json::to_value(TestBifrostAttachAck {
        session_id,
        subscribed: true,
        completed: false,
        scopes_completed: 0,
        batches_acked: 0,
    })
    .map_err(|error| ServiceError::Internal(error.to_string()))
}

pub(super) async fn bifrost_inject_batch_handle(
    params: TestBifrostInjectBatchParams,
) -> Result<Value, ServiceError> {
    let session = {
        let sessions = bifrost_sessions()
            .lock()
            .map_err(|error| ServiceError::Internal(format!("bifrost session lock: {error}")))?;
        sessions.get(&params.session_id).cloned()
    }
    .ok_or_else(|| ServiceError::InvalidParams {
        method: "test.bifrost_inject_batch".into(),
        message: format!("unknown bifrost session {}", params.session_id),
    })?;
    if session.account_id != params.account_id {
        return Err(ServiceError::InvalidParams {
            method: "test.bifrost_inject_batch".into(),
            message: "session account_id mismatch".into(),
        });
    }
    let scope = parse_bifrost_scope(&params.scope)?;
    let checkpoint = params
        .checkpoint
        .clone()
        .map(|bytes| synthetic_change_checkpoint(scope.clone(), bytes));
    let checkpoint_blob = checkpoint.as_ref().map(bifrost_sync::encode_envelope);
    let changes = params
        .messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            let forced_outcome = params
                .item_outcomes
                .get(index)
                .copied()
                .map(synthetic_outcome);
            let synthetic = crate::bifrost::consumer::hydrate::SyntheticMessage {
                id: message.id.clone(),
                thread_id: message.thread_id.clone(),
                subject: message.subject.clone(),
                from_addr: message.from_addr.clone(),
                to_addrs: message.to_addrs.clone(),
                folder_ids: message.folder_ids.clone(),
                label_ids: message.label_ids.clone(),
                keywords: message.keywords.clone(),
                raw_body: message.raw_body.clone(),
                degraded_body: matches!(
                    forced_outcome,
                    Some(crate::bifrost::consumer::hydrate::SyntheticOutcome::DegradedBody)
                ),
                forced_outcome,
                reaction_emoji: None,
            };
            crate::bifrost::consumer::hydrate::encode_synthetic_message(&synthetic).map(|id| {
                bifrost_types::Change::ObjectChange(bifrost_types::ObjectChange {
                    id: bifrost_types::ObjectId(id),
                    kind: bifrost_types::ObjectChangeKind::Created,
                })
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(ServiceError::Internal)?;
    let hydrated = params
        .item_outcomes
        .iter()
        .filter(|outcome| !matches!(outcome, TestBifrostItemOutcome::Failed))
        .count()
        .max(
            params
                .messages
                .len()
                .saturating_sub(params.item_outcomes.len()),
        );
    let blocked = params
        .item_outcomes
        .iter()
        .any(|outcome| matches!(outcome, TestBifrostItemOutcome::Uncertain));
    let batch = bifrost_types::Batch {
        bytes_in: params
            .messages
            .iter()
            .map(|message| u64::try_from(message.raw_body.len()).unwrap_or(u64::MAX))
            .sum(),
        checkpoint: checkpoint.clone(),
        items: changes,
        page_boundary: bifrost_types::PageBoundary::Page,
        server_latency: Duration::from_millis(0),
    };
    let event = bifrost_sync::multiplexer::MultiplexerEvent {
        scope: scope.clone(),
        event: Arc::new(bifrost_types::SyncEvent::Batch(batch)),
        checkpoint,
    };
    // A CrashBeforeAck hook exits the drive task between the search flush
    // and the ack, so the cursor advance never lands. Read the armed hook
    // BEFORE injecting (the consumer `take`s it while processing, so a
    // post-send peek would race) and skip the cursor wait rather than block
    // on a checkpoint the consumer deliberately withholds. `acked` still
    // reports the INTENDED ack (checkpoint present, not Uncertain-blocked);
    // the withheld case is distinguished by probing the (absent) cursor.
    let ack_withheld_by_crash = bifrost_hooks().peek_withholds_ack(&params.account_id).await;
    session.inject_tx.send(event).map_err(|error| {
        ServiceError::Internal(format!("send bifrost synthetic batch: {error}"))
    })?;
    wait_for_bifrost_messages(&session.db, &params.account_id, &params.messages).await?;
    if let Some(blob) = &checkpoint_blob
        && !blocked
        && !ack_withheld_by_crash
    {
        wait_for_bifrost_checkpoint(&session.db, &params.account_id, &scope, blob).await?;
    }
    serde_json::to_value(TestBifrostInjectBatchAck {
        hydrated: hydrated.try_into().unwrap_or(u32::MAX),
        persisted: hydrated.try_into().unwrap_or(u32::MAX),
        acked: checkpoint_blob.is_some() && !blocked,
        blocked,
        checkpoint_blob,
    })
    .map_err(|error| ServiceError::Internal(error.to_string()))
}

fn parse_bifrost_scope(scope: &str) -> Result<bifrost_types::CursorScope, ServiceError> {
    match scope {
        "account" | "Account" => Ok(bifrost_types::CursorScope::Account),
        folder if folder.starts_with("folder:") => Ok(bifrost_types::CursorScope::Folder(
            bifrost_types::FolderId(folder["folder:".len()..].to_string()),
        )),
        other => Err(ServiceError::InvalidParams {
            method: "test.bifrost_inject_batch".into(),
            message: format!("unsupported bifrost test scope {other:?}"),
        }),
    }
}

fn bifrost_probe_scope_key(scope: &bifrost_types::CursorScope) -> String {
    match scope {
        bifrost_types::CursorScope::Account => "account".to_string(),
        bifrost_types::CursorScope::Folder(folder) => {
            format!("folder:{}:{}", folder.0.len(), folder.0)
        }
        other => format!("{other:?}"),
    }
}

fn synthetic_change_checkpoint(
    scope: bifrost_types::CursorScope,
    bytes: Vec<u8>,
) -> bifrost_types::Checkpoint {
    bifrost_types::Checkpoint::Change(bifrost_types::ChangeCursor {
        scope,
        server_state: bifrost_types::OpaqueChangeState {
            protocol: bifrost_types::ProtocolKind::Jmap,
            envelope_version: 1,
            bytes,
        },
        advanced_through: None,
        envelope_version: 1,
    })
}

fn synthetic_outcome(
    outcome: TestBifrostItemOutcome,
) -> crate::bifrost::consumer::hydrate::SyntheticOutcome {
    match outcome {
        TestBifrostItemOutcome::Succeeded => {
            crate::bifrost::consumer::hydrate::SyntheticOutcome::Succeeded
        }
        TestBifrostItemOutcome::DegradedBody => {
            crate::bifrost::consumer::hydrate::SyntheticOutcome::DegradedBody
        }
        TestBifrostItemOutcome::Failed => {
            crate::bifrost::consumer::hydrate::SyntheticOutcome::Failed
        }
        TestBifrostItemOutcome::Uncertain => {
            crate::bifrost::consumer::hydrate::SyntheticOutcome::Uncertain
        }
    }
}

async fn wait_for_bifrost_messages(
    db: &service_state::WriteDbState,
    account_id: &str,
    messages: &[service_api::TestBifrostSyntheticMessage],
) -> Result<(), ServiceError> {
    if messages.is_empty() {
        return Ok(());
    }
    let account_id = account_id.to_string();
    let ids = messages
        .iter()
        .map(|message| message.id.clone())
        .collect::<Vec<_>>();
    for _ in 0..30 {
        let account_id = account_id.clone();
        let ids = ids.clone();
        let present = db
            .with_read(move |conn| {
                let mut count = 0usize;
                for id in &ids {
                    let exists = conn
                        .query_row(
                            "SELECT 1 FROM messages WHERE account_id = ?1 AND id = ?2 LIMIT 1",
                            rusqlite::params![account_id, id],
                            |_| Ok(()),
                        )
                        .map(|()| true)
                        .or_else(|error| match error {
                            db::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows) => {
                                Ok(false)
                            }
                            other => Err(other.to_string()),
                        })?;
                    if exists {
                        count = count.saturating_add(1);
                    }
                }
                Ok(count)
            })
            .await
            .map_err(ServiceError::Internal)?;
        if present == messages.len() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(ServiceError::Internal(
        "timed out waiting for bifrost synthetic messages".into(),
    ))
}

async fn wait_for_bifrost_checkpoint(
    db: &service_state::WriteDbState,
    account_id: &str,
    scope: &bifrost_types::CursorScope,
    checkpoint_blob: &[u8],
) -> Result<(), ServiceError> {
    let account_id = account_id.to_string();
    let scope_key = bifrost_probe_scope_key(scope);
    let checkpoint_blob = checkpoint_blob.to_vec();
    for _ in 0..30 {
        let account_id = account_id.clone();
        let scope_key = scope_key.clone();
        let checkpoint_blob = checkpoint_blob.clone();
        let present = db
            .with_read(move |conn| {
                conn.query_row(
                    "SELECT 1 FROM sync_cursors \
                     WHERE account_id = ?1 AND scope_key = ?2 AND checkpoint_blob = ?3 LIMIT 1",
                    rusqlite::params![account_id, scope_key, checkpoint_blob],
                    |_| Ok(()),
                )
                .map(|()| true)
                .or_else(|error| match error {
                    db::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
                    other => Err(other.to_string()),
                })
            })
            .await
            .map_err(ServiceError::Internal)?;
        if present {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(ServiceError::Internal(
        "timed out waiting for bifrost checkpoint".into(),
    ))
}

pub(super) async fn bifrost_probe_handle(
    boot_state: &Arc<BootSharedState>,
    params: TestBifrostProbeParams,
) -> Result<Value, ServiceError> {
    let db = boot_state.write_db_state()?;
    let account_id = params.account_id.clone();
    let scope_key = bifrost_probe_scope_key(&parse_bifrost_scope(&params.scope)?);
    let seen_address = params.seen_address.clone();
    let searchable_message_id = params.searchable_message_id.clone();
    let (durable_cursor, times_sent_to, marker_rows, message_present) = db
        .with_read(move |conn| {
            let durable_cursor = conn
                .query_row(
                    "SELECT kind, scope_key, checkpoint_blob FROM sync_cursors \
                     WHERE account_id = ?1 AND scope_key = ?2 \
                     ORDER BY updated_at DESC LIMIT 1",
                    rusqlite::params![account_id, scope_key],
                    |row| {
                        Ok(TestBifrostDurableCursor {
                            kind: row.get(0)?,
                            scope_key: row.get(1)?,
                            checkpoint_blob: row.get(2)?,
                        })
                    },
                )
                .map(Some)
                .or_else(|error| match error {
                    db::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                    other => Err(other.to_string()),
                })?;
            let times_sent_to = if let Some(address) = seen_address {
                match conn.query_row(
                    "SELECT times_sent_to FROM seen_addresses \
                     WHERE account_id = ?1 AND email = ?2 LIMIT 1",
                    rusqlite::params![account_id, address],
                    |row| row.get::<_, i64>(0),
                ) {
                    Ok(value) => Some(value),
                    Err(db::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => None,
                    Err(error) => return Err(error.to_string()),
                }
            } else {
                None
            };
            let marker_rows: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM seen_ingest_markers WHERE account_id = ?1",
                    rusqlite::params![account_id],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            let message_present = if let Some(message_id) = searchable_message_id {
                conn.query_row(
                    "SELECT 1 FROM messages WHERE id = ?1 LIMIT 1",
                    rusqlite::params![message_id],
                    |_| Ok(()),
                )
                .map(|()| true)
                .or_else(|error| match error {
                    db::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
                    other => Err(other.to_string()),
                })
                .map(Some)?
            } else {
                None
            };
            Ok((durable_cursor, times_sent_to, marker_rows, message_present))
        })
        .await
        .map_err(ServiceError::Internal)?;
    let completion_edge = bifrost_completion()
        .lock()
        .map_err(|error| ServiceError::Internal(format!("bifrost completion lock: {error}")))?
        .get(&params.account_id)
        .map(|edge| edge.load(Ordering::Relaxed));
    serde_json::to_value(TestBifrostProbeAck {
        durable_cursor,
        times_sent_to,
        is_searchable: message_present,
        marker_rows: marker_rows.try_into().unwrap_or(u32::MAX),
        completion_edge,
    })
    .map_err(|error| ServiceError::Internal(error.to_string()))
}

pub(super) async fn counter_read_handle(counter: String) -> Result<Value, ServiceError> {
    let value =
        crate::test_counters::read(&counter).ok_or_else(|| ServiceError::InvalidParams {
            method: "test.counter_read".into(),
            message: format!("unknown counter {counter:?}"),
        })?;
    serde_json::to_value(TestCounterReadAck { counter, value })
        .map_err(|error| ServiceError::Internal(error.to_string()))
}

pub(super) async fn crash_after_n_writes_handle(
    params: TestCrashAfterNWritesParams,
) -> Result<Value, ServiceError> {
    crate::test_counters::configure_crash(params.kind, params.n).map_err(|message| {
        ServiceError::InvalidParams {
            method: "test.crash_after_n_writes".into(),
            message,
        }
    })?;
    serde_json::to_value(TestCrashAfterNWritesAck)
        .map_err(|error| ServiceError::Internal(error.to_string()))
}

pub(super) async fn seed_thread_handle(
    boot_state: &Arc<BootSharedState>,
    params: TestSeedThreadParams,
) -> Result<Value, ServiceError> {
    if params.account_id.is_empty() {
        return Err(ServiceError::InvalidParams {
            method: "test.seed_thread".into(),
            message: "account_id is required".into(),
        });
    }
    let write_db = boot_state.write_db_state()?;
    let account_id = params.account_id.clone();
    let thread_id = params
        .thread_id
        .clone()
        .unwrap_or_else(|| format!("thread-{}", uuid::Uuid::new_v4()));
    let message_id = params
        .message_id
        .clone()
        .unwrap_or_else(|| format!("message-{}", uuid::Uuid::new_v4()));
    let ack = TestSeedThreadAck {
        account_id: account_id.clone(),
        thread_id: thread_id.clone(),
        message_id: message_id.clone(),
    };
    let body_html = params.body_html.clone();
    let body_text = Some(
        params
            .body_text
            .clone()
            .unwrap_or_else(|| "Harness message".into()),
    );
    let app_data_dir = boot_state.app_data_dir().to_path_buf();
    write_db
        .with_write(move |conn| insert_harness_thread(conn, params, &thread_id, &message_id))
        .await
        .map_err(ServiceError::Internal)?;
    store::body_store::BodyStoreReadState::init(&app_data_dir)
        .map_err(ServiceError::Internal)?
        .put(ack.message_id.clone(), body_html, body_text)
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(ack).map_err(|error| ServiceError::Internal(error.to_string()))
}

pub(super) async fn seed_cached_attachment_handle(
    boot_state: &Arc<BootSharedState>,
    params: TestSeedCachedAttachmentParams,
) -> Result<Value, ServiceError> {
    if params.account_id.is_empty() {
        return Err(ServiceError::InvalidParams {
            method: "test.seed_cached_attachment".into(),
            message: "account_id is required".into(),
        });
    }
    if params.message_id.is_empty() {
        return Err(ServiceError::InvalidParams {
            method: "test.seed_cached_attachment".into(),
            message: "message_id is required".into(),
        });
    }
    let write_db = boot_state.write_db_state()?;
    let pack_store = boot_state.pack_store().ok_or_else(|| {
        ServiceError::Internal("pack store not installed; UI must wait for boot.ready".into())
    })?;
    let attachment_id = params
        .attachment_id
        .clone()
        .unwrap_or_else(|| format!("attachment-{}", uuid::Uuid::new_v4()));
    let filename = params.filename.unwrap_or_else(|| "harness.txt".into());
    let mime_type = params.mime_type.unwrap_or_else(|| "text/plain".into());
    let bytes = params.content.into_bytes();
    let size_bytes = u64::try_from(bytes.len())
        .map_err(|e| ServiceError::Internal(format!("attachment size conversion: {e}")))?;
    let size_i64 = i64::try_from(bytes.len())
        .map_err(|e| ServiceError::Internal(format!("attachment size conversion: {e}")))?;
    let content_hash = pack_store
        .put(bytes)
        .await
        .map_err(|e| ServiceError::Internal(format!("PackStore::put: {e}")))?;
    let hash_hex = content_hash.to_hex();
    // The harness ack mirrors the historical "attachment_cache/<hash>"
    // shape so existing scripts that compare against the relative_path
    // keep working. The path no longer corresponds to a real file on
    // disk - bytes live in PackStore - but harness callers that need
    // the bytes go through `attachment.fetch`, which materializes them
    // into `attachment_fetch_tmp/<hash>-<uuid>`.
    let relative_path = format!("attachment_cache/{hash_hex}");
    let account_id = params.account_id.clone();
    let message_id = params.message_id.clone();
    let ack = TestSeedCachedAttachmentAck {
        account_id: account_id.clone(),
        message_id: message_id.clone(),
        attachment_id: attachment_id.clone(),
        content_hash: hash_hex,
        relative_path,
        size_bytes,
    };
    write_db
        .with_write(move |conn| {
            insert_harness_cached_attachment(
                conn,
                &CachedAttachmentInsert {
                    account_id: &account_id,
                    message_id: &message_id,
                    attachment_id: &attachment_id,
                    filename: &filename,
                    mime_type: &mime_type,
                    size_bytes: size_i64,
                    content_hash: &content_hash,
                },
            )
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(ack).map_err(|error| ServiceError::Internal(error.to_string()))
}

pub(super) async fn seed_remote_attachment_handle(
    boot_state: &Arc<BootSharedState>,
    params: TestSeedRemoteAttachmentParams,
) -> Result<Value, ServiceError> {
    if params.account_id.is_empty() {
        return Err(ServiceError::InvalidParams {
            method: "test.seed_remote_attachment".into(),
            message: "account_id is required".into(),
        });
    }
    if params.message_id.is_empty() {
        return Err(ServiceError::InvalidParams {
            method: "test.seed_remote_attachment".into(),
            message: "message_id is required".into(),
        });
    }
    let bytes = common::encoding::decode_base64_standard(&params.content_base64)
        .map_err(ServiceError::Internal)?;
    let size_bytes = u64::try_from(bytes.len())
        .map_err(|e| ServiceError::Internal(format!("attachment size conversion: {e}")))?;
    let size_i64 = i64::try_from(bytes.len())
        .map_err(|e| ServiceError::Internal(format!("attachment size conversion: {e}")))?;
    let attachment_id = params
        .attachment_id
        .clone()
        .unwrap_or_else(|| format!("attachment-{}", uuid::Uuid::new_v4()));
    let filename = params.filename.unwrap_or_else(|| "harness.bin".into());
    let mime_type = params
        .mime_type
        .unwrap_or_else(|| "application/octet-stream".into());
    let account_id = params.account_id.clone();
    let message_id = params.message_id.clone();
    let registered_account_id = account_id.clone();
    let registered_message_id = message_id.clone();
    let registered_attachment_id = attachment_id.clone();
    let registered_bytes = bytes.clone();
    let write_db = boot_state.write_db_state()?;
    let ack = TestSeedRemoteAttachmentAck {
        account_id: account_id.clone(),
        message_id: message_id.clone(),
        attachment_id: attachment_id.clone(),
        size_bytes,
    };
    write_db
        .with_write(move |conn| {
            insert_harness_remote_attachment(
                conn,
                &RemoteAttachmentInsert {
                    account_id: &account_id,
                    message_id: &message_id,
                    attachment_id: &attachment_id,
                    filename: &filename,
                    mime_type: &mime_type,
                    size_bytes: size_i64,
                },
            )
        })
        .await
        .map_err(ServiceError::Internal)?;
    crate::actions::provider::register_harness_attachment(
        &registered_account_id,
        &registered_message_id,
        &registered_attachment_id,
        registered_bytes,
    );
    serde_json::to_value(ack).map_err(|error| ServiceError::Internal(error.to_string()))
}

pub(super) async fn remove_cached_attachment_bytes_handle(
    boot_state: &Arc<BootSharedState>,
    params: TestRemoveCachedAttachmentBytesParams,
) -> Result<Value, ServiceError> {
    if params.relative_path.is_empty() {
        return Err(ServiceError::InvalidParams {
            method: "test.remove_cached_attachment_bytes".into(),
            message: "relative_path is required".into(),
        });
    }
    // Attachments roadmap Phase 3: bytes live in PackStore, not on
    // disk under `attachment_cache/<hash>`. Harness scripts still pass
    // the path returned by `seed_cached_attachment`; we parse the hex
    // hash out of it and tombstone the blob in PackStore so subsequent
    // `attachment.fetch` calls go through the cache-miss path.
    let pack_store = boot_state.pack_store().ok_or_else(|| {
        ServiceError::Internal("pack store not installed; UI must wait for boot.ready".into())
    })?;
    let hash_hex = params
        .relative_path
        .strip_prefix("attachment_cache/")
        .or_else(|| params.relative_path.strip_prefix("attachment_fetch_tmp/"))
        .ok_or_else(|| {
            ServiceError::Internal(format!(
                "test.remove_cached_attachment_bytes: relative_path must start with \
                 attachment_cache/ or attachment_fetch_tmp/: {}",
                params.relative_path
            ))
        })?;
    // Strip any trailing -<uuid> suffix from the tmp-file form.
    let hash_hex = hash_hex.split('-').next().unwrap_or(hash_hex);
    let hash = db::blob_hash::BlobHash::from_hex(hash_hex)
        .map_err(|e| ServiceError::Internal(format!("parse content hash: {e}")))?;
    pack_store
        .tombstone(&hash)
        .await
        .map_err(|e| ServiceError::Internal(format!("PackStore::tombstone: {e}")))?;
    serde_json::to_value(TestRemoveCachedAttachmentBytesAck {
        relative_path: params.relative_path,
        removed: true,
    })
    .map_err(|error| ServiceError::Internal(error.to_string()))
}

pub(super) async fn thread_read_handle(
    boot_state: &Arc<BootSharedState>,
    params: TestThreadReadParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    let ack = write_db
        .with_write(move |conn| read_harness_thread(conn, &params))
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(ack).map_err(|error| ServiceError::Internal(error.to_string()))
}

pub(super) async fn pending_ops_read_handle(
    boot_state: &Arc<BootSharedState>,
    params: TestPendingOpsReadParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    let ack = write_db
        .with_write(move |conn| read_harness_pending_ops(conn, &params))
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(ack).map_err(|error| ServiceError::Internal(error.to_string()))
}

pub(super) async fn start_sync_handle(
    boot_state: &Arc<BootSharedState>,
    params: TestStartSyncParams,
) -> Result<Value, ServiceError> {
    if params.account_id.is_empty() {
        return Err(ServiceError::InvalidParams {
            method: "test.start_sync".into(),
            message: "account_id is required".into(),
        });
    }
    let runtime = boot_state.sync_runtime().ok_or_else(|| {
        ServiceError::Internal("test.start_sync received before SyncRuntime was installed".into())
    })?;
    let ack = runtime.start_account(params.account_id).await;
    serde_json::to_value(ack).map_err(|error| ServiceError::Internal(error.to_string()))
}

pub(super) async fn query_db_state_handle(
    boot_state: &Arc<BootSharedState>,
    params: TestQueryDbStateParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    let encryption_key = boot_state.encryption_key();
    let ack = write_db
        .with_write(move |conn| read_harness_db_state(conn, &params, encryption_key))
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(ack).map_err(|error| ServiceError::Internal(error.to_string()))
}

pub(super) async fn search_index_handle(
    boot_state: &Arc<BootSharedState>,
    params: TestSearchIndexParams,
) -> Result<Value, ServiceError> {
    let query = params.query.trim().to_string();
    if query.is_empty() {
        return Err(ServiceError::InvalidParams {
            method: "test.search_index".into(),
            message: "query is required".into(),
        });
    }
    let limit = usize::try_from(params.limit.unwrap_or(20).min(100))
        .map_err(|e| ServiceError::Internal(format!("search limit conversion: {e}")))?;

    // Normal post-boot handlers read the writer from BootSharedState.
    // The sync-runtime fallback covers older harness boot shapes and
    // tests that construct sync first, where the same writer handle is
    // reachable through the runtime.
    if let Some(search_write) = boot_state.search_write().or_else(|| {
        boot_state
            .sync_runtime()
            .map(|runtime| runtime.search_write())
    }) {
        search_write
            .flush_now()
            .await
            .map_err(|e| ServiceError::Internal(format!("test.search_index flush: {e}")))?;
    }

    let search_read = search::SearchReadState::init(boot_state.app_data_dir())
        .map_err(|e| ServiceError::Internal(format!("test.search_index open: {e}")))?;
    search_read
        .reload()
        .map_err(|e| ServiceError::Internal(format!("test.search_index reload: {e}")))?;

    let account_ids = params
        .account_id
        .filter(|account_id| !account_id.is_empty())
        .map(|account_id| vec![account_id]);
    let mut results = search_read
        .search_with_filters(&search::SearchParams {
            account_ids,
            free_text: Some(query.clone()),
            from: Vec::new(),
            to: Vec::new(),
            subject: None,
            has_attachment: None,
            is_unread: None,
            is_starred: None,
            before: None,
            after: None,
            thread_filter: None,
            limit: Some(limit),
        })
        .map_err(|e| ServiceError::Internal(format!("test.search_index query: {e}")))?;
    enrich_test_search_results(boot_state, &search_read, &query, &mut results).await?;

    let mut rows = Vec::with_capacity(results.len());
    for result in results {
        let match_kind = serde_json::to_value(result.match_kind)
            .map_err(|e| ServiceError::Internal(format!("serialize match_kind: {e}")))?;
        let also_matched = result
            .also_matched
            .into_iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| ServiceError::Internal(format!("serialize also_matched: {e}")))?;
        rows.push(TestSearchIndexResult {
            message_id: result.message_id,
            account_id: result.account_id,
            thread_id: result.thread_id,
            subject: result.subject,
            snippet: result.snippet,
            rank: result.rank,
            match_kind,
            also_matched,
        });
    }
    let total = u64::try_from(rows.len())
        .map_err(|e| ServiceError::Internal(format!("search total conversion: {e}")))?;
    serde_json::to_value(TestSearchIndexAck {
        total,
        results: rows,
    })
    .map_err(|error| ServiceError::Internal(error.to_string()))
}

async fn enrich_test_search_results(
    boot_state: &Arc<BootSharedState>,
    search_read: &search::SearchReadState,
    query: &str,
    results: &mut [search::SearchResult],
) -> Result<(), ServiceError> {
    if query.trim().is_empty() || results.is_empty() {
        return Ok(());
    }
    let mut rows = Vec::with_capacity(results.len());
    let mut pairs = Vec::with_capacity(results.len());
    let mut message_ids = Vec::with_capacity(results.len());
    for result in results.iter() {
        rows.push((
            result.account_id.clone(),
            result.message_id.clone(),
            result.subject.clone(),
            result.from_name.clone(),
        ));
        pairs.push((result.account_id.clone(), result.message_id.clone()));
        message_ids.push(result.message_id.clone());
    }
    let body_read = store::body_store::BodyStoreReadState::init(boot_state.app_data_dir())
        .map_err(|e| ServiceError::Internal(format!("test.search_index body store: {e}")))?;
    let write_db = boot_state.write_db_state()?;
    let inputs = write_db
        .with_write(move |conn| {
            let read = conn.as_read();
            let fragments =
                db::db::queries_extra::select_attachment_fragments_batch(&read, &pairs)?;
            let mut body_by_mid: HashMap<String, String> = HashMap::new();
            for body in body_read.get_batch_sync(&message_ids)? {
                if let Some(text) = body.body_text {
                    body_by_mid.insert(body.message_id, text);
                }
            }

            let mut inputs = HashMap::with_capacity(rows.len());
            for (account_id, message_id, subject, from_name) in rows {
                let key = (account_id, message_id.clone());
                let attachments = fragments
                    .get(&key)
                    .map(|rows| {
                        rows.iter()
                            .map(|row| search::AttachmentAttributionInput {
                                attachment_id: row.attachment_id.clone(),
                                filename: row.filename.clone(),
                                mime: row.mime_type.clone(),
                                extracted_text: row.extracted_text.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                inputs.insert(
                    message_id.clone(),
                    search::AttributionInputs {
                        subject: subject.unwrap_or_default(),
                        from_name: from_name.unwrap_or_default(),
                        body_text: body_by_mid.remove(&message_id).unwrap_or_default(),
                        attachments,
                    },
                );
            }
            Ok(inputs)
        })
        .await
        .map_err(ServiceError::Internal)?;
    search_read
        .enrich_match_kinds(query, results, &inputs)
        .map_err(|e| ServiceError::Internal(format!("test.search_index attribution: {e}")))
}

pub(super) async fn delay_next_write_handle(
    params: TestDelayNextWriteParams,
) -> Result<Value, ServiceError> {
    crate::test_counters::configure_delay(params.kind, params.millis).map_err(|message| {
        ServiceError::InvalidParams {
            method: "test.delay_next_write".into(),
            message,
        }
    })?;
    serde_json::to_value(TestDelayNextWriteAck)
        .map_err(|error| ServiceError::Internal(error.to_string()))
}

/// Phase 8c: read `attachment_blobs.tombstoned_at` for a single
/// content_hash. Harness scripts use this after account-delete or
/// clear-cache flows once the referencing `attachments` row has
/// cascade-deleted; the blob's tombstone state survives.
pub(super) async fn query_blob_tombstone_state_handle(
    boot_state: &Arc<BootSharedState>,
    params: TestQueryBlobTombstoneStateParams,
) -> Result<Value, ServiceError> {
    let hex = params.content_hash;
    let hash =
        db::blob_hash::BlobHash::from_hex(&hex).map_err(|e| ServiceError::InvalidParams {
            method: "test.query_blob_tombstone_state".into(),
            message: format!("content_hash: {e}"),
        })?;
    let db_state = boot_state
        .write_db_state()
        .map_err(|_| ServiceError::Internal("write_db_state unavailable".into()))?;
    let ack: TestQueryBlobTombstoneStateAck = db_state
        .with_write(move |conn| {
            let row: Option<Option<i64>> = conn
                .query_row(
                    "SELECT tombstoned_at FROM attachment_blobs WHERE content_hash = ?1",
                    rusqlite::params![hash],
                    |r| r.get::<_, Option<i64>>(0),
                )
                .ok();
            Ok(match row {
                Some(ts) => TestQueryBlobTombstoneStateAck {
                    present: true,
                    tombstoned_at: ts,
                },
                None => TestQueryBlobTombstoneStateAck {
                    present: false,
                    tombstoned_at: None,
                },
            })
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(ack).map_err(|e| ServiceError::Internal(e.to_string()))
}

/// Harness probe: runs `rtsk::discovery::discover` for the given email
/// and returns the full `DiscoveredConfig` as JSON. Wired against
/// saehrimnir via `RATATOSKR_TEST_DISCOVERY_BASE`; the cascade's reqwest
/// clients pick the env var up and route to the local listener.
pub(super) async fn run_discovery_handle(
    params: TestRunDiscoveryParams,
) -> Result<Value, ServiceError> {
    let config = rtsk::discovery::discover(&params.email)
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(config).map_err(|e| ServiceError::Internal(e.to_string()))
}

fn insert_harness_account_rows(
    conn: &impl db::db::WriteTransactionTarget,
    account_id: &str,
    email: &str,
) -> Result<u64, String> {
    let mut label_count = 0_u64;
    for (sort_order, role) in db::db::folder_roles::SYSTEM_FOLDER_ROLES.iter().enumerate() {
        let changed = conn
            .execute(
                "INSERT INTO folders (
                    id, account_id, name, visible, sort_order,
                    imap_folder_path, imap_special_use, is_undeletable
                ) VALUES (?1, ?2, ?3, 1, ?4, ?3, ?5, 1)
                ON CONFLICT(account_id, id) DO NOTHING",
                params![
                    role.label_id,
                    account_id,
                    role.label_name,
                    i64::try_from(sort_order).map_err(|e| e.to_string())?,
                    role.imap_special_use,
                ],
            )
            .map_err(|e| format!("insert system folder {}: {e}", role.label_id))?;
        label_count = add_changed_rows(label_count, changed)?;
    }

    let changed = conn
        .execute(
            "INSERT INTO labels (
                id, account_id, name, visible, sort_order
            ) VALUES ('harness-label', ?1, 'Harness', 1, 1000)
            ON CONFLICT(account_id, id) DO NOTHING",
            params![account_id],
        )
        .map_err(|e| format!("insert harness label: {e}"))?;
    label_count = add_changed_rows(label_count, changed)?;

    conn.execute(
        "INSERT INTO signatures (
            id, account_id, name, body_html, body_text, is_default,
            is_reply_default, sort_order
        ) VALUES (?1, ?2, 'Harness', '', '', 1, 1, 0)",
        params![uuid::Uuid::new_v4().to_string(), account_id],
    )
    .map_err(|e| format!("insert harness signature: {e}"))?;

    conn.execute(
        "INSERT INTO send_identities (
            account_id, email, display_name, is_primary
        ) VALUES (?1, ?2, 'Harness', 1)",
        params![account_id, email],
    )
    .map_err(|e| format!("insert harness send identity: {e}"))?;

    Ok(label_count)
}

fn insert_harness_thread(
    conn: &impl db::db::WriteTransactionTarget,
    params: TestSeedThreadParams,
    thread_id: &str,
    message_id: &str,
) -> Result<(), String> {
    let subject = params.subject.unwrap_or_else(|| "Harness thread".into());
    let label_ids = if params.label_ids.is_empty() {
        vec!["INBOX".to_string()]
    } else {
        params.label_ids
    };
    let account_email: String = conn
        .query_row(
            "SELECT email FROM accounts WHERE id = ?1",
            params![params.account_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("query account email: {e}"))?;
    let chat_email = params.chat_email.map(|email| email.to_lowercase());
    let from_address = chat_email
        .clone()
        .unwrap_or_else(|| "sender@example.test".into());
    let is_chat_thread = params.is_chat_thread || chat_email.is_some();
    let unread_messages = if params.is_read { 0_i64 } else { 1_i64 };
    let tx = conn.transaction().map_err(|e| format!("begin: {e}"))?;

    tx.execute(
        "INSERT INTO threads (
            id, account_id, subject, snippet, last_message_at, message_count,
            is_read, is_starred, is_pinned, is_muted, is_chat_thread
        ) VALUES (?1, ?2, ?3, ?4, 1700000000, 1, ?5, ?6, ?7, ?8, ?9)
        ON CONFLICT(account_id, id) DO UPDATE SET
            subject = excluded.subject,
            snippet = excluded.snippet,
            last_message_at = excluded.last_message_at,
            message_count = excluded.message_count,
            is_read = excluded.is_read,
            is_starred = excluded.is_starred,
            is_pinned = excluded.is_pinned,
            is_muted = excluded.is_muted,
            is_chat_thread = excluded.is_chat_thread",
        params![
            thread_id,
            params.account_id,
            subject,
            "Harness message",
            params.is_read,
            params.is_starred,
            params.is_pinned,
            params.is_muted,
            is_chat_thread,
        ],
    )
    .map_err(|e| format!("insert thread: {e}"))?;

    tx.execute(
        "DELETE FROM thread_labels WHERE account_id = ?1 AND thread_id = ?2",
        params![params.account_id, thread_id],
    )
    .map_err(|e| format!("clear thread labels: {e}"))?;
    tx.execute(
        "DELETE FROM thread_folders WHERE account_id = ?1 AND thread_id = ?2",
        params![params.account_id, thread_id],
    )
    .map_err(|e| format!("clear thread folders: {e}"))?;
    for label_id in label_ids {
        if db::db::folder_roles::is_gmail_system_folder_label_id(label_id.as_str()) {
            tx.execute(
                "INSERT OR IGNORE INTO thread_folders (account_id, thread_id, folder_id)
                 VALUES (?1, ?2, ?3)",
                params![params.account_id, thread_id, label_id],
            )
            .map_err(|e| format!("insert thread folder: {e}"))?;
        } else {
            tx.execute(
                "INSERT OR IGNORE INTO thread_labels (account_id, thread_id, label_id)
                 VALUES (?1, ?2, ?3)",
                params![params.account_id, thread_id, label_id],
            )
            .map_err(|e| format!("insert thread label: {e}"))?;
        }
    }

    tx.execute(
        "INSERT INTO messages (
            id, account_id, thread_id, from_address, to_addresses, subject,
            snippet, date, is_read, is_starred, message_id_header
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1700000000, ?8, ?9, ?10)
        ON CONFLICT(account_id, id) DO UPDATE SET
            thread_id = excluded.thread_id,
            from_address = excluded.from_address,
            to_addresses = excluded.to_addresses,
            subject = excluded.subject,
            snippet = excluded.snippet,
            date = excluded.date,
            is_read = excluded.is_read,
            is_starred = excluded.is_starred,
            message_id_header = excluded.message_id_header",
        params![
            message_id,
            params.account_id,
            thread_id,
            from_address,
            account_email,
            subject,
            "Harness message",
            params.is_read,
            params.is_starred,
            format!("<{message_id}@example.test>"),
        ],
    )
    .map_err(|e| format!("insert message: {e}"))?;

    tx.execute(
        "INSERT OR IGNORE INTO thread_participants (account_id, thread_id, email)
         VALUES (?1, ?2, ?3)",
        params![params.account_id, thread_id, account_email.to_lowercase()],
    )
    .map_err(|e| format!("insert account participant: {e}"))?;
    tx.execute(
        "INSERT OR IGNORE INTO thread_participants (account_id, thread_id, email)
         VALUES (?1, ?2, ?3)",
        params![params.account_id, thread_id, from_address.to_lowercase()],
    )
    .map_err(|e| format!("insert sender participant: {e}"))?;

    if let Some(chat_email) = chat_email {
        tx.execute(
            "INSERT INTO chat_contacts (
                email, display_name, latest_message_at, latest_message_preview,
                unread_count
            ) VALUES (?1, 'Harness Chat', 1700000000, ?2, ?3)
            ON CONFLICT(email) DO UPDATE SET
                latest_message_at = excluded.latest_message_at,
                latest_message_preview = excluded.latest_message_preview,
                unread_count = excluded.unread_count",
            params![chat_email, "Harness message", unread_messages],
        )
        .map_err(|e| format!("insert chat contact: {e}"))?;
    }

    tx.commit().map_err(|e| format!("commit: {e}"))
}

struct CachedAttachmentInsert<'a> {
    account_id: &'a str,
    message_id: &'a str,
    attachment_id: &'a str,
    filename: &'a str,
    mime_type: &'a str,
    size_bytes: i64,
    content_hash: &'a db::blob_hash::BlobHash,
}

struct RemoteAttachmentInsert<'a> {
    account_id: &'a str,
    message_id: &'a str,
    attachment_id: &'a str,
    filename: &'a str,
    mime_type: &'a str,
    size_bytes: i64,
}

fn insert_harness_cached_attachment(
    conn: &impl db::db::WriteTransactionTarget,
    insert: &CachedAttachmentInsert<'_>,
) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| format!("begin: {e}"))?;
    tx.execute(
        "INSERT INTO attachments (
            id, message_id, account_id, filename, mime_type, size,
            content_hash, text_indexed_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)
        ON CONFLICT(id) DO UPDATE SET
            message_id = excluded.message_id,
            account_id = excluded.account_id,
            filename = excluded.filename,
            mime_type = excluded.mime_type,
            size = excluded.size,
            content_hash = excluded.content_hash,
            text_indexed_at = NULL",
        params![
            insert.attachment_id,
            insert.message_id,
            insert.account_id,
            insert.filename,
            insert.mime_type,
            insert.size_bytes,
            insert.content_hash,
        ],
    )
    .map_err(|e| format!("insert cached attachment: {e}"))?;
    tx.commit().map_err(|e| format!("commit: {e}"))
}

fn insert_harness_remote_attachment(
    conn: &impl db::db::WriteTransactionTarget,
    insert: &RemoteAttachmentInsert<'_>,
) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| format!("begin: {e}"))?;
    tx.execute(
        "INSERT INTO attachments (
            id, message_id, account_id, filename, mime_type, size,
            content_hash, text_indexed_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL)
        ON CONFLICT(id) DO UPDATE SET
            message_id = excluded.message_id,
            account_id = excluded.account_id,
            filename = excluded.filename,
            mime_type = excluded.mime_type,
            size = excluded.size,
            content_hash = NULL,
            text_indexed_at = NULL",
        params![
            insert.attachment_id,
            insert.message_id,
            insert.account_id,
            insert.filename,
            insert.mime_type,
            insert.size_bytes,
        ],
    )
    .map_err(|e| format!("insert remote attachment: {e}"))?;
    tx.commit().map_err(|e| format!("commit: {e}"))
}

fn read_harness_thread(
    conn: &impl db::db::WriteTransactionTarget,
    params: &TestThreadReadParams,
) -> Result<TestThreadReadAck, String> {
    let flags = conn
        .query_row(
            "SELECT is_read, is_starred, is_pinned, is_muted, is_chat_thread
             FROM threads WHERE account_id = ?1 AND id = ?2",
            params![params.account_id, params.thread_id],
            |row| {
                Ok((
                    row.get::<_, bool>(0)?,
                    row.get::<_, bool>(1)?,
                    row.get::<_, bool>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, bool>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|e| format!("query thread: {e}"))?;
    let Some((is_read, is_starred, is_pinned, is_muted, is_chat_thread)) = flags else {
        return Ok(TestThreadReadAck {
            exists: false,
            is_read: false,
            is_starred: false,
            is_pinned: false,
            is_muted: false,
            is_chat_thread: false,
            label_ids: Vec::new(),
            unread_messages: 0,
        });
    };

    let mut stmt = conn
        .prepare(
            "SELECT label_id FROM thread_labels
             WHERE account_id = ?1 AND thread_id = ?2
             UNION
             SELECT folder_id AS label_id FROM thread_folders
             WHERE account_id = ?1 AND thread_id = ?2
             ORDER BY label_id",
        )
        .map_err(|e| format!("prepare labels: {e}"))?;
    let label_ids = stmt
        .query_map(params![params.account_id, params.thread_id], |row| {
            row.get(0)
        })
        .map_err(|e| format!("query labels: {e}"))?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|e| format!("collect labels: {e}"))?;
    let unread_messages: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM messages
             WHERE account_id = ?1 AND thread_id = ?2 AND is_read = 0",
            params![params.account_id, params.thread_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("query unread messages: {e}"))?;
    let unread_messages = u64::try_from(unread_messages).map_err(|e| e.to_string())?;

    Ok(TestThreadReadAck {
        exists: true,
        is_read,
        is_starred,
        is_pinned,
        is_muted,
        is_chat_thread,
        label_ids,
        unread_messages,
    })
}

fn read_harness_pending_ops(
    conn: &impl db::db::WriteTransactionTarget,
    filters: &TestPendingOpsReadParams,
) -> Result<TestPendingOpsReadAck, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, account_id, operation_type, resource_id, params, status,
                    retry_count, max_retries, next_retry_at, created_at,
                    error_message
             FROM pending_operations
             ORDER BY created_at ASC, id ASC",
        )
        .map_err(|e| format!("prepare pending ops: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(TestPendingOpRow {
                id: row.get(0)?,
                account_id: row.get(1)?,
                operation_type: row.get(2)?,
                resource_id: row.get(3)?,
                params: row.get(4)?,
                status: row.get(5)?,
                retry_count: row.get(6)?,
                max_retries: row.get(7)?,
                next_retry_at: row.get(8)?,
                created_at: row.get(9)?,
                error_message: row.get(10)?,
            })
        })
        .map_err(|e| format!("query pending ops: {e}"))?;

    let mut operations = Vec::new();
    for row in rows {
        let op = row.map_err(|e| format!("collect pending op: {e}"))?;
        if let Some(account_id) = &filters.account_id
            && op.account_id != *account_id
        {
            continue;
        }
        if let Some(resource_id) = &filters.resource_id
            && op.resource_id != *resource_id
        {
            continue;
        }
        if let Some(operation_type) = &filters.operation_type
            && op.operation_type != *operation_type
        {
            continue;
        }
        if let Some(status) = &filters.status
            && op.status != *status
        {
            continue;
        }
        operations.push(op);
    }

    let total = u64::try_from(operations.len()).map_err(|e| e.to_string())?;
    let pending = u64::try_from(
        operations
            .iter()
            .filter(|op| op.status == "pending")
            .count(),
    )
    .map_err(|e| e.to_string())?;
    let failed = u64::try_from(operations.iter().filter(|op| op.status == "failed").count())
        .map_err(|e| e.to_string())?;

    Ok(TestPendingOpsReadAck {
        total,
        pending,
        failed,
        operations,
    })
}

fn read_harness_db_state(
    conn: &impl db::db::WriteTransactionTarget,
    params: &TestQueryDbStateParams,
    encryption_key: Option<[u8; 32]>,
) -> Result<TestQueryDbStateAck, String> {
    let account_id = params.account_id.as_deref();
    Ok(TestQueryDbStateAck {
        account_count: count_accounts(conn, account_id)?,
        folder_count: count_account_rows(conn, "folders", account_id)?,
        label_count: count_account_rows(conn, "labels", account_id)?,
        thread_count: count_account_rows(conn, "threads", account_id)?,
        thread_label_count: count_account_rows(conn, "thread_labels", account_id)?,
        message_count: count_account_rows(conn, "messages", account_id)?,
        unread_message_count: count_unread_messages(conn, account_id)?,
        attachment_count: count_account_rows(conn, "attachments", account_id)?,
        local_draft_count: count_account_rows(conn, "local_drafts", account_id)?,
        calendar_count: count_account_rows(conn, "calendars", account_id)?,
        calendar_event_count: count_account_rows(conn, "calendar_events", account_id)?,
        contact_count: count_account_rows(conn, "contacts", account_id)?,
        contact_group_count: count_account_rows(conn, "contact_groups", account_id)?,
        accounts: read_harness_accounts(conn, account_id, encryption_key.as_ref())?,
        folders: read_harness_folders(conn, account_id)?,
        labels: read_harness_labels(conn, account_id)?,
        signatures: read_harness_signatures(conn, account_id)?,
        messages: read_harness_messages(conn, params)?,
        local_drafts: read_harness_local_drafts(conn, params)?,
        attachments: read_harness_attachments(conn, params)?,
        calendars: read_harness_calendars(conn, params)?,
        calendar_events: read_harness_calendar_events(conn, params)?,
        contacts: read_harness_contacts(conn, params)?,
        contact_groups: read_harness_contact_groups(conn, params)?,
    })
}

struct HarnessAccountRaw {
    id: String,
    email: String,
    provider: String,
    auth_method: String,
    oauth_provider: Option<String>,
    oauth_client_id: Option<String>,
    token_expires_at: Option<i64>,
    initial_sync_completed: bool,
    access_token: Option<String>,
    refresh_token: Option<String>,
}

fn read_harness_accounts(
    conn: &impl db::db::WriteTransactionTarget,
    account_id: Option<&str>,
    encryption_key: Option<&[u8; 32]>,
) -> Result<Vec<TestDbAccountRow>, String> {
    let where_clause = if account_id.is_some() {
        " WHERE id = ?1"
    } else {
        ""
    };
    let sql = format!(
        "SELECT id, email, provider, auth_method, oauth_provider,
                oauth_client_id, token_expires_at, initial_sync_completed,
                access_token, refresh_token
         FROM accounts{where_clause}
         ORDER BY email ASC, id ASC"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("prepare accounts: {e}"))?;
    let rows = match account_id {
        Some(account_id) => stmt
            .query_map(params![account_id], account_raw_from_row)
            .map_err(|e| format!("query accounts: {e}"))?,
        None => stmt
            .query_map([], account_raw_from_row)
            .map_err(|e| format!("query accounts: {e}"))?,
    };
    let mut accounts = Vec::new();
    for row in rows {
        accounts.push(test_db_account_from_raw(
            row.map_err(|e| format!("collect account: {e}"))?,
            encryption_key,
        )?);
    }
    Ok(accounts)
}

fn account_raw_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HarnessAccountRaw> {
    Ok(HarnessAccountRaw {
        id: row.get(0)?,
        email: row.get(1)?,
        provider: row.get(2)?,
        auth_method: row.get(3)?,
        oauth_provider: row.get(4)?,
        oauth_client_id: row.get(5)?,
        token_expires_at: row.get(6)?,
        initial_sync_completed: row.get::<_, i64>(7)? != 0,
        access_token: row.get(8)?,
        refresh_token: row.get(9)?,
    })
}

fn test_db_account_from_raw(
    raw: HarnessAccountRaw,
    encryption_key: Option<&[u8; 32]>,
) -> Result<TestDbAccountRow, String> {
    let access = credential_summary(raw.access_token.as_deref(), encryption_key)?;
    let refresh = credential_summary(raw.refresh_token.as_deref(), encryption_key)?;
    Ok(TestDbAccountRow {
        id: raw.id,
        email: raw.email,
        provider: raw.provider,
        auth_method: raw.auth_method,
        oauth_provider: raw.oauth_provider,
        oauth_client_id: raw.oauth_client_id,
        token_expires_at: raw.token_expires_at,
        initial_sync_completed: raw.initial_sync_completed,
        access_token_present: access.present,
        refresh_token_present: refresh.present,
        access_token_encrypted: access.encrypted,
        refresh_token_encrypted: refresh.encrypted,
        access_token_sha256: access.sha256,
        refresh_token_sha256: refresh.sha256,
    })
}

struct CredentialSummary {
    present: bool,
    encrypted: bool,
    sha256: Option<String>,
}

fn credential_summary(
    value: Option<&str>,
    encryption_key: Option<&[u8; 32]>,
) -> Result<CredentialSummary, String> {
    let Some(value) = value else {
        return Ok(CredentialSummary {
            present: false,
            encrypted: false,
            sha256: None,
        });
    };
    let (encrypted, plaintext) = decrypt_if_service_ciphertext(value, encryption_key)?
        .unwrap_or_else(|| (false, value.to_string()));
    let mut hasher = Sha256::new();
    hasher.update(plaintext.as_bytes());
    Ok(CredentialSummary {
        present: true,
        encrypted,
        sha256: Some(hex_bytes(&hasher.finalize())),
    })
}

// The harness wire envelope surfaces `folders` and `labels` as separate
// fields, mirroring the storage split. Scripts query
// `state.folders[id]` or `state.labels[id]` for the right table.

fn read_harness_folders(
    conn: &impl db::db::WriteTransactionTarget,
    account_id: Option<&str>,
) -> Result<Vec<TestDbFolderRow>, String> {
    let cols = "id, account_id, name, parent_id, imap_folder_path,
                imap_special_use, sort_order, visible, is_subscribed,
                is_undeletable";
    if let Some(id) = account_id {
        let sql = format!(
            "SELECT {cols} FROM folders WHERE account_id = ?1 \
             ORDER BY account_id ASC, id ASC"
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("prepare folders query: {e}"))?;
        let mapped = stmt
            .query_map(params![id], test_db_folder_from_row)
            .map_err(|e| format!("query folders: {e}"))?;
        collect_rows(mapped, "folders")
    } else {
        let sql = format!("SELECT {cols} FROM folders ORDER BY account_id ASC, id ASC");
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("prepare folders query: {e}"))?;
        let mapped = stmt
            .query_map([], test_db_folder_from_row)
            .map_err(|e| format!("query folders: {e}"))?;
        collect_rows(mapped, "folders")
    }
}

fn test_db_folder_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TestDbFolderRow> {
    let sort_order = row.get::<_, Option<i64>>(6)?.unwrap_or(0);
    let visible = row.get::<_, Option<i64>>(7)?.unwrap_or(1) != 0;
    let is_subscribed = row.get::<_, Option<i64>>(8)?.map(|value| value != 0);
    let is_undeletable = row.get::<_, Option<i64>>(9)?.unwrap_or(0) != 0;
    Ok(TestDbFolderRow {
        id: row.get(0)?,
        account_id: row.get(1)?,
        name: row.get(2)?,
        parent_id: row.get(3)?,
        imap_folder_path: row.get(4)?,
        imap_special_use: row.get(5)?,
        sort_order,
        visible,
        is_subscribed,
        is_undeletable,
    })
}

fn read_harness_labels(
    conn: &impl db::db::WriteTransactionTarget,
    account_id: Option<&str>,
) -> Result<Vec<TestDbLabelRow>, String> {
    let cols = "id, account_id, name, sort_order, visible, is_undeletable,
                server_color_bg, server_color_fg, user_color_bg, user_color_fg";
    if let Some(id) = account_id {
        let sql = format!(
            "SELECT {cols} FROM labels WHERE account_id = ?1 \
             ORDER BY account_id ASC, id ASC"
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("prepare labels query: {e}"))?;
        let mapped = stmt
            .query_map(params![id], test_db_label_from_row)
            .map_err(|e| format!("query labels: {e}"))?;
        collect_rows(mapped, "labels")
    } else {
        let sql = format!("SELECT {cols} FROM labels ORDER BY account_id ASC, id ASC");
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("prepare labels query: {e}"))?;
        let mapped = stmt
            .query_map([], test_db_label_from_row)
            .map_err(|e| format!("query labels: {e}"))?;
        collect_rows(mapped, "labels")
    }
}

fn test_db_label_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TestDbLabelRow> {
    let sort_order = row.get::<_, Option<i64>>(3)?.unwrap_or(0);
    let visible = row.get::<_, Option<i64>>(4)?.unwrap_or(1) != 0;
    let is_undeletable = row.get::<_, Option<i64>>(5)?.unwrap_or(0) != 0;
    Ok(TestDbLabelRow {
        id: row.get(0)?,
        account_id: row.get(1)?,
        name: row.get(2)?,
        sort_order,
        visible,
        is_undeletable,
        server_color_bg: row.get(6)?,
        server_color_fg: row.get(7)?,
        user_color_bg: row.get(8)?,
        user_color_fg: row.get(9)?,
    })
}

fn read_harness_signatures(
    conn: &impl db::db::WriteTransactionTarget,
    account_id: Option<&str>,
) -> Result<Vec<TestDbSignatureRow>, String> {
    match account_id {
        Some(account_id) => {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, name, body_html, body_text,
                            is_default, is_reply_default, sort_order,
                            source, server_id, server_html_hash
                     FROM signatures
                     WHERE account_id = ?1
                     ORDER BY account_id ASC, sort_order ASC, id ASC",
                )
                .map_err(|e| format!("prepare signatures query: {e}"))?;
            let mapped = stmt
                .query_map(params![account_id], test_db_signature_from_row)
                .map_err(|e| format!("query signatures: {e}"))?;
            collect_rows(mapped, "signatures")
        }
        None => {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, name, body_html, body_text,
                            is_default, is_reply_default, sort_order,
                            source, server_id, server_html_hash
                     FROM signatures
                     ORDER BY account_id ASC, sort_order ASC, id ASC",
                )
                .map_err(|e| format!("prepare signatures query: {e}"))?;
            let mapped = stmt
                .query_map([], test_db_signature_from_row)
                .map_err(|e| format!("query signatures: {e}"))?;
            collect_rows(mapped, "signatures")
        }
    }
}

fn test_db_signature_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TestDbSignatureRow> {
    let is_default = row.get::<_, Option<i64>>(5)?.unwrap_or(0) != 0;
    let is_reply_default = row.get::<_, Option<i64>>(6)?.unwrap_or(0) != 0;
    let sort_order = row.get::<_, Option<i64>>(7)?.unwrap_or(0);
    Ok(TestDbSignatureRow {
        id: row.get(0)?,
        account_id: row.get(1)?,
        name: row.get(2)?,
        body_html: row.get(3)?,
        body_text: row.get(4)?,
        is_default,
        is_reply_default,
        sort_order,
        source: row.get(8)?,
        server_id: row.get(9)?,
        server_html_hash: row.get(10)?,
    })
}

fn decrypt_if_service_ciphertext(
    value: &str,
    encryption_key: Option<&[u8; 32]>,
) -> Result<Option<(bool, String)>, String> {
    if !common::crypto::is_encrypted(value) {
        return Ok(None);
    }
    let key = encryption_key.ok_or_else(|| {
        "cannot hash encrypted credential before encryption key is loaded".to_string()
    })?;
    match common::crypto::decrypt_value(key, value) {
        Ok(plaintext) => Ok(Some((true, plaintext))),
        Err(_) => Ok(None),
    }
}

fn count_accounts(
    conn: &impl db::db::WriteTransactionTarget,
    account_id: Option<&str>,
) -> Result<u64, String> {
    let count = match account_id {
        Some(account_id) => conn.query_row(
            "SELECT COUNT(*) FROM accounts WHERE id = ?1",
            params![account_id],
            |row| row.get::<_, i64>(0),
        ),
        None => conn.query_row("SELECT COUNT(*) FROM accounts", [], |row| {
            row.get::<_, i64>(0)
        }),
    }
    .map_err(|e| format!("count accounts: {e}"))?;
    non_negative_count(count, "accounts")
}

fn count_account_rows(
    conn: &impl db::db::WriteTransactionTarget,
    table: &str,
    account_id: Option<&str>,
) -> Result<u64, String> {
    let count = match account_id {
        Some(account_id) => {
            let sql = format!("SELECT COUNT(*) FROM {table} WHERE account_id = ?1");
            conn.query_row(&sql, params![account_id], |row| row.get::<_, i64>(0))
        }
        None => {
            let sql = format!("SELECT COUNT(*) FROM {table}");
            conn.query_row(&sql, [], |row| row.get::<_, i64>(0))
        }
    }
    .map_err(|e| format!("count {table}: {e}"))?;
    non_negative_count(count, table)
}

fn count_unread_messages(
    conn: &impl db::db::WriteTransactionTarget,
    account_id: Option<&str>,
) -> Result<u64, String> {
    let count = match account_id {
        Some(account_id) => conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE account_id = ?1 AND is_read = 0",
            params![account_id],
            |row| row.get::<_, i64>(0),
        ),
        None => conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE is_read = 0",
            [],
            |row| row.get::<_, i64>(0),
        ),
    }
    .map_err(|e| format!("count unread messages: {e}"))?;
    non_negative_count(count, "unread messages")
}

fn read_harness_messages(
    conn: &impl db::db::WriteTransactionTarget,
    params: &TestQueryDbStateParams,
) -> Result<Vec<TestDbMessageRow>, String> {
    let limit = i64::try_from(params.message_limit.unwrap_or(20).min(200))
        .map_err(|e| format!("message_limit conversion: {e}"))?;
    match params.account_id.as_deref() {
        Some(account_id) => {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, thread_id, subject, from_address,
                            to_addresses, date, is_read, is_starred
                     FROM messages
                     WHERE account_id = ?1
                     ORDER BY date ASC, id ASC
                     LIMIT ?2",
                )
                .map_err(|e| format!("prepare messages query: {e}"))?;
            let mapped = stmt
                .query_map(params![account_id, limit], test_db_message_from_row)
                .map_err(|e| format!("query messages: {e}"))?;
            collect_rows(mapped, "messages")
        }
        None => {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, thread_id, subject, from_address,
                            to_addresses, date, is_read, is_starred
                     FROM messages
                     ORDER BY account_id ASC, date ASC, id ASC
                     LIMIT ?1",
                )
                .map_err(|e| format!("prepare messages query: {e}"))?;
            let mapped = stmt
                .query_map(params![limit], test_db_message_from_row)
                .map_err(|e| format!("query messages: {e}"))?;
            collect_rows(mapped, "messages")
        }
    }
}

fn read_harness_local_drafts(
    conn: &impl db::db::WriteTransactionTarget,
    params: &TestQueryDbStateParams,
) -> Result<Vec<TestDbLocalDraftRow>, String> {
    let limit = i64::try_from(params.message_limit.unwrap_or(20).min(200))
        .map_err(|e| format!("message_limit conversion: {e}"))?;
    match params.account_id.as_deref() {
        Some(account_id) => {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, to_addresses, cc_addresses, bcc_addresses,
                            subject, body_html, reply_to_message_id, thread_id,
                            from_email, signature_id, remote_draft_id, attachments,
                            signature_separator_index, sync_status
                     FROM local_drafts
                     WHERE account_id = ?1
                     ORDER BY updated_at ASC, id ASC
                     LIMIT ?2",
                )
                .map_err(|e| format!("prepare local drafts query: {e}"))?;
            let mapped = stmt
                .query_map(params![account_id, limit], test_db_local_draft_from_row)
                .map_err(|e| format!("query local drafts: {e}"))?;
            collect_rows(mapped, "local drafts")
        }
        None => {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, to_addresses, cc_addresses, bcc_addresses,
                            subject, body_html, reply_to_message_id, thread_id,
                            from_email, signature_id, remote_draft_id, attachments,
                            signature_separator_index, sync_status
                     FROM local_drafts
                     ORDER BY account_id ASC, updated_at ASC, id ASC
                     LIMIT ?1",
                )
                .map_err(|e| format!("prepare local drafts query: {e}"))?;
            let mapped = stmt
                .query_map(params![limit], test_db_local_draft_from_row)
                .map_err(|e| format!("query local drafts: {e}"))?;
            collect_rows(mapped, "local drafts")
        }
    }
}

fn read_harness_attachments(
    conn: &impl db::db::WriteTransactionTarget,
    params: &TestQueryDbStateParams,
) -> Result<Vec<TestDbAttachmentRow>, String> {
    let limit = i64::try_from(params.attachment_limit.unwrap_or(20).min(200))
        .map_err(|e| format!("attachment limit conversion: {e}"))?;
    match params.account_id.as_deref() {
        Some(account_id) => {
            let mut stmt = conn
                .prepare(
                    "SELECT a.id, a.account_id, a.message_id, a.filename, a.mime_type,
                            a.size, a.content_hash, a.text_indexed_at,
                            t.status, t.extracted_text
                     FROM attachments a
                     LEFT JOIN attachment_extracted_text t ON t.content_hash = a.content_hash
                     WHERE a.account_id = ?1
                     ORDER BY a.message_id ASC, a.id ASC
                     LIMIT ?2",
                )
                .map_err(|e| format!("prepare attachments query: {e}"))?;
            let mapped = stmt
                .query_map(params![account_id, limit], test_db_attachment_from_row)
                .map_err(|e| format!("query attachments: {e}"))?;
            collect_rows(mapped, "attachments")
        }
        None => {
            let mut stmt = conn
                .prepare(
                    "SELECT a.id, a.account_id, a.message_id, a.filename, a.mime_type,
                            a.size, a.content_hash, a.text_indexed_at,
                            t.status, t.extracted_text
                     FROM attachments a
                     LEFT JOIN attachment_extracted_text t ON t.content_hash = a.content_hash
                     ORDER BY a.account_id ASC, a.message_id ASC, a.id ASC
                     LIMIT ?1",
                )
                .map_err(|e| format!("prepare attachments query: {e}"))?;
            let mapped = stmt
                .query_map(params![limit], test_db_attachment_from_row)
                .map_err(|e| format!("query attachments: {e}"))?;
            collect_rows(mapped, "attachments")
        }
    }
}

fn read_harness_calendars(
    conn: &impl db::db::WriteTransactionTarget,
    params: &TestQueryDbStateParams,
) -> Result<Vec<TestDbCalendarRow>, String> {
    let limit = i64::try_from(params.calendar_limit.unwrap_or(20).min(200))
        .map_err(|e| format!("calendar limit conversion: {e}"))?;
    match params.account_id.as_deref() {
        Some(account_id) => {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, provider, remote_id, display_name,
                            color, is_primary, is_visible, is_default,
                            provider_id, can_edit
                     FROM calendars
                     WHERE account_id = ?1
                     ORDER BY sort_order ASC, display_name ASC, id ASC
                     LIMIT ?2",
                )
                .map_err(|e| format!("prepare calendars query: {e}"))?;
            let mapped = stmt
                .query_map(params![account_id, limit], test_db_calendar_from_row)
                .map_err(|e| format!("query calendars: {e}"))?;
            collect_rows(mapped, "calendars")
        }
        None => {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, provider, remote_id, display_name,
                            color, is_primary, is_visible, is_default,
                            provider_id, can_edit
                     FROM calendars
                     ORDER BY account_id ASC, sort_order ASC, display_name ASC, id ASC
                     LIMIT ?1",
                )
                .map_err(|e| format!("prepare calendars query: {e}"))?;
            let mapped = stmt
                .query_map(params![limit], test_db_calendar_from_row)
                .map_err(|e| format!("query calendars: {e}"))?;
            collect_rows(mapped, "calendars")
        }
    }
}

fn read_harness_calendar_events(
    conn: &impl db::db::WriteTransactionTarget,
    params: &TestQueryDbStateParams,
) -> Result<Vec<TestDbCalendarEventRow>, String> {
    let limit = i64::try_from(params.calendar_limit.unwrap_or(20).min(200))
        .map_err(|e| format!("calendar limit conversion: {e}"))?;
    match params.account_id.as_deref() {
        Some(account_id) => {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, calendar_id, google_event_id,
                            remote_event_id, summary, title, description,
                            location, start_time, end_time, is_all_day, status,
                            organizer_email, organizer_name, attendees_json,
                            recurrence_rule
                     FROM calendar_events
                     WHERE account_id = ?1
                     ORDER BY start_time ASC, id ASC
                     LIMIT ?2",
                )
                .map_err(|e| format!("prepare calendar events query: {e}"))?;
            let mapped = stmt
                .query_map(params![account_id, limit], test_db_calendar_event_from_row)
                .map_err(|e| format!("query calendar events: {e}"))?;
            collect_rows(mapped, "calendar events")
        }
        None => {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, calendar_id, google_event_id,
                            remote_event_id, summary, title, description,
                            location, start_time, end_time, is_all_day, status,
                            organizer_email, organizer_name, attendees_json,
                            recurrence_rule
                     FROM calendar_events
                     ORDER BY account_id ASC, start_time ASC, id ASC
                     LIMIT ?1",
                )
                .map_err(|e| format!("prepare calendar events query: {e}"))?;
            let mapped = stmt
                .query_map(params![limit], test_db_calendar_event_from_row)
                .map_err(|e| format!("query calendar events: {e}"))?;
            collect_rows(mapped, "calendar events")
        }
    }
}

fn read_harness_contacts(
    conn: &impl db::db::WriteTransactionTarget,
    params: &TestQueryDbStateParams,
) -> Result<Vec<TestDbContactRow>, String> {
    let limit = i64::try_from(params.contact_limit.unwrap_or(20).min(200))
        .map_err(|e| format!("contact limit conversion: {e}"))?;
    match params.account_id.as_deref() {
        Some(account_id) => {
            let mut stmt = conn
                .prepare(
                    "SELECT id, email, display_name, source, account_id,
                            server_id, email2, phone, company, notes,
                            display_name_overridden
                     FROM contacts
                     WHERE account_id = ?1
                     ORDER BY email ASC, id ASC
                     LIMIT ?2",
                )
                .map_err(|e| format!("prepare contacts query: {e}"))?;
            let mapped = stmt
                .query_map(params![account_id, limit], test_db_contact_from_row)
                .map_err(|e| format!("query contacts: {e}"))?;
            collect_rows(mapped, "contacts")
        }
        None => {
            let mut stmt = conn
                .prepare(
                    "SELECT id, email, display_name, source, account_id,
                            server_id, email2, phone, company, notes,
                            display_name_overridden
                     FROM contacts
                     ORDER BY account_id ASC, email ASC, id ASC
                     LIMIT ?1",
                )
                .map_err(|e| format!("prepare contacts query: {e}"))?;
            let mapped = stmt
                .query_map(params![limit], test_db_contact_from_row)
                .map_err(|e| format!("query contacts: {e}"))?;
            collect_rows(mapped, "contacts")
        }
    }
}

fn read_harness_contact_groups(
    conn: &impl db::db::WriteTransactionTarget,
    params: &TestQueryDbStateParams,
) -> Result<Vec<TestDbContactGroupRow>, String> {
    let limit = i64::try_from(params.contact_group_limit.unwrap_or(20).min(200))
        .map_err(|e| format!("contact group limit conversion: {e}"))?;
    match params.account_id.as_deref() {
        Some(account_id) => {
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, source, account_id, server_id, email, group_type
                     FROM contact_groups
                     WHERE account_id = ?1
                     ORDER BY name ASC, id ASC
                     LIMIT ?2",
                )
                .map_err(|e| format!("prepare contact groups query: {e}"))?;
            let mapped = stmt
                .query_map(params![account_id, limit], test_db_contact_group_from_row)
                .map_err(|e| format!("query contact groups: {e}"))?;
            collect_rows(mapped, "contact groups")
        }
        None => {
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, source, account_id, server_id, email, group_type
                     FROM contact_groups
                     ORDER BY account_id ASC, name ASC, id ASC
                     LIMIT ?1",
                )
                .map_err(|e| format!("prepare contact groups query: {e}"))?;
            let mapped = stmt
                .query_map(params![limit], test_db_contact_group_from_row)
                .map_err(|e| format!("query contact groups: {e}"))?;
            collect_rows(mapped, "contact groups")
        }
    }
}

fn test_db_message_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TestDbMessageRow> {
    Ok(TestDbMessageRow {
        id: row.get(0)?,
        account_id: row.get(1)?,
        thread_id: row.get(2)?,
        subject: row.get(3)?,
        from_address: row.get(4)?,
        to_addresses: row.get(5)?,
        date: row.get(6)?,
        is_read: row.get::<_, i64>(7)? != 0,
        is_starred: row.get::<_, i64>(8)? != 0,
    })
}

fn test_db_local_draft_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TestDbLocalDraftRow> {
    Ok(TestDbLocalDraftRow {
        id: row.get(0)?,
        account_id: row.get(1)?,
        to_addresses: row.get(2)?,
        cc_addresses: row.get(3)?,
        bcc_addresses: row.get(4)?,
        subject: row.get(5)?,
        body_html: row.get(6)?,
        reply_to_message_id: row.get(7)?,
        thread_id: row.get(8)?,
        from_email: row.get(9)?,
        signature_id: row.get(10)?,
        remote_draft_id: row.get(11)?,
        attachments: row.get(12)?,
        signature_separator_index: row.get(13)?,
        sync_status: row.get(14)?,
    })
}

fn test_db_attachment_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TestDbAttachmentRow> {
    Ok(TestDbAttachmentRow {
        id: row.get(0)?,
        account_id: row.get(1)?,
        message_id: row.get(2)?,
        filename: row.get(3)?,
        mime_type: row.get(4)?,
        size: row.get(5)?,
        content_hash: row
            .get::<_, Option<db::blob_hash::BlobHash>>(6)?
            .map(|h| h.to_hex()),
        text_indexed_at: row.get(7)?,
        extraction_status: row.get(8)?,
        extracted_text: row.get(9)?,
    })
}

fn test_db_calendar_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TestDbCalendarRow> {
    Ok(TestDbCalendarRow {
        id: row.get(0)?,
        account_id: row.get(1)?,
        provider: row.get(2)?,
        remote_id: row.get(3)?,
        display_name: row.get(4)?,
        color: row.get(5)?,
        is_primary: row.get::<_, i64>(6)? != 0,
        is_visible: row.get::<_, i64>(7)? != 0,
        is_default: row.get::<_, i64>(8)? != 0,
        provider_id: row.get(9)?,
        can_edit: row.get::<_, i64>(10)? != 0,
    })
}

fn test_db_calendar_event_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<TestDbCalendarEventRow> {
    Ok(TestDbCalendarEventRow {
        id: row.get(0)?,
        account_id: row.get(1)?,
        calendar_id: row.get(2)?,
        google_event_id: row.get(3)?,
        remote_event_id: row.get(4)?,
        summary: row.get(5)?,
        title: row.get(6)?,
        description: row.get(7)?,
        location: row.get(8)?,
        start_time: row.get(9)?,
        end_time: row.get(10)?,
        is_all_day: row.get::<_, i64>(11)? != 0,
        status: row.get(12)?,
        organizer_email: row.get(13)?,
        organizer_name: row.get(14)?,
        attendees_json: row.get(15)?,
        recurrence_rule: row.get(16)?,
    })
}

fn test_db_contact_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TestDbContactRow> {
    Ok(TestDbContactRow {
        id: row.get(0)?,
        email: row.get(1)?,
        display_name: row.get(2)?,
        source: row.get(3)?,
        account_id: row.get(4)?,
        server_id: row.get(5)?,
        email2: row.get(6)?,
        phone: row.get(7)?,
        company: row.get(8)?,
        notes: row.get(9)?,
        display_name_overridden: row.get::<_, i64>(10)? != 0,
    })
}

fn test_db_contact_group_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<TestDbContactGroupRow> {
    Ok(TestDbContactGroupRow {
        id: row.get(0)?,
        name: row.get(1)?,
        source: row.get(2)?,
        account_id: row.get(3)?,
        server_id: row.get(4)?,
        email: row.get(5)?,
        group_type: row.get(6)?,
    })
}

fn collect_rows<T, F>(rows: rusqlite::MappedRows<'_, F>, label: &str) -> Result<Vec<T>, String>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("read {label}: {e}"))?);
    }
    Ok(out)
}

fn non_negative_count(value: i64, label: &str) -> Result<u64, String> {
    u64::try_from(value).map_err(|e| format!("{label} count was negative: {e}"))
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn add_changed_rows(current: u64, changed: usize) -> Result<u64, String> {
    let changed = u64::try_from(changed).map_err(|e| e.to_string())?;
    current
        .checked_add(changed)
        .ok_or_else(|| "label insert count overflow".to_string())
}
