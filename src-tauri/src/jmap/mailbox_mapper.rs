use std::collections::HashMap;

/// (jmap_role, label_id, label_name)
const ROLE_MAP: &[(&str, &str, &str)] = &[
    ("inbox", "INBOX", "Inbox"),
    ("archive", "archive", "Archive"),
    ("drafts", "DRAFT", "Drafts"),
    ("sent", "SENT", "Sent"),
    ("trash", "TRASH", "Trash"),
    ("junk", "SPAM", "Spam"),
    ("important", "IMPORTANT", "Important"),
];

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
    if let Some(r) = role {
        if let Some(&(_, label_id, label_name)) = ROLE_MAP.iter().find(|&&(rr, _, _)| rr == r) {
            return MailboxLabelMapping {
                label_id: label_id.to_string(),
                label_name: label_name.to_string(),
                label_type: "system",
            };
        }
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
    let mut labels = Vec::new();

    for &mb_id in mailbox_ids {
        if let Some(info) = mailbox_map.get(mb_id) {
            let mapping = map_mailbox_to_label(info.role.as_deref(), mb_id, &info.name);
            labels.push(mapping.label_id);
        }
    }

    if !keywords.contains(&"$seen") {
        labels.push("UNREAD".to_string());
    }
    if keywords.contains(&"$flagged") {
        labels.push("STARRED".to_string());
    }
    if keywords.contains(&"$draft") && !labels.contains(&"DRAFT".to_string()) {
        labels.push("DRAFT".to_string());
    }

    labels
}

/// Reverse lookup: Gmail-style label ID → JMAP mailbox ID.
pub fn label_id_to_mailbox_id(
    label_id: &str,
    mailboxes: &[(String, Option<String>, String)], // (id, role, name)
) -> Option<String> {
    // Check system role mappings
    for &(_, sys_label_id, _) in ROLE_MAP {
        if sys_label_id == label_id {
            // Find the role name for this label
            let role_name = ROLE_MAP
                .iter()
                .find(|(_, lid, _)| *lid == label_id)
                .map(|(r, _, _)| *r);
            if let Some(role) = role_name {
                return mailboxes
                    .iter()
                    .find(|(_, r, _)| r.as_deref() == Some(role))
                    .map(|(id, _, _)| id.clone());
            }
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
