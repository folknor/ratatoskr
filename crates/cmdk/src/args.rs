use types::{FolderId, LabelGroupId};

/// Typed execution payload for parameterized commands.
///
/// Each variant carries exactly the fields that command needs.
/// The app layer matches on the variant and dispatches to the
/// appropriate handler in `update()`.
#[derive(Debug, Clone)]
pub enum CommandArgs {
    /// EmailMoveToFolder -- folder_id from ListPicker selection
    MoveToFolder { folder_id: FolderId },
    /// EmailAddLabel -- label group id from ListPicker selection
    AddLabel { group_id: LabelGroupId },
    /// EmailRemoveLabel -- label group id from ListPicker selection
    RemoveLabel { group_id: LabelGroupId },
    /// EmailSnooze -- unix timestamp from DateTime picker
    Snooze { until: i64 },
    /// Navigate to a provider folder. Includes account_id because
    /// cross-account navigation needs to scope the sidebar.
    NavigateToFolder {
        folder_id: FolderId,
        account_id: String,
    },
    /// Navigate to a label group.
    NavigateToLabel { group_id: LabelGroupId },
    /// SmartFolderSave -- name from Text input
    SmartFolderSave { name: String },
}
