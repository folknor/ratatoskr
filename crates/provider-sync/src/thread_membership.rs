use std::collections::HashSet;

pub(crate) fn filtered_membership_ids<'a>(
    items: impl IntoIterator<Item = &'a str>,
) -> HashSet<&'a str> {
    items
        .into_iter()
        .filter(|label_id| !common::folder_roles::is_message_state_label_id(label_id))
        .filter(|label_id| !is_reserved_imap_keyword_membership_id(label_id))
        .collect()
}

fn is_reserved_imap_keyword_membership_id(label_id: &str) -> bool {
    if common::folder_roles::is_reserved_imap_system_keyword(label_id) {
        return true;
    }

    for reserved_id in [
        "kw:$Forwarded",
        "kw:$MDNSent",
        "kw:$Junk",
        "kw:$NotJunk",
        "kw:$Phishing",
    ] {
        if label_id.eq_ignore_ascii_case(reserved_id) {
            return true;
        }
    }

    false
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::filtered_membership_ids;

    #[test]
    fn filters_message_state_and_reserved_keywords() {
        let input = filtered_membership_ids(["kw:todo", "cat:Project", "STARRED", "kw:$Junk"]);

        assert!(input.contains("kw:todo"));
        assert!(input.contains("cat:Project"));
        assert!(!input.contains("STARRED"));
        assert!(!input.contains("kw:$Junk"));
    }
}
