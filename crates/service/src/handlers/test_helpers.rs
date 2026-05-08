//! Handlers for the `test-helpers` feature. Compiled out of release builds.
//!
//! Each handler maps to a `RequestParams::Test*` variant defined in
//! `service-api` under the same feature flag. They exist exclusively to give
//! integration tests a deterministic way to drive panic-safety, version-
//! mismatch, in-flight-cap, and stdio-corruption behaviors.

use crate::boot::BootSharedState;
use rusqlite::{Connection, params};
use serde_json::Value;
use service_api::{
    HealthPingResponse, ServiceError, TestCounterReadAck, TestCrashAfterNWritesAck,
    TestCrashAfterNWritesParams, TestSeedAccountAck, TestSeedAccountParams,
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

fn add_changed_rows(current: u64, changed: usize) -> Result<u64, String> {
    let changed = u64::try_from(changed).map_err(|e| e.to_string())?;
    current
        .checked_add(changed)
        .ok_or_else(|| "label insert count overflow".to_string())
}
