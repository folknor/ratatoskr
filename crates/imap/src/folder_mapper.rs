use super::types::ImapFolder;
use ratatoskr_provider_utils::folder_roles::{imap_name_to_special_use, system_folder_by_imap_special_use};

/// Mapping from an IMAP folder to a Gmail-style label.
#[derive(Debug, Clone)]
pub struct FolderLabelMapping {
    pub label_id: String,
    pub label_name: String,
    pub label_type: String,
}

/// Map IMAP special-use flags to Gmail-style label IDs.
fn special_use_mapping(special_use: &str) -> Option<FolderLabelMapping> {
    let mapping = system_folder_by_imap_special_use(special_use)?;
    Some(FolderLabelMapping {
        label_id: mapping.label_id.to_string(),
        label_name: mapping.label_name.to_string(),
        label_type: "system".to_string(),
    })
}

/// Well-known folder names (case-insensitive) → special-use attribute.
fn folder_name_to_special_use(name: &str) -> Option<&'static str> {
    imap_name_to_special_use(name)
}

/// Map an IMAP folder to a Gmail-style label.
pub fn map_folder_to_label(folder: &ImapFolder) -> FolderLabelMapping {
    // Check special-use attribute first
    if let Some(ref su) = folder.special_use
        && let Some(mapping) = special_use_mapping(su)
    {
        return mapping;
    }

    // Fall back to name-based detection
    let lower_path = folder.path.to_lowercase();
    let lower_name = folder.name.to_lowercase();

    if let Some(su) =
        folder_name_to_special_use(&lower_path).or_else(|| folder_name_to_special_use(&lower_name))
        && let Some(mapping) = special_use_mapping(su)
    {
        return mapping;
    }

    // User-defined folder
    FolderLabelMapping {
        label_id: format!("folder-{}", folder.path),
        label_name: folder.name.clone(),
        label_type: "user".to_string(),
    }
}

/// Get label IDs that a message in a given folder should have.
pub fn get_labels_for_message(
    folder_label_id: &str,
    is_read: bool,
    is_starred: bool,
    is_draft: bool,
) -> Vec<String> {
    let mut labels = vec![folder_label_id.to_string()];
    if !is_read {
        labels.push("UNREAD".to_string());
    }
    if is_starred {
        labels.push("STARRED".to_string());
    }
    if is_draft {
        labels.push("DRAFT".to_string());
    }
    labels
}

/// Filter syncable folders (exclude Gmail parent containers etc).
pub fn get_syncable_folders(folders: &[ImapFolder]) -> Vec<&ImapFolder> {
    folders
        .iter()
        .filter(|f| {
            let lower = f.path.to_lowercase();
            lower != "[gmail]" && lower != "[google mail]" && !lower.starts_with("[nostromo]")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_folder(path: &str, name: &str, special_use: Option<&str>) -> ImapFolder {
        ImapFolder {
            path: path.to_string(),
            raw_path: path.to_string(),
            name: name.to_string(),
            delimiter: "/".to_string(),
            special_use: special_use.map(ToString::to_string),
            exists: 10,
            unseen: 2,
            namespace_type: None,
        }
    }

    #[test]
    fn test_special_use_inbox() {
        let f = make_folder("INBOX", "INBOX", Some("\\Inbox"));
        let m = map_folder_to_label(&f);
        assert_eq!(m.label_id, "INBOX");
        assert_eq!(m.label_type, "system");
    }

    #[test]
    fn test_name_fallback_sent() {
        let f = make_folder("Sent Items", "Sent Items", None);
        let m = map_folder_to_label(&f);
        assert_eq!(m.label_id, "SENT");
    }

    #[test]
    fn test_user_folder() {
        let f = make_folder("Work/Projects", "Projects", None);
        let m = map_folder_to_label(&f);
        assert_eq!(m.label_id, "folder-Work/Projects");
        assert_eq!(m.label_type, "user");
    }

    #[test]
    fn test_syncable_folders() {
        let folders = vec![
            make_folder("INBOX", "INBOX", Some("\\Inbox")),
            make_folder("[Gmail]", "[Gmail]", None),
            make_folder("[Gmail]/All Mail", "All Mail", Some("\\All")),
        ];
        let syncable = get_syncable_folders(&folders);
        assert_eq!(syncable.len(), 2);
        assert_eq!(syncable[0].path, "INBOX");
        assert_eq!(syncable[1].path, "[Gmail]/All Mail");
    }

    #[test]
    fn test_get_labels_for_message() {
        let labels = get_labels_for_message("INBOX", false, true, false);
        assert!(labels.contains(&"INBOX".to_string()));
        assert!(labels.contains(&"UNREAD".to_string()));
        assert!(labels.contains(&"STARRED".to_string()));
        assert!(!labels.contains(&"DRAFT".to_string()));
    }
}
