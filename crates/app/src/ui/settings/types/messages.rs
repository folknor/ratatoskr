use iced::Point;

use super::contacts::{ContactField, ImportContactField};
use super::import_wizard::ImportResult;
use super::signatures::SignatureSaveRequest;
use super::{FilterId, InputField, Tab};

use crate::db::DateDisplay;

#[derive(Debug, Clone)]
pub enum SettingsMessage {
    Close,
    SelectTab(Tab),
    // General
    ScaleDragged(f32),
    ScaleReleased,
    ThemeChanged(String),
    DensityChanged(String),
    FontSizeChanged(String),
    ReadingPaneChanged(String),
    ThemeSelected(usize),
    ToggleSyncStatusBar(bool),
    ToggleBlockRemoteImages(bool),
    TogglePhishingDetection(bool),
    PhishingSensitivityChanged(String),
    DateDisplayChanged(String),
    EmailBodyBgChanged(String),
    ToggleSelect(SelectField),
    // About
    CheckForUpdates,
    OpenGithub,
    // Composing
    UndoDelayChanged(String),
    MarkAsReadChanged(String),
    // Notifications
    ToggleNotifications(bool),
    ToggleSmartNotifications(bool),
    ToggleNotifyCategory(String),
    VipEmailChanged(String),
    AddVipSender,
    RemoveVipSender(String),
    // AI
    AiProviderChanged(String),
    AiModelChanged(String),
    ToggleAiEnabled(bool),
    ToggleAiAutoCategorize(bool),
    ToggleAiAutoSummarize(bool),
    ToggleAiAutoDraft(bool),
    ToggleAiWritingStyle(bool),
    ToggleAiAutoArchiveUpdates(bool),
    ToggleAiAutoArchivePromotions(bool),
    ToggleAiAutoArchiveSocial(bool),
    ToggleAiAutoArchiveNewsletters(bool),
    AiApiKeyChanged(String),
    OllamaUrlChanged(String),
    OllamaModelChanged(String),
    // Editable list
    ListGripPress(String, usize), // grip pressed - start potential drag
    ListDragMove(String, Point),  // cursor moved while grip held
    ListDragEnd(String),          // grip released - end drag
    ListRowClick(String, usize),  // row clicked (not grip) - toggle
    ListRemove(String, usize),    // (list_id, item index)
    ListAdd(String),              // (list_id)
    ListToggle(String, usize, bool), // (list_id, item index, new value)
    ListMenu(String, usize),      // (list_id, item index)
    // Input/info rows
    FocusInput(String),
    CopyToClipboard(String),
    UndoInput(InputField),
    RedoInput(InputField),
    Noop,
    /// Save current preference edits (commit shadow to committed state).
    SavePreferences,
    /// Cancel preference edits (discard shadow, restore committed state).
    CancelPreferences,
    // Help tooltips
    HelpHover(String),
    HelpUnhover(String),
    // Sheet
    OpenSheet(SettingsSheetPage),
    CloseSheet,
    SheetAnimTick(iced::time::Instant),
    /// User confirmed discarding pending editor changes via the
    /// "Discard unsaved changes?" dialog. Applies whatever dismissal
    /// target was deferred (close-sheet, switch-tab, ...).
    ConfirmDiscardEditorChanges,
    /// User declined to discard, dismissing the confirm dialog.
    CancelDiscardEditorChanges,
    // Accounts tab
    AddAccountFromSettings,
    AccountCardClicked(String),
    AccountGripPress(usize),
    AccountDragMove(Point),
    AccountDragEnd,
    CloseAccountEditor,
    SaveAccountEditor,
    AccountNameEditorChanged(String),
    DisplayNameEditorChanged(String),
    AccountColorEditorChanged(usize),
    CaldavUrlChanged(String),
    CaldavUsernameChanged(String),
    CaldavPasswordChanged(String),
    ReauthenticateAccount(String),
    DeleteAccountRequested(String),
    DeleteAccountConfirmed(String),
    DeleteAccountCancelled,
    // Signatures
    SignatureEdit(String),                 // signature_id - open editor sheet
    SignatureCreate, // open editor for a new signature (no account preselected)
    SignatureDelete(String), // signature_id - request delete (shows confirm)
    SignatureDeleteConfirmed(String), // signature_id - confirmed delete
    SignatureDeleteCancelled, // cancel pending delete
    SignatureEditorAccountChanged(String), // account_id selected in the editor's account picker
    SignatureEditorNameChanged(String),
    SignatureEditorBodyChanged(String),
    SignatureEditorAction(rte::Action),
    SignatureEditorToggleDefault(bool),
    SignatureEditorToggleReplyDefault(bool),
    SignatureEditorSave,
    // Filter inputs (People tab + group editor)
    FilterFocused(FilterId),
    FilterCleared(FilterId),
    /// Result of a widget-tree focus query (run after each mouse press)
    /// telling us which filter input - if any - currently owns focus.
    FilterFocusUpdated(Option<FilterId>),
    // Contacts
    ContactFilterChanged(String),
    ContactsLoaded(Result<Vec<crate::db::ContactEntry>, String>),
    ContactClick(String),
    ContactEditorFieldChanged(ContactField, String),
    ContactEditorSave,
    ContactEditorAccountChanged(Option<String>),
    ContactDelete(String),
    ContactConfirmDelete(String),
    ContactCancelDelete,
    ContactCreate,
    ContactSaved(Result<(), String>),
    ContactDeleted(Result<(), String>),
    // Contact Import
    ImportContactsOpen,
    ImportFileSelected(String, Vec<u8>),
    ImportMappingChanged(usize, ImportContactField),
    ImportToggleHeader(bool),
    ImportToggleUpdateExisting(bool),
    ImportAccountChanged(Option<String>),
    ImportExecute,
    ImportExecuted(Result<ImportResult, String>),
    ImportBack,
    // Groups
    GroupFilterChanged(String),
    GroupsLoaded(Result<Vec<crate::db::GroupEntry>, String>),
    GroupClick(String),
    GroupCreate,
    GroupDelete(String),
    GroupConfirmDelete(String),
    GroupCancelDelete,
    GroupSaved(Result<(), String>),
    GroupDeleted(Result<(), String>),
    GroupEditorNameChanged(String),
    GroupEditorRemoveMember(String),
    GroupEditorAddMember(String),
    GroupEditorSave,
    GroupEditorFilterChanged(String),
    GroupEditorMembersFilterChanged(String),
    GroupMembersLoaded(String, Result<Vec<String>, String>),

    // ── Mail Rules > Labels editor ─────────────────────────
    /// Open the label editor sheet. Empty IDs = create new.
    OpenLabelEditor {
        account_id: String,
        label_id: String,
    },
    /// Name field in the editor changed.
    LabelEditorNameChanged(String),
    /// User picked a new color in the editor.
    /// Index into `label_colors::preset_colors::all_presets()` for the
    /// swatch the user picked.
    LabelEditorColorChanged(usize),
    /// User clicked the "+" tile in the colour grid - open a custom
    /// colour picker. Stub until the picker lands.
    LabelEditorOpenCustomColor,
    /// User reset the override (use synced/hash color again).
    LabelEditorColorReset,
    /// Save the editor (create or rename + recolor).
    LabelEditorSave,
    /// Delete the label being edited.
    LabelEditorDelete,
    /// Cancel and close the editor without saving.
    LabelEditorCancel,
    /// Confirm-delete dialog shown.
    LabelEditorConfirmDelete,
    /// Cancel the confirm-delete dialog.
    LabelEditorCancelDelete,

    // ── Settings > Labels group editor ─────────────────────
    /// Open the label-group editor sheet. None = create new.
    OpenLabelGroupEditor {
        group_id: Option<i64>,
    },
    /// Name field in the group editor changed.
    LabelGroupEditorNameChanged(String),
    /// User picked a new colour in the group editor. Index into
    /// `label_colors::preset_colors::all_presets()`.
    LabelGroupEditorColorChanged(usize),
    /// User clicked the "+" tile in the colour grid - open a custom
    /// colour picker. Stub until the picker lands.
    LabelGroupEditorOpenCustomColor,
    /// Add a raw `(account_id, label_id)` tag as a member of the group
    /// being edited.
    LabelGroupEditorAddMember(String, String),
    /// Remove a raw `(account_id, label_id)` tag from the group being
    /// edited.
    LabelGroupEditorRemoveMember(String, String),
    /// Async load result: existing members of the label group currently
    /// being edited.
    LabelGroupMembersLoaded(i64, Result<Vec<(String, String)>, String>),
    /// Save the group editor (create or update + members).
    LabelGroupEditorSave,
    /// Delete the group being edited.
    LabelGroupEditorDelete,
    /// Cancel and close the editor without saving.
    LabelGroupEditorCancel,
    /// Confirm-delete dialog shown.
    LabelGroupEditorConfirmDelete,
    /// Cancel the confirm-delete dialog.
    LabelGroupEditorCancelDelete,
}

/// Events the settings component emits upward to the App.
#[derive(Debug, Clone)]
#[allow(dead_code)] // ReorderSignatures is reserved for the upcoming drag-and-drop save path
pub enum SettingsEvent {
    /// Settings panel closed. If preferences were dirty, they have been
    /// committed (auto-save on close). The App should apply them.
    Closed,
    /// Preferences were explicitly saved via SavePreferences.
    PreferencesCommitted,
    /// Preferences were explicitly cancelled - committed state restored.
    PreferencesDiscarded,
    DateDisplayChanged(DateDisplay),
    OpenAddAccountWizard,
    /// Request the App to save a signature (insert or update) via core CRUD.
    SaveSignature(SignatureSaveRequest),
    /// Request the App to delete a signature by ID.
    DeleteSignature(String),
    /// Request the App to reorder signatures by ID list.
    ReorderSignatures(Vec<String>),
    /// Request the App to save a contact.
    SaveContact(crate::db::ContactEntry),
    /// Request the App to delete a contact by ID.
    DeleteContact(String),
    /// Request the App to save a group.
    SaveGroup(crate::db::GroupEntry, Vec<String>),
    /// Request the App to delete a group by ID.
    DeleteGroup(String),
    /// Request the App to delete an account.
    DeleteAccount(String),
    /// Request the App to save account editor changes.
    SaveAccountChanges {
        account_id: String,
        params: rtsk::db::queries_extra::UpdateAccountParams,
    },
    /// Request the App to load contacts (with filter).
    LoadContacts(String),
    /// Request the App to load groups (with filter).
    LoadGroups(String),
    /// Request the App to load group members by group ID.
    LoadGroupMembers(String),
    /// Load `(account_id, label_id)` members for a `label_groups` row.
    LoadLabelGroupMembers(i64),
    /// Request the App to execute a contact import.
    ExecuteContactImport {
        prepared: import::PreparedImport,
        account_id: Option<String>,
        update_existing: bool,
    },
    /// Request the App to persist reordered account sort orders.
    ReorderAccounts(Vec<(String, i64)>),
    /// Persist a new ordering for label groups. Pairs are
    /// `(label_groups.id, sort_order)`.
    ReorderLabelGroups(Vec<(i64, i64)>),
    /// Request the App to open the re-auth wizard for an account.
    ReauthenticateAccount(String),
}

/// Settings pages that slide in from the right, covering the settings content.
/// One level deep - no stacking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsSheetPage {
    AccountEditor,
    EditSignature {
        /// None for new signature, Some for editing existing.
        signature_id: Option<String>,
        /// None until the user picks an account in the editor (for new sigs).
        account_id: Option<String>,
    },
    EditContact {
        /// None for new contact, Some for editing existing.
        contact_id: Option<String>,
    },
    EditGroup {
        /// None for new group, Some for editing existing.
        group_id: Option<String>,
    },
    ImportContacts,
    /// Per-account label editor.
    /// Empty `account_id`+`label_id` = creating a new label.
    EditLabel {
        account_id: String,
        label_id: String,
    },
    /// User-visible label group editor.
    /// None = creating a new group.
    EditLabelGroup {
        group_id: Option<i64>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectField {
    Theme,
    ReadingPane,
    Density,
    FontSize,
    UndoDelay,
    MarkAsRead,
    SignatureAccount,
    ContactAccount,
    AiProvider,
    AiModel,
    DateDisplay,
    EmailBodyBg,
}
