use iced::widget::{button, column, container, pick_list, row, scrollable, text, text_input, Space};
use iced::{Alignment, Element, Length};

use ratatoskr_rich_text_editor::widget::{Action as EditorAction, EditorState};
use ratatoskr_rich_text_editor::{EditAction, InlineStyle};

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

    /// Whether this mode represents a reply or forward (vs. new compose).
    pub fn is_reply(&self) -> bool {
        !matches!(self, Self::New)
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

/// A tracked attachment in the compose window.
#[derive(Debug, Clone)]
pub struct ComposeAttachment {
    /// Display name of the file.
    pub name: String,
    /// File path on disk.
    pub path: std::path::PathBuf,
    /// Size in bytes.
    pub size: u64,
}

// ── Messages ────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ComposeMessage {
    SubjectChanged(String),
    EditorAction(EditorAction),
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
    AutocompleteResults(u64, Result<Vec<ContactMatch>, String>),
    /// User selected an autocomplete suggestion.
    AutocompleteSelect(usize),
    /// User navigated the autocomplete list (up/down).
    AutocompleteNavigate(i32),
    /// Dismiss the autocomplete dropdown.
    AutocompleteDismiss,
    /// Formatting toolbar actions.
    FormatBold,
    FormatItalic,
    FormatUnderline,
    FormatStrikethrough,
    FormatList,
    FormatBlockquote,
    FormatLink,
    /// Signature resolved from DB after compose window opens.
    SignatureResolved(Option<(String, String, Option<String>)>),
    /// Attach files button pressed.
    AttachFiles,
    /// Files selected by the user (path list).
    FilesAttached(Vec<(String, std::path::PathBuf, u64)>),
    /// Remove an attachment by index.
    RemoveAttachment(usize),
    /// Draft auto-save completed.
    DraftSaved(Result<(), String>),
    /// Send finalization completed.
    SendFinalized(Result<(), String>),
}

// ── Autocomplete state ──────────────────────────────────

/// State for the recipient autocomplete dropdown.
pub struct AutocompleteState {
    /// Current query being searched.
    pub query: String,
    /// Results from the most recent autocomplete search.
    pub results: Vec<ContactMatch>,
    /// Currently highlighted index in the results list.
    pub highlighted: Option<usize>,
    /// Generation counter to discard stale results.
    pub search_generation: u64,
}

impl AutocompleteState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            results: Vec::new(),
            highlighted: None,
            search_generation: 0,
        }
    }
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

    // Rich text editor state
    pub editor: EditorState,

    // Compose mode
    pub mode: ComposeMode,

    // Reply context
    pub reply_thread_id: Option<String>,
    pub reply_message_id: Option<String>,

    // Status message (e.g. "Sending..." or error)
    pub status: Option<String>,

    // Discard confirmation
    pub discard_confirm_open: bool,

    // Autocomplete
    pub autocomplete: AutocompleteState,

    // Signature tracking
    pub signature_separator_index: Option<usize>,
    pub active_signature_id: Option<String>,

    // Attachments
    pub attachments: Vec<ComposeAttachment>,

    // Draft persistence
    pub draft_id: String,
    pub draft_dirty: bool,

    // Window geometry
    pub width: f32,
    pub height: f32,
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
            editor: EditorState::new(),
            mode: ComposeMode::New,
            reply_thread_id: None,
            reply_message_id: None,
            status: None,
            discard_confirm_open: false,
            autocomplete: AutocompleteState::new(),
            signature_separator_index: None,
            active_signature_id: None,
            attachments: Vec::new(),
            draft_id: uuid::Uuid::new_v4().to_string(),
            draft_dirty: false,
            width: COMPOSE_DEFAULT_WIDTH,
            height: COMPOSE_DEFAULT_HEIGHT,
        }
    }

    pub fn new_reply(
        accounts: &[db::Account],
        mode: ComposeMode,
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

        // Set quoted body — the initial document is just an empty paragraph.
        // Signature will be assembled later via SignatureResolved.
        // For now, set up a basic quoted document if there is quoted text.
        if let Some(body) = quoted_body {
            let attribution = build_attribution(to_name, to_email);
            let quoted_content = ratatoskr_rich_text_editor::compose::QuotedContent {
                attribution,
                body_html: format!("<p>{}</p>", html_escape(body)),
            };
            let assembly = ratatoskr_rich_text_editor::compose::assemble_compose_document(
                None, None,
                Some(quoted_content),
            );
            state.editor = EditorState::from_document(assembly.document);
            state.signature_separator_index = assembly.signature_separator_index;
            state.active_signature_id = assembly.active_signature_id;
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

    /// Returns true if the compose body has user content beyond the
    /// initial quoted text / signature.
    pub fn has_user_content(&self) -> bool {
        // Check if there is non-empty text in any block before the signature
        let sig_end = self.signature_separator_index.unwrap_or(self.editor.document.block_count());
        for i in 0..sig_end {
            if let Some(block) = self.editor.document.block(i) {
                let text = block.flattened_text();
                if !text.trim().is_empty() {
                    return true;
                }
            }
        }
        // Also check if there are recipients or subject
        !self.to.tokens.is_empty()
            || !self.cc.tokens.is_empty()
            || !self.subject.is_empty()
            || !self.attachments.is_empty()
    }

    /// Total attachment size in bytes.
    pub fn total_attachment_size(&self) -> u64 {
        self.attachments.iter().map(|a| a.size).sum()
    }
}

// ── Update ──────────────────────────────────────────────

pub fn update_compose(state: &mut ComposeState, msg: ComposeMessage) {
    match msg {
        ComposeMessage::SubjectChanged(s) => {
            state.subject = s;
            state.draft_dirty = true;
        }
        ComposeMessage::EditorAction(action) => {
            // Check if this is a content-modifying action (for draft dirty tracking)
            let is_content_action = matches!(
                action,
                EditorAction::Edit(_)
                | EditorAction::Paste(_)
                | EditorAction::Cut
                | EditorAction::Undo
                | EditorAction::Redo
            );
            state.editor.perform(action);
            if is_content_action {
                state.draft_dirty = true;
            }
        }
        ComposeMessage::FromAccountChanged(account) => {
            state.from_account = Some(account);
            state.draft_dirty = true;
        }
        ComposeMessage::ShowCc => state.show_cc = true,
        ComposeMessage::ShowBcc => state.show_bcc = true,
        ComposeMessage::ToTokenInput(msg) => {
            handle_token_input_message(
                &mut state.to,
                msg,
                &mut state.selected_to_token,
            );
            state.draft_dirty = true;
        }
        ComposeMessage::CcTokenInput(msg) => {
            handle_token_input_message(
                &mut state.cc,
                msg,
                &mut state.selected_cc_token,
            );
            state.draft_dirty = true;
        }
        ComposeMessage::BccTokenInput(msg) => {
            handle_token_input_message(
                &mut state.bcc,
                msg,
                &mut state.selected_bcc_token,
            );
            state.draft_dirty = true;
        }
        ComposeMessage::Send => {
            // Validation happens in handler — this is a stub for direct update calls
            let has_recipients = !state.to.tokens.is_empty()
                || !state.cc.tokens.is_empty()
                || !state.bcc.tokens.is_empty();
            if !has_recipients {
                state.status =
                    Some("Add at least one recipient".to_string());
                return;
            }
            state.status = Some("Preparing message...".to_string());
        }
        ComposeMessage::Discard => {
            // Handled by the caller (close window)
        }
        ComposeMessage::ToggleDiscardConfirm => {
            state.discard_confirm_open = !state.discard_confirm_open;
        }
        // Autocomplete
        ComposeMessage::AutocompleteResults(generation, Ok(results)) => {
            if generation == state.autocomplete.search_generation {
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
                let id = state.to.next_token_id();
                state.to.tokens.push(token_input::Token {
                    id,
                    email: match_entry.email,
                    label,
                    is_group: match_entry.is_group,
                    group_id: match_entry.group_id,
                    member_count: match_entry.member_count,
                });
                state.to.text.clear();
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
        // Formatting toolbar — dispatch through editor
        ComposeMessage::FormatBold => {
            state.editor.perform(EditorAction::Edit(
                EditAction::ToggleInlineStyle(InlineStyle::BOLD),
            ));
            state.draft_dirty = true;
        }
        ComposeMessage::FormatItalic => {
            state.editor.perform(EditorAction::Edit(
                EditAction::ToggleInlineStyle(InlineStyle::ITALIC),
            ));
            state.draft_dirty = true;
        }
        ComposeMessage::FormatUnderline => {
            state.editor.perform(EditorAction::Edit(
                EditAction::ToggleInlineStyle(InlineStyle::UNDERLINE),
            ));
            state.draft_dirty = true;
        }
        ComposeMessage::FormatStrikethrough => {
            state.editor.perform(EditorAction::Edit(
                EditAction::ToggleInlineStyle(InlineStyle::STRIKETHROUGH),
            ));
            state.draft_dirty = true;
        }
        ComposeMessage::FormatList | ComposeMessage::FormatBlockquote => {
            // Block-type toggles are not yet exposed via EditAction.
            // Stub for now — the editor supports these block types but
            // the rules engine SetBlockType path needs a specific action.
        }
        ComposeMessage::FormatLink => {
            // Link insertion needs a URL dialog — stub for now.
        }
        // Signature resolved from DB
        ComposeMessage::SignatureResolved(Some((sig_html, sig_id, quoted_body_html))) => {
            apply_signature(state, &sig_html, Some(sig_id), quoted_body_html.as_deref());
        }
        ComposeMessage::SignatureResolved(None) => {
            // No signature found — leave editor as-is
        }
        // Attachments
        ComposeMessage::AttachFiles => {
            // Handled by the handler which opens a file dialog
        }
        ComposeMessage::FilesAttached(files) => {
            for (name, path, size) in files {
                state.attachments.push(ComposeAttachment { name, path, size });
            }
            state.draft_dirty = true;
        }
        ComposeMessage::RemoveAttachment(idx) => {
            if idx < state.attachments.len() {
                state.attachments.remove(idx);
                state.draft_dirty = true;
            }
        }
        // Draft save result
        ComposeMessage::DraftSaved(Ok(())) => {
            state.draft_dirty = false;
        }
        ComposeMessage::DraftSaved(Err(e)) => {
            eprintln!("Draft save failed: {e}");
        }
        // Send finalization result
        ComposeMessage::SendFinalized(Ok(())) => {
            state.status = Some("Message saved as draft. Send not yet wired to provider.".to_string());
        }
        ComposeMessage::SendFinalized(Err(e)) => {
            state.status = Some(format!("Send failed: {e}"));
        }
    }
}

/// Apply a resolved signature to the compose state's editor document.
fn apply_signature(
    state: &mut ComposeState,
    sig_html: &str,
    sig_id: Option<String>,
    quoted_body_html: Option<&str>,
) {
    let quoted_content = quoted_body_html.map(|html| {
        let attribution = build_attribution_from_state(state);
        ratatoskr_rich_text_editor::compose::QuotedContent {
            attribution,
            body_html: html.to_string(),
        }
    });

    let assembly = ratatoskr_rich_text_editor::compose::assemble_compose_document(
        Some(sig_html),
        sig_id,
        quoted_content,
    );

    state.editor = EditorState::from_document(assembly.document);
    state.signature_separator_index = assembly.signature_separator_index;
    state.active_signature_id = assembly.active_signature_id;
}

fn build_attribution_from_state(state: &ComposeState) -> String {
    let to_name = state.to.tokens.first().map(|t| t.label.as_str());
    let to_email = state.to.tokens.first().map(|t| t.email.as_str());
    build_attribution(to_name, to_email)
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
        TokenInputMessage::TokenContextMenu(_, _) => {}
        TokenInputMessage::ArrowSelectToken(_) => {}
        TokenInputMessage::ArrowToText => {}
        TokenInputMessage::Paste(content) => {
            // Split pasted text by commas/semicolons and tokenize
            for part in
                content.split([',', ';', '\n'])
            {
                let trimmed = part.trim();
                if !trimmed.is_empty() {
                    let id = value.next_token_id();
                    value.tokens.push(token_input::Token {
                        id,
                        email: trimmed.to_string(),
                        label: trimmed.to_string(),
                        is_group: false,
                        group_id: None,
                        member_count: None,
                    });
                }
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
    let attachment_section = compose_attachments(window_id, state);
    let footer = compose_footer(window_id, state);

    let mut content = column![
        header,
        widgets::divider(),
        toolbar,
        widgets::divider(),
        body,
    ]
    .spacing(SPACE_0);

    // Attachment section (if any)
    if !state.attachments.is_empty() {
        content = content.push(widgets::divider());
        content = content.push(attachment_section);
    }

    content = content.push(widgets::divider());
    content = content.push(footer);

    // Discard confirmation overlay
    if state.discard_confirm_open {
        content = content.push(discard_confirmation(window_id));
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
    build_recipient_row_inner(
        "To",
        &state.to,
        state.selected_to_token,
        window_id,
        "Add recipients...",
        ComposeMessage::ToTokenInput,
    )
}

fn build_cc_row<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    build_recipient_row_inner(
        "Cc",
        &state.cc,
        state.selected_cc_token,
        window_id,
        "Add Cc...",
        ComposeMessage::CcTokenInput,
    )
}

fn build_bcc_row<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    build_recipient_row_inner(
        "Bcc",
        &state.bcc,
        state.selected_bcc_token,
        window_id,
        "Add Bcc...",
        ComposeMessage::BccTokenInput,
    )
}

fn build_recipient_row_inner<'a>(
    label: &'a str,
    value: &'a TokenInputValue,
    selected: Option<TokenId>,
    window_id: iced::window::Id,
    placeholder: &'a str,
    wrap: fn(TokenInputMessage) -> ComposeMessage,
) -> Element<'a, Message> {
    let field = token_input::token_input_field(
        &value.tokens,
        &value.text,
        placeholder,
        selected,
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
    let editor = ratatoskr_rich_text_editor::rich_text_editor(&state.editor)
        .on_action(move |action| {
            Message::PopOut(
                window_id,
                PopOutMessage::Compose(ComposeMessage::EditorAction(action)),
            )
        })
        .height(Length::Fill)
        .padding(SPACE_XS)
        .font(font::text());

    container(editor)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

// ── Attachments ─────────────────────────────────────────

fn compose_attachments<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let mut attachment_list = column![].spacing(SPACE_XXS);

    for (idx, att) in state.attachments.iter().enumerate() {
        let size_str = format_file_size(att.size);
        let att_row = row![
            icon::paperclip().size(ICON_SM).style(text::secondary),
            text(&att.name).size(TEXT_SM),
            text(size_str).size(TEXT_XS).style(theme::TextClass::Tertiary.style()),
            Space::new().width(Length::Fill),
            button(icon::x().size(ICON_XS).style(text::secondary))
                .on_press(Message::PopOut(
                    window_id,
                    PopOutMessage::Compose(ComposeMessage::RemoveAttachment(idx)),
                ))
                .padding(PAD_ICON_BTN)
                .style(theme::ButtonClass::BareIcon.style()),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center);
        attachment_list = attachment_list.push(att_row);
    }

    // Total size
    let total = format_file_size(state.total_attachment_size());
    attachment_list = attachment_list.push(
        text(format!("Total: {total}"))
            .size(TEXT_XS)
            .style(theme::TextClass::Tertiary.style()),
    );

    container(
        scrollable(attachment_list)
            .height(Length::Shrink),
    )
    .padding(PAD_CONTENT)
    .width(Length::Fill)
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

    // Attach button
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

    let footer_row = row![
        discard_btn,
        attach_btn,
        Space::new().width(Length::Fill),
        send_btn,
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    container(footer_row)
        .padding(PAD_CONTENT)
        .width(Length::Fill)
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

/// Simple HTML entity escaping for plain text being wrapped in <p> tags.
fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Format a file size in human-readable form.
fn format_file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Collect recipient emails into a comma-separated string for draft storage.
pub fn tokens_to_csv(value: &TokenInputValue) -> Option<String> {
    if value.tokens.is_empty() {
        None
    } else {
        Some(
            value
                .tokens
                .iter()
                .map(|t| t.email.as_str())
                .collect::<Vec<_>>()
                .join(","),
        )
    }
}
