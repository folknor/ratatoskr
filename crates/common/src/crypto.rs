use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::path::Path;
use zeroize::Zeroize;

/// App-wide encryption key state shared by commands that don't own provider clients.
/// Key material is zeroized on drop to prevent lingering in freed memory.
#[derive(Clone)]
pub struct AppCryptoState {
    encryption_key: [u8; 32],
}

impl Drop for AppCryptoState {
    fn drop(&mut self) {
        self.encryption_key.zeroize();
    }
}

impl AppCryptoState {
    pub fn new(encryption_key: [u8; 32]) -> Self {
        Self { encryption_key }
    }

    pub fn encryption_key(&self) -> &[u8; 32] {
        &self.encryption_key
    }
}

/// Load the AES-256-GCM encryption key from the key file.
///
/// Tries `ratatoskr.key` first, falls back to legacy `velo.key`. Thin
/// wrapper around `crypto_key::load_encryption_key` that flattens the
/// structured `LoadError` into a `String` for backward-compat with
/// existing callers (the `rtsk::load_encryption_key` re-export and
/// `crates/app/src/app.rs`).
///
/// The shared crate handles every security property the previous in-line
/// implementation lacked: TOCTOU-safe permission repair via fchmod on
/// the open fd, file-owner UID validation (Unix), zeroizing buffer for
/// the loaded bytes, and unconditional rejection of an all-zero key. See
/// `crates/crypto-key/src/lib.rs` for details.
pub fn load_encryption_key(app_data_dir: &Path) -> Result<[u8; 32], String> {
    let secret = crypto_key::load_encryption_key(app_data_dir).map_err(|e| e.to_string())?;
    // Copy out before `secret` drops and zeroes its buffer. Production
    // callers wrap the returned `[u8; 32]` in `AppCryptoState` (defined
    // above) which preserves the zeroize-on-drop property.
    Ok(*secret.expose())
}

/// Decrypt a value encrypted by the TS crypto module.
///
/// Expected format: `base64(iv):base64(ciphertext+tag)` (AES-256-GCM).
pub fn decrypt_value(key: &[u8; 32], encrypted: &str) -> Result<String, String> {
    let (iv_part, ct_part) = encrypted.split_once(':').ok_or_else(|| {
        log::error!("Decrypt failed: invalid format (missing ':' separator)");
        "Invalid encrypted format: missing ':'".to_string()
    })?;

    let iv_bytes = STANDARD
        .decode(iv_part)
        .map_err(|e| format!("Failed to decode IV: {e}"))?;
    let ciphertext = STANDARD
        .decode(ct_part)
        .map_err(|e| format!("Failed to decode ciphertext: {e}"))?;

    if iv_bytes.len() != 12 {
        return Err(format!(
            "Invalid IV length: expected 12, got {}",
            iv_bytes.len()
        ));
    }

    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| format!("Invalid encryption key: {e}"))?;
    let nonce = Nonce::from_slice(&iv_bytes);

    let plaintext = cipher.decrypt(nonce, ciphertext.as_ref()).map_err(|e| {
        log::error!("AES-256-GCM decryption failed: {e}");
        format!("Decryption failed: {e}")
    })?;

    String::from_utf8(plaintext).map_err(|e| format!("Decrypted value is not valid UTF-8: {e}"))
}

/// Encrypt a value using the same AES-256-GCM scheme as the TS crypto module.
///
/// Returns `base64(iv):base64(ciphertext+tag)`.
pub fn encrypt_value(key: &[u8; 32], plaintext: &str) -> Result<String, String> {
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| format!("Invalid encryption key: {e}"))?;

    let mut nonce_bytes = [0u8; 12];
    getrandom::fill(&mut nonce_bytes).map_err(|e| format!("RNG failed: {e}"))?;
    let nonce = Nonce::from(nonce_bytes);

    let ciphertext = cipher.encrypt(&nonce, plaintext.as_bytes()).map_err(|e| {
        log::error!("AES-256-GCM encryption failed: {e}");
        format!("Encryption failed: {e}")
    })?;

    Ok(format!(
        "{}:{}",
        STANDARD.encode(nonce_bytes),
        STANDARD.encode(&ciphertext)
    ))
}

/// Check if a value appears to be encrypted (matches `base64:base64` format
/// with a 12-byte IV).
pub fn is_encrypted(value: &str) -> bool {
    let Some((iv_part, ct_part)) = value.split_once(':') else {
        return false;
    };
    let Ok(iv) = STANDARD.decode(iv_part) else {
        return false;
    };
    iv.len() == 12 && STANDARD.decode(ct_part).is_ok()
}

/// Try to decrypt a value, falling back to the raw string for pre-encryption data.
///
/// Used by Gmail and Graph where the value is always present (non-Option).
pub fn decrypt_or_raw(key: &[u8; 32], value: &str) -> String {
    if is_encrypted(value) {
        decrypt_value(key, value).unwrap_or_else(|e| {
            log::warn!("Decryption failed for encrypted value - returning raw. Key may be corrupted or rotated: {e}");
            value.to_string()
        })
    } else {
        value.to_string()
    }
}

/// Decrypt an `Option<String>` if it looks encrypted, pass through otherwise.
///
/// Used by JMAP and IMAP where credentials may be `None`.
pub fn decrypt_if_needed(key: &[u8; 32], value: Option<String>) -> Result<Option<String>, String> {
    value
        .map(|raw| {
            if is_encrypted(&raw) {
                decrypt_value(key, &raw).map_err(|e| format!("decrypt credential: {e}"))
            } else {
                Ok(raw)
            }
        })
        .transpose()
}
