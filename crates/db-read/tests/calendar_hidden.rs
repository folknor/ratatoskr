//! Brick R gate (B7c): a calendar stamped `unlisted_since` is hidden from the
//! production sidebar/agenda loaders, while a still-listed calendar and a
//! `calendar_id IS NULL` local event stay visible.
//!
//! Lives as an integration test (not a `#[cfg(test)]` module under
//! `db-read/src`) because the db-read raw-rusqlite lockdown bans `.execute(`
//! anywhere in `src`; seeding needs the writer pool, which is only reachable
//! from a test outside the quarantined tree.

use db::db::open_writer_pool;
use db_read::db::open_reader_pool;
use db_read::db::queries_extra::calendars::{
    load_calendars_for_sidebar_sync, load_view_event_rows_sync,
};

#[test]
fn calendar_hidden_when_unlisted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let writer = open_writer_pool(dir.path()).expect("writer");
    writer
        .with_write_sync(|conn| {
            conn.execute(
                "INSERT INTO accounts (id, email, provider) VALUES ('acc', 'a@example.test', 'caldav')",
                [],
            )
            .map_err(|error| error.to_string())?;
            conn.execute(
                "INSERT INTO calendars (id, account_id, provider, remote_id, display_name, is_visible, unlisted_since) \
                 VALUES ('visible', 'acc', 'caldav', 'visible', 'Visible', 1, NULL), \
                        ('hidden', 'acc', 'caldav', 'hidden', 'Hidden', 1, 1)",
                [],
            )
            .map_err(|error| error.to_string())?;
            conn.execute(
                "INSERT INTO calendar_events (id, account_id, google_event_id, start_time, end_time, calendar_id) \
                 VALUES ('visible-event', 'acc', 'visible-event', 10, 20, 'visible'), \
                        ('hidden-event', 'acc', 'hidden-event', 10, 20, 'hidden'), \
                        ('local-event', 'acc', 'local-event', 10, 20, NULL)",
                [],
            )
            .map_err(|error| error.to_string())?;
            Ok(())
        })
        .expect("seed database");
    let reader = open_reader_pool(dir.path()).expect("reader");
    reader
        .with_read_sync(|conn| {
            let sidebar = load_calendars_for_sidebar_sync(conn)?;
            assert_eq!(sidebar.len(), 1);
            assert_eq!(sidebar[0].id, "visible");
            let events = load_view_event_rows_sync(conn, 0, 30)?;
            let ids: Vec<_> = events.into_iter().map(|event| event.id).collect();
            assert!(ids.contains(&"visible-event".to_string()));
            assert!(ids.contains(&"local-event".to_string()));
            assert!(!ids.contains(&"hidden-event".to_string()));
            Ok(())
        })
        .expect("read filtered calendars");
}
