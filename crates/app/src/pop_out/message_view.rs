use iced::widget::{button, column, container, mouse_area, row, scrollable, stack, text};
use iced::{Alignment, Element, Length, Padding};

use crate::Message;
use crate::db::{self, ThreadMessage};
use crate::font;
use crate::icon;
use crate::pop_out::session::MessageViewSessionEntry;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;

use super::PopOutMessage;

pub use db::MessageViewAttachment;

// ── Rendering mode ─────────────────────────────────────

/// Rendering mode for the message body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderingMode {
    PlainText,
    #[default]
    SimpleHtml,
    OriginalHtml,
    Source,
}

// ── Messages ───────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum MessageViewMessage {
    /// Body content loaded from the body store.
    BodyLoaded(
        rtsk::generation::GenerationToken<rtsk::generation::PopOut>,
        Result<(Option<String>, Option<String>), String>,
    ),
    /// Attachments loaded for this message.
    AttachmentsLoaded(
        rtsk::generation::GenerationToken<rtsk::generation::PopOut>,
        Result<Vec<MessageViewAttachment>, String>,
    ),
    /// Raw source loaded (for Source rendering mode).
    RawSourceLoaded(Result<String, String>),
    /// User changed the rendering mode toggle.
    SetRenderingMode(RenderingMode),
    /// Reply/Reply All/Forward button pressed.
    Reply,
    ReplyAll,
    Forward,
    /// Overflow menu actions.
    Archive,
    Delete,
    Print,
    SaveAs,
    /// Open or close the message action context menu.
    OpenContextMenu,
    CloseContextMenu,
    /// Load remote content in Original HTML mode.
    LoadRemoteContent,
    /// User clicked Open on an attachment card. The string is the
    /// attachment id. Stubbed: opens with the system default handler.
    OpenAttachment(String),
    /// User clicked Save on an attachment card. The string is the
    /// attachment id. Stubbed: saves with file picker.
    SaveAttachment(String),
    /// User clicked Save All in the attachments panel header. Stubbed:
    /// saves every attachment with a folder picker.
    SaveAllAttachments,
    /// Cursor entered or exited an attachment card. `Some(id)` on enter,
    /// `None` on exit.
    HoverAttachment(Option<String>),
    /// No-op (placeholder for unimplemented actions).
    Noop,
}

// ── State ──────────────────────────────────────────────

/// Per-window state for a message view pop-out.
#[derive(Debug, Clone)]
pub struct MessageViewState {
    // ── Identity ──
    pub message_id: String,
    pub thread_id: String,
    pub account_id: String,

    // ── Header data ──
    pub from_name: Option<String>,
    pub from_address: Option<String>,
    pub to_addresses: Option<String>,
    pub cc_addresses: Option<String>,
    pub subject: Option<String>,
    pub date: Option<i64>,

    // ── Body ──
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    /// Snippet fallback used before async body load completes.
    pub snippet: Option<String>,
    /// Raw email source (headers + MIME), loaded lazily on Source mode.
    pub raw_source: Option<String>,

    // ── Attachments ──
    pub attachments: Vec<MessageViewAttachment>,
    /// Attachment id currently under the cursor, if any. Drives the
    /// hover-overlay (Save / Open buttons) on the compact attachment cards.
    pub hovered_attachment_id: Option<String>,

    // ── Window-local state ──
    pub rendering_mode: RenderingMode,
    pub context_menu_open: bool,
    pub remote_content_loaded: bool,
    pub error_banner: Option<String>,

    // ── Action context ──
    /// The sidebar selection when the pop-out was opened. Used for action
    /// resolution (spam toggle direction, trash undo source folder) so the
    /// pop-out resolves against its original context, not whatever the main
    /// window happens to show later.
    pub source_selection: Option<types::SidebarSelection>,

    // ── Window geometry ──
    pub width: f32,
    pub height: f32,
    pub x: Option<f32>,
    pub y: Option<f32>,

    // ── Generation tracking ──
    /// Per-window generation token to guard against stale data loads.
    pub generation: rtsk::generation::GenerationToken<rtsk::generation::PopOut>,
}

impl MessageViewState {
    pub fn from_thread_message(
        msg: &ThreadMessage,
        generation: rtsk::generation::GenerationToken<rtsk::generation::PopOut>,
        source_selection: Option<types::SidebarSelection>,
        default_rendering_mode: RenderingMode,
    ) -> Self {
        Self {
            message_id: msg.id.clone(),
            thread_id: msg.thread_id.clone(),
            account_id: msg.account_id.clone(),
            from_name: msg.from_name.clone(),
            from_address: msg.from_address.clone(),
            to_addresses: msg.to_addresses.clone(),
            cc_addresses: msg.cc_addresses.clone(),
            subject: msg.subject.clone(),
            date: msg.date,
            body_text: None,
            body_html: None,
            snippet: msg.snippet.clone(),
            raw_source: None,
            attachments: Vec::new(),
            hovered_attachment_id: None,
            rendering_mode: default_rendering_mode,

            context_menu_open: false,
            remote_content_loaded: false,
            error_banner: None,
            source_selection,
            width: MESSAGE_VIEW_DEFAULT_WIDTH,
            height: MESSAGE_VIEW_DEFAULT_HEIGHT,
            x: None,
            y: None,
            generation,
        }
    }

    /// Construct state from a session restore entry (minimal data, body loaded async).
    pub fn from_session_entry(
        entry: &MessageViewSessionEntry,
        generation: rtsk::generation::GenerationToken<rtsk::generation::PopOut>,
        default_rendering_mode: RenderingMode,
    ) -> Self {
        Self {
            message_id: entry.message_id.clone(),
            thread_id: entry.thread_id.clone(),
            account_id: entry.account_id.clone(),
            from_name: None,
            from_address: None,
            to_addresses: None,
            cc_addresses: None,
            subject: None,
            date: None,
            body_text: None,
            body_html: None,
            snippet: None,
            raw_source: None,
            attachments: Vec::new(),
            hovered_attachment_id: None,
            rendering_mode: default_rendering_mode,

            context_menu_open: false,
            remote_content_loaded: false,
            error_banner: None,
            source_selection: None, // not available on session restore
            width: entry.width,
            height: entry.height,
            x: entry.x,
            y: entry.y,
            generation,
        }
    }

    /// Check if a loaded result matches this window's current generation.
    pub fn is_current_generation(
        &self,
        token: rtsk::generation::GenerationToken<rtsk::generation::PopOut>,
    ) -> bool {
        self.generation == token
    }
}

// ── View ───────────────────────────────────────────────

/// Render the message view window content.
pub fn view_message_window<'a>(
    window_id: iced::window::Id,
    state: &'a MessageViewState,
    bimi_cache: &rtsk::bimi::BimiLruCache,
) -> Element<'a, Message> {
    let header = message_view_header(window_id, state, bimi_cache);
    let body = message_view_body(window_id, state);

    // Header stays pinned at the top (non-scrolling). The body card takes
    // the leftover vertical space and scrolls its own content; attachments
    // (when present) pin to the bottom below it. Rendering-mode picker
    // lives inside the header's overflow context menu.
    let mut content = column![header]
        .spacing(SPACE_0)
        .width(Length::Fill)
        .height(Length::Fill);

    // Error banner (shown above body for failed loads / deleted messages)
    if let Some(ref error) = state.error_banner {
        content = content.push(error_banner_view(error));
    }

    // Remote content banner for Original HTML mode
    if state.rendering_mode == RenderingMode::OriginalHtml && !state.remote_content_loaded {
        content = content.push(remote_content_banner(window_id));
    }

    content = content.push(body);

    if !state.attachments.is_empty() {
        content = content.push(message_view_attachments(
            window_id,
            &state.attachments,
            state.hovered_attachment_id.as_deref(),
        ));
    }

    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::ContainerClass::Content.style())
        .into()
}

// ── Header ─────────────────────────────────────────────

fn message_view_header<'a>(
    window_id: iced::window::Id,
    state: &'a MessageViewState,
    bimi_cache: &rtsk::bimi::BimiLruCache,
) -> Element<'a, Message> {
    let sender_name = state
        .from_name
        .as_deref()
        .or(state.from_address.as_deref())
        .unwrap_or("(unknown)");
    let sender_email = state.from_address.as_deref().unwrap_or("");

    // Action buttons + overflow menu (right-aligned on sender name row)
    let actions = action_buttons_with_overflow(window_id, state);

    // Sender avatar: BIMI logo if cached for the sender's domain, else
    // initials. Same lookup pattern as the thread list.
    let bimi_logo = state
        .from_address
        .as_deref()
        .and_then(|addr| addr.rsplit('@').next())
        .and_then(|domain| match bimi_cache.get(domain) {
            Some(Some(path)) if path.exists() => Some(path),
            _ => None,
        });
    let avatar =
        widgets::sender_avatar(sender_name, bimi_logo.as_deref(), AVATAR_MESSAGE_CARD);

    // From row: avatar + (name + email) + actions
    let from_row = row![
        avatar,
        column![
            text(sender_name)
                .size(TEXT_LG)
                .font(font::text_semibold())
                .style(text::base),
            text(sender_email)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        ]
        .spacing(SPACE_XXXS)
        .width(Length::Fill),
        actions,
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Start);

    // To row
    let to_text = state.to_addresses.as_deref().unwrap_or("");
    let mut header_fields = column![from_row].spacing(SPACE_XS);

    if !to_text.is_empty() {
        header_fields = header_fields.push(
            row![
                text("To: ")
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
                text(to_text).size(TEXT_SM).style(text::secondary),
            ]
            .spacing(SPACE_XXS),
        );
    }

    // Cc row (if present)
    if let Some(ref cc) = state.cc_addresses {
        if !cc.is_empty() {
            header_fields = header_fields.push(
                row![
                    text("Cc: ")
                        .size(TEXT_SM)
                        .style(theme::TextClass::Tertiary.style()),
                    text(cc.as_str()).size(TEXT_SM).style(text::secondary),
                ]
                .spacing(SPACE_XXS),
            );
        }
    }

    // Subject + date row
    let date_str = format_date(state.date).trim().to_string();
    let subject = state.subject.as_deref().unwrap_or("(no subject)");
    let subject_row = row![
        text(subject)
            .size(TEXT_HEADING)
            .style(text::base)
            .width(Length::Fill),
        container(
            text(date_str)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        )
        .padding(Padding {
            right: PAD_ICON_BTN.right,
            ..Padding::ZERO
        })
        .align_y(Alignment::Center),
    ]
    .align_y(Alignment::Center);

    header_fields = header_fields.push(subject_row);

    container(header_fields)
        .padding(Padding {
            top: PAD_CONTENT.top,
            right: SPACE_SM,
            bottom: SPACE_XS,
            left: SPACE_SM,
        })
        .width(Length::Fill)
        .into()
}

// ── Action buttons with overflow menu ──────────────────

fn action_buttons_with_overflow<'a>(
    window_id: iced::window::Id,
    state: &'a MessageViewState,
) -> Element<'a, Message> {
    let reply_btn = widgets::action_icon_button(
        icon::reply(),
        "Reply",
        Message::PopOut(
            window_id,
            PopOutMessage::MessageView(MessageViewMessage::Reply),
        ),
    );
    let reply_all_btn = widgets::action_icon_button(
        icon::reply_all(),
        "Reply All",
        Message::PopOut(
            window_id,
            PopOutMessage::MessageView(MessageViewMessage::ReplyAll),
        ),
    );
    let forward_btn = widgets::action_icon_button(
        icon::forward(),
        "Forward",
        Message::PopOut(
            window_id,
            PopOutMessage::MessageView(MessageViewMessage::Forward),
        ),
    );
    let overflow =
        overflow_context_menu(state.context_menu_open, state.rendering_mode, window_id);

    row![reply_btn, reply_all_btn, forward_btn, overflow]
        .spacing(SPACE_XXS)
        .into()
}

// ── Overflow menu ──────────────────────────────────────

fn overflow_context_menu<'a>(
    open: bool,
    current_mode: RenderingMode,
    window_id: iced::window::Id,
) -> Element<'a, Message> {
    // Wrap the icon in a row with a zero-width TEXT_SM text alongside it so
    // the trigger's content has the same line-height extent as
    // `action_icon_button` (which packs ICON_MD + TEXT_SM label). Without
    // this, the hamburger renders shorter than Reply/Reply All/Forward.
    let trigger = button(
        row![
            icon::ellipsis_vertical()
                .size(ICON_MD)
                .style(text::secondary),
            text("").size(TEXT_SM),
        ]
        .align_y(Alignment::Center),
    )
    .on_press(Message::PopOut(
        window_id,
        PopOutMessage::MessageView(MessageViewMessage::OpenContextMenu),
    ))
    .padding(PAD_ICON_BTN)
    .style(theme::ButtonClass::BareIcon.style());

    if !open {
        return trigger.into();
    }

    let modes = [
        (RenderingMode::PlainText, "Plain Text"),
        (RenderingMode::SimpleHtml, "Simple HTML"),
        (RenderingMode::OriginalHtml, "Original HTML"),
        (RenderingMode::Source, "Source"),
    ];

    let mut menu_items = column![
        context_menu_item(
            icon::archive(),
            "Archive",
            Message::PopOut(
                window_id,
                PopOutMessage::MessageView(MessageViewMessage::Archive),
            ),
        ),
        context_menu_item(
            icon::trash(),
            "Delete",
            Message::PopOut(
                window_id,
                PopOutMessage::MessageView(MessageViewMessage::Delete),
            ),
        ),
        context_menu_item(
            icon::printer(),
            "Print",
            Message::PopOut(
                window_id,
                PopOutMessage::MessageView(MessageViewMessage::Print),
            ),
        ),
        context_menu_item(
            icon::download(),
            "Save As",
            Message::PopOut(
                window_id,
                PopOutMessage::MessageView(MessageViewMessage::SaveAs),
            ),
        ),
        widgets::divider(),
    ]
    .spacing(SPACE_XXS);

    for (mode, label) in modes {
        menu_items = menu_items.push(context_menu_radio_item(
            mode == current_mode,
            label,
            Message::PopOut(
                window_id,
                PopOutMessage::MessageView(MessageViewMessage::SetRenderingMode(mode)),
            ),
        ));
    }

    let menu = container(menu_items)
        .padding(PAD_DROPDOWN)
        .style(theme::ContainerClass::SelectMenu.style());

    crate::ui::anchored_overlay::anchored_overlay(trigger)
        .popup(menu)
        .popup_width(SIDEBAR_MIN_WIDTH)
        .position(crate::ui::anchored_overlay::AnchorPosition::BelowRight)
        .on_dismiss(Message::PopOut(
            window_id,
            PopOutMessage::MessageView(MessageViewMessage::CloseContextMenu),
        ))
        .into()
}

fn context_menu_item<'a>(
    ico: iced::widget::Text<'a>,
    label: &'a str,
    on_press: Message,
) -> Element<'a, Message> {
    button(
        row![
            container(ico.size(ICON_MD).style(text::secondary))
                .width(SLOT_DROPDOWN)
                .height(SLOT_DROPDOWN)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center),
            container(text(label).size(TEXT_MD).style(text::base)).align_y(Alignment::Center),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .on_press(on_press)
    .padding(PAD_NAV_ITEM)
    .height(DROPDOWN_ITEM_HEIGHT)
    .style(theme::ButtonClass::Dropdown { selected: false }.style())
    .width(Length::Fill)
    .into()
}

/// Variant of `context_menu_item` that shows a radio circle in the leading
/// slot instead of an icon. Used for single-select options inside a
/// dropdown (e.g. the rendering-mode picker in the overflow menu).
fn context_menu_radio_item<'a>(
    selected: bool,
    label: &'a str,
    on_press: Message,
) -> Element<'a, Message> {
    button(
        row![
            container(widgets::radio_circle(selected))
                .width(SLOT_DROPDOWN)
                .height(SLOT_DROPDOWN)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center),
            container(text(label).size(TEXT_MD).style(text::base)).align_y(Alignment::Center),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .on_press(on_press)
    .padding(PAD_NAV_ITEM)
    .height(DROPDOWN_ITEM_HEIGHT)
    .style(theme::ButtonClass::Dropdown { selected: false }.style())
    .width(Length::Fill)
    .into()
}


// ── Body ───────────────────────────────────────────────

fn message_view_body<'a>(
    _window_id: iced::window::Id,
    state: &'a MessageViewState,
) -> Element<'a, Message> {
    let body_content: Element<'_, Message> = match state.rendering_mode {
        RenderingMode::PlainText => {
            let txt = state
                .body_text
                .as_deref()
                .or(state.snippet.as_deref())
                .unwrap_or("(no content)");
            text(txt)
                .size(TEXT_LG)
                .style(text::secondary)
                .width(Length::Fill)
                .into()
        }
        RenderingMode::SimpleHtml | RenderingMode::OriginalHtml => {
            // Phase 3 placeholder: fall back to plain text until HTML pipeline exists
            let txt = state
                .body_text
                .as_deref()
                .or(state.snippet.as_deref())
                .unwrap_or("(no content)");
            text(txt)
                .size(TEXT_LG)
                .style(text::secondary)
                .width(Length::Fill)
                .into()
        }
        RenderingMode::Source => {
            let src = state.raw_source.as_deref().unwrap_or("Loading source...");
            text(src)
                .size(TEXT_SM)
                .font(font::monospace())
                .style(text::secondary)
                .width(Length::Fill)
                .into()
        }
    };

    // Inner scrollable wraps just the text so the body card itself can fill
    // the leftover window height while overflowing text scrolls in place.
    let scrollable_text = scrollable(body_content)
        .spacing(SCROLLBAR_SPACING)
        .width(Length::Fill)
        .height(Length::Fill);

    let body_inset = container(scrollable_text)
        .padding(PAD_CONTENT)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::ContainerClass::EmailBody.style());

    // Outer wrapper hugs the window edges more tightly than the header
    // (12px vs the header's 24px), matching the compose pop-out so the
    // body card visually owns the full content area.
    container(body_inset)
        .padding(Padding {
            top: 0.0,
            right: SPACE_SM,
            bottom: SPACE_XS,
            left: SPACE_SM,
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

// ── Error banner ───────────────────────────────────────

fn error_banner_view<'a>(error: &'a str) -> Element<'a, Message> {
    container(
        row![
            icon::alert_triangle().size(ICON_MD).style(text::secondary),
            text(error)
                .size(TEXT_MD)
                .style(theme::TextClass::Tertiary.style()),
        ]
        .spacing(SPACE_SM)
        .align_y(Alignment::Center),
    )
    .padding(PAD_CARD)
    .style(theme::ContainerClass::Elevated.style())
    .width(Length::Fill)
    .into()
}

// ── Remote content banner ──────────────────────────────

fn remote_content_banner<'a>(window_id: iced::window::Id) -> Element<'a, Message> {
    container(
        row![
            text("Remote content is blocked.")
                .size(TEXT_SM)
                .style(text::secondary),
            button(
                text("Load for this message")
                    .size(TEXT_SM)
                    .style(theme::TextClass::Accent.style()),
            )
            .on_press(Message::PopOut(
                window_id,
                PopOutMessage::MessageView(MessageViewMessage::LoadRemoteContent,),
            ))
            .padding(PAD_ICON_BTN)
            .style(theme::ButtonClass::Ghost.style()),
        ]
        .spacing(SPACE_SM)
        .align_y(Alignment::Center),
    )
    .padding(PAD_CARD)
    .style(theme::ContainerClass::Elevated.style())
    .width(Length::Fill)
    .into()
}

// ── Attachments ────────────────────────────────────────

fn message_view_attachments<'a>(
    window_id: iced::window::Id,
    attachments: &'a [MessageViewAttachment],
    hovered_id: Option<&str>,
) -> Element<'a, Message> {
    // ── Panel header: "Attachments (N)" left + Save All right ──
    let title = text(format!("Attachments ({})", attachments.len()))
        .size(TEXT_MD)
        .font(font::text_semibold())
        .style(text::base);

    let save_all_btn = widgets::action_icon_button(
        icon::download(),
        "Save All",
        Message::PopOut(
            window_id,
            PopOutMessage::MessageView(MessageViewMessage::SaveAllAttachments),
        ),
    );

    let header_row = row![
        container(title)
            .width(Length::Fill)
            .align_y(Alignment::Center),
        save_all_btn,
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    // ── Wrapping row of compact cards ──
    let mut cards = row![].spacing(SPACE_XS);
    for att in attachments {
        let is_hovered = hovered_id == Some(att.id.as_str());
        cards = cards.push(compact_attachment_card(window_id, att, is_hovered));
    }
    let wrapping_cards = cards.wrap().vertical_spacing(SPACE_XS);

    let scroll_area = scrollable(container(wrapping_cards).padding(Padding {
        top: 0.0,
        right: SCROLLBAR_SPACING,
        bottom: 0.0,
        left: 0.0,
    }))
    .spacing(SCROLLBAR_SPACING)
    .width(Length::Fill)
    .height(Length::Shrink);

    let panel_inner = column![header_row, scroll_area].spacing(SPACE_XS);

    let panel_padding = Padding {
        top: SPACE_XXS,
        right: SPACE_SM,
        bottom: PAD_CONTENT.bottom,
        left: SPACE_SM,
    };

    container(panel_inner)
        .padding(panel_padding)
        .width(Length::Fill)
        .max_height(
            POPOUT_ATTACHMENT_PANEL_INNER_HEIGHT
                + panel_padding.top
                + panel_padding.bottom
                + 36.0
                + SPACE_XS,
        )
        .into()
}

/// Compact attachment pill: `[icon] filename size` at rest. On hover, a
/// semi-transparent overlay (left half = Save, right half = Open) covers
/// the pill. Width is content-driven so cards size to their filename.
fn compact_attachment_card<'a>(
    window_id: iced::window::Id,
    att: &'a MessageViewAttachment,
    is_hovered: bool,
) -> Element<'a, Message> {
    let filename = att.filename.as_deref().unwrap_or("(unnamed)");
    let file_ico = file_type_icon(att.mime_type.as_deref());
    let size_str = format_file_size(att.size);

    // ── At-rest pill content ──
    let pill_row = row![
        container(file_ico.size(ICON_MD).style(text::secondary))
            .align_y(Alignment::Center),
        container(text(filename).size(TEXT_MD).style(text::base))
            .align_y(Alignment::Center),
        container(
            text(size_str)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        )
        .align_y(Alignment::Center),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    let pill: Element<'a, Message> = container(pill_row)
        .padding(Padding {
            top: 0.0,
            right: SPACE_SM,
            bottom: 0.0,
            left: SPACE_SM,
        })
        .height(Length::Fixed(POPOUT_ATTACHMENT_CARD_HEIGHT))
        .align_y(Alignment::Center)
        .style(theme::ContainerClass::EmailBody.style())
        .into();

    let id_for_enter = att.id.clone();

    let inner: Element<'a, Message> = if is_hovered {
        let save_btn = button(
            container(
                row![
                    icon::download().size(ICON_MD).style(text::base),
                    text("Save").size(TEXT_SM).style(text::base),
                ]
                .spacing(SPACE_XS)
                .align_y(Alignment::Center),
            )
            .center(Length::Fill),
        )
        .on_press(Message::PopOut(
            window_id,
            PopOutMessage::MessageView(MessageViewMessage::SaveAttachment(att.id.clone())),
        ))
        .style(|theme, status| attachment_overlay_button_style(theme, status, true))
        .width(Length::Fill)
        .height(Length::Fill);

        let open_btn = button(
            container(
                row![
                    icon::external_link().size(ICON_MD).style(text::base),
                    text("Open").size(TEXT_SM).style(text::base),
                ]
                .spacing(SPACE_XS)
                .align_y(Alignment::Center),
            )
            .center(Length::Fill),
        )
        .on_press(Message::PopOut(
            window_id,
            PopOutMessage::MessageView(MessageViewMessage::OpenAttachment(att.id.clone())),
        ))
        .style(|theme, status| attachment_overlay_button_style(theme, status, false))
        .width(Length::Fill)
        .height(Length::Fill);

        let overlay = container(
            row![save_btn, open_btn]
                .spacing(0.0)
                .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill);

        stack![pill, overlay].into()
    } else {
        pill
    };

    mouse_area(inner)
        .on_enter(Message::PopOut(
            window_id,
            PopOutMessage::MessageView(MessageViewMessage::HoverAttachment(Some(id_for_enter))),
        ))
        .on_exit(Message::PopOut(
            window_id,
            PopOutMessage::MessageView(MessageViewMessage::HoverAttachment(None)),
        ))
        .into()
}

/// Opaque button style for the hover-overlay action buttons on compact
/// attachment cards. The two buttons share the pill's rounded corners:
/// the left button rounds its left side, the right button rounds its
/// right side. The shared inner edge is square so the two buttons meet
/// flush.
fn attachment_overlay_button_style(
    theme: &iced::Theme,
    status: button::Status,
    is_left: bool,
) -> button::Style {
    let palette = theme.palette();
    // Resting bg uses the strong-contrast hover color one palette step
    // brighter than the panel base. On direct hover the bg shifts another
    // step so the half being clicked stands out.
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => palette.background.strong.color,
        _ => palette.background.weak.color,
    };
    let radius = if is_left {
        iced::border::Radius::default()
            .top_left(RADIUS_MD)
            .bottom_left(RADIUS_MD)
    } else {
        iced::border::Radius::default()
            .top_right(RADIUS_MD)
            .bottom_right(RADIUS_MD)
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: palette.background.base.text,
        border: iced::Border {
            radius,
            ..iced::Border::default()
        },
        shadow: iced::Shadow::default(),
        ..button::Style::default()
    }
}

// ── Helpers ────────────────────────────────────────────

fn format_date(timestamp: Option<i64>) -> String {
    timestamp
        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
        .map(|dt| dt.format("%a, %b %d, %Y, %l:%M %p").to_string())
        .unwrap_or_default()
}

fn file_type_icon<'a>(mime_type: Option<&str>) -> iced::widget::Text<'a> {
    match mime_type.unwrap_or("") {
        t if t.starts_with("image/") => icon::image(),
        t if t.contains("pdf") => icon::file_text(),
        t if t.contains("spreadsheet") || t.contains("excel") => icon::file_spreadsheet(),
        _ => icon::file(),
    }
}

fn mime_to_type_label(mime: Option<&str>) -> &'static str {
    match mime.unwrap_or("") {
        t if t.starts_with("image/") => "Image",
        t if t.contains("pdf") => "PDF",
        t if t.contains("spreadsheet") || t.contains("excel") => "Excel",
        t if t.contains("word") || t.contains("document") => "Word",
        t if t.contains("zip") || t.contains("archive") => "Archive",
        _ => "File",
    }
}

fn format_file_size(size: Option<i64>) -> String {
    match size {
        None => "\u{2014}".to_string(),
        Some(b) if b < 1024 => format!("{b} B"),
        Some(b) if b < 1024 * 1024 => {
            format!("{:.0} KB", b as f64 / 1024.0)
        }
        Some(b) => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}
