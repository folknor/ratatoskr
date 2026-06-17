use std::collections::HashMap;

use common::folder_roles::graph_well_known_aliases;
use common::types::{FolderKind, LabelKind};

use super::types::GraphMailFolder;

/// Runtime mapping between opaque Graph folder IDs and Ratatoskr folder IDs.
#[derive(Debug, Clone)]
pub struct FolderMap {
    /// opaque Graph folder ID -> Ratatoskr folder mapping
    by_graph_id: HashMap<String, FolderMapping>,
    /// Ratatoskr folder ID -> opaque Graph folder ID (reverse lookup for actions)
    by_folder_id: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct FolderMapping {
    pub folder_id: String,
    pub folder_name: String,
    pub folder_type: &'static str, // "system" or "user"
    /// The parent folder's Ratatoskr folder ID, if this is a nested folder.
    pub parent_folder_id: Option<String>,
}

impl FolderMap {
    /// Build a folder map from resolved well-known IDs and the full folder list.
    ///
    /// `resolved_wellknown` maps opaque folder IDs to (folder_id, folder_name)
    /// for system folders that were resolved by calling
    /// `GET /me/mailFolders/{alias}` for each well-known alias.
    pub fn build(
        resolved_wellknown: &HashMap<String, (&str, &str)>,
        all_folders: &[GraphMailFolder],
    ) -> Result<Self, String> {
        // First pass: build opaque Graph ID -> Ratatoskr folder ID map for parent resolution.
        let graph_to_folder_id: HashMap<&str, String> = all_folders
            .iter()
            .map(|folder| {
                let folder_id = if let Some(&(lid, _)) = resolved_wellknown.get(&folder.id) {
                    lid.to_string()
                } else {
                    FolderKind::graph_user(&folder.id)?.storage_id()
                };
                Ok((folder.id.as_str(), folder_id))
            })
            .collect::<Result<_, String>>()?;

        let mut by_graph_id = HashMap::new();
        let mut by_folder_id = HashMap::new();

        for folder in all_folders {
            let parent_folder_id = folder
                .parent_folder_id
                .as_deref()
                .and_then(|pid| graph_to_folder_id.get(pid))
                .cloned();

            let mapping =
                if let Some(&(folder_id, folder_name)) = resolved_wellknown.get(&folder.id) {
                    FolderMapping {
                        folder_id: folder_id.to_string(),
                        folder_name: folder_name.to_string(),
                        folder_type: "system",
                        parent_folder_id,
                    }
                } else {
                    FolderMapping {
                        folder_id: FolderKind::graph_user(&folder.id)?.storage_id(),
                        folder_name: folder.display_name.clone(),
                        folder_type: "user",
                        parent_folder_id,
                    }
                };

            by_folder_id.insert(mapping.folder_id.clone(), folder.id.clone());
            by_graph_id.insert(folder.id.clone(), mapping);
        }

        Ok(Self {
            by_graph_id,
            by_folder_id,
        })
    }

    /// Look up a folder mapping by its opaque Graph ID.
    pub fn get_by_graph_folder_id(&self, graph_folder_id: &str) -> Option<&FolderMapping> {
        self.by_graph_id.get(graph_folder_id)
    }

    /// Resolve a Ratatoskr folder ID to an opaque Graph folder ID.
    pub fn resolve_graph_folder_id(&self, folder_id: &str) -> Option<&str> {
        self.by_folder_id.get(folder_id).map(String::as_str)
    }

    /// Derive folder and label IDs for a message from its folder and categories.
    ///
    /// Returns `Err` if any category fails the `CategoryName` validator (empty
    /// or control characters). Dropping a category silently here would land a
    /// message with an incomplete label set, so the parser surfaces the error
    /// to the sync caller instead.
    pub fn get_folder_and_label_ids_for_message(
        &self,
        parent_folder_id: &str,
        categories: &[String],
    ) -> Result<Vec<String>, String> {
        let mut ids = self
            .by_graph_id
            .get(parent_folder_id)
            .map(|mapping| vec![mapping.folder_id.clone()])
            .unwrap_or_default();
        for cat in categories {
            ids.push(LabelKind::graph_category(cat)?.storage_id());
        }
        Ok(ids)
    }

    /// Return all mappings (for list_folders trait method).
    pub fn all_mappings(&self) -> impl Iterator<Item = &FolderMapping> {
        self.by_graph_id.values()
    }

    /// Iterate over all (opaque_folder_id, mapping) pairs.
    /// Used by sync to enumerate folders for message fetching and delta tokens.
    pub fn folder_entries(&self) -> impl Iterator<Item = (&str, &FolderMapping)> + '_ {
        self.by_graph_id.iter().map(|(fid, m)| (fid.as_str(), m))
    }

    /// The well-known alias list (used by the resolution step in sync).
    pub fn well_known_aliases() -> Vec<(&'static str, &'static str, &'static str)> {
        graph_well_known_aliases()
    }
}
