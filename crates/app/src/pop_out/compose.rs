use iced::widget::{button, column, container, mouse_area, pick_list, row, text, text_input, Space};
use iced::{Alignment, Element, Length, Point};

use crate::db::{self, ContactMatch};
use crate::font;
use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::token_input::{self, RecipientField, Token, TokenId, TokenInputMessage, TokenInputValue};
use crate::ui::token_input_parse::{dedup_parsed, parse_pasted_addresses};
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

// ── Autocomplete state ──────────────────────────────────

/// Autocomplete state for the compose window.
pub struct AutocompleteState {
    /// Which field is currently showing autocomplete.
    pub active_field: Option<RecipientField>,
    /// Current search query (mirrors the focused field's text).
    pub query: String,
    /// Search results from the last query.
    pub results: Vec<ContactMatch>,
    /// Index of the highlighted result (keyboard navigation).
    pub highlighted: Option<usize>,
    /// Generation counter to discard stale search results.
    pub search_generation: u64,
}

impl Default for AutocompleteState {
    fn default() -> Self {
        Self {
            active_field: None,
            query: String::new(),
            results: Vec::new(),
            highlighted: None,
            search_generation: 0,
        }
    }
}

/// Banner shown after a bulk paste (10+ addresses).
pub struct BulkPasteBanner {
    /// Whether the banner is visible.
    pub visible: bool,
    /// Number of addresses pasted.
    pub count: usize,
}

/// Context menu state for right-clicked tokens.
pub struct TokenContextMenuState {
    /// Which token was right-clicked.
    pub token_id: TokenId,
    /// Which field the token belongs to.
    pub field: RecipientField,
    /// Screen position for the menu.
    pub position: Point,
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
    /// Autocomplete search results arrived (generation, results).
    AutocompleteResults(u64, Result<Vec<ContactMatch>, String>),
    /// User selected an autocomplete result by index.
    AutocompleteSelect(usize),
    /// User navigated the autocomplete dropdown (up/down).
    AutocompleteNavigate(i32),
    /// Dismiss the autocomplete dropdown.
    AutocompleteDismiss,
    /// Dismiss the bulk paste banner.
    DismissBulkPasteBanner,
    /// Context menu: delete token.
    ContextMenuDelete(RecipientField, TokenId),
    /// Context menu dismissed.
    ContextMenuDismiss,
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

    // Autocomplete
    pub autocomplete: AutocompleteState,

    // Bulk paste banner
    pub bulk_paste_banner: Option<BulkPasteBanner>,

    // Context menu
    pub context_menu: Option<TokenContextMenuState>,

    // From account
    pub from_account: Option<AccountInfo>,
    pub from_accounts: Vec<AccountInfo>,

    // Subject
    pub subject: String,

    // Body (plain text for V1)
    pub body: iced::widget::text_editor::Content,

    // Compose mode
    pub mode: ComposeMode,

    // Reply context
    pub reply_thread_id: Option<String>,
    pub reply_message_id: Option<String>,

    // Status message (e.g. "Send not yet wired")
    pub status: Option<String>,

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
            autocomplete: AutocompleteState::default(),
            bulk_paste_banner: None,
            context_menu: None,
            from_account,
            from_accounts,
            subject: String::new(),
            body: iced::widget::text_editor::Content::new(),
            mode: ComposeMode::New,
            reply_thread_id: None,
            reply_message_id: None,
            status: None,
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

        // Add To recipient (not for Forward)
        if !matches!(state.mode, ComposeMode::Forward { .. }) {
            if let Some(email) = to_email {
                let label = to_name
                    .filter(|n| !n.is_empty())
                    .unwrap_or(email);
                let id = state.to.next_token_id();
                state.to.tokens.push(Token {
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
                for addr in cc_str
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    let id = state.cc.next_token_id();
                    state.cc.tokens.push(Token {
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

        // Set quoted body
        if let Some(body) = quoted_body {
            let quoted = body
                .lines()
                .map(|line| format!("> {line}"))
                .collect::<Vec<_>>()
                .join("\n");
            let full_body = format!("\n\n{quoted}");
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

    /// Whether the autocomplete dropdown should be visible.
    pub fn autocomplete_visible(&self) -> bool {
        self.autocomplete.active_field.is_some()
            && !self.autocomplete.query.is_empty()
            && !self.autocomplete.results.is_empty()
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
            let field = RecipientField::To;
            handle_token_input_msg(state, field, msg);
        }
        ComposeMessage::CcTokenInput(msg) => {
            let field = RecipientField::Cc;
            handle_token_input_msg(state, field, msg);
        }
        ComposeMessage::BccTokenInput(msg) => {
            let field = RecipientField::Bcc;
            handle_token_input_msg(state, field, msg);
        }
        ComposeMessage::Send => handle_send(state),
        ComposeMessage::Discard => {
            // Handled by the caller (close window)
        }
        ComposeMessage::AutocompleteResults(gen, results) => {
            handle_autocomplete_results(state, gen, results);
        }
        ComposeMessage::AutocompleteSelect(idx) => {
            handle_autocomplete_select(state, idx);
        }
        ComposeMessage::AutocompleteNavigate(delta) => {
            handle_autocomplete_navigate(state, delta);
        }
        ComposeMessage::AutocompleteDismiss => {
            state.autocomplete.results.clear();
            state.autocomplete.highlighted = None;
        }
        ComposeMessage::DismissBulkPasteBanner => {
            state.bulk_paste_banner = None;
        }
        ComposeMessage::ContextMenuDelete(field, token_id) => {
            let value = field_value_mut(state, field);
            value.tokens.retain(|t| t.id != token_id);
            state.context_menu = None;
        }
        ComposeMessage::ContextMenuDismiss => {
            state.context_menu = None;
        }
    }
}

fn handle_send(state: &mut ComposeState) {
    let has_recipients = !state.to.tokens.is_empty()
        || !state.cc.tokens.is_empty()
        || !state.bcc.tokens.is_empty();
    if !has_recipients {
        state.status = Some("Add at least one recipient".to_string());
        return;
    }
    state.status = Some("Send not yet wired".to_string());
}

fn handle_token_input_msg(
    state: &mut ComposeState,
    field: RecipientField,
    msg: TokenInputMessage,
) {
    let value = field_value_mut(state, field);
    let selected = field_selected_mut(state, field);

    match msg {
        TokenInputMessage::TextChanged(ref text) => {
            value.text = text.clone();
            // Update autocomplete query
            state.autocomplete.active_field = Some(field);
            state.autocomplete.query = text.clone();
            // Results will be populated asynchronously via
            // AutocompleteResults after the App dispatches the search.
        }
        TokenInputMessage::RemoveToken(id) => {
            value.tokens.retain(|t| t.id != id);
            *selected = None;
        }
        TokenInputMessage::TokenizeText(ref text) => {
            tokenize_raw_text(value, text);
        }
        TokenInputMessage::SelectToken(id) => *selected = Some(id),
        TokenInputMessage::DeselectTokens => *selected = None,
        TokenInputMessage::BackspaceAtStart => {
            if let Some(last) = value.tokens.last() {
                *selected = Some(last.id);
            }
        }
        TokenInputMessage::Focused => {
            state.autocomplete.active_field = Some(field);
        }
        TokenInputMessage::Blurred => {
            // Clear autocomplete when field loses focus
            state.autocomplete.active_field = None;
            state.autocomplete.results.clear();
            state.autocomplete.highlighted = None;
        }
        TokenInputMessage::Paste(ref content) => {
            handle_paste(state, field, content);
        }
        TokenInputMessage::TokenContextMenu(token_id, position) => {
            state.context_menu = Some(TokenContextMenuState {
                token_id,
                field,
                position,
            });
        }
        TokenInputMessage::ArrowSelectToken(id) => {
            *selected = Some(id);
        }
        TokenInputMessage::ArrowToText => {
            *selected = None;
        }
    }
}

/// Tokenize raw text into a token, with basic email validation.
fn tokenize_raw_text(value: &mut TokenInputValue, text: &str) {
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        let id = value.next_token_id();
        let email = trimmed.to_lowercase();
        let label = trimmed.to_string();
        value.tokens.push(Token {
            id,
            email,
            label,
            is_group: false,
            group_id: None,
            member_count: None,
        });
    }
    value.text.clear();
}

/// Handle paste with RFC 5322 parsing, dedup, and validation.
fn handle_paste(
    state: &mut ComposeState,
    field: RecipientField,
    content: &str,
) {
    let mut parsed = parse_pasted_addresses(content);

    // Dedup within paste
    dedup_parsed(&mut parsed);

    let value = field_value_mut(state, field);

    // Dedup against existing tokens
    let existing: std::collections::HashSet<String> = value
        .tokens
        .iter()
        .map(|t| t.email.to_lowercase())
        .collect();

    let mut added_count = 0usize;
    for addr in &parsed {
        if existing.contains(&addr.email) {
            continue;
        }
        let id = value.next_token_id();
        let label = addr
            .display_name
            .as_deref()
            .unwrap_or(&addr.email)
            .to_string();
        value.tokens.push(Token {
            id,
            email: addr.email.clone(),
            label,
            is_group: false,
            group_id: None,
            member_count: None,
        });
        added_count += 1;
    }

    value.text.clear();

    // Bulk paste banner for 10+ addresses
    let bulk_threshold = 10;
    if added_count >= bulk_threshold {
        state.bulk_paste_banner = Some(BulkPasteBanner {
            visible: true,
            count: added_count,
        });
    }
}

fn handle_autocomplete_results(
    state: &mut ComposeState,
    gen: u64,
    results: Result<Vec<ContactMatch>, String>,
) {
    // Discard stale results
    if gen != state.autocomplete.search_generation {
        return;
    }
    match results {
        Ok(r) => {
            state.autocomplete.highlighted = if r.is_empty() {
                None
            } else {
                Some(0)
            };
            state.autocomplete.results = r;
        }
        Err(_) => {
            state.autocomplete.results.clear();
            state.autocomplete.highlighted = None;
        }
    }
}

fn handle_autocomplete_select(state: &mut ComposeState, idx: usize) {
    let Some(result) = state.autocomplete.results.get(idx) else {
        return;
    };
    let Some(field) = state.autocomplete.active_field else {
        return;
    };

    let value = field_value_mut(state, field);
    let id = value.next_token_id();
    let label = result
        .display_name
        .as_deref()
        .unwrap_or(&result.email)
        .to_string();
    value.tokens.push(Token {
        id,
        email: result.email.clone(),
        label,
        is_group: false,
        group_id: None,
        member_count: None,
    });
    value.text.clear();

    // Clear autocomplete
    state.autocomplete.query.clear();
    state.autocomplete.results.clear();
    state.autocomplete.highlighted = None;
}

fn handle_autocomplete_navigate(state: &mut ComposeState, delta: i32) {
    if state.autocomplete.results.is_empty() {
        return;
    }
    let len = state.autocomplete.results.len();
    let current = state.autocomplete.highlighted.unwrap_or(0);
    #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
    let new_idx = ((current as i32 + delta).rem_euclid(len as i32)) as usize;
    state.autocomplete.highlighted = Some(new_idx);
}

// ── Field accessors ──────────────────────────────────────

fn field_value_mut(
    state: &mut ComposeState,
    field: RecipientField,
) -> &mut TokenInputValue {
    match field {
        RecipientField::To => &mut state.to,
        RecipientField::Cc => &mut state.cc,
        RecipientField::Bcc => &mut state.bcc,
    }
}

fn field_selected_mut(
    state: &mut ComposeState,
    field: RecipientField,
) -> &mut Option<TokenId> {
    match field {
        RecipientField::To => &mut state.selected_to_token,
        RecipientField::Cc => &mut state.selected_cc_token,
        RecipientField::Bcc => &mut state.selected_bcc_token,
    }
}

// ── View ────────────────────────────────────────────────

pub fn view_compose_window<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let header = compose_header(window_id, state);
    let body = compose_body(window_id, state);
    let footer = compose_footer(window_id, state);

    let mut content_parts: Vec<Element<'a, Message>> =
        vec![header, widgets::divider()];

    // Bulk paste banner
    if let Some(ref banner) = state.bulk_paste_banner {
        if banner.visible {
            content_parts.push(bulk_paste_banner_view(window_id, banner));
        }
    }

    content_parts.push(body);
    content_parts.push(widgets::divider());
    content_parts.push(footer);

    let content = iced::widget::Column::with_children(content_parts)
        .spacing(SPACE_0);

    // Wrap in context menu dismiss layer if menu is open
    let base: Element<'a, Message> = container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::ContainerClass::Content.style())
        .into();

    if state.context_menu.is_some() {
        // Click anywhere to dismiss context menu
        mouse_area(base)
            .on_press(Message::PopOut(
                window_id,
                PopOutMessage::Compose(ComposeMessage::ContextMenuDismiss),
            ))
            .into()
    } else {
        base
    }
}

fn bulk_paste_banner_view<'a>(
    window_id: iced::window::Id,
    banner: &BulkPasteBanner,
) -> Element<'a, Message> {
    let dismiss_msg = Message::PopOut(
        window_id,
        PopOutMessage::Compose(ComposeMessage::DismissBulkPasteBanner),
    );

    let banner_row = row![
        text(format!("{} addresses pasted. Save as a contact group?", banner.count))
            .size(TEXT_SM),
        Space::new().width(Length::Fill),
        button(text("Dismiss").size(TEXT_SM))
            .style(theme::ButtonClass::Ghost.style())
            .on_press(dismiss_msg)
            .padding(PAD_ICON_BTN),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    container(banner_row)
        .padding(PAD_CARD)
        .width(Length::Fill)
        .style(theme::ContainerClass::Surface.style())
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

    // To field + autocomplete dropdown
    fields = fields.push(build_recipient_field_with_autocomplete(
        window_id,
        state,
        "To",
        &state.to,
        state.selected_to_token,
        "Add recipients...",
        ComposeMessage::ToTokenInput,
        RecipientField::To,
    ));

    // Cc field (if shown)
    if state.show_cc {
        fields = fields.push(build_recipient_field_with_autocomplete(
            window_id,
            state,
            "Cc",
            &state.cc,
            state.selected_cc_token,
            "Add Cc...",
            ComposeMessage::CcTokenInput,
            RecipientField::Cc,
        ));
    }

    // Bcc field (if shown)
    if state.show_bcc {
        fields = fields.push(build_recipient_field_with_autocomplete(
            window_id,
            state,
            "Bcc",
            &state.bcc,
            state.selected_bcc_token,
            "Add Bcc...",
            ComposeMessage::BccTokenInput,
            RecipientField::Bcc,
        ));
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

#[allow(clippy::too_many_arguments)]
fn build_recipient_field_with_autocomplete<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
    label: &'a str,
    value: &'a TokenInputValue,
    selected: Option<TokenId>,
    placeholder: &'a str,
    wrap: fn(TokenInputMessage) -> ComposeMessage,
    field: RecipientField,
) -> Element<'a, Message> {
    let token_field = token_input::token_input_field(
        &value.tokens,
        &value.text,
        placeholder,
        selected,
        move |msg| {
            Message::PopOut(window_id, PopOutMessage::Compose(wrap(msg)))
        },
    );

    let show_dropdown = state.autocomplete_visible()
        && state.autocomplete.active_field == Some(field);

    let mut field_col = column![token_field].spacing(SPACE_0);

    if show_dropdown {
        field_col = field_col
            .push(autocomplete_dropdown_view(window_id, state));
    }

    row![
        container(
            text(label)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style())
        )
        .width(COMPOSE_LABEL_WIDTH)
        .align_y(Alignment::Center),
        field_col,
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Start)
    .into()
}

fn autocomplete_dropdown_view<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let items: Vec<Element<'a, Message>> = state
        .autocomplete
        .results
        .iter()
        .enumerate()
        .map(|(idx, result)| {
            let is_highlighted =
                state.autocomplete.highlighted == Some(idx);
            autocomplete_row_view(window_id, result, is_highlighted, idx)
        })
        .collect();

    let menu = container(
        iced::widget::Column::with_children(items)
            .spacing(SPACE_0)
            .width(Length::Fill),
    )
    .padding(PAD_BADGE)
    .width(Length::Fill)
    .max_height(AUTOCOMPLETE_MAX_HEIGHT)
    .style(theme::ContainerClass::Floating.style());

    menu.into()
}

fn autocomplete_row_view<'a>(
    window_id: iced::window::Id,
    result: &'a ContactMatch,
    highlighted: bool,
    index: usize,
) -> Element<'a, Message> {
    let name_text = result
        .display_name
        .as_deref()
        .unwrap_or(&result.email);

    let email_text = if result.display_name.is_some() {
        result.email.as_str()
    } else {
        "" // Don't duplicate email if it's already the name
    };

    let mut content = row![
        text(name_text).size(TEXT_MD).width(Length::Fill),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    if !email_text.is_empty() {
        content = content.push(
            text(email_text)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        );
    }

    let style = if highlighted {
        theme::ContainerClass::Surface
    } else {
        theme::ContainerClass::Content
    };

    let row_container = container(content)
        .padding(PAD_NAV_ITEM)
        .width(Length::Fill)
        .height(AUTOCOMPLETE_ROW_HEIGHT)
        .style(style.style());

    let select_msg = Message::PopOut(
        window_id,
        PopOutMessage::Compose(ComposeMessage::AutocompleteSelect(index)),
    );

    mouse_area(row_container)
        .on_press(select_msg)
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
            PopOutMessage::Compose(ComposeMessage::FromAccountChanged(
                account,
            )),
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

fn compose_footer<'a>(
    window_id: iced::window::Id,
    _state: &'a ComposeState,
) -> Element<'a, Message> {
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
        PopOutMessage::Compose(ComposeMessage::Discard),
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
