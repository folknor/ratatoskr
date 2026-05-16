use super::types::ImapFolder;
use common::folder_roles::{imap_name_to_special_use, system_folder_by_imap_special_use};

/// Mapping from an IMAP folder to a Ratatoskr folder.
#[derive(Debug, Clone)]
pub struct FolderMapping {
    pub folder_id: String,
    pub folder_name: String,
    pub folder_type: String,
}

/// Map IMAP special-use flags to Ratatoskr folder IDs.
fn special_use_mapping(special_use: &str) -> Option<FolderMapping> {
    let mapping = system_folder_by_imap_special_use(special_use)?;
    Some(FolderMapping {
        folder_id: mapping.label_id.to_string(),
        folder_name: mapping.label_name.to_string(),
        folder_type: "system".to_string(),
    })
}

/// Well-known folder names (case-insensitive) → special-use attribute.
fn folder_name_to_special_use(name: &str) -> Option<&'static str> {
    imap_name_to_special_use(name)
}

/// Map an IMAP folder to a Ratatoskr folder.
pub fn map_folder_to_folder(folder: &ImapFolder) -> FolderMapping {
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
    FolderMapping {
        folder_id: format!("folder-{}", folder.path),
        folder_name: folder.name.clone(),
        folder_type: "user".to_string(),
    }
}

/// Folder IDs that a message in a given folder should have. The `DRAFT`
/// system folder is added when the message carries the `\Draft` flag, so
/// drafts surface in the universal Drafts view regardless of which folder
/// the server actually stores them in.
pub fn get_folder_ids_for_message(folder_id: &str, is_draft: bool) -> Vec<String> {
    let mut folders = vec![folder_id.to_string()];
    if is_draft {
        folders.push("DRAFT".to_string());
    }
    folders
}

/// Filter syncable folders (exclude Gmail parent containers etc).
pub fn get_syncable_folders(folders: &[ImapFolder]) -> Vec<&ImapFolder> {
    folders
        .iter()
        .filter(|f| {
            let lower = f.path.to_lowercase();
            let is_flagged_virtual = f.special_use.as_deref() == Some("\\Flagged");
            lower != "[gmail]"
                && lower != "[google mail]"
                && !lower.starts_with("[nostromo]")
                && !is_flagged_virtual
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
        let m = map_folder_to_folder(&f);
        assert_eq!(m.folder_id, "INBOX");
        assert_eq!(m.folder_type, "system");
    }

    #[test]
    fn test_name_fallback_sent() {
        let f = make_folder("Sent Items", "Sent Items", None);
        let m = map_folder_to_folder(&f);
        assert_eq!(m.folder_id, "SENT");
    }

    #[test]
    fn test_user_folder() {
        let f = make_folder("Work/Projects", "Projects", None);
        let m = map_folder_to_folder(&f);
        assert_eq!(m.folder_id, "folder-Work/Projects");
        assert_eq!(m.folder_type, "user");
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
    fn test_get_folder_ids_for_message() {
        let folders = get_folder_ids_for_message("INBOX", false);
        assert_eq!(folders, vec!["INBOX".to_string()]);
    }

    #[test]
    fn test_get_folder_ids_for_draft() {
        // Production input is the canonical "DRAFT" id from
        // map_folder_to_folder (special-use `\Drafts` or name-fallback).
        // The function adds the universal DRAFT marker so drafts surface
        // in the universal Drafts view regardless of the source folder id.
        let folders = get_folder_ids_for_message("DRAFT", true);
        assert_eq!(folders, vec!["DRAFT".to_string(), "DRAFT".to_string()]);
    }
}
