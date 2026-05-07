//! Shared marker-file helper: sync, push, and account-delete recovery
//! markers all live here as `MarkerFile<T>`. Each marker carries a
//! step-completed list (or status) serialised as JSON. Recovery on
//! boot: read marker -> identify next un-completed step -> run
//! forward. Each step must be idempotent. Account-delete steps are
//! ordered: body -> inline -> attachment-cache-clear -> search ->
//! accounts row CASCADE; CASCADE is always last because external
//! stores cannot be reverse-mapped by `account_id` once the row is
//! gone.
//!
//! The helper is generic over the payload type so each consumer can
//! shape its own state. Sync markers carry a four-state enum
//! (`InProgress | Completed | Cancelled | Failed`); account-delete
//! markers carry a step list. The on-disk shape is "one JSON file
//! per key under `<app_data>/<dir_name>/<key>.json`," with
//! atomic-rename writes (temp file in the same directory + `rename`)
//! so a crash mid-write leaves either the prior payload or no file
//! at all - never a partial write.

use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::fs;
use tokio::io::AsyncWriteExt;

/// Generic marker-file helper. Parameterised over the payload type
/// so each consumer can shape its own state. The directory name is
/// fixed at construction; the key (typically an `account_id`) is
/// passed per-call so a single helper instance serves N markers.
pub(crate) struct MarkerFile<T> {
    dir_name: &'static str,
    _phantom: PhantomData<T>,
}

impl<T> MarkerFile<T>
where
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    pub(crate) const fn new(dir_name: &'static str) -> Self {
        Self {
            dir_name,
            _phantom: PhantomData,
        }
    }

    fn dir(&self, app_data_dir: &Path) -> PathBuf {
        app_data_dir.join(self.dir_name)
    }

    fn path(&self, app_data_dir: &Path, key: &str) -> PathBuf {
        self.dir(app_data_dir).join(format!("{key}.json"))
    }

    /// Write the marker atomically and durably. Crash mid-write
    /// leaves either the prior payload or no file at all - never a
    /// partial write. `rename` is atomic on POSIX; Windows uses
    /// `ReplaceFileExW`-equivalent semantics via `fs::rename`.
    ///
    /// Durability sequence:
    ///   1. Write tmp file, fsync the tmp file's data + metadata.
    ///   2. Rename tmp -> final.
    ///   3. fsync the parent directory so the rename's dirent is
    ///      durable before the call returns (Unix). Power loss
    ///      between rename and the next dirty-page flush would
    ///      otherwise leave the dirent on disk while the data
    ///      blocks point at stale or zeroed sectors on filesystems
    ///      with looser ordering (ext4 `data=writeback`, xfs).
    ///
    /// On Windows the directory fsync step is a no-op (NTFS
    /// journals directory metadata; opening a directory for sync
    /// is not supported through the same API).
    pub(crate) async fn write(
        &self,
        app_data_dir: &Path,
        key: &str,
        payload: &T,
    ) -> Result<(), String> {
        let dir = self.dir(app_data_dir);
        fs::create_dir_all(&dir)
            .await
            .map_err(|e| format!("create {} dir: {e}", self.dir_name))?;
        let final_path = self.path(app_data_dir, key);
        let tmp_path = dir.join(format!("{key}.json.tmp"));
        let bytes = serde_json::to_vec_pretty(payload)
            .map_err(|e| format!("serialize {} marker: {e}", self.dir_name))?;

        // Step 1: write + fsync the tmp file.
        let mut tmp_file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp_path)
            .await
            .map_err(|e| format!("open {} tmp: {e}", self.dir_name))?;
        tmp_file
            .write_all(&bytes)
            .await
            .map_err(|e| format!("write {} tmp: {e}", self.dir_name))?;
        tmp_file
            .sync_all()
            .await
            .map_err(|e| format!("fsync {} tmp: {e}", self.dir_name))?;
        drop(tmp_file);

        // Step 2: rename tmp -> final.
        fs::rename(&tmp_path, &final_path)
            .await
            .map_err(|e| format!("rename {} tmp: {e}", self.dir_name))?;

        // Step 3: fsync the parent dir so the dirent is durable
        // (Unix only; NTFS does not need or support this).
        #[cfg(unix)]
        {
            let dir_handle = fs::File::open(&dir)
                .await
                .map_err(|e| format!("open {} dir for fsync: {e}", self.dir_name))?;
            dir_handle
                .sync_all()
                .await
                .map_err(|e| format!("fsync {} dir: {e}", self.dir_name))?;
        }

        Ok(())
    }

    /// Read one marker by key. Returns `Ok(None)` if no file exists.
    pub(crate) async fn read(
        &self,
        app_data_dir: &Path,
        key: &str,
    ) -> Result<Option<T>, String> {
        let path = self.path(app_data_dir, key);
        match fs::read(&path).await {
            Ok(bytes) => {
                let payload: T = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("parse {} marker {key}: {e}", self.dir_name))?;
                Ok(Some(payload))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("read {} marker {key}: {e}", self.dir_name)),
        }
    }

    /// Idempotent unlink. `NotFound` is treated as success because
    /// drain code paths re-run on boot and may double-unlink.
    pub(crate) async fn unlink(
        &self,
        app_data_dir: &Path,
        key: &str,
    ) -> Result<(), String> {
        let path = self.path(app_data_dir, key);
        match fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("unlink {} marker {key}: {e}", self.dir_name)),
        }
    }

    /// List every marker in the directory. Returns `Ok(vec![])` if
    /// the directory does not exist (clean boot, no prior crashes).
    /// Each entry is `(key, payload)`; the key is the file stem with
    /// the `.json` extension stripped.
    ///
    /// Per-marker read or parse errors log + skip the offending file
    /// so a single corrupt marker does not stop the drain for every
    /// other account. The previous behavior short-circuited the
    /// whole call on the first parse failure, which made one bad
    /// JSON file freeze account-deletion drain workspace-wide.
    ///
    /// Sync markers do their own walk inside
    /// `service::startup_invariants::discover_dirty_accounts` because
    /// that path needs to surface `Unparseable` markers as a
    /// distinct dirty status.
    #[allow(dead_code)]
    pub(crate) async fn list(
        &self,
        app_data_dir: &Path,
    ) -> Result<Vec<(String, T)>, String> {
        let dir = self.dir(app_data_dir);
        let mut entries = match fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("read {} dir: {e}", self.dir_name)),
        };
        let mut out = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| format!("iterate {} dir: {e}", self.dir_name))?
        {
            let path = entry.path();
            let Some(stem) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            // Skip temp files left behind by a crashed write.
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let bytes = match fs::read(&path).await {
                Ok(b) => b,
                Err(e) => {
                    log::warn!(
                        "{}: skip marker {stem}, read failed: {e}",
                        self.dir_name
                    );
                    continue;
                }
            };
            let payload: T = match serde_json::from_slice(&bytes) {
                Ok(p) => p,
                Err(e) => {
                    log::warn!(
                        "{}: skip marker {stem}, parse failed: {e}",
                        self.dir_name
                    );
                    continue;
                }
            };
            out.push((stem, payload));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct StepList {
        completed: Vec<String>,
    }

    fn fixture() -> MarkerFile<StepList> {
        MarkerFile::new("test_markers")
    }

    #[tokio::test]
    async fn write_then_read_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let m = fixture();
        let payload = StepList {
            completed: vec!["body".into(), "inline".into()],
        };
        m.write(dir.path(), "acct-1", &payload).await.expect("write");
        let recovered = m.read(dir.path(), "acct-1").await.expect("read");
        assert_eq!(recovered.as_ref(), Some(&payload));
    }

    #[tokio::test]
    async fn read_missing_returns_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let m = fixture();
        let recovered = m.read(dir.path(), "nope").await.expect("read");
        assert!(recovered.is_none());
    }

    #[tokio::test]
    async fn unlink_is_idempotent_on_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let m = fixture();
        // First unlink: no marker exists. Should still succeed.
        m.unlink(dir.path(), "ghost").await.expect("first unlink");
        // Second unlink on the same key: still no marker. Still ok.
        m.unlink(dir.path(), "ghost").await.expect("second unlink");
    }

    #[tokio::test]
    async fn list_returns_all_markers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let m = fixture();
        m.write(
            dir.path(),
            "a",
            &StepList {
                completed: vec!["body".into()],
            },
        )
        .await
        .expect("write a");
        m.write(
            dir.path(),
            "b",
            &StepList {
                completed: vec!["body".into(), "inline".into()],
            },
        )
        .await
        .expect("write b");
        let mut listed = m.list(dir.path()).await.expect("list");
        listed.sort_by(|x, y| x.0.cmp(&y.0));
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].0, "a");
        assert_eq!(listed[1].0, "b");
        assert_eq!(listed[1].1.completed.len(), 2);
    }

    #[tokio::test]
    async fn list_returns_empty_when_dir_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let m = fixture();
        let listed = m.list(dir.path()).await.expect("list");
        assert!(listed.is_empty());
    }

    #[tokio::test]
    async fn list_skips_tmp_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let m = fixture();
        m.write(
            dir.path(),
            "real",
            &StepList {
                completed: Vec::new(),
            },
        )
        .await
        .expect("write");
        // Drop a stray .tmp file in the directory, simulating a
        // crashed write.
        let inner = dir.path().join("test_markers");
        std::fs::write(inner.join("real.json.tmp"), b"junk").expect("write tmp");
        let listed = m.list(dir.path()).await.expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0, "real");
    }

    #[tokio::test]
    async fn write_overwrites_existing_atomically() {
        let dir = tempfile::tempdir().expect("tempdir");
        let m = fixture();
        m.write(
            dir.path(),
            "k",
            &StepList {
                completed: vec!["one".into()],
            },
        )
        .await
        .expect("write 1");
        m.write(
            dir.path(),
            "k",
            &StepList {
                completed: vec!["one".into(), "two".into()],
            },
        )
        .await
        .expect("write 2");
        let recovered = m.read(dir.path(), "k").await.expect("read").expect("some");
        assert_eq!(recovered.completed, vec!["one", "two"]);
    }
}
