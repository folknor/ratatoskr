use std::collections::HashMap;

use common::folder_roles::graph_well_known_aliases;
use common::label_flags::{assemble_labels, prefixed_labels};

use super::types::GraphMailFolder;

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
    pub label_id: String,
    pub label_name: String,
    pub label_type: &'static str, // "system" or "user"
    /// The parent folder's label ID, if this is a nested folder.
    pub parent_label_id: Option<String>,
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
        // First pass: build opaque_id → label_id map for parent resolution
        let opaque_to_label: HashMap<&str, String> = all_folders
            .iter()
            .map(|folder| {
                let label_id = if let Some(&(lid, _)) = resolved_wellknown.get(&folder.id) {
                    lid.to_string()
                } else {
                    format!("graph-{}", folder.id)
                };
                (folder.id.as_str(), label_id)
            })
            .collect();

        let mut by_id = HashMap::new();
        let mut by_label = HashMap::new();

        for folder in all_folders {
            let parent_label_id = folder
                .parent_folder_id
                .as_deref()
                .and_then(|pid| opaque_to_label.get(pid))
                .cloned();

            let mapping = if let Some(&(label_id, label_name)) = resolved_wellknown.get(&folder.id)
            {
                FolderLabelMapping {
                    label_id: label_id.to_string(),
                    label_name: label_name.to_string(),
                    label_type: "system",
                    parent_label_id,
                }
            } else {
                FolderLabelMapping {
                    label_id: format!("graph-{}", folder.id),
                    label_name: folder.display_name.clone(),
                    label_type: "user",
                    parent_label_id,
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
        let primary_labels = self
            .by_id
            .get(parent_folder_id)
            .map(|mapping| vec![mapping.label_id.clone()])
            .unwrap_or_default();
        let category_refs = categories.iter().map(String::as_str);

        assemble_labels(
            primary_labels,
            prefixed_labels("cat:", category_refs),
            is_read,
            flag_status == "flagged",
            false,
        )
    }

    /// Return all mappings (for list_folders trait method).
    pub fn all_mappings(&self) -> impl Iterator<Item = &FolderLabelMapping> {
        self.by_id.values()
    }

    /// Iterate over all (opaque_folder_id, mapping) pairs.
    /// Used by sync to enumerate folders for message fetching and delta tokens.
    pub fn folder_entries(&self) -> impl Iterator<Item = (&str, &FolderLabelMapping)> + '_ {
        self.by_id.iter().map(|(fid, m)| (fid.as_str(), m))
    }

    /// The well-known alias list (used by the resolution step in sync).
    pub fn well_known_aliases() -> Vec<(&'static str, &'static str, &'static str)> {
        graph_well_known_aliases()
    }
}
