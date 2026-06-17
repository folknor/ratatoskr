//! Internal Service IPCs that mediate the encryption-key boundary.
//!
//! Three methods land together so a half-migrated UI cannot reach a
//! state where it can write a blob it cannot read or boot without
//! decrypting its own settings:
//!
//! - `internal.read_bootstrap_snapshots` - cold-boot read. Service
//!   runs `get_ui_bootstrap_snapshot` and `get_settings_bootstrap_snapshot`
//!   with the in-memory key and returns the already-decrypted structs.
//!   One round-trip per cold boot, hides the encryption boundary
//!   entirely. The UI never sees an encrypted byte for the bootstrap
//!   path, so the IPC is not a "general decryption oracle" - it
//!   exposes only the two bounded snapshot shapes.
//! - `internal.encrypt_for_storage` - one-shot encrypt for credential
//!   persistence. Returns the existing `iv:ciphertext_with_tag` string
//!   shape that `crypto::encrypt_value` produces.
//! - `internal.decrypt_for_storage` - one-shot decrypt for the rare
//!   re-auth wizard pre-fill. Returns the plaintext bytes; sensitive
//!   values are wrapped in `RedactedString` so a stray `Debug` print
//!   cannot leak them.
//!
//! `read_bootstrap_snapshots` is part of the cold-boot critical path
//! (the UI cannot apply persisted preferences until it lands), so its
//! request timeout is widened to 10 s to absorb a cold-disk read +
//! any AES key-stretch contention.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::redacted::RedactedString;

/// `internal.read_bootstrap_snapshots` request body.
///
/// No params today; an unconditional cold-boot read. A future filter
/// (e.g. a `keys: Vec<String>` to scope which secure settings the
/// caller cares about) would extend this struct without breaking the
/// wire shape.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadBootstrapSnapshotsParams {}

/// `internal.read_bootstrap_snapshots` ack.
///
/// Both `ui` and `settings` carry the JSON shape of
/// `rtsk::db::queries::UiBootstrapSnapshot` and
/// `rtsk::db::queries::SettingsBootstrapSnapshot` respectively. The
/// wire type stores them as `serde_json::Value` because `service-api`
/// does not depend on `rtsk` (and pulling the snapshot structs down
/// into the wire layer would invert the dependency graph). UI-side,
/// `serde_json::from_value::<UiBootstrapSnapshot>` recovers the typed
/// struct.
///
/// `ui_error` / `settings_error` carry per-field error reports for
/// the partial-failure tolerance the plan codifies: an unparseable
/// secure setting must not block the rest of the bootstrap. Today
/// the helpers fall back to the raw value silently; the errors slot
/// is reserved so the helpers can grow per-field error reporting
/// without a wire-shape change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadBootstrapSnapshotsAck {
    pub ui: Value,
    pub settings: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// `internal.encrypt_for_storage` request body.
///
/// `plaintext` carried as `RedactedString` so the wire-debug log
/// surface cannot leak account passwords / API keys via a stray
/// `Debug` print. Service-side handler unwraps and feeds into
/// `crypto::encrypt_value`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptForStorageParams {
    pub plaintext: RedactedString,
}

/// `internal.encrypt_for_storage` ack.
///
/// `ciphertext` is the `iv:ciphertext_with_tag` string format that
/// `crypto::encrypt_value` produces. The shape is wire-stable - the
/// existing on-disk format already uses this string layout, so the
/// IPC introduces no new encoding contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptForStorageAck {
    pub ciphertext: String,
}

/// `internal.decrypt_for_storage` request body.
///
/// `ciphertext` is the `iv:ciphertext_with_tag` string the encrypt
/// path produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecryptForStorageParams {
    pub ciphertext: String,
}

/// `internal.decrypt_for_storage` ack.
///
/// Returned `plaintext` is wrapped in `RedactedString` for the same
/// reason `encrypt_for_storage` takes one - the value is sensitive
/// and must not bleed into wire-debug logs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecryptForStorageAck {
    pub plaintext: RedactedString,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_bootstrap_snapshots_params_round_trips() {
        let params = ReadBootstrapSnapshotsParams::default();
        let json = serde_json::to_value(&params).expect("serialize");
        let recovered: ReadBootstrapSnapshotsParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(params, recovered);
    }

    #[test]
    fn read_bootstrap_snapshots_ack_round_trips() {
        let ack = ReadBootstrapSnapshotsAck {
            ui: serde_json::json!({ "active_account_id": "acct-1", "show_sync_status": true }),
            settings: serde_json::json!({ "block_remote_images": true }),
            warnings: vec!["one stale secure setting skipped".to_string()],
        };
        let json = serde_json::to_value(&ack).expect("serialize");
        let recovered: ReadBootstrapSnapshotsAck =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(ack, recovered);
    }

    #[test]
    fn read_bootstrap_snapshots_ack_skips_empty_warnings() {
        let ack = ReadBootstrapSnapshotsAck {
            ui: serde_json::json!({}),
            settings: serde_json::json!({}),
            warnings: Vec::new(),
        };
        let json = serde_json::to_value(&ack).expect("serialize");
        let object = json.as_object().expect("object");
        assert!(
            !object.contains_key("warnings"),
            "empty warnings must not serialize: got {object:?}",
        );
    }

    #[test]
    fn encrypt_for_storage_params_round_trips() {
        let params = EncryptForStorageParams {
            plaintext: RedactedString::new("hunter2"),
        };
        let json = serde_json::to_value(&params).expect("serialize");
        let recovered: EncryptForStorageParams = serde_json::from_value(json).expect("deserialize");
        assert_eq!(params, recovered);
    }

    #[test]
    fn encrypt_for_storage_ack_round_trips() {
        let ack = EncryptForStorageAck {
            ciphertext: "AAAA:BBBB".to_string(),
        };
        let json = serde_json::to_value(&ack).expect("serialize");
        let recovered: EncryptForStorageAck = serde_json::from_value(json).expect("deserialize");
        assert_eq!(ack, recovered);
    }

    #[test]
    fn decrypt_for_storage_params_round_trips() {
        let params = DecryptForStorageParams {
            ciphertext: "AAAA:BBBB".to_string(),
        };
        let json = serde_json::to_value(&params).expect("serialize");
        let recovered: DecryptForStorageParams = serde_json::from_value(json).expect("deserialize");
        assert_eq!(params, recovered);
    }

    #[test]
    fn decrypt_for_storage_ack_round_trips() {
        let ack = DecryptForStorageAck {
            plaintext: RedactedString::new("hunter2"),
        };
        let json = serde_json::to_value(&ack).expect("serialize");
        let recovered: DecryptForStorageAck = serde_json::from_value(json).expect("deserialize");
        assert_eq!(ack, recovered);
    }

    #[test]
    fn redacted_plaintext_does_not_leak_in_debug() {
        let params = EncryptForStorageParams {
            plaintext: RedactedString::new("supersecret"),
        };
        let formatted = format!("{params:?}");
        assert!(
            !formatted.contains("supersecret"),
            "Debug must not leak plaintext: {formatted}"
        );
    }
}
