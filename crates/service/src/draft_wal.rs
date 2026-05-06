//! Service-side draft WAL drain.
//!
//! Phase 6a-part-2: the UI's compose auto-save and window-close
//! paths append to `<data_dir>/drafts.wal` synchronously. The
//! Service drains the WAL on next boot via
//! `BootPhase::DrainingDraftWal` before signalling boot.ready, so
//! the UI's editor restore reads `local_drafts` against the fully-
//! replayed state.
//!
//! Crash safety:
//! - The WAL writer flushes + `sync_all`s after every line, so a
//!   process kill mid-write leaves at most one partial trailing
//!   line on disk. The drainer skips any unparseable line (logs a
//!   warning) and continues.
//! - SQLite's `ON CONFLICT(id) DO UPDATE` (in `db_save_local_draft_sync`)
//!   makes a duplicate replay a no-op. A crash mid-drain that left
//!   N rows persisted and the rest unprocessed is safe to re-run -
//!   the persisted rows update with the same payload, the
//!   unprocessed rows insert.
//! - On successful drain the active WAL is renamed aside to
//!   `drafts.wal.replayed.<epoch_ms>`. A repeat boot finds no
//!   active WAL (the rename target is ignored on subsequent
//!   drains) and skips. Old `*.replayed.*` files are not pruned
//!   automatically; they are bounded by the user's per-session
//!   draft turnover and can be GC'd on a future maintenance pass.

use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use db::db::Connection;
use rtsk::db::queries_extra::{SaveLocalDraftParams, db_save_local_draft_sync};
use serde::{Deserialize, Serialize};

/// Filename of the active WAL inside the user data directory.
/// Mirrors the UI-side constant (the two crates do not share a
/// module today; the constant is short and stable).
pub(crate) const WAL_FILENAME: &str = "drafts.wal";

/// One entry on the WAL. Matches the UI-side serializer in
/// `crates/app/src/draft_wal.rs`. The two crates do not share a
/// module today; the wire shape is "NDJSON of `WalEntry`," validated
/// at each end by serde.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WalEntry {
    epoch_ms: u64,
    params: SaveLocalDraftParams,
}

fn wal_path(data_dir: &Path) -> PathBuf {
    data_dir.join(WAL_FILENAME)
}

fn rotated_path(data_dir: &Path) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0);
    data_dir.join(format!("{WAL_FILENAME}.replayed.{stamp}"))
}

/// Drain the WAL, replaying entries into `local_drafts`.
///
/// Returns the count of entries persisted (excluding skipped
/// unparseable lines). A missing WAL file is `Ok(0)`. Failures
/// reading or replaying log a warning and the drain continues -
/// per the boot recovery contract (log+continue is preferable to
/// blocking the boot handshake on a broken local file).
pub(crate) fn drain(conn: &Connection, data_dir: &Path) -> Result<usize, String> {
    let path = wal_path(data_dir);
    let file = match File::open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(format!("open {}: {e}", path.display())),
    };
    let reader = BufReader::new(file);
    let mut replayed: usize = 0;
    for (idx, line_result) in reader.lines().enumerate() {
        let line = match line_result {
            Ok(l) => l,
            Err(e) => {
                log::warn!(
                    "drafts.wal: read error at line {}: {e}; remaining entries skipped",
                    idx + 1,
                );
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let entry: WalEntry = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(e) => {
                log::warn!(
                    "drafts.wal: skipping unparseable line {}: {e}",
                    idx + 1,
                );
                continue;
            }
        };
        if let Err(e) = db_save_local_draft_sync(conn, &entry.params) {
            log::warn!(
                "drafts.wal: replay of draft {} failed: {e}; continuing",
                entry.params.id,
            );
            continue;
        }
        replayed += 1;
    }
    let target = rotated_path(data_dir);
    if let Err(e) = fs::rename(&path, &target) {
        log::warn!(
            "drafts.wal: rotate {} -> {} failed: {e}; the next boot will replay these \
             entries again (idempotent)",
            path.display(),
            target.display(),
        );
    }
    Ok(replayed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use db::db::Connection;
    use std::io::Write;

    fn fresh_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open conn");
        conn.execute_batch(
            "CREATE TABLE local_drafts (
                id TEXT PRIMARY KEY,
                account_id TEXT NOT NULL,
                to_addresses TEXT,
                cc_addresses TEXT,
                bcc_addresses TEXT,
                subject TEXT,
                body_html TEXT,
                reply_to_message_id TEXT,
                thread_id TEXT,
                from_email TEXT,
                signature_id TEXT,
                remote_draft_id TEXT,
                attachments TEXT,
                signature_separator_index INTEGER,
                updated_at INTEGER NOT NULL,
                sync_status TEXT NOT NULL
            )",
        )
        .expect("create local_drafts");
        conn
    }

    fn fresh_params(id: &str, subject: &str) -> SaveLocalDraftParams {
        SaveLocalDraftParams {
            id: id.to_string(),
            account_id: "acct-1".to_string(),
            to_addresses: None,
            cc_addresses: None,
            bcc_addresses: None,
            subject: Some(subject.to_string()),
            body_html: None,
            reply_to_message_id: None,
            thread_id: None,
            from_email: None,
            signature_id: None,
            remote_draft_id: None,
            attachments: None,
            signature_separator_index: None,
        }
    }

    fn write_wal_lines(data_dir: &Path, entries: &[WalEntry]) {
        let path = wal_path(data_dir);
        let mut f = std::fs::File::create(&path).expect("create wal");
        for entry in entries {
            let line = serde_json::to_string(entry).expect("ser");
            writeln!(f, "{line}").expect("write");
        }
        f.sync_all().expect("sync");
    }

    fn count_drafts(conn: &Connection) -> i64 {
        conn.query_row("SELECT count(*) FROM local_drafts", [], |row| row.get(0))
            .expect("count")
    }

    fn subject_of(conn: &Connection, id: &str) -> Option<String> {
        conn.query_row(
            "SELECT subject FROM local_drafts WHERE id = ?",
            [id],
            |row| row.get(0),
        )
        .ok()
    }

    #[test]
    fn drain_replays_each_entry_into_local_drafts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let entries = vec![
            WalEntry {
                epoch_ms: 1,
                params: fresh_params("draft-a", "subj-a"),
            },
            WalEntry {
                epoch_ms: 2,
                params: fresh_params("draft-b", "subj-b"),
            },
        ];
        write_wal_lines(dir.path(), &entries);
        let conn = fresh_conn();
        let replayed = drain(&conn, dir.path()).expect("drain");
        assert_eq!(replayed, 2);
        assert_eq!(count_drafts(&conn), 2);
        assert_eq!(subject_of(&conn, "draft-a").as_deref(), Some("subj-a"));
        assert_eq!(subject_of(&conn, "draft-b").as_deref(), Some("subj-b"));
        assert!(!wal_path(dir.path()).exists(), "active WAL must be rotated");
    }

    #[test]
    fn drain_with_no_wal_file_is_ok() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = fresh_conn();
        let replayed = drain(&conn, dir.path()).expect("drain empty");
        assert_eq!(replayed, 0);
    }

    #[test]
    fn drain_skips_unparseable_line_and_replays_the_rest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = wal_path(dir.path());
        let mut f = std::fs::File::create(&path).expect("create");
        let good = WalEntry {
            epoch_ms: 1,
            params: fresh_params("draft-good", "subj"),
        };
        writeln!(f, "{}", serde_json::to_string(&good).expect("ser")).expect("write");
        writeln!(f, "this is not valid json").expect("write garbage");
        let good2 = WalEntry {
            epoch_ms: 2,
            params: fresh_params("draft-good-2", "subj-2"),
        };
        writeln!(f, "{}", serde_json::to_string(&good2).expect("ser")).expect("write");
        f.sync_all().expect("sync");

        let conn = fresh_conn();
        let replayed = drain(&conn, dir.path()).expect("drain");
        assert_eq!(replayed, 2);
        assert_eq!(count_drafts(&conn), 2);
    }

    #[test]
    fn drain_is_idempotent_under_partial_replay() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = fresh_conn();

        // First drain: WAL has 2 entries, both replay.
        let entries = vec![
            WalEntry {
                epoch_ms: 1,
                params: fresh_params("dup", "first"),
            },
            WalEntry {
                epoch_ms: 2,
                params: fresh_params("new", "first-new"),
            },
        ];
        write_wal_lines(dir.path(), &entries);
        let first = drain(&conn, dir.path()).expect("first drain");
        assert_eq!(first, 2);

        // Crash and reboot: simulate a fresh WAL containing the same
        // first entry plus a new third one. The first entry should
        // UPSERT (not duplicate), the third should insert.
        let entries = vec![
            WalEntry {
                epoch_ms: 3,
                params: fresh_params("dup", "second"),
            },
            WalEntry {
                epoch_ms: 4,
                params: fresh_params("third", "third-new"),
            },
        ];
        write_wal_lines(dir.path(), &entries);
        let second = drain(&conn, dir.path()).expect("second drain");
        assert_eq!(second, 2);

        assert_eq!(count_drafts(&conn), 3);
        assert_eq!(subject_of(&conn, "dup").as_deref(), Some("second"));
        assert_eq!(subject_of(&conn, "new").as_deref(), Some("first-new"));
        assert_eq!(subject_of(&conn, "third").as_deref(), Some("third-new"));
    }
}
