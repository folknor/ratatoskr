use crate::db::ReadConn;
use cmdk::OptionItem;

use crate::db::queries_extra::command_palette;

/// User-visible folders for an account, excluding system folders.
pub fn get_user_folders_for_palette(
    conn: &ReadConn<'_>,
    account_id: &str,
) -> Result<Vec<OptionItem>, String> {
    Ok(
        command_palette::get_user_folders_for_account_sync(conn, account_id)?
            .into_iter()
            .map(|r| label_name_to_option_item(r.id, &r.name))
            .collect(),
    )
}

/// All user-visible label groups.
pub fn get_label_groups_for_palette(conn: &ReadConn<'_>) -> Result<Vec<OptionItem>, String> {
    Ok(command_palette::get_label_groups_for_palette_sync(conn)?
        .into_iter()
        .map(|r| label_name_to_option_item(r.id.to_string(), &r.name))
        .collect())
}

/// Label groups currently rendered for a specific thread.
pub fn get_thread_label_groups_for_palette(
    conn: &ReadConn<'_>,
    account_id: &str,
    thread_id: &str,
) -> Result<Vec<OptionItem>, String> {
    Ok(
        command_palette::get_thread_label_groups_sync(conn, account_id, thread_id)?
            .into_iter()
            .map(|r| label_name_to_option_item(r.id.to_string(), &r.name))
            .collect(),
    )
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
