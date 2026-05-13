//! `PackStore`: content-addressed pack-file blob store.
//!
//! Phase 2 of the attachments roadmap (see
//! `docs/attachments/problem-statement.md` and
//! `docs/attachments/implementation-roadmap.md`). Library only - this
//! module has no consumers yet; Phase 3 wires it into the Service.
//!
//! ## Layout
//!
//! Each pack file is an append-only segment under
//! `<packs_dir>/data-NNNNNN.pack` (sealed) or `data-NNNNNN.pack.open`
//! (currently being written). Bytes are written in frames:
//!
//! ```text
//! +--------+--------+----------+-----------------+
//! | magic  | length | xxh3_64  | payload bytes...|
//! | 4 B    | 4 B    | 8 B (LE) |   length B      |
//! +--------+--------+----------+-----------------+
//! ```
//!
//! When a pack rotates (exceeds the target size) a `TAIL` frame is
//! appended:
//!
//! ```text
//! +--------+---------+-------------+--------+
//! | magic  | version | frame_count | crc32  |
//! | 4 B    | 1 B     | 4 B (u32 LE)| 4 B    |
//! +--------+---------+-------------+--------+
//! ```
//!
//! The on-disk SQLite index lives in `attachment_blobs` (see
//! `02_mail.sql`). It carries the (`pack_file_id`, `offset`, `length`)
//! triple for each live blob plus a `tombstoned_at` column for
//! logically-evicted blobs.
//!
//! ## Crash safety
//!
//! - Sealed packs are immutable.
//! - The open pack is recovered on `open()`: any trailing partial frame
//!   is truncated; any committed-to-disk frame that lacks an index
//!   entry is re-indexed.
//! - Tombstones are recorded in `tombstones-NNNNNN.log` so they can be
//!   replayed if the SQLite index is lost.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, params};
use tokio::sync::Mutex as AsyncMutex;
use xxhash_rust::xxh3::xxh3_64;

use db::blob_hash::BlobHash;

pub const FRAME_MAGIC: [u8; 4] = *b"RTSK";
pub const TAIL_MAGIC: [u8; 4] = *b"TAIL";
pub const PACK_FORMAT_VERSION: u8 = 1;

/// Frame header byte count: `magic(4) + length(4) + xxh3(8)`.
pub const FRAME_HEADER_LEN: usize = 16;

/// Tail byte count: `magic(4) + version(1) + frame_count(4) + crc32(4)`.
pub const TAIL_LEN: usize = 13;

/// Default `target_size` for `PackStore::open`. Pack files rotate when
/// the next write would push them past this.
pub const DEFAULT_PACK_TARGET_SIZE: u64 = 256 * 1024 * 1024;

/// Per-frame payload size cap (matches the 4-byte length field).
pub const MAX_FRAME_PAYLOAD: u64 = u32::MAX as u64;

#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("sql: {0}")]
    Sql(String),
    #[error("corruption: {0}")]
    Corruption(String),
    #[error("payload too large: {0} bytes")]
    TooLarge(u64),
}

impl From<rusqlite::Error> for PackError {
    fn from(e: rusqlite::Error) -> Self {
        PackError::Sql(e.to_string())
    }
}

#[derive(Debug, Default, Clone)]
pub struct GcStats {
    pub packs_compacted: u32,
    pub blobs_dropped: u64,
    pub bytes_reclaimed: u64,
}

pub struct PackStore {
    inner: Arc<Inner>,
}

struct Inner {
    packs_dir: PathBuf,
    conn: Arc<Mutex<Connection>>,
    target_size: u64,
    writer: AsyncMutex<OpenPack>,
}

struct OpenPack {
    pack_id: u32,
    file: File,
    offset: u64,
}

impl PackStore {
    /// Open (or create) a `PackStore` rooted at `packs_dir` with its
    /// index in `conn`. Runs recovery: opens the highest-numbered
    /// `.open` pack, truncates any torn trailing frame, and registers
    /// any frame that lacks an index entry.
    pub async fn open(
        packs_dir: PathBuf,
        conn: Arc<Mutex<Connection>>,
        target_size: u64,
    ) -> Result<Self, PackError> {
        let dir = packs_dir.clone();
        let conn_clone = Arc::clone(&conn);
        let open_pack = tokio::task::spawn_blocking(move || -> Result<OpenPack, PackError> {
            fs::create_dir_all(&dir)?;
            recover_and_open_current_pack(&dir, &conn_clone)
        })
        .await
        .map_err(|e| PackError::Sql(format!("spawn_blocking open: {e}")))??;

        Ok(Self {
            inner: Arc::new(Inner {
                packs_dir,
                conn,
                target_size,
                writer: AsyncMutex::new(open_pack),
            }),
        })
    }

    /// `flush` fsyncs the currently-open pack file. Useful for tests
    /// and clean-shutdown sentinel ordering.
    pub async fn flush(&self) -> Result<(), PackError> {
        let writer = self.inner.writer.lock().await;
        let file = writer.file.try_clone()?;
        tokio::task::spawn_blocking(move || file.sync_all())
            .await
            .map_err(|e| PackError::Sql(format!("spawn_blocking flush: {e}")))??;
        Ok(())
    }

    /// Append `bytes` to the current pack if not already stored. The
    /// content hash (BLAKE3) is the identity; two calls with the same
    /// bytes return the same hash and only the first writes a frame.
    pub async fn put(&self, bytes: Vec<u8>) -> Result<BlobHash, PackError> {
        let hash = BlobHash::hash(&bytes);
        let payload_len: u64 = bytes.len() as u64;
        if payload_len > MAX_FRAME_PAYLOAD {
            return Err(PackError::TooLarge(payload_len));
        }

        // Fast-path dedup: if the blob is already indexed, no work to do.
        let conn = Arc::clone(&self.inner.conn);
        let hash_for_check = hash;
        let already_present = tokio::task::spawn_blocking(move || -> Result<bool, PackError> {
            let conn = conn
                .lock()
                .map_err(|e| PackError::Sql(format!("conn poisoned: {e}")))?;
            let exists: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM attachment_blobs WHERE content_hash = ?1",
                    params![hash_for_check],
                    |r| r.get(0),
                )
                .ok();
            Ok(exists.is_some())
        })
        .await
        .map_err(|e| PackError::Sql(format!("spawn_blocking put-dedup: {e}")))??;
        if already_present {
            return Ok(hash);
        }

        // Acquire the writer.
        let inner = Arc::clone(&self.inner);
        let hash_for_write = hash;
        tokio::task::spawn_blocking(move || -> Result<(), PackError> {
            let mut writer = inner.writer.blocking_lock();

            // Rotate if this write would push us past the target.
            let frame_len = FRAME_HEADER_LEN as u64 + payload_len;
            if writer.offset > 0
                && writer.offset.saturating_add(frame_len) > inner.target_size
            {
                rotate_pack(&inner.packs_dir, &mut writer)?;
            }

            let frame_offset = writer.offset;
            let checksum = xxh3_64(&bytes);
            // payload_len fits in u32: checked against MAX_FRAME_PAYLOAD above.
            #[allow(clippy::cast_possible_truncation)]
            let length_u32 = payload_len as u32;
            let mut header = [0u8; FRAME_HEADER_LEN];
            header[..4].copy_from_slice(&FRAME_MAGIC);
            header[4..8].copy_from_slice(&length_u32.to_le_bytes());
            header[8..16].copy_from_slice(&checksum.to_le_bytes());

            writer.file.write_all(&header)?;
            writer.file.write_all(&bytes)?;
            writer.file.sync_all()?;

            let conn = inner
                .conn
                .lock()
                .map_err(|e| PackError::Sql(format!("conn poisoned: {e}")))?;
            #[allow(clippy::cast_possible_wrap)]
            let pack_id_i64 = i64::from(writer.pack_id);
            #[allow(clippy::cast_possible_wrap)]
            let offset_i64 = frame_offset as i64;
            #[allow(clippy::cast_possible_wrap)]
            let length_i64 = payload_len as i64;
            conn.execute(
                "INSERT OR IGNORE INTO attachment_blobs \
                 (content_hash, pack_file_id, offset, length, written_at) \
                 VALUES (?1, ?2, ?3, ?4, unixepoch())",
                params![hash_for_write, pack_id_i64, offset_i64, length_i64],
            )?;

            writer.offset = writer.offset.saturating_add(frame_len);
            Ok(())
        })
        .await
        .map_err(|e| PackError::Sql(format!("spawn_blocking put-write: {e}")))??;

        Ok(hash)
    }

    /// Fetch the bytes for `hash`. Returns `Ok(None)` for misses or
    /// tombstoned blobs. Verifies the frame payload checksum on read.
    pub async fn get(&self, hash: &BlobHash) -> Result<Option<Vec<u8>>, PackError> {
        let hash = *hash;
        let inner = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || -> Result<Option<Vec<u8>>, PackError> {
            // SQL lookup.
            let (pack_id, offset, length, tombstoned) = {
                let conn = inner
                    .conn
                    .lock()
                    .map_err(|e| PackError::Sql(format!("conn poisoned: {e}")))?;
                let row: Option<(i64, i64, i64, Option<i64>)> = conn
                    .query_row(
                        "SELECT pack_file_id, offset, length, tombstoned_at \
                         FROM attachment_blobs WHERE content_hash = ?1",
                        params![hash],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                    )
                    .ok();
                match row {
                    Some(row) => row,
                    None => return Ok(None),
                }
            };
            if tombstoned.is_some() {
                return Ok(None);
            }

            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let pack_id_u32 = pack_id as u32;
            #[allow(clippy::cast_sign_loss)]
            let offset_u64 = offset as u64;
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let length_usize = length as usize;

            let bytes = read_frame_payload(
                &inner.packs_dir,
                pack_id_u32,
                offset_u64,
                length_usize,
            )?;

            // Bump last_read_at, best-effort.
            if let Ok(conn) = inner.conn.lock() {
                let _ = conn.execute(
                    "UPDATE attachment_blobs SET last_read_at = unixepoch() \
                     WHERE content_hash = ?1",
                    params![hash],
                );
            }

            Ok(Some(bytes))
        })
        .await
        .map_err(|e| PackError::Sql(format!("spawn_blocking get: {e}")))?
    }

    /// Mark `hash` as tombstoned (logically evicted). Idempotent. The
    /// tombstone is also recorded in the per-pack `tombstones-NNNNNN.log`
    /// so the index can be rebuilt if it is lost.
    pub async fn tombstone(&self, hash: &BlobHash) -> Result<(), PackError> {
        let hash = *hash;
        let inner = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || -> Result<(), PackError> {
            // First: find which pack the blob lives in.
            let pack_id: Option<u32> = {
                let conn = inner
                    .conn
                    .lock()
                    .map_err(|e| PackError::Sql(format!("conn poisoned: {e}")))?;
                conn.query_row(
                    "SELECT pack_file_id FROM attachment_blobs \
                     WHERE content_hash = ?1 AND tombstoned_at IS NULL",
                    params![hash],
                    |r| r.get::<_, i64>(0),
                )
                .ok()
                .map(|p| {
                    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                    let p_u32 = p as u32;
                    p_u32
                })
            };

            let Some(pack_id) = pack_id else {
                // Already tombstoned or missing - idempotent no-op.
                return Ok(());
            };

            {
                let conn = inner
                    .conn
                    .lock()
                    .map_err(|e| PackError::Sql(format!("conn poisoned: {e}")))?;
                conn.execute(
                    "UPDATE attachment_blobs SET tombstoned_at = unixepoch() \
                     WHERE content_hash = ?1 AND tombstoned_at IS NULL",
                    params![hash],
                )?;
            }

            append_tombstone_log(&inner.packs_dir, pack_id, &hash)?;
            Ok(())
        })
        .await
        .map_err(|e| PackError::Sql(format!("spawn_blocking tombstone: {e}")))??;
        Ok(())
    }

    /// Compact every sealed pack whose `dead / total` ratio is at or
    /// above `density_threshold`. Live frames are copied to a fresh
    /// pack at the chain tail; the old pack is unlinked after the
    /// index swap commits.
    pub async fn gc(&self, density_threshold: f32) -> Result<GcStats, PackError> {
        let inner = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || -> Result<GcStats, PackError> {
            run_gc(&inner, density_threshold)
        })
        .await
        .map_err(|e| PackError::Sql(format!("spawn_blocking gc: {e}")))?
    }
}

// ── helpers ─────────────────────────────────────────────────────────

fn pack_path_sealed(packs_dir: &Path, pack_id: u32) -> PathBuf {
    packs_dir.join(format!("data-{pack_id:06}.pack"))
}

fn pack_path_open(packs_dir: &Path, pack_id: u32) -> PathBuf {
    packs_dir.join(format!("data-{pack_id:06}.pack.open"))
}

fn tombstone_log_path(packs_dir: &Path, pack_id: u32) -> PathBuf {
    packs_dir.join(format!("tombstones-{pack_id:06}.log"))
}

/// Enumerate every pack in `packs_dir`. Returns `(pack_id, is_sealed)`
/// pairs sorted ascending by `pack_id`.
fn list_packs(packs_dir: &Path) -> Result<Vec<(u32, bool)>, PackError> {
    let mut out: Vec<(u32, bool)> = Vec::new();
    for entry in fs::read_dir(packs_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with("data-") {
            continue;
        }
        let (sealed, stem) = if let Some(rest) = name.strip_suffix(".pack.open") {
            (false, rest)
        } else if let Some(rest) = name.strip_suffix(".pack") {
            (true, rest)
        } else {
            continue;
        };
        let id_str = stem.strip_prefix("data-").unwrap_or(stem);
        let Ok(id) = id_str.parse::<u32>() else {
            continue;
        };
        out.push((id, sealed));
    }
    out.sort_by_key(|&(id, _)| id);
    Ok(out)
}

/// Find the next pack_id to open for writing, and recover any torn
/// trailing frame in the current open pack.
fn recover_and_open_current_pack(
    packs_dir: &Path,
    conn: &Arc<Mutex<Connection>>,
) -> Result<OpenPack, PackError> {
    let packs = list_packs(packs_dir)?;
    let next_pack_id = match packs.last() {
        None => 0,
        Some(&(id, true)) => id.saturating_add(1),
        Some(&(id, false)) => {
            // Recover the open pack in place.
            recover_open_pack(packs_dir, id, conn)?;
            return open_existing_open_pack(packs_dir, id);
        }
    };
    create_new_open_pack(packs_dir, next_pack_id)
}

/// Open an existing `.open` pack at its end-of-file position.
fn open_existing_open_pack(packs_dir: &Path, pack_id: u32) -> Result<OpenPack, PackError> {
    let path = pack_path_open(packs_dir, pack_id);
    let mut file = OpenOptions::new().read(true).write(true).open(&path)?;
    let offset = file.seek(SeekFrom::End(0))?;
    // Touch the tombstone log so it exists before any tombstone fires.
    let _ = OpenOptions::new()
        .create(true)
        .append(true)
        .open(tombstone_log_path(packs_dir, pack_id))?;
    Ok(OpenPack {
        pack_id,
        file,
        offset,
    })
}

/// Create a brand-new `.open` pack.
fn create_new_open_pack(packs_dir: &Path, pack_id: u32) -> Result<OpenPack, PackError> {
    let path = pack_path_open(packs_dir, pack_id);
    let file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&path)?;
    file.sync_all()?;
    let _ = OpenOptions::new()
        .create(true)
        .append(true)
        .open(tombstone_log_path(packs_dir, pack_id))?;
    Ok(OpenPack {
        pack_id,
        file,
        offset: 0,
    })
}

/// Walk an `.open` pack from offset 0:
/// - Truncate the file at the offset of any torn trailing frame.
/// - For every fully-written frame, ensure an `attachment_blobs` row
///   exists pointing at it.
fn recover_open_pack(
    packs_dir: &Path,
    pack_id: u32,
    conn: &Arc<Mutex<Connection>>,
) -> Result<(), PackError> {
    let path = pack_path_open(packs_dir, pack_id);
    let mut file = OpenOptions::new().read(true).write(true).open(&path)?;
    let file_len = file.seek(SeekFrom::End(0))?;
    file.seek(SeekFrom::Start(0))?;

    let mut offset: u64 = 0;
    let conn_locked = conn
        .lock()
        .map_err(|e| PackError::Sql(format!("conn poisoned: {e}")))?;

    loop {
        // Try to read a frame header.
        let remaining = file_len.saturating_sub(offset);
        if remaining == 0 {
            break;
        }
        if remaining < FRAME_HEADER_LEN as u64 {
            // Partial header - truncate.
            file.set_len(offset)?;
            file.sync_all()?;
            log::warn!(
                "PackStore::recover: truncated partial header in pack {pack_id} at offset {offset}",
            );
            break;
        }
        let mut header = [0u8; FRAME_HEADER_LEN];
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut header)?;
        if header[..4] != FRAME_MAGIC {
            // Garbage at this position - truncate.
            file.set_len(offset)?;
            file.sync_all()?;
            log::warn!(
                "PackStore::recover: truncated bad magic in pack {pack_id} at offset {offset}",
            );
            break;
        }
        let length = u64::from(read_u32_le(&header[4..8]));
        let checksum = read_u64_le(&header[8..16]);
        let frame_total = FRAME_HEADER_LEN as u64 + length;
        if remaining < frame_total {
            // Partial payload - truncate the header too.
            file.set_len(offset)?;
            file.sync_all()?;
            log::warn!(
                "PackStore::recover: truncated partial payload in pack {pack_id} at offset {offset}",
            );
            break;
        }
        // Read the payload to verify the checksum.
        #[allow(clippy::cast_possible_truncation)]
        let mut payload = vec![0u8; length as usize];
        file.read_exact(&mut payload)?;
        let actual = xxh3_64(&payload);
        if actual != checksum {
            // Corrupt frame - truncate from here.
            file.set_len(offset)?;
            file.sync_all()?;
            log::warn!(
                "PackStore::recover: truncated bad checksum in pack {pack_id} at offset {offset}",
            );
            break;
        }
        // Ensure an index entry exists.
        let hash = BlobHash::hash(&payload);
        #[allow(clippy::cast_possible_wrap)]
        let pack_id_i64 = i64::from(pack_id);
        #[allow(clippy::cast_possible_wrap)]
        let offset_i64 = offset as i64;
        #[allow(clippy::cast_possible_wrap)]
        let length_i64 = length as i64;
        conn_locked.execute(
            "INSERT OR IGNORE INTO attachment_blobs \
             (content_hash, pack_file_id, offset, length, written_at) \
             VALUES (?1, ?2, ?3, ?4, unixepoch())",
            params![hash, pack_id_i64, offset_i64, length_i64],
        )?;
        offset = offset.saturating_add(frame_total);
    }
    Ok(())
}

/// Seal the current open pack (write tail + rename) and open the next one.
fn rotate_pack(packs_dir: &Path, open: &mut OpenPack) -> Result<(), PackError> {
    seal_open_pack(packs_dir, open)?;
    let next_id = open.pack_id.saturating_add(1);
    let new_open = create_new_open_pack(packs_dir, next_id)?;
    *open = new_open;
    Ok(())
}

fn seal_open_pack(packs_dir: &Path, open: &mut OpenPack) -> Result<(), PackError> {
    let frame_count = frame_count_in_open_pack(&mut open.file, open.offset)?;
    let mut tail = [0u8; TAIL_LEN];
    tail[..4].copy_from_slice(&TAIL_MAGIC);
    tail[4] = PACK_FORMAT_VERSION;
    tail[5..9].copy_from_slice(&frame_count.to_le_bytes());
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&tail[4..9]);
    let crc = hasher.finalize();
    tail[9..13].copy_from_slice(&crc.to_le_bytes());

    open.file.seek(SeekFrom::Start(open.offset))?;
    open.file.write_all(&tail)?;
    open.file.sync_all()?;

    let open_path = pack_path_open(packs_dir, open.pack_id);
    let sealed_path = pack_path_sealed(packs_dir, open.pack_id);
    fs::rename(&open_path, &sealed_path)?;
    Ok(())
}

fn frame_count_in_open_pack(file: &mut File, end_offset: u64) -> Result<u32, PackError> {
    let mut offset = 0u64;
    let mut count: u32 = 0;
    while offset < end_offset {
        let mut header = [0u8; FRAME_HEADER_LEN];
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut header)?;
        let length = u64::from(read_u32_le(&header[4..8]));
        offset = offset.saturating_add(FRAME_HEADER_LEN as u64 + length);
        count = count.saturating_add(1);
    }
    Ok(count)
}

/// Read the payload bytes of a frame at `(pack_id, offset, length)`.
/// Verifies magic + length + checksum.
fn read_frame_payload(
    packs_dir: &Path,
    pack_id: u32,
    offset: u64,
    length: usize,
) -> Result<Vec<u8>, PackError> {
    let sealed = pack_path_sealed(packs_dir, pack_id);
    let open = pack_path_open(packs_dir, pack_id);
    let mut file = match File::open(&sealed) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => File::open(&open)?,
        Err(e) => return Err(PackError::Io(e)),
    };
    file.seek(SeekFrom::Start(offset))?;
    let mut header = [0u8; FRAME_HEADER_LEN];
    file.read_exact(&mut header)?;
    if header[..4] != FRAME_MAGIC {
        return Err(PackError::Corruption(format!(
            "bad magic in pack {pack_id} at offset {offset}"
        )));
    }
    let header_length = read_u32_le(&header[4..8]) as usize;
    if header_length != length {
        return Err(PackError::Corruption(format!(
            "length mismatch: header={header_length} index={length}"
        )));
    }
    let checksum = read_u64_le(&header[8..16]);
    let mut payload = vec![0u8; length];
    file.read_exact(&mut payload)?;
    let actual = xxh3_64(&payload);
    if actual != checksum {
        return Err(PackError::Corruption(format!(
            "checksum mismatch in pack {pack_id} at offset {offset}"
        )));
    }
    Ok(payload)
}

/// Read a little-endian `u32` from a 4-byte slice. Caller must pass
/// at least 4 bytes; we panic-free by saturating to zero on undersize.
fn read_u32_le(bytes: &[u8]) -> u32 {
    let mut buf = [0u8; 4];
    let n = bytes.len().min(4);
    buf[..n].copy_from_slice(&bytes[..n]);
    u32::from_le_bytes(buf)
}

/// Read a little-endian `u64` from an 8-byte slice. Same convention.
fn read_u64_le(bytes: &[u8]) -> u64 {
    let mut buf = [0u8; 8];
    let n = bytes.len().min(8);
    buf[..n].copy_from_slice(&bytes[..n]);
    u64::from_le_bytes(buf)
}

fn append_tombstone_log(
    packs_dir: &Path,
    pack_id: u32,
    hash: &BlobHash,
) -> Result<(), PackError> {
    let path = tombstone_log_path(packs_dir, pack_id);
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    let mut record = [0u8; 48];
    // Little-endian u32 for the pack id (4 bytes), zero pad to 16 to leave
    // room for a future pack ID widening. The hash takes the trailing 32.
    record[..4].copy_from_slice(&pack_id.to_le_bytes());
    record[16..48].copy_from_slice(hash.as_bytes());
    file.write_all(&record)?;
    file.sync_all()?;
    Ok(())
}

// ── GC ──────────────────────────────────────────────────────────────

fn run_gc(inner: &Inner, density_threshold: f32) -> Result<GcStats, PackError> {
    let mut stats = GcStats::default();
    // Hold the writer mutex for the whole GC pass in v1 (see plan).
    let mut writer = inner.writer.blocking_lock();

    let packs = list_packs(&inner.packs_dir)?;
    for (pack_id, sealed) in packs {
        if !sealed {
            continue;
        }
        let (total, dead, dead_bytes) = pack_density(&inner.conn, pack_id)?;
        if total == 0 {
            continue;
        }
        #[allow(clippy::cast_precision_loss)]
        let ratio = dead as f32 / total as f32;
        if ratio < density_threshold {
            continue;
        }
        compact_pack(inner, &mut writer, pack_id)?;
        stats.packs_compacted = stats.packs_compacted.saturating_add(1);
        stats.blobs_dropped = stats.blobs_dropped.saturating_add(dead);
        #[allow(clippy::cast_sign_loss)]
        let dead_bytes_u64 = dead_bytes as u64;
        stats.bytes_reclaimed = stats.bytes_reclaimed.saturating_add(dead_bytes_u64);
    }

    Ok(stats)
}

fn pack_density(
    conn: &Arc<Mutex<Connection>>,
    pack_id: u32,
) -> Result<(u64, u64, i64), PackError> {
    let conn = conn
        .lock()
        .map_err(|e| PackError::Sql(format!("conn poisoned: {e}")))?;
    #[allow(clippy::cast_possible_wrap)]
    let pack_id_i64 = i64::from(pack_id);
    let (total, dead, dead_bytes): (i64, i64, Option<i64>) = conn.query_row(
        "SELECT \
            COUNT(*), \
            COALESCE(SUM(CASE WHEN tombstoned_at IS NOT NULL THEN 1 ELSE 0 END), 0), \
            COALESCE(SUM(CASE WHEN tombstoned_at IS NOT NULL THEN length ELSE 0 END), 0) \
         FROM attachment_blobs WHERE pack_file_id = ?1",
        params![pack_id_i64],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    #[allow(clippy::cast_sign_loss)]
    let total_u64 = total as u64;
    #[allow(clippy::cast_sign_loss)]
    let dead_u64 = dead as u64;
    Ok((total_u64, dead_u64, dead_bytes.unwrap_or(0)))
}

fn compact_pack(
    inner: &Inner,
    writer: &mut OpenPack,
    src_pack_id: u32,
) -> Result<(), PackError> {
    // Allocate the destination pack at the chain tail.
    let dst_pack_id = writer.pack_id.saturating_add(1);
    let dst_path = pack_path_open(&inner.packs_dir, dst_pack_id);
    let mut dst_file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&dst_path)?;

    // Collect live (hash, src_offset, length) for the source pack.
    let live: Vec<(BlobHash, u64, usize)> = {
        let conn = inner
            .conn
            .lock()
            .map_err(|e| PackError::Sql(format!("conn poisoned: {e}")))?;
        #[allow(clippy::cast_possible_wrap)]
        let src_pack_i64 = i64::from(src_pack_id);
        let mut stmt = conn.prepare(
            "SELECT content_hash, offset, length FROM attachment_blobs \
             WHERE pack_file_id = ?1 AND tombstoned_at IS NULL",
        )?;
        let rows = stmt
            .query_map(params![src_pack_i64], |r| {
                let hash: BlobHash = r.get(0)?;
                let offset: i64 = r.get(1)?;
                let length: i64 = r.get(2)?;
                Ok((hash, offset, length))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(h, o, l)| {
                #[allow(clippy::cast_sign_loss)]
                let o_u64 = o as u64;
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                let l_usize = l as usize;
                (h, o_u64, l_usize)
            })
            .collect()
    };

    let src_path = pack_path_sealed(&inner.packs_dir, src_pack_id);
    let mut src_file = File::open(&src_path)?;

    // Copy live frames forward.
    let mut new_offsets: Vec<(BlobHash, u64, usize)> = Vec::with_capacity(live.len());
    let mut dst_offset: u64 = 0;
    let mut frame_count: u32 = 0;
    for (hash, src_offset, length) in live {
        let mut header = [0u8; FRAME_HEADER_LEN];
        src_file.seek(SeekFrom::Start(src_offset))?;
        src_file.read_exact(&mut header)?;
        if header[..4] != FRAME_MAGIC {
            return Err(PackError::Corruption(format!(
                "GC: bad magic in source pack {src_pack_id} at offset {src_offset}"
            )));
        }
        let mut payload = vec![0u8; length];
        src_file.read_exact(&mut payload)?;
        dst_file.write_all(&header)?;
        dst_file.write_all(&payload)?;
        new_offsets.push((hash, dst_offset, length));
        dst_offset =
            dst_offset.saturating_add(FRAME_HEADER_LEN as u64 + length as u64);
        frame_count = frame_count.saturating_add(1);
    }

    // Seal the destination pack.
    let mut tail = [0u8; TAIL_LEN];
    tail[..4].copy_from_slice(&TAIL_MAGIC);
    tail[4] = PACK_FORMAT_VERSION;
    tail[5..9].copy_from_slice(&frame_count.to_le_bytes());
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&tail[4..9]);
    let crc = hasher.finalize();
    tail[9..13].copy_from_slice(&crc.to_le_bytes());
    dst_file.write_all(&tail)?;
    dst_file.sync_all()?;
    drop(dst_file);
    let dst_sealed = pack_path_sealed(&inner.packs_dir, dst_pack_id);
    fs::rename(&dst_path, &dst_sealed)?;

    // Index swap + dead-row delete inside one transaction.
    {
        let mut conn = inner
            .conn
            .lock()
            .map_err(|e| PackError::Sql(format!("conn poisoned: {e}")))?;
        let tx = conn.transaction()?;
        #[allow(clippy::cast_possible_wrap)]
        let dst_pack_i64 = i64::from(dst_pack_id);
        #[allow(clippy::cast_possible_wrap)]
        let src_pack_i64 = i64::from(src_pack_id);
        {
            let mut update = tx.prepare(
                "UPDATE attachment_blobs SET pack_file_id = ?1, offset = ?2 \
                 WHERE content_hash = ?3",
            )?;
            for (hash, dst_offset, _length) in &new_offsets {
                #[allow(clippy::cast_possible_wrap)]
                let dst_offset_i64 = *dst_offset as i64;
                update.execute(params![dst_pack_i64, dst_offset_i64, hash])?;
            }
        }
        tx.execute(
            "DELETE FROM attachment_blobs \
             WHERE pack_file_id = ?1 AND tombstoned_at IS NOT NULL",
            params![src_pack_i64],
        )?;
        tx.commit()?;
    }

    // Unlink the old source pack (and its tombstone log - entries
    // there only matter when the index is missing; the entries we
    // just deleted from the index are also gone from any live SQL
    // query, so the log copies are dead weight).
    let _ = fs::remove_file(&src_path);
    let _ = fs::remove_file(tombstone_log_path(&inner.packs_dir, src_pack_id));

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn schema_sql() -> &'static str {
        "CREATE TABLE attachment_blobs (\
            content_hash  BLOB    PRIMARY KEY,\
            pack_file_id  INTEGER NOT NULL,\
            offset        INTEGER NOT NULL,\
            length        INTEGER NOT NULL,\
            written_at    INTEGER NOT NULL,\
            last_read_at  INTEGER,\
            tombstoned_at INTEGER\
         );\
         CREATE INDEX idx_attachment_blobs_tombstoned \
            ON attachment_blobs(tombstoned_at);"
    }

    fn new_conn() -> Arc<Mutex<Connection>> {
        let conn = Connection::open_in_memory().expect("conn");
        conn.execute_batch(schema_sql()).expect("schema");
        Arc::new(Mutex::new(conn))
    }

    async fn fresh_store(target_size: u64) -> (TempDir, PackStore) {
        let dir = TempDir::new().expect("tempdir");
        let store = PackStore::open(dir.path().to_path_buf(), new_conn(), target_size)
            .await
            .expect("open");
        (dir, store)
    }

    #[tokio::test]
    async fn roundtrip_small() {
        let (_dir, store) = fresh_store(DEFAULT_PACK_TARGET_SIZE).await;
        let bytes = b"hello pack store".to_vec();
        let hash = store.put(bytes.clone()).await.expect("put");
        let got = store.get(&hash).await.expect("get").expect("hit");
        assert_eq!(got, bytes);
    }

    #[tokio::test]
    async fn roundtrip_large() {
        let (_dir, store) = fresh_store(DEFAULT_PACK_TARGET_SIZE).await;
        let bytes = (0..5u32 * 1024 * 1024)
            .map(|i| u8::try_from(i & 0xff).unwrap_or(0))
            .collect::<Vec<u8>>();
        let hash = store.put(bytes.clone()).await.expect("put");
        let got = store.get(&hash).await.expect("get").expect("hit");
        assert_eq!(got.len(), bytes.len());
        assert!(got == bytes);
    }

    #[tokio::test]
    async fn dedup_idempotent() {
        let (dir, store) = fresh_store(DEFAULT_PACK_TARGET_SIZE).await;
        let bytes = b"dedup me".to_vec();
        let hash_a = store.put(bytes.clone()).await.expect("first");
        let len_after_first = fs::metadata(pack_path_open(dir.path(), 0)).unwrap().len();
        let hash_b = store.put(bytes.clone()).await.expect("second");
        let len_after_second = fs::metadata(pack_path_open(dir.path(), 0)).unwrap().len();
        assert_eq!(hash_a, hash_b);
        assert_eq!(len_after_first, len_after_second);
    }

    #[tokio::test]
    async fn dedup_under_race() {
        let (_dir, store) = fresh_store(DEFAULT_PACK_TARGET_SIZE).await;
        let store = Arc::new(store);
        let bytes = b"race".to_vec();
        let s1 = Arc::clone(&store);
        let s2 = Arc::clone(&store);
        let b1 = bytes.clone();
        let b2 = bytes.clone();
        let (a, b) = tokio::join!(
            tokio::spawn(async move { s1.put(b1).await }),
            tokio::spawn(async move { s2.put(b2).await }),
        );
        let hash_a = a.unwrap().expect("a");
        let hash_b = b.unwrap().expect("b");
        assert_eq!(hash_a, hash_b);
        let conn = Arc::clone(&store.inner.conn);
        let conn = conn.lock().unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM attachment_blobs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn tombstone_hides_blob() {
        let (_dir, store) = fresh_store(DEFAULT_PACK_TARGET_SIZE).await;
        let hash = store.put(b"goodbye".to_vec()).await.expect("put");
        store.tombstone(&hash).await.expect("tombstone");
        assert!(store.get(&hash).await.expect("get").is_none());
    }

    #[tokio::test]
    async fn tombstone_idempotent() {
        let (_dir, store) = fresh_store(DEFAULT_PACK_TARGET_SIZE).await;
        let hash = store.put(b"twice".to_vec()).await.expect("put");
        store.tombstone(&hash).await.expect("first");
        store.tombstone(&hash).await.expect("second");
    }

    #[tokio::test]
    async fn pack_rotation() {
        // Each frame = 16 B header + 4096 B payload = 4112 B.
        // Three frames at target_size=10000 forces two rotations.
        let (dir, store) = fresh_store(10_000).await;
        for i in 0u8..3 {
            let payload = vec![i; 4096];
            store.put(payload).await.expect("put");
        }
        let packs = list_packs(dir.path()).expect("list");
        assert!(packs.len() >= 2, "expected rotations: {packs:?}");
        // The earlier packs must be sealed (.pack) and the highest open (.open).
        let highest = packs.iter().map(|(id, _)| *id).max().unwrap();
        for (id, sealed) in &packs {
            if *id == highest {
                assert!(!sealed, "highest pack should be .open");
            } else {
                assert!(sealed, "earlier pack {id} should be sealed");
            }
        }
    }

    #[tokio::test]
    async fn recover_truncates_torn_frame() {
        let (dir, store) = fresh_store(DEFAULT_PACK_TARGET_SIZE).await;
        let h1 = store.put(b"a".to_vec()).await.expect("put1");
        let h2 = store.put(b"bb".to_vec()).await.expect("put2");
        let h3 = store.put(b"ccc".to_vec()).await.expect("put3");
        store.flush().await.expect("flush");
        // Drop the store handles (and the conn) so we can reopen with a fresh one.
        drop(store);
        // Append 8 garbage bytes to the open pack.
        let pack_path = pack_path_open(dir.path(), 0);
        let mut f = OpenOptions::new().append(true).open(&pack_path).unwrap();
        f.write_all(&[0xaa; 8]).unwrap();
        f.sync_all().unwrap();
        drop(f);
        // Reopen with a fresh empty index - recover should re-register
        // the three frames and truncate the garbage.
        let conn = new_conn();
        let store2 = PackStore::open(dir.path().to_path_buf(), conn, DEFAULT_PACK_TARGET_SIZE)
            .await
            .expect("reopen");
        assert_eq!(store2.get(&h1).await.unwrap().as_deref(), Some(&b"a"[..]));
        assert_eq!(store2.get(&h2).await.unwrap().as_deref(), Some(&b"bb"[..]));
        assert_eq!(store2.get(&h3).await.unwrap().as_deref(), Some(&b"ccc"[..]));
        let len_after = fs::metadata(&pack_path).unwrap().len();
        // 3 frames * (16 header + payload) = 16*3 + 1 + 2 + 3 = 54.
        assert_eq!(len_after, 54);
    }

    #[tokio::test]
    async fn recover_indexes_missing_entry() {
        // Reuse recover_truncates_torn_frame's path but with a clean
        // bytes-on-disk + empty index.
        let dir = TempDir::new().unwrap();
        let conn1 = new_conn();
        let store = PackStore::open(dir.path().to_path_buf(), conn1, DEFAULT_PACK_TARGET_SIZE)
            .await
            .unwrap();
        let h = store.put(b"reindex me".to_vec()).await.unwrap();
        store.flush().await.unwrap();
        drop(store);
        // Fresh index.
        let conn2 = new_conn();
        let store2 = PackStore::open(dir.path().to_path_buf(), conn2, DEFAULT_PACK_TARGET_SIZE)
            .await
            .unwrap();
        let got = store2.get(&h).await.unwrap().expect("recovered");
        assert_eq!(got, b"reindex me");
    }

    #[tokio::test]
    async fn gc_drops_tombstoned() {
        // Use a small target_size so the first three blobs roll over,
        // sealing pack 0, before we tombstone and GC it.
        let (dir, store) = fresh_store(60).await;
        let h1 = store.put(b"one".to_vec()).await.unwrap();
        let h2 = store.put(b"two".to_vec()).await.unwrap();
        let h3 = store.put(b"three".to_vec()).await.unwrap();
        let _h4 = store.put(b"four".to_vec()).await.unwrap();
        // h1, h2, h3 should be in pack 0 (sealed).
        // Tombstone h1, h2 - 2/3 = 67% dead.
        store.tombstone(&h1).await.unwrap();
        store.tombstone(&h2).await.unwrap();
        let stats = store.gc(0.5).await.unwrap();
        assert_eq!(stats.packs_compacted, 1, "should compact pack 0");
        assert_eq!(stats.blobs_dropped, 2);
        // h3 still readable, h1/h2 gone, original pack-0 file gone.
        assert_eq!(store.get(&h3).await.unwrap().as_deref(), Some(&b"three"[..]));
        assert!(store.get(&h1).await.unwrap().is_none());
        assert!(store.get(&h2).await.unwrap().is_none());
        assert!(!pack_path_sealed(dir.path(), 0).exists(), "old pack unlinked");
    }

    #[tokio::test]
    async fn gc_skips_low_density() {
        let (dir, store) = fresh_store(60).await;
        let h1 = store.put(b"one".to_vec()).await.unwrap();
        let _h2 = store.put(b"two".to_vec()).await.unwrap();
        let _h3 = store.put(b"three".to_vec()).await.unwrap();
        let _h4 = store.put(b"four".to_vec()).await.unwrap();
        // Tombstone just h1 - 1/3 ≈ 33% density.
        store.tombstone(&h1).await.unwrap();
        let before = fs::metadata(pack_path_sealed(dir.path(), 0)).unwrap().len();
        let stats = store.gc(0.5).await.unwrap();
        assert_eq!(stats.packs_compacted, 0);
        let after = fs::metadata(pack_path_sealed(dir.path(), 0)).unwrap().len();
        assert_eq!(before, after);
    }

    /// Library-level benchmark. Sanity baseline only - no Criterion.
    /// Run with `brokkr check -p store -- --include-ignored
    /// attachment_pack::tests::bench_pack_throughput --nocapture`.
    #[tokio::test]
    #[ignore = "benchmark; run explicitly"]
    async fn bench_pack_throughput() {
        let (dir, store) = fresh_store(DEFAULT_PACK_TARGET_SIZE).await;

        // 10k 4 KB puts + 1k 1 MB puts. All distinct content so dedup
        // never short-circuits.
        let n_small: u32 = 10_000;
        let n_large: u32 = 1_000;
        let small_size: usize = 4 * 1024;
        let large_size: usize = 1024 * 1024;

        let mut put_latencies = Vec::with_capacity((n_small + n_large) as usize);
        let mut hashes = Vec::with_capacity((n_small + n_large) as usize);
        let total_start = std::time::Instant::now();

        for i in 0..n_small {
            let mut payload = vec![0u8; small_size];
            payload[..4].copy_from_slice(&i.to_le_bytes());
            let t = std::time::Instant::now();
            let h = store.put(payload).await.unwrap();
            put_latencies.push(t.elapsed());
            hashes.push(h);
        }
        for i in 0..n_large {
            let mut payload = vec![0u8; large_size];
            payload[..4].copy_from_slice(&i.to_le_bytes());
            let t = std::time::Instant::now();
            let h = store.put(payload).await.unwrap();
            put_latencies.push(t.elapsed());
            hashes.push(h);
        }
        let put_total = total_start.elapsed();

        let read_start = std::time::Instant::now();
        let mut get_latencies = Vec::with_capacity(hashes.len());
        for h in &hashes {
            let t = std::time::Instant::now();
            let _ = store.get(h).await.unwrap();
            get_latencies.push(t.elapsed());
        }
        let get_total = read_start.elapsed();

        put_latencies.sort();
        get_latencies.sort();
        let p50_put = put_latencies[put_latencies.len() / 2];
        let p99_put = put_latencies[put_latencies.len() * 99 / 100];
        let p50_get = get_latencies[get_latencies.len() / 2];
        let p99_get = get_latencies[get_latencies.len() * 99 / 100];

        let packs_dir_size = walk_size(dir.path());
        let pack_files = fs::read_dir(dir.path())
            .unwrap()
            .filter(Result::is_ok)
            .count();

        println!(
            "[bench_pack_throughput] {} puts in {:?} (p50={:?} p99={:?})",
            put_latencies.len(),
            put_total,
            p50_put,
            p99_put,
        );
        println!(
            "[bench_pack_throughput] {} gets in {:?} (p50={:?} p99={:?})",
            get_latencies.len(),
            get_total,
            p50_get,
            p99_get,
        );
        println!(
            "[bench_pack_throughput] on-disk bytes: {packs_dir_size} across {pack_files} files",
        );
    }

    fn walk_size(p: &Path) -> u64 {
        let mut total = 0u64;
        if let Ok(entries) = fs::read_dir(p) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if meta.is_file() {
                        total = total.saturating_add(meta.len());
                    } else if meta.is_dir() {
                        total = total.saturating_add(walk_size(&entry.path()));
                    }
                }
            }
        }
        total
    }

    #[tokio::test]
    async fn corruption_returns_error() {
        let (dir, store) = fresh_store(DEFAULT_PACK_TARGET_SIZE).await;
        let hash = store.put(b"please don't bitrot me".to_vec()).await.unwrap();
        store.flush().await.unwrap();
        // Flip a payload byte. Frame layout: 16 B header + payload.
        // Open pack 0 (the .open file since we never rotated).
        let path = pack_path_open(dir.path(), 0);
        let mut f = OpenOptions::new().read(true).write(true).open(&path).unwrap();
        f.seek(SeekFrom::Start(16)).unwrap();
        let mut b = [0u8; 1];
        f.read_exact(&mut b).unwrap();
        b[0] ^= 0x01;
        f.seek(SeekFrom::Start(16)).unwrap();
        f.write_all(&b).unwrap();
        f.sync_all().unwrap();
        drop(f);
        let err = store.get(&hash).await.unwrap_err();
        assert!(matches!(err, PackError::Corruption(_)), "{err:?}");
    }
}
