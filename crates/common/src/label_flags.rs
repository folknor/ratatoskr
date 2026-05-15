pub fn assemble_labels<I, J>(
    primary_labels: I,
    supplemental_labels: J,
    _is_read: bool,
    _is_starred: bool,
    is_draft: bool,
) -> Vec<String>
where
    I: IntoIterator<Item = String>,
    J: IntoIterator<Item = String>,
{
    let mut labels: Vec<String> = primary_labels.into_iter().collect();
    labels.extend(supplemental_labels);

    if is_draft && !labels.iter().any(|label| label == "DRAFT") {
        labels.push("DRAFT".to_string());
    }

    labels
}

pub fn prefixed_labels<'a, I>(prefix: &str, values: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a str>,
{
    values
        .into_iter()
        .map(|value| format!("{prefix}{value}"))
        .collect()
}

pub fn is_message_state_label_id(label_id: &str) -> bool {
    matches!(label_id, "UNREAD" | "STARRED")
}

pub fn is_reserved_imap_system_keyword(keyword: &str) -> bool {
    matches!(
        keyword.to_ascii_lowercase().as_str(),
        "$forwarded" | "$mdnsent" | "$junk" | "$notjunk" | "$phishing"
    )
}

pub fn is_user_visible_keyword(keyword: &str) -> bool {
    !keyword.starts_with('$') && !is_reserved_imap_system_keyword(keyword)
}

#[cfg(test)]
mod tests {
    use super::{assemble_labels, prefixed_labels};

    #[test]
    fn assembles_base_labels_without_message_state() {
        let labels = assemble_labels(
            vec!["INBOX".to_string()],
            vec!["cat:Work".to_string()],
            false,
            true,
            false,
        );
        assert_eq!(
            labels,
            vec!["INBOX".to_string(), "cat:Work".to_string()]
        );
    }

    #[test]
    fn avoids_duplicate_draft_label() {
        let labels = assemble_labels(
            vec!["DRAFT".to_string()],
            Vec::<String>::new(),
            true,
            false,
            true,
        );
        assert_eq!(labels, vec!["DRAFT".to_string()]);
    }

    #[test]
    fn prefixes_labels() {
        let labels = prefixed_labels("cat:", ["Work", "Urgent"]);
        assert_eq!(
            labels,
            vec!["cat:Work".to_string(), "cat:Urgent".to_string()]
        );
    }
}
