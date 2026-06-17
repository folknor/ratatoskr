//! Phase 8-2: per-store "last clean shutdown" cursors.
//!
//! Read at boot by the cross-store invariant pass to bound the row
//! scans. Written during the graceful-shutdown drain (after subsystem
//! drains, before the `clean_shutdown` sentinel) so the cursor reflects
//! the most recent moment at which the store was known consistent.
//!
//! See `crates/service/src/startup_invariants.rs` for the consumer side
//! and `crates/service/src/lifecycle.rs` for the producer side.

use rusqlite::params;

use crate::db::{WriteTarget, WriteTransactionTarget};

/// Read the cursor for a store. Returns 0 if the row is absent
/// (semantics: "no clean shutdown ever recorded for this store" -
/// scan everything).
pub fn get_clean_shutdown_cursor(conn: &impl WriteTarget, store_name: &str) -> Result<i64, String> {
    match conn.query_row(
        "SELECT last_clean_shutdown_at FROM clean_shutdown_cursors WHERE store_name = ?1",
        params![store_name],
        |r| r.get::<_, i64>(0),
    ) {
        Ok(v) => Ok(v),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
        Err(e) => Err(format!("get_clean_shutdown_cursor({store_name}): {e}")),
    }
}

/// Update the cursors for the given stores to `unixepoch()` in a single
/// transaction. Idempotent. Failure is non-fatal: callers log at warn
/// and continue (the cost is just a wider scan on the next dirty boot).
///
pub fn update_clean_shutdown_cursors(
    conn: &impl WriteTransactionTarget,
    stores: &[&str],
) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("update_clean_shutdown_cursors begin: {e}"))?;
    for store in stores {
        tx.execute(
            "INSERT INTO clean_shutdown_cursors (store_name, last_clean_shutdown_at) \
             VALUES (?1, unixepoch()) \
             ON CONFLICT(store_name) DO UPDATE SET last_clean_shutdown_at = unixepoch()",
            params![store],
        )
        .map_err(|e| format!("update_clean_shutdown_cursors upsert {store}: {e}"))?;
    }
    tx.commit()
        .map_err(|e| format!("update_clean_shutdown_cursors commit: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn fresh_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory");
        conn.execute_batch(
            "CREATE TABLE clean_shutdown_cursors (
                store_name TEXT PRIMARY KEY,
                last_clean_shutdown_at INTEGER NOT NULL DEFAULT 0
            );",
        )
        .expect("create table");
        conn
    }

    fn write(conn: &Connection) -> crate::db::WriteConn<'_> {
        crate::db::WriteConn::from_raw(conn)
    }

    #[test]
    fn missing_cursor_returns_zero() {
        let conn = fresh_conn();
        assert_eq!(
            get_clean_shutdown_cursor(&write(&conn), "body").expect("get"),
            0
        );
    }

    #[test]
    fn update_then_read_back() {
        let conn = fresh_conn();
        update_clean_shutdown_cursors(&write(&conn), &["body", "inline", "extract"])
            .expect("update");
        let body = get_clean_shutdown_cursor(&write(&conn), "body").expect("body");
        let inline = get_clean_shutdown_cursor(&write(&conn), "inline").expect("inline");
        let extract = get_clean_shutdown_cursor(&write(&conn), "extract").expect("extract");
        assert!(body > 0);
        assert!(inline > 0);
        assert!(extract > 0);
        // All written in the same transaction so they share the same epoch.
        assert_eq!(body, inline);
        assert_eq!(inline, extract);
    }

    #[test]
    fn update_is_idempotent_and_advances() {
        let conn = fresh_conn();
        update_clean_shutdown_cursors(&write(&conn), &["body"]).expect("first update");
        let first = get_clean_shutdown_cursor(&write(&conn), "body").expect("first read");
        // unixepoch() granularity is 1 second; sleep briefly to observe advance.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        update_clean_shutdown_cursors(&write(&conn), &["body"]).expect("second update");
        let second = get_clean_shutdown_cursor(&write(&conn), "body").expect("second read");
        assert!(second >= first, "cursor must not move backwards");
    }
}
