use rte::EditorState;

use crate::db;
use crate::ui::layout::{COMPOSE_DEFAULT_HEIGHT, COMPOSE_DEFAULT_WIDTH};
use crate::ui::token_input::{self, TokenId, TokenInputValue};

use super::helpers::{accounts_to_info, build_attribution, csv_to_token_input};
use super::messages::ComposeMode;
use super::types::{
    AccountInfo, AutocompleteState, BccNudgeBanner, BulkPasteBanner, ComposeAttachment,
    ComposeTokenDrag, TokenContextMenuState,
};

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
    pub from_dropdown_open: bool,

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

    /// Inline validation error shown directly under the To row when the
    /// user tries to send with no recipients. Cleared as soon as any
    /// recipient is added so the message disappears the moment the
    /// problem is fixed - no separate "dismiss" UI needed.
    pub recipients_error: Option<String>,

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

    // Save-as-group dialog (driven from the bulk-paste banner)
    pub save_group_dialog_open: bool,
    pub save_group_name: String,
    pub save_group_error: Option<String>,
    pub save_group_in_flight: bool,

    // Window geometry
    pub width: f32,
    pub height: f32,
    pub x: Option<f32>,
    pub y: Option<f32>,

    // Signature tracking
    pub active_signature_id: Option<String>,
    pub signature_separator_index: Option<usize>,

    // Draft auto-save
    pub draft_id: String,
    pub draft_dirty: bool,

    // Send in progress - disables Send button, shows "Sending..." status
    pub sending: bool,

    // Draft ID for the send path - set on first send attempt, reused on retry
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
            from_dropdown_open: false,
            subject: String::new(),
            body: EditorState::new(),
            mode: ComposeMode::New,
            reply_thread_id: None,
            reply_message_id: None,
            status: None,
            recipients_error: None,
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
            save_group_dialog_open: false,
            save_group_name: String::new(),
            save_group_error: None,
            save_group_in_flight: false,
            active_signature_id: None,
            signature_separator_index: None,
            width: COMPOSE_DEFAULT_WIDTH,
            height: COMPOSE_DEFAULT_HEIGHT,
            x: None,
            y: None,
            draft_id: uuid::Uuid::new_v4().to_string(),
            draft_dirty: false,
            sending: false,
            send_draft_id: None,
        }
    }

    /// Restore a compose window from a persisted local draft.
    ///
    /// Reconstructs the body from saved HTML and restores the signature
    /// separator index and active signature ID so the editor knows where
    /// the signature boundary is.
    pub fn from_local_draft(
        accounts: &[db::Account],
        draft: &rtsk::db::types::DbLocalDraft,
    ) -> Self {
        let from_accounts = accounts_to_info(accounts);
        let from_account = draft
            .from_email
            .as_ref()
            .and_then(|email| from_accounts.iter().find(|a| &a.email == email).cloned())
            .or_else(|| from_accounts.first().cloned());

        let to = csv_to_token_input(draft.to_addresses.as_deref());
        let cc = csv_to_token_input(draft.cc_addresses.as_deref());
        let bcc = csv_to_token_input(draft.bcc_addresses.as_deref());
        let show_cc = !cc.tokens.is_empty();
        let show_bcc = !bcc.tokens.is_empty();

        let body = draft
            .body_html
            .as_deref()
            .filter(|h| !h.trim().is_empty())
            .map(EditorState::from_html)
            .unwrap_or_default();

        let signature_separator_index = draft
            .signature_separator_index
            .filter(|&i| i >= 0)
            .and_then(|i| usize::try_from(i).ok());

        Self {
            to,
            cc,
            bcc,
            show_cc,
            show_bcc,
            selected_to_token: None,
            selected_cc_token: None,
            selected_bcc_token: None,
            from_account,
            from_accounts,
            from_dropdown_open: false,
            subject: draft.subject.clone().unwrap_or_default(),
            body,
            mode: ComposeMode::New,
            reply_thread_id: draft.thread_id.clone(),
            reply_message_id: draft.reply_to_message_id.clone(),
            status: None,
            recipients_error: None,
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
            save_group_dialog_open: false,
            save_group_name: String::new(),
            save_group_error: None,
            save_group_in_flight: false,
            active_signature_id: draft.signature_id.clone(),
            signature_separator_index,
            width: COMPOSE_DEFAULT_WIDTH,
            height: COMPOSE_DEFAULT_HEIGHT,
            x: None,
            y: None,
            draft_id: draft.id.clone(),
            draft_dirty: false,
            sending: false,
            send_draft_id: None,
        }
    }

    // TODO(refactor): wrap reply fields in a ReplyContext struct.
    #[allow(clippy::too_many_arguments)]
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

        state.subject = mode.prefixed_subject();

        if !matches!(state.mode, ComposeMode::Forward { .. })
            && let Some(email) = to_email
        {
            let label = to_name.filter(|n| !n.is_empty()).unwrap_or(email);
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

        if let ComposeMode::ReplyAll { .. } = &state.mode
            && let Some(cc_str) = cc_emails
        {
            for addr in cc_str.split(',').map(str::trim).filter(|s| !s.is_empty()) {
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

        if let Some(body) = quoted_body {
            let attribution = build_attribution(to_name, to_email);
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
    pub fn set_shared_mailbox_from(&mut self, parent_account_id: &str, shared_email: &str) {
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
    pub fn has_user_content(&self) -> bool {
        let body_text = self.body.document.flattened_text();
        let trimmed = body_text.trim();
        !trimmed.is_empty()
    }
}
