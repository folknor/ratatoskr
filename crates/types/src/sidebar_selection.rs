use crate::{FolderId, LabelGroupId};

/// What the sidebar is currently showing / the user has selected.
///
/// Variants are split by FK-safety: `Inbox`, `Folder(SystemFolder)`, and
/// `ProviderFolder(FolderId)` correspond to real `folders.id` rows and may be
/// passed to thread queries that join `thread_folders`. `VirtualView`,
/// `LabelGroup`, `SmartFolder`, `Bundle`, and `FeatureView` are NOT folders -
/// they have no row in `folders` and must be routed by callers to dedicated
/// loaders. Producing `WHERE thread_folders.folder_id = 'STARRED'` is a bug;
/// the type system now makes it unspellable.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SidebarSelection {
    /// Inbox - the default view when nothing else is selected.
    Inbox,
    /// A well-known system folder (real `folders.id` row).
    Folder(SystemFolder),
    /// A virtual navigation view backed by a thread-state boolean or no
    /// filter at all (Starred, Snoozed, All Mail). No `folders` row exists
    /// for these.
    VirtualView(VirtualView),
    /// An AI inbox bundle (Primary, Updates, etc.).
    Bundle(Bundle),
    /// A non-folder feature view reachable from the sidebar.
    FeatureView(FeatureView),
    /// A user-defined smart folder backed by a saved search query.
    SmartFolder { id: String },
    /// A provider-specific container folder (Exchange folder, IMAP mailbox, etc.).
    ProviderFolder(FolderId),
    /// A user-created label group.
    LabelGroup(LabelGroupId),
}

/// Real system-folder rows present on every email account. Each variant
/// corresponds to a `folders.id` row using a canonical Ratatoskr ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SystemFolder {
    Sent,
    Draft,
    Trash,
    Spam,
}

/// Virtual sidebar destinations that are not folders. Each is backed either
/// by a boolean column on `threads` (Starred, Snoozed) or no filter at all
/// (AllMail). They never reach `thread_folders` / `folders` queries; callers
/// must dispatch on the variant and route to the dedicated helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum VirtualView {
    Starred,
    Snoozed,
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
    /// The canonical folder ID string used in the database. Every variant
    /// here maps to a real `folders.id` row.
    pub fn as_folder_id_str(&self) -> &'static str {
        match self {
            Self::Sent => "SENT",
            Self::Draft => "DRAFT",
            Self::Trash => "TRASH",
            Self::Spam => "SPAM",
        }
    }
}

impl VirtualView {
    /// The legacy string identifier for this virtual view. NOT a folder ID -
    /// must not be used as an FK in `thread_folders`. Kept for URL/wire
    /// serialisation only (sidebar persistence, command palette IDs).
    pub fn as_id_str(&self) -> &'static str {
        match self {
            Self::Starred => "STARRED",
            Self::Snoozed => "SNOOZED",
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
    /// The folder ID for generic thread-loading DB queries that join
    /// `thread_folders`. Returns `Some(id)` only for variants backed by a
    /// real `folders.id` row (Inbox, real system folders, provider folders).
    /// Returns `None` for virtuals, label groups, bundles, feature views,
    /// and smart folders - those have dedicated loaders and must not be
    /// passed to folder-id-shaped queries.
    pub fn folder_id_for_thread_query(&self) -> Option<String> {
        match self {
            Self::Inbox => Some("INBOX".to_string()),
            Self::Folder(f) => Some(f.as_folder_id_str().to_string()),
            Self::ProviderFolder(fid) => Some(fid.0.clone()),
            Self::VirtualView(_)
            | Self::Bundle(_)
            | Self::FeatureView(_)
            | Self::SmartFolder { .. }
            | Self::LabelGroup(_) => None,
        }
    }

    /// The navigation-folder identity for rights / nav_state lookup.
    ///
    /// Returns the raw ID string including `"INBOX"` for Inbox.
    /// Returns `None` for SmartFolder, VirtualView, Bundle, FeatureView,
    /// LabelGroup (no ACL data).
    pub fn navigation_folder_id(&self) -> Option<String> {
        match self {
            Self::Inbox => Some("INBOX".to_string()),
            Self::Folder(f) => Some(f.as_folder_id_str().to_string()),
            Self::ProviderFolder(fid) => Some(fid.0.clone()),
            Self::VirtualView(_)
            | Self::SmartFolder { .. }
            | Self::Bundle(_)
            | Self::FeatureView(_)
            | Self::LabelGroup(_) => None,
        }
    }

    /// Source folder for trash/move undo. Only meaningful for selections
    /// that correspond to a real `folders.id` row.
    pub fn source_folder_for_undo(&self) -> Option<FolderId> {
        match self {
            Self::Inbox => Some(FolderId::from("INBOX")),
            Self::Folder(f) => Some(FolderId::from(f.as_folder_id_str())),
            Self::ProviderFolder(fid) => Some(fid.clone()),
            Self::VirtualView(_)
            | Self::SmartFolder { .. }
            | Self::Bundle(_)
            | Self::FeatureView(_)
            | Self::LabelGroup(_) => None,
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
