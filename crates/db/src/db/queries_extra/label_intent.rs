use rusqlite::params;

use crate::db::{WriteTarget, WriteTxn};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingLabelIntentOp {
    Add,
    Remove,
}

impl PendingLabelIntentOp {
    fn as_str(self) -> &'static str {
        match self {
            Self::Add => "Add",
            Self::Remove => "Remove",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PendingLabelIntent<'a> {
    pub label_id: &'a str,
    pub op: PendingLabelIntentOp,
}

fn now_epoch() -> Result<i64, String> {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs(),
    )
    .map_err(|_| "current time exceeds i64 range".to_string())
}

/// Upsert a batch of pending label intents for one thread, capturing the
/// current `threads.label_membership_generation` once and stamping every
/// row with that snapshot. Returns the captured generation so the caller
/// can later attach an `action_id` keyed to this exact snapshot - that
/// is what makes the attach immune to a same-`op` overwrite by a
/// concurrent action.
pub fn upsert_pending_thread_label_intents<'a>(
    conn: &impl WriteTarget,
    account_id: &str,
    thread_id: &str,
    intents: impl IntoIterator<Item = PendingLabelIntent<'a>>,
    action_id: Option<&str>,
) -> Result<i64, String> {
    let generation: i64 = conn
        .query_row(
            "SELECT label_membership_generation FROM threads \
             WHERE account_id = ?1 AND id = ?2",
            params![account_id, thread_id],
            |row| row.get(0),
        )
        .map_err(|e| {
            format!("read thread generation for {account_id}/{thread_id}: {e}")
        })?;

    let now = now_epoch()?;
    for intent in intents {
        conn.execute(
            "INSERT INTO pending_thread_label_intents \
             (account_id, thread_id, label_id, op, generation_seen, action_id, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7) \
             ON CONFLICT(account_id, thread_id, label_id) DO UPDATE SET \
               op = excluded.op, \
               generation_seen = excluded.generation_seen, \
               action_id = excluded.action_id, \
               updated_at = excluded.updated_at",
            params![
                account_id,
                thread_id,
                intent.label_id,
                intent.op.as_str(),
                generation,
                action_id,
                now,
            ],
        )
        .map_err(|e| format!("upsert pending label intent: {e}"))?;
    }

    Ok(generation)
}

/// Attach an `action_id` to pending intents previously written by the
/// caller. Matches on `(label_id, op, generation_seen)` so that a same-
/// `op` overwrite by a concurrent action - which would have refreshed
/// `generation_seen` to a newer snapshot - silently no-ops here instead
/// of clobbering the newer action's `action_id`.
pub fn attach_action_id_to_pending_thread_label_intents<'a>(
    conn: &impl WriteTarget,
    account_id: &str,
    thread_id: &str,
    intents: impl IntoIterator<Item = PendingLabelIntent<'a>>,
    generation_seen: i64,
    action_id: &str,
) -> Result<(), String> {
    let now = now_epoch()?;
    for intent in intents {
        conn.execute(
            "UPDATE pending_thread_label_intents \
             SET action_id = ?5, updated_at = ?6 \
             WHERE account_id = ?1 \
               AND thread_id = ?2 \
               AND label_id = ?3 \
               AND op = ?4 \
               AND generation_seen = ?7",
            params![
                account_id,
                thread_id,
                intent.label_id,
                intent.op.as_str(),
                action_id,
                now,
                generation_seen,
            ],
        )
        .map_err(|e| format!("attach pending label intent action id: {e}"))?;
    }
    Ok(())
}

/// Delete pending intents for a single action's label set. Used as the
/// immediate-permanent-failure path: when dispatch returns a permanent
/// provider error we tear down the optimistic state instead of waiting
/// for the stale-intent sweep.
pub fn delete_pending_thread_label_intents_for_labels<'a>(
    conn: &impl WriteTarget,
    account_id: &str,
    thread_id: &str,
    intents: impl IntoIterator<Item = PendingLabelIntent<'a>>,
    generation_seen: i64,
) -> Result<(), String> {
    for intent in intents {
        conn.execute(
            "DELETE FROM pending_thread_label_intents \
             WHERE account_id = ?1 \
               AND thread_id = ?2 \
               AND label_id = ?3 \
               AND op = ?4 \
               AND generation_seen = ?5",
            params![
                account_id,
                thread_id,
                intent.label_id,
                intent.op.as_str(),
                generation_seen,
            ],
        )
        .map_err(|e| format!("delete pending label intent: {e}"))?;
    }
    Ok(())
}

pub fn delete_pending_thread_label_intents_for_action(
    conn: &impl WriteTarget,
    action_id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM pending_thread_label_intents WHERE action_id = ?1",
        params![action_id],
    )
    .map_err(|e| format!("delete pending label intents for action: {e}"))?;
    Ok(())
}

pub fn delete_stale_pending_thread_label_intents(
    conn: &impl WriteTarget,
    max_age_secs: i64,
) -> Result<usize, String> {
    if max_age_secs <= 0 {
        return Err("pending label intent stale age must be positive".to_string());
    }
    let cutoff = now_epoch()?
        .checked_sub(max_age_secs)
        .ok_or_else(|| "pending label intent cutoff underflow".to_string())?;
    conn.execute(
        "DELETE FROM pending_thread_label_intents \
         WHERE updated_at < ?1 \
           AND ( \
             action_id IS NULL \
             OR NOT EXISTS ( \
               SELECT 1 FROM pending_operations po \
               WHERE po.id = pending_thread_label_intents.action_id \
                 AND po.status IN ('pending', 'executing') \
             ) \
           )",
        params![cutoff],
    )
    .map_err(|e| format!("delete stale pending label intents: {e}"))
}

pub fn bump_thread_label_membership_generation(
    tx: &WriteTxn<'_>,
    account_id: &str,
    thread_id: &str,
) -> Result<(), String> {
    tx.execute(
        "UPDATE threads \
         SET label_membership_generation = label_membership_generation + 1 \
         WHERE account_id = ?1 AND id = ?2",
        params![account_id, thread_id],
    )
    .map_err(|e| format!("bump label membership generation: {e}"))?;
    Ok(())
}

pub fn clear_satisfied_pending_thread_label_intents(
    tx: &WriteTxn<'_>,
    account_id: &str,
    thread_id: &str,
) -> Result<(), String> {
    tx.execute(
        "DELETE FROM pending_thread_label_intents \
         WHERE account_id = ?1 \
           AND thread_id = ?2 \
           AND generation_seen < ( \
             SELECT label_membership_generation FROM threads \
             WHERE account_id = ?1 AND id = ?2 \
           ) \
           AND ( \
             (op = 'Add' AND EXISTS ( \
               SELECT 1 FROM thread_labels tl \
               WHERE tl.account_id = pending_thread_label_intents.account_id \
                 AND tl.thread_id = pending_thread_label_intents.thread_id \
                 AND tl.label_id = pending_thread_label_intents.label_id \
             )) \
             OR \
             (op = 'Remove' AND NOT EXISTS ( \
               SELECT 1 FROM thread_labels tl \
               WHERE tl.account_id = pending_thread_label_intents.account_id \
                 AND tl.thread_id = pending_thread_label_intents.thread_id \
                 AND tl.label_id = pending_thread_label_intents.label_id \
             )) \
           )",
        params![account_id, thread_id],
    )
    .map_err(|e| format!("clear satisfied pending label intents: {e}"))?;
    Ok(())
}

pub fn finalize_provider_truth_label_membership(
    tx: &WriteTxn<'_>,
    account_id: &str,
    thread_id: &str,
) -> Result<(), String> {
    bump_thread_label_membership_generation(tx, account_id, thread_id)?;
    clear_satisfied_pending_thread_label_intents(tx, account_id, thread_id)
}

/// Apply a confirmed provider-truth delta inside the caller's
/// transaction. Each `Add` stamps the label on `thread_labels` and on
/// `message_labels` for every current message in the thread; each
/// `Remove` deletes from both, plus from `message_keywords` when the
/// label is keyword-shaped (`kw:<keyword>`).
///
/// The per-message writes are load-bearing under cure D: the recompute
/// paths (`recompute_thread_labels_from_messages` and friends) derive
/// `thread_labels` from `message_labels ∪ message_keywords`, so a
/// thread-only write would be silently wiped by any later recompute
/// (for example, a sibling-message delta on Graph). Writing both halves
/// here keeps `confirmed_provider_label_intents` idempotent with the
/// recompute and removes the design-window the discrepancies doc named
/// in the cross-client move entry.
///
/// `finalize_provider_truth_label_membership` then bumps the per-thread
/// generation and clears any pending intents the resulting truth
/// satisfies. The caller owns the `Transaction` so this helper can be
/// composed with other writes in the same atomic step.
pub fn confirmed_provider_label_intents<'a>(
    tx: &WriteTxn<'_>,
    account_id: &str,
    thread_id: &str,
    intents: impl IntoIterator<Item = PendingLabelIntent<'a>>,
) -> Result<(), String> {
    for intent in intents {
        match intent.op {
            PendingLabelIntentOp::Add => {
                tx.execute(
                    "INSERT OR IGNORE INTO thread_labels (account_id, thread_id, label_id) \
                     VALUES (?1, ?2, ?3)",
                    params![account_id, thread_id, intent.label_id],
                )
                .map_err(|e| format!("confirm add label intent (thread): {e}"))?;
                tx.execute(
                    "INSERT OR IGNORE INTO message_labels (account_id, message_id, label_id) \
                     SELECT m.account_id, m.id, ?3 \
                     FROM messages m \
                     WHERE m.account_id = ?1 AND m.thread_id = ?2",
                    params![account_id, thread_id, intent.label_id],
                )
                .map_err(|e| format!("confirm add label intent (per-message): {e}"))?;
            }
            PendingLabelIntentOp::Remove => {
                tx.execute(
                    "DELETE FROM thread_labels \
                     WHERE account_id = ?1 AND thread_id = ?2 AND label_id = ?3",
                    params![account_id, thread_id, intent.label_id],
                )
                .map_err(|e| format!("confirm remove label intent (thread): {e}"))?;
                tx.execute(
                    "DELETE FROM message_labels \
                     WHERE account_id = ?1 \
                       AND label_id = ?3 \
                       AND message_id IN ( \
                         SELECT id FROM messages \
                         WHERE account_id = ?1 AND thread_id = ?2 \
                       )",
                    params![account_id, thread_id, intent.label_id],
                )
                .map_err(|e| format!("confirm remove label intent (per-message): {e}"))?;
                if let Some(keyword) = intent.label_id.strip_prefix("kw:") {
                    tx.execute(
                        "DELETE FROM message_keywords \
                         WHERE account_id = ?1 \
                           AND keyword = ?3 \
                           AND message_id IN ( \
                             SELECT id FROM messages \
                             WHERE account_id = ?1 AND thread_id = ?2 \
                           )",
                        params![account_id, thread_id, keyword],
                    )
                    .map_err(|e| format!("confirm remove label intent (keyword): {e}"))?;
                }
            }
        }
    }
    finalize_provider_truth_label_membership(tx, account_id, thread_id)
}

pub fn user_visible_label_exists_fragment(
    account_column: &str,
    thread_column: &str,
    label_expr: &str,
) -> String {
    format!(
        "((EXISTS (SELECT 1 FROM thread_labels tl \
              WHERE tl.account_id = {account_column} \
                AND tl.thread_id = {thread_column} \
                AND tl.label_id = {label_expr} \
                AND NOT EXISTS (SELECT 1 FROM pending_thread_label_intents pli_rm \
                  WHERE pli_rm.account_id = tl.account_id \
                    AND pli_rm.thread_id = tl.thread_id \
                    AND pli_rm.label_id = tl.label_id \
                    AND pli_rm.op = 'Remove'))) \
          OR EXISTS (SELECT 1 FROM pending_thread_label_intents pli_add \
              WHERE pli_add.account_id = {account_column} \
                AND pli_add.thread_id = {thread_column} \
                AND pli_add.label_id = {label_expr} \
                AND pli_add.op = 'Add'))"
    )
}

pub fn user_visible_label_group_rendered_fragment(
    account_column: &str,
    thread_column: &str,
    group_predicate: &str,
) -> String {
    let visible_member = user_visible_label_exists_fragment(
        account_column,
        thread_column,
        "lgm.label_id",
    );
    format!(
        "EXISTS (SELECT 1 FROM label_group_members lgm \
           JOIN label_groups lg ON lg.id = lgm.group_id \
           WHERE lgm.account_id = {account_column} \
             AND {group_predicate} \
             AND {visible_member})"
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE threads (
               account_id TEXT NOT NULL,
               id TEXT NOT NULL,
               label_membership_generation INTEGER NOT NULL DEFAULT 1,
               PRIMARY KEY (account_id, id)
             );
             CREATE TABLE labels (
               account_id TEXT NOT NULL,
               id TEXT NOT NULL,
               PRIMARY KEY (account_id, id)
             );
             CREATE TABLE messages (
               account_id TEXT NOT NULL,
               id TEXT NOT NULL,
               thread_id TEXT NOT NULL,
               PRIMARY KEY (account_id, id)
             );
             CREATE TABLE thread_labels (
               account_id TEXT NOT NULL,
               thread_id TEXT NOT NULL,
               label_id TEXT NOT NULL,
               PRIMARY KEY (account_id, thread_id, label_id)
             );
             CREATE TABLE message_labels (
               account_id TEXT NOT NULL,
               message_id TEXT NOT NULL,
               label_id TEXT NOT NULL,
               PRIMARY KEY (account_id, message_id, label_id)
             );
             CREATE TABLE message_keywords (
               account_id TEXT NOT NULL,
               message_id TEXT NOT NULL,
               keyword TEXT NOT NULL,
               label_id TEXT NOT NULL,
               PRIMARY KEY (account_id, message_id, keyword)
             );
             CREATE TABLE pending_thread_label_intents (
               account_id TEXT NOT NULL,
               thread_id TEXT NOT NULL,
               label_id TEXT NOT NULL,
               op TEXT NOT NULL CHECK (op IN ('Add', 'Remove')),
               generation_seen INTEGER NOT NULL,
               action_id TEXT,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL,
               PRIMARY KEY (account_id, thread_id, label_id)
             );
             CREATE TABLE pending_operations (
               id TEXT PRIMARY KEY,
               status TEXT NOT NULL
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO threads (account_id, id) VALUES ('acc', 'thr')",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO labels (account_id, id) VALUES ('acc', 'lab')", [])
            .unwrap();
        conn.execute(
            "INSERT INTO messages (account_id, id, thread_id) VALUES ('acc', 'm1', 'thr')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (account_id, id, thread_id) VALUES ('acc', 'm2', 'thr')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn confirmed_add_bumps_and_clears_matching_intent() {
        let conn = conn();
        upsert_pending_thread_label_intents(
            &crate::db::WriteConn::from_raw(&conn),
            "acc",
            "thr",
            [PendingLabelIntent {
                label_id: "lab",
                op: PendingLabelIntentOp::Add,
            }],
            None,
        )
        .unwrap();

        let write = crate::db::WriteConn::from_raw(&conn);
        let tx = write.transaction().unwrap();
        confirmed_provider_label_intents(
            &tx,
            "acc",
            "thr",
            [PendingLabelIntent {
                label_id: "lab",
                op: PendingLabelIntentOp::Add,
            }],
        )
        .unwrap();
        tx.commit().unwrap();

        let pending: i64 = conn
            .query_row("SELECT COUNT(*) FROM pending_thread_label_intents", [], |row| {
                row.get(0)
            })
            .unwrap();
        let generation: i64 = conn
            .query_row(
                "SELECT label_membership_generation FROM threads",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pending, 0);
        assert_eq!(generation, 2);
    }

    #[test]
    fn last_intent_wins_per_label() {
        let conn = conn();
        upsert_pending_thread_label_intents(
            &crate::db::WriteConn::from_raw(&conn),
            "acc",
            "thr",
            [PendingLabelIntent {
                label_id: "lab",
                op: PendingLabelIntentOp::Add,
            }],
            None,
        )
        .unwrap();
        upsert_pending_thread_label_intents(
            &crate::db::WriteConn::from_raw(&conn),
            "acc",
            "thr",
            [PendingLabelIntent {
                label_id: "lab",
                op: PendingLabelIntentOp::Remove,
            }],
            None,
        )
        .unwrap();

        let op: String = conn
            .query_row("SELECT op FROM pending_thread_label_intents", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(op, "Remove");
    }

    #[test]
    fn confirmed_add_stamps_per_message_rows_so_recompute_preserves_truth() {
        // Regression: under cure D the recompute paths derive `thread_labels`
        // from `message_labels ∪ message_keywords`. If `confirmed_provider_
        // label_intents` only wrote `thread_labels`, a subsequent recompute
        // (delete-sibling, JWZ rethread, etc.) would silently wipe the
        // label. This test simulates that recompute against the schema
        // confirmed-intent writes to, and asserts truth survives.
        let conn = conn();
        let write = crate::db::WriteConn::from_raw(&conn);
        let tx = write.transaction().unwrap();
        confirmed_provider_label_intents(
            &tx,
            "acc",
            "thr",
            [PendingLabelIntent {
                label_id: "lab",
                op: PendingLabelIntentOp::Add,
            }],
        )
        .unwrap();
        // Recompute analogue: delete thread_labels and rebuild from
        // message_labels. If confirm hadn't stamped per-message rows, the
        // INSERT below would find nothing and the label would vanish.
        tx.execute(
            "DELETE FROM thread_labels WHERE account_id = 'acc' AND thread_id = 'thr'",
            [],
        )
        .unwrap();
        tx.execute(
            "INSERT OR IGNORE INTO thread_labels (account_id, thread_id, label_id) \
             SELECT DISTINCT m.account_id, m.thread_id, ml.label_id \
             FROM messages m \
             JOIN message_labels ml ON ml.account_id = m.account_id AND ml.message_id = m.id \
             WHERE m.account_id = 'acc' AND m.thread_id = 'thr'",
            [],
        )
        .unwrap();
        tx.commit().unwrap();

        let label_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM thread_labels WHERE label_id = 'lab'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(label_count, 1, "label should survive recompute");

        let per_message: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM message_labels WHERE label_id = 'lab'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(per_message, 2, "every current message should carry the label");
    }

    #[test]
    fn confirmed_remove_clears_per_message_and_keyword_rows() {
        let conn = conn();
        // Seed both per-message tables as if the label had been added previously.
        conn.execute(
            "INSERT INTO message_labels (account_id, message_id, label_id) VALUES ('acc', 'm1', 'kw:todo')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_labels (account_id, message_id, label_id) VALUES ('acc', 'm2', 'kw:todo')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_keywords (account_id, message_id, keyword, label_id) VALUES ('acc', 'm1', 'todo', 'kw:todo')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO thread_labels (account_id, thread_id, label_id) VALUES ('acc', 'thr', 'kw:todo')",
            [],
        )
        .unwrap();

        let write = crate::db::WriteConn::from_raw(&conn);
        let tx = write.transaction().unwrap();
        confirmed_provider_label_intents(
            &tx,
            "acc",
            "thr",
            [PendingLabelIntent {
                label_id: "kw:todo",
                op: PendingLabelIntentOp::Remove,
            }],
        )
        .unwrap();
        tx.commit().unwrap();

        let per_message: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM message_labels WHERE label_id = 'kw:todo'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(per_message, 0);
        let keyword_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM message_keywords WHERE keyword = 'todo'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(keyword_rows, 0);
        let thread_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM thread_labels WHERE label_id = 'kw:todo'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(thread_rows, 0);
    }

    #[test]
    fn stale_intent_sweep_keeps_live_queue_rows() {
        let conn = conn();
        upsert_pending_thread_label_intents(
            &crate::db::WriteConn::from_raw(&conn),
            "acc",
            "thr",
            [PendingLabelIntent {
                label_id: "lab",
                op: PendingLabelIntentOp::Add,
            }],
            Some("live"),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO pending_operations (id, status) VALUES ('live', 'pending')",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE pending_thread_label_intents SET updated_at = 0",
            [],
        )
        .unwrap();

        let deleted = delete_stale_pending_thread_label_intents(&crate::db::WriteConn::from_raw(&conn), 1).unwrap();
        assert_eq!(deleted, 0);

        conn.execute("DELETE FROM pending_operations WHERE id = 'live'", [])
            .unwrap();
        let deleted = delete_stale_pending_thread_label_intents(&crate::db::WriteConn::from_raw(&conn), 1).unwrap();
        assert_eq!(deleted, 1);
    }
}
