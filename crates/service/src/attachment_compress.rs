//! Attachments roadmap Phase 9: inline `squeeze::compress` at the
//! cache-write boundary.
//!
//! Two call sites in the codebase put provider bytes into PackStore:
//! the prefetch worker (`prefetch.rs`) and the `attachment.fetch`
//! cache-miss path (`handlers/attachment.rs`). Both route through
//! `maybe_compress` so the on-disk pack frames are pre-compressed
//! when prefs allow.
//!
//! **Determinism contract**: PackStore is content-addressed via
//! BLAKE3, so two fetches of the same provider bytes must yield the
//! same compressed bytes to preserve dedup. Squeeze's underlying
//! tools (mozjpeg, oxipng, lopdf, the zip-format crate) are
//! deterministic given a fixed `Config`, so two `maybe_compress`
//! calls with the same `(bytes, Config)` produce byte-identical
//! output. A `Config` flip mid-cache-lifetime (e.g. user toggles
//! lossy on) breaks dedup for blobs fetched on either side of the
//! flip; acceptable trade.
//!
//! **Failure mode**: compression errors are non-fatal. We log and
//! pass the original bytes through. Cache correctness must not
//! depend on squeeze working.

use service_state::WriteDbState;
use squeeze::config::Config;

/// Returns the bytes to hand to `PackStore::put`. May be the
/// compressed form, may be the original.
///
/// Looks up `mime_type` from the `attachments` row and reads the
/// Phase 6 prefs `compress_attachments` (default true) and
/// `allow_lossy_compression` (default false). When
/// `compress_attachments=false`, returns the original bytes without
/// touching squeeze. Otherwise calls `squeeze::compress` with
/// `Config::lossless()` (allow_lossy=false) or `Config::email_default()`
/// (allow_lossy=true) and logs the per-mime savings.
pub(crate) async fn maybe_compress(
    db: &WriteDbState,
    attachment_id: String,
    bytes: Vec<u8>,
) -> Vec<u8> {
    let probe = match db
        .with_conn(move |conn| {
            let mime: Option<String> = conn
                .query_row(
                    "SELECT mime_type FROM attachments WHERE id = ?1",
                    rusqlite::params![attachment_id],
                    |r| r.get(0),
                )
                .ok()
                .flatten();
            let compress_pref = read_bool(conn, "compress_attachments", true);
            let allow_lossy = read_bool(conn, "allow_lossy_compression", false);
            Ok((mime, compress_pref, allow_lossy))
        })
        .await
    {
        Ok(t) => t,
        Err(e) => {
            log::debug!("maybe_compress: settings probe failed, passing through: {e}");
            return bytes;
        }
    };
    let (mime, compress_pref, allow_lossy) = probe;
    if !compress_pref {
        return bytes;
    }
    let mime_for_squeeze = mime.as_deref().unwrap_or("application/octet-stream");
    let cfg = if allow_lossy { Config::email_default() } else { Config::lossless() };

    let original_size = bytes.len();
    match squeeze::compress(&bytes, mime_for_squeeze, &cfg) {
        Ok(result) => {
            if result.was_compressed() {
                let pct = result.savings_pct();
                let compressed_size = result.compressed_size;
                log::info!(
                    "attachment-compress: mime={mime_for_squeeze} original={original_size} compressed={compressed_size} pct={pct:.1}",
                );
                result.into_bytes(&bytes)
            } else {
                log::debug!(
                    "attachment-compress: mime={mime_for_squeeze} unchanged (size={original_size})",
                );
                bytes
            }
        }
        Err(e) => {
            log::warn!(
                "attachment-compress: squeeze failed for mime={mime_for_squeeze}, passing through: {e}",
            );
            bytes
        }
    }
}

fn read_bool(conn: &rusqlite::Connection, key: &str, default: bool) -> bool {
    match rtsk::db::queries::get_setting(conn, key) {
        Ok(Some(s)) => s == "true",
        _ => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_bool_parses_settings_table() {
        let conn = rusqlite::Connection::open_in_memory().expect("open");
        conn.execute(
            "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            [],
        )
        .expect("create");
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('on_key', 'true'), ('off_key', 'false')",
            [],
        )
        .expect("insert");
        assert!(read_bool(&conn, "on_key", false));
        assert!(!read_bool(&conn, "off_key", true));
        // Missing key falls back to the default.
        assert!(read_bool(&conn, "absent_key", true));
        assert!(!read_bool(&conn, "absent_key", false));
    }
}
