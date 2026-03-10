use std::collections::HashMap;

use super::types::GraphMailFolder;

/// Well-known folder aliases that Graph accepts as URL path segments.
/// (graph_alias, label_id, label_name)
const WELL_KNOWN_ALIASES: &[(&str, &str, &str)] = &[
    ("inbox", "INBOX", "Inbox"),
    ("drafts", "DRAFT", "Drafts"),
    ("sentitems", "SENT", "Sent"),
    ("deleteditems", "TRASH", "Trash"),
    ("junkemail", "SPAM", "Spam"),
    ("archive", "archive", "Archive"),
];

/// Runtime mapping between opaque Graph folder IDs and Gmail-style label IDs.
#[derive(Debug, Clone)]
pub struct FolderMap {
    /// opaque_folder_id → FolderLabelMapping
    by_id: HashMap<String, FolderLabelMapping>,
    /// label_id → opaque_folder_id (reverse lookup for actions)
    by_label: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct FolderLabelMapping {
    pub folder_id: String,
    pub label_id: String,
    pub label_name: String,
    pub label_type: &'static str, // "system" or "user"
}

impl FolderMap {
    /// Build a folder map from resolved well-known IDs and the full folder list.
    ///
    /// `resolved_wellknown` maps opaque folder IDs to (label_id, label_name)
    /// for system folders that were resolved by calling
    /// `GET /me/mailFolders/{alias}` for each well-known alias.
    pub fn build(
        resolved_wellknown: &HashMap<String, (&str, &str)>,
        all_folders: &[GraphMailFolder],
    ) -> Self {
        let mut by_id = HashMap::new();
        let mut by_label = HashMap::new();

        for folder in all_folders {
            let mapping = if let Some(&(label_id, label_name)) =
                resolved_wellknown.get(&folder.id)
            {
                FolderLabelMapping {
                    folder_id: folder.id.clone(),
                    label_id: label_id.to_string(),
                    label_name: label_name.to_string(),
                    label_type: "system",
                }
            } else {
                FolderLabelMapping {
                    folder_id: folder.id.clone(),
                    label_id: format!("graph-{}", folder.id),
                    label_name: folder.display_name.clone(),
                    label_type: "user",
                }
            };

            by_label.insert(mapping.label_id.clone(), folder.id.clone());
            by_id.insert(folder.id.clone(), mapping);
        }

        Self { by_id, by_label }
    }

    /// Look up a folder's label info by its opaque Graph ID.
    pub fn get_by_folder_id(&self, folder_id: &str) -> Option<&FolderLabelMapping> {
        self.by_id.get(folder_id)
    }

    /// Resolve a label ID to an opaque Graph folder ID.
    pub fn resolve_folder_id(&self, label_id: &str) -> Option<&str> {
        self.by_label.get(label_id).map(String::as_str)
    }

    /// Derive label IDs for a message from its folder, categories, and flags.
    pub fn get_labels_for_message(
        &self,
        parent_folder_id: &str,
        categories: &[String],
        is_read: bool,
        flag_status: &str,
    ) -> Vec<String> {
        let mut labels = Vec::new();

        // Primary folder label
        if let Some(mapping) = self.by_id.get(parent_folder_id) {
            labels.push(mapping.label_id.clone());
        }

        // Categories as supplementary labels
        for cat in categories {
            labels.push(format!("cat:{cat}"));
        }

        // Pseudo-labels from flags
        if !is_read {
            labels.push("UNREAD".to_string());
        }
        if flag_status == "flagged" {
            labels.push("STARRED".to_string());
        }

        labels
    }

    /// Return all mappings (for list_folders trait method).
    pub fn all_mappings(&self) -> impl Iterator<Item = &FolderLabelMapping> {
        self.by_id.values()
    }

    /// The well-known alias list (used by the resolution step in sync).
    pub fn well_known_aliases() -> &'static [(&'static str, &'static str, &'static str)] {
        WELL_KNOWN_ALIASES
    }
}
