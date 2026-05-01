use iced::Point;
use rte::Action as RteAction;

use crate::db::ContactMatch;
use crate::ui::token_input::{TokenId, TokenInputMessage};

use super::types::{AccountInfo, ComposeAttachment, GroupSaveSuccess, RecipientField};

/// How the compose window was opened.
#[derive(Debug, Clone)]
pub enum ComposeMode {
    New,
    Reply { original_subject: String },
    ReplyAll { original_subject: String },
    Forward { original_subject: String },
}

impl ComposeMode {
    /// Returns the subject line with the appropriate prefix applied.
    pub fn prefixed_subject(&self) -> String {
        match self {
            Self::New => String::new(),
            Self::Reply { original_subject } | Self::ReplyAll { original_subject } => {
                if original_subject.starts_with("Re: ") {
                    original_subject.clone()
                } else {
                    format!("Re: {original_subject}")
                }
            }
            Self::Forward { original_subject } => {
                if original_subject.starts_with("Fwd: ") {
                    original_subject.clone()
                } else {
                    format!("Fwd: {original_subject}")
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum ComposeMessage {
    /// Event sink used by the modal backdrop to block background clicks
    /// without dismissing the active modal.
    Noop,
    SubjectChanged(String),
    BodyChanged(RteAction),
    FromAccountChanged(AccountInfo),
    ToggleFromDropdown,
    ShowCc,
    ShowBcc,
    ToTokenInput(TokenInputMessage),
    CcTokenInput(TokenInputMessage),
    BccTokenInput(TokenInputMessage),
    Send,
    /// Manual save request from the Save button - flushes the current draft
    /// state immediately rather than waiting for the next auto-save tick.
    SaveDraftNow,
    Discard,
    /// Toggle discard confirmation dialog.
    ToggleDiscardConfirm,
    /// Autocomplete results arrived from the database.
    AutocompleteResults(
        rtsk::generation::GenerationToken<rtsk::generation::Autocomplete>,
        Result<Vec<ContactMatch>, String>,
    ),
    /// User selected an autocomplete suggestion.
    AutocompleteSelect(usize),
    /// User navigated the autocomplete list (up/down).
    AutocompleteNavigate(i32),
    /// Dismiss the autocomplete dropdown.
    AutocompleteDismiss,
    /// Formatting toolbar actions - emit ToggleInlineStyle to the rich text editor.
    FormatBold,
    FormatItalic,
    FormatUnderline,
    FormatStrikethrough,
    /// List toggle via SetBlockType.
    FormatList,
    /// Open the link insertion dialog.
    FormatLink,
    // ── Attachments ──
    /// User clicked the attach button - opens file picker.
    AttachFiles,
    /// File picker returned selected files (read asynchronously).
    FilesSelected(Vec<ComposeAttachment>),
    /// Remove an attachment by index.
    RemoveAttachment(usize),
    // ── Context menu ──
    /// Show the token context menu at a given position.
    ShowTokenContextMenu {
        field: RecipientField,
        token_id: TokenId,
        position: Point,
    },
    /// Show the field-area context menu at a given position. Renders only
    /// the Paste action since Cut/Copy/Delete need a token target.
    ShowFieldContextMenu {
        field: RecipientField,
        position: Point,
    },
    /// Dismiss the token context menu.
    DismissContextMenu,
    /// Delete a token from a specific field via context menu.
    ContextMenuDelete {
        field: RecipientField,
        token_id: TokenId,
    },
    /// Move a token to a different recipient field.
    ContextMenuMoveTo {
        token_id: TokenId,
        from: RecipientField,
        to_field: RecipientField,
    },
    /// Copy a token's `Name <email>` to the clipboard.
    ContextMenuCopy {
        field: RecipientField,
        token_id: TokenId,
    },
    /// Copy a token's `Name <email>` to the clipboard, then delete it.
    ContextMenuCut {
        field: RecipientField,
        token_id: TokenId,
    },
    /// Read the clipboard and paste into `field` via the standard token-input
    /// paste path (so the bulk-paste banner still triggers at 10+).
    ContextMenuPaste {
        field: RecipientField,
    },
    /// Expand a group token into individual member tokens.
    /// The expansion result arrives via `GroupExpanded`.
    ContextMenuExpandGroup {
        field: RecipientField,
        token_id: TokenId,
    },
    /// Group expansion results arrived from the database.
    GroupExpanded {
        field: RecipientField,
        token_id: TokenId,
        members: Result<Vec<(String, Option<String>)>, String>,
    },
    // ── Drag and drop ──
    /// A token drag was initiated (passed threshold).
    DragStarted {
        field: RecipientField,
        token_id: TokenId,
    },
    /// Mouse moved while dragging.
    DragMove(Point),
    /// Mouse released - drop the token.
    DragEnd(Point),
    /// Cancel the drag.
    DragCancel,
    // ── Banners ──
    /// Bcc nudge: move group to Bcc.
    BccNudgeAccept(TokenId),
    /// Bcc nudge: dismiss.
    BccNudgeDismiss(TokenId),
    /// Bulk paste: dismiss.
    BulkPasteDismiss,
    /// Bulk paste: open the save-as-group dialog.
    BulkPasteSaveAsGroup,
    /// Save-as-group dialog: name input changed.
    GroupSaveNameChanged(String),
    /// Save-as-group dialog: confirm. Triggers the async DB write at the
    /// app layer; the result comes back as `GroupSaveResult`.
    GroupSaveConfirm,
    /// Save-as-group dialog: cancel.
    GroupSaveCancel,
    /// Save-as-group async result. On success carries the new `group_id`
    /// and `name` so the pasted tokens can be replaced with one group token.
    GroupSaveResult(Result<GroupSaveSuccess, String>),
    /// Result of looking up whether a freshly-pasted set of addresses
    /// matches an existing contact group exactly. If `group` is `Some`,
    /// the tokens listed in `added_ids` are replaced with one group token.
    PasteGroupMatchResult {
        field: RecipientField,
        added_ids: Vec<TokenId>,
        group: Option<rtsk::db::queries_extra::MatchedGroup>,
    },
    // ── Link dialog ──
    /// Toggle the link insertion overlay.
    ToggleLinkDialog,
    /// URL field changed in the link dialog.
    LinkUrlChanged(String),
    /// Display text field changed in the link dialog.
    LinkTextChanged(String),
    /// Confirm link insertion.
    LinkInsert,
    // ── Signature ──
    /// Signature resolved for the current From account.
    SignatureResolved {
        signature_id: Option<String>,
        signature_html: Option<String>,
    },
}
