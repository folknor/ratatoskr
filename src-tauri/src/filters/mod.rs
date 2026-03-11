pub mod commands;

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct FilterCriteria {
    pub from: Option<String>,
    pub to: Option<String>,
    pub subject: Option<String>,
    pub body: Option<String>,
    #[serde(rename = "hasAttachment")]
    pub has_attachment: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FilterActions {
    #[serde(rename = "applyLabel")]
    pub apply_label: Option<String>,
    pub archive: Option<bool>,
    pub star: Option<bool>,
    #[serde(rename = "markRead")]
    pub mark_read: Option<bool>,
    pub trash: Option<bool>,
}

/// Simplified message representation for filter matching.
#[derive(Debug, Clone, Deserialize)]
pub struct FilterableMessage {
    pub thread_id: String,
    pub from_name: Option<String>,
    pub from_address: Option<String>,
    pub to_addresses: Option<String>,
    pub subject: Option<String>,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub has_attachments: bool,
}

/// The aggregate result of filter matching for a single thread.
#[derive(Debug, Clone, Serialize)]
pub struct FilterResult {
    pub add_label_ids: Vec<String>,
    pub remove_label_ids: Vec<String>,
    pub mark_read: bool,
    pub star: bool,
}

// ---------------------------------------------------------------------------
// Core matching logic
// ---------------------------------------------------------------------------

/// Check if a message matches the given filter criteria (AND logic, case-insensitive substring).
pub fn message_matches_filter(message: &FilterableMessage, criteria: &FilterCriteria) -> bool {
    if !message_matches_filter_without_body(message, criteria) {
        return false;
    }

    if let Some(ref body) = criteria.body {
        let body_str = format!(
            "{} {}",
            message.body_text.as_deref().unwrap_or(""),
            message.body_html.as_deref().unwrap_or("")
        )
        .to_lowercase();
        if !body_str.contains(&body.to_lowercase()) {
            return false;
        }
    }

    true
}

/// Check whether a message matches the non-body parts of a filter.
pub fn message_matches_filter_without_body(
    message: &FilterableMessage,
    criteria: &FilterCriteria,
) -> bool {
    if let Some(ref from) = criteria.from {
        let from_str = format!(
            "{} {}",
            message.from_name.as_deref().unwrap_or(""),
            message.from_address.as_deref().unwrap_or("")
        )
        .to_lowercase();
        if !from_str.contains(&from.to_lowercase()) {
            return false;
        }
    }

    if let Some(ref to) = criteria.to {
        let to_str = message.to_addresses.as_deref().unwrap_or("").to_lowercase();
        if !to_str.contains(&to.to_lowercase()) {
            return false;
        }
    }

    if let Some(ref subject) = criteria.subject {
        let subject_str = message.subject.as_deref().unwrap_or("").to_lowercase();
        if !subject_str.contains(&subject.to_lowercase()) {
            return false;
        }
    }

    if criteria.has_attachment == Some(true) && !message.has_attachments {
        return false;
    }

    true
}

/// Compute the aggregate label/flag changes from a set of filter actions.
pub fn compute_filter_actions(actions: &FilterActions) -> FilterResult {
    let mut add_label_ids = Vec::new();
    let mut remove_label_ids = Vec::new();

    if let Some(ref label) = actions.apply_label {
        add_label_ids.push(label.clone());
    }

    if actions.archive == Some(true) {
        remove_label_ids.push("INBOX".to_string());
    }

    if actions.trash == Some(true) {
        add_label_ids.push("TRASH".to_string());
        remove_label_ids.push("INBOX".to_string());
    }

    if actions.star == Some(true) {
        add_label_ids.push("STARRED".to_string());
    }

    FilterResult {
        add_label_ids,
        remove_label_ids,
        mark_read: actions.mark_read == Some(true),
        star: actions.star == Some(true),
    }
}

/// Evaluate all filters against messages and return per-thread actions.
/// Pure computation — does not touch DB or providers.
pub fn evaluate_filters(
    filters: &[(FilterCriteria, FilterActions)],
    messages: &[FilterableMessage],
) -> HashMap<String, FilterResult> {
    let mut thread_actions: HashMap<String, FilterResult> = HashMap::new();

    for msg in messages {
        for (criteria, actions) in filters {
            if message_matches_filter(msg, criteria) {
                let result = compute_filter_actions(actions);
                thread_actions
                    .entry(msg.thread_id.clone())
                    .and_modify(|existing| {
                        existing.add_label_ids.extend(result.add_label_ids.clone());
                        existing
                            .remove_label_ids
                            .extend(result.remove_label_ids.clone());
                        existing.mark_read = existing.mark_read || result.mark_read;
                        existing.star = existing.star || result.star;
                    })
                    .or_insert(result);
            }
        }
    }

    // Deduplicate label IDs
    for result in thread_actions.values_mut() {
        let add: HashSet<String> = result.add_label_ids.drain(..).collect();
        result.add_label_ids = add.into_iter().collect();
        let remove: HashSet<String> = result.remove_label_ids.drain(..).collect();
        result.remove_label_ids = remove.into_iter().collect();
    }

    thread_actions
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_msg() -> FilterableMessage {
        FilterableMessage {
            thread_id: "t1".to_string(),
            from_name: Some("Alice Smith".to_string()),
            from_address: Some("alice@example.com".to_string()),
            to_addresses: Some("bob@example.com".to_string()),
            subject: Some("Project update".to_string()),
            body_text: Some("Hello from the team".to_string()),
            body_html: None,
            has_attachments: false,
        }
    }

    #[test]
    fn matches_from_criteria() {
        let msg = make_msg();
        let criteria = FilterCriteria {
            from: Some("alice".to_string()),
            to: None,
            subject: None,
            body: None,
            has_attachment: None,
        };
        assert!(message_matches_filter(&msg, &criteria));
    }

    #[test]
    fn matches_from_name() {
        let msg = make_msg();
        let criteria = FilterCriteria {
            from: Some("Smith".to_string()),
            to: None,
            subject: None,
            body: None,
            has_attachment: None,
        };
        assert!(message_matches_filter(&msg, &criteria));
    }

    #[test]
    fn no_match_wrong_from() {
        let msg = make_msg();
        let criteria = FilterCriteria {
            from: Some("charlie".to_string()),
            to: None,
            subject: None,
            body: None,
            has_attachment: None,
        };
        assert!(!message_matches_filter(&msg, &criteria));
    }

    #[test]
    fn matches_to() {
        let msg = make_msg();
        let criteria = FilterCriteria {
            from: None,
            to: Some("bob".to_string()),
            subject: None,
            body: None,
            has_attachment: None,
        };
        assert!(message_matches_filter(&msg, &criteria));
    }

    #[test]
    fn matches_subject() {
        let msg = make_msg();
        let criteria = FilterCriteria {
            from: None,
            to: None,
            subject: Some("project".to_string()),
            body: None,
            has_attachment: None,
        };
        assert!(message_matches_filter(&msg, &criteria));
    }

    #[test]
    fn matches_body() {
        let msg = make_msg();
        let criteria = FilterCriteria {
            from: None,
            to: None,
            subject: None,
            body: Some("hello from".to_string()),
            has_attachment: None,
        };
        assert!(message_matches_filter(&msg, &criteria));
    }

    #[test]
    fn matches_non_body_parts_without_hydration() {
        let msg = make_msg();
        let criteria = FilterCriteria {
            from: Some("alice".to_string()),
            to: None,
            subject: Some("project".to_string()),
            body: Some("missing".to_string()),
            has_attachment: None,
        };
        assert!(message_matches_filter_without_body(&msg, &criteria));
        assert!(!message_matches_filter(&msg, &criteria));
    }

    #[test]
    fn matches_attachment() {
        let mut msg = make_msg();
        msg.has_attachments = true;
        let criteria = FilterCriteria {
            from: None,
            to: None,
            subject: None,
            body: None,
            has_attachment: Some(true),
        };
        assert!(message_matches_filter(&msg, &criteria));
    }

    #[test]
    fn no_match_attachment_when_none() {
        let msg = make_msg();
        let criteria = FilterCriteria {
            from: None,
            to: None,
            subject: None,
            body: None,
            has_attachment: Some(true),
        };
        assert!(!message_matches_filter(&msg, &criteria));
    }

    #[test]
    fn and_logic_all_match() {
        let msg = make_msg();
        let criteria = FilterCriteria {
            from: Some("alice".to_string()),
            to: None,
            subject: Some("project".to_string()),
            body: None,
            has_attachment: None,
        };
        assert!(message_matches_filter(&msg, &criteria));
    }

    #[test]
    fn and_logic_one_fails() {
        let msg = make_msg();
        let criteria = FilterCriteria {
            from: Some("alice".to_string()),
            to: None,
            subject: Some("invoice".to_string()),
            body: None,
            has_attachment: None,
        };
        assert!(!message_matches_filter(&msg, &criteria));
    }

    #[test]
    fn empty_criteria_matches_everything() {
        let msg = make_msg();
        let criteria = FilterCriteria {
            from: None,
            to: None,
            subject: None,
            body: None,
            has_attachment: None,
        };
        assert!(message_matches_filter(&msg, &criteria));
    }

    #[test]
    fn null_from_fields() {
        let mut msg = make_msg();
        msg.from_name = None;
        msg.from_address = None;
        let criteria = FilterCriteria {
            from: Some("alice".to_string()),
            to: None,
            subject: None,
            body: None,
            has_attachment: None,
        };
        assert!(!message_matches_filter(&msg, &criteria));
    }

    // -- compute_filter_actions --

    #[test]
    fn compute_empty_actions() {
        let actions = FilterActions {
            apply_label: None,
            archive: None,
            star: None,
            mark_read: None,
            trash: None,
        };
        let result = compute_filter_actions(&actions);
        assert!(result.add_label_ids.is_empty());
        assert!(result.remove_label_ids.is_empty());
        assert!(!result.mark_read);
        assert!(!result.star);
    }

    #[test]
    fn compute_archive() {
        let actions = FilterActions {
            apply_label: None,
            archive: Some(true),
            star: None,
            mark_read: None,
            trash: None,
        };
        let result = compute_filter_actions(&actions);
        assert!(result.remove_label_ids.contains(&"INBOX".to_string()));
    }

    #[test]
    fn compute_trash() {
        let actions = FilterActions {
            apply_label: None,
            archive: None,
            star: None,
            mark_read: None,
            trash: Some(true),
        };
        let result = compute_filter_actions(&actions);
        assert!(result.add_label_ids.contains(&"TRASH".to_string()));
        assert!(result.remove_label_ids.contains(&"INBOX".to_string()));
    }

    #[test]
    fn compute_combined() {
        let actions = FilterActions {
            apply_label: Some("Label_1".to_string()),
            archive: Some(true),
            star: Some(true),
            mark_read: Some(true),
            trash: None,
        };
        let result = compute_filter_actions(&actions);
        assert!(result.add_label_ids.contains(&"Label_1".to_string()));
        assert!(result.add_label_ids.contains(&"STARRED".to_string()));
        assert!(result.remove_label_ids.contains(&"INBOX".to_string()));
        assert!(result.mark_read);
        assert!(result.star);
    }

    // -- evaluate_filters --

    #[test]
    fn evaluate_merges_multiple_filters() {
        let filters = vec![
            (
                FilterCriteria {
                    from: Some("alice".to_string()),
                    to: None,
                    subject: None,
                    body: None,
                    has_attachment: None,
                },
                FilterActions {
                    apply_label: Some("Important".to_string()),
                    archive: None,
                    star: None,
                    mark_read: None,
                    trash: None,
                },
            ),
            (
                FilterCriteria {
                    from: None,
                    to: None,
                    subject: Some("project".to_string()),
                    body: None,
                    has_attachment: None,
                },
                FilterActions {
                    apply_label: None,
                    archive: None,
                    star: Some(true),
                    mark_read: None,
                    trash: None,
                },
            ),
        ];

        let messages = vec![make_msg()];
        let results = evaluate_filters(&filters, &messages);

        let result = results.get("t1").expect("should have result for t1");
        assert!(result.add_label_ids.contains(&"Important".to_string()));
        assert!(result.add_label_ids.contains(&"STARRED".to_string()));
        assert!(result.star);
    }
}
