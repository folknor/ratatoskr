pub mod accounts;
pub mod contacts;
pub mod import_wizard;
pub mod messages;
pub mod preferences;
pub mod signatures;

pub use accounts::*;
pub use contacts::*;
pub use import_wizard::*;
pub use messages::*;
pub use preferences::*;
pub use signatures::*;

use iced::animation::{self, Easing};
use iced::time::Duration;

use crate::db::DateDisplay;
use crate::pop_out::RenderingMode;
use crate::ui::undoable::UndoableText;

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

/// Identifies a filter input so Escape / X-button handlers know which
/// filter to clear and which one currently owns focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterId {
    Contacts,
    Groups,
    GroupAddMembers,
    GroupMembers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputField {
    VipEmail,
    AiApiKey,
    OllamaUrl,
    OllamaModel,
    SignatureName,
    // Account editor
    AccountName,
    AccountDisplayName,
    CaldavUrl,
    CaldavUsername,
    CaldavPassword,
    // Group editor
    GroupName,
    // Contact editor
    ContactDisplayName,
    ContactEmail,
    ContactEmail2,
    ContactPhone,
    ContactCompany,
    ContactNotes,
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
    // Filter focus tracking
    pub focused_filter: Option<FilterId>,
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
    #[allow(dead_code)] // see SignatureDragState above
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
    /// Deferred dismissal target awaiting "Discard unsaved changes?" confirmation.
    /// `Some` while the confirm dialog is visible; `None` otherwise.
    pub pending_discard: Option<PendingDiscard>,
}

/// What dismissal action the user requested while an editor was dirty.
/// Applied verbatim once the user confirms via the discard-changes dialog.
#[derive(Debug, Clone)]
pub enum PendingDiscard {
    /// Close the slide-in sheet entirely (Back button, Escape, etc.).
    CloseSheet,
    /// Switch to a different settings tab.
    SwitchTab(Tab),
}

impl Settings {
    pub fn with_scale(scale: f32) -> Self {
        let mut s = Self {
            scale,
            ..Self::default()
        };
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
    #[allow(dead_code)] // not yet consumed by close-without-save confirmation flow
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
            focused_filter: None,
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
            pending_discard: None,
        }
    }
}

impl Settings {
    /// Returns `true` if any of the slide-in editor sheets has unsaved
    /// changes (the `dirty` flag on its state). Used by the dismissal
    /// chokepoint to decide whether to pop the discard-confirmation dialog.
    pub fn any_editor_dirty(&self) -> bool {
        self.contact_editor.as_ref().is_some_and(|e| e.dirty)
            || self.group_editor.as_ref().is_some_and(|e| e.dirty)
            || self.signature_editor.as_ref().is_some_and(|e| e.dirty)
            || self.editing_account.as_ref().is_some_and(|e| e.dirty)
    }

    /// Apply a dismissal target unconditionally (no dirty check).
    pub(crate) fn apply_dismissal(&mut self, target: PendingDiscard) {
        // Both targets share the same editor teardown; only their final
        // state diverges.
        self.active_sheet = None;
        self.sheet_anim
            .go_mut(false, iced::time::Instant::now());
        self.signature_editor = None;
        self.editing_account = None;
        self.contact_editor = None;
        self.group_editor = None;
        match target {
            PendingDiscard::CloseSheet => {
                self.import_wizard = None;
            }
            PendingDiscard::SwitchTab(tab) => {
                self.active_tab = tab;
                self.hovered_help = None;
            }
        }
    }

    /// Single chokepoint for any "leave the active editor" request.
    /// If an editor is dirty, defers the action behind a confirm dialog
    /// (`pending_discard`). Otherwise applies immediately.
    pub(crate) fn try_dismiss_editor(&mut self, target: PendingDiscard) {
        if self.any_editor_dirty() {
            self.pending_discard = Some(target);
        } else {
            self.apply_dismissal(target);
        }
    }
}
