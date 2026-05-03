//! AES-256-GCM key file loader for Ratatoskr.
//!
//! Single canonical implementation shared by `common` (UI-side load before
//! Phase 2's IPC plumbing relocates this entirely Service-side) and
//! `service` (Service-side load during boot, gating `boot.ready`). Before
//! this crate existed both sides carried near-identical copies that drifted
//! in trivial ways (log capitalization, error string casing) and would have
//! drifted further as security hardening landed in only one - the security
//! review explicitly called this out as a regression vector for the KEK
//! that decrypts every stored credential.
//!
//! Security properties:
//!
//! - **TOCTOU-safe permissions**: opens the key file with `O_NOFOLLOW`,
//!   reads via the open file descriptor, and uses `fchmod` to repair the
//!   mode (Unix). The path is resolved exactly once; a symlink swap or
//!   permission flip between resolve and read cannot redirect us. The
//!   pre-extraction code did `read_to_string(path)` then
//!   `set_permissions(path, ...)` which had a window where a parallel
//!   reader on a transiently mode-644 file saw the key bytes.
//! - **File-owner UID validation**: rejects key files not owned by the
//!   current process UID on Unix. A shared XDG misconfiguration or hostile
//!   local user cannot substitute a key file at our `app_data_dir/`.
//! - **Zeroizing buffer**: returns a [`SecretKey`] wrapper whose `Drop`
//!   zeroes the 32-byte buffer. Callers that need the raw bytes for
//!   AES-256-GCM should `.expose()` and copy into the cipher's key slot
//!   without keeping the slice alive past the cipher construction.
//! - **All-zero rejection**: in release builds the loader refuses to
//!   return an all-zero key. `dev-seed` writes a deterministic zero key
//!   for ephemeral test data; if a stray dev key file ever ships in a
//!   release build the silent AES-256-GCM downgrade to a known key would
//!   be catastrophic, so production hard-fails instead. Debug builds warn
//!   so dev workflows continue working.

use base64::{Engine, engine::general_purpose::STANDARD};
use std::path::{Path, PathBuf};
use zeroize::{Zeroize, Zeroizing};

/// 32-byte AES-256-GCM key. Implements `Zeroize` on drop so the key bytes
/// don't linger in freed memory after callers finish using them. Callers
/// that need the raw bytes for AES-256-GCM construction should `.expose()`
/// and copy into the cipher's key slot without keeping the slice alive
/// past the cipher construction.
pub struct SecretKey {
    bytes: [u8; 32],
}

impl SecretKey {
    /// Construct from raw bytes. Used by callers that have already
    /// loaded a key (e.g., the dev-seed tooling or in-process tests).
    /// Production paths use [`load_encryption_key`].
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { bytes }
    }

    /// Borrow the raw 32-byte slice. The borrow lives only as long as
    /// the [`SecretKey`]; once that drops, the bytes are zeroed. The
    /// AES-256-GCM cipher constructors copy out of this borrow into
    /// their own internal key slot, so the lifetime is short in
    /// practice.
    pub fn expose(&self) -> &[u8; 32] {
        &self.bytes
    }
}

impl Drop for SecretKey {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never expose the key bytes via Debug. Many telemetry / logging
        // surfaces bottom out in `format!("{x:?}")`; SecretKey is the
        // last line of defense against an accidental expose.
        f.debug_struct("SecretKey").field("bytes", &"<redacted>").finish()
    }
}

/// Outcome of [`load_encryption_key`].
#[derive(Debug)]
pub enum LoadError {
    /// No key file found at either `ratatoskr.key` or the legacy
    /// `velo.key`. Production callers should treat this as fatal -
    /// silently falling back to a zero key would land credentials
    /// under known-public bytes.
    NotFound,
    /// File exists but is owned by a different UID. The file may be
    /// genuinely intended for another user (shared XDG dir
    /// misconfiguration) or substituted by a hostile local user; in
    /// either case we cannot trust it.
    WrongOwner { path: PathBuf, expected_uid: u32, actual_uid: u32 },
    /// Filesystem I/O failed while reading the key. The wrapped error
    /// carries the underlying cause.
    Io { path: PathBuf, error: std::io::Error },
    /// File contents were not valid base64 (whitespace stripped before
    /// decoding).
    InvalidBase64(base64::DecodeError),
    /// File decoded but yielded a buffer of the wrong length. AES-256-GCM
    /// requires exactly 32 bytes.
    WrongLength { expected: usize, actual: usize },
    /// Production builds refuse to load an all-zero key (the dev-seed
    /// fixture). Triggers only in release builds; debug builds emit a
    /// warning and continue.
    AllZeroInRelease { path: PathBuf },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str(
                "no encryption key file found (ratatoskr.key or legacy velo.key)",
            ),
            Self::WrongOwner {
                path,
                expected_uid,
                actual_uid,
            } => write!(
                f,
                "encryption key file {} is owned by uid {actual_uid}, expected {expected_uid}",
                path.display(),
            ),
            Self::Io { path, error } => write!(
                f,
                "failed to read encryption key file {}: {error}",
                path.display(),
            ),
            Self::InvalidBase64(error) => write!(f, "encryption key is not valid base64: {error}"),
            Self::WrongLength { expected, actual } => write!(
                f,
                "encryption key length is {actual} bytes; expected {expected}",
            ),
            Self::AllZeroInRelease { path } => write!(
                f,
                "encryption key at {} is all zeros (dev-seed fixture); refusing to use it in a release build",
                path.display(),
            ),
        }
    }
}

impl std::error::Error for LoadError {}

/// Load the AES-256-GCM key from `<app_data_dir>/ratatoskr.key`, falling
/// back to legacy `<app_data_dir>/velo.key`. See module docs for the
/// security properties this enforces.
pub fn load_encryption_key(app_data_dir: &Path) -> Result<SecretKey, LoadError> {
    let primary = app_data_dir.join("ratatoskr.key");
    let legacy = app_data_dir.join("velo.key");

    let path = if primary.exists() {
        primary
    } else if legacy.exists() {
        log::debug!("using legacy key file velo.key");
        legacy
    } else {
        return Err(LoadError::NotFound);
    };

    log::debug!("loading encryption key from {}", path.display());
    // `contents` and `decoded` carry the raw key material; both go through
    // `Zeroizing` so the bytes are wiped on every exit path (success, the
    // wrong-length error, the all-zero release path) rather than waiting
    // for the allocator to overwrite them. Without this, a heap inspection
    // of the Service process between key load and the next allocation
    // would surface plaintext key bytes in the `String` and `Vec<u8>`
    // intermediate buffers that drop normally otherwise.
    let contents: Zeroizing<String> = Zeroizing::new(open_and_read(&path)?);

    let trimmed = contents.trim();
    let decoded: Zeroizing<Vec<u8>> = Zeroizing::new(
        STANDARD.decode(trimmed).map_err(LoadError::InvalidBase64)?,
    );
    if decoded.len() != 32 {
        return Err(LoadError::WrongLength {
            expected: 32,
            actual: decoded.len(),
        });
    }

    let mut buf = [0u8; 32];
    buf.copy_from_slice(&decoded);

    if buf.iter().all(|&b| b == 0) {
        // dev-seed writes 32 zero bytes so test data round-trips
        // through the same crypto path as production. A release build
        // that boots against a stray dev key would silently downgrade
        // AES-256-GCM to a known key; refuse rather than continue.
        if cfg!(debug_assertions) {
            log::warn!(
                "loaded all-zero encryption key from {} - this is the dev-seed fixture; \
                 production builds will refuse to load it",
                path.display(),
            );
        } else {
            // Zero the local stack buffer before bailing. (`contents` and
            // `decoded` zeroize via their `Zeroizing` wrappers on drop.)
            buf.zeroize();
            return Err(LoadError::AllZeroInRelease { path });
        }
    }

    Ok(SecretKey { bytes: buf })
}

/// Open the key file with `O_NOFOLLOW` (Unix) so a symlink at the path
/// can't redirect us, then read its contents via the open fd. While the
/// fd is open, fstat for owner-UID validation and fchmod to repair the
/// mode. Path is resolved exactly once - no TOCTOU window between read
/// and chmod, no symlink swap window between owner check and read.
#[cfg(unix)]
fn open_and_read(path: &Path) -> Result<String, LoadError> {
    use std::io::Read;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let mut file = std::fs::OpenOptions::new()
        .read(true)
        // O_NOFOLLOW makes the open fail with ELOOP if the final path
        // component is a symlink. The pre-extraction code resolved the
        // path twice (read + chmod) so a swap-after-read attack was
        // possible against a transient mode-644 key file.
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| LoadError::Io {
            path: path.to_path_buf(),
            error,
        })?;

    // fstat through the open fd, NOT a fresh stat on the path - the
    // whole point of O_NOFOLLOW + fstat is that the fd is bound to the
    // inode the open succeeded on.
    let meta = file.metadata().map_err(|error| LoadError::Io {
        path: path.to_path_buf(),
        error,
    })?;

    let actual_uid = meta.uid();
    let expected_uid = unsafe { libc::geteuid() };
    if actual_uid != expected_uid {
        return Err(LoadError::WrongOwner {
            path: path.to_path_buf(),
            expected_uid,
            actual_uid,
        });
    }

    // fchmod through the open fd. Skip if the mode is already 0o600 (or
    // tighter) so we don't churn the inode mtime on every boot.
    let current_mode = meta.mode() & 0o777;
    if current_mode != 0o600 {
        let fd = file.as_raw_fd();
        let result = unsafe { libc::fchmod(fd, 0o600) };
        if result != 0 {
            log::warn!(
                "failed to repair key permissions on {} (fchmod returned {}): {}",
                path.display(),
                result,
                std::io::Error::last_os_error(),
            );
        }
    }

    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(|error| LoadError::Io {
            path: path.to_path_buf(),
            error,
        })?;
    Ok(contents)
}

/// Windows fallback: no `O_NOFOLLOW` equivalent in stable std, no UID
/// model. Read via the path; permissions are not repaired (NTFS ACLs
/// are out of scope for this loader). The key file's ACL inheritance
/// is the user's responsibility on Windows.
#[cfg(not(unix))]
fn open_and_read(path: &Path) -> Result<String, LoadError> {
    std::fs::read_to_string(path).map_err(|error| LoadError::Io {
        path: path.to_path_buf(),
        error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(suffix: &str) -> std::io::Result<PathBuf> {
        let path = std::env::current_dir()?
            .join("target")
            .join(format!(
                "crypto-key-test-{}-{}-{}",
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
    fn loads_well_formed_ratatoskr_key() {
        let dir = temp_dir("ok").expect("temp dir");
        let original = [7u8; 32];
        write_b64_key(&dir.join("ratatoskr.key"), &original).expect("write key");
        let key = load_encryption_key(&dir).expect("load");
        assert_eq!(key.expose(), &original);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn falls_back_to_legacy_velo_key() {
        let dir = temp_dir("legacy").expect("temp dir");
        let original = [42u8; 32];
        write_b64_key(&dir.join("velo.key"), &original).expect("write velo");
        let key = load_encryption_key(&dir).expect("load");
        assert_eq!(key.expose(), &original);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prefers_ratatoskr_over_velo() {
        let dir = temp_dir("prefer").expect("temp dir");
        let primary = [1u8; 32];
        let legacy = [2u8; 32];
        write_b64_key(&dir.join("ratatoskr.key"), &primary).expect("primary");
        write_b64_key(&dir.join("velo.key"), &legacy).expect("legacy");
        let key = load_encryption_key(&dir).expect("load");
        assert_eq!(key.expose(), &primary);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_key_file_returns_not_found() {
        let dir = temp_dir("missing").expect("temp dir");
        match load_encryption_key(&dir) {
            Err(LoadError::NotFound) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_length_key_is_rejected() {
        let dir = temp_dir("wrong_len").expect("temp dir");
        let bytes = [0u8; 16];
        std::fs::write(dir.join("ratatoskr.key"), STANDARD.encode(bytes)).expect("write");
        match load_encryption_key(&dir) {
            Err(LoadError::WrongLength { expected: 32, actual: 16 }) => {}
            other => panic!("expected WrongLength, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_base64_key_is_rejected() {
        let dir = temp_dir("garbage").expect("temp dir");
        std::fs::write(dir.join("ratatoskr.key"), "this is not base64!!!@@@")
            .expect("write");
        match load_encryption_key(&dir) {
            Err(LoadError::InvalidBase64(_)) => {}
            other => panic!("expected InvalidBase64, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn fchmod_repairs_overly_open_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("chmod").expect("temp dir");
        let path = dir.join("ratatoskr.key");
        write_b64_key(&path, &[3u8; 32]).expect("write");
        // Set a deliberately too-open mode so the loader has something
        // to repair. The fchmod-via-fd repair runs while the file is
        // open for read.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("set perms");
        let _ = load_encryption_key(&dir).expect("load");
        let mode = std::fs::metadata(&path)
            .expect("stat")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "loader must fchmod the file to 0o600");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Behaviour split: debug builds warn-and-load on the dev-seed
    /// all-zero key so local development continues working; release
    /// builds hard-fail so a stray dev key cannot silently downgrade
    /// AES-256-GCM. Both arms are covered here so the contract is
    /// observable from a single test source.
    #[test]
    fn all_zero_key_warn_in_debug_hard_fail_in_release() {
        let dir = temp_dir("zero").expect("temp dir");
        write_b64_key(&dir.join("ratatoskr.key"), &[0u8; 32]).expect("write");
        let result = load_encryption_key(&dir);
        if cfg!(debug_assertions) {
            let key = result.expect("debug build must load with warn");
            assert_eq!(key.expose(), &[0u8; 32]);
        } else {
            match result {
                Err(LoadError::AllZeroInRelease { .. }) => {}
                other => panic!("release must hard-fail; got {other:?}"),
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SecretKey's Debug never spills the key bytes. The struct lives
    /// long enough to be format-printed in many places (logs, error
    /// messages); a stray `{:?}` must not exfiltrate the key.
    #[test]
    fn debug_does_not_leak_key_bytes() {
        let key = SecretKey::from_bytes([0xABu8; 32]);
        let rendered = format!("{key:?}");
        assert!(
            !rendered.contains("171") && !rendered.contains("0xAB") && !rendered.contains("ab"),
            "Debug must redact key bytes; got: {rendered}"
        );
        assert!(rendered.contains("redacted"));
    }
}
