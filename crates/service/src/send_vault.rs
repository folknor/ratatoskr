//! Send vault: durable transfer of compose-send attachment bytes.
//!
//! Phase 2 task 13's bytes-ownership boundary. The UI writes attachment
//! bytes into `<app_data>/staging/<send_id>/<index>.bin` before issuing
//! `action.send`; the handler validates each path, verifies the
//! declared BLAKE3 hash, and atomically renames the file into the
//! Service-owned vault under `<app_data>/send_vault/<send_id>/`. After
//! the rename + journal write, the staging directory is the UI's
//! responsibility; the vault is the Service's, unlinked when the job
//! reaches a terminal status (or when boot recovery finds an orphan).
//!
//! Why not a content-hash reference scheme? A hash-validated reference
//! to the staging file would not give the Service a durable claim on
//! the bytes - the staging file could be deleted, have its permissions
//! changed, have a symlink swapped under it, or hit an FS error
//! mid-SMTP after the ack returned. The handler therefore takes
//! ownership of the bytes (rename = atomic on the same filesystem) so
//! the journal entry references a path the Service controls.
//!
//! Same-filesystem assumption: staging and vault both live under
//! `<app_data>/`, so `rename(2)` is atomic and cheap. Cross-filesystem
//! transfer would need copy + fsync + verify and is explicitly out of
//! scope for Phase 2 - we fail loudly rather than silently degrading.

#![allow(
    dead_code,
    reason = "wired up by handle_send / worker drain / boot recovery in following commits"
)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use service_api::PlanId;

/// Errors surfaced from the send vault module. The handler maps these
/// to `ServiceError` variants; the boot-time orphan cleanup logs and
/// continues.
#[derive(Debug, thiserror::Error)]
pub(crate) enum VaultError {
    #[error("invalid staging path: {0}")]
    InvalidPath(String),
    #[error("staging file missing or unreadable: {0}: {1}")]
    StagingIo(PathBuf, std::io::Error),
    #[error("staging file is a symlink: {0}")]
    StagingSymlink(PathBuf),
    #[error("hash mismatch for staging file: {0}")]
    HashMismatch(PathBuf),
    #[error("vault io error: {0}: {1}")]
    VaultIo(PathBuf, std::io::Error),
}

/// `<app_data>/staging/`. UI-owned root. Returns the path; does NOT
/// create the directory (the Service should not be writing here).
pub(crate) fn staging_root(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("staging")
}

/// `<app_data>/send_vault/`. Service-owned root; the handler creates
/// per-send subdirectories during the transfer step.
pub(crate) fn vault_root(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("send_vault")
}

/// `<app_data>/staging/<send_id>/`.
pub(crate) fn staging_dir(app_data_dir: &Path, send_id: &PlanId) -> PathBuf {
    staging_root(app_data_dir).join(send_id.to_string())
}

/// `<app_data>/send_vault/<send_id>/`.
pub(crate) fn vault_dir(app_data_dir: &Path, send_id: &PlanId) -> PathBuf {
    vault_root(app_data_dir).join(send_id.to_string())
}

/// Validate a UI-supplied relative path.
///
/// The handler trusts the UI for *intent* (it can put any bytes in
/// staging that the user asked it to) but never for *layout*. A
/// staging path that escapes its `<send_id>/` subdirectory could read
/// arbitrary app-data files (e.g. `ratatoskr.key`, the SQLite DB) and
/// move them into the vault. The rules:
///
/// - non-empty
/// - no `..` segments anywhere in the path
/// - no absolute path
/// - no NUL bytes (defensive against C-string smuggling)
/// - no leading `/` or platform separator
/// - no Windows drive letters (`C:`) or UNC prefixes
///
/// Symlink rejection is enforced separately at I/O time via `lstat`
/// rather than `stat`; a relative path that *names* a symlink survives
/// this check but the I/O path will reject it.
pub(crate) fn validate_relative_path(rel: &str) -> Result<(), VaultError> {
    if rel.is_empty() {
        return Err(VaultError::InvalidPath("empty".into()));
    }
    if rel.contains('\0') {
        return Err(VaultError::InvalidPath("contains NUL byte".into()));
    }
    let p = Path::new(rel);
    if p.is_absolute() {
        return Err(VaultError::InvalidPath(format!("absolute: {rel}")));
    }
    for component in p.components() {
        use std::path::Component;
        match component {
            Component::ParentDir => {
                return Err(VaultError::InvalidPath(format!(
                    "contains ..: {rel}"
                )));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(VaultError::InvalidPath(format!(
                    "rooted or prefixed: {rel}"
                )));
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }
    Ok(())
}

/// Compute the BLAKE3 of a file's contents. Caller has already
/// verified the path is non-symlinked via `lstat`.
fn blake3_file(path: &Path) -> Result<[u8; 32], std::io::Error> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(*hasher.finalize().as_bytes())
}

/// Validate, hash-verify, and atomically transfer a staged attachment
/// into the Service-owned vault. Returns the vault-relative path the
/// journal should record.
///
/// Caller is responsible for creating the vault directory once per
/// send; this function just runs `rename` into it.
///
/// Hash mismatch / IO error / symlinked staging path -> Err. The
/// handler checks the variant to decide whether to rollback partial
/// vault state before journaling.
pub(crate) fn verify_and_transfer(
    app_data_dir: &Path,
    send_id: &PlanId,
    relative_path: &str,
    expected_hash: &[u8; 32],
    vault_index: usize,
) -> Result<PathBuf, VaultError> {
    validate_relative_path(relative_path)?;

    let staging_path = staging_dir(app_data_dir, send_id).join(relative_path);

    // lstat-checked: a `symlink_metadata` lookup that does NOT follow
    // symlinks. If the staging file is a symlink, reject - we never
    // want to follow a UI-supplied link out of the staging dir.
    let meta = std::fs::symlink_metadata(&staging_path)
        .map_err(|e| VaultError::StagingIo(staging_path.clone(), e))?;
    if meta.file_type().is_symlink() {
        return Err(VaultError::StagingSymlink(staging_path));
    }

    let actual_hash = blake3_file(&staging_path)
        .map_err(|e| VaultError::StagingIo(staging_path.clone(), e))?;
    if &actual_hash != expected_hash {
        return Err(VaultError::HashMismatch(staging_path));
    }

    let vault_path = vault_dir(app_data_dir, send_id).join(format!("{vault_index}.bin"));

    // Same-FS rename. If staging and vault diverge across filesystems
    // (a future deployment quirk), this fails with EXDEV and we
    // surface the error rather than silently degrading.
    std::fs::rename(&staging_path, &vault_path)
        .map_err(|e| VaultError::VaultIo(vault_path.clone(), e))?;

    Ok(vault_path)
}

/// Best-effort recursive removal of a vault directory. Called by the
/// worker after a job reaches terminal status, and by the handler on
/// pre-journal rollback if any attachment fails its transfer.
pub(crate) fn cleanup_vault_dir(app_data_dir: &Path, send_id: &PlanId) {
    let dir = vault_dir(app_data_dir, send_id);
    if let Err(error) = std::fs::remove_dir_all(&dir)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!(
            "send_vault: failed to remove {}: {error}",
            dir.display()
        );
    }
}

/// Boot-time orphan cleanup. Walks `<app_data>/send_vault/`; any
/// subdirectory whose name does not parse as a UUIDv7, or whose
/// parsed PlanId is not in `live_jobs`, is unlinked. The action-job
/// journal is the source of truth: a vault dir that the journal
/// doesn't know about is leftover from a crash before journaling, or
/// from a job that already reached terminal status without unlinking
/// (a bug, but the boot pass cleans it up).
///
/// `live_jobs` is the set of `PlanId`s currently `kind = 'send'` and
/// `status NOT IN ('completed', 'failed')`. The caller computes this
/// from the journal during boot recovery (see `boot.rs`).
pub(crate) fn cleanup_orphan_vaults(
    app_data_dir: &Path,
    live_jobs: &HashSet<PlanId>,
) -> Result<usize, std::io::Error> {
    let root = vault_root(app_data_dir);
    if !root.exists() {
        return Ok(0);
    }
    let mut removed = 0usize;
    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => {
                log::warn!(
                    "send_vault: non-utf8 vault entry, removing: {}",
                    path.display()
                );
                let _ = std::fs::remove_dir_all(&path);
                removed += 1;
                continue;
            }
        };
        let parsed = uuid::Uuid::parse_str(&name).ok().map(PlanId);
        let keep = parsed.is_some_and(|id| live_jobs.contains(&id));
        if !keep {
            if let Err(error) = std::fs::remove_dir_all(&path) {
                log::warn!(
                    "send_vault: failed to remove orphan {}: {error}",
                    path.display()
                );
            } else {
                removed += 1;
            }
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn setup() -> (TempDir, PlanId) {
        let dir = TempDir::new().expect("tmp");
        let send_id = PlanId::new_v7();
        std::fs::create_dir_all(staging_dir(dir.path(), &send_id)).expect("mkstaging");
        std::fs::create_dir_all(vault_dir(dir.path(), &send_id)).expect("mkvault");
        (dir, send_id)
    }

    fn write_staged(staging: &Path, name: &str, bytes: &[u8]) -> [u8; 32] {
        let mut file = std::fs::File::create(staging.join(name)).expect("create");
        file.write_all(bytes).expect("write");
        *blake3::hash(bytes).as_bytes()
    }

    #[test]
    fn validate_rejects_traversal() {
        assert!(validate_relative_path("../etc/passwd").is_err());
        assert!(validate_relative_path("a/../b").is_err());
        assert!(validate_relative_path("/abs").is_err());
        assert!(validate_relative_path("").is_err());
        assert!(validate_relative_path("a/\0/b").is_err());
    }

    #[test]
    fn validate_accepts_simple_relative_paths() {
        assert!(validate_relative_path("0.bin").is_ok());
        assert!(validate_relative_path("a/b.bin").is_ok());
        assert!(validate_relative_path("./0.bin").is_ok());
    }

    #[test]
    fn transfer_happy_path_renames_into_vault() {
        let (tmp, send_id) = setup();
        let staging = staging_dir(tmp.path(), &send_id);
        let hash = write_staged(&staging, "0.bin", b"hello world");
        let vault_path =
            verify_and_transfer(tmp.path(), &send_id, "0.bin", &hash, 0).expect("transfer");
        assert!(vault_path.exists());
        assert!(!staging.join("0.bin").exists(), "staging consumed");
        let bytes = std::fs::read(&vault_path).expect("read");
        assert_eq!(bytes, b"hello world");
    }

    #[test]
    fn transfer_rejects_hash_mismatch() {
        let (tmp, send_id) = setup();
        let staging = staging_dir(tmp.path(), &send_id);
        let _ = write_staged(&staging, "0.bin", b"hello world");
        let bogus = [0u8; 32];
        let err = verify_and_transfer(tmp.path(), &send_id, "0.bin", &bogus, 0)
            .expect_err("should reject");
        assert!(matches!(err, VaultError::HashMismatch(_)));
        assert!(staging.join("0.bin").exists(), "staging preserved on rejection");
    }

    #[test]
    fn transfer_rejects_traversal() {
        let (tmp, send_id) = setup();
        let bogus = [0u8; 32];
        let err = verify_and_transfer(tmp.path(), &send_id, "../escape.bin", &bogus, 0)
            .expect_err("should reject");
        assert!(matches!(err, VaultError::InvalidPath(_)));
    }

    #[cfg(unix)]
    #[test]
    fn transfer_rejects_symlink_in_staging() {
        let (tmp, send_id) = setup();
        let staging = staging_dir(tmp.path(), &send_id);
        let target = tmp.path().join("outside.bin");
        std::fs::write(&target, b"secret").expect("target");
        std::os::unix::fs::symlink(&target, staging.join("link.bin")).expect("symlink");
        let bogus = [0u8; 32];
        let err = verify_and_transfer(tmp.path(), &send_id, "link.bin", &bogus, 0)
            .expect_err("should reject");
        assert!(matches!(err, VaultError::StagingSymlink(_)));
    }

    #[test]
    fn cleanup_orphan_removes_unknown_dirs() {
        let tmp = TempDir::new().expect("tmp");
        let live_id = PlanId::new_v7();
        let orphan_id = PlanId::new_v7();
        std::fs::create_dir_all(vault_dir(tmp.path(), &live_id)).expect("live");
        std::fs::create_dir_all(vault_dir(tmp.path(), &orphan_id)).expect("orphan");
        std::fs::create_dir_all(vault_root(tmp.path()).join("not-a-uuid")).expect("garbage");

        let mut live = HashSet::new();
        live.insert(live_id);
        let removed =
            cleanup_orphan_vaults(tmp.path(), &live).expect("cleanup");
        assert_eq!(removed, 2);
        assert!(vault_dir(tmp.path(), &live_id).exists(), "live preserved");
        assert!(!vault_dir(tmp.path(), &orphan_id).exists(), "orphan removed");
    }

    #[test]
    fn cleanup_vault_dir_removes_recursively() {
        let (tmp, send_id) = setup();
        let staging = staging_dir(tmp.path(), &send_id);
        let hash = write_staged(&staging, "0.bin", b"x");
        let _ = verify_and_transfer(tmp.path(), &send_id, "0.bin", &hash, 0).expect("transfer");
        cleanup_vault_dir(tmp.path(), &send_id);
        assert!(!vault_dir(tmp.path(), &send_id).exists());
    }
}
