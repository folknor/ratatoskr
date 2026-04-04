pub mod accounts;
pub mod calendars;
pub mod config;
pub mod contacts;
pub mod people;
pub mod pinned_searches;
pub mod templates;
pub mod threads;

pub use config::Config;

use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use std::path::Path;

/// Generate a deterministic UUID v4 from the seeded RNG.
pub fn next_uuid(rng: &mut impl Rng) -> String {
    let mut bytes = [0u8; 16];
    rng.fill(&mut bytes);
    // Set version 4 and variant bits per RFC 4122
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        u16::from_be_bytes([bytes[4], bytes[5]]),
        u16::from_be_bytes([bytes[6], bytes[7]]),
        u16::from_be_bytes([bytes[8], bytes[9]]),
        u64::from_be_bytes([
            0, 0, bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
        ])
    )
}

/// Generate a deterministic Message-ID header.
pub fn next_message_id(rng: &mut impl Rng) -> String {
    let mut bytes = [0u8; 8];
    rng.fill(&mut bytes);
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("<{hex}@mail.ratatoskr.test>")
}

/// Seed a fresh database at the given data directory.
///
/// Creates `ratatoskr.db` (with schema via migrations) and `bodies.db`
/// (via the body store API), then populates both with synthetic data.
pub fn seed_database(config: &Config, app_data_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(app_data_dir).map_err(|e| format!("create data dir: {e}"))?;

    let start = std::time::Instant::now();
    let mut rng = SmallRng::seed_from_u64(config.seed);

    // ── Create main database with schema ────────────────────
    let db_path = app_data_dir.join("ratatoskr.db");
    log::info!("Dev-seed: creating {}", db_path.display());

    let conn = rusqlite::Connection::open(&db_path).map_err(|e| format!("open db: {e}"))?;

    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA busy_timeout = 15000;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA temp_store = MEMORY;",
    )
    .map_err(|e| format!("pragmas: {e}"))?;

    db::db::migrations::run_all(&conn)?;

    // ── Seed data in a single transaction ───────────────────
    conn.execute_batch("BEGIN")
        .map_err(|e| format!("begin: {e}"))?;

    let accs = accounts::seed_accounts(&conn, &mut rng, config.accounts)?;
    calendars::seed_calendars(&conn, &mut rng, &accs)?;
    let pools = people::generate_pools(&mut rng);
    let (pending_bodies, stats) = threads::generate_threads(
        &conn,
        &mut rng,
        &accs,
        &pools,
        &config.locale,
        config.threads,
    )?;
    contacts::seed_vips(&conn, &mut rng, &pools.combined, &accs)?;
    pinned_searches::seed_pinned_searches(&conn, &accs)?;

    conn.execute_batch("COMMIT")
        .map_err(|e| format!("commit: {e}"))?;

    // ── Populate body store via the stores API ──────────────
    let body_store = store::body_store::BodyStoreState::init(app_data_dir)
        .map_err(|e| format!("init body store: {e}"))?;

    let bs_conn = body_store.conn();
    let bs_conn = bs_conn
        .lock()
        .map_err(|e| format!("body store lock: {e}"))?;
    bs_conn
        .execute_batch("BEGIN")
        .map_err(|e| format!("body begin: {e}"))?;

    {
        let mut stmt = bs_conn
            .prepare(
                "INSERT OR REPLACE INTO bodies (message_id, body_html, body_text)
                 VALUES (?1, ?2, ?3)",
            )
            .map_err(|e| format!("prepare body insert: {e}"))?;

        for body in &pending_bodies {
            let html_blob = {
                use flate2::Compression;
                use flate2::write::ZlibEncoder;
                use std::io::Write;
                let mut enc = ZlibEncoder::new(Vec::new(), Compression::new(3));
                enc.write_all(body.body_html.as_bytes())
                    .map_err(|e| format!("zlib compress html: {e}"))?;
                enc.finish()
                    .map_err(|e| format!("zlib compress html: {e}"))?
            };
            let text_blob = {
                use flate2::Compression;
                use flate2::write::ZlibEncoder;
                use std::io::Write;
                let mut enc = ZlibEncoder::new(Vec::new(), Compression::new(3));
                enc.write_all(body.body_text.as_bytes())
                    .map_err(|e| format!("zlib compress text: {e}"))?;
                enc.finish()
                    .map_err(|e| format!("zlib compress text: {e}"))?
            };
            stmt.execute(rusqlite::params![body.message_id, html_blob, text_blob])
                .map_err(|e| format!("insert body: {e}"))?;
        }
    }

    bs_conn
        .execute_batch("COMMIT")
        .map_err(|e| format!("body commit: {e}"))?;

    let elapsed = start.elapsed();
    log::info!(
        "Dev-seed complete: {} threads, {} messages, {} attachments, {} bodies in {:.1}s",
        stats.threads,
        stats.messages,
        stats.attachments,
        pending_bodies.len(),
        elapsed.as_secs_f64()
    );

    Ok(())
}
