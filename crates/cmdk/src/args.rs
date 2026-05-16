use types::{FolderId, LabelGroupId};

// `NavigateToFolder { folder_id, account_id }` previously existed here as
// the dispatch arg for cross-account folder navigation. Removed in the
// labels-unification slice 5 cleanup: nothing in the palette pipeline
// produces it post-split, and the `NavigateToLabel` dispatch arm that
// consumed it became unreachable. Restore both ends together if a
// future palette entry surfaces cross-account folder targets.

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
    /// Navigate to a label group.
    NavigateToLabel { group_id: LabelGroupId },
    /// SmartFolderSave -- name from Text input
    SmartFolderSave { name: String },
}
