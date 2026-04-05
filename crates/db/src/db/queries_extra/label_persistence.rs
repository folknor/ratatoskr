use rusqlite::{Transaction, params};

#[derive(Debug, Clone)]
pub struct LabelWriteRow {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub label_type: String,
    pub label_kind: String,
    pub color_bg: Option<String>,
    pub color_fg: Option<String>,
    pub sort_order: Option<i64>,
    pub imap_folder_path: Option<String>,
    pub imap_special_use: Option<String>,
    pub parent_label_id: Option<String>,
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
}

pub fn upsert_labels(tx: &Transaction, rows: &[LabelWriteRow]) -> Result<(), String> {
    for row in rows {
        tx.execute(
            "INSERT INTO labels \
             (id, account_id, name, type, label_kind, color_bg, color_fg, sort_order, \
              imap_folder_path, imap_special_use, parent_label_id, \
              right_read, right_add, right_remove, right_set_seen, \
              right_set_keywords, right_create_child, right_rename, \
              right_delete, right_submit, is_subscribed) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, COALESCE(?8, 0), ?9, ?10, ?11, \
                     ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21) \
             ON CONFLICT(account_id, id) DO UPDATE SET \
               name = excluded.name, \
               type = excluded.type, \
               label_kind = excluded.label_kind, \
               color_bg = excluded.color_bg, \
               color_fg = excluded.color_fg, \
               sort_order = COALESCE(excluded.sort_order, labels.sort_order), \
               imap_folder_path = excluded.imap_folder_path, \
               imap_special_use = excluded.imap_special_use, \
               parent_label_id = excluded.parent_label_id, \
               right_read = excluded.right_read, \
               right_add = excluded.right_add, \
               right_remove = excluded.right_remove, \
               right_set_seen = excluded.right_set_seen, \
               right_set_keywords = excluded.right_set_keywords, \
               right_create_child = excluded.right_create_child, \
               right_rename = excluded.right_rename, \
               right_delete = excluded.right_delete, \
               right_submit = excluded.right_submit, \
               is_subscribed = excluded.is_subscribed",
            params![
                row.id,
                row.account_id,
                row.name,
                row.label_type,
                row.label_kind,
                row.color_bg,
                row.color_fg,
                row.sort_order,
                row.imap_folder_path,
                row.imap_special_use,
                row.parent_label_id,
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
            ],
        )
        .map_err(|e| format!("upsert label: {e}"))?;
    }

    Ok(())
}
