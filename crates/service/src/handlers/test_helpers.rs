//! Handlers for the `test-helpers` feature. Compiled out of release builds.
//!
//! Each handler maps to a `RequestParams::Test*` variant defined in
//! `service-api` under the same feature flag. They exist exclusively to give
//! integration tests a deterministic way to drive panic-safety, version-
//! mismatch, in-flight-cap, and stdio-corruption behaviors.

use crate::boot::BootSharedState;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use service_api::{
    HealthPingResponse, ServiceError, TestCounterReadAck, TestCrashAfterNWritesAck,
    TestCrashAfterNWritesParams, TestDbAccountRow, TestDbAttachmentRow,
    TestDbCalendarEventRow, TestDbCalendarRow, TestDbContactGroupRow, TestDbContactRow,
    TestDbLocalDraftRow, TestDbMessageRow, TestDelayNextWriteAck, TestDelayNextWriteParams,
    TestPendingOpRow, TestPendingOpsReadAck, TestPendingOpsReadParams, TestQueryDbStateAck,
    TestQueryDbStateParams, TestRemoveCachedAttachmentBytesAck,
    TestRemoveCachedAttachmentBytesParams, TestSeedAccountAck, TestSeedAccountParams,
    TestSearchIndexAck, TestSearchIndexParams, TestSearchIndexResult,
    TestSeedCachedAttachmentAck, TestSeedCachedAttachmentParams, TestSeedThreadAck,
    TestSeedThreadParams, TestStartSyncParams, TestThreadReadAck, TestThreadReadParams,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

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
        access_token: params
            .access_token
            .or_else(|| uses_oauth.then(|| "test-access-token".into())),
        refresh_token: params
            .refresh_token
            .or_else(|| uses_oauth.then(|| "test-refresh-token".into())),
        token_expires_at: params
            .token_expires_at
            .or_else(|| uses_oauth.then(|| chrono::Utc::now().timestamp() + 3_600)),
        oauth_provider,
        oauth_client_id: params
            .oauth_client_id
            .or_else(|| uses_oauth.then(|| "test-client-id".into())),
        oauth_token_url: params.oauth_token_url,
        imap_host: Some("imap.example.test".into()),
        imap_port: Some(993),
        imap_security: Some("tls".into()),
        imap_username: Some(email.clone()),
        imap_password: Some("test-password".into()),
        smtp_host: Some("smtp.example.test".into()),
        smtp_port: Some(587),
        smtp_security: Some("starttls".into()),
        smtp_username: Some(email.clone()),
        smtp_password: Some("test-password".into()),
        jmap_url: Some("https://jmap.example.test".into()),
        accept_invalid_certs: true,
    };
    let ack_email = email.clone();
    let (account_id, label_count) = write_db
        .with_conn(move |conn| {
            let account_id =
                db::db::queries_extra::create_account_sync(conn, &create_params)?;
            if caldav_url.is_some() || caldav_username.is_some() || caldav_password.is_some() {
                conn.execute(
                    "UPDATE accounts
                     SET caldav_url = ?1,
                         caldav_username = ?2,
                         caldav_password = ?3
                     WHERE id = ?4",
                    params![
                        caldav_url,
                        caldav_username,
                        caldav_password,
                        &account_id
                    ],
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

pub(super) async fn counter_read_handle(counter: String) -> Result<Value, ServiceError> {
    let value = crate::test_counters::read(&counter).ok_or_else(|| {
        ServiceError::InvalidParams {
            method: "test.counter_read".into(),
            message: format!("unknown counter {counter:?}"),
        }
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
        .with_conn(move |conn| insert_harness_thread(conn, params, &thread_id, &message_id))
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
    let app_data_dir = boot_state.app_data_dir().to_path_buf();
    let attachment_id = params
        .attachment_id
        .clone()
        .unwrap_or_else(|| format!("attachment-{}", uuid::Uuid::new_v4()));
    let filename = params.filename.unwrap_or_else(|| "harness.txt".into());
    let mime_type = params.mime_type.unwrap_or_else(|| "text/plain".into());
    let bytes = params.content.into_bytes();
    let content_hash = store::attachment_cache::hash_bytes(&bytes);
    let relative_path =
        store::attachment_cache::write_cached(&app_data_dir, &content_hash, &bytes)
            .map_err(ServiceError::Internal)?;
    let size_bytes = u64::try_from(bytes.len())
        .map_err(|e| ServiceError::Internal(format!("attachment size conversion: {e}")))?;
    let cache_size = i64::try_from(bytes.len())
        .map_err(|e| ServiceError::Internal(format!("attachment cache size conversion: {e}")))?;
    let account_id = params.account_id.clone();
    let message_id = params.message_id.clone();
    let ack = TestSeedCachedAttachmentAck {
        account_id: account_id.clone(),
        message_id: message_id.clone(),
        attachment_id: attachment_id.clone(),
        content_hash: content_hash.clone(),
        relative_path: relative_path.clone(),
        size_bytes,
    };
    write_db
        .with_conn(move |conn| {
            insert_harness_cached_attachment(
                conn,
                &CachedAttachmentInsert {
                    account_id: &account_id,
                    message_id: &message_id,
                    attachment_id: &attachment_id,
                    filename: &filename,
                    mime_type: &mime_type,
                    relative_path: &relative_path,
                    cache_size,
                    content_hash: &content_hash,
                },
            )
        })
        .await
        .map_err(ServiceError::Internal)?;
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
    let app_data_dir = boot_state.app_data_dir().to_path_buf();
    let full_path = app_data_dir.join(&params.relative_path);
    let removed = full_path.is_file();
    if removed {
        store::attachment_cache::remove_cached_relative(&app_data_dir, &params.relative_path)
            .map_err(ServiceError::Internal)?;
    }
    serde_json::to_value(TestRemoveCachedAttachmentBytesAck {
        relative_path: params.relative_path,
        removed,
    })
    .map_err(|error| ServiceError::Internal(error.to_string()))
}

pub(super) async fn thread_read_handle(
    boot_state: &Arc<BootSharedState>,
    params: TestThreadReadParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    let ack = write_db
        .with_conn(move |conn| read_harness_thread(conn, &params))
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
        .with_conn(move |conn| read_harness_pending_ops(conn, &params))
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
        ServiceError::Internal(
            "test.start_sync received before SyncRuntime was installed".into(),
        )
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
        .with_conn(move |conn| read_harness_db_state(conn, &params, encryption_key))
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
    if let Some(search_write) = boot_state
        .search_write()
        .or_else(|| boot_state.sync_runtime().map(|runtime| runtime.search_write()))
    {
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
        .with_conn(move |conn| {
            let fragments = db::db::queries_extra::select_attachment_fragments_batch(conn, &pairs)?;
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
                                attachment_id:  row.attachment_id.clone(),
                                filename:       row.filename.clone(),
                                mime:           row.mime_type.clone(),
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

fn insert_harness_account_rows(
    conn: &Connection,
    account_id: &str,
    email: &str,
) -> Result<u64, String> {
    let mut label_count = 0_u64;
    for (sort_order, role) in db::db::folder_roles::SYSTEM_FOLDER_ROLES.iter().enumerate() {
        let changed = conn
            .execute(
                "INSERT INTO labels (
                    id, account_id, name, type, visible, sort_order,
                    imap_folder_path, imap_special_use, label_kind
                ) VALUES (?1, ?2, ?3, 'system', 1, ?4, ?3, ?5, 'container')
                ON CONFLICT(account_id, id) DO NOTHING",
                params![
                    role.label_id,
                    account_id,
                    role.label_name,
                    i64::try_from(sort_order).map_err(|e| e.to_string())?,
                    role.imap_special_use,
                ],
            )
            .map_err(|e| format!("insert system label {}: {e}", role.label_id))?;
        label_count = add_changed_rows(label_count, changed)?;
    }

    let changed = conn
        .execute(
            "INSERT INTO labels (
                id, account_id, name, type, visible, sort_order, label_kind
            ) VALUES ('harness-label', ?1, 'Harness', 'user', 1, 1000, 'tag')
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
    conn: &Connection,
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
    let tx = conn.unchecked_transaction().map_err(|e| format!("begin: {e}"))?;

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
    for label_id in label_ids {
        tx.execute(
            "INSERT OR IGNORE INTO thread_labels (account_id, thread_id, label_id)
             VALUES (?1, ?2, ?3)",
            params![params.account_id, thread_id, label_id],
        )
        .map_err(|e| format!("insert thread label: {e}"))?;
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
    relative_path: &'a str,
    cache_size: i64,
    content_hash: &'a str,
}

fn insert_harness_cached_attachment(
    conn: &Connection,
    insert: &CachedAttachmentInsert<'_>,
) -> Result<(), String> {
    let tx = conn.unchecked_transaction().map_err(|e| format!("begin: {e}"))?;
    // Harness fixtures write raw bytes straight into the flat cache,
    // so size and cache_size intentionally match for seeded rows.
    tx.execute(
        "INSERT INTO attachments (
            id, message_id, account_id, filename, mime_type, size,
            local_path, cached_at, cache_size, content_hash, text_indexed_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, unixepoch(), ?8, ?9, NULL)
        ON CONFLICT(id) DO UPDATE SET
            message_id = excluded.message_id,
            account_id = excluded.account_id,
            filename = excluded.filename,
            mime_type = excluded.mime_type,
            size = excluded.size,
            local_path = excluded.local_path,
            cached_at = unixepoch(),
            cache_size = excluded.cache_size,
            content_hash = excluded.content_hash,
            text_indexed_at = NULL",
        params![
            insert.attachment_id,
            insert.message_id,
            insert.account_id,
            insert.filename,
            insert.mime_type,
            insert.cache_size,
            insert.relative_path,
            insert.cache_size,
            insert.content_hash,
        ],
    )
    .map_err(|e| format!("insert cached attachment: {e}"))?;
    tx.commit().map_err(|e| format!("commit: {e}"))
}

fn read_harness_thread(
    conn: &Connection,
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
             ORDER BY label_id",
        )
        .map_err(|e| format!("prepare labels: {e}"))?;
    let label_ids = stmt
        .query_map(params![params.account_id, params.thread_id], |row| row.get(0))
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
    conn: &Connection,
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
    let failed = u64::try_from(
        operations
            .iter()
            .filter(|op| op.status == "failed")
            .count(),
    )
    .map_err(|e| e.to_string())?;

    Ok(TestPendingOpsReadAck {
        total,
        pending,
        failed,
        operations,
    })
}

fn read_harness_db_state(
    conn: &Connection,
    params: &TestQueryDbStateParams,
    encryption_key: Option<[u8; 32]>,
) -> Result<TestQueryDbStateAck, String> {
    let account_id = params.account_id.as_deref();
    Ok(TestQueryDbStateAck {
        account_count: count_accounts(conn, account_id)?,
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
    conn: &Connection,
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
    let (encrypted, plaintext) =
        decrypt_if_service_ciphertext(value, encryption_key)?.unwrap_or_else(|| {
            (false, value.to_string())
        });
    let mut hasher = Sha256::new();
    hasher.update(plaintext.as_bytes());
    Ok(CredentialSummary {
        present: true,
        encrypted,
        sha256: Some(hex_bytes(&hasher.finalize())),
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

fn count_accounts(conn: &Connection, account_id: Option<&str>) -> Result<u64, String> {
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
    conn: &Connection,
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
    conn: &Connection,
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
    conn: &Connection,
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
    conn: &Connection,
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
    conn: &Connection,
    params: &TestQueryDbStateParams,
) -> Result<Vec<TestDbAttachmentRow>, String> {
    let limit = i64::try_from(params.attachment_limit.unwrap_or(20).min(200))
        .map_err(|e| format!("attachment limit conversion: {e}"))?;
    match params.account_id.as_deref() {
        Some(account_id) => {
            let mut stmt = conn
                .prepare(
                    "SELECT a.id, a.account_id, a.message_id, a.filename, a.mime_type,
                            a.size, a.local_path, a.cached_at, a.cache_size,
                            a.content_hash, a.text_indexed_at, t.status, t.extracted_text
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
                            a.size, a.local_path, a.cached_at, a.cache_size,
                            a.content_hash, a.text_indexed_at, t.status, t.extracted_text
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
    conn: &Connection,
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
    conn: &Connection,
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
                            organizer_email, organizer_name, attendees_json
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
                            organizer_email, organizer_name, attendees_json
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
    conn: &Connection,
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
    conn: &Connection,
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

fn test_db_local_draft_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<TestDbLocalDraftRow> {
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

fn test_db_attachment_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<TestDbAttachmentRow> {
    Ok(TestDbAttachmentRow {
        id: row.get(0)?,
        account_id: row.get(1)?,
        message_id: row.get(2)?,
        filename: row.get(3)?,
        mime_type: row.get(4)?,
        size: row.get(5)?,
        local_path: row.get(6)?,
        cached_at: row.get(7)?,
        cache_size: row.get(8)?,
        content_hash: row.get(9)?,
        text_indexed_at: row.get(10)?,
        extraction_status: row.get(11)?,
        extracted_text: row.get(12)?,
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
