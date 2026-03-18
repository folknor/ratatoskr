use crate::provider::folder_roles::{SYSTEM_FOLDER_ROLES, system_folder_by_jmap_role};
use crate::provider::label_flags::assemble_labels;
use std::collections::HashMap;

pub struct MailboxLabelMapping {
    pub label_id: String,
    pub label_name: String,
    pub label_type: &'static str,
}

/// Map a JMAP mailbox to a Gmail-style label ID.
///
/// System roles map to well-known IDs (INBOX, SENT, TRASH, etc.).
/// User mailboxes get a `jmap-{id}` prefix.
pub fn map_mailbox_to_label(
    role: Option<&str>,
    mailbox_id: &str,
    name: &str,
) -> MailboxLabelMapping {
    if let Some(r) = role
        && let Some(mapping) = system_folder_by_jmap_role(r)
    {
        return MailboxLabelMapping {
            label_id: mapping.label_id.to_string(),
            label_name: mapping.label_name.to_string(),
            label_type: "system",
        };
    }
    MailboxLabelMapping {
        label_id: format!("jmap-{mailbox_id}"),
        label_name: name.to_string(),
        label_type: "user",
    }
}

/// Cached mailbox info for label resolution.
pub struct MailboxInfo {
    pub role: Option<String>,
    pub name: String,
}

/// Derive Gmail-style label IDs from an email's mailbox membership and keywords.
pub fn get_labels_for_email(
    mailbox_ids: &[&str],
    keywords: &[&str],
    mailbox_map: &HashMap<String, MailboxInfo>,
) -> Vec<String> {
    let primary_labels = mailbox_ids.iter().filter_map(|mb_id| {
        mailbox_map
            .get(*mb_id)
            .map(|info| map_mailbox_to_label(info.role.as_deref(), mb_id, &info.name).label_id)
    });

    assemble_labels(
        primary_labels,
        std::iter::empty::<String>(),
        keywords.contains(&"$seen"),
        keywords.contains(&"$flagged"),
        keywords.contains(&"$draft"),
    )
}

/// Reverse lookup: Gmail-style label ID → JMAP mailbox ID.
pub fn label_id_to_mailbox_id(
    label_id: &str,
    mailboxes: &[(String, Option<String>, String)], // (id, role, name)
) -> Option<String> {
    // Check system role mappings
    for mapping in SYSTEM_FOLDER_ROLES {
        if mapping.label_id == label_id
            && let Some(role) = mapping.jmap_role
        {
            return mailboxes
                .iter()
                .find(|(_, r, _)| r.as_deref() == Some(role))
                .map(|(id, _, _)| id.clone());
        }
    }

    // User mailbox: strip "jmap-" prefix
    if let Some(raw_id) = label_id.strip_prefix("jmap-") {
        return Some(raw_id.to_string());
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
