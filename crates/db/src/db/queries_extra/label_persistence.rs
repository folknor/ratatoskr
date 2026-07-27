use std::collections::HashSet;

use rusqlite::params;

use crate::db::WriteTxn;

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
    pub owner_id: Option<String>,
    pub content_class: Option<String>,
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

pub trait LabelPersistenceTarget {
    fn execute<P: rusqlite::Params>(&self, sql: &str, params: P) -> rusqlite::Result<usize>;
}

impl LabelPersistenceTarget for WriteTxn<'_> {
    fn execute<P: rusqlite::Params>(&self, sql: &str, params: P) -> rusqlite::Result<usize> {
        WriteTxn::execute(self, sql, params)
    }
}

impl LabelPersistenceTarget for rusqlite::Transaction<'_> {
    fn execute<P: rusqlite::Params>(&self, sql: &str, params: P) -> rusqlite::Result<usize> {
        std::ops::Deref::deref(self).execute(sql, params)
    }
}

/// The column list / VALUES half shared by the authoritative upsert and the
/// insert-if-absent variant, so the two can never drift apart on columns.
const FOLDER_INSERT_SQL: &str = "\
    INSERT INTO folders \
     (id, account_id, name, visible, sort_order, imap_folder_path, imap_special_use, \
      namespace_type, owner_id, content_class, parent_id, right_read, right_add, right_remove, right_set_seen, \
      right_set_keywords, right_create_child, right_rename, right_delete, right_submit, \
      is_subscribed, is_undeletable) \
     VALUES (?1, ?2, ?3, COALESCE(?4, 1), COALESCE(?5, 0), ?6, ?7, ?8, ?9, ?10, \
             ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22) ";

/// Ensure a `folders` row EXISTS for each id without touching an existing one.
///
/// The counterpart to `insert_folders_batch`, for callers that only need the
/// row to satisfy `message_folders`' / `thread_folders`' foreign key and have
/// nothing authoritative to say about the folder itself. `insert_folders_batch`
/// is the container-sync seam: its `ON CONFLICT ... DO UPDATE` assigns
/// `excluded.*` unconditionally so a revoked share really does lose its rights
/// and a reparented folder really does lose its old `parent_id`. A caller that
/// mints a row from a `FolderKind` alone carries `NULL` in every one of those
/// columns, so routing it through that upsert silently BLANKS a shared folder's
/// `namespace_type`, `owner_id` and `right_*` set - which reads downstream as a
/// personal folder with unreported (therefore permitted) rights, defeating the
/// B12 obstacle K read-only preflight. `DO NOTHING` keeps the container
/// projection authoritative wherever it has already run.
pub fn ensure_folder_rows<T: LabelPersistenceTarget + ?Sized>(
    tx: &T,
    rows: &[FolderWriteRow],
) -> Result<(), String> {
    let sql = format!("{FOLDER_INSERT_SQL}ON CONFLICT(account_id, id) DO NOTHING");
    for row in sort_folders_parent_first(rows)? {
        tx.execute(&sql, folder_row_params(row).as_slice())
            .map_err(|e| format!("ensure folder: {e}"))?;
    }
    Ok(())
}

fn folder_row_params(row: &FolderWriteRow) -> Vec<&dyn rusqlite::types::ToSql> {
    vec![
        &row.id,
        &row.account_id,
        &row.name,
        &row.visible,
        &row.sort_order,
        &row.imap_folder_path,
        &row.imap_special_use,
        &row.namespace_type,
        &row.owner_id,
        &row.content_class,
        &row.parent_id,
        &row.right_read,
        &row.right_add,
        &row.right_remove,
        &row.right_set_seen,
        &row.right_set_keywords,
        &row.right_create_child,
        &row.right_rename,
        &row.right_delete,
        &row.right_submit,
        &row.is_subscribed,
        &row.is_undeletable,
    ]
}

pub fn insert_folders_batch<T: LabelPersistenceTarget + ?Sized>(
    tx: &T,
    rows: &[FolderWriteRow],
) -> Result<(), String> {
    let sorted_rows = sort_folders_parent_first(rows)?;

    let sql = format!(
        "{FOLDER_INSERT_SQL}\
         ON CONFLICT(account_id, id) DO UPDATE SET \
               name = excluded.name, \
               visible = excluded.visible, \
               sort_order = COALESCE(excluded.sort_order, folders.sort_order), \
               imap_folder_path = excluded.imap_folder_path, \
               imap_special_use = excluded.imap_special_use, \
               namespace_type = excluded.namespace_type, \
               owner_id = excluded.owner_id, \
               content_class = excluded.content_class, \
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
    );

    for row in sorted_rows {
        tx.execute(&sql, folder_row_params(row).as_slice())
            .map_err(|e| format!("upsert folder: {e}"))?;
    }

    Ok(())
}

/// Upsert `labels` rows. `is_undeletable` uses OR semantics on conflict
/// so the invariant holds even if a later sync pass forgets to set the
/// flag: once a row is marked undeletable (e.g. by the bootstrap synth
/// for `importance:*`, the typed action-side label writer, or a
/// system-flag classification at ingest), it stays that way.
pub fn upsert_labels<T: LabelPersistenceTarget + ?Sized>(
    tx: &T,
    rows: &[LabelWriteRow],
) -> Result<(), String> {
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
    use rusqlite::Connection;

    use super::{
        FolderWriteRow, ensure_folder_rows, insert_folders_batch, validate_label_color_pairs,
    };

    fn bare_row(id: &str, name: &str) -> FolderWriteRow {
        FolderWriteRow {
            id: id.to_string(),
            account_id: "acc".to_string(),
            name: name.to_string(),
            visible: None,
            sort_order: None,
            imap_folder_path: None,
            imap_special_use: None,
            namespace_type: None,
            owner_id: None,
            content_class: None,
            parent_id: None,
            right_read: None,
            right_add: None,
            right_remove: None,
            right_set_seen: None,
            right_set_keywords: None,
            right_create_child: None,
            right_rename: None,
            right_delete: None,
            right_submit: None,
            is_subscribed: None,
            is_undeletable: false,
        }
    }

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run_all(&conn).unwrap();
        conn.execute(
            "INSERT INTO accounts (id, email, provider) VALUES ('acc', 'a@example.com', 'imap')",
            [],
        )
        .unwrap();
        conn
    }

    /// The membership-side folder mint must never redefine a row the container
    /// projection already wrote. Blanking `namespace_type` / `right_*` here is
    /// what let a read-only IMAP share pass the shared-mailbox rights preflight.
    #[test]
    fn ensure_folder_rows_preserves_container_metadata() {
        let mut conn = setup_conn();
        let tx = conn.transaction().unwrap();

        let shared = FolderWriteRow {
            namespace_type: Some("shared".to_string()),
            owner_id: Some("alice@example.test".to_string()),
            right_read: Some(1),
            right_set_seen: Some(0),
            ..bare_row(
                "shared:alice@example.test:folder-#user/alice/Read only",
                "Read only",
            )
        };
        insert_folders_batch(&tx, std::slice::from_ref(&shared)).unwrap();

        // What the consumer mints from a `FolderKind` alone.
        ensure_folder_rows(
            &tx,
            &[FolderWriteRow {
                namespace_type: Some("shared".to_string()),
                owner_id: Some("alice@example.test".to_string()),
                ..bare_row(&shared.id, &shared.id)
            }],
        )
        .unwrap();

        let (name, namespace, right_set_seen) = tx
            .query_row(
                "SELECT name, namespace_type, right_set_seen FROM folders WHERE id = ?1",
                [&shared.id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(name, "Read only");
        assert_eq!(namespace.as_deref(), Some("shared"));
        assert_eq!(right_set_seen, Some(0));
    }

    /// The same call still CREATES the row when the container pass has not run,
    /// which is the foreign-key guarantee membership writes depend on.
    #[test]
    fn ensure_folder_rows_creates_missing_row() {
        let mut conn = setup_conn();
        let tx = conn.transaction().unwrap();
        ensure_folder_rows(&tx, &[bare_row("folder-Projects", "folder-Projects")]).unwrap();
        let count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM folders WHERE id = 'folder-Projects'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    /// `insert_folders_batch` keeps its authoritative overwrite: a revoked share
    /// must actually lose its namespace and rights on the next container pass.
    #[test]
    fn insert_folders_batch_still_clears_revoked_metadata() {
        let mut conn = setup_conn();
        let tx = conn.transaction().unwrap();
        insert_folders_batch(
            &tx,
            &[FolderWriteRow {
                namespace_type: Some("shared".to_string()),
                owner_id: Some("alice@example.test".to_string()),
                right_set_seen: Some(0),
                ..bare_row("folder-Shared", "Shared")
            }],
        )
        .unwrap();
        insert_folders_batch(&tx, &[bare_row("folder-Shared", "Shared")]).unwrap();
        let (namespace, right_set_seen) = tx
            .query_row(
                "SELECT namespace_type, right_set_seen FROM folders WHERE id = 'folder-Shared'",
                [],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(namespace, None);
        assert_eq!(right_set_seen, None);
    }

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
