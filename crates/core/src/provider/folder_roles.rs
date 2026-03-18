pub struct SystemFolderRole {
    pub label_id: &'static str,
    pub label_name: &'static str,
    pub jmap_role: Option<&'static str>,
    pub graph_alias: Option<&'static str>,
    pub imap_special_use: Option<&'static str>,
    pub imap_name_aliases: &'static [&'static str],
}

pub const SYSTEM_FOLDER_ROLES: &[SystemFolderRole] = &[
    SystemFolderRole {
        label_id: "INBOX",
        label_name: "Inbox",
        jmap_role: Some("inbox"),
        graph_alias: Some("inbox"),
        imap_special_use: Some("\\Inbox"),
        imap_name_aliases: &["inbox"],
    },
    SystemFolderRole {
        label_id: "DRAFT",
        label_name: "Drafts",
        jmap_role: Some("drafts"),
        graph_alias: Some("drafts"),
        imap_special_use: Some("\\Drafts"),
        imap_name_aliases: &[
            "drafts",
            "draft",
            "draftbox",
            "brouillons",
            "[gmail]/drafts",
            "entwuerfe",
            "entw\u{00FC}rfe",
            "borradores",
            "bozze",
            "rascunhos",
        ],
    },
    SystemFolderRole {
        label_id: "SENT",
        label_name: "Sent",
        jmap_role: Some("sent"),
        graph_alias: Some("sentitems"),
        imap_special_use: Some("\\Sent"),
        imap_name_aliases: &[
            "sent",
            "sent items",
            "sent mail",
            "[gmail]/sent mail",
            "gesendet",
            "enviados",
            "posta inviata",
        ],
    },
    SystemFolderRole {
        label_id: "TRASH",
        label_name: "Trash",
        jmap_role: Some("trash"),
        graph_alias: Some("deleteditems"),
        imap_special_use: Some("\\Trash"),
        imap_name_aliases: &[
            "trash",
            "deleted",
            "deleted items",
            "deleted messages",
            "bin",
            "corbeille",
            "unsolbox",
            "[gmail]/trash",
            "papierkorb",
            "papelera",
            "cestino",
            "lixeira",
        ],
    },
    SystemFolderRole {
        label_id: "SPAM",
        label_name: "Spam",
        jmap_role: Some("junk"),
        graph_alias: Some("junkemail"),
        imap_special_use: Some("\\Junk"),
        imap_name_aliases: &["junk", "junk e-mail", "spam", "[gmail]/spam", "bulk mail"],
    },
    SystemFolderRole {
        label_id: "archive",
        label_name: "Archive",
        jmap_role: Some("archive"),
        graph_alias: Some("archive"),
        imap_special_use: Some("\\Archive"),
        imap_name_aliases: &["archive", "archives"],
    },
    SystemFolderRole {
        label_id: "STARRED",
        label_name: "Starred",
        jmap_role: None,
        graph_alias: None,
        imap_special_use: Some("\\Flagged"),
        imap_name_aliases: &["flagged", "starred", "[gmail]/starred"],
    },
    SystemFolderRole {
        label_id: "all-mail",
        label_name: "All Mail",
        jmap_role: None,
        graph_alias: None,
        imap_special_use: Some("\\All"),
        imap_name_aliases: &["all mail", "[gmail]/all mail"],
    },
    SystemFolderRole {
        label_id: "IMPORTANT",
        label_name: "Important",
        jmap_role: Some("important"),
        graph_alias: None,
        imap_special_use: Some("\\Important"),
        imap_name_aliases: &["important", "[gmail]/important"],
    },
];

pub fn system_folder_by_jmap_role(role: &str) -> Option<&'static SystemFolderRole> {
    SYSTEM_FOLDER_ROLES
        .iter()
        .find(|entry| entry.jmap_role == Some(role))
}

pub fn system_folder_by_graph_alias(alias: &str) -> Option<&'static SystemFolderRole> {
    SYSTEM_FOLDER_ROLES
        .iter()
        .find(|entry| entry.graph_alias == Some(alias))
}

pub fn graph_well_known_aliases() -> Vec<(&'static str, &'static str, &'static str)> {
    SYSTEM_FOLDER_ROLES
        .iter()
        .filter_map(|entry| {
            entry
                .graph_alias
                .map(|alias| (alias, entry.label_id, entry.label_name))
        })
        .collect()
}

pub fn system_folder_by_imap_special_use(special_use: &str) -> Option<&'static SystemFolderRole> {
    SYSTEM_FOLDER_ROLES
        .iter()
        .find(|entry| entry.imap_special_use == Some(special_use))
}

pub fn imap_special_use_to_label_id(special_use: &str) -> Option<&'static str> {
    system_folder_by_imap_special_use(special_use).map(|entry| entry.label_id)
}

pub fn imap_name_to_special_use(name: &str) -> Option<&'static str> {
    SYSTEM_FOLDER_ROLES.iter().find_map(|entry| {
        entry
            .imap_name_aliases
            .iter()
            .any(|candidate| *candidate == name)
            .then_some(entry.imap_special_use)
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::{
        graph_well_known_aliases, imap_name_to_special_use, imap_special_use_to_label_id,
        system_folder_by_jmap_role,
    };

    #[test]
    fn maps_jmap_archive_role() {
        let mapping = system_folder_by_jmap_role("archive").expect("archive role should exist");
        assert_eq!(mapping.label_id, "archive");
        assert_eq!(mapping.label_name, "Archive");
    }

    #[test]
    fn maps_imap_special_use() {
        assert_eq!(imap_special_use_to_label_id("\\Sent"), Some("SENT"));
        assert_eq!(imap_special_use_to_label_id("\\Archive"), Some("archive"));
    }

    #[test]
    fn maps_imap_name_aliases() {
        assert_eq!(imap_name_to_special_use("[gmail]/all mail"), Some("\\All"));
        assert_eq!(imap_name_to_special_use("spam"), Some("\\Junk"));
    }

    #[test]
    fn exposes_graph_aliases() {
        let aliases = graph_well_known_aliases();
        assert!(aliases.contains(&("inbox", "INBOX", "Inbox")));
        assert!(aliases.contains(&("archive", "archive", "Archive")));
    }
}
