use common::folder_roles::{SYSTEM_FOLDER_ROLES, system_folder_by_jmap_role};
use common::types::{FolderKind, MailProviderKind};
use std::collections::HashMap;

pub struct MailboxFolderMapping {
    pub folder_id: String,
    pub folder_name: String,
    pub folder_type: &'static str,
}

/// Map a JMAP mailbox to a Ratatoskr folder ID.
///
/// System roles map to well-known IDs (INBOX, SENT, TRASH, etc.).
/// User mailboxes get a `jmap-{id}` prefix.
///
/// Returns `Err` only when a user mailbox's JMAP ID fails the `JmapId`
/// validator (empty or contains control characters).
pub fn map_mailbox_to_folder(
    role: Option<&str>,
    mailbox_id: &str,
    name: &str,
) -> Result<MailboxFolderMapping, String> {
    if let Some(r) = role
        && let Some(mapping) = system_folder_by_jmap_role(r)
    {
        return Ok(MailboxFolderMapping {
            folder_id: mapping.label_id.to_string(),
            folder_name: mapping.label_name.to_string(),
            folder_type: "system",
        });
    }
    Ok(MailboxFolderMapping {
        folder_id: FolderKind::jmap_user(mailbox_id)?.storage_id(),
        folder_name: name.to_string(),
        folder_type: "user",
    })
}

/// Cached mailbox info for folder resolution.
pub struct MailboxInfo {
    pub role: Option<String>,
    pub name: String,
}

/// Resolve an email's mailbox membership to Ratatoskr folder IDs, plus a
/// synthetic DRAFT folder ID when the message carries the `$draft` keyword.
pub fn get_labels_for_email(
    mailbox_ids: &[&str],
    keywords: &[&str],
    mailbox_map: &HashMap<String, MailboxInfo>,
) -> Result<Vec<String>, String> {
    let mut folder_ids: Vec<String> = Vec::new();
    for mb_id in mailbox_ids {
        let Some(info) = mailbox_map.get(*mb_id) else {
            continue;
        };
        let mapping = map_mailbox_to_folder(info.role.as_deref(), mb_id, &info.name)?;
        folder_ids.push(mapping.folder_id);
    }

    if keywords.contains(&"$draft") && !folder_ids.iter().any(|id| id == "DRAFT") {
        folder_ids.push("DRAFT".to_string());
    }

    Ok(folder_ids)
}

/// Reverse lookup: Ratatoskr folder ID -> JMAP mailbox ID.
pub fn folder_id_to_mailbox_id(
    folder_id: &str,
    mailboxes: &[(String, Option<String>, String)], // (id, role, name)
) -> Option<String> {
    // Check system role mappings
    for mapping in SYSTEM_FOLDER_ROLES {
        if mapping.label_id == folder_id
            && let Some(role) = mapping.jmap_role
        {
            return mailboxes
                .iter()
                .find(|(_, r, _)| r.as_deref() == Some(role))
                .map(|(id, _, _)| id.clone());
        }
    }

    if let Ok(FolderKind::JmapUser(id)) = FolderKind::parse(folder_id, MailProviderKind::Jmap) {
        return Some(id.as_jmap_id().to_string());
    }

    None
}

/// Find the JMAP mailbox ID for a given role.
pub fn find_mailbox_id_by_role(
    mailboxes: &[(String, Option<String>, String)],
    role: &str,
) -> Option<String> {
    mailboxes
        .iter()
        .find(|(_, r, _)| r.as_deref() == Some(role))
        .map(|(id, _, _)| id.clone())
}
