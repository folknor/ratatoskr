//! Encryption-key load for the Service boot sequence.
//!
//! This is a deliberate micro-duplication of the `load_encryption_key`
//! function in `crates/common/src/crypto.rs`. Depending on `common` from
//! `service` would pull in `db`, `store`, `search`, `seen`, and `types` as
//! transitive deps, none of which the Service needs for the key-load step.
//!
//! The key file format is stable (base64-encoded 32 bytes; tries
//! `ratatoskr.key` first, falls back to legacy `velo.key`); divergence risk
//! between the two implementations is small. Phase 6 or later may deduplicate
//! by extracting a tiny `crypto-key` crate; Phase 1.5 keeps `service`'s dep
//! tree lean instead.

use base64::{Engine, engine::general_purpose::STANDARD};
use std::path::Path;

pub(crate) fn load_encryption_key(app_data_dir: &Path) -> Result<[u8; 32], String> {
    let key_path = app_data_dir.join("ratatoskr.key");
    let legacy_path = app_data_dir.join("velo.key");

    let path = if key_path.exists() {
        key_path
    } else if legacy_path.exists() {
        log::debug!("using legacy key file velo.key");
        legacy_path
    } else {
        return Err("no encryption key file found (ratatoskr.key)".to_string());
    };

    let contents =
        std::fs::read_to_string(&path).map_err(|error| format!("failed to read key file: {error}"))?;

    // Retroactively fix permissions on existing key files. Same policy the
    // UI side applies in `common::crypto::load_encryption_key`.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) =
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        {
            log::warn!(
                "failed to set key permissions on {}: {error}",
                path.display()
            );
        }
    }

    let key_bytes = STANDARD
        .decode(contents.trim())
        .map_err(|error| format!("failed to decode key: {error}"))?;

    let key = <[u8; 32]>::try_from(key_bytes.as_slice())
        .map_err(|_| "encryption key must be exactly 32 bytes".to_string())?;

    // Dev-seed writes 32 zero bytes to ratatoskr.key so Service boot's
    // key-load step succeeds without bringing up a real key generator. A
    // release build accidentally shipping with that file would silently
    // encrypt every credential under a key of all zeros - no AES-256-GCM
    // confidentiality at all. Emit a warning when the loaded bytes are all
    // zero so a stray dev key file in production logs visibly. The check
    // costs one comparison; it is not a security boundary, just a
    // diagnostic tripwire.
    if key.iter().all(|&b| b == 0) {
        log::warn!(
            "loaded encryption key is all zeros; this is the dev-seed key. \
             Production builds must use a real key file."
        );
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(suffix: &str) -> std::io::Result<std::path::PathBuf> {
        let path = std::env::current_dir()?
            .join("target")
            .join(format!(
                "key-load-test-{}-{}-{}",
                std::process::id(),
                suffix,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path)?;
        Ok(path)
    }

    fn write_b64_key(path: &Path, bytes: &[u8; 32]) -> std::io::Result<()> {
        std::fs::write(path, STANDARD.encode(bytes))
    }

    #[test]
    fn load_succeeds_for_a_well_formed_ratatoskr_key() {
        let dir = temp_dir("ok").expect("temp dir");
        let original = [7u8; 32];
        write_b64_key(&dir.join("ratatoskr.key"), &original).expect("write key");
        let loaded = load_encryption_key(&dir).expect("load key");
        assert_eq!(loaded, original);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_falls_back_to_legacy_velo_key() {
        let dir = temp_dir("legacy").expect("temp dir");
        let original = [42u8; 32];
        write_b64_key(&dir.join("velo.key"), &original).expect("write velo key");
        let loaded = load_encryption_key(&dir).expect("load legacy");
        assert_eq!(loaded, original);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_prefers_ratatoskr_key_over_legacy() {
        let dir = temp_dir("prefer").expect("temp dir");
        let primary = [1u8; 32];
        let legacy = [2u8; 32];
        write_b64_key(&dir.join("ratatoskr.key"), &primary).expect("write primary");
        write_b64_key(&dir.join("velo.key"), &legacy).expect("write legacy");
        let loaded = load_encryption_key(&dir).expect("load");
        assert_eq!(loaded, primary, "primary key file must take precedence");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_returns_err_when_no_key_file_exists() {
        let dir = temp_dir("missing").expect("temp dir");
        let result = load_encryption_key(&dir);
        assert!(result.is_err(), "missing key file must return Err");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_returns_err_when_key_file_decodes_to_wrong_length() {
        let dir = temp_dir("wrong_len").expect("temp dir");
        // 16 bytes instead of 32.
        let bytes = [0u8; 16];
        std::fs::write(dir.join("ratatoskr.key"), STANDARD.encode(bytes)).expect("write key");
        let result = load_encryption_key(&dir);
        assert!(
            result.is_err(),
            "wrong-length key must return Err, got {result:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_returns_err_when_key_file_is_not_base64() {
        let dir = temp_dir("not_b64").expect("temp dir");
        std::fs::write(dir.join("ratatoskr.key"), "this is not base64!!!@@@")
            .expect("write garbage");
        let result = load_encryption_key(&dir);
        assert!(
            result.is_err(),
            "non-base64 key must return Err, got {result:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
