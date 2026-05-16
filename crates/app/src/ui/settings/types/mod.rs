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
    // Label editor
    LabelName,
    LabelColorBg,
    LabelColorFg,
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

/// State for the per-account label editor sheet.
///
/// Edits target one specific `(account_id, label_id)` pair. Create mode
/// leaves both blank and writes the new label to every account on save
/// (cross-account fan-out happens at the action layer, not here).
#[derive(Debug, Clone)]
pub struct LabelEditorState {
    /// Account whose label is being edited. Empty in create mode.
    #[allow(dead_code)]
    pub account_id: String,
    /// Label being edited. Empty in create mode.
    pub label_id: String,
    /// User-editable display name.
    pub name: String,
    /// User-selected background color hex (or the resolved current).
    pub color_bg: String,
    /// User-selected foreground color hex (or the resolved current).
    pub color_fg: String,
    /// True if `color_bg`/`color_fg` came from an existing user color.
    pub has_override: bool,
    /// Show the destructive delete confirmation modal.
    pub show_delete_confirmation: bool,
    /// True once the user has edited any field.
    pub dirty: bool,
}

impl LabelEditorState {
    pub fn new_create() -> Self {
        Self {
            account_id: String::new(),
            label_id: String::new(),
            name: String::new(),
            color_bg: "#999999".to_owned(),
            color_fg: "#ffffff".to_owned(),
            has_override: false,
            show_delete_confirmation: false,
            dirty: false,
        }
    }

    pub fn from_row(
        row: &rtsk::db::queries_extra::navigation::AccountLabelRow,
    ) -> Self {
        Self {
            account_id: row.account_id.clone(),
            label_id: row.label_id.clone(),
            name: row.name.clone(),
            color_bg: row.color_bg.clone(),
            color_fg: row.color_fg.clone(),
            has_override: row.has_color_override,
            show_delete_confirmation: false,
            dirty: false,
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
    // Mail Rules > Labels (raw per-account provider labels).
    // Loaded from `query_labels_by_account` on boot and after any label mutation.
    pub labels_by_account: Vec<rtsk::db::queries_extra::navigation::AccountLabelsGroup>,
    /// Label editor sheet state (Mail Rules > Labels).
    pub editing_label: Option<LabelEditorState>,
    // Demo data for Mail Rules > Filters (placeholder until filters land).
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
#[derive(Debug, Clone, Copy)]
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

    /// Apply persisted preferences from the DB bootstrap snapshots, called
    /// once at app startup. Mirrors the writes performed by the commit
    /// handler so a launch round-trip preserves what the user saved.
    /// Only fields tracked by `Settings` are pulled in; other snapshot
    /// fields are owned by other components.
    ///
    /// String values are matched case-insensitively against their canonical
    /// option lists - early DBs were seeded with lowercase values that
    /// don't match the dropdowns, and reading those literally would leave
    /// the dropdown displaying an unselectable string. Unknown values are
    /// dropped, leaving the existing `Settings::default()` value in place.
    pub fn apply_bootstrap(
        &mut self,
        ui: &rtsk::db::queries::UiBootstrapSnapshot,
        settings: &rtsk::db::queries::SettingsBootstrapSnapshot,
    ) {
        if let Some(canonical) = ui.theme.as_deref().and_then(canonical_theme) {
            self.theme = canonical.into();
        }
        if let Some(canonical) = ui.font_size.as_deref().and_then(canonical_font_size) {
            self.font_size = canonical.into();
        }
        if let Some(canonical) = ui
            .reading_pane_position
            .as_deref()
            .and_then(canonical_reading_pane)
        {
            self.reading_pane_position = canonical.into();
        }
        self.sync_status_bar = ui.show_sync_status;
        self.block_remote_images = settings.block_remote_images;
        self.phishing_detection = settings.phishing_detection_enabled;
        if let Some(canonical) = settings
            .phishing_sensitivity
            .as_deref()
            .and_then(canonical_phishing_sensitivity)
        {
            self.phishing_sensitivity = canonical.into();
        }
        self.committed_preferences = self.snapshot_preferences();
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
            labels_by_account: Vec::new(),
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
            editing_label: None,
            signatures: Vec::new(),
            signature_editor: None,
            confirm_delete_signature: None,
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

/// Resolve `value` to one of `options` using ASCII case-insensitive
/// equality. Returns the canonical option string when matched so the
/// dropdown can highlight it. Returns `None` for anything else.
fn canonical_match(value: &str, options: &[&'static str]) -> Option<&'static str> {
    options
        .iter()
        .find(|opt| opt.eq_ignore_ascii_case(value))
        .copied()
}

fn canonical_theme(value: &str) -> Option<&'static str> {
    canonical_match(value, &["System", "Light", "Dark", "Theme"])
}

fn canonical_font_size(value: &str) -> Option<&'static str> {
    canonical_match(value, &["Small", "Default", "Large", "XLarge"])
}

fn canonical_reading_pane(value: &str) -> Option<&'static str> {
    canonical_match(value, &["Right", "Bottom", "Hidden"])
}

fn canonical_phishing_sensitivity(value: &str) -> Option<&'static str> {
    canonical_match(value, &["Low", "Default", "High"])
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtsk::db::queries::{SettingsBootstrapSnapshot, UiBootstrapSnapshot};

    fn empty_ui_snapshot() -> UiBootstrapSnapshot {
        UiBootstrapSnapshot {
            active_account_id: None,
            language: None,
            global_compose_shortcut: None,
            custom_shortcuts: None,
            search_index_version: None,
            theme: None,
            sidebar_collapsed: false,
            contact_sidebar_visible: true,
            reading_pane_position: None,
            read_filter: None,
            email_list_width: None,
            email_density: None,
            mark_as_read_behavior: None,
            font_size: None,
            color_theme: None,
            inbox_view_mode: None,
            show_sync_status: true,
            task_sidebar_visible: false,
            sidebar_nav_config: None,
        }
    }

    fn default_settings_snapshot() -> SettingsBootstrapSnapshot {
        SettingsBootstrapSnapshot {
            notifications_enabled: true,
            undo_send_delay_seconds: None,
            block_remote_images: true,
            phishing_detection_enabled: true,
            phishing_sensitivity: None,
            sync_period_days: None,
            ai_provider: None,
            ollama_server_url: None,
            ollama_model: None,
            claude_model: None,
            openai_model: None,
            gemini_model: None,
            copilot_model: None,
            ai_enabled: true,
            ai_auto_categorize: true,
            ai_auto_summarize: true,
            ai_auto_draft_enabled: true,
            ai_writing_style_enabled: true,
            auto_archive_categories: None,
            smart_notifications: true,
            notify_categories: None,
            attachment_cache_max_mb: None,
            compress_attachments: true,
            allow_lossy_compression: false,
            opened_files_cleanup_days: None,
        }
    }

    /// Every Settings field that the commit handler in `handlers/core.rs`
    /// writes to the DB must round-trip through `apply_bootstrap` from the
    /// matching snapshot field. String values are validated against the
    /// canonical option lists; only the booleans flow through unchanged.
    #[test]
    fn apply_bootstrap_round_trips_seven_persisted_fields() {
        let mut settings = Settings::default();

        let mut ui = empty_ui_snapshot();
        ui.theme = Some("Dark".into());
        ui.font_size = Some("Large".into());
        ui.reading_pane_position = Some("Bottom".into());
        ui.show_sync_status = false;

        let mut s = default_settings_snapshot();
        s.block_remote_images = false;
        s.phishing_detection_enabled = false;
        s.phishing_sensitivity = Some("High".into());

        settings.apply_bootstrap(&ui, &s);

        assert_eq!(settings.theme, "Dark");
        assert_eq!(settings.font_size, "Large");
        assert_eq!(settings.reading_pane_position, "Bottom");
        assert!(!settings.sync_status_bar);
        assert!(!settings.block_remote_images);
        assert!(!settings.phishing_detection);
        assert_eq!(settings.phishing_sensitivity, "High");
    }

    /// Pre-existing dev DBs were seeded with lowercase values like 'system'
    /// or 'right' that don't match the dropdown option lists, leaving the
    /// dropdown displaying an unselectable string. `apply_bootstrap` must
    /// canonicalize these against their option lists - case-insensitively -
    /// rather than copying them through verbatim.
    #[test]
    fn apply_bootstrap_canonicalizes_case_for_string_prefs() {
        let mut settings = Settings::default();

        let mut ui = empty_ui_snapshot();
        ui.theme = Some("system".into());
        ui.font_size = Some("default".into());
        ui.reading_pane_position = Some("right".into());
        let mut s = default_settings_snapshot();
        s.phishing_sensitivity = Some("default".into());

        settings.apply_bootstrap(&ui, &s);

        assert_eq!(settings.theme, "System");
        assert_eq!(settings.font_size, "Default");
        assert_eq!(settings.reading_pane_position, "Right");
        assert_eq!(settings.phishing_sensitivity, "Default");
    }

    /// A garbage value in the DB (typo, abandoned theme, value left over
    /// from a renamed option) must not overwrite the in-memory default and
    /// leave the dropdown in an unselectable state.
    #[test]
    fn apply_bootstrap_keeps_default_when_db_value_is_unknown() {
        let mut settings = Settings::default();
        let original_theme = settings.theme.clone();
        let original_font_size = settings.font_size.clone();

        let mut ui = empty_ui_snapshot();
        ui.theme = Some("Solarized-Lapland".into());
        ui.font_size = Some("XXXLarge".into());
        let s = default_settings_snapshot();

        settings.apply_bootstrap(&ui, &s);

        assert_eq!(settings.theme, original_theme);
        assert_eq!(settings.font_size, original_font_size);
    }

    /// `committed_preferences` is the baseline the editor opens with. After
    /// `apply_bootstrap` it must reflect the persisted values, otherwise
    /// the first edit shows a spurious "unsaved changes" state.
    #[test]
    fn apply_bootstrap_resets_committed_shadow() {
        let mut settings = Settings::default();
        settings.committed_preferences.theme = "Light".into();
        settings.committed_preferences.sync_status_bar = true;

        let mut ui = empty_ui_snapshot();
        ui.theme = Some("Dark".into());
        ui.show_sync_status = false;

        let s = default_settings_snapshot();
        settings.apply_bootstrap(&ui, &s);

        assert_eq!(settings.committed_preferences.theme, "Dark");
        assert!(!settings.committed_preferences.sync_status_bar);
        // Live and committed must be in lockstep so the next begin_editing()
        // sees an empty diff.
        assert_eq!(settings.theme, settings.committed_preferences.theme);
        assert_eq!(
            settings.sync_status_bar,
            settings.committed_preferences.sync_status_bar
        );
    }

    /// Snapshot fields that arrive as `None` (no row in the settings table)
    /// must leave existing Settings fields untouched, so a fresh DB without
    /// rows for these keys boots with sensible defaults rather than empty
    /// strings.
    #[test]
    fn apply_bootstrap_leaves_string_defaults_when_snapshot_is_none() {
        let mut settings = Settings::default();
        let original_theme = settings.theme.clone();
        let original_font_size = settings.font_size.clone();
        let original_pane = settings.reading_pane_position.clone();
        let original_sensitivity = settings.phishing_sensitivity.clone();

        let ui = empty_ui_snapshot();
        let s = default_settings_snapshot();
        settings.apply_bootstrap(&ui, &s);

        assert_eq!(settings.theme, original_theme);
        assert_eq!(settings.font_size, original_font_size);
        assert_eq!(settings.reading_pane_position, original_pane);
        assert_eq!(settings.phishing_sensitivity, original_sensitivity);
    }
}
