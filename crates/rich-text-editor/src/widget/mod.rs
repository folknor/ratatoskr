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
pub mod input;
pub mod render;

use crate::document::{Block, DocPosition, DocSelection, Document, InlineStyle};
use crate::html_parse::from_html;
use crate::html_serialize::to_html;
use crate::normalize::{normalize, normalize_blocks};
use crate::operations::EditOp;
use crate::rules::{self, EditAction};
use crate::undo::UndoStack;

use cursor::{
    BlockLayout, BlockSelectionKind, CursorState, DragState, SelectionRect, CURSOR_WIDTH,
    SELECTION_ALPHA,
};
use input::{KeyAction, MoveAction};
use render::ParagraphCache;

use iced::advanced::layout;
use iced::advanced::mouse::click::Click;
use iced::advanced::renderer;
use iced::advanced::renderer::Renderer as _;
use iced::advanced::text::Renderer as TextRenderer;
use iced::advanced::widget::{self, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::keyboard;
use iced::mouse;
use iced::time::{Duration, Instant};
use iced::window;
use iced::{Color, Element, Event, Font, Length, Padding, Point, Rectangle, Size};

/// The paragraph type used by the iced Renderer.
type IcedParagraph = <iced::Renderer as TextRenderer>::Paragraph;

// ── Action (events emitted by the widget) ───────────────

/// An action emitted by the rich text editor widget to the application.
///
/// The application should call [`EditorState::perform`] in its `update` method
/// for each action received.
#[derive(Debug, Clone)]
pub enum Action {
    /// An editing action (text input, delete, format toggle, etc.).
    Edit(EditAction),
    /// A cursor movement (the selection should be collapsed to the new position).
    Move(MoveAction),
    /// A selection extension (move focus, keep anchor).
    Select(MoveAction),
    /// Select all text in the document.
    SelectAll,
    /// Undo the last action.
    Undo,
    /// Redo the last undone action.
    Redo,
    /// Copy the current selection to the clipboard.
    Copy,
    /// Cut the current selection to the clipboard.
    Cut,
    /// Paste text from the clipboard.
    Paste(String),
    /// A click at a pixel position (relative to the widget origin).
    Click(Point),
    /// A drag to a pixel position (extends selection).
    Drag(Point),
    /// A link was clicked.
    LinkClicked(String),
    /// The editor gained focus.
    Focus,
    /// The editor lost focus.
    Blur,
}

// ── EditorState (application-owned mutable state) ───────

/// The mutable state of a rich text editor, owned by the application.
///
/// The widget renders this state immutably; mutations happen in the app's
/// `update()` via [`EditorState::perform`].
#[derive(Debug, Clone)]
pub struct EditorState {
    /// The document being edited.
    pub document: Document,
    /// Current cursor / selection.
    pub selection: DocSelection,
    /// Undo/redo history.
    pub undo_stack: UndoStack,
    /// Pending inline style (for typing at a collapsed caret after toggling
    /// a format shortcut before typing).
    pub pending_style: InlineStyle,
    /// Visual cursor state (blink, focus).
    cursor: CursorState,
    /// Active mouse drag state.
    drag: Option<DragState>,
    /// Block layouts computed during the last layout pass. Stored here so
    /// that `perform` can do hit-testing for click/drag actions.
    block_layouts: Vec<BlockLayout>,
}

impl EditorState {
    /// Create a new editor state with an empty document.
    pub fn new() -> Self {
        Self {
            document: Document::new(),
            selection: DocSelection::caret(DocPosition::zero()),
            undo_stack: UndoStack::default(),
            pending_style: InlineStyle::empty(),
            cursor: CursorState::new(),
            drag: None,
            block_layouts: Vec::new(),
        }
    }

    /// Create an editor state from an existing document.
    pub fn from_document(doc: Document) -> Self {
        Self {
            document: doc,
            selection: DocSelection::caret(DocPosition::zero()),
            undo_stack: UndoStack::default(),
            pending_style: InlineStyle::empty(),
            cursor: CursorState::new(),
            drag: None,
            block_layouts: Vec::new(),
        }
    }

    /// Create an editor state by parsing HTML.
    pub fn from_html(html: &str) -> Self {
        Self::from_document(from_html(html))
    }

    /// Serialize the document to HTML.
    pub fn to_html(&self) -> String {
        to_html(&self.document)
    }

    /// Get the plain text of the current selection, or an empty string
    /// if the selection is collapsed.
    pub fn selection_text(&self) -> String {
        if self.selection.is_collapsed() {
            return String::new();
        }
        let start = self.selection.start();
        let end = self.selection.end();
        self.document
            .slice(start, end)
            .map_or_else(String::new, |slice| {
                let mut buf = String::new();
                for (i, block) in slice.blocks.iter().enumerate() {
                    if i > 0 {
                        buf.push('\n');
                    }
                    buf.push_str(&block.flattened_text());
                }
                buf
            })
    }

    /// Set the selection explicitly.
    pub fn set_selection(&mut self, sel: DocSelection) {
        self.selection = sel;
        self.cursor.reset_blink();
        self.cursor.clear_target_x();
    }

    /// Whether the editor currently has focus.
    pub fn is_focused(&self) -> bool {
        self.cursor.is_focused()
    }

    /// Perform an action (called by the app in its `update` method).
    pub fn perform(&mut self, action: Action) {
        match action {
            Action::Edit(edit_action) => self.apply_action(edit_action),
            Action::Move(move_action) => self.apply_move(move_action, false),
            Action::Select(move_action) => self.apply_move(move_action, true),
            Action::SelectAll => {
                let end = self.document.end_position();
                self.selection = DocSelection::range(DocPosition::zero(), end);
                self.cursor.reset_blink();
            }
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            Action::Copy | Action::Cut => {
                // Clipboard operations are handled by the widget's update()
                // method which has clipboard access. The perform() call for
                // Cut will just delete the selection.
                if matches!(action, Action::Cut) && !self.selection.is_collapsed() {
                    self.apply_action(EditAction::DeleteSelection);
                }
            }
            Action::Paste(text) => {
                self.apply_action(EditAction::InsertText(text));
            }
            Action::Click(point) => {
                self.handle_click(point);
            }
            Action::Drag(point) => {
                self.handle_drag(point);
            }
            Action::LinkClicked(_) => {
                // The app handles this in its update() method.
            }
            Action::Focus => {
                self.cursor.focus();
            }
            Action::Blur => {
                self.cursor.unfocus();
                self.drag = None;
            }
        }
    }

    /// Apply a high-level edit action through the rules engine.
    pub fn apply_action(&mut self, action: EditAction) {
        let cursor_before = self.selection;

        // Special case: ToggleInlineStyle at a collapsed caret toggles pending style.
        if let EditAction::ToggleInlineStyle(style) = &action
            && self.selection.is_collapsed()
        {
            self.pending_style.toggle(*style);
            self.undo_stack.break_group();
            return;
        }

        let ops = rules::resolve(
            &self.document,
            self.selection,
            action,
            self.pending_style,
        );

        if ops.is_empty() {
            return;
        }

        // Apply all ops.
        let mut dirty_blocks: Vec<usize> = Vec::new();
        for op in &ops {
            let pos_map = op.apply(&mut self.document);
            // Track dirty blocks for normalization.
            dirty_blocks.push(pos_map.block_index);
            // Map the selection through the edit.
            self.selection.anchor = pos_map.map(self.selection.anchor);
            self.selection.focus = pos_map.map(self.selection.focus);
        }

        // Compute cursor_after based on the operation type.
        // For insert: cursor moves past inserted text.
        // For delete: cursor collapses to start.
        // For split: cursor at start of new block.
        // For merge: cursor at merge point.
        self.update_cursor_after_ops(&ops);

        // Normalize dirty blocks.
        dirty_blocks.sort_unstable();
        dirty_blocks.dedup();
        normalize_blocks(&mut self.document, &dirty_blocks);

        // Clamp selection to valid bounds after normalization.
        self.selection.anchor = self.document.clamp_position(self.selection.anchor);
        self.selection.focus = self.document.clamp_position(self.selection.focus);

        let cursor_after = self.selection;

        // Push to undo stack.
        self.undo_stack.push(ops, cursor_before, cursor_after);

        // Clear pending style after any edit (it's consumed by the insertion).
        self.pending_style = InlineStyle::empty();

        // Reset blink on edit.
        self.cursor.reset_blink();
        self.cursor.clear_target_x();
    }

    /// Update the cursor position after applying ops.
    fn update_cursor_after_ops(&mut self, ops: &[EditOp]) {
        // Use the last op to determine final cursor position.
        let Some(last_op) = ops.last() else {
            return;
        };

        match last_op {
            EditOp::InsertText { position, text } => {
                let char_count = text.chars().count();
                let new_pos =
                    DocPosition::new(position.block_index, position.offset + char_count);
                self.selection = DocSelection::caret(new_pos);
            }
            EditOp::DeleteRange { start, .. } => {
                self.selection = DocSelection::caret(*start);
            }
            EditOp::SplitBlock { position } => {
                // Cursor at start of the new (second) block.
                self.selection =
                    DocSelection::caret(DocPosition::new(position.block_index + 1, 0));
            }
            EditOp::MergeBlocks { merge_offset, block_index, .. } => {
                // Cursor at the merge point in the previous block.
                let target_block = block_index.saturating_sub(1);
                self.selection =
                    DocSelection::caret(DocPosition::new(target_block, *merge_offset));
            }
            EditOp::ToggleInlineStyle { .. }
            | EditOp::SetBlockType { .. }
            | EditOp::InsertBlock { .. }
            | EditOp::RemoveBlock { .. } => {
                // These don't move the cursor.
            }
        }
    }

    /// Undo the last action.
    pub fn undo(&mut self) {
        let Some(group) = self.undo_stack.undo() else {
            return;
        };

        // Apply inverse ops in reverse order.
        for op in group.ops.iter().rev() {
            op.invert().apply(&mut self.document);
        }

        normalize(&mut self.document);
        self.selection = group.cursor_before;
        self.selection.anchor = self.document.clamp_position(self.selection.anchor);
        self.selection.focus = self.document.clamp_position(self.selection.focus);
        self.cursor.reset_blink();
    }

    /// Redo the last undone action.
    pub fn redo(&mut self) {
        let Some(group) = self.undo_stack.redo() else {
            return;
        };

        // Re-apply ops in order.
        for op in &group.ops {
            op.apply(&mut self.document);
        }

        normalize(&mut self.document);
        self.selection = group.cursor_after;
        self.selection.anchor = self.document.clamp_position(self.selection.anchor);
        self.selection.focus = self.document.clamp_position(self.selection.focus);
        self.cursor.reset_blink();
    }

    /// Apply a cursor movement.
    fn apply_move(&mut self, move_action: MoveAction, extend_selection: bool) {
        let doc = &self.document;
        let focus = self.selection.focus;

        let new_focus = match move_action {
            MoveAction::Left => {
                // If there's a non-collapsed selection and not extending, collapse to start.
                if !extend_selection && !self.selection.is_collapsed() {
                    self.selection.start()
                } else {
                    input::move_left(doc, focus)
                }
            }
            MoveAction::Right => {
                if !extend_selection && !self.selection.is_collapsed() {
                    self.selection.end()
                } else {
                    input::move_right(doc, focus)
                }
            }
            MoveAction::WordLeft => input::word_left(doc, focus),
            MoveAction::WordRight => input::word_right(doc, focus),
            MoveAction::Home => input::home(focus),
            MoveAction::End => input::end(doc, focus),
            MoveAction::DocumentStart => input::document_start(),
            MoveAction::DocumentEnd => input::document_end(doc),
            MoveAction::Up | MoveAction::Down => {
                // Vertical movement requires paragraph layout info which is
                // renderer-specific. For now, move to start/end of adjacent block.
                match move_action {
                    MoveAction::Up => {
                        if focus.block_index > 0 {
                            let prev_len = doc
                                .block(focus.block_index - 1)
                                .map_or(0, Block::char_len);
                            DocPosition::new(focus.block_index - 1, prev_len.min(focus.offset))
                        } else {
                            DocPosition::new(0, 0)
                        }
                    }
                    MoveAction::Down => {
                        if focus.block_index + 1 < doc.block_count() {
                            let next_len = doc
                                .block(focus.block_index + 1)
                                .map_or(0, Block::char_len);
                            DocPosition::new(focus.block_index + 1, next_len.min(focus.offset))
                        } else {
                            doc.end_position()
                        }
                    }
                    _ => focus,
                }
            }
        };

        if extend_selection {
            self.selection = DocSelection::range(self.selection.anchor, new_focus);
        } else {
            self.selection = DocSelection::caret(new_focus);
        }

        self.cursor.reset_blink();

        // Clear target_x on horizontal movement, preserve on vertical.
        if !matches!(move_action, MoveAction::Up | MoveAction::Down) {
            self.cursor.clear_target_x();
        }
    }

    /// Handle a click at a pixel position by hit-testing block layouts.
    fn handle_click(&mut self, point: Point) {
        self.cursor.focus();
        self.cursor.reset_blink();

        let doc_pos = self.hit_test_point(point);
        self.selection = DocSelection::caret(doc_pos);
        self.drag = Some(DragState::start(doc_pos));
        self.pending_style = InlineStyle::empty();
    }

    /// Handle a drag to a pixel position by extending the selection.
    fn handle_drag(&mut self, point: Point) {
        let doc_pos = self.hit_test_point(point);
        if let Some(drag) = &self.drag {
            self.selection = DocSelection::range(drag.anchor, doc_pos);
        }
        self.cursor.reset_blink();
    }

    /// Hit-test a pixel point against block layouts.
    ///
    /// Returns the document position closest to the point. If there are no
    /// block layouts (before first layout), returns position zero.
    fn hit_test_point(&self, point: Point) -> DocPosition {
        if let Some((block_index, _local)) =
            cursor::hit_test(point, &self.block_layouts)
        {
            // Without a renderer-level Paragraph::hit_test, we place the
            // cursor at the start of the clicked block. A proper
            // implementation would call Paragraph::hit_test to find the
            // exact character offset from the x position.
            DocPosition::new(block_index, 0)
        } else {
            DocPosition::zero()
        }
    }

    /// Update block layouts from the paragraph cache (called during widget layout).
    pub fn update_block_layouts(&mut self, layouts: Vec<BlockLayout>) {
        self.block_layouts = layouts;
    }
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}

// ── Widget tree state ───────────────────────────────────

/// Internal widget state stored in the iced widget tree. Holds the paragraph
/// cache and focus/blink timing.
struct WidgetState {
    /// Paragraph cache: one entry per document block.
    cache: ParagraphCache<IcedParagraph>,
    /// Focus tracking for cursor blink.
    focus: Option<FocusState>,
    /// Last mouse click for double/triple click detection (future use).
    _last_click: Option<Click>,
    /// Whether a drag is active.
    dragging: bool,
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

// ── Widget trait implementation ─────────────────────────

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for RichTextEditor<'_, Message> {
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<WidgetState>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(WidgetState {
            cache: ParagraphCache::new(),
            focus: None,
            _last_click: None,
            dragging: false,
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

        match self.height {
            Length::Fill | Length::FillPortion(_) | Length::Fixed(_) => {
                layout::Node::new(limits.max())
            }
            Length::Shrink => {
                let size = limits
                    .height(Length::Fixed(content_height))
                    .max();
                layout::Node::new(size)
            }
        }
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

        // Handle window focus/unfocus and redraw for cursor blink.
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

        // Handle keyboard events.
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
                        shell.capture_event();
                    }
                    KeyAction::Cut => {
                        let text = self.state.selection_text();
                        if !text.is_empty() {
                            clipboard.write(iced::advanced::clipboard::Kind::Standard, text.clone());
                            shell.publish(on_action(Action::Edit(EditAction::DeleteSelection)));
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

        // Handle mouse events.
        match event {
            Event::Mouse(mouse::Event::ButtonPressed { button: mouse::Button::Left, .. }) => {
                if let Some(position) = cursor_pos.position_in(bounds) {
                    // Translate to content coordinates (account for padding).
                    let content_pos = Point::new(
                        position.x - self.padding.left,
                        position.y - self.padding.top,
                    );

                    widget_state.focus = Some(FocusState::now());
                    widget_state.dragging = true;

                    shell.publish(on_action(Action::Focus));
                    shell.publish(on_action(Action::Click(content_pos)));
                    shell.capture_event();
                } else if widget_state.focus.is_some() {
                    // Click outside the editor: blur.
                    widget_state.focus = None;
                    widget_state.dragging = false;
                    shell.publish(on_action(Action::Blur));
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if widget_state.dragging
                    && let Some(position) = cursor_pos.position_in(bounds)
                {
                    let content_pos = Point::new(
                        position.x - self.padding.left,
                        position.y - self.padding.top,
                    );
                    shell.publish(on_action(Action::Drag(content_pos)));
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                widget_state.dragging = false;
            }
            _ => {}
        }
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

        // Draw each block.
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

                    if let Some(paragraph) = entry.paragraph() {
                        let para_origin = Point::new(
                            block_origin.x + render::BLOCKQUOTE_INDENT,
                            block_origin.y,
                        );
                        render::draw_paragraph(
                            renderer,
                            paragraph,
                            para_origin,
                            self.text_color,
                            text_bounds,
                        );
                    }
                }
                Block::List { .. } => {
                    // Draw the combined paragraph if available.
                    if let Some(paragraph) = entry.paragraph() {
                        let para_origin = Point::new(
                            block_origin.x + render::LIST_MARKER_WIDTH,
                            block_origin.y,
                        );
                        render::draw_paragraph(
                            renderer,
                            paragraph,
                            para_origin,
                            self.text_color,
                            text_bounds,
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
                            text_bounds,
                        );
                    }
                }
            }
        }

        // Draw selection highlights.
        if !self.state.selection.is_collapsed() {
            let sel_ranges = cursor::selection_block_ranges(self.state.selection);

            for (block_idx, kind) in &sel_ranges {
                let Some(entry) = cache.get(*block_idx) else {
                    continue;
                };

                // Compute selection rectangle for this block.
                // For the initial implementation, highlight the entire block height.
                let block_y = text_bounds.y + entry.y_offset();
                let block_height = entry.height();

                let sel_rect = match kind {
                    BlockSelectionKind::Full => SelectionRect {
                        x: text_bounds.x,
                        y: block_y,
                        width: text_bounds.width,
                        height: block_height,
                    },
                    BlockSelectionKind::Single { .. }
                    | BlockSelectionKind::First { .. }
                    | BlockSelectionKind::Last { .. } => {
                        // Full-line highlight for now; proper per-character
                        // selection requires Paragraph::grapheme_position.
                        SelectionRect {
                            x: text_bounds.x,
                            y: block_y,
                            width: text_bounds.width,
                            height: block_height,
                        }
                    }
                };

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

        // Draw cursor.
        if let Some(focus) = &widget_state.focus
            && focus.is_cursor_visible()
            && self.state.selection.is_collapsed()
        {
            let pos = self.state.selection.focus;
            if let Some(entry) = cache.get(pos.block_index) {
                let cursor_x = text_bounds.x;
                let cursor_y = text_bounds.y + entry.y_offset();
                // Use a default line height. Proper implementation would use
                // Paragraph::grapheme_position for exact placement.
                let cursor_height = entry.height().max(render::FONT_SIZE_BODY * 1.4);

                let cursor_rect = Rectangle::new(
                    Point::new(cursor_x, cursor_y),
                    Size::new(CURSOR_WIDTH, cursor_height),
                );

                if let Some(clipped) = text_bounds.intersection(&cursor_rect) {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: clipped,
                            ..Default::default()
                        },
                        self.cursor_color,
                    );
                }
            }
        }
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

// ── Into<Element> ───────────────────────────────────────

impl<'a, Message: 'a> From<RichTextEditor<'a, Message>>
    for Element<'a, Message, iced::Theme, iced::Renderer>
{
    fn from(editor: RichTextEditor<'a, Message>) -> Self {
        Self::new(editor)
    }
}

// ── Tests ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Block, DocPosition, DocSelection, Document, InlineStyle};

    // ── EditorState::new ─────────────────────────────────

    #[test]
    fn new_editor_has_empty_document() {
        let state = EditorState::new();
        assert_eq!(state.document.block_count(), 1);
        assert_eq!(state.document.block(0).map(Block::char_len), Some(0));
        assert!(state.selection.is_collapsed());
        assert_eq!(state.selection.focus, DocPosition::zero());
    }

    #[test]
    fn from_document_preserves_blocks() {
        let doc = Document::from_blocks(vec![
            Block::paragraph("hello"),
            Block::paragraph("world"),
        ]);
        let state = EditorState::from_document(doc);
        assert_eq!(state.document.block_count(), 2);
        assert_eq!(state.document.block(0).map(Block::flattened_text).as_deref(), Some("hello"));
        assert_eq!(state.document.block(1).map(Block::flattened_text).as_deref(), Some("world"));
    }

    #[test]
    fn from_html_parses() {
        let state = EditorState::from_html("<p>hello</p>");
        assert_eq!(state.document.block(0).map(Block::flattened_text).as_deref(), Some("hello"));
    }

    // ── EditorState::to_html ─────────────────────────────

    #[test]
    fn to_html_round_trips() {
        let state = EditorState::from_html("<p>hello</p><p>world</p>");
        let html = state.to_html();
        assert!(html.contains("hello"));
        assert!(html.contains("world"));
    }

    // ── EditorState::selection_text ──────────────────────

    #[test]
    fn selection_text_collapsed_is_empty() {
        let state = EditorState::new();
        assert!(state.selection_text().is_empty());
    }

    #[test]
    fn selection_text_single_block() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello world"),
        ]));
        state.selection = DocSelection::range(
            DocPosition::new(0, 0),
            DocPosition::new(0, 5),
        );
        assert_eq!(state.selection_text(), "hello");
    }

    #[test]
    fn selection_text_cross_block() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
            Block::paragraph("world"),
        ]));
        state.selection = DocSelection::range(
            DocPosition::new(0, 3),
            DocPosition::new(1, 2),
        );
        let text = state.selection_text();
        assert!(text.contains("lo"));
        assert!(text.contains("wo"));
    }

    // ── EditorState::apply_action — insert ───────────────

    #[test]
    fn apply_action_insert_text() {
        let mut state = EditorState::new();
        state.apply_action(EditAction::InsertText("hello".into()));
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("hello"),
        );
        assert_eq!(state.selection.focus, DocPosition::new(0, 5));
    }

    #[test]
    fn apply_action_insert_multiple_chars() {
        let mut state = EditorState::new();
        state.apply_action(EditAction::InsertText("h".into()));
        state.apply_action(EditAction::InsertText("e".into()));
        state.apply_action(EditAction::InsertText("l".into()));
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("hel"),
        );
        assert_eq!(state.selection.focus, DocPosition::new(0, 3));
    }

    // ── EditorState::apply_action — delete ───────────────

    #[test]
    fn apply_action_delete_backward() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
        ]));
        state.selection = DocSelection::caret(DocPosition::new(0, 5));
        state.apply_action(EditAction::DeleteBackward);
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("hell"),
        );
        assert_eq!(state.selection.focus, DocPosition::new(0, 4));
    }

    #[test]
    fn apply_action_delete_forward() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
        ]));
        state.selection = DocSelection::caret(DocPosition::new(0, 0));
        state.apply_action(EditAction::DeleteForward);
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("ello"),
        );
    }

    #[test]
    fn apply_action_delete_selection() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello world"),
        ]));
        state.selection = DocSelection::range(
            DocPosition::new(0, 5),
            DocPosition::new(0, 11),
        );
        state.apply_action(EditAction::DeleteSelection);
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("hello"),
        );
    }

    // ── EditorState::apply_action — split block ──────────

    #[test]
    fn apply_action_split_block() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello world"),
        ]));
        state.selection = DocSelection::caret(DocPosition::new(0, 5));
        state.apply_action(EditAction::SplitBlock);
        assert_eq!(state.document.block_count(), 2);
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("hello"),
        );
        assert_eq!(
            state.document.block(1).map(Block::flattened_text).as_deref(),
            Some(" world"),
        );
        // Cursor should be at start of new block.
        assert_eq!(state.selection.focus, DocPosition::new(1, 0));
    }

    // ── EditorState::apply_action — toggle inline style ──

    #[test]
    fn toggle_style_at_caret_sets_pending() {
        let mut state = EditorState::new();
        state.apply_action(EditAction::ToggleInlineStyle(InlineStyle::BOLD));
        assert!(state.pending_style.contains(InlineStyle::BOLD));
    }

    #[test]
    fn toggle_style_at_caret_toggles_off() {
        let mut state = EditorState::new();
        state.apply_action(EditAction::ToggleInlineStyle(InlineStyle::BOLD));
        assert!(state.pending_style.contains(InlineStyle::BOLD));
        state.apply_action(EditAction::ToggleInlineStyle(InlineStyle::BOLD));
        assert!(!state.pending_style.contains(InlineStyle::BOLD));
    }

    #[test]
    fn toggle_style_with_selection_applies() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello world"),
        ]));
        state.selection = DocSelection::range(
            DocPosition::new(0, 0),
            DocPosition::new(0, 5),
        );
        state.apply_action(EditAction::ToggleInlineStyle(InlineStyle::BOLD));

        let runs = state.document.block(0).and_then(Block::runs).expect("runs");
        // The first part should be bold.
        assert!(runs[0].style.contains(InlineStyle::BOLD));
    }

    // ── EditorState::undo / redo ─────────────────────────

    #[test]
    fn undo_reverts_insert() {
        let mut state = EditorState::new();
        state.apply_action(EditAction::InsertText("hello".into()));
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("hello"),
        );

        state.undo();
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some(""),
        );
    }

    #[test]
    fn redo_reapplies_insert() {
        let mut state = EditorState::new();
        state.apply_action(EditAction::InsertText("hello".into()));
        state.undo();
        state.redo();
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("hello"),
        );
    }

    #[test]
    fn undo_redo_preserves_cursor() {
        let mut state = EditorState::new();
        let cursor_before = state.selection;
        state.apply_action(EditAction::InsertText("hello".into()));
        let cursor_after = state.selection;

        state.undo();
        assert_eq!(state.selection, cursor_before);

        state.redo();
        assert_eq!(state.selection, cursor_after);
    }

    #[test]
    fn multiple_undo_redo() {
        let mut state = EditorState::new();
        state.apply_action(EditAction::InsertText("a".into()));
        state.undo_stack.break_group();
        state.apply_action(EditAction::InsertText("b".into()));
        state.undo_stack.break_group();
        state.apply_action(EditAction::InsertText("c".into()));

        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("abc"),
        );

        state.undo(); // remove "c"
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("ab"),
        );

        state.undo(); // remove "b"
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("a"),
        );

        state.redo(); // re-add "b"
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("ab"),
        );
    }

    // ── EditorState::apply_move ──────────────────────────

    #[test]
    fn move_left() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
        ]));
        state.selection = DocSelection::caret(DocPosition::new(0, 3));
        state.perform(Action::Move(MoveAction::Left));
        assert_eq!(state.selection.focus, DocPosition::new(0, 2));
        assert!(state.selection.is_collapsed());
    }

    #[test]
    fn move_right() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
        ]));
        state.selection = DocSelection::caret(DocPosition::new(0, 2));
        state.perform(Action::Move(MoveAction::Right));
        assert_eq!(state.selection.focus, DocPosition::new(0, 3));
    }

    #[test]
    fn move_left_collapses_selection_to_start() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello world"),
        ]));
        state.selection = DocSelection::range(
            DocPosition::new(0, 2),
            DocPosition::new(0, 7),
        );
        state.perform(Action::Move(MoveAction::Left));
        assert!(state.selection.is_collapsed());
        assert_eq!(state.selection.focus, DocPosition::new(0, 2));
    }

    #[test]
    fn move_right_collapses_selection_to_end() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello world"),
        ]));
        state.selection = DocSelection::range(
            DocPosition::new(0, 2),
            DocPosition::new(0, 7),
        );
        state.perform(Action::Move(MoveAction::Right));
        assert!(state.selection.is_collapsed());
        assert_eq!(state.selection.focus, DocPosition::new(0, 7));
    }

    #[test]
    fn select_extends_selection() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
        ]));
        state.selection = DocSelection::caret(DocPosition::new(0, 2));
        state.perform(Action::Select(MoveAction::Right));
        assert!(!state.selection.is_collapsed());
        assert_eq!(state.selection.anchor, DocPosition::new(0, 2));
        assert_eq!(state.selection.focus, DocPosition::new(0, 3));
    }

    #[test]
    fn select_all() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
            Block::paragraph("world"),
        ]));
        state.perform(Action::SelectAll);
        assert_eq!(state.selection.start(), DocPosition::zero());
        assert_eq!(state.selection.end(), DocPosition::new(1, 5));
    }

    #[test]
    fn move_home_end() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello world"),
        ]));
        state.selection = DocSelection::caret(DocPosition::new(0, 5));

        state.perform(Action::Move(MoveAction::Home));
        assert_eq!(state.selection.focus, DocPosition::new(0, 0));

        state.perform(Action::Move(MoveAction::End));
        assert_eq!(state.selection.focus, DocPosition::new(0, 11));
    }

    #[test]
    fn move_document_start_end() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
            Block::paragraph("world"),
        ]));
        state.selection = DocSelection::caret(DocPosition::new(0, 3));

        state.perform(Action::Move(MoveAction::DocumentEnd));
        assert_eq!(state.selection.focus, DocPosition::new(1, 5));

        state.perform(Action::Move(MoveAction::DocumentStart));
        assert_eq!(state.selection.focus, DocPosition::zero());
    }

    #[test]
    fn move_up_down() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
            Block::paragraph("world"),
            Block::paragraph("foo"),
        ]));
        state.selection = DocSelection::caret(DocPosition::new(1, 3));

        state.perform(Action::Move(MoveAction::Up));
        assert_eq!(state.selection.focus.block_index, 0);

        state.selection = DocSelection::caret(DocPosition::new(1, 3));
        state.perform(Action::Move(MoveAction::Down));
        assert_eq!(state.selection.focus.block_index, 2);
    }

    // ── EditorState::set_selection ───────────────────────

    #[test]
    fn set_selection() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
        ]));
        let sel = DocSelection::range(
            DocPosition::new(0, 1),
            DocPosition::new(0, 4),
        );
        state.set_selection(sel);
        assert_eq!(state.selection, sel);
    }

    // ── EditorState::perform — focus / blur ──────────────

    #[test]
    fn focus_and_blur() {
        let mut state = EditorState::new();
        assert!(!state.is_focused());

        state.perform(Action::Focus);
        assert!(state.is_focused());

        state.perform(Action::Blur);
        assert!(!state.is_focused());
    }

    // ── Pending style cleared after edit ─────────────────

    #[test]
    fn pending_style_cleared_after_insert() {
        let mut state = EditorState::new();
        state.apply_action(EditAction::ToggleInlineStyle(InlineStyle::BOLD));
        assert!(state.pending_style.contains(InlineStyle::BOLD));

        state.apply_action(EditAction::InsertText("x".into()));
        assert!(state.pending_style.is_empty());
    }

    // ── Insert replaces selection ────────────────────────

    #[test]
    fn insert_replaces_selection() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello world"),
        ]));
        state.selection = DocSelection::range(
            DocPosition::new(0, 5),
            DocPosition::new(0, 11),
        );
        state.apply_action(EditAction::InsertText("!".into()));
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("hello!"),
        );
    }

    // ── Backspace at block start merges ──────────────────

    #[test]
    fn backspace_at_block_start_merges() {
        let mut state = EditorState::from_document(Document::from_blocks(vec![
            Block::paragraph("hello"),
            Block::paragraph("world"),
        ]));
        state.selection = DocSelection::caret(DocPosition::new(1, 0));
        state.apply_action(EditAction::DeleteBackward);
        assert_eq!(state.document.block_count(), 1);
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some("helloworld"),
        );
        assert_eq!(state.selection.focus, DocPosition::new(0, 5));
    }

    // ── Undo stack grouping ─────────────────────────────

    #[test]
    fn consecutive_inserts_group_in_undo() {
        let mut state = EditorState::new();
        state.apply_action(EditAction::InsertText("a".into()));
        state.apply_action(EditAction::InsertText("b".into()));
        state.apply_action(EditAction::InsertText("c".into()));

        // All consecutive inserts should merge into one undo group.
        assert_eq!(state.undo_stack.undo_len(), 1);

        state.undo();
        assert_eq!(
            state.document.block(0).map(Block::flattened_text).as_deref(),
            Some(""),
        );
    }

    // ── Default trait ────────────────────────────────────

    #[test]
    fn default_creates_empty() {
        let state = EditorState::default();
        assert_eq!(state.document.block_count(), 1);
        assert!(state.selection.is_collapsed());
    }
}
