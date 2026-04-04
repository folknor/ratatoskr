use cmdk::OptionItem;
use rusqlite::{params, Connection};

/// User-visible folders/labels for an account, excluding system labels.
///
/// For Gmail, splits `/`-delimited labels into path segments.
/// Returns `OptionItem`s for the palette's ListPicker stage 2.
pub fn get_user_folders_for_palette(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<OptionItem>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name FROM labels
             WHERE account_id = ?1 AND type != 'system' AND visible = 1
             ORDER BY sort_order ASC, name ASC",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map(params![account_id], |row| {
        let id: String = row.get("id")?;
        let name: String = row.get("name")?;
        Ok((id, name))
    })
    .map_err(|e| e.to_string())?
    .map(|r| {
        let (id, name) = r.map_err(|e| e.to_string())?;
        Ok(label_name_to_option_item(id, &name))
    })
    .collect::<Result<Vec<_>, String>>()
}

/// All user labels for an account (same as folders for now).
pub fn get_user_labels_for_palette(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<OptionItem>, String> {
    get_user_folders_for_palette(conn, account_id)
}

/// Labels currently applied to a specific thread.
pub fn get_thread_labels_for_palette(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
) -> Result<Vec<OptionItem>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT l.id, l.name FROM labels l
             INNER JOIN thread_labels tl
               ON tl.account_id = l.account_id AND tl.label_id = l.id
             WHERE tl.account_id = ?1 AND tl.thread_id = ?2
               AND l.type != 'system' AND l.visible = 1
             ORDER BY l.sort_order ASC, l.name ASC",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map(params![account_id, thread_id], |row| {
        let id: String = row.get("id")?;
        let name: String = row.get("name")?;
        Ok((id, name))
    })
    .map_err(|e| e.to_string())?
    .map(|r| {
        let (id, name) = r.map_err(|e| e.to_string())?;
        Ok(label_name_to_option_item(id, &name))
    })
    .collect::<Result<Vec<_>, String>>()
}

/// All user labels across all accounts, with account name in path.
///
/// Each `OptionItem.id` is encoded as `"account_id:kind:label_id"` where
/// kind is `f` (folder/container) or `t` (tag) so the palette can
/// construct the correct typed `SidebarSelection` variant.
pub fn get_all_labels_cross_account(conn: &Connection) -> Result<Vec<OptionItem>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT a.id AS account_id,
                    COALESCE(a.display_name, a.email) AS account_name,
                    l.id AS label_id,
                    l.name AS label_name,
                    l.label_kind
             FROM labels l
             INNER JOIN accounts a ON a.id = l.account_id
             WHERE l.type != 'system' AND l.visible = 1 AND a.is_active = 1
             ORDER BY a.email ASC, l.sort_order ASC, l.name ASC",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map([], |row| {
        let account_id: String = row.get("account_id")?;
        let account_name: String = row.get("account_name")?;
        let label_id: String = row.get("label_id")?;
        let label_name: String = row.get("label_name")?;
        let label_kind: String = row.get("label_kind")?;
        Ok((account_id, account_name, label_id, label_name, label_kind))
    })
    .map_err(|e| e.to_string())?
    .map(|r| {
        let (account_id, account_name, label_id, label_name, label_kind) =
            r.map_err(|e| e.to_string())?;
        let mut item = label_name_to_option_item(label_id.clone(), &label_name);
        let mut new_path = vec![account_name];
        if let Some(existing) = item.path.take() {
            new_path.extend(existing);
        }
        item.path = Some(new_path);
        let kind = if label_kind == "tag" { "t" } else { "f" };
        item.id = format!("{account_id}:{kind}:{label_id}");
        Ok(item)
    })
    .collect::<Result<Vec<_>, String>>()
}

/// Check whether an account uses folder-based semantics (Exchange/IMAP/JMAP)
/// as opposed to tag-based (Gmail). Folder-based providers don't support
/// Add Label / Remove Label — only Move to Folder.
pub fn is_folder_based_provider(conn: &Connection, account_id: &str) -> Result<bool, String> {
    let provider: String = conn
        .query_row(
            "SELECT provider FROM accounts WHERE id = ?1",
            params![account_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(provider != "gmail_api")
}

/// Convert a label name to an `OptionItem`, splitting `/`-delimited names
/// into path segments (Gmail convention).
fn label_name_to_option_item(id: String, name: &str) -> OptionItem {
    let segments: Vec<&str> = name.split('/').collect();
    let (label, path) = if segments.len() > 1 {
        let label = segments.last().unwrap_or(&name).to_string();
        let path: Vec<String> = segments[..segments.len() - 1]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        (label, Some(path))
    } else {
        (name.to_string(), None)
    };

    OptionItem {
        id,
        label,
        path,
        keywords: None,
        disabled: false,
    }
}
