//! Rich text editor widget for iced.
//!
//! Custom `Widget` trait implementation. Renders blocks as `Paragraph::with_spans`,
//! handles keyboard/mouse/IME input, cursor blink, and selection.
//!
//! # Usage
//!
//! ```ignore
//! use ratatoskr_rich_text_editor::widget::{EditorState, RichTextEditor, Action};
//!
//! struct State {
//!     editor: EditorState,
//! }
//!
//! #[derive(Debug, Clone)]
//! enum Message {
//!     EditorAction(Action),
//! }
//!
//! fn view(state: &State) -> Element<'_, Message> {
//!     rich_text_editor(&state.editor)
//!         .on_action(Message::EditorAction)
//!         .into()
//! }
//!
//! fn update(state: &mut State, message: Message) {
//!     match message {
//!         Message::EditorAction(action) => {
//!             state.editor.perform(action);
//!         }
//!     }
//! }
//! ```

pub mod cursor;
mod editor_state;
pub mod input;
pub mod render;

pub use editor_state::{Action, EditorState};

use crate::document::Block;

use cursor::{
    BlockSelectionKind, SelectionRect, CURSOR_WIDTH, SELECTION_ALPHA,
};
use input::KeyAction;
use render::ParagraphCache;

use iced::advanced::layout;
use iced::advanced::mouse::click::Click;
use iced::advanced::renderer;
use iced::advanced::renderer::Renderer as _;
use iced::advanced::text::Paragraph as _;
use iced::advanced::text::Renderer as TextRenderer;
use iced::advanced::widget::{self, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::keyboard;
use iced::mouse;
use iced::time::{Duration, Instant};
use iced::window;
use iced::{Color, Element, Event, Font, Length, Padding, Point, Rectangle, Size, Vector};

/// The paragraph type used by the iced Renderer.
type IcedParagraph = <iced::Renderer as TextRenderer>::Paragraph;

// ── Widget tree state ───────────────────────────────────

/// Internal widget state stored in the iced widget tree. Holds the paragraph
/// cache and focus/blink timing.
struct WidgetState {
    /// Paragraph cache: one entry per document block.
    cache: ParagraphCache<IcedParagraph>,
    /// Focus tracking for cursor blink.
    focus: Option<FocusState>,
    /// Last mouse click for double/triple click detection.
    last_click: Option<Click>,
    /// Whether a drag is active.
    dragging: bool,
    /// Vertical scroll offset in pixels. 0.0 means the top of the document
    /// is aligned with the top of the viewport.
    scroll_offset: f32,
}

/// Focus timing state for cursor blink.
#[derive(Debug, Clone)]
struct FocusState {
    updated_at: Instant,
    now: Instant,
    is_window_focused: bool,
}

impl FocusState {
    const BLINK_INTERVAL_MILLIS: u128 = 500;

    fn now() -> Self {
        let now = Instant::now();
        Self {
            updated_at: now,
            now,
            is_window_focused: true,
        }
    }

    fn is_cursor_visible(&self) -> bool {
        self.is_window_focused
            && ((self.now - self.updated_at).as_millis() / Self::BLINK_INTERVAL_MILLIS)
                .is_multiple_of(2)
    }
}

// ── RichTextEditor widget ───────────────────────────────

/// A rich text editor widget for iced.
///
/// Created via the [`rich_text_editor`] function. Renders a [`Document`] with
/// styled text, handles keyboard and mouse input, and emits [`Action`]s to the
/// application.
pub struct RichTextEditor<'a, Message> {
    state: &'a EditorState,
    on_action: Option<Box<dyn Fn(Action) -> Message + 'a>>,
    font: Font,
    text_color: Color,
    link_color: Color,
    cursor_color: Color,
    selection_color: Color,
    padding: Padding,
    width: Length,
    height: Length,
}

/// Create a [`RichTextEditor`] widget for the given state.
pub fn rich_text_editor<'a, Message>(state: &'a EditorState) -> RichTextEditor<'a, Message> {
    RichTextEditor::new(state)
}

impl<'a, Message> RichTextEditor<'a, Message> {
    /// Create a new rich text editor widget.
    pub fn new(state: &'a EditorState) -> Self {
        Self {
            state,
            on_action: None,
            font: Font::DEFAULT,
            text_color: Color::BLACK,
            link_color: Color::from_rgb(0.2, 0.4, 0.8),
            cursor_color: Color::BLACK,
            selection_color: Color::from_rgba(0.2, 0.4, 0.8, SELECTION_ALPHA),
            padding: Padding::new(8.0),
            width: Length::Fill,
            height: Length::Shrink,
        }
    }

    /// Set the callback for when an action occurs.
    ///
    /// If not set, the editor is disabled (read-only).
    pub fn on_action(mut self, f: impl Fn(Action) -> Message + 'a) -> Self {
        self.on_action = Some(Box::new(f));
        self
    }

    /// Set the base font.
    pub fn font(mut self, font: Font) -> Self {
        self.font = font;
        self
    }

    /// Set the text color.
    pub fn text_color(mut self, color: Color) -> Self {
        self.text_color = color;
        self
    }

    /// Set the link color.
    pub fn link_color(mut self, color: Color) -> Self {
        self.link_color = color;
        self
    }

    /// Set the cursor (caret) color.
    pub fn cursor_color(mut self, color: Color) -> Self {
        self.cursor_color = color;
        self
    }

    /// Set the selection highlight color.
    pub fn selection_color(mut self, color: Color) -> Self {
        self.selection_color = color;
        self
    }

    /// Set the padding.
    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.padding = padding.into();
        self
    }

    /// Set the width.
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Set the height.
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }
}

// ── Event handling helpers ───────────────────────────────

impl<Message> RichTextEditor<'_, Message> {
    /// Handle window focus/unfocus and redraw-requested events for cursor blink.
    fn handle_window_events(
        widget_state: &mut WidgetState,
        event: &Event,
        shell: &mut Shell<'_, Message>,
    ) {
        match event {
            Event::Window(window::Event::Unfocused) => {
                if let Some(focus) = &mut widget_state.focus {
                    focus.is_window_focused = false;
                }
            }
            Event::Window(window::Event::Focused) => {
                if let Some(focus) = &mut widget_state.focus {
                    focus.is_window_focused = true;
                    focus.updated_at = Instant::now();
                    shell.request_redraw();
                }
            }
            Event::Window(window::Event::RedrawRequested(now)) => {
                if let Some(focus) = &mut widget_state.focus
                    && focus.is_window_focused
                {
                    focus.now = *now;

                    let elapsed = (focus.now - focus.updated_at).as_millis()
                        % FocusState::BLINK_INTERVAL_MILLIS;
                    let millis_until_redraw =
                        FocusState::BLINK_INTERVAL_MILLIS.saturating_sub(elapsed);

                    shell.request_redraw_at(
                        focus.now
                            + Duration::from_millis(
                                u64::try_from(millis_until_redraw).unwrap_or(500),
                            ),
                    );
                }
            }
            _ => {}
        }
    }

    /// Handle keyboard events (key presses) when the editor is focused.
    fn handle_keyboard(
        &self,
        widget_state: &mut WidgetState,
        event: &Event,
        on_action: &dyn Fn(Action) -> Message,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        if let Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            modifiers,
            text,
            ..
        }) = event
            && widget_state.focus.is_some()
        {
                let text_str = text.as_deref();
                let key_action = input::map_key_event(key, *modifiers, text_str);

                match key_action {
                    KeyAction::Edit(edit_action) => {
                        shell.publish(on_action(Action::Edit(edit_action)));
                        shell.capture_event();
                    }
                    KeyAction::Move(move_action) => {
                        shell.publish(on_action(Action::Move(move_action)));
                        shell.capture_event();
                    }
                    KeyAction::Select(move_action) => {
                        shell.publish(on_action(Action::Select(move_action)));
                        shell.capture_event();
                    }
                    KeyAction::SelectAll => {
                        shell.publish(on_action(Action::SelectAll));
                        shell.capture_event();
                    }
                    KeyAction::Copy => {
                        let text = self.state.selection_text();
                        if !text.is_empty() {
                            clipboard.write(iced::advanced::clipboard::Kind::Standard, text);
                        }
                        // Emit Action::Copy so EditorState captures the structured slice.
                        shell.publish(on_action(Action::Copy));
                        shell.capture_event();
                    }
                    KeyAction::Cut => {
                        let text = self.state.selection_text();
                        if !text.is_empty() {
                            clipboard.write(iced::advanced::clipboard::Kind::Standard, text.clone());
                            // Emit Action::Cut so EditorState captures the structured
                            // slice and then deletes the selection.
                            shell.publish(on_action(Action::Cut));
                        }
                        shell.capture_event();
                    }
                    KeyAction::Paste => {
                        if let Some(contents) =
                            clipboard.read(iced::advanced::clipboard::Kind::Standard)
                        {
                            shell.publish(on_action(Action::Paste(contents)));
                        }
                        shell.capture_event();
                    }
                    KeyAction::Undo => {
                        shell.publish(on_action(Action::Undo));
                        shell.capture_event();
                    }
                    KeyAction::Redo => {
                        shell.publish(on_action(Action::Redo));
                        shell.capture_event();
                    }
                    KeyAction::None => {}
                }

                // Reset blink on any handled key.
                if let Some(focus) = &mut widget_state.focus {
                    focus.updated_at = Instant::now();
                }
        }
    }

    /// Handle a drag move: auto-scroll when above/below the viewport,
    /// normal drag within bounds.
    #[allow(clippy::too_many_arguments)]
    fn handle_drag(
        &self,
        widget_state: &mut WidgetState,
        position: Point,
        bounds: Rectangle,
        cursor_pos: mouse::Cursor,
        on_action: &dyn Fn(Action) -> Message,
        shell: &mut Shell<'_, Message>,
    ) {
        if position.y < bounds.y {
            // Cursor is above the viewport — scroll up.
            let overshoot = bounds.y - position.y;
            let scroll_amount = (overshoot * DRAG_SCROLL_SPEED).min(DRAG_SCROLL_MAX);
            widget_state.scroll_offset =
                (widget_state.scroll_offset - scroll_amount).max(0.0);

            let content_pos = Point::new(position.x - bounds.x - self.padding.left, 0.0);
            let doc_pos =
                hit_test_content_point(content_pos, &widget_state.cache, &self.state.document);
            shell.publish(on_action(Action::Drag(doc_pos)));
            shell.request_redraw();
            shell.capture_event();
        } else if position.y > bounds.y + bounds.height {
            // Cursor is below the viewport — scroll down.
            let overshoot = position.y - (bounds.y + bounds.height);
            let scroll_amount = (overshoot * DRAG_SCROLL_SPEED).min(DRAG_SCROLL_MAX);
            let viewport_height = bounds.height - self.padding.top - self.padding.bottom;
            let total_height = widget_state.cache.total_height();
            let max_scroll = (total_height - viewport_height).max(0.0);
            widget_state.scroll_offset =
                (widget_state.scroll_offset + scroll_amount).min(max_scroll);

            let content_pos = Point::new(
                position.x - bounds.x - self.padding.left,
                widget_state.scroll_offset + viewport_height,
            );
            let doc_pos =
                hit_test_content_point(content_pos, &widget_state.cache, &self.state.document);
            shell.publish(on_action(Action::Drag(doc_pos)));
            shell.request_redraw();
            shell.capture_event();
        } else if let Some(rel_pos) = cursor_pos.position_in(bounds) {
            // Normal drag within bounds.
            let content_pos = Point::new(
                rel_pos.x - self.padding.left,
                rel_pos.y - self.padding.top + widget_state.scroll_offset,
            );
            let doc_pos =
                hit_test_content_point(content_pos, &widget_state.cache, &self.state.document);
            shell.publish(on_action(Action::Drag(doc_pos)));
        }
    }

    /// Handle mouse events (clicks, drags, scroll wheel).
    #[allow(clippy::too_many_arguments)]
    fn handle_mouse(
        &self,
        widget_state: &mut WidgetState,
        event: &Event,
        bounds: Rectangle,
        cursor_pos: mouse::Cursor,
        on_action: &dyn Fn(Action) -> Message,
        shell: &mut Shell<'_, Message>,
    ) {
        let scroll_offset = widget_state.scroll_offset;
        match event {
            Event::Mouse(mouse::Event::ButtonPressed { button: mouse::Button::Left, .. }) => {
                if let Some(position) = cursor_pos.position_in(bounds) {
                    // Translate to content coordinates (account for padding and scroll).
                    let content_pos = Point::new(
                        position.x - self.padding.left,
                        position.y - self.padding.top + scroll_offset,
                    );

                    let doc_pos = hit_test_content_point(
                        content_pos,
                        &widget_state.cache,
                        &self.state.document,
                    );

                    // Detect double/triple click using iced's Click type.
                    let click = Click::new(position, mouse::Button::Left, widget_state.last_click.take());
                    let click_kind = click.kind();
                    widget_state.last_click = Some(click);

                    widget_state.focus = Some(FocusState::now());

                    shell.publish(on_action(Action::Focus));

                    match click_kind {
                        iced::advanced::mouse::click::Kind::Triple => {
                            // Triple click: select entire block.
                            widget_state.dragging = false;
                            shell.publish(on_action(Action::TripleClick(doc_pos)));
                        }
                        iced::advanced::mouse::click::Kind::Double => {
                            // Double click: select word.
                            widget_state.dragging = false;
                            shell.publish(on_action(Action::DoubleClick(doc_pos)));
                        }
                        iced::advanced::mouse::click::Kind::Single => {
                            widget_state.dragging = true;
                            shell.publish(on_action(Action::Click(doc_pos)));
                        }
                    }

                    shell.capture_event();
                } else if widget_state.focus.is_some() {
                    // Click outside the editor: blur.
                    widget_state.focus = None;
                    widget_state.dragging = false;
                    shell.publish(on_action(Action::Blur));
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. })
                if widget_state.dragging
                    && let Some(position) = cursor_pos.position() =>
            {
                self.handle_drag(
                    widget_state, position, bounds, cursor_pos, on_action, shell,
                );
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta })
                if cursor_pos.is_over(bounds) =>
            {
                let delta_y = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => -y * SCROLL_LINE_HEIGHT,
                    mouse::ScrollDelta::Pixels { y, .. } => -y,
                };

                let total_height = widget_state.cache.total_height();
                let viewport_height =
                    bounds.height - self.padding.top - self.padding.bottom;
                let max_scroll = (total_height - viewport_height).max(0.0);

                widget_state.scroll_offset =
                    (widget_state.scroll_offset + delta_y).clamp(0.0, max_scroll);

                shell.capture_event();
                shell.request_redraw();
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                widget_state.dragging = false;
            }
            _ => {}
        }
    }
}

// ── Drawing helper ──────────────────────────────────────

impl<Message> RichTextEditor<'_, Message> {
    /// Draw all document content (blocks, selection, cursor) into the
    /// current renderer context. Called from `draw()` inside a layer +
    /// translation so scrolling and clipping are already applied.
    fn draw_content(
        &self,
        renderer: &mut iced::Renderer,
        cache: &ParagraphCache<IcedParagraph>,
        text_bounds: &Rectangle,
        cursor_visible: bool,
    ) {
        self.draw_blocks(renderer, cache, text_bounds);
        self.draw_selection(renderer, cache, text_bounds);
        self.draw_cursor(renderer, cache, text_bounds, cursor_visible);
    }

    /// Draw a single list item block: marker + paragraph content.
    #[allow(clippy::too_many_arguments)]
    fn draw_list_item(
        &self,
        renderer: &mut iced::Renderer,
        paragraph: &IcedParagraph,
        block_index: usize,
        ordered: bool,
        indent_level: u8,
        block_origin: Point,
        block_height: f32,
        text_bounds: &Rectangle,
    ) {
        let indent =
            render::LIST_MARKER_WIDTH + (indent_level as f32) * render::LIST_INDENT_PER_LEVEL;
        let content_origin = Point::new(block_origin.x + indent, block_origin.y);
        let content_bounds = Rectangle::new(
            content_origin,
            Size::new((text_bounds.width - indent).max(0.0), block_height),
        );
        // Count consecutive preceding ListItem blocks with the same
        // ordered flag and indent_level to determine the item index.
        let item_idx = {
            let mut idx = 0usize;
            for prev_i in (0..block_index).rev() {
                if let Some(Block::ListItem {
                    ordered: prev_ord,
                    indent_level: prev_indent,
                    ..
                }) = self.state.document.block(prev_i)
                {
                    if *prev_ord == ordered && *prev_indent == indent_level {
                        idx += 1;
                    } else if *prev_indent < indent_level {
                        break;
                    }
                } else {
                    break;
                }
            }
            idx
        };
        render::draw_list_marker(
            renderer,
            content_bounds,
            ordered,
            item_idx,
            self.font,
            self.text_color,
            *text_bounds,
        );
        render::draw_paragraph(
            renderer,
            paragraph,
            content_origin,
            self.text_color,
            *text_bounds,
        );
    }

    /// Draw each document block (paragraphs, headings, lists, etc.).
    fn draw_blocks(
        &self,
        renderer: &mut iced::Renderer,
        cache: &ParagraphCache<IcedParagraph>,
        text_bounds: &Rectangle,
    ) {
        for (i, block) in self.state.document.blocks.iter().enumerate() {
            let Some(entry) = cache.get(i) else {
                continue;
            };

            let block_origin = Point::new(
                text_bounds.x,
                text_bounds.y + entry.y_offset(),
            );

            match block.as_ref() {
                Block::HorizontalRule => {
                    let hr_bounds = Rectangle::new(
                        block_origin,
                        Size::new(text_bounds.width, entry.height()),
                    );
                    render::draw_horizontal_rule(renderer, hr_bounds, self.text_color);
                }
                Block::BlockQuote { .. } => {
                    let bq_bounds = Rectangle::new(
                        block_origin,
                        Size::new(text_bounds.width, entry.height()),
                    );
                    render::draw_blockquote_border(renderer, bq_bounds, self.text_color);

                    for child in entry.child_paragraphs() {
                        let para_origin = Point::new(
                            block_origin.x + render::BLOCKQUOTE_INDENT,
                            block_origin.y + child.local_y_offset,
                        );
                        render::draw_paragraph(
                            renderer,
                            &child.paragraph,
                            para_origin,
                            self.text_color,
                            *text_bounds,
                        );
                    }
                }
                Block::ListItem { ordered, indent_level, .. } => {
                    if let Some(paragraph) = entry.paragraph() {
                        self.draw_list_item(
                            renderer, paragraph, i, *ordered, *indent_level,
                            block_origin, entry.height(), text_bounds,
                        );
                    }
                }
                Block::Paragraph { .. } | Block::Heading { .. } => {
                    if let Some(paragraph) = entry.paragraph() {
                        render::draw_paragraph(
                            renderer,
                            paragraph,
                            block_origin,
                            self.text_color,
                            *text_bounds,
                        );
                    }
                }
                Block::Image { .. } => {
                    // Draw a placeholder rectangle with a 1px border.
                    let img_bounds = Rectangle::new(
                        block_origin,
                        Size::new(text_bounds.width, entry.height()),
                    );
                    let border_color = Color {
                        a: 0.3,
                        ..self.text_color
                    };
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: img_bounds,
                            border: iced::Border {
                                color: border_color,
                                width: 1.0,
                                radius: 2.0.into(),
                            },
                            ..Default::default()
                        },
                        Color::TRANSPARENT,
                    );
                    // Draw alt text inside the placeholder.
                    if let Some(paragraph) = entry.paragraph() {
                        let text_origin = Point::new(
                            block_origin.x + render::IMAGE_PLACEHOLDER_PADDING,
                            block_origin.y + render::IMAGE_PLACEHOLDER_PADDING,
                        );
                        let alt_color = Color {
                            a: 0.5,
                            ..self.text_color
                        };
                        render::draw_paragraph(
                            renderer,
                            paragraph,
                            text_origin,
                            alt_color,
                            *text_bounds,
                        );
                    }
                }
            }
        }
    }

    /// Draw selection highlights for the current selection range.
    fn draw_selection(
        &self,
        renderer: &mut iced::Renderer,
        cache: &ParagraphCache<IcedParagraph>,
        text_bounds: &Rectangle,
    ) {
        if self.state.selection.is_collapsed() {
            return;
        }

        let sel_ranges = cursor::selection_block_ranges(self.state.selection);

        for (block_idx, kind) in &sel_ranges {
            let Some(entry) = cache.get(*block_idx) else {
                continue;
            };

            let block_y = text_bounds.y + entry.y_offset();
            let content_x_off = self
                .state
                .document
                .block(*block_idx)
                .map(block_content_x_offset)
                .unwrap_or(0.0);
            let para_origin_x = text_bounds.x + content_x_off;

            let sel_rects: Vec<SelectionRect> = match (kind, entry.paragraph()) {
                (BlockSelectionKind::Full, _) => {
                    vec![SelectionRect {
                        x: text_bounds.x,
                        y: block_y,
                        width: text_bounds.width,
                        height: entry.height(),
                    }]
                }
                (
                    BlockSelectionKind::Single {
                        start_offset,
                        end_offset,
                    },
                    Some(paragraph),
                ) => compute_selection_rects(
                    paragraph,
                    *start_offset,
                    *end_offset,
                    para_origin_x,
                    block_y,
                    text_bounds.width,
                ),
                (BlockSelectionKind::First { start_offset }, Some(paragraph)) => {
                    compute_selection_rects(
                        paragraph,
                        *start_offset,
                        usize::MAX,
                        para_origin_x,
                        block_y,
                        text_bounds.width,
                    )
                }
                (BlockSelectionKind::Last { end_offset }, Some(paragraph)) => {
                    compute_selection_rects(
                        paragraph,
                        0,
                        *end_offset,
                        para_origin_x,
                        block_y,
                        text_bounds.width,
                    )
                }
                (_, None) => {
                    vec![SelectionRect {
                        x: text_bounds.x,
                        y: block_y,
                        width: text_bounds.width,
                        height: entry.height(),
                    }]
                }
            };

            for sel_rect in &sel_rects {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: Rectangle::new(
                            Point::new(sel_rect.x, sel_rect.y),
                            Size::new(sel_rect.width, sel_rect.height),
                        ),
                        ..Default::default()
                    },
                    self.selection_color,
                );
            }
        }
    }

    /// Draw the cursor caret (blink visibility is determined by the caller).
    fn draw_cursor(
        &self,
        renderer: &mut iced::Renderer,
        cache: &ParagraphCache<IcedParagraph>,
        text_bounds: &Rectangle,
        cursor_visible: bool,
    ) {
        if !cursor_visible || !self.state.selection.is_collapsed() {
            return;
        }

        let pos = self.state.selection.focus;
        let Some(entry) = cache.get(pos.block_index) else {
            return;
        };

        let content_x_off = self
            .state
            .document
            .block(pos.block_index)
            .map(block_content_x_offset)
            .unwrap_or(0.0);

        let para_origin_x = text_bounds.x + content_x_off;
        let para_origin_y = text_bounds.y + entry.y_offset();

        let (cursor_x, cursor_y, cursor_height) =
            if let Some(paragraph) = entry.paragraph() {
                let gp = grapheme_pixel_position(paragraph, pos.offset);
                let lh = paragraph_line_height_px(paragraph);
                (para_origin_x + gp.x, para_origin_y + gp.y, lh)
            } else {
                let lh = render::FONT_SIZE_BODY * render::LINE_HEIGHT_MULTIPLIER;
                (para_origin_x, para_origin_y, lh)
            };

        // Draw the cursor directly. We're inside with_layer(bounds)
        // which clips to the viewport, so no manual intersection
        // check is needed. (The previous check against text_bounds
        // used viewport-space coordinates inside a content-space
        // translation, causing the cursor to disappear when scrolled.)
        renderer.fill_quad(
            renderer::Quad {
                bounds: Rectangle::new(
                    Point::new(cursor_x, cursor_y),
                    Size::new(CURSOR_WIDTH, cursor_height),
                ),
                ..Default::default()
            },
            self.cursor_color,
        );
    }
}

// ── Widget trait implementation ─────────────────────────

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for RichTextEditor<'_, Message> {
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<WidgetState>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(WidgetState {
            cache: ParagraphCache::new(),
            focus: None,
            last_click: None,
            dragging: false,
            scroll_offset: 0.0,
        })
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let widget_state = tree.state.downcast_mut::<WidgetState>();
        let cache = &mut widget_state.cache;

        let limits = limits.width(self.width).height(self.height);
        let max_size = limits.max();
        let available_width = (max_size.width - self.padding.left - self.padding.right).max(0.0);

        // Layout paragraphs using the cache.
        let total_height = cache.layout(
            &self.state.document.blocks,
            available_width,
            self.font,
            self.text_color,
            self.link_color,
        );

        let content_height = total_height + self.padding.top + self.padding.bottom;

        let node = match self.height {
            Length::Fill | Length::FillPortion(_) | Length::Fixed(_) => {
                layout::Node::new(limits.max())
            }
            Length::Shrink => {
                let size = limits
                    .height(Length::Fixed(content_height))
                    .max();
                layout::Node::new(size)
            }
        };

        // Clamp scroll offset to valid range after layout.
        let viewport_height = node.size().height - self.padding.top - self.padding.bottom;
        let max_scroll = (total_height - viewport_height).max(0.0);
        widget_state.scroll_offset = widget_state.scroll_offset.clamp(0.0, max_scroll);

        // Auto-scroll to keep the cursor visible after edits/moves.
        // Use the actual caret line position within the block (not just
        // the block top) so scrolling works correctly in wrapped paragraphs.
        if self.state.cursor.is_focused() {
            let focus_pos = self.state.selection.focus;
            if let Some(entry) = cache.get(focus_pos.block_index) {
                let (caret_y_in_block, line_h) = if let Some(para) = entry.paragraph() {
                    let glyph_pos = grapheme_pixel_position(para, focus_pos.offset);
                    (glyph_pos.y, paragraph_line_height_px(para))
                } else {
                    (0.0, render::FONT_SIZE_BODY * render::LINE_HEIGHT_MULTIPLIER)
                };

                let cursor_y = entry.y_offset() + caret_y_in_block;
                ensure_cursor_visible(
                    &mut widget_state.scroll_offset,
                    cursor_y,
                    line_h,
                    viewport_height,
                    total_height,
                );
            }
        }

        node
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: iced::advanced::Layout<'_>,
        cursor_pos: mouse::Cursor,
        _renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let Some(on_action) = self.on_action.as_ref() else {
            return;
        };

        let widget_state = tree.state.downcast_mut::<WidgetState>();
        let bounds = layout.bounds();

        Self::handle_window_events(widget_state, event, shell);
        self.handle_keyboard(widget_state, event, on_action, clipboard, shell);
        self.handle_mouse(widget_state, event, bounds, cursor_pos, on_action, shell);
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: iced::advanced::Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let widget_state = tree.state.downcast_ref::<WidgetState>();
        let bounds = layout.bounds();
        let text_bounds = bounds.shrink(self.padding);
        let cache = &widget_state.cache;
        let scroll_offset = widget_state.scroll_offset;

        let cursor_visible = widget_state
            .focus
            .as_ref()
            .is_some_and(FocusState::is_cursor_visible);

        // Clip to the widget bounds and translate by -scroll_offset so
        // content scrolls upward as scroll_offset increases.
        renderer.with_layer(bounds, |renderer| {
            renderer.with_translation(
                Vector::new(0.0, -scroll_offset),
                |renderer| {
                    self.draw_content(renderer, cache, &text_bounds, cursor_visible);
                },
            );
        });
    }

    fn mouse_interaction(
        &self,
        _tree: &widget::Tree,
        layout: iced::advanced::Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let is_disabled = self.on_action.is_none();

        if cursor.is_over(layout.bounds()) {
            if is_disabled {
                mouse::Interaction::NotAllowed
            } else {
                mouse::Interaction::Text
            }
        } else {
            mouse::Interaction::default()
        }
    }
}

// ── Block content x-offset helper ────────────────────────

/// Returns the horizontal content offset for a block type. List and blockquote
/// blocks indent their paragraph content.
fn block_content_x_offset(block: &Block) -> f32 {
    match block {
        Block::ListItem { indent_level, .. } => {
            render::LIST_MARKER_WIDTH + (*indent_level as f32) * render::LIST_INDENT_PER_LEVEL
        }
        Block::BlockQuote { .. } => render::BLOCKQUOTE_INDENT,
        _ => 0.0,
    }
}

// ── Paragraph line mapping helpers ──────────────────────

/// Information about a visual line within a paragraph.
struct LineInfo {
    /// The visual line index.
    line: usize,
    /// The character offset at the start of this line (relative to block start).
    start_offset: usize,
}

/// Compute the absolute line height in pixels for a paragraph.
fn paragraph_line_height_px(paragraph: &IcedParagraph) -> f32 {
    let font_size: f32 = paragraph.size().0;
    font_size * render::LINE_HEIGHT_MULTIPLIER
}

/// Estimate the number of visual lines in a paragraph.
fn paragraph_line_count(paragraph: &IcedParagraph) -> usize {
    let line_height_px = paragraph_line_height_px(paragraph);
    if line_height_px <= 0.0 {
        return 1;
    }
    let total_height = paragraph.min_bounds().height;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let count = (total_height / line_height_px).ceil() as usize;
    count.max(1)
}

/// Build a list of `(line_index, start_char_offset)` pairs for all visual lines
/// in a paragraph. Uses `hit_test` at the left edge of each line to discover
/// line-start offsets.
fn build_line_starts(paragraph: &IcedParagraph) -> Vec<(usize, usize)> {
    let line_count = paragraph_line_count(paragraph);
    let line_height_px = paragraph_line_height_px(paragraph);

    let mut line_starts: Vec<(usize, usize)> = Vec::with_capacity(line_count);
    line_starts.push((0, 0));

    for line_idx in 1..line_count {
        let probe_y = line_idx as f32 * line_height_px;
        let probe = Point::new(0.0, probe_y);
        if let Some(hit) = paragraph.hit_test(probe) {
            let offset = hit.cursor();
            // Only add if this line starts at a different offset than the previous.
            if line_starts.last().is_some_and(|&(_, prev)| prev != offset) {
                line_starts.push((line_idx, offset));
            }
        }
    }

    line_starts
}

/// Find which visual line a character offset falls on within a paragraph.
/// Returns the line index and the character offset at the start of that line.
fn find_line_for_offset(
    paragraph: &IcedParagraph,
    char_offset: usize,
) -> LineInfo {
    let line_starts = build_line_starts(paragraph);

    // Find the last line whose start_offset <= char_offset.
    let mut best = LineInfo {
        line: 0,
        start_offset: 0,
    };
    for &(line_idx, start) in &line_starts {
        if start <= char_offset {
            best = LineInfo {
                line: line_idx,
                start_offset: start,
            };
        } else {
            break;
        }
    }

    best
}

/// Compute the pixel position of a character offset within a paragraph.
/// Returns a point relative to the paragraph origin, or `(0, 0)` as fallback.
fn grapheme_pixel_position(
    paragraph: &IcedParagraph,
    char_offset: usize,
) -> Point {
    let line_info = find_line_for_offset(paragraph, char_offset);
    let within_line = char_offset.saturating_sub(line_info.start_offset);

    paragraph
        .grapheme_position(line_info.line, within_line)
        .unwrap_or(Point::ORIGIN)
}

// ── Selection rectangle computation ─────────────────────

/// Compute per-line selection rectangles for a partial selection within a block.
///
/// `start_offset` and `end_offset` are character offsets within the block.
/// `para_origin_x` / `para_origin_y` are the absolute pixel positions of the
/// paragraph origin. `available_width` is the full text area width.
///
/// Returns a list of `SelectionRect`s, one per visual line that intersects the
/// selection range.
fn compute_selection_rects(
    paragraph: &IcedParagraph,
    start_offset: usize,
    end_offset: usize,
    para_origin_x: f32,
    para_origin_y: f32,
    available_width: f32,
) -> Vec<SelectionRect> {
    let line_height_px = paragraph_line_height_px(paragraph);
    let line_starts = build_line_starts(paragraph);
    let line_count = line_starts.len();
    let mut rects = Vec::new();

    for (i, &(line_idx, line_start)) in line_starts.iter().enumerate() {
        // Determine the character range for this line.
        let line_end = if i + 1 < line_count {
            line_starts[i + 1].1
        } else {
            usize::MAX // last line extends to end of block
        };

        // Skip lines that don't overlap the selection.
        if line_start >= end_offset || line_end <= start_offset {
            continue;
        }

        let line_y = para_origin_y + line_idx as f32 * line_height_px;

        // Determine x-coordinates for this line's selection portion.
        let sel_start_in_line = start_offset.max(line_start);
        let sel_end_in_line = end_offset.min(line_end);

        let x_start = if sel_start_in_line <= line_start {
            // Selection starts at or before this line — use left edge.
            para_origin_x
        } else {
            let within_line = sel_start_in_line.saturating_sub(line_start);
            let pos = paragraph
                .grapheme_position(line_idx, within_line)
                .unwrap_or(Point::ORIGIN);
            para_origin_x + pos.x
        };

        let x_end = if sel_end_in_line >= line_end {
            // Selection extends to or past the end of this line — use right edge.
            // This covers both middle lines (where line_end is the next line's
            // start) and the last line when end_offset >= the block's char count
            // (or is usize::MAX, as used by BlockSelectionKind::First).
            para_origin_x + available_width
        } else {
            let within_line = sel_end_in_line.saturating_sub(line_start);
            let pos = paragraph
                .grapheme_position(line_idx, within_line)
                .unwrap_or(Point::ORIGIN);
            para_origin_x + pos.x
        };

        let width = (x_end - x_start).max(0.0);
        if width > 0.0 {
            rects.push(SelectionRect {
                x: x_start,
                y: line_y,
                width,
                height: line_height_px,
            });
        }
    }

    rects
}

// ── Auto-scroll helper ───────────────────────────────────

/// Scrolling line height used for mouse wheel (px per line).
const SCROLL_LINE_HEIGHT: f32 = 20.0;

/// Drag auto-scroll: pixels scrolled per pixel of cursor overshoot beyond the
/// viewport edge.
const DRAG_SCROLL_SPEED: f32 = 2.0;

/// Drag auto-scroll: maximum scroll distance per frame (pixels).
const DRAG_SCROLL_MAX: f32 = 30.0;

/// Ensure the cursor (at `cursor_y` with `cursor_height`) is visible within
/// the viewport defined by `scroll_offset` and `viewport_height`.
///
/// Adjusts `scroll_offset` so the cursor is fully visible, scrolling up or
/// down as needed. Also clamps to `[0, max_scroll]`.
fn ensure_cursor_visible(
    scroll_offset: &mut f32,
    cursor_y: f32,
    cursor_height: f32,
    viewport_height: f32,
    total_content_height: f32,
) {
    let max_scroll = (total_content_height - viewport_height).max(0.0);

    // Cursor is above the viewport — scroll up.
    if cursor_y < *scroll_offset {
        *scroll_offset = cursor_y;
    }
    // Cursor bottom is below the viewport — scroll down.
    if cursor_y + cursor_height > *scroll_offset + viewport_height {
        *scroll_offset = cursor_y + cursor_height - viewport_height;
    }

    *scroll_offset = scroll_offset.clamp(0.0, max_scroll);
}

// ── Hit testing helper ───────────────────────────────────

/// Convert a pixel position (relative to the content origin, after padding) to a
/// `DocPosition` by hit-testing the paragraph cache.
///
/// Finds which block the point falls in via `ParagraphCache::block_at_y`, then
/// calls `Paragraph::hit_test` on that block's cached paragraph to get the
/// character offset within the block.
fn hit_test_content_point(
    content_pos: Point,
    cache: &ParagraphCache<IcedParagraph>,
    document: &crate::document::Document,
) -> crate::document::DocPosition {
    // Find the block at the click y-coordinate.
    let Some(block_index) = cache.block_at_y(content_pos.y) else {
        return crate::document::DocPosition::zero();
    };

    let Some(entry) = cache.get(block_index) else {
        return crate::document::DocPosition::new(block_index, 0);
    };

    let content_x_offset = document
        .block(block_index)
        .map(block_content_x_offset)
        .unwrap_or(0.0);

    // For container blocks (List, BlockQuote) with child paragraphs,
    // find which child the click falls in and hit-test that child.
    let children = entry.child_paragraphs();
    if !children.is_empty() {
        let local_y = content_pos.y - entry.y_offset();

        // Find the child whose y-range contains the click. Fall back to
        // the last child if the click is below all children.
        let child = children
            .iter()
            .rev()
            .find(|c| local_y >= c.local_y_offset)
            .unwrap_or(&children[0]);

        let local_point = Point::new(
            content_pos.x - content_x_offset,
            local_y - child.local_y_offset,
        );

        let char_offset = child
            .paragraph
            .hit_test(local_point)
            .map(iced::advanced::text::Hit::cursor)
            .unwrap_or(0);

        return crate::document::DocPosition::new(block_index, char_offset);
    }

    // For inline blocks, hit-test the single paragraph.
    let Some(paragraph) = entry.paragraph() else {
        // No paragraph (e.g. HorizontalRule) — place cursor at start of block.
        return crate::document::DocPosition::new(block_index, 0);
    };

    // Translate into paragraph-local coordinates.
    let local_point = Point::new(
        content_pos.x - content_x_offset,
        content_pos.y - entry.y_offset(),
    );

    let char_offset = paragraph
        .hit_test(local_point)
        .map(iced::advanced::text::Hit::cursor)
        .unwrap_or(0);

    crate::document::DocPosition::new(block_index, char_offset)
}

// ── Into<Element> ───────────────────────────────────────

impl<'a, Message: 'a> From<RichTextEditor<'a, Message>>
    for Element<'a, Message, iced::Theme, iced::Renderer>
{
    fn from(editor: RichTextEditor<'a, Message>) -> Self {
        Self::new(editor)
    }
}
