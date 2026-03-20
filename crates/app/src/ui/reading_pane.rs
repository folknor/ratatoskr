use std::collections::HashMap;

use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Element, Length, Padding, Task};

use ratatoskr_command_palette::{BindingTable, CommandContext, CommandId, CommandRegistry};

use crate::component::Component;
use crate::db::{DateDisplay, Thread, ThreadAttachment, ThreadMessage};
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
    Noop,
}

/// Events the reading pane emits upward to the App.
#[derive(Debug, Clone)]
pub enum ReadingPaneEvent {
    AttachmentCollapseChanged { thread_key: String, collapsed: bool },
}

// ── State ──────────────────────────────────────────────

pub struct ReadingPane {
    pub thread_messages: Vec<ThreadMessage>,
    pub thread_attachments: Vec<ThreadAttachment>,
    pub message_expanded: Vec<bool>,
    pub attachments_collapsed: bool,
    pub attachment_collapse_cache: HashMap<String, bool>,
    pub date_display: DateDisplay,
    /// The currently viewed thread (set by App when a thread is selected).
    current_thread: Option<ThreadRef>,
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
            message_expanded: Vec::new(),
            attachments_collapsed: false,
            attachment_collapse_cache: HashMap::new(),
            date_display: DateDisplay::RelativeOffset,
            current_thread: None,
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
        self.message_expanded.clear();
        // Restore attachment collapse state from cache
        self.attachments_collapsed = thread
            .map(|t| {
                let key = format!("{}:{}", t.account_id, t.id);
                self.attachment_collapse_cache.get(&key).copied().unwrap_or(false)
            })
            .unwrap_or(false);
    }

    /// Apply message expansion rules after messages are loaded.
    pub fn apply_message_expansion(&mut self, messages: &[ThreadMessage]) {
        let len = messages.len();
        let mut expanded = vec![false; len];

        for (i, msg) in messages.iter().enumerate() {
            if !msg.is_read || i == 0 || i == len - 1 {
                expanded[i] = true;
            }
        }

        self.message_expanded = expanded;
    }

    fn thread_key(&self) -> Option<String> {
        self.current_thread.as_ref().map(|t| format!("{}:{}", t.account_id, t.id))
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
        thread_header(thread_ref, &pane.message_expanded, &pane.thread_messages)
            .map(Message::ReadingPane),
    );

    // Command-backed toolbar
    col = col.push(command_toolbar(registry, binding_table, ctx));

    if !pane.thread_attachments.is_empty() {
        col = col.push(
            attachment_group(&pane.thread_attachments, pane.attachments_collapsed)
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
        widgets::command_icon_button(
            CommandId::EmailStar,
            icon::star(),
            registry,
            binding_table,
            ctx,
        ),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    container(toolbar)
        .padding(PAD_TOOLBAR)
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

    col = col.push(thread_header(thread_ref, &pane.message_expanded, &pane.thread_messages));

    if !pane.thread_attachments.is_empty() {
        col = col.push(attachment_group(&pane.thread_attachments, pane.attachments_collapsed));
    }

    col = col.push(message_list(pane));
    col.into()
}

fn thread_header<'a>(
    thread_ref: &'a ThreadRef,
    message_expanded: &'a [bool],
    messages: &'a [ThreadMessage],
) -> Element<'a, ReadingPaneMessage> {
    let subject = thread_ref.subject.as_deref().unwrap_or("(no subject)");

    let star_icon_style: fn(&iced::Theme) -> text::Style = if thread_ref.is_starred {
        text::warning
    } else {
        text::secondary
    };
    let star_btn_class = if thread_ref.is_starred {
        theme::ButtonClass::StarActive
    } else {
        theme::ButtonClass::BareIcon
    };

    let star_btn = button(icon::star().size(ICON_XL).style(star_icon_style))
        .on_press(ReadingPaneMessage::Noop)
        .padding(PAD_ICON_BTN)
        .style(star_btn_class.style());

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

    container(
        column![
            row![
                container(text(subject).size(TEXT_HEADING).style(text::base))
                    .width(Length::Fill),
                star_btn,
            ]
            .align_y(Alignment::Center),
            row![
                container(
                    text(format!("{} messages", messages.len()))
                        .size(TEXT_SM)
                        .style(theme::TextClass::Tertiary.style()),
                )
                .align_y(Alignment::Center),
                Space::new().width(Length::Fill),
                expand_collapse_btn,
            ]
            .align_y(Alignment::Center),
        ]
        .spacing(SPACE_XXS),
    )
    .padding(PAD_CONTENT)
    .into()
}

fn message_list<'a>(pane: &'a ReadingPane) -> Element<'a, ReadingPaneMessage> {
    let messages_pad = Padding::from([0.0, SPACE_LG]);
    let first_message_date = pane.thread_messages.last().and_then(|m| m.date);
    let mut msg_col = column![].spacing(SPACE_XS).padding(messages_pad);

    for (i, msg) in pane.thread_messages.iter().enumerate() {
        let is_expanded = pane.message_expanded.get(i).copied().unwrap_or(false);
        if is_expanded {
            msg_col = msg_col.push(widgets::expanded_message_card(
                msg,
                i,
                pane.date_display,
                first_message_date,
                ReadingPaneMessage::ToggleMessageExpanded,
                ReadingPaneMessage::Noop,
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

fn dedup_attachments(attachments: &[ThreadAttachment]) -> Vec<(&ThreadAttachment, usize)> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    let mut result: Vec<(&ThreadAttachment, usize)> = Vec::new();

    for att in attachments {
        let name = att.filename.as_deref().unwrap_or("");
        *seen.entry(name).or_insert(0) += 1;
    }

    let mut emitted: HashMap<&str, bool> = HashMap::new();
    for att in attachments {
        let name = att.filename.as_deref().unwrap_or("");
        if !emitted.contains_key(name) {
            let count = seen.get(name).copied().unwrap_or(1);
            result.push((att, count));
            emitted.insert(name, true);
        }
    }

    result
}

fn attachment_group<'a>(
    attachments: &'a [ThreadAttachment],
    collapsed: bool,
) -> Element<'a, ReadingPaneMessage> {
    let deduped = dedup_attachments(attachments);

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
                text(format!("Attachments ({})", deduped.len()))
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
        for (att, version_count) in &deduped {
            content_col = content_col.push(widgets::attachment_card(att, *version_count));
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
