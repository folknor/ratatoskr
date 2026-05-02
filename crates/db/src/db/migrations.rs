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
pub fn run_all(conn: &Connection) -> Result<(), String> {
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
    for m in MIGRATIONS {
        if applied.contains(&m.version) {
            continue;
        }

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
    }

    log::info!("All migrations applied.");
    Ok(())
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
}
