use std::sync::Arc;

use iced::widget::{button, column, container, mouse_area, pick_list, row, scrollable, text, text_input, Space};
use iced::{Alignment, Element, Length, Point};
use ratatoskr_rich_text_editor::{
    rich_text_editor, Action as RteAction, EditAction, EditorState, InlineStyle,
};

use crate::db::{self, ContactMatch};
use crate::font;
use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::token_input::{self, TokenId, TokenInputMessage, TokenInputValue};
use crate::ui::widgets;
use crate::Message;

use super::PopOutMessage;

// ── Data types ──────────────────────────────────────────

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
            Self::Reply { original_subject }
            | Self::ReplyAll { original_subject } => {
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

/// Account info for the From dropdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountInfo {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub account_name: Option<String>,
}

impl std::fmt::Display for AccountInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(ref name) = self.display_name {
            write!(f, "{name} <{}>", self.email)
        } else {
            write!(f, "{}", self.email)
        }
    }
}

/// An attachment queued for sending.
#[derive(Debug, Clone)]
pub struct ComposeAttachment {
    /// Original file name.
    pub name: String,
    /// MIME type (guessed from extension).
    pub mime_type: String,
    /// File contents.
    pub data: Arc<Vec<u8>>,
}

impl ComposeAttachment {
    /// Human-readable file size.
    pub fn display_size(&self) -> String {
        let bytes = self.data.len();
        if bytes < 1024 {
            format!("{bytes} B")
        } else if bytes < 1024 * 1024 {
            format!("{:.1} KB", bytes as f64 / 1024.0)
        } else {
            format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
        }
    }
}

/// Guess a MIME type from a file name (uses the `mime_guess` crate's
/// database of 800+ extension mappings, falling back to
/// `application/octet-stream` for unknown extensions).
pub fn mime_from_extension(name: &str) -> String {
    mime_guess::from_path(name)
        .first_raw()
        .unwrap_or("application/octet-stream")
        .to_string()
}

// ── Messages ────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ComposeMessage {
    SubjectChanged(String),
    BodyChanged(RteAction),
    FromAccountChanged(AccountInfo),
    ShowCc,
    ShowBcc,
    ToTokenInput(TokenInputMessage),
    CcTokenInput(TokenInputMessage),
    BccTokenInput(TokenInputMessage),
    Send,
    Discard,
    /// Toggle discard confirmation dialog.
    ToggleDiscardConfirm,
    /// Autocomplete results arrived from the database.
    AutocompleteResults(ratatoskr_core::generation::GenerationToken<ratatoskr_core::generation::Autocomplete>, Result<Vec<ContactMatch>, String>),
    /// User selected an autocomplete suggestion.
    AutocompleteSelect(usize),
    /// User navigated the autocomplete list (up/down).
    AutocompleteNavigate(i32),
    /// Dismiss the autocomplete dropdown.
    AutocompleteDismiss,
    /// Formatting toolbar actions — emit ToggleInlineStyle to the rich text editor.
    FormatBold,
    FormatItalic,
    FormatUnderline,
    FormatStrikethrough,
    /// List / blockquote toggle via SetBlockType.
    FormatList,
    FormatBlockquote,
    /// Open the link insertion dialog.
    FormatLink,
    // ── Attachments ──
    /// User clicked the attach button — opens file picker.
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
    /// Mouse released — drop the token.
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

// ── Autocomplete state ──────────────────────────────────

/// Which recipient field is currently active for autocomplete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecipientField {
    To,
    Cc,
    Bcc,
}

/// State for the recipient autocomplete dropdown.
pub struct AutocompleteState {
    /// Current query being searched.
    pub query: String,
    /// Results from the most recent autocomplete search.
    pub results: Vec<ContactMatch>,
    /// Currently highlighted index in the results list.
    pub highlighted: Option<usize>,
    /// Generation counter to discard stale results.
    pub search_generation: ratatoskr_core::generation::GenerationCounter<ratatoskr_core::generation::Autocomplete>,
    /// Which recipient field is currently active.
    pub active_field: RecipientField,
}

impl AutocompleteState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            results: Vec::new(),
            highlighted: None,
            search_generation: ratatoskr_core::generation::GenerationCounter::new(),
            active_field: RecipientField::To,
        }
    }
}

/// Active drag state for a token being moved between fields.
pub struct ComposeTokenDrag {
    pub token_id: TokenId,
    pub source_field: RecipientField,
    pub label: String,
    pub current_position: Point,
}

/// A Bcc nudge banner shown when a group token is added to To or Cc.
pub struct BccNudgeBanner {
    pub group_name: String,
    pub token_id: TokenId,
    pub source_field: RecipientField,
}

/// A bulk paste banner shown when 10+ addresses are pasted.
pub struct BulkPasteBanner {
    pub count: usize,
}

/// State for the right-click context menu on a token.
pub struct TokenContextMenuState {
    pub token_id: TokenId,
    pub field: RecipientField,
    pub position: Point,
    pub is_group: bool,
}

// ── State ───────────────────────────────────────────────

/// Per-window state for a compose pop-out.
pub struct ComposeState {
    // Recipients
    pub to: TokenInputValue,
    pub cc: TokenInputValue,
    pub bcc: TokenInputValue,
    pub show_cc: bool,
    pub show_bcc: bool,
    pub selected_to_token: Option<TokenId>,
    pub selected_cc_token: Option<TokenId>,
    pub selected_bcc_token: Option<TokenId>,

    // From account
    pub from_account: Option<AccountInfo>,
    pub from_accounts: Vec<AccountInfo>,

    // Subject
    pub subject: String,

    // Body (rich text editor)
    pub body: EditorState,

    // Compose mode
    pub mode: ComposeMode,

    // Reply context
    pub reply_thread_id: Option<String>,
    pub reply_message_id: Option<String>,

    // Status message (e.g. "Send not yet wired")
    pub status: Option<String>,

    // Discard confirmation
    pub discard_confirm_open: bool,

    // Autocomplete
    pub autocomplete: AutocompleteState,

    // Attachments
    pub attachments: Vec<ComposeAttachment>,

    // Context menu
    pub context_menu: Option<TokenContextMenuState>,

    // Drag and drop
    pub drag: Option<ComposeTokenDrag>,

    // Banners
    pub bcc_nudges: Vec<BccNudgeBanner>,
    pub bulk_paste_banner: Option<BulkPasteBanner>,

    // Link dialog
    pub link_dialog_open: bool,
    pub link_url: String,
    pub link_text: String,

    // Window geometry
    pub width: f32,
    pub height: f32,

    // Signature tracking
    pub active_signature_id: Option<String>,
    pub signature_separator_index: Option<usize>,

    // Draft auto-save
    pub draft_id: String,
    pub draft_dirty: bool,

    // Send in progress — disables Send button, shows "Sending..." status
    pub sending: bool,

    // Draft ID for the send path — set on first send attempt, reused on retry
    // so that failed retries update the existing draft row instead of creating
    // a new one.
    pub send_draft_id: Option<String>,
}

impl ComposeState {
    pub fn new(accounts: &[db::Account]) -> Self {
        let from_accounts = accounts_to_info(accounts);
        let from_account = from_accounts.first().cloned();
        Self {
            to: TokenInputValue::new(),
            cc: TokenInputValue::new(),
            bcc: TokenInputValue::new(),
            show_cc: false,
            show_bcc: false,
            selected_to_token: None,
            selected_cc_token: None,
            selected_bcc_token: None,
            from_account,
            from_accounts,
            subject: String::new(),
            body: EditorState::new(),
            mode: ComposeMode::New,
            reply_thread_id: None,
            reply_message_id: None,
            status: None,
            discard_confirm_open: false,
            autocomplete: AutocompleteState::new(),
            context_menu: None,
            drag: None,
            bcc_nudges: Vec::new(),
            bulk_paste_banner: None,
            attachments: Vec::new(),
            link_dialog_open: false,
            link_url: String::new(),
            link_text: String::new(),
            active_signature_id: None,
            signature_separator_index: None,
            width: COMPOSE_DEFAULT_WIDTH,
            height: COMPOSE_DEFAULT_HEIGHT,
            draft_id: uuid::Uuid::new_v4().to_string(),
            draft_dirty: false,
            sending: false,
            send_draft_id: None,
        }
    }

    pub fn new_reply(
        accounts: &[db::Account],
        mode: &ComposeMode,
        to_email: Option<&str>,
        to_name: Option<&str>,
        cc_emails: Option<&str>,
        quoted_body: Option<&str>,
        thread_id: Option<&str>,
        message_id: Option<&str>,
    ) -> Self {
        let mut state = Self::new(accounts);
        state.mode = mode.clone();

        // Set subject
        state.subject = mode.prefixed_subject();

        // Add To recipient (not for Forward — forward starts with empty To)
        if !matches!(state.mode, ComposeMode::Forward { .. }) {
            if let Some(email) = to_email {
                let label = to_name
                    .filter(|n| !n.is_empty())
                    .unwrap_or(email);
                let id = state.to.next_token_id();
                state.to.tokens.push(token_input::Token {
                    id,
                    email: email.to_string(),
                    label: label.to_string(),
                    is_group: false,
                    group_id: None,
                    member_count: None,
                });
            }
        }

        // Add Cc recipients for ReplyAll
        if let ComposeMode::ReplyAll { .. } = &state.mode {
            if let Some(cc_str) = cc_emails {
                for addr in
                    cc_str.split(',').map(str::trim).filter(|s| !s.is_empty())
                {
                    let id = state.cc.next_token_id();
                    state.cc.tokens.push(token_input::Token {
                        id,
                        email: addr.to_string(),
                        label: addr.to_string(),
                        is_group: false,
                        group_id: None,
                        member_count: None,
                    });
                }
                if !state.cc.tokens.is_empty() {
                    state.show_cc = true;
                }
            }
        }

        // Set quoted body with attribution line as HTML blockquote
        if let Some(body) = quoted_body {
            let attribution = build_attribution(to_name, to_email);
            // Build HTML: empty paragraph for user to type, attribution, blockquote
            let escaped_body = body
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            let body_paras = escaped_body
                .lines()
                .map(|line| format!("<p>{line}</p>"))
                .collect::<Vec<_>>()
                .join("");
            let html = format!(
                "<p><br></p><p><em>{attribution}</em></p><blockquote>{body_paras}</blockquote>"
            );
            state.body = EditorState::from_html(&html);
            // Mark block index 1 as the signature insertion point so that
            // the initial `SignatureResolved` message inserts the signature
            // between the user content area and the quoted text (attribution
            // + blockquote).  Block 0 is the empty paragraph for the user;
            // block 1 is the attribution line.
            state.signature_separator_index = Some(1);
        }

        state.reply_thread_id = thread_id.map(String::from);
        state.reply_message_id = message_id.map(String::from);

        state
    }

    /// Window title based on compose mode.
    pub fn window_title(&self) -> String {
        match &self.mode {
            ComposeMode::New => "New Message".to_string(),
            _ => self.mode.prefixed_subject(),
        }
    }

    /// Override the From identity for a shared mailbox context.
    ///
    /// Inserts a synthetic `AccountInfo` for the shared mailbox email at the
    /// front of the From dropdown and selects it. The `parent_account_id` is
    /// the authenticated account that will actually perform the send.
    pub fn set_shared_mailbox_from(
        &mut self,
        parent_account_id: &str,
        shared_email: &str,
    ) {
        let shared_info = AccountInfo {
            id: parent_account_id.to_string(),
            email: shared_email.to_string(),
            display_name: None,
            account_name: Some(format!("{shared_email} (shared)")),
        };
        self.from_accounts.insert(0, shared_info.clone());
        self.from_account = Some(shared_info);
    }

    /// Returns true if the compose body has user content beyond the
    /// initial quoted text / signature.
    fn has_user_content(&self) -> bool {
        // Simple heuristic: non-empty flattened text
        let body_text = self.body.document.flattened_text();
        let trimmed = body_text.trim();
        !trimmed.is_empty()
    }
}

// ── Update ──────────────────────────────────────────────

/// Update compose state for a given message.
///
/// NOTE: The caller (`handlers/pop_out.rs`) must check
/// `handlers::contacts::should_trigger_autocomplete(&msg)` BEFORE calling
/// this function. If it returns `true`, the caller should call
/// `handlers::contacts::dispatch_autocomplete_search(db, window_id, state)`
/// AFTER this function returns, to fire the async DB search.
pub fn update_compose(state: &mut ComposeState, msg: ComposeMessage) {
    match msg {
        ComposeMessage::SubjectChanged(s) => {
            state.subject = s;
            state.draft_dirty = true;
        }
        ComposeMessage::BodyChanged(action) => {
            state.body.perform(action);
            state.draft_dirty = true;
        }
        ComposeMessage::FromAccountChanged(account) => {
            state.from_account = Some(account);
        }
        ComposeMessage::ShowCc => state.show_cc = true,
        ComposeMessage::ShowBcc => state.show_bcc = true,
        ComposeMessage::ToTokenInput(inner) => {
            handle_recipient_token_input(state, RecipientField::To, inner);
        }
        ComposeMessage::CcTokenInput(inner) => {
            handle_recipient_token_input(state, RecipientField::Cc, inner);
        }
        ComposeMessage::BccTokenInput(inner) => {
            handle_recipient_token_input(state, RecipientField::Bcc, inner);
        }
        ComposeMessage::Send => {
            // V1: validate and show stub status
            let has_recipients = !state.to.tokens.is_empty()
                || !state.cc.tokens.is_empty()
                || !state.bcc.tokens.is_empty();
            if !has_recipients {
                state.status =
                    Some("Add at least one recipient".to_string());
                return;
            }
            // TODO: When send is implemented, call
            // `status_bar.show_confirmation("Message sent".to_string())`
            // on the success path (via a Message variant back to App).
            state.status = Some("Send not yet wired".to_string());
        }
        ComposeMessage::Discard => {
            // Handled by the caller (close window)
        }
        ComposeMessage::ToggleDiscardConfirm => {
            state.discard_confirm_open = !state.discard_confirm_open;
        }
        // Autocomplete
        ComposeMessage::AutocompleteResults(generation, Ok(results)) => {
            if state.autocomplete.search_generation.is_current(generation) {
                state.autocomplete.results = results;
                state.autocomplete.highlighted = if state.autocomplete.results.is_empty() {
                    None
                } else {
                    Some(0)
                };
            }
        }
        ComposeMessage::AutocompleteResults(_, Err(_)) => {
            state.autocomplete.results.clear();
            state.autocomplete.highlighted = None;
        }
        ComposeMessage::AutocompleteSelect(idx) => {
            if let Some(match_entry) = state.autocomplete.results.get(idx).cloned() {
                let label = match_entry.display_name.as_deref()
                    .filter(|n| !n.is_empty())
                    .unwrap_or(&match_entry.email)
                    .to_string();
                let target = match state.autocomplete.active_field {
                    RecipientField::To => &mut state.to,
                    RecipientField::Cc => &mut state.cc,
                    RecipientField::Bcc => &mut state.bcc,
                };
                let id = target.next_token_id();
                target.tokens.push(token_input::Token {
                    id,
                    email: match_entry.email,
                    label,
                    is_group: match_entry.is_group,
                    group_id: match_entry.group_id,
                    member_count: match_entry.member_count,
                });
                target.text.clear();
                state.autocomplete.results.clear();
                state.autocomplete.highlighted = None;
                state.autocomplete.query.clear();
            }
        }
        ComposeMessage::AutocompleteNavigate(delta) => {
            if let Some(current) = state.autocomplete.highlighted {
                let len = state.autocomplete.results.len();
                if len > 0 {
                    let new_idx = if delta > 0 {
                        (current + 1).min(len - 1)
                    } else if current > 0 {
                        current - 1
                    } else {
                        0
                    };
                    state.autocomplete.highlighted = Some(new_idx);
                }
            }
        }
        ComposeMessage::AutocompleteDismiss => {
            state.autocomplete.results.clear();
            state.autocomplete.highlighted = None;
        }
        // Context menu actions
        ComposeMessage::ShowTokenContextMenu { field, token_id, position } => {
            let is_group = match field {
                RecipientField::To => &state.to,
                RecipientField::Cc => &state.cc,
                RecipientField::Bcc => &state.bcc,
            }
            .tokens
            .iter()
            .any(|t| t.id == token_id && t.is_group);
            state.context_menu = Some(TokenContextMenuState {
                token_id,
                field,
                position,
                is_group,
            });
        }
        ComposeMessage::DismissContextMenu => {
            state.context_menu = None;
        }
        ComposeMessage::ContextMenuDelete { field, token_id } => {
            let tokens = match field {
                RecipientField::To => &mut state.to.tokens,
                RecipientField::Cc => &mut state.cc.tokens,
                RecipientField::Bcc => &mut state.bcc.tokens,
            };
            tokens.retain(|t| t.id != token_id);
            state.context_menu = None;
            state.draft_dirty = true;
        }
        ComposeMessage::ContextMenuMoveTo { token_id, from, to_field } => {
            let source_tokens = match from {
                RecipientField::To => &mut state.to.tokens,
                RecipientField::Cc => &mut state.cc.tokens,
                RecipientField::Bcc => &mut state.bcc.tokens,
            };
            if let Some(pos) = source_tokens.iter().position(|t| t.id == token_id) {
                let mut token = source_tokens.remove(pos);
                let target = match to_field {
                    RecipientField::To => &mut state.to,
                    RecipientField::Cc => &mut state.cc,
                    RecipientField::Bcc => &mut state.bcc,
                };
                token.id = target.next_token_id();
                target.tokens.push(token);
                // Show the target field if hidden
                match to_field {
                    RecipientField::Cc => state.show_cc = true,
                    RecipientField::Bcc => state.show_bcc = true,
                    RecipientField::To => {}
                }
            }
            state.context_menu = None;
            state.draft_dirty = true;
        }
        ComposeMessage::ContextMenuExpandGroup { .. } => {
            // Group expansion requires DB access — handled by pop_out.rs
            state.context_menu = None;
        }
        ComposeMessage::GroupExpanded { field, token_id, members } => {
            if let Ok(member_list) = members {
                let tokens = match field {
                    RecipientField::To => &mut state.to,
                    RecipientField::Cc => &mut state.cc,
                    RecipientField::Bcc => &mut state.bcc,
                };
                // Remove the group token
                tokens.tokens.retain(|t| t.id != token_id);
                // Add individual member tokens
                for (email, display_name) in member_list {
                    let label = display_name
                        .as_deref()
                        .filter(|n| !n.is_empty())
                        .unwrap_or(&email)
                        .to_string();
                    let id = tokens.next_token_id();
                    tokens.tokens.push(token_input::Token {
                        id,
                        email,
                        label,
                        is_group: false,
                        group_id: None,
                        member_count: None,
                    });
                }
                state.draft_dirty = true;
            }
        }
        // Drag and drop
        ComposeMessage::DragStarted { field, token_id } => {
            let label = match field {
                RecipientField::To => &state.to,
                RecipientField::Cc => &state.cc,
                RecipientField::Bcc => &state.bcc,
            }
            .tokens
            .iter()
            .find(|t| t.id == token_id)
            .map(|t| t.label.clone())
            .unwrap_or_default();
            state.drag = Some(ComposeTokenDrag {
                token_id,
                source_field: field,
                label,
                current_position: Point::ORIGIN,
            });
        }
        ComposeMessage::DragMove(pos) => {
            if let Some(ref mut drag) = state.drag {
                drag.current_position = pos;
            }
        }
        ComposeMessage::DragEnd(_pos) => {
            // Drop detection would require knowing field bounds at runtime.
            // For now, just cancel — the context menu "Move to" is the
            // primary cross-field move mechanism. Full visual DnD with
            // hit-testing requires storing field bounds from the view pass.
            state.drag = None;
        }
        ComposeMessage::DragCancel => {
            state.drag = None;
        }
        // Bcc nudge banner
        ComposeMessage::BccNudgeAccept(token_id) => {
            // Find which field has this token (To or Cc) and move to Bcc
            let source = if state.to.tokens.iter().any(|t| t.id == token_id) {
                Some(RecipientField::To)
            } else if state.cc.tokens.iter().any(|t| t.id == token_id) {
                Some(RecipientField::Cc)
            } else {
                None
            };
            if let Some(from) = source {
                let source_tokens = match from {
                    RecipientField::To => &mut state.to.tokens,
                    RecipientField::Cc => &mut state.cc.tokens,
                    RecipientField::Bcc => &mut state.bcc.tokens,
                };
                if let Some(pos) = source_tokens.iter().position(|t| t.id == token_id) {
                    let mut token = source_tokens.remove(pos);
                    token.id = state.bcc.next_token_id();
                    state.bcc.tokens.push(token);
                    state.show_bcc = true;
                }
            }
            state.bcc_nudges.retain(|n| n.token_id != token_id);
            state.draft_dirty = true;
        }
        ComposeMessage::BccNudgeDismiss(token_id) => {
            state.bcc_nudges.retain(|n| n.token_id != token_id);
        }
        ComposeMessage::BulkPasteDismiss => {
            state.bulk_paste_banner = None;
        }
        // Formatting toolbar — emit ToggleInlineStyle to the rich text editor
        ComposeMessage::FormatBold => {
            state.body.perform(RteAction::Edit(
                EditAction::ToggleInlineStyle(InlineStyle::BOLD),
            ));
            state.draft_dirty = true;
        }
        ComposeMessage::FormatItalic => {
            state.body.perform(RteAction::Edit(
                EditAction::ToggleInlineStyle(InlineStyle::ITALIC),
            ));
            state.draft_dirty = true;
        }
        ComposeMessage::FormatUnderline => {
            state.body.perform(RteAction::Edit(
                EditAction::ToggleInlineStyle(InlineStyle::UNDERLINE),
            ));
            state.draft_dirty = true;
        }
        ComposeMessage::FormatStrikethrough => {
            state.body.perform(RteAction::Edit(
                EditAction::ToggleInlineStyle(InlineStyle::STRIKETHROUGH),
            ));
            state.draft_dirty = true;
        }
        ComposeMessage::FormatList => {
            state.body.perform(RteAction::Edit(
                EditAction::SetBlockType(
                    ratatoskr_rich_text_editor::BlockKind::ListItem {
                        ordered: false,
                    },
                ),
            ));
            state.draft_dirty = true;
        }
        ComposeMessage::FormatBlockquote => {
            // BlockQuote toggle not yet supported via SetBlockType
        }
        // Link dialog
        ComposeMessage::FormatLink | ComposeMessage::ToggleLinkDialog => {
            if !state.link_dialog_open {
                // Pre-fill display text with current selection
                state.link_text = state.body.selection_text();
                state.link_url.clear();
            }
            state.link_dialog_open = !state.link_dialog_open;
        }
        ComposeMessage::LinkUrlChanged(url) => state.link_url = url,
        ComposeMessage::LinkTextChanged(t) => state.link_text = t,
        ComposeMessage::LinkInsert => {
            let url = state.link_url.trim().to_string();
            let display = state.link_text.trim().to_string();
            if !url.is_empty() {
                let link_label = if display.is_empty() {
                    url.clone()
                } else {
                    display
                };
                // Build a small HTML snippet with the link and paste it
                // by deleting the selection, inserting the text, then
                // we rely on the editor's paste mechanism.
                // Delete selection first if any.
                if !state.body.selection.is_collapsed() {
                    state.body.perform(RteAction::Edit(
                        EditAction::DeleteSelection,
                    ));
                }
                // Insert the display text
                state.body.perform(RteAction::Edit(
                    EditAction::InsertText(link_label),
                ));
                // Note: The rich text editor does not currently expose
                // a link-insertion API via EditAction. The text is
                // inserted without a hyperlink. A future editor update
                // will add an InsertLink action.
            }
            state.link_dialog_open = false;
            state.link_url.clear();
            state.link_text.clear();
        }
        // Attachments
        ComposeMessage::AttachFiles => {
            // Handled by the pop-out handler (async file picker)
        }
        ComposeMessage::FilesSelected(files) => {
            state.attachments.extend(files);
        }
        ComposeMessage::RemoveAttachment(idx) => {
            if idx < state.attachments.len() {
                state.attachments.remove(idx);
            }
        }
        ComposeMessage::SignatureResolved {
            signature_id,
            signature_html,
        } => {
            use ratatoskr_rich_text_editor::compose::replace_signature;

            let old_sep = state.signature_separator_index.unwrap_or(
                state.body.document.block_count(),
            );
            let new_sep = replace_signature(
                &mut state.body.document,
                old_sep,
                state.signature_separator_index,
                signature_html.as_deref(),
            );
            state.signature_separator_index = new_sep;
            state.active_signature_id = signature_id;
            state.draft_dirty = true;
        }
    }
}

/// Handle a token input message for a specific recipient field.
/// Intercepts autocomplete keyboard events before delegating to the
/// generic token input handler.
fn handle_recipient_token_input(
    state: &mut ComposeState,
    field: RecipientField,
    inner: TokenInputMessage,
) {
    // Intercept autocomplete keyboard events
    match &inner {
        TokenInputMessage::AutocompleteDown => {
            let len = state.autocomplete.results.len();
            if len > 0 {
                let next = state
                    .autocomplete
                    .highlighted
                    .map_or(0, |h| (h + 1).min(len - 1));
                state.autocomplete.highlighted = Some(next);
            }
            return;
        }
        TokenInputMessage::AutocompleteUp => {
            if let Some(h) = state.autocomplete.highlighted {
                state.autocomplete.highlighted = Some(h.saturating_sub(1));
            }
            return;
        }
        TokenInputMessage::AutocompleteAccept => {
            let idx = state.autocomplete.highlighted.unwrap_or(0);
            if let Some(match_entry) =
                state.autocomplete.results.get(idx).cloned()
            {
                let label = match_entry
                    .display_name
                    .as_deref()
                    .filter(|n| !n.is_empty())
                    .unwrap_or(&match_entry.email)
                    .to_string();
                let target = match state.autocomplete.active_field {
                    RecipientField::To => &mut state.to,
                    RecipientField::Cc => &mut state.cc,
                    RecipientField::Bcc => &mut state.bcc,
                };
                let id = target.next_token_id();
                let is_group = match_entry.is_group;
                let token_label = label.clone();
                target.tokens.push(token_input::Token {
                    id,
                    email: match_entry.email,
                    label,
                    is_group,
                    group_id: match_entry.group_id,
                    member_count: match_entry.member_count,
                });
                target.text.clear();
                state.autocomplete.results.clear();
                state.autocomplete.highlighted = None;
                state.autocomplete.query.clear();
                state.draft_dirty = true;

                // Bcc nudge: suggest moving group to Bcc if added to To/Cc
                let active = state.autocomplete.active_field;
                if is_group
                    && (active == RecipientField::To
                        || active == RecipientField::Cc)
                {
                    state.bcc_nudges.push(BccNudgeBanner {
                        group_name: token_label,
                        token_id: id,
                        source_field: active,
                    });
                }
            }
            return;
        }
        TokenInputMessage::AutocompleteDismissKey => {
            state.autocomplete.results.clear();
            state.autocomplete.highlighted = None;
            return;
        }
        TokenInputMessage::DragStarted(token_id) => {
            let tid = *token_id;
            let label = match field {
                RecipientField::To => &state.to,
                RecipientField::Cc => &state.cc,
                RecipientField::Bcc => &state.bcc,
            }
            .tokens
            .iter()
            .find(|t| t.id == tid)
            .map(|t| t.label.clone())
            .unwrap_or_default();
            state.drag = Some(ComposeTokenDrag {
                token_id: tid,
                source_field: field,
                label,
                current_position: Point::ORIGIN,
            });
            return;
        }
        TokenInputMessage::TokenContextMenu(token_id, position) => {
            let tid = *token_id;
            let pos = *position;
            let is_group = match field {
                RecipientField::To => &state.to,
                RecipientField::Cc => &state.cc,
                RecipientField::Bcc => &state.bcc,
            }
            .tokens
            .iter()
            .any(|t| t.id == tid && t.is_group);

            state.context_menu = Some(TokenContextMenuState {
                token_id: tid,
                field,
                position: pos,
                is_group,
            });
            return;
        }
        _ => {}
    }

    // Track which field is active for autocomplete
    if let TokenInputMessage::TextChanged(ref t) = inner {
        state.autocomplete.query = t.clone();
        state.autocomplete.active_field = field;
    }

    // Track token count before paste to detect bulk paste
    let is_paste = matches!(&inner, TokenInputMessage::Paste(_));
    let tokens_before = if is_paste {
        match field {
            RecipientField::To => state.to.tokens.len(),
            RecipientField::Cc => state.cc.tokens.len(),
            RecipientField::Bcc => state.bcc.tokens.len(),
        }
    } else {
        0
    };

    let (value, selected) = match field {
        RecipientField::To => (&mut state.to, &mut state.selected_to_token),
        RecipientField::Cc => (&mut state.cc, &mut state.selected_cc_token),
        RecipientField::Bcc => {
            (&mut state.bcc, &mut state.selected_bcc_token)
        }
    };
    handle_token_input_message(value, inner, selected);
    state.draft_dirty = true;

    // Bulk paste banner: show if 10+ addresses were added by paste
    if is_paste {
        let tokens_after = match field {
            RecipientField::To => state.to.tokens.len(),
            RecipientField::Cc => state.cc.tokens.len(),
            RecipientField::Bcc => state.bcc.tokens.len(),
        };
        let added = tokens_after.saturating_sub(tokens_before);
        if added >= 10 {
            state.bulk_paste_banner = Some(BulkPasteBanner { count: added });
        }
    }
}

fn handle_token_input_message(
    value: &mut TokenInputValue,
    msg: TokenInputMessage,
    selected: &mut Option<TokenId>,
) {
    match msg {
        TokenInputMessage::TextChanged(text) => value.text = text,
        TokenInputMessage::RemoveToken(id) => {
            value.tokens.retain(|t| t.id != id);
            *selected = None;
        }
        TokenInputMessage::TokenizeText(text) => {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                let id = value.next_token_id();
                let label = trimmed.to_string();
                value.tokens.push(token_input::Token {
                    id,
                    email: label.clone(),
                    label,
                    is_group: false,
                    group_id: None,
                    member_count: None,
                });
            }
            value.text.clear();
        }
        TokenInputMessage::SelectToken(id) => *selected = Some(id),
        TokenInputMessage::DeselectTokens => *selected = None,
        TokenInputMessage::BackspaceAtStart => {
            if let Some(last) = value.tokens.last() {
                *selected = Some(last.id);
            }
        }
        TokenInputMessage::Focused | TokenInputMessage::Blurred => {}
        TokenInputMessage::TokenContextMenu(_, _) => {
            // Handled at the compose level via handle_recipient_token_input
        }
        TokenInputMessage::ArrowSelectToken(_) => {}
        TokenInputMessage::ArrowToText => {}
        // Autocomplete keyboard events are handled at the compose level,
        // not here — they should never reach this function.
        TokenInputMessage::AutocompleteDown
        | TokenInputMessage::AutocompleteUp
        | TokenInputMessage::AutocompleteAccept
        | TokenInputMessage::AutocompleteDismissKey
        | TokenInputMessage::DragStarted(_) => {}
        TokenInputMessage::Paste(content) => {
            // Use RFC 5322 parser for proper name + email extraction
            let parsed = crate::ui::token_input_parse::parse_pasted_addresses(&content);
            for addr in parsed {
                let label = addr.display_name.as_deref()
                    .filter(|n| !n.is_empty())
                    .unwrap_or(&addr.email)
                    .to_string();
                let id = value.next_token_id();
                value.tokens.push(token_input::Token {
                    id,
                    email: addr.email,
                    label,
                    is_group: false,
                    group_id: None,
                    member_count: None,
                });
            }
            value.text.clear();
        }
    }
}

// ── View ────────────────────────────────────────────────

pub fn view_compose_window<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let header = compose_header(window_id, state);
    let toolbar = formatting_toolbar(window_id);
    let body = compose_body(window_id, state);
    let footer = compose_footer(window_id, state);

    let mut content = column![
        header,
    ]
    .spacing(SPACE_0);

    // Bcc nudge banners
    for nudge in &state.bcc_nudges {
        content = content.push(bcc_nudge_banner(window_id, nudge));
    }

    // Bulk paste banner
    if let Some(ref banner) = state.bulk_paste_banner {
        content = content.push(bulk_paste_banner_view(window_id, banner));
    }

    content = content
        .push(widgets::divider())
        .push(toolbar)
        .push(widgets::divider())
        .push(body);

    // Attachment list (between body and footer)
    if !state.attachments.is_empty() {
        content = content.push(widgets::divider());
        content = content.push(attachment_list(window_id, state));
    }

    content = content.push(widgets::divider());
    content = content.push(footer);

    // Token context menu overlay
    if let Some(ref ctx) = state.context_menu {
        content = content.push(token_context_menu(window_id, ctx));
    }

    // Discard confirmation overlay
    if state.discard_confirm_open {
        content = content.push(discard_confirmation(window_id));
    }

    // Link insertion dialog overlay
    if state.link_dialog_open {
        content = content.push(link_dialog(window_id, state));
    }

    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::ContainerClass::Content.style())
        .into()
}

fn compose_header<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let mut fields = column![].spacing(SPACE_XS);

    // From row + Cc/Bcc toggle buttons
    let from_row = build_from_row(window_id, state);
    fields = fields.push(from_row);

    // To field
    fields = fields.push(build_to_row(window_id, state));

    // Cc field (if shown)
    if state.show_cc {
        fields = fields.push(build_cc_row(window_id, state));
    }

    // Bcc field (if shown)
    if state.show_bcc {
        fields = fields.push(build_bcc_row(window_id, state));
    }

    // Autocomplete dropdown (rendered below the active recipient field)
    if !state.autocomplete.query.is_empty()
        && !state.autocomplete.results.is_empty()
    {
        fields = fields.push(autocomplete_dropdown(window_id, state));
    }

    // Subject
    let subject_input = text_input("Subject", &state.subject)
        .on_input(move |s| {
            Message::PopOut(
                window_id,
                PopOutMessage::Compose(ComposeMessage::SubjectChanged(s)),
            )
        })
        .size(TEXT_LG)
        .padding(PAD_INPUT);

    let subject_row = row![
        container(
            text("Subject")
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style())
        )
        .width(COMPOSE_LABEL_WIDTH)
        .align_y(Alignment::Center),
        subject_input,
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);
    fields = fields.push(subject_row);

    // Status message
    if let Some(ref status) = state.status {
        fields = fields.push(
            text(status.as_str())
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        );
    }

    container(fields)
        .padding(PAD_CONTENT)
        .width(Length::Fill)
        .into()
}

fn build_from_row<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let from_picker = pick_list(
        state.from_account.clone(),
        state.from_accounts.clone(),
        |a: &AccountInfo| a.to_string(),
    )
    .on_select(move |account: AccountInfo| {
        Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::FromAccountChanged(account)),
        )
    })
    .text_size(TEXT_MD)
    .padding(PAD_INPUT)
    .width(Length::Fill)
    .style(theme::PickListClass::Ghost.style());

    let from_label = container(
        text("From")
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style()),
    )
    .width(COMPOSE_LABEL_WIDTH)
    .align_y(Alignment::Center);

    let mut from_row = row![from_label, from_picker]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    // Cc/Bcc toggle buttons
    if !state.show_cc {
        from_row = from_row.push(
            button(text("Cc").size(TEXT_SM))
                .style(theme::ButtonClass::Ghost.style())
                .on_press(Message::PopOut(
                    window_id,
                    PopOutMessage::Compose(ComposeMessage::ShowCc),
                ))
                .padding(PAD_INPUT),
        );
    }
    if !state.show_bcc {
        from_row = from_row.push(
            button(text("Bcc").size(TEXT_SM))
                .style(theme::ButtonClass::Ghost.style())
                .on_press(Message::PopOut(
                    window_id,
                    PopOutMessage::Compose(ComposeMessage::ShowBcc),
                ))
                .padding(PAD_INPUT),
        );
    }

    from_row.into()
}

fn build_to_row<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let ac_open = state.autocomplete.active_field == RecipientField::To
        && !state.autocomplete.results.is_empty();
    build_recipient_row_inner(
        "To",
        &state.to,
        state.selected_to_token,
        ac_open,
        window_id,
        "Add recipients...",
        ComposeMessage::ToTokenInput,
    )
}

fn build_cc_row<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let ac_open = state.autocomplete.active_field == RecipientField::Cc
        && !state.autocomplete.results.is_empty();
    build_recipient_row_inner(
        "Cc",
        &state.cc,
        state.selected_cc_token,
        ac_open,
        window_id,
        "Add Cc...",
        ComposeMessage::CcTokenInput,
    )
}

fn build_bcc_row<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let ac_open = state.autocomplete.active_field == RecipientField::Bcc
        && !state.autocomplete.results.is_empty();
    build_recipient_row_inner(
        "Bcc",
        &state.bcc,
        state.selected_bcc_token,
        ac_open,
        window_id,
        "Add Bcc...",
        ComposeMessage::BccTokenInput,
    )
}

fn build_recipient_row_inner<'a>(
    label: &'a str,
    value: &'a TokenInputValue,
    selected: Option<TokenId>,
    autocomplete_open: bool,
    window_id: iced::window::Id,
    placeholder: &'a str,
    wrap: fn(TokenInputMessage) -> ComposeMessage,
) -> Element<'a, Message> {
    let field = token_input::token_input_field(
        &value.tokens,
        &value.text,
        placeholder,
        selected,
        autocomplete_open,
        move |msg| Message::PopOut(window_id, PopOutMessage::Compose(wrap(msg))),
    );

    row![
        container(
            text(label)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style())
        )
        .width(COMPOSE_LABEL_WIDTH)
        .align_y(Alignment::Center),
        field,
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Start)
    .into()
}

// ── Formatting toolbar ─────────────────────────────────

fn formatting_toolbar<'a>(
    window_id: iced::window::Id,
) -> Element<'a, Message> {
    let fmt_btn = |ico: iced::widget::Text<'a>, msg: ComposeMessage| {
        button(ico.size(ICON_SM).style(text::secondary))
            .on_press(Message::PopOut(
                window_id,
                PopOutMessage::Compose(msg),
            ))
            .padding(PAD_ICON_BTN)
            .style(theme::ButtonClass::BareIcon.style())
    };

    let toolbar = row![
        fmt_btn(icon::bold(), ComposeMessage::FormatBold),
        fmt_btn(icon::italic(), ComposeMessage::FormatItalic),
        fmt_btn(icon::underline(), ComposeMessage::FormatUnderline),
        fmt_btn(icon::list(), ComposeMessage::FormatList),
        fmt_btn(icon::link(), ComposeMessage::FormatLink),
    ]
    .spacing(SPACE_XXS)
    .align_y(Alignment::Center);

    container(toolbar)
        .padding(PAD_TOOLBAR)
        .width(Length::Fill)
        .into()
}

// ── Body ────────────────────────────────────────────────

fn compose_body<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let editor = rich_text_editor(&state.body)
        .on_action(move |action| {
            Message::PopOut(
                window_id,
                PopOutMessage::Compose(ComposeMessage::BodyChanged(action)),
            )
        })
        .height(Length::Fill)
        .font(font::text());

    container(editor)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

// ── Footer ──────────────────────────────────────────────

fn compose_footer<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    // Discard button — shows confirmation if there's user content
    let discard_msg = if state.has_user_content() {
        ComposeMessage::ToggleDiscardConfirm
    } else {
        ComposeMessage::Discard
    };

    let discard_btn = button(
        row![
            icon::trash().size(ICON_SM),
            text("Discard").size(TEXT_MD),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center),
    )
    .style(theme::ButtonClass::Ghost.style())
    .on_press(Message::PopOut(
        window_id,
        PopOutMessage::Compose(discard_msg),
    ))
    .padding(PAD_BUTTON);

    let send_btn = button(
        row![
            icon::send().size(ICON_SM),
            text("Send").size(TEXT_MD).font(font::text_semibold()),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center),
    )
    .style(theme::ButtonClass::Primary.style())
    .on_press(Message::PopOut(
        window_id,
        PopOutMessage::Compose(ComposeMessage::Send),
    ))
    .padding(PAD_BUTTON);

    let attach_btn = button(
        row![
            icon::paperclip().size(ICON_SM),
            text("Attach").size(TEXT_MD),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center),
    )
    .style(theme::ButtonClass::Ghost.style())
    .on_press(Message::PopOut(
        window_id,
        PopOutMessage::Compose(ComposeMessage::AttachFiles),
    ))
    .padding(PAD_BUTTON);

    let footer_row = row![
        discard_btn,
        attach_btn,
        Space::new().width(Length::Fill),
        send_btn,
    ]
    .align_y(Alignment::Center);

    container(footer_row)
        .padding(PAD_CONTENT)
        .width(Length::Fill)
        .into()
}

// ── Bcc nudge banner ────────────────────────────────────

fn bcc_nudge_banner<'a>(
    window_id: iced::window::Id,
    nudge: &BccNudgeBanner,
) -> Element<'a, Message> {
    let tid = nudge.token_id;
    let label = format!(
        "\u{2139} \"{}\" is a group. Move to Bcc to hide member addresses?",
        nudge.group_name,
    );

    let move_btn = button(text("Move").size(TEXT_SM).font(font::text_semibold()))
        .on_press(Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::BccNudgeAccept(tid)),
        ))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Primary.style());

    let dismiss_btn = button(text("Dismiss").size(TEXT_SM))
        .on_press(Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::BccNudgeDismiss(tid)),
        ))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());

    container(
        row![
            text(label).size(TEXT_SM).width(Length::Fill),
            move_btn,
            dismiss_btn,
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .padding(PAD_CONTENT)
    .width(Length::Fill)
    .style(theme::ContainerClass::Elevated.style())
    .into()
}

// ── Bulk paste banner ───────────────────────────────────

fn bulk_paste_banner_view<'a>(
    window_id: iced::window::Id,
    banner: &BulkPasteBanner,
) -> Element<'a, Message> {
    let label = format!(
        "\u{2139} {} addresses pasted. Save as a contact group?",
        banner.count,
    );

    let dismiss_btn = button(text("Dismiss").size(TEXT_SM))
        .on_press(Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::BulkPasteDismiss),
        ))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());

    container(
        row![
            text(label).size(TEXT_SM).width(Length::Fill),
            dismiss_btn,
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .padding(PAD_CONTENT)
    .width(Length::Fill)
    .style(theme::ContainerClass::Elevated.style())
    .into()
}

// ── Token context menu ──────────────────────────────────

fn token_context_menu<'a>(
    window_id: iced::window::Id,
    ctx: &TokenContextMenuState,
) -> Element<'a, Message> {
    let mk = |label: &'a str, msg: ComposeMessage| {
        button(text(label).size(TEXT_SM))
            .on_press(Message::PopOut(
                window_id,
                PopOutMessage::Compose(msg),
            ))
            .width(Length::Fill)
            .padding(PAD_INPUT)
            .style(theme::ButtonClass::Ghost.style())
    };

    let field = ctx.field;
    let token_id = ctx.token_id;

    let mut items = column![].spacing(SPACE_0);

    items = items.push(mk(
        "Delete",
        ComposeMessage::ContextMenuDelete { field, token_id },
    ));

    // Expand group (only for group tokens)
    if ctx.is_group {
        items = items.push(mk(
            "Expand group",
            ComposeMessage::ContextMenuExpandGroup { field, token_id },
        ));
    }

    // Move to other fields
    if field != RecipientField::To {
        items = items.push(mk(
            "Move to To",
            ComposeMessage::ContextMenuMoveTo {
                token_id,
                from: field,
                to_field: RecipientField::To,
            },
        ));
    }
    if field != RecipientField::Cc {
        items = items.push(mk(
            "Move to Cc",
            ComposeMessage::ContextMenuMoveTo {
                token_id,
                from: field,
                to_field: RecipientField::Cc,
            },
        ));
    }
    if field != RecipientField::Bcc {
        items = items.push(mk(
            "Move to Bcc",
            ComposeMessage::ContextMenuMoveTo {
                token_id,
                from: field,
                to_field: RecipientField::Bcc,
            },
        ));
    }

    let menu = container(items)
        .padding(PAD_DROPDOWN)
        .style(theme::ContainerClass::Elevated.style())
        .width(180.0);

    // Wrap in a mouse_area to dismiss when clicking outside
    mouse_area(menu)
        .on_press(Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::DismissContextMenu),
        ))
        .into()
}

// ── Discard confirmation ────────────────────────────────

fn discard_confirmation<'a>(
    window_id: iced::window::Id,
) -> Element<'a, Message> {
    let confirm_btn = button(
        text("Discard")
            .size(TEXT_MD)
            .font(font::text_semibold()),
    )
    .style(theme::ButtonClass::Ghost.style())
    .on_press(Message::PopOut(
        window_id,
        PopOutMessage::Compose(ComposeMessage::Discard),
    ))
    .padding(PAD_BUTTON);

    let cancel_btn = button(
        text("Keep editing").size(TEXT_MD),
    )
    .style(theme::ButtonClass::Primary.style())
    .on_press(Message::PopOut(
        window_id,
        PopOutMessage::Compose(ComposeMessage::ToggleDiscardConfirm),
    ))
    .padding(PAD_BUTTON);

    container(
        column![
            text("Discard this draft?")
                .size(TEXT_TITLE)
                .font(font::text_semibold())
                .style(text::base),
            text("Your unsaved changes will be lost.")
                .size(TEXT_MD)
                .style(text::secondary),
            row![confirm_btn, cancel_btn]
                .spacing(SPACE_SM)
                .align_y(Alignment::Center),
        ]
        .spacing(SPACE_SM)
        .align_x(Alignment::Center),
    )
    .padding(PAD_CONTENT)
    .style(theme::ContainerClass::Elevated.style())
    .width(Length::Fill)
    .into()
}

// ── Attachment list ──────────────────────────────────────

fn attachment_list<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let mut items = column![].spacing(SPACE_XXS);

    for (idx, att) in state.attachments.iter().enumerate() {
        let size_label = att.display_size();
        let remove_btn = button(
            icon::x().size(ICON_XS).style(text::secondary),
        )
        .on_press(Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::RemoveAttachment(idx)),
        ))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::BareIcon.style());

        let att_row = row![
            icon::paperclip()
                .size(ICON_SM)
                .style(text::secondary),
            text(&att.name).size(TEXT_SM),
            text(size_label)
                .size(TEXT_XS)
                .style(theme::TextClass::Tertiary.style()),
            remove_btn,
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center);

        items = items.push(att_row);
    }

    container(items)
        .padding(PAD_CONTENT)
        .width(Length::Fill)
        .into()
}

// ── Autocomplete dropdown ───────────────────────────────

fn autocomplete_dropdown<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let mut items = column![].spacing(SPACE_0);

    for (idx, entry) in state.autocomplete.results.iter().enumerate() {
        let is_highlighted = state.autocomplete.highlighted == Some(idx);

        let display = if let Some(ref name) = entry.display_name {
            format!("{name} <{}>", entry.email)
        } else {
            entry.email.clone()
        };

        let row_style = if is_highlighted {
            theme::ButtonClass::Primary.style()
        } else {
            theme::ButtonClass::Ghost.style()
        };

        let row_btn = button(
            text(display)
                .size(TEXT_SM),
        )
        .on_press(Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::AutocompleteSelect(idx)),
        ))
        .width(Length::Fill)
        .padding(PAD_INPUT)
        .style(row_style);

        items = items.push(
            container(row_btn)
                .width(Length::Fill)
                .height(AUTOCOMPLETE_ROW_HEIGHT),
        );
    }

    let dropdown = scrollable(items)
        .height(Length::Shrink);

    // Offset by label width to align with the token input fields
    let offset_row = row![
        Space::new().width(COMPOSE_LABEL_WIDTH + SPACE_XS),
        container(dropdown)
            .max_height(AUTOCOMPLETE_MAX_HEIGHT)
            .width(Length::Fill)
            .style(theme::ContainerClass::Elevated.style()),
    ];

    // Wrap in a mouse_area to dismiss when clicking outside
    mouse_area(offset_row)
        .on_press(Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::AutocompleteDismiss),
        ))
        .into()
}

// ── Link insertion dialog ───────────────────────────────

fn link_dialog<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let url_input = text_input("https://...", &state.link_url)
        .on_input(move |s| {
            Message::PopOut(
                window_id,
                PopOutMessage::Compose(ComposeMessage::LinkUrlChanged(s)),
            )
        })
        .size(TEXT_MD)
        .padding(PAD_INPUT);

    let text_input_field =
        text_input("Display text (optional)", &state.link_text)
            .on_input(move |s| {
                Message::PopOut(
                    window_id,
                    PopOutMessage::Compose(
                        ComposeMessage::LinkTextChanged(s),
                    ),
                )
            })
            .size(TEXT_MD)
            .padding(PAD_INPUT);

    let cancel_btn = button(text("Cancel").size(TEXT_MD))
        .style(theme::ButtonClass::Ghost.style())
        .on_press(Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::ToggleLinkDialog),
        ))
        .padding(PAD_BUTTON);

    let insert_btn = button(
        text("Insert")
            .size(TEXT_MD)
            .font(font::text_semibold()),
    )
    .style(theme::ButtonClass::Primary.style())
    .on_press(Message::PopOut(
        window_id,
        PopOutMessage::Compose(ComposeMessage::LinkInsert),
    ))
    .padding(PAD_BUTTON);

    container(
        column![
            text("Insert Link")
                .size(TEXT_TITLE)
                .font(font::text_semibold())
                .style(text::base),
            column![
                text("URL").size(TEXT_SM).style(text::secondary),
                url_input,
            ]
            .spacing(SPACE_XXS),
            column![
                text("Display text")
                    .size(TEXT_SM)
                    .style(text::secondary),
                text_input_field,
            ]
            .spacing(SPACE_XXS),
            row![cancel_btn, insert_btn]
                .spacing(SPACE_SM)
                .align_y(Alignment::Center),
        ]
        .spacing(SPACE_SM),
    )
    .padding(PAD_CONTENT)
    .style(theme::ContainerClass::Elevated.style())
    .width(Length::Fill)
    .into()
}

// ── Helpers ─────────────────────────────────────────────

fn accounts_to_info(accounts: &[db::Account]) -> Vec<AccountInfo> {
    accounts
        .iter()
        .map(|a| AccountInfo {
            id: a.id.clone(),
            email: a.email.clone(),
            display_name: a.display_name.clone(),
            account_name: a.account_name.clone(),
        })
        .collect()
}

/// Build an attribution line for quoted content, e.g.
/// "On Mar 19, Alice Smith <alice@corp.com> wrote:"
fn build_attribution(name: Option<&str>, email: Option<&str>) -> String {
    let sender = match (name, email) {
        (Some(n), Some(e)) if !n.is_empty() => format!("{n} <{e}>"),
        (_, Some(e)) => e.to_string(),
        (Some(n), None) if !n.is_empty() => n.to_string(),
        _ => "someone".to_string(),
    };
    // We omit the date here since we don't have it in the compose context.
    // The full implementation would include the original message date.
    format!("{sender} wrote:")
}
