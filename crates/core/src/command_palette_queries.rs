use cmdk::OptionItem;
use crate::db::Connection;

use crate::db::queries_extra::command_palette;

/// User-visible folders/labels for an account.
pub fn get_user_folders_for_palette(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<OptionItem>, String> {
    Ok(command_palette::get_user_labels_for_account_sync(conn, account_id)?
        .into_iter()
        .map(|r| label_name_to_option_item(r.id, &r.name))
        .collect())
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
    Ok(command_palette::get_thread_labels_sync(conn, account_id, thread_id)?
        .into_iter()
        .map(|r| label_name_to_option_item(r.id, &r.name))
        .collect())
}

/// All user labels across all accounts, with account name in path.
pub fn get_all_labels_cross_account(conn: &Connection) -> Result<Vec<OptionItem>, String> {
    command_palette::get_all_labels_cross_account_sync(conn)?
        .into_iter()
        .map(|r| {
            let mut item = label_name_to_option_item(r.label_id.clone(), &r.label_name);
            let mut new_path = vec![r.account_name];
            if let Some(existing) = item.path.take() {
                new_path.extend(existing);
            }
            item.path = Some(new_path);
            let kind = if r.label_kind == "tag" { "t" } else { "f" };
            item.id = format!("{}:{kind}:{}", r.account_id, r.label_id);
            Ok(item)
        })
        .collect::<Result<Vec<_>, String>>()
}

/// Check whether an account uses folder-based semantics.
pub fn is_folder_based_provider(conn: &Connection, account_id: &str) -> Result<bool, String> {
    command_palette::is_folder_based_provider_sync(conn, account_id)
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
