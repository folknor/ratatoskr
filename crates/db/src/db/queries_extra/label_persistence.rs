use std::collections::HashSet;

use rusqlite::{Transaction, params};

#[derive(Debug, Clone)]
pub struct FolderWriteRow {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub visible: Option<bool>,
    pub sort_order: Option<i64>,
    pub imap_folder_path: Option<String>,
    pub imap_special_use: Option<String>,
    pub namespace_type: Option<String>,
    pub parent_id: Option<String>,
    pub right_read: Option<i64>,
    pub right_add: Option<i64>,
    pub right_remove: Option<i64>,
    pub right_set_seen: Option<i64>,
    pub right_set_keywords: Option<i64>,
    pub right_create_child: Option<i64>,
    pub right_rename: Option<i64>,
    pub right_delete: Option<i64>,
    pub right_submit: Option<i64>,
    pub is_subscribed: Option<i64>,
    pub is_undeletable: bool,
}

#[derive(Debug, Clone)]
pub struct LabelWriteRow {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub visible: Option<bool>,
    pub sort_order: Option<i64>,
    pub server_color_bg: Option<String>,
    pub server_color_fg: Option<String>,
    pub user_color_bg: Option<String>,
    pub user_color_fg: Option<String>,
    pub is_undeletable: bool,
}

pub fn insert_folders_batch(tx: &Transaction, rows: &[FolderWriteRow]) -> Result<(), String> {
    let sorted_rows = sort_folders_parent_first(rows)?;

    for row in sorted_rows {
        tx.execute(
            "INSERT INTO folders \
             (id, account_id, name, visible, sort_order, imap_folder_path, imap_special_use, \
              namespace_type, parent_id, right_read, right_add, right_remove, right_set_seen, \
              right_set_keywords, right_create_child, right_rename, right_delete, right_submit, \
              is_subscribed, is_undeletable) \
             VALUES (?1, ?2, ?3, COALESCE(?4, 1), COALESCE(?5, 0), ?6, ?7, ?8, ?9, \
                     ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20) \
             ON CONFLICT(account_id, id) DO UPDATE SET \
               name = excluded.name, \
               visible = excluded.visible, \
               sort_order = COALESCE(excluded.sort_order, folders.sort_order), \
               imap_folder_path = excluded.imap_folder_path, \
               imap_special_use = excluded.imap_special_use, \
               namespace_type = excluded.namespace_type, \
               parent_id = excluded.parent_id, \
               right_read = excluded.right_read, \
               right_add = excluded.right_add, \
               right_remove = excluded.right_remove, \
               right_set_seen = excluded.right_set_seen, \
               right_set_keywords = excluded.right_set_keywords, \
               right_create_child = excluded.right_create_child, \
               right_rename = excluded.right_rename, \
               right_delete = excluded.right_delete, \
               right_submit = excluded.right_submit, \
               is_subscribed = excluded.is_subscribed, \
               is_undeletable = excluded.is_undeletable",
            params![
                row.id,
                row.account_id,
                row.name,
                row.visible,
                row.sort_order,
                row.imap_folder_path,
                row.imap_special_use,
                row.namespace_type,
                row.parent_id,
                row.right_read,
                row.right_add,
                row.right_remove,
                row.right_set_seen,
                row.right_set_keywords,
                row.right_create_child,
                row.right_rename,
                row.right_delete,
                row.right_submit,
                row.is_subscribed,
                row.is_undeletable,
            ],
        )
        .map_err(|e| format!("upsert folder: {e}"))?;
    }

    Ok(())
}

/// Upsert `labels` rows. `is_undeletable` uses OR semantics on conflict so
/// the invariant from `docs/labels-unification/redesign.md` "is_undeletable"
/// holds even if a later sync pass forgets to set the flag: once a row is
/// marked undeletable (e.g. by the bootstrap synth for `importance:*`,
/// the typed action-side label writer, or a system-flag classification at ingest),
/// it stays that way.
pub fn upsert_labels(tx: &Transaction, rows: &[LabelWriteRow]) -> Result<(), String> {
    for row in rows {
        validate_label_color_pairs(
            &row.id,
            row.server_color_bg.as_deref(),
            row.server_color_fg.as_deref(),
            row.user_color_bg.as_deref(),
            row.user_color_fg.as_deref(),
        )?;

        tx.execute(
            "INSERT INTO labels \
             (id, account_id, name, visible, sort_order, server_color_bg, server_color_fg, \
              user_color_bg, user_color_fg, is_undeletable) \
             VALUES (?1, ?2, ?3, COALESCE(?4, 1), COALESCE(?5, 0), ?6, ?7, ?8, ?9, ?10) \
             ON CONFLICT(account_id, id) DO UPDATE SET \
               name = excluded.name, \
               visible = excluded.visible, \
               sort_order = COALESCE(excluded.sort_order, labels.sort_order), \
               server_color_bg = excluded.server_color_bg, \
               server_color_fg = excluded.server_color_fg, \
               user_color_bg = COALESCE(excluded.user_color_bg, labels.user_color_bg), \
               user_color_fg = COALESCE(excluded.user_color_fg, labels.user_color_fg), \
               is_undeletable = (excluded.is_undeletable OR labels.is_undeletable)",
            params![
                row.id,
                row.account_id,
                row.name,
                row.visible,
                row.sort_order,
                row.server_color_bg,
                row.server_color_fg,
                row.user_color_bg,
                row.user_color_fg,
                row.is_undeletable,
            ],
        )
        .map_err(|e| format!("upsert label: {e}"))?;
    }

    Ok(())
}

pub fn validate_label_color_pairs(
    label_id: &str,
    server_color_bg: Option<&str>,
    server_color_fg: Option<&str>,
    user_color_bg: Option<&str>,
    user_color_fg: Option<&str>,
) -> Result<(), String> {
    validate_label_color_pair(label_id, "server", server_color_bg, server_color_fg)?;
    validate_label_color_pair(label_id, "user", user_color_bg, user_color_fg)?;
    Ok(())
}

fn validate_label_color_pair(
    label_id: &str,
    source: &str,
    bg: Option<&str>,
    fg: Option<&str>,
) -> Result<(), String> {
    match (bg, fg) {
        (Some(_), Some(_)) | (None, None) => Ok(()),
        _ => Err(format!(
            "label {label_id} has incomplete {source} color pair"
        )),
    }
}

fn sort_folders_parent_first(rows: &[FolderWriteRow]) -> Result<Vec<&FolderWriteRow>, String> {
    let input_keys: HashSet<(String, String)> = rows
        .iter()
        .map(|row| (row.account_id.clone(), row.id.clone()))
        .collect();
    let mut inserted = HashSet::new();
    let mut remaining: Vec<&FolderWriteRow> = rows.iter().collect();
    let mut sorted = Vec::with_capacity(rows.len());

    while !remaining.is_empty() {
        let mut progress = false;
        let mut next_remaining = Vec::new();

        for row in remaining {
            let parent_ready = row.parent_id.as_ref().is_none_or(|parent_id| {
                let parent_key = (row.account_id.clone(), parent_id.clone());
                !input_keys.contains(&parent_key) || inserted.contains(&parent_key)
            });

            if parent_ready {
                inserted.insert((row.account_id.clone(), row.id.clone()));
                sorted.push(row);
                progress = true;
            } else {
                next_remaining.push(row);
            }
        }

        if !progress {
            return Err("folder parent cycle or unresolved parent in batch".to_owned());
        }

        remaining = next_remaining;
    }

    Ok(sorted)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::validate_label_color_pairs;

    #[test]
    fn label_color_pairs_accept_complete_or_missing() {
        validate_label_color_pairs("lbl", Some("#111111"), Some("#ffffff"), None, None).unwrap();
        validate_label_color_pairs("lbl", None, None, Some("#222222"), Some("#000000")).unwrap();
        validate_label_color_pairs("lbl", None, None, None, None).unwrap();
    }

    #[test]
    fn label_color_pairs_reject_partial_values() {
        let server = validate_label_color_pairs("lbl", Some("#111111"), None, None, None)
            .expect_err("partial server color should fail");
        assert!(server.contains("incomplete server color pair"));

        let user = validate_label_color_pairs("lbl", None, None, None, Some("#ffffff"))
            .expect_err("partial user color should fail");
        assert!(user.contains("incomplete user color pair"));
    }
}
