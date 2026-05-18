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
/// Looks up `mime_type` from the `attachments` row. The
/// `compress_attachments` / `allow_lossy_compression` prefs are read
/// by the caller via `BootSharedState::compress_attachments_enabled`
/// / `allow_lossy_compression_enabled` (cached atomics, refreshed on
/// every `settings.set` touching those keys) rather than by this
/// function. The previous design hit the DB writer-mutex twice per
/// call for the prefs alone, which serialized against unrelated
/// writes during prefetch backfill bursts of thousands of items.
pub(crate) async fn maybe_compress(
    db: &WriteDbState,
    attachment_id: String,
    bytes: Vec<u8>,
    compress_pref: bool,
    allow_lossy: bool,
) -> Vec<u8> {
    if !compress_pref {
        return bytes;
    }
    let mime = match db
        .with_write(move |conn| {
            let mime: Option<String> = conn
                .query_row(
                    "SELECT mime_type FROM attachments WHERE id = ?1",
                    rusqlite::params![attachment_id],
                    |r| r.get(0),
                )
                .ok()
                .flatten();
            Ok(mime)
        })
        .await
    {
        Ok(m) => m,
        Err(e) => {
            log::debug!("maybe_compress: mime probe failed, passing through: {e}");
            return bytes;
        }
    };
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
