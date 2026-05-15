use crate::ui::undoable::UndoableText;

use rte::EditorState as RteEditorState;

/// A signature entry for display in the settings list.
/// Mirrors the relevant fields of `DbSignature` without depending on the db crate.
#[derive(Debug, Clone)]
pub struct SignatureEntry {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub body_html: String,
    pub body_text: Option<String>,
    pub is_default: bool,
    pub is_reply_default: bool,
}

/// Request to save a signature, emitted upward to the App for DB persistence.
#[derive(Debug, Clone)]
pub struct SignatureSaveRequest {
    pub id: Option<String>,
    pub account_id: String,
    pub name: String,
    pub body_html: String,
    pub is_default: bool,
    pub is_reply_default: bool,
}

/// Editing state for the signature editor sheet.
#[derive(Debug, Clone)]
pub struct SignatureEditorState {
    /// The signature being edited (None = new).
    pub signature_id: Option<String>,
    pub account_id: String,
    pub name: UndoableText,
    /// Rich text editor state for the signature body.
    pub body_editor: RteEditorState,
    pub is_default: bool,
    pub is_reply_default: bool,
    /// Whether fields have been modified since opening the editor.
    pub dirty: bool,
}

