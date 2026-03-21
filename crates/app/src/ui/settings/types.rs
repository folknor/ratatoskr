use iced::animation::{self, Easing};
use iced::time::Duration;

use crate::db::DateDisplay;
use crate::ui::undoable::UndoableText;

use ratatoskr_rich_text_editor::EditorState as RteEditorState;

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
    ListGripPress(String, usize),         // grip pressed — start potential drag
    ListDragMove(String, Point),          // cursor moved while grip held
    ListDragEnd(String),                  // grip released — end drag
    ListRowClick(String, usize),          // row clicked (not grip) — toggle
    ListRemove(String, usize),            // (list_id, item index)
    ListAdd(String),                      // (list_id)
    ListToggle(String, usize, bool),      // (list_id, item index, new value)
    ListMenu(String, usize),              // (list_id, item index)
    // Input/info rows
    FocusInput(String),
    CopyToClipboard(String),
    UndoInput(InputField),
    RedoInput(InputField),
    Noop,
    // Help tooltips
    HelpHover(String),
    HelpUnhover(String),
    ToggleHelpPin(String),
    DismissHelp,
    // Overlay
    OpenOverlay(SettingsOverlay),
    CloseOverlay,
    OverlayAnimTick(iced::time::Instant),
    // Accounts tab
    AddAccountFromSettings,
    AccountCardClicked(String),
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
    SignatureEdit(String),                     // signature_id — open editor overlay
    SignatureCreate(String),                   // account_id — open editor for new sig
    SignatureDelete(String),                   // signature_id — request delete (shows confirm)
    SignatureDeleteConfirmed(String),          // signature_id — confirmed delete
    SignatureDeleteCancelled,                  // cancel pending delete
    SignatureEditorNameChanged(String),
    SignatureEditorBodyChanged(String),
    SignatureEditorAction(ratatoskr_rich_text_editor::Action),
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
    Closed,
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
        params: ratatoskr_core::db::queries_extra::UpdateAccountParams,
    },
    /// Request the App to load contacts (with filter).
    LoadContacts(String),
    /// Request the App to load groups (with filter).
    LoadGroups(String),
    /// Request the App to load group members by group ID.
    LoadGroupMembers(String),
}

/// Overlays that slide in from the right, covering the settings content.
/// One level deep — no stacking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsOverlay {
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

/// Editing state for the signature editor overlay.
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

/// Editing state for the contact editor overlay.
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
}

/// Editing state for the group editor overlay.
#[derive(Debug, Clone)]
pub struct GroupEditorState {
    pub group_id: Option<String>,
    pub name: String,
    pub members: Vec<String>,
    pub filter: String,
}

pub struct Settings {
    pub active_tab: Tab,
    pub open_select: Option<SelectField>,
    // General
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
    // Overlay
    pub overlay: Option<SettingsOverlay>,
    pub overlay_anim: animation::Animation<bool>,
    // Help tooltips
    pub hovered_help: Option<String>,
    pub pinned_help: Option<String>,
    // Editable lists
    pub drag_state: Option<DragState>,
    // Demo data for Mail Rules tab
    pub demo_labels: Vec<EditableItem>,
    pub demo_filters: Vec<EditableItem>,
    // Accounts tab
    pub managed_accounts: Vec<ManagedAccount>,
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

/// An item in an editable list.
#[derive(Debug, Clone)]
pub struct EditableItem {
    pub label: String,
    pub enabled: Option<bool>,
}

impl Settings {
    pub fn with_scale(scale: f32) -> Self {
        Self {
            scale,
            ..Self::default()
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            active_tab: Tab::General,
            open_select: None,
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
            overlay: None,
            overlay_anim: animation::Animation::new(false)
                .easing(Easing::EaseOutCubic)
                .duration(Duration::from_millis(200)),
            hovered_help: None,
            pinned_help: None,
            drag_state: None,
            demo_labels: vec![
                EditableItem { label: "Important".into(), enabled: Some(true) },
                EditableItem { label: "Personal".into(), enabled: Some(true) },
                EditableItem { label: "Receipts".into(), enabled: Some(false) },
                EditableItem { label: "Travel".into(), enabled: None },
            ],
            demo_filters: vec![
                EditableItem { label: "Auto-archive promotions".into(), enabled: Some(true) },
                EditableItem { label: "Star from VIPs".into(), enabled: Some(true) },
            ],
            managed_accounts: Vec::new(),
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
        }
    }
}
