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
    TestCrashAfterNWritesParams, TestDelayNextWriteAck, TestDelayNextWriteParams,
    TestPendingOpRow, TestPendingOpsReadAck, TestPendingOpsReadParams, TestSeedAccountAck,
    TestSeedAccountParams, TestSeedThreadAck, TestSeedThreadParams, TestThreadReadAck,
    TestThreadReadParams,
};
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
    let create_params = db::db::queries_extra::CreateAccountParams {
        email: email.clone(),
        provider: params.provider.unwrap_or_else(|| "imap".into()),
        display_name: params.display_name.or_else(|| Some("Harness".into())),
        account_name: params
            .account_name
            .unwrap_or_else(|| "Harness Account".into()),
        account_color: "#4285f4".into(),
        auth_method: "password".into(),
        access_token: None,
        refresh_token: None,
        token_expires_at: None,
        oauth_provider: None,
        oauth_client_id: None,
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
        jmap_url: None,
        accept_invalid_certs: true,
    };
    let ack_email = email.clone();
    let (account_id, label_count) = write_db
        .with_conn(move |conn| {
            let account_id =
                db::db::queries_extra::create_account_sync(conn, &create_params)?;
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
    write_db
        .with_conn(move |conn| insert_harness_thread(conn, params, &thread_id, &message_id))
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(ack).map_err(|error| ServiceError::Internal(error.to_string()))
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

fn add_changed_rows(current: u64, changed: usize) -> Result<u64, String> {
    let changed = u64::try_from(changed).map_err(|e| e.to_string())?;
    current
        .checked_add(changed)
        .ok_or_else(|| "label insert count overflow".to_string())
}
