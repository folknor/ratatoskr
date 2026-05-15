use super::Sidebar;

/// Build a scope query prefix for "Search here" from a label/folder name.
pub(super) fn build_search_here_prefix(name: &str, sidebar: &Sidebar) -> String {
    if let Some(idx) = sidebar.selected_account_index() {
        let account_name = sidebar
            .accounts
            .get(idx)
            .map(|a| a.display_name.as_deref().unwrap_or(&a.email))
            .unwrap_or("Unknown");
        format!(
            "account:{} label:{} ",
            quote_if_needed(account_name),
            quote_if_needed(name),
        )
    } else {
        format!("label:{} ", quote_if_needed(name))
    }
}

/// Build a scope query prefix for provider folders. These use `folder:`
/// because `label:` is reserved for tag-kind labels in search.
pub(super) fn build_search_here_user_folder_prefix(name: &str, sidebar: &Sidebar) -> String {
    if let Some(idx) = sidebar.selected_account_index() {
        let account_name = sidebar
            .accounts
            .get(idx)
            .map(|a| a.display_name.as_deref().unwrap_or(&a.email))
            .unwrap_or("Unknown");
        format!(
            "account:{} folder:{} ",
            quote_if_needed(account_name),
            quote_if_needed(name),
        )
    } else {
        format!("folder:{} ", quote_if_needed(name))
    }
}

/// Build a scope query prefix for universal folders (Inbox, Sent, etc.).
pub(super) fn build_search_here_folder_prefix(folder_name: &str, sidebar: &Sidebar) -> String {
    if let Some(idx) = sidebar.selected_account_index() {
        let account_name = sidebar
            .accounts
            .get(idx)
            .map(|a| a.display_name.as_deref().unwrap_or(&a.email))
            .unwrap_or("Unknown");
        format!(
            "account:{} in:{} ",
            quote_if_needed(account_name),
            folder_name.to_lowercase(),
        )
    } else {
        format!("in:{} ", folder_name.to_lowercase())
    }
}

/// Wrap a value in quotes if it contains spaces.
fn quote_if_needed(s: &str) -> String {
    if s.contains(' ') {
        format!("\"{s}\"")
    } else {
        s.to_string()
    }
}
