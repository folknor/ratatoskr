use iced::widget::{button, column, container, pick_list, row, text, text_input, Space};
use iced::{Alignment, Element, Length};

use crate::db::{self, ContactMatch};
use crate::font;
use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::token_input::{self, RecipientField, TokenId, TokenInputMessage, TokenInputValue};
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

// ── Messages ────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ComposeMessage {
    SubjectChanged(String),
    BodyChanged(iced::widget::text_editor::Action),
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
    /// Move the currently selected token to a different recipient field.
    MoveSelectedTokenToField(RecipientField),
    /// Formatting toolbar actions (stubs for V1).
    FormatBold,
    FormatItalic,
    FormatUnderline,
    FormatStrikethrough,
    FormatList,
    FormatBlockquote,
    FormatLink,
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

    // Body (plain text for V1 — rich text editor in a future iteration)
    pub body: iced::widget::text_editor::Content,

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
            body: iced::widget::text_editor::Content::new(),
            mode: ComposeMode::New,
            reply_thread_id: None,
            reply_message_id: None,
            status: None,
            discard_confirm_open: false,
            autocomplete: AutocompleteState::new(),
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

        // Set quoted body with attribution line
        if let Some(body) = quoted_body {
            let attribution = build_attribution(to_name, to_email);
            let quoted = body
                .lines()
                .map(|line| format!("> {line}"))
                .collect::<Vec<_>>()
                .join("\n");
            let full_body = format!("\n\n{attribution}\n{quoted}");
            state.body =
                iced::widget::text_editor::Content::with_text(&full_body);
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

    /// Returns which field has a selected token, if any.
    pub fn selected_token_field(&self) -> Option<RecipientField> {
        if self.selected_to_token.is_some() {
            Some(RecipientField::To)
        } else if self.selected_cc_token.is_some() {
            Some(RecipientField::Cc)
        } else if self.selected_bcc_token.is_some() {
            Some(RecipientField::Bcc)
        } else {
            None
        }
    }

    /// Returns true if the compose body has user content beyond the
    /// initial quoted text / signature.
    fn has_user_content(&self) -> bool {
        // Simple heuristic: non-empty body text
        let body_text = self.body.text();
        let trimmed = body_text.trim();
        !trimmed.is_empty() && !trimmed.starts_with('>')
    }
}

// ── Update ──────────────────────────────────────────────

pub fn update_compose(state: &mut ComposeState, msg: ComposeMessage) {
    match msg {
        ComposeMessage::SubjectChanged(s) => state.subject = s,
        ComposeMessage::BodyChanged(action) => state.body.perform(action),
        ComposeMessage::FromAccountChanged(account) => {
            state.from_account = Some(account);
        }
        ComposeMessage::ShowCc => state.show_cc = true,
        ComposeMessage::ShowBcc => state.show_bcc = true,
        ComposeMessage::ToTokenInput(msg) => {
            handle_token_input_message(
                &mut state.to,
                msg,
                &mut state.selected_to_token,
            );
        }
        ComposeMessage::CcTokenInput(msg) => {
            handle_token_input_message(
                &mut state.cc,
                msg,
                &mut state.selected_cc_token,
            );
        }
        ComposeMessage::BccTokenInput(msg) => {
            handle_token_input_message(
                &mut state.bcc,
                msg,
                &mut state.selected_bcc_token,
            );
        }
        ComposeMessage::MoveSelectedTokenToField(target_field) => {
            move_selected_token(state, target_field);
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
        // Formatting toolbar stubs
        ComposeMessage::FormatBold
        | ComposeMessage::FormatItalic
        | ComposeMessage::FormatUnderline
        | ComposeMessage::FormatStrikethrough
        | ComposeMessage::FormatList
        | ComposeMessage::FormatBlockquote
        | ComposeMessage::FormatLink => {
            // V1 stub — rich text editor not yet wired
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

/// Move the currently selected token from its source field to the target field.
fn move_selected_token(state: &mut ComposeState, target: RecipientField) {
    // Find which field has a selected token and extract it.
    let (token, source) = if let Some(token_id) = state.selected_to_token {
        let pos = state.to.tokens.iter().position(|t| t.id == token_id);
        if let Some(idx) = pos {
            (state.to.tokens.remove(idx), RecipientField::To)
        } else {
            return;
        }
    } else if let Some(token_id) = state.selected_cc_token {
        let pos = state.cc.tokens.iter().position(|t| t.id == token_id);
        if let Some(idx) = pos {
            (state.cc.tokens.remove(idx), RecipientField::Cc)
        } else {
            return;
        }
    } else if let Some(token_id) = state.selected_bcc_token {
        let pos = state.bcc.tokens.iter().position(|t| t.id == token_id);
        if let Some(idx) = pos {
            (state.bcc.tokens.remove(idx), RecipientField::Bcc)
        } else {
            return;
        }
    } else {
        return;
    };

    // Don't move to the same field.
    if source == target {
        return;
    }

    // Clear selection on source.
    match source {
        RecipientField::To => state.selected_to_token = None,
        RecipientField::Cc => state.selected_cc_token = None,
        RecipientField::Bcc => state.selected_bcc_token = None,
    }

    // Re-ID the token for the target field and insert.
    let target_value = match target {
        RecipientField::To => &mut state.to,
        RecipientField::Cc => &mut state.cc,
        RecipientField::Bcc => &mut state.bcc,
    };
    let new_id = target_value.next_token_id();
    target_value.tokens.push(token_input::Token {
        id: new_id,
        email: token.email,
        label: token.label,
        is_group: token.is_group,
        group_id: token.group_id,
        member_count: token.member_count,
    });

    // Auto-show Cc/Bcc if moving a token there.
    match target {
        RecipientField::Cc => state.show_cc = true,
        RecipientField::Bcc => state.show_bcc = true,
        RecipientField::To => {}
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
        widgets::divider(),
        toolbar,
        widgets::divider(),
        body,
        widgets::divider(),
        footer
    ]
    .spacing(SPACE_0);

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

    // Show Cc/Bcc fields when visible, or when a token is selected in
    // another field (as drop targets for cross-field token movement).
    let has_selection = state.selected_token_field().is_some();
    if state.show_cc || has_selection {
        fields = fields.push(build_cc_row(window_id, state));
    }
    if state.show_bcc || has_selection {
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
        RecipientField::To,
        &state.to,
        state.selected_to_token,
        state.selected_token_field(),
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
        RecipientField::Cc,
        &state.cc,
        state.selected_cc_token,
        state.selected_token_field(),
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
        RecipientField::Bcc,
        &state.bcc,
        state.selected_bcc_token,
        state.selected_token_field(),
        window_id,
        "Add Bcc...",
        ComposeMessage::BccTokenInput,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_recipient_row_inner<'a>(
    label: &'a str,
    this_field: RecipientField,
    value: &'a TokenInputValue,
    selected: Option<TokenId>,
    drag_source: Option<RecipientField>,
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

    // When a token is selected in a *different* field, show the label as a
    // drop-zone button so the user can click to move the token here.
    let is_drop_target = drag_source.is_some_and(|src| src != this_field);

    let label_element: Element<'a, Message> = if is_drop_target {
        button(
            row![
                icon::arrow_right().size(ICON_XS).style(text::primary),
                text(label)
                    .size(TEXT_SM)
                    .font(crate::font::text_semibold())
                    .style(text::primary),
            ]
            .spacing(SPACE_XXXS)
            .align_y(Alignment::Center),
        )
        .on_press(Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::MoveSelectedTokenToField(
                this_field,
            )),
        ))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style())
        .width(COMPOSE_LABEL_WIDTH)
        .into()
    } else {
        container(
            text(label)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        )
        .width(COMPOSE_LABEL_WIDTH)
        .align_y(Alignment::Center)
        .into()
    };

    row![label_element, field]
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
    let editor = iced::widget::text_editor(&state.body)
        .on_action(move |action| {
            Message::PopOut(
                window_id,
                PopOutMessage::Compose(ComposeMessage::BodyChanged(action)),
            )
        })
        .height(Length::Fill)
        .padding(SPACE_XS)
        .font(font::text())
        .size(TEXT_LG);

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

    let footer_row =
        row![discard_btn, Space::new().width(Length::Fill), send_btn]
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
