/// Typed execution payload for parameterized commands.
///
/// Each variant carries exactly the fields that command needs.
/// The app layer matches on the variant and dispatches to the
/// appropriate handler in `update()`.
#[derive(Debug, Clone)]
pub enum CommandArgs {
    /// EmailMoveToFolder -- folder_id from ListPicker selection
    MoveToFolder { folder_id: String },
    /// EmailAddLabel -- label_id from ListPicker selection
    AddLabel { label_id: String },
    /// EmailRemoveLabel -- label_id from ListPicker selection
    RemoveLabel { label_id: String },
    /// EmailSnooze -- unix timestamp from DateTime picker
    Snooze { until: i64 },
    /// NavigateToLabel -- label_id from ListPicker selection.
    /// Includes account_id because cross-account navigation needs
    /// to know which account the label belongs to.
    NavigateToLabel { label_id: String, account_id: String },
}
