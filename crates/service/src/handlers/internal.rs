//! Internal Service IPCs that mediate the encryption-key boundary.
//!
//! The encryption-key handle is the boundary that closes Phase 2
//! carry-forward 19d. After Phase 6a-part-2 the UI no longer reads
//! `ratatoskr.key` from disk for the bootstrap-snapshot path and no
//! longer calls the snapshot-decrypt helpers locally. Cold-boot
//! bootstrap data flows through `read_bootstrap_snapshots` (one IPC,
//! both `UiBootstrapSnapshot` and `SettingsBootstrapSnapshot`
//! returned already-decrypted). Credential persistence flows through
//! `encrypt_for_storage`. Re-auth pre-fill flows through
//! `decrypt_for_storage`. All three IPCs land in the same commit -
//! splitting them lets a half-migrated UI either write a blob it
//! cannot read or boot without decrypting its settings.
//!
//! The handler reads the encryption key from `BootSharedState` (the
//! same in-memory copy boot loaded and validated in
//! `BootPhase::LoadingKey`); the UI never sees a raw key byte.
//!
//! `read_bootstrap_snapshots` is on the cold-boot critical path. The
//! handler runs both helpers under one `with_conn` so the snapshot
//! pair commits atomically against a single connection lock - if the
//! key fails to decrypt one secure setting, the typed snapshot still
//! returns the rest (today's helpers fall back to the raw value
//! silently; future per-field error reporting goes in `warnings`).

use std::sync::Arc;

use serde_json::Value;
use service_api::{
    DecryptForStorageAck, DecryptForStorageParams, EncryptForStorageAck, EncryptForStorageParams,
    ReadBootstrapSnapshotsAck, ReadBootstrapSnapshotsParams, RedactedString, ServiceError,
};

use crate::boot::BootSharedState;

fn key_or_internal_error(boot_state: &BootSharedState) -> Result<[u8; 32], ServiceError> {
    boot_state.encryption_key().ok_or_else(|| {
        ServiceError::Internal(
            "encryption key not loaded; UI must wait for boot.ready before \
             calling internal.* methods"
                .into(),
        )
    })
}

pub(crate) async fn handle_read_bootstrap_snapshots(
    boot_state: &Arc<BootSharedState>,
    _params: ReadBootstrapSnapshotsParams,
) -> Result<Value, ServiceError> {
    let key = key_or_internal_error(boot_state)?;
    let write_db = boot_state.write_db_state()?;
    let (ui_value, settings_value) = write_db
        .with_conn(move |conn| {
            let ui = rtsk::db::queries::get_ui_bootstrap_snapshot(conn, &key)?;
            let settings = rtsk::db::queries::get_settings_bootstrap_snapshot(conn, &key)?;
            let ui_value = serde_json::to_value(&ui)
                .map_err(|e| format!("serialize ui snapshot: {e}"))?;
            let settings_value = serde_json::to_value(&settings)
                .map_err(|e| format!("serialize settings snapshot: {e}"))?;
            Ok((ui_value, settings_value))
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(ReadBootstrapSnapshotsAck {
        ui: ui_value,
        settings: settings_value,
        warnings: Vec::new(),
    })
    .map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_encrypt_for_storage(
    boot_state: &Arc<BootSharedState>,
    params: EncryptForStorageParams,
) -> Result<Value, ServiceError> {
    let key = key_or_internal_error(boot_state)?;
    let plaintext = params.plaintext.into_inner();
    // AES-GCM encrypt is in-memory CPU work; keep it off the
    // dispatch loop's executor by running on the blocking pool.
    let ciphertext = tokio::task::spawn_blocking(move || {
        common::crypto::encrypt_value(&key, &plaintext)
    })
    .await
    .map_err(|e| ServiceError::Internal(format!("spawn_blocking encrypt: {e}")))?
    .map_err(ServiceError::Internal)?;
    serde_json::to_value(EncryptForStorageAck { ciphertext })
        .map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_decrypt_for_storage(
    boot_state: &Arc<BootSharedState>,
    params: DecryptForStorageParams,
) -> Result<Value, ServiceError> {
    let key = key_or_internal_error(boot_state)?;
    let ciphertext = params.ciphertext;
    let plaintext = tokio::task::spawn_blocking(move || {
        common::crypto::decrypt_value(&key, &ciphertext)
    })
    .await
    .map_err(|e| ServiceError::Internal(format!("spawn_blocking decrypt: {e}")))?
    .map_err(ServiceError::Internal)?;
    serde_json::to_value(DecryptForStorageAck {
        plaintext: RedactedString::new(plaintext),
    })
    .map_err(|e| ServiceError::Internal(e.to_string()))
}
