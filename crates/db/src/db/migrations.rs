use rusqlite::Connection;

struct Migration {
    version: u32,
    description: &'static str,
    sql: &'static str,
}

// Schema collapsed from incremental migrations on 2026-03-30, with subsequent
// schema additions folded back in on 2026-05-02 (was v100 + v101 pinned-search
// tables + v102 caldav_event_map). The schema itself lives in `schema/*.sql`
// files split by domain; this file just glues them together and runs them.
// No releases have been made, so there are no databases in the wild that need
// incremental migration. If you have an existing dev database, delete it and
// re-seed (run_all will detect stale DBs and error).
const SCHEMA_V100: &str = concat!(
    // accounts, settings (+ defaults INSERT)
    include_str!("schema/01_core.sql"),
    "\n",
    // labels, label_color_overrides, threads, thread_labels, thread_bundles,
    // messages, attachments, cloud_attachments, thread_ui_state
    include_str!("schema/02_mail.sql"),
    "\n",
    // contacts (+ FTS + triggers), seen_addresses (+ FTS + triggers),
    // contact_groups, contact_group_members, contact_photo_cache, gal_cache,
    // graph_contact_map, google_contact_map, google_other_contact_map,
    // carddav_contact_map, graph_contact_delta_tokens
    include_str!("schema/03_contacts.sql"),
    "\n",
    // signatures, send_as_aliases, send_identities, local_drafts, scheduled_emails
    include_str!("schema/04_compose.sql"),
    "\n",
    // calendars, calendar_events, calendar_attendees, calendar_reminders, caldav_event_map
    include_str!("schema/05_calendar.sql"),
    "\n",
    // tasks, task_tags
    include_str!("schema/06_tasks.sql"),
    "\n",
    // smart_folders (+ defaults), quick_steps, pinned_searches,
    // pinned_search_threads, ai_cache, smart_label_rules, writing_style_profiles
    include_str!("schema/07_smart.sql"),
    "\n",
    // follow_up_reminders, notification_vips, unsubscribe_actions, bundle_rules,
    // bundled_threads, auto_responses
    include_str!("schema/08_notifications.sql"),
    "\n",
    // link_scan_results, phishing_allowlist, bimi_cache, message_reactions,
    // read_receipt_policy, filter_rules, templates, image_allowlist
    include_str!("schema/09_security.sql"),
    "\n",
    // folder_sync_state, jmap_sync_state, graph_folder_delta_tokens,
    // graph_shared_mailbox_delta_tokens, shared_mailbox_sync_state,
    // jmap_push_state, graph_subscriptions, pending_operations
    include_str!("schema/10_sync.sql"),
    "\n",
    // public_folders, public_folder_items, public_folder_pins,
    // public_folder_sync_state, public_folder_content_routing,
    // chat_contacts, thread_participants
    include_str!("schema/11_collaboration.sql"),
    "\n",
    // action_jobs, action_job_ops (Phase 2 sibling-job journal)
    include_str!("schema/12_actions.sql"),
);

// PRE-RELEASE POLICY: until we ship a release, schema changes go directly into
// the relevant `schema/*.sql` file (extending v100 in place). Do NOT add a v101
// migration entry here. Dev DBs are wiped and re-seeded on each launch
// (`--features dev-seed`), so there are no databases in the wild whose state
// would be skipped. After the first release, the next change becomes v101 with
// its own ALTER/CREATE statements appended to MIGRATIONS.
static MIGRATIONS: &[Migration] = &[Migration {
    version: 100,
    description: "Initial schema",
    sql: SCHEMA_V100,
}];

/// Run all pending migrations. Called once at startup.
///
/// Convenience wrapper for `run_all_with_progress` with a no-op callback.
/// Returns the number of migrations actually applied (0 if the DB was
/// already up to date).
pub fn run_all(conn: &Connection) -> Result<u32, String> {
    run_all_with_progress(conn, &mut |_, _| {})
}

/// Run all pending migrations, invoking the `progress(current, total)`
/// callback at two points per applied migration:
/// - `progress(index, total)` BEFORE the migration's transaction begins,
///   where `index` is 0-based for the first migration, 1-based thereafter.
///   This produces the "now applying N/total" frame the UI splash needs to
///   render the `Migrating` phase even when the migration itself completes
///   before the post-commit frame can race other phase notifications.
/// - `progress(current, total)` AFTER the COMMIT, where `current` is the
///   1-based count of migrations that have completed in this run. Emitting
///   the completion frame post-commit means the user-visible "applied N
///   of M" never overstates: a crash mid-migration rolls back via SQLite
///   WAL recovery and the next boot re-runs from the same starting count.
///
/// Net: `total` callbacks for `total` migrations, where the first frame
/// carries `(0, total)` ("starting migration 1") and subsequent frames
/// carry `(N, total)` ("migration N committed"). The wire-side per-phase
/// `CoalesceKey::BootProgress(BootPhaseKind::Migrating)` collapses these
/// to the latest, so the user experience is monotonic.
///
/// Phase 1.5 ships with a single v100 migration, so a fresh DB sees frames
/// `(0, 1)` then `(1, 1)`; the per-step callback exists for future multi-
/// migration releases.
///
/// Contract for migration authors:
/// - Each migration must wrap in a single SQLite transaction; the runner's
///   BEGIN / COMMIT around `m.sql` is the canonical shape.
/// - If a future migration MUST batch into multiple committed transactions
///   (per-row backfills), each batch must be idempotent and resumable, AND
///   the `_migrations` row must NOT be inserted until every batch has
///   committed - a partial-apply that lacks the row will be re-run from
///   scratch on the next boot.
/// - `progress(current, total)` may emit values that go BACKWARDS on
///   respawn (first run got to 4/10, second run starts at 0/10). The wire-
///   side per-phase coalesce key handles compaction; the splash UX of
///   "moving backwards on respawn" is an accepted user-visible behaviour.
pub fn run_all_with_progress(
    conn: &Connection,
    progress: &mut dyn FnMut(u32, u32),
) -> Result<u32, String> {
    // Ensure migrations table exists
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _migrations (
           version INTEGER PRIMARY KEY,
           description TEXT,
           applied_at INTEGER DEFAULT (unixepoch())
         )",
    )
    .map_err(|e| format!("create _migrations: {e}"))?;

    // Collect applied versions
    let mut stmt = conn
        .prepare("SELECT version FROM _migrations ORDER BY version")
        .map_err(|e| format!("prepare: {e}"))?;
    let applied: std::collections::HashSet<u32> = stmt
        .query_map([], |row| row.get::<_, u32>("version"))
        .map_err(|e| format!("query: {e}"))?
        .filter_map(Result::ok)
        .collect();
    drop(stmt);

    // Detect stale pre-collapse databases. The old schema had versions 1-80;
    // the collapsed schema starts fresh at version 1 with different content.
    // If we see any version not in our MIGRATIONS array, this DB predates
    // the collapse and must be recreated.
    let known_versions: std::collections::HashSet<u32> =
        MIGRATIONS.iter().map(|m| m.version).collect();
    let stale_versions: Vec<u32> = applied
        .iter()
        .filter(|v| !known_versions.contains(v))
        .copied()
        .collect();
    if !stale_versions.is_empty() {
        return Err(format!(
            "Database was created with a previous schema version \
             (found migration versions {stale_versions:?} which are not in \
             the current schema). Delete ratatoskr.db and re-seed."
        ));
    }

    // ── Run pending migrations ─────────────────────────────────
    let pending: Vec<&Migration> = MIGRATIONS
        .iter()
        .filter(|m| !applied.contains(&m.version))
        .collect();
    let total = u32::try_from(pending.len()).unwrap_or(u32::MAX);
    let mut applied_count: u32 = 0;
    for (index, m) in pending.iter().enumerate() {
        // Emit a "starting" frame BEFORE the transaction begins so the
        // splash gets a Migrating event even on a sub-second migration that
        // would otherwise complete before a post-commit notification could
        // race other phases through the writer queue. `current` here is
        // 0-based for the first migration; subsequent migrations carry the
        // number of already-committed migrations as their starting count.
        let starting = u32::try_from(index).unwrap_or(u32::MAX);
        progress(starting, total);

        log::info!("Running migration v{}: {}", m.version, m.description);

        conn.execute_batch("BEGIN")
            .map_err(|e| format!("begin: {e}"))?;

        if let Err(e) = conn.execute_batch(m.sql) {
            log::error!("Migration v{} failed: {e}", m.version);
            drop(conn.execute_batch("ROLLBACK"));
            return Err(format!("migration v{}: {e}", m.version));
        }

        conn.execute(
            "INSERT OR IGNORE INTO _migrations (version, description) VALUES (?1, ?2)",
            rusqlite::params![m.version, m.description],
        )
        .map_err(|e| format!("record migration: {e}"))?;
        conn.execute_batch("COMMIT")
            .map_err(|e| format!("commit: {e}"))?;

        applied_count = applied_count.saturating_add(1);
        let current = u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1);
        progress(current, total);
    }

    log::info!("All migrations applied.");
    Ok(applied_count)
}

/// Read the highest applied schema version from the `_migrations` table.
/// Returns 0 if the table doesn't exist or has no rows. Used by the Service
/// boot sequence to populate `BootReadyResponse.schema_version`.
pub fn current_schema_version(conn: &Connection) -> Result<u32, String> {
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='_migrations'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("query schema version: {e}"))?;
    if exists == 0 {
        return Ok(0);
    }
    let max_version: Option<u32> = conn
        .query_row("SELECT MAX(version) FROM _migrations", [], |row| row.get(0))
        .map_err(|e| format!("query max version: {e}"))?;
    Ok(max_version.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_run_on_fresh_db() {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("pragmas");
        run_all(&conn).expect("migrations should succeed");

        // Verify key tables exist
        let count: u32 = conn
            .query_row(
                "SELECT COUNT(*) AS cnt FROM sqlite_master WHERE type='table' AND name='threads'",
                [],
                |row| row.get("cnt"),
            )
            .expect("query");
        assert_eq!(count, 1);

        // Verify latest migration recorded
        let max_ver: u32 = conn
            .query_row(
                "SELECT MAX(version) AS max_ver FROM _migrations",
                [],
                |row| row.get("max_ver"),
            )
            .expect("query");
        let expected = MIGRATIONS.last().expect("at least one migration").version;
        assert_eq!(max_ver, expected);
    }

    /// Locks in the per-migration progress contract: each migration produces
    /// exactly two callback invocations - one BEFORE the transaction begins
    /// (current=index, 0-based) and one AFTER the commit completes
    /// (current=index+1, 1-based). Phase 1.5 ships with a single v100
    /// migration so a fresh DB sees `(0, 1)` then `(1, 1)`.
    #[test]
    fn run_all_with_progress_emits_before_and_after_commit() {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("pragmas");

        let mut frames: Vec<(u32, u32)> = Vec::new();
        let applied = run_all_with_progress(&conn, &mut |current, total| {
            frames.push((current, total));
        })
        .expect("migrations should succeed");

        // One migration applied, so total == 1.
        assert_eq!(applied, 1);
        assert_eq!(
            frames,
            vec![(0, 1), (1, 1)],
            "expected one before-COMMIT (0/1) and one after-COMMIT (1/1) frame, got {frames:?}"
        );
    }

    /// On a DB that's already at the latest schema, the runner must not call
    /// `progress` at all. Locks in "no spurious frames" - the splash would
    /// otherwise tick a Migrating phase on every UI launch even when no
    /// migration is actually running.
    #[test]
    fn run_all_with_progress_emits_nothing_on_up_to_date_db() {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("pragmas");
        run_all(&conn).expect("first run applies migrations");

        let mut frames: Vec<(u32, u32)> = Vec::new();
        let applied = run_all_with_progress(&conn, &mut |current, total| {
            frames.push((current, total));
        })
        .expect("second run should be a no-op");

        assert_eq!(applied, 0);
        assert!(frames.is_empty(), "no frames expected, got {frames:?}");
    }
}
