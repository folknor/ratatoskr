//! Dev-seed for the labels-unification rollout: a couple of explicit
//! `label_groups`, member rows for matching per-account labels, and a
//! sample member-label attachments so the sidebar's LABELS section,
//! the `is:tagged` smart-folder operator, message-pill decoration,
//! and label-group counts exercise the post-overlay rendering path.
//!
//! Per the redesign, a fresh install starts empty: groups are explicit
//! user creations. Dev-seed ships them as demo state, separate from
//! anything the schema or the action service knows.

use rand::RngExt;
use rusqlite::Connection;

use crate::accounts::Account;

/// Names dev-seed groups under. For each group, every per-account
/// label matching one of these names becomes a member.
const GROUP_PRESETS: &[(&str, &str, &str)] = &[
    // (group_name, color_bg, color_fg)
    ("Important", "#d62728", "#ffffff"),
    ("Personal", "#1f77b4", "#ffffff"),
    ("Projects", "#2ca02c", "#ffffff"),
];

pub fn seed_label_groups(
    conn: &Connection,
    rng: &mut impl RngExt,
    accounts: &[Account],
) -> Result<(), String> {
    for (name, bg, fg) in GROUP_PRESETS {
        conn.execute(
            "INSERT OR IGNORE INTO label_groups (name, color_bg, color_fg) \
             VALUES (?1, ?2, ?3)",
            rusqlite::params![name, bg, fg],
        )
        .map_err(|e| format!("insert label_group: {e}"))?;

        let group_id: i64 = conn
            .query_row(
                "SELECT id FROM label_groups WHERE name = ?1 COLLATE NOCASE",
                rusqlite::params![name],
                |row| row.get(0),
            )
            .map_err(|e| format!("lookup label_group id: {e}"))?;

        // Members: any per-account label whose name matches the group name
        // case-insensitively, unique to one group per (account_id, label_id)
        // via the schema's UNIQUE constraint.
        for acc in accounts {
            for (label_name, label_id) in &acc.labels {
                if !label_name.eq_ignore_ascii_case(name) {
                    continue;
                }
                conn.execute(
                    "INSERT OR IGNORE INTO label_group_members \
                       (group_id, account_id, label_id) \
                     VALUES (?1, ?2, ?3)",
                    rusqlite::params![group_id, acc.id, label_id],
                )
                .map_err(|e| format!("insert label_group_member: {e}"))?;
            }
        }
    }

    // Attach a sample of threads through member raw-label rows. The
    // action-service-only `thread_label_groups` shortcut has been retired;
    // rendered group state derives from `thread_labels` plus pending intent.
    for acc in accounts {
        let label_id: Option<String> = conn
            .query_row(
                "SELECT lgm.label_id
                 FROM label_groups lg
                 INNER JOIN label_group_members lgm ON lgm.group_id = lg.id
                 WHERE lg.name = 'Important' COLLATE NOCASE
                   AND lgm.account_id = ?1
                 ORDER BY lgm.label_id
                 LIMIT 1",
                rusqlite::params![acc.id],
                |row| row.get(0),
            )
            .ok();
        let Some(label_id) = label_id else { continue };
        let mut stmt = conn
            .prepare(
                "SELECT id FROM threads \
                 WHERE account_id = ?1 \
                 ORDER BY last_message_at DESC LIMIT 50",
            )
            .map_err(|e| format!("prepare thread sample: {e}"))?;
        let thread_ids: Vec<String> = stmt
            .query_map(rusqlite::params![acc.id], |row| row.get::<_, String>(0))
            .map_err(|e| format!("query thread sample: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("map thread sample: {e}"))?;
        for thread_id in thread_ids {
            if rng.random::<f64>() < 0.1 {
                conn.execute(
                    "INSERT OR IGNORE INTO thread_labels \
                       (account_id, thread_id, label_id) \
                     VALUES (?1, ?2, ?3)",
                    rusqlite::params![acc.id, thread_id, label_id],
                )
                .map_err(|e| format!("insert dev-seed group member label: {e}"))?;
            }
        }
    }

    Ok(())
}
