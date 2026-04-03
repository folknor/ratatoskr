use crate::{FolderId, TagId};

/// What the sidebar is currently showing / the user has selected.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SidebarSelection {
    /// Inbox — the default view when nothing else is selected.
    Inbox,
    /// A well-known system folder.
    Folder(SystemFolder),
    /// An AI inbox bundle (Primary, Updates, etc.).
    Bundle(Bundle),
    /// A non-folder feature view reachable from the sidebar.
    FeatureView(FeatureView),
    /// A user-defined smart folder backed by a saved search query.
    SmartFolder { id: String },
    /// A provider-specific container folder (Exchange folder, IMAP mailbox, etc.).
    ProviderFolder(FolderId),
    /// A tag-type label (Gmail user label, Exchange category, IMAP keyword, etc.).
    Tag(TagId),
}

/// System folders that exist on every email account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SystemFolder {
    Starred,
    Sent,
    Draft,
    Snoozed,
    Trash,
    Spam,
    AllMail,
}

/// AI inbox bundle classifications (not folders in the glossary sense).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Bundle {
    Primary,
    Updates,
    Promotions,
    Social,
    Newsletters,
}

/// Non-folder feature views reachable from the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum FeatureView {
    Tasks,
    Attachments,
}

impl SystemFolder {
    /// The canonical folder ID string used in the database.
    pub fn as_folder_id_str(&self) -> &'static str {
        match self {
            Self::Starred => "STARRED",
            Self::Sent => "SENT",
            Self::Draft => "DRAFT",
            Self::Snoozed => "SNOOZED",
            Self::Trash => "TRASH",
            Self::Spam => "SPAM",
            Self::AllMail => "all-mail",
        }
    }
}

impl Bundle {
    /// The canonical bundle ID string used in the database.
    pub fn as_id_str(&self) -> &'static str {
        match self {
            Self::Primary => "BUNDLE_PRIMARY",
            Self::Updates => "BUNDLE_UPDATES",
            Self::Promotions => "BUNDLE_PROMOTIONS",
            Self::Social => "BUNDLE_SOCIAL",
            Self::Newsletters => "BUNDLE_NEWSLETTERS",
        }
    }
}

impl FeatureView {
    /// The canonical view ID string.
    pub fn as_id_str(&self) -> &'static str {
        match self {
            Self::Tasks => "TASKS",
            Self::Attachments => "ATTACHMENTS",
        }
    }
}

impl SidebarSelection {
    /// The folder/label ID for generic thread-loading DB queries.
    ///
    /// Returns `None` for Inbox (no filter convention), bundles, and feature
    /// views, which have dedicated loaders.
    /// Returns `Some(id)` for normal folder/tag selections.
    pub fn folder_id_for_thread_query(&self) -> Option<String> {
        match self {
            Self::Inbox | Self::Bundle(_) | Self::FeatureView(_) => None,
            Self::Folder(f) => Some(f.as_folder_id_str().to_string()),
            Self::SmartFolder { id } => Some(id.clone()),
            Self::ProviderFolder(fid) => Some(fid.0.clone()),
            Self::Tag(tid) => Some(tid.0.clone()),
        }
    }

    /// The navigation-folder identity for rights / nav_state lookup.
    ///
    /// Returns the raw ID string including `"INBOX"` for Inbox.
    /// Returns `None` for SmartFolder, Bundle, FeatureView (no ACL data).
    pub fn navigation_folder_id(&self) -> Option<String> {
        match self {
            Self::Inbox => Some("INBOX".to_string()),
            Self::Folder(f) => Some(f.as_folder_id_str().to_string()),
            Self::ProviderFolder(fid) => Some(fid.0.clone()),
            Self::Tag(tid) => Some(tid.0.clone()),
            Self::SmartFolder { .. } | Self::Bundle(_) | Self::FeatureView(_) => None,
        }
    }

    /// Source folder for trash/move undo. Only meaningful for folder-type
    /// selections — returns `None` for smart folders, tags, bundles, feature views.
    pub fn source_folder_for_undo(&self) -> Option<FolderId> {
        match self {
            Self::Inbox => Some(FolderId::from("INBOX")),
            Self::Folder(f) => Some(FolderId::from(f.as_folder_id_str())),
            Self::ProviderFolder(fid) => Some(fid.clone()),
            Self::SmartFolder { .. } | Self::Bundle(_) | Self::FeatureView(_) | Self::Tag(_) => {
                None
            }
        }
    }

    pub fn is_trash(&self) -> bool {
        matches!(self, Self::Folder(SystemFolder::Trash))
    }

    pub fn is_spam(&self) -> bool {
        matches!(self, Self::Folder(SystemFolder::Spam))
    }

    pub fn is_draft(&self) -> bool {
        matches!(self, Self::Folder(SystemFolder::Draft))
    }
}
