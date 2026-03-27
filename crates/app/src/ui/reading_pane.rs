use std::collections::HashMap;

use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Element, Length, Padding, Task};

use ratatoskr_command_palette::{BindingTable, CommandContext, CommandId, CommandRegistry};

use crate::component::Component;
use crate::db::{AppThreadDetail, DateDisplay, ResolvedLabel, Thread, ThreadAttachment, ThreadMessage};
use crate::font;
use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;
use crate::Message;

// ── Messages & Events ──────────────────────────────────

#[derive(Debug, Clone)]
pub enum ReadingPaneMessage {
    ToggleMessageExpanded(usize),
    ToggleAllMessages,
    ToggleAttachmentsCollapsed,
    PopOut(usize),
    /// Reply to the message at the given index.
    Reply(usize),
    /// Reply All to the message at the given index.
    ReplyAll(usize),
    /// Forward the message at the given index.
    Forward(usize),
    /// Open the inline contact editor popover for the given email address.
    EditContact(String),
    /// Create a calendar event from this message (📅 button).
    CreateEventFromEmail(usize),
    /// Navigate to the next message in the thread (expand it).
    NextMessage,
    /// Navigate to the previous message in the thread (expand it).
    PrevMessage,
    ToggleStar,
    /// User clicked a hyperlink in the HTML body; open it in the system browser.
    LinkClicked(String),
    Noop,
}

/// Events the reading pane emits upward to the App.
#[derive(Debug, Clone)]
pub enum ReadingPaneEvent {
    AttachmentCollapseChanged { thread_key: String, collapsed: bool },
    /// User clicked the pop-out button on a message; open it in a new window.
    OpenMessagePopOut { message_index: usize },
    /// User clicked Reply on a specific message.
    ReplyToMessage { message_index: usize },
    /// User clicked Reply All on a specific message.
    ReplyAllToMessage { message_index: usize },
    /// User clicked Forward on a specific message.
    ForwardMessage { message_index: usize },
    /// User clicked a sender/recipient to edit their contact info.
    EditContact { email: String },
    /// User clicked 📅 on a message to create a calendar event.
    CreateEventFromEmail { message_index: usize },
    /// User toggled the star on the thread.
    ToggleStar,
}

// ── State ──────────────────────────────────────────────

pub struct ReadingPane {
    pub thread_messages: Vec<ThreadMessage>,
    pub thread_attachments: Vec<ThreadAttachment>,
    pub thread_labels: Vec<ResolvedLabel>,
    pub message_expanded: Vec<bool>,
    /// Index of the currently focused message (for keyboard navigation).
    pub focused_message: Option<usize>,
    pub attachments_collapsed: bool,
    pub attachment_collapse_cache: HashMap<String, bool>,
    pub date_display: DateDisplay,
    /// The currently viewed thread (set by App when a thread is selected).
    current_thread: Option<ThreadRef>,
    /// Search terms to highlight in message bodies. Empty when not in search mode.
    pub search_highlight_terms: Vec<String>,
    /// Cached deduplicated attachments: `(index_into_thread_attachments, duplicate_count)`.
    /// Computed once when thread detail loads, not on every view cycle.
    deduped_attachments: Vec<(usize, usize)>,
    /// Pre-parsed HTML bodies, one per message. Avoids re-parsing HTML on every
    /// view cycle — only rebuilt when thread detail loads.
    cached_html: Vec<Option<super::html_render::CachedHtmlBody>>,
    /// Pre-loaded inline images for CID resolution in HTML bodies.
    /// Maps Content-ID to image bytes.
    inline_images: HashMap<String, Vec<u8>>,
}

/// Minimal reference to the currently selected thread for display purposes.
#[derive(Debug, Clone)]
struct ThreadRef {
    account_id: String,
    id: String,
    subject: Option<String>,
    is_starred: bool,
}

impl ReadingPane {
    pub fn new() -> Self {
        Self {
            thread_messages: Vec::new(),
            thread_attachments: Vec::new(),
            thread_labels: Vec::new(),
            message_expanded: Vec::new(),
            focused_message: None,
            attachments_collapsed: false,
            attachment_collapse_cache: HashMap::new(),
            date_display: DateDisplay::RelativeOffset,
            current_thread: None,
            search_highlight_terms: Vec::new(),
            deduped_attachments: Vec::new(),
            cached_html: Vec::new(),
            inline_images: HashMap::new(),
        }
    }

    /// Set the currently viewed thread. Called by App on thread selection.
    pub fn set_thread(&mut self, thread: Option<&Thread>) {
        self.current_thread = thread.map(|t| ThreadRef {
            account_id: t.account_id.clone(),
            id: t.id.clone(),
            subject: t.subject.clone(),
            is_starred: t.is_starred,
        });
        self.thread_messages.clear();
        self.thread_attachments.clear();
        self.thread_labels.clear();
        self.message_expanded.clear();
        self.focused_message = None;
        self.deduped_attachments.clear();
        self.cached_html.clear();
        self.inline_images.clear();
        // Restore attachment collapse state from cache
        self.attachments_collapsed = thread
            .map(|t| {
                let key = format!("{}:{}", t.account_id, t.id);
                self.attachment_collapse_cache.get(&key).copied().unwrap_or(false)
            })
            .unwrap_or(false);
    }

    /// Apply message expansion rules after messages are loaded.
    ///
    /// Rules (messages are newest-first):
    /// 1. Unread messages are expanded
    /// 2. Most recent message (index 0) is expanded
    /// 3. First message in thread (last index) is expanded
    /// 4. Own messages are collapsed (unless rule 1-3 applies)
    /// 5. When search terms are present, messages containing any term are expanded
    pub fn apply_message_expansion(&mut self, messages: &[ThreadMessage]) {
        let len = messages.len();
        let mut expanded = vec![false; len];

        if !self.search_highlight_terms.is_empty() {
            // Search mode: expand messages that contain any search term
            for (i, msg) in messages.iter().enumerate() {
                if message_matches_search_terms(msg, &self.search_highlight_terms) {
                    expanded[i] = true;
                }
            }
            // Always expand first and last if nothing matched
            if !expanded.iter().any(|&e| e) && len > 0 {
                expanded[0] = true;
                if len > 1 {
                    expanded[len - 1] = true;
                }
            }
        } else {
            for (i, msg) in messages.iter().enumerate() {
                // Rules 1-3: unread, most recent, or initial message
                if !msg.is_read || i == 0 || i == len - 1 {
                    expanded[i] = true;
                }
                // Rule 4: own messages default to collapsed
                if msg.is_own_message && msg.is_read && i != 0 && i != len - 1 {
                    expanded[i] = false;
                }
            }
        }

        self.message_expanded = expanded;
    }

    /// Load thread detail from core's get_thread_detail response.
    pub fn load_thread_detail(&mut self, detail: AppThreadDetail) {
        // Update thread ref with detail data
        self.current_thread = Some(ThreadRef {
            account_id: detail.account_id.clone(),
            id: detail.thread_id.clone(),
            subject: detail.subject,
            is_starred: detail.is_starred,
        });

        // Set attachments collapsed from persisted state
        self.attachments_collapsed = detail.attachments_collapsed;

        // Update cache too
        let key = format!("{}:{}", detail.account_id, detail.thread_id);
        self.attachment_collapse_cache
            .insert(key, detail.attachments_collapsed);

        // Apply expansion rules then set messages
        self.apply_message_expansion(&detail.messages);
        // Pre-parse HTML bodies so we don't re-parse on every view cycle.
        self.cached_html = detail
            .messages
            .iter()
            .map(|msg| {
                msg.body_html
                    .as_deref()
                    .map(super::html_render::preparse_html)
            })
            .collect();

        self.thread_messages = detail.messages;
        self.thread_attachments = detail.attachments;
        self.thread_labels = detail.labels;
        self.inline_images = detail.inline_images;
        self.recompute_deduped_attachments();
    }

    /// Recompute the deduplicated attachment list from `thread_attachments`.
    fn recompute_deduped_attachments(&mut self) {
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for att in &self.thread_attachments {
            let name = att.filename.as_deref().unwrap_or("");
            *seen.entry(name).or_insert(0) += 1;
        }

        let mut emitted: HashMap<&str, bool> = HashMap::new();
        self.deduped_attachments.clear();
        for (i, att) in self.thread_attachments.iter().enumerate() {
            let name = att.filename.as_deref().unwrap_or("");
            if !emitted.contains_key(name) {
                let count = seen.get(name).copied().unwrap_or(1);
                self.deduped_attachments.push((i, count));
                emitted.insert(name, true);
            }
        }
    }

    fn thread_key(&self) -> Option<String> {
        self.current_thread.as_ref().map(|t| format!("{}:{}", t.account_id, t.id))
    }

    /// Update the star state for a thread if it's currently displayed.
    pub fn update_star(&mut self, thread_id: &str, is_starred: bool) {
        if let Some(ref mut t) = self.current_thread {
            if t.id == thread_id {
                t.is_starred = is_starred;
            }
        }
    }

    /// Get the message ID of the currently focused message (for CommandContext).
    pub fn focused_message_id(&self) -> Option<String> {
        let idx = self.focused_message?;
        self.thread_messages
            .get(idx)
            .map(|m| m.id.clone())
    }
}

// ── Component impl ─────────────────────────────────────

impl Component for ReadingPane {
    type Message = ReadingPaneMessage;
    type Event = ReadingPaneEvent;

    fn update(
        &mut self,
        message: ReadingPaneMessage,
    ) -> (Task<ReadingPaneMessage>, Option<ReadingPaneEvent>) {
        match message {
            ReadingPaneMessage::ToggleMessageExpanded(index) => {
                if let Some(e) = self.message_expanded.get_mut(index) {
                    *e = !*e;
                }
                (Task::none(), None)
            }
            ReadingPaneMessage::ToggleAllMessages => {
                let all_expanded = self.message_expanded.iter().all(|&e| e);
                for e in &mut self.message_expanded {
                    *e = !all_expanded;
                }
                (Task::none(), None)
            }
            ReadingPaneMessage::ToggleAttachmentsCollapsed => {
                self.attachments_collapsed = !self.attachments_collapsed;
                let event = self.thread_key().map(|key| {
                    self.attachment_collapse_cache
                        .insert(key.clone(), self.attachments_collapsed);
                    ReadingPaneEvent::AttachmentCollapseChanged {
                        thread_key: key,
                        collapsed: self.attachments_collapsed,
                    }
                });
                (Task::none(), event)
            }
            ReadingPaneMessage::PopOut(index) => {
                let event = ReadingPaneEvent::OpenMessagePopOut {
                    message_index: index,
                };
                (Task::none(), Some(event))
            }
            ReadingPaneMessage::Reply(index) => {
                (Task::none(), Some(ReadingPaneEvent::ReplyToMessage {
                    message_index: index,
                }))
            }
            ReadingPaneMessage::ReplyAll(index) => {
                (Task::none(), Some(ReadingPaneEvent::ReplyAllToMessage {
                    message_index: index,
                }))
            }
            ReadingPaneMessage::Forward(index) => {
                (Task::none(), Some(ReadingPaneEvent::ForwardMessage {
                    message_index: index,
                }))
            }
            ReadingPaneMessage::EditContact(email) => {
                (Task::none(), Some(ReadingPaneEvent::EditContact { email }))
            }
            ReadingPaneMessage::CreateEventFromEmail(index) => {
                (Task::none(), Some(ReadingPaneEvent::CreateEventFromEmail { message_index: index }))
            }
            ReadingPaneMessage::NextMessage => {
                let len = self.thread_messages.len();
                if len == 0 {
                    return (Task::none(), None);
                }
                let next = match self.focused_message {
                    Some(idx) if idx + 1 < len => idx + 1,
                    Some(_) => return (Task::none(), None),
                    None => 0,
                };
                self.focused_message = Some(next);
                if let Some(e) = self.message_expanded.get_mut(next) {
                    *e = true;
                }
                (Task::none(), None)
            }
            ReadingPaneMessage::PrevMessage => {
                if self.thread_messages.is_empty() {
                    return (Task::none(), None);
                }
                let prev = match self.focused_message {
                    Some(idx) if idx > 0 => idx - 1,
                    Some(_) => return (Task::none(), None),
                    None => self.thread_messages.len() - 1,
                };
                self.focused_message = Some(prev);
                if let Some(e) = self.message_expanded.get_mut(prev) {
                    *e = true;
                }
                (Task::none(), None)
            }
            ReadingPaneMessage::ToggleStar => {
                (Task::none(), Some(ReadingPaneEvent::ToggleStar))
            }
            ReadingPaneMessage::LinkClicked(url) => {
                open_url_in_browser(&url);
                (Task::none(), None)
            }
            ReadingPaneMessage::Noop => (Task::none(), None),
        }
    }

    fn view(&self) -> Element<'_, ReadingPaneMessage> {
        match self.current_thread.as_ref() {
            None => empty_reading_pane(),
            Some(thread_ref) => thread_view(self, thread_ref),
        }
    }
}

// ── Command-backed view ─────────────────────────────────

impl ReadingPane {
    /// Render the reading pane with a command-backed toolbar.
    ///
    /// Unlike the `Component::view()` trait method (which returns
    /// `Element<ReadingPaneMessage>`), this returns `Element<Message>`
    /// so that toolbar buttons can emit `Message::ExecuteCommand`
    /// directly. Internal reading pane messages are mapped via
    /// `Message::ReadingPane`.
    pub fn view_with_commands(
        &self,
        registry: &CommandRegistry,
        binding_table: &BindingTable,
        ctx: &CommandContext,
    ) -> Element<'_, Message> {
        match self.current_thread.as_ref() {
            None => empty_reading_pane().map(Message::ReadingPane),
            Some(thread_ref) => {
                thread_view_with_commands(self, thread_ref, registry, binding_table, ctx)
            }
        }
    }
}

fn thread_view_with_commands<'a>(
    pane: &'a ReadingPane,
    thread_ref: &'a ThreadRef,
    registry: &CommandRegistry,
    binding_table: &BindingTable,
    ctx: &CommandContext,
) -> Element<'a, Message> {
    let mut col = column![].spacing(0).width(Length::Fill);

    // Thread header (subject, expand/collapse) — uses ReadingPaneMessage internally
    col = col.push(
        thread_header(thread_ref, &pane.message_expanded, &pane.thread_messages, &pane.thread_labels, ctx.allows_set_keywords())
            .map(Message::ReadingPane),
    );

    // Command-backed toolbar
    col = col.push(command_toolbar(registry, binding_table, ctx));

    if !pane.thread_attachments.is_empty() {
        col = col.push(
            attachment_group(&pane.thread_attachments, &pane.deduped_attachments, pane.attachments_collapsed)
                .map(Message::ReadingPane),
        );
    }

    col = col.push(message_list(pane).map(Message::ReadingPane));
    col.into()
}

/// Build the reading pane toolbar from command registry metadata.
///
/// Buttons pull labels, availability, and keybinding hints from the
/// registry. Toggle commands (Star/Unstar) resolve their label
/// automatically via `resolved_label()`.
fn command_toolbar<'a>(
    registry: &CommandRegistry,
    binding_table: &BindingTable,
    ctx: &CommandContext,
) -> Element<'a, Message> {
    let toolbar = row![
        widgets::command_icon_button(
            CommandId::ComposeReply,
            icon::reply(),
            registry,
            binding_table,
            ctx,
        ),
        widgets::command_icon_button(
            CommandId::ComposeReplyAll,
            icon::reply_all(),
            registry,
            binding_table,
            ctx,
        ),
        widgets::command_icon_button(
            CommandId::ComposeForward,
            icon::forward(),
            registry,
            binding_table,
            ctx,
        ),
        Space::new().width(Length::Fill),
        widgets::command_icon_button(
            CommandId::EmailArchive,
            icon::archive(),
            registry,
            binding_table,
            ctx,
        ),
        widgets::command_icon_button(
            CommandId::EmailTrash,
            icon::trash(),
            registry,
            binding_table,
            ctx,
        ),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    container(toolbar)
        .padding(PAD_CONTENT)
        .width(Length::Fill)
        .into()
}

// ── View helpers ────────────────────────────────────────

fn empty_reading_pane<'a>() -> Element<'a, ReadingPaneMessage> {
    container(widgets::empty_placeholder(
        "No conversation selected",
        "Select a thread to read",
    ))
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn thread_view<'a>(
    pane: &'a ReadingPane,
    thread_ref: &'a ThreadRef,
) -> Element<'a, ReadingPaneMessage> {
    let mut col = column![].spacing(0).width(Length::Fill);

    // No command context → allow star by default (non-command path)
    col = col.push(thread_header(thread_ref, &pane.message_expanded, &pane.thread_messages, &pane.thread_labels, true));

    if !pane.thread_attachments.is_empty() {
        col = col.push(attachment_group(&pane.thread_attachments, &pane.deduped_attachments, pane.attachments_collapsed));
    }

    col = col.push(message_list(pane));
    col.into()
}

fn thread_header<'a>(
    thread_ref: &'a ThreadRef,
    message_expanded: &'a [bool],
    messages: &'a [ThreadMessage],
    labels: &'a [ResolvedLabel],
    may_set_keywords: bool,
) -> Element<'a, ReadingPaneMessage> {
    let subject = thread_ref.subject.as_deref().unwrap_or("(no subject)");

    let star_icon_style: fn(&iced::Theme) -> text::Style = if !may_set_keywords {
        theme::TextClass::Tertiary.style()
    } else if thread_ref.is_starred {
        text::warning
    } else {
        text::secondary
    };

    let mut star_btn = button(icon::star().size(ICON_XL).style(star_icon_style))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());

    if may_set_keywords {
        star_btn = star_btn.on_press(ReadingPaneMessage::ToggleStar);
    }

    let toggle_label = if message_expanded.iter().all(|&e| e) {
        "Collapse all"
    } else {
        "Expand all"
    };

    let expand_collapse_btn = button(
        text(toggle_label)
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style()),
    )
    .on_press(ReadingPaneMessage::ToggleAllMessages)
    .style(theme::ButtonClass::Ghost.style())
    .padding(PAD_ICON_BTN);

    let mut info_row = row![
        container(
            text(format!("{} messages", messages.len()))
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        )
        .align_y(Alignment::Center),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    // Label pills — only show tag-type labels (not folder/container labels)
    for label in labels.iter().filter(|l| l.label_kind == "tag") {
        let bg = theme::hex_to_color(&label.color_bg);
        let fg = theme::hex_to_color(&label.color_fg);
        info_row = info_row.push(
            container(
                text(&label.name).size(TEXT_XS).color(fg),
            )
            .padding(Padding { top: 2.0, right: 6.0, bottom: 2.0, left: 6.0 })
            .style(move |_theme: &iced::Theme| container::Style {
                background: Some(bg.into()),
                border: iced::Border {
                    radius: RADIUS_LG.into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
        );
    }

    info_row = info_row.push(Space::new().width(Length::Fill));
    info_row = info_row.push(expand_collapse_btn);

    container(
        column![
            row![
                container(text(subject).size(TEXT_HEADING).style(text::base))
                    .width(Length::Fill),
                star_btn,
            ]
            .align_y(Alignment::Center),
            info_row,
        ]
        .spacing(SPACE_XXS),
    )
    .padding(PAD_CONTENT)
    .into()
}

/// Check if a message's content matches any of the search terms (case-insensitive).
fn message_matches_search_terms(msg: &ThreadMessage, terms: &[String]) -> bool {
    let check = |field: &Option<String>| {
        field.as_deref().is_some_and(|text| {
            let lower = text.to_lowercase();
            terms.iter().any(|t| lower.contains(&t.to_lowercase()))
        })
    };
    check(&msg.body_text)
        || check(&msg.body_html)
        || check(&msg.subject)
        || check(&msg.from_name)
        || check(&msg.from_address)
        || check(&msg.snippet)
}

fn message_list<'a>(pane: &'a ReadingPane) -> Element<'a, ReadingPaneMessage> {
    let messages_pad = Padding::from([0.0, SPACE_LG]);
    let first_message_date = pane.thread_messages.last().and_then(|m| m.date);
    let mut msg_col = column![].spacing(SPACE_XS).padding(messages_pad);

    for (i, msg) in pane.thread_messages.iter().enumerate() {
        let is_expanded = pane.message_expanded.get(i).copied().unwrap_or(false);
        if is_expanded {
            let cached_html = pane.cached_html.get(i).and_then(|c| c.as_ref());
            msg_col = msg_col.push(widgets::expanded_message_card(
                msg,
                i,
                pane.date_display,
                first_message_date,
                &pane.search_highlight_terms,
                cached_html,
                &pane.thread_labels,
                &pane.inline_images,
                ReadingPaneMessage::ToggleMessageExpanded,
                ReadingPaneMessage::PopOut,
                ReadingPaneMessage::Reply,
                ReadingPaneMessage::ReplyAll,
                ReadingPaneMessage::Forward,
                ReadingPaneMessage::EditContact,
                ReadingPaneMessage::CreateEventFromEmail,
                ReadingPaneMessage::LinkClicked,
            ));
        } else {
            msg_col = msg_col.push(widgets::collapsed_message_row(
                msg,
                i,
                ReadingPaneMessage::ToggleMessageExpanded,
            ));
        }
    }

    msg_col = msg_col.push(Space::new().height(SPACE_MD));

    scrollable(msg_col)
        .spacing(SCROLLBAR_SPACING)
        .height(Length::Fill)
        .into()
}

// ── Attachment group ────────────────────────────────────

fn attachment_group<'a>(
    attachments: &'a [ThreadAttachment],
    deduped_indices: &'a [(usize, usize)],
    collapsed: bool,
) -> Element<'a, ReadingPaneMessage> {

    let chevron = if collapsed {
        icon::chevron_right()
    } else {
        icon::chevron_down()
    };

    let header = button(
        row![
            container(chevron.size(ICON_XS).style(theme::TextClass::Tertiary.style()))
                .align_y(Alignment::Center),
            Space::new().width(SPACE_XXS),
            container(
                text(format!("Attachments ({})", deduped_indices.len()))
                    .size(TEXT_MD)
                    .font(font::text_semibold())
                    .style(text::base),
            )
            .align_y(Alignment::Center),
            Space::new().width(Length::Fill),
            container(
                text("Save All")
                    .size(TEXT_SM)
                    .style(theme::TextClass::Accent.style()),
            )
            .align_y(Alignment::Center),
        ]
        .align_y(Alignment::Center),
    )
    .on_press(ReadingPaneMessage::ToggleAttachmentsCollapsed)
    .style(theme::ButtonClass::Ghost.style())
    .width(Length::Fill);

    let mut content_col = column![header].spacing(SPACE_XS);

    if !collapsed {
        for &(att_idx, version_count) in deduped_indices {
            if let Some(att) = attachments.get(att_idx) {
                content_col = content_col.push(widgets::attachment_card(att, version_count));
            }
        }
    }

    container(
        container(content_col)
            .padding(PAD_CARD)
            .style(theme::ContainerClass::Elevated.style()),
    )
    .padding(PAD_CONTENT)
    .width(Length::Fill)
    .into()
}

/// Open a URL in the system default browser.
fn open_url_in_browser(url: &str) {
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg(url)
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg(url)
            .spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", url])
            .spawn();
    }
}
