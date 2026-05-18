//! Write helpers for `label_groups`. Reads live in
//! `rtsk::db::queries_extra::navigation`; writes belong here so the
//! Service is the only crate that can mutate them - the `app-no-db`
//! dependency rule enforces that the app cannot import this module.

use crate::db::{WriteConn, params};

/// Persist a new ordering for label groups. Each `(group_id, sort_order)`
/// pair is written in a single transaction. Drives drag-to-reorder in
/// Settings > Labels.
pub fn update_label_group_sort_order_sync(
    conn: &WriteConn<'_>,
    updates: &[(i64, i64)],
) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("label_group.reorder begin tx: {e}"))?;
    {
        let mut stmt = tx
            .prepare("UPDATE label_groups SET sort_order = ?1 WHERE id = ?2")
            .map_err(|e| e.to_string())?;
        for (id, order) in updates {
            stmt.execute(params![order, id])
                .map_err(|e| e.to_string())?;
        }
    }
    tx.commit()
        .map_err(|e| format!("label_group.reorder commit: {e}"))?;
    Ok(())
}
