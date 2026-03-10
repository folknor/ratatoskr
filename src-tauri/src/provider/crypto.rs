use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::path::Path;

/// Load the AES-256-GCM encryption key from the key file.
///
/// Tries `ratatoskr.key` first, falls back to legacy `velo.key`.
pub fn load_encryption_key(app_data_dir: &Path) -> Result<[u8; 32], String> {
    let key_path = app_data_dir.join("ratatoskr.key");
    let legacy_path = app_data_dir.join("velo.key");

    let path = if key_path.exists() {
        key_path
    } else if legacy_path.exists() {
        legacy_path
    } else {
        return Err("No encryption key file found (ratatoskr.key)".to_string());
    };

    let contents =
        std::fs::read_to_string(&path).map_err(|e| format!("Failed to read key file: {e}"))?;

    // Retroactively fix permissions on existing key files
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }

    let key_bytes = STANDARD
        .decode(contents.trim())
        .map_err(|e| format!("Failed to decode key: {e}"))?;

    <[u8; 32]>::try_from(key_bytes.as_slice())
        .map_err(|_| "Encryption key must be exactly 32 bytes".to_string())
}

/// Decrypt a value encrypted by the TS crypto module.
///
/// Expected format: `base64(iv):base64(ciphertext+tag)` (AES-256-GCM).
pub fn decrypt_value(key: &[u8; 32], encrypted: &str) -> Result<String, String> {
    let (iv_part, ct_part) = encrypted
        .split_once(':')
        .ok_or_else(|| "Invalid encrypted format: missing ':'".to_string())?;

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

    let plaintext = cipher
        .decrypt(nonce, ciphertext.as_ref())
        .map_err(|e| format!("Decryption failed: {e}"))?;

    String::from_utf8(plaintext).map_err(|e| format!("Decrypted value is not valid UTF-8: {e}"))
}

/// Encrypt a value using the same AES-256-GCM scheme as the TS crypto module.
///
/// Returns `base64(iv):base64(ciphertext+tag)`.
pub fn encrypt_value(key: &[u8; 32], plaintext: &str) -> Result<String, String> {
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| format!("Invalid encryption key: {e}"))?;

    let mut nonce_bytes = [0u8; 12];
    getrandom::getrandom(&mut nonce_bytes).map_err(|e| format!("RNG failed: {e}"))?;
    let nonce = Nonce::from(nonce_bytes);

    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| format!("Encryption failed: {e}"))?;

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
