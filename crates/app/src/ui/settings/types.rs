use iced::animation::{self, Easing};
use iced::time::Duration;

use crate::db::DateDisplay;
use crate::pop_out::RenderingMode;
use crate::ui::undoable::UndoableText;

use rte::EditorState as RteEditorState;

// ── Email body background preference ────────────────────

/// Controls the background color of email body containers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmailBodyBackground {
    /// Always use a white background (best for email rendering fidelity).
    #[default]
    AlwaysWhite,
    /// Use the current theme's background color.
    MatchTheme,
    /// White in light themes, theme background in dark themes.
    Auto,
}

impl EmailBodyBackground {
    pub fn label(self) -> &'static str {
        match self {
            Self::AlwaysWhite => "Always White",
            Self::MatchTheme => "Match Theme",
            Self::Auto => "Auto",
        }
    }

    pub fn from_label(s: &str) -> Self {
        match s {
            "Match Theme" => Self::MatchTheme,
            "Auto" => Self::Auto,
            _ => Self::AlwaysWhite,
        }
    }
}

// ── Messages ────────────────────────────────────────────

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
    ToggleSendAndArchive(bool),
    UndoDelayChanged(String),
    DefaultReplyChanged(String),
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
    SaveAiSettings,
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
    SignatureEdit(String),            // signature_id - open editor sheet
    SignatureCreate(String),          // account_id - open editor for new sig
    SignatureDelete(String),          // signature_id - request delete (shows confirm)
    SignatureDeleteConfirmed(String), // signature_id - confirmed delete
    SignatureDeleteCancelled,         // cancel pending delete
    SignatureEditorNameChanged(String),
    SignatureEditorBodyChanged(String),
    SignatureEditorAction(rte::Action),
    SignatureEditorToggleDefault(bool),
    SignatureEditorToggleReplyDefault(bool),
    SignatureEditorSave,
    SignatureDragGripPress(usize),
    SignatureDragMove(Point),
    SignatureDragEnd,
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
    GroupMembersLoaded(String, Result<Vec<String>, String>),
}

// Use iced::Point for ListDragMove
use iced::Point;

/// Which field in the contact editor changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContactField {
    DisplayName,
    Email,
    Email2,
    Phone,
    Company,
    Notes,
}

/// Events the settings component emits upward to the App.
#[derive(Debug, Clone)]
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
    /// Request the App to execute a contact import.
    ExecuteContactImport {
        contacts: Vec<import::ImportedContact>,
        account_id: Option<String>,
        update_existing: bool,
    },
    /// Request the App to persist reordered account sort orders.
    ReorderAccounts(Vec<(String, i64)>),
    /// Request the App to open the re-auth wizard for an account.
    ReauthenticateAccount(String),
}

/// Settings pages that slide in from the right, covering the settings content.
/// One level deep - no stacking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsSheetPage {
    CreateFilter,
    AccountEditor,
    EditSignature {
        /// None for new signature, Some for editing existing.
        signature_id: Option<String>,
        account_id: String,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectField {
    Theme,
    ReadingPane,
    Density,
    FontSize,
    UndoDelay,
    DefaultReply,
    MarkAsRead,
    AiProvider,
    AiModel,
    DateDisplay,
    EmailBodyBg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Accounts,
    General,
    Theme,
    Notifications,
    Composing,
    MailRules,
    People,
    Shortcuts,
    Ai,
    About,
}

impl Tab {
    pub(super) const ALL: &[Tab] = &[
        Tab::Accounts,
        Tab::General,
        Tab::Theme,
        Tab::Notifications,
        Tab::Composing,
        Tab::MailRules,
        Tab::People,
        Tab::Shortcuts,
        Tab::Ai,
        Tab::About,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Tab::Accounts => "Accounts",
            Tab::General => "General",
            Tab::Theme => "Theme",
            Tab::Notifications => "Notifications",
            Tab::Composing => "Composing",
            Tab::MailRules => "Mail Rules",
            Tab::People => "People",
            Tab::Shortcuts => "Shortcuts",
            Tab::Ai => "AI",
            Tab::About => "About",
        }
    }

    pub(super) fn icon(self) -> iced::widget::Text<'static> {
        use crate::icon;
        match self {
            Tab::Accounts => icon::users(),
            Tab::General => icon::settings(),
            Tab::Theme => icon::palette(),
            Tab::Notifications => icon::bell(),
            Tab::Composing => icon::pencil(),
            Tab::MailRules => icon::filter(),
            Tab::People => icon::users(),
            Tab::Shortcuts => icon::zap(),
            Tab::Ai => icon::globe(),
            Tab::About => icon::info(),
        }
    }
}

// ── Preferences shadow ──────────────────────────────────
//
// The "config shadow pattern": on settings open, clone the current
// preferences into `editing_preferences`. All edits go to the shadow.
// Save commits the shadow back, Close/Cancel discards it and restores
// the original values. This enables live preview with safe rollback.

/// Snapshot of user-facing preferences that support live preview.
/// Compared with `PartialEq` for change detection.
#[derive(Debug, Clone, PartialEq)]
pub struct PreferencesState {
    pub theme: String,
    pub scale: f32,
    pub density: String,
    pub font_size: String,
    pub date_display: DateDisplay,
    pub reading_pane_position: String,
    pub sync_status_bar: bool,
    pub block_remote_images: bool,
    pub phishing_detection: bool,
    pub phishing_sensitivity: String,
    pub default_rendering_mode: RenderingMode,
    pub email_body_background: EmailBodyBackground,
}

// ── State ───────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputField {
    VipEmail,
    AiApiKey,
    OllamaUrl,
    OllamaModel,
    SignatureName,
}

// ── Signature types ──────────────────────────────────────

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

/// State for signature drag-reorder within a single account section.
#[derive(Debug, Clone)]
pub struct SignatureDragState {
    /// Account ID of the group being dragged within.
    pub account_id: String,
    /// Index of the signature being dragged within the account's list.
    pub dragging_index: usize,
    /// Y coordinate when the grip was pressed (list-relative).
    pub start_y: f32,
    /// Whether the mouse has moved far enough to count as a real drag.
    pub is_dragging: bool,
}

/// Editing state for the contact editor sheet.
#[derive(Debug, Clone)]
pub struct ContactEditorState {
    pub contact_id: Option<String>,
    pub account_id: Option<String>,
    pub display_name: String,
    pub email: String,
    pub email2: String,
    pub phone: String,
    pub company: String,
    pub notes: String,
    /// Contact source: "user" (local), "google", "graph", "carddav", etc.
    /// None for newly created contacts (treated as local).
    pub source: Option<String>,
    /// Provider-assigned server ID for synced contacts.
    pub server_id: Option<String>,
    /// Whether fields have been modified since opening the editor.
    pub dirty: bool,
}

/// Editing state for the group editor sheet.
#[derive(Debug, Clone)]
pub struct GroupEditorState {
    pub group_id: Option<String>,
    pub name: String,
    pub members: Vec<String>,
    pub filter: String,
    /// Whether fields have been modified since opening the editor.
    pub dirty: bool,
}

/// Re-export of `ContactField` from the import crate, used in settings messages.
/// We wrap it to derive the traits needed for iced messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportContactField {
    DisplayName,
    FirstName,
    LastName,
    Email,
    Email2,
    Phone,
    Company,
    Notes,
    Group,
    Ignore,
}

impl ImportContactField {
    pub const ALL_OPTIONS: &[ImportContactField] = &[
        ImportContactField::Ignore,
        ImportContactField::DisplayName,
        ImportContactField::FirstName,
        ImportContactField::LastName,
        ImportContactField::Email,
        ImportContactField::Email2,
        ImportContactField::Phone,
        ImportContactField::Company,
        ImportContactField::Notes,
        ImportContactField::Group,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ImportContactField::DisplayName => "Name",
            ImportContactField::FirstName => "First Name",
            ImportContactField::LastName => "Last Name",
            ImportContactField::Email => "Email",
            ImportContactField::Email2 => "Email 2",
            ImportContactField::Phone => "Phone",
            ImportContactField::Company => "Company",
            ImportContactField::Notes => "Notes",
            ImportContactField::Group => "Group",
            ImportContactField::Ignore => "Ignore",
        }
    }

    /// Convert to the import crate's `ContactField`.
    pub fn to_import_field(self) -> import::ContactField {
        match self {
            ImportContactField::DisplayName => import::ContactField::DisplayName,
            ImportContactField::FirstName => import::ContactField::FirstName,
            ImportContactField::LastName => import::ContactField::LastName,
            ImportContactField::Email => import::ContactField::Email,
            ImportContactField::Email2 => import::ContactField::Email2,
            ImportContactField::Phone => import::ContactField::Phone,
            ImportContactField::Company => import::ContactField::Company,
            ImportContactField::Notes => import::ContactField::Notes,
            ImportContactField::Group => import::ContactField::Group,
            ImportContactField::Ignore => import::ContactField::Ignore,
        }
    }

    /// Convert from the import crate's `ContactField`.
    pub fn from_import_field(field: import::ContactField) -> Self {
        match field {
            import::ContactField::DisplayName => ImportContactField::DisplayName,
            import::ContactField::FirstName => ImportContactField::FirstName,
            import::ContactField::LastName => ImportContactField::LastName,
            import::ContactField::Email => ImportContactField::Email,
            import::ContactField::Email2 => ImportContactField::Email2,
            import::ContactField::Phone => ImportContactField::Phone,
            import::ContactField::Company => ImportContactField::Company,
            import::ContactField::Notes => ImportContactField::Notes,
            import::ContactField::Group => ImportContactField::Group,
            import::ContactField::Ignore => ImportContactField::Ignore,
        }
    }
}

impl std::fmt::Display for ImportContactField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// The current step of the import wizard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportStep {
    /// Waiting for file selection.
    FileSelect,
    /// Column mapping + preview (CSV only).
    Mapping,
    /// Preview for vCard imports (no mapping needed).
    VcfPreview,
    /// Import is running.
    Importing,
    /// Import complete - show summary.
    Summary,
}

/// Result of an import operation.
#[derive(Debug, Clone)]
pub struct ImportResult {
    pub imported: usize,
    pub skipped_no_email: usize,
    pub skipped_duplicate: usize,
    pub updated: usize,
    pub groups_created: usize,
}

/// State for the contact import wizard.
#[derive(Debug, Clone)]
pub struct ImportWizardState {
    pub step: ImportStep,
    /// Selected file path (display only).
    pub file_path: Option<String>,
    /// Parsed import source.
    pub source: Option<import::ImportSource>,
    /// Preview data (for CSV).
    pub preview: Option<import::ImportPreview>,
    /// Column mappings (one per header column).
    pub mappings: Vec<ImportContactField>,
    /// Whether the first row is treated as a header.
    pub has_header: bool,
    /// Parsed vCard contacts (for VCF files).
    pub vcf_contacts: Vec<import::ImportedContact>,
    /// Target account for import.
    pub account_id: Option<String>,
    /// Whether to update existing contacts on duplicate email.
    pub update_existing: bool,
    /// Import result after completion.
    pub result: Option<ImportResult>,
}

impl ImportWizardState {
    pub fn new() -> Self {
        Self {
            step: ImportStep::FileSelect,
            file_path: None,
            source: None,
            preview: None,
            mappings: Vec::new(),
            has_header: true,
            vcf_contacts: Vec::new(),
            account_id: None,
            update_existing: false,
            result: None,
        }
    }
}

pub struct Settings {
    pub active_tab: Tab,
    pub open_select: Option<SelectField>,
    // General - preferences shadow pattern
    /// The committed (persisted) preferences. Updated only on explicit save.
    pub committed_preferences: PreferencesState,
    /// The editing shadow. `Some` when settings panel is open. All preference
    /// edits go here. Save commits it to `committed_preferences`, cancel
    /// discards it and restores the committed values.
    pub editing_preferences: Option<PreferencesState>,
    pub scale: f32,
    pub scale_preview: Option<f32>,
    pub theme: String,
    pub density: String,
    pub font_size: String,
    pub reading_pane_position: String,
    pub selected_theme: Option<usize>,
    pub sync_status_bar: bool,
    pub block_remote_images: bool,
    pub phishing_detection: bool,
    pub phishing_sensitivity: String,
    pub date_display: DateDisplay,
    pub default_rendering_mode: RenderingMode,
    pub email_body_background: EmailBodyBackground,
    // Composing
    pub undo_delay: String,
    pub send_and_archive: bool,
    pub default_reply_mode: String,
    pub mark_as_read: String,
    // Notifications
    pub notifications_enabled: bool,
    pub smart_notifications: bool,
    pub notify_categories: Vec<String>,
    pub vip_email_input: UndoableText,
    pub vip_senders: Vec<String>,
    // AI
    pub ai_provider: String,
    pub ai_api_key: UndoableText,
    pub ai_model: String,
    pub ai_ollama_url: UndoableText,
    pub ai_ollama_model: UndoableText,
    pub ai_enabled: bool,
    pub ai_auto_categorize: bool,
    pub ai_auto_summarize: bool,
    pub ai_auto_draft: bool,
    pub ai_writing_style: bool,
    pub ai_auto_archive_updates: bool,
    pub ai_auto_archive_promotions: bool,
    pub ai_auto_archive_social: bool,
    pub ai_auto_archive_newsletters: bool,
    pub ai_key_saved: bool,
    // Sheet
    pub active_sheet: Option<SettingsSheetPage>,
    pub sheet_anim: animation::Animation<bool>,
    // Help tooltips
    pub hovered_help: Option<String>,
    // Editable lists
    pub drag_state: Option<DragState>,
    // Demo data for Mail Rules tab
    pub demo_labels: Vec<EditableItem>,
    pub demo_filters: Vec<EditableItem>,
    // Accounts tab
    pub managed_accounts: Vec<ManagedAccount>,
    pub account_drag: Option<AccountDragState>,
    pub editing_account: Option<AccountEditor>,
    // Signatures
    pub signatures: Vec<SignatureEntry>,
    pub signature_editor: Option<SignatureEditorState>,
    /// Signature ID pending delete confirmation.
    pub confirm_delete_signature: Option<String>,
    /// Active signature drag-reorder state.
    pub signature_drag: Option<SignatureDragState>,
    // Contacts management
    pub contact_filter: String,
    pub contacts: Vec<crate::db::ContactEntry>,
    pub contact_editor: Option<ContactEditorState>,
    /// Pending contact delete ID awaiting confirmation.
    pub confirm_delete_contact: Option<String>,
    // Groups management
    pub group_filter: String,
    pub groups: Vec<crate::db::GroupEntry>,
    pub group_editor: Option<GroupEditorState>,
    /// Pending group delete ID awaiting confirmation.
    pub confirm_delete_group: Option<String>,
    // Contact import wizard
    pub import_wizard: Option<ImportWizardState>,
}

/// An account card in the settings list.
#[derive(Debug, Clone)]
pub struct ManagedAccount {
    pub id: String,
    pub email: String,
    pub provider: String,
    pub account_name: Option<String>,
    pub account_color: Option<String>,
    pub display_name: Option<String>,
    pub last_sync_at: Option<i64>,
    pub health: AccountHealth,
}

/// Account health status for the settings card indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountHealth {
    Healthy,
    Warning,
    Error,
    Disabled,
}

impl Default for AccountHealth {
    fn default() -> Self {
        Self::Healthy
    }
}

/// Compute account health from token expiry and sync state.
pub fn compute_health(
    last_sync_at: Option<i64>,
    token_expires_at: Option<i64>,
    is_active: bool,
) -> AccountHealth {
    if !is_active {
        return AccountHealth::Disabled;
    }
    let now = chrono::Utc::now().timestamp();
    if let Some(expires) = token_expires_at {
        let no_recent_sync = last_sync_at.map_or(true, |ls| now - ls > 3600);
        if expires < now && no_recent_sync {
            return AccountHealth::Error;
        }
        if expires < now + 86400 {
            return AccountHealth::Warning;
        }
    }
    AccountHealth::Healthy
}

/// The slide-in editor state for a single account.
#[derive(Debug, Clone)]
pub struct AccountEditor {
    pub account_id: String,
    pub account_email: String,
    pub account_name: String,
    pub display_name: String,
    pub account_color_index: Option<usize>,
    pub caldav_url: String,
    pub caldav_username: String,
    pub caldav_password: String,
    pub show_delete_confirmation: bool,
    pub dirty: bool,
}

/// State for an active drag operation.
#[derive(Debug, Clone)]
pub struct DragState {
    pub list_id: String,
    pub dragging_index: usize,
    /// Y coordinate when the grip was pressed (list-relative, set on first move).
    pub start_y: f32,
    /// Whether the mouse has moved far enough to count as a real drag.
    pub is_dragging: bool,
}

/// Minimum Y movement before a grip press becomes a drag.
pub(super) const DRAG_START_THRESHOLD: f32 = 4.0;

/// State for an active account card drag operation.
#[derive(Debug, Clone)]
pub struct AccountDragState {
    pub dragging_index: usize,
    /// Y coordinate when the grip was pressed (list-relative, set on first move).
    pub start_y: f32,
    /// Whether the mouse has moved far enough to count as a real drag.
    pub is_dragging: bool,
}

/// An item in an editable list.
#[derive(Debug, Clone)]
pub struct EditableItem {
    pub label: String,
    pub enabled: Option<bool>,
}

impl Settings {
    pub fn with_scale(scale: f32) -> Self {
        let mut s = Self::default();
        s.scale = scale;
        s.committed_preferences.scale = scale;
        s
    }

    /// Snapshot current preferences into the editing shadow.
    /// Called when the settings panel opens.
    pub fn begin_editing(&mut self) {
        self.editing_preferences = Some(self.snapshot_preferences());
    }

    /// Commit the editing shadow to committed state and apply to live fields.
    /// Called on explicit save or when closing with intent to keep changes.
    pub fn commit_preferences(&mut self) {
        if let Some(prefs) = self.editing_preferences.take() {
            self.committed_preferences = prefs.clone();
            self.apply_preferences(&prefs);
        }
    }

    /// Discard the editing shadow and restore committed preferences.
    /// Called when the user cancels or navigates away without saving.
    pub fn discard_preferences(&mut self) {
        self.editing_preferences = None;
        let committed = self.committed_preferences.clone();
        self.apply_preferences(&committed);
    }

    /// Whether the editing shadow differs from committed state.
    /// Returns `false` if no editing session is active.
    pub fn has_unsaved_changes(&self) -> bool {
        self.editing_preferences
            .as_ref()
            .is_some_and(|editing| *editing != self.committed_preferences)
    }

    /// Build a `PreferencesState` from the current live fields.
    fn snapshot_preferences(&self) -> PreferencesState {
        PreferencesState {
            theme: self.theme.clone(),
            scale: self.scale,
            density: self.density.clone(),
            font_size: self.font_size.clone(),
            date_display: self.date_display,
            reading_pane_position: self.reading_pane_position.clone(),
            sync_status_bar: self.sync_status_bar,
            block_remote_images: self.block_remote_images,
            phishing_detection: self.phishing_detection,
            phishing_sensitivity: self.phishing_sensitivity.clone(),
            default_rendering_mode: self.default_rendering_mode,
            email_body_background: self.email_body_background,
        }
    }

    /// Apply a `PreferencesState` to the live fields (for preview or restore).
    fn apply_preferences(&mut self, prefs: &PreferencesState) {
        self.theme = prefs.theme.clone();
        self.scale = prefs.scale;
        self.density = prefs.density.clone();
        self.font_size = prefs.font_size.clone();
        self.date_display = prefs.date_display;
        self.reading_pane_position = prefs.reading_pane_position.clone();
        self.sync_status_bar = prefs.sync_status_bar;
        self.block_remote_images = prefs.block_remote_images;
        self.phishing_detection = prefs.phishing_detection;
        self.phishing_sensitivity = prefs.phishing_sensitivity.clone();
        self.default_rendering_mode = prefs.default_rendering_mode;
        self.email_body_background = prefs.email_body_background;
        crate::ui::theme::set_email_body_background(prefs.email_body_background);
    }
}

impl Default for Settings {
    fn default() -> Self {
        let initial_preferences = PreferencesState {
            theme: "Light".into(),
            scale: 1.0,
            density: "Default".into(),
            font_size: "Default".into(),
            date_display: DateDisplay::RelativeOffset,
            reading_pane_position: "Right".into(),
            sync_status_bar: true,
            block_remote_images: false,
            phishing_detection: true,
            phishing_sensitivity: "Default".into(),
            default_rendering_mode: RenderingMode::default(),
            email_body_background: EmailBodyBackground::default(),
        };
        Self {
            active_tab: Tab::General,
            open_select: None,
            committed_preferences: initial_preferences,
            editing_preferences: None,
            scale: 1.0,
            scale_preview: None,
            theme: "Light".into(),
            density: "Default".into(),
            font_size: "Default".into(),
            reading_pane_position: "Right".into(),
            selected_theme: None,
            sync_status_bar: true,
            block_remote_images: false,
            phishing_detection: true,
            phishing_sensitivity: "Default".into(),
            date_display: DateDisplay::RelativeOffset,
            default_rendering_mode: RenderingMode::default(),
            email_body_background: EmailBodyBackground::default(),
            undo_delay: "5 seconds".into(),
            send_and_archive: false,
            default_reply_mode: "Reply".into(),
            mark_as_read: "After 2 Seconds".into(),
            notifications_enabled: true,
            smart_notifications: true,
            notify_categories: vec!["Primary".into()],
            vip_email_input: UndoableText::new(),
            vip_senders: Vec::new(),
            ai_provider: "Claude".into(),
            ai_api_key: UndoableText::new(),
            ai_model: "claude-sonnet-4-6".into(),
            ai_ollama_url: UndoableText::with_initial("http://localhost:11434"),
            ai_ollama_model: UndoableText::with_initial("llama3.2"),
            ai_enabled: true,
            ai_auto_categorize: true,
            ai_auto_summarize: true,
            ai_auto_draft: true,
            ai_writing_style: true,
            ai_auto_archive_updates: false,
            ai_auto_archive_promotions: false,
            ai_auto_archive_social: false,
            ai_auto_archive_newsletters: false,
            ai_key_saved: false,
            active_sheet: None,
            sheet_anim: animation::Animation::new(false)
                .easing(Easing::EaseOutCubic)
                .duration(Duration::from_millis(200)),
            hovered_help: None,
            drag_state: None,
            demo_labels: vec![
                EditableItem {
                    label: "Important".into(),
                    enabled: Some(true),
                },
                EditableItem {
                    label: "Personal".into(),
                    enabled: Some(true),
                },
                EditableItem {
                    label: "Receipts".into(),
                    enabled: Some(false),
                },
                EditableItem {
                    label: "Travel".into(),
                    enabled: None,
                },
            ],
            demo_filters: vec![
                EditableItem {
                    label: "Auto-archive promotions".into(),
                    enabled: Some(true),
                },
                EditableItem {
                    label: "Star from VIPs".into(),
                    enabled: Some(true),
                },
            ],
            managed_accounts: Vec::new(),
            account_drag: None,
            editing_account: None,
            signatures: Vec::new(),
            signature_editor: None,
            confirm_delete_signature: None,
            signature_drag: None,
            contact_filter: String::new(),
            contacts: Vec::new(),
            contact_editor: None,
            confirm_delete_contact: None,
            group_filter: String::new(),
            groups: Vec::new(),
            group_editor: None,
            confirm_delete_group: None,
            import_wizard: None,
        }
    }
}
