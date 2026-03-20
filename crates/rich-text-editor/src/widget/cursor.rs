//! Cursor and selection rendering, hit testing, vertical cursor movement.
//!
//! This module provides renderer-agnostic infrastructure for:
//! - Cursor blink state and focus tracking
//! - Hit testing (pixel position to `DocPosition`)
//! - Cursor geometry computation (for rendering the caret)
//! - Selection rectangle computation (for rendering highlighted ranges)
//! - Vertical cursor movement across block boundaries
//! - Drag selection state
//!
//! The actual `Paragraph::grapheme_position()` and `Paragraph::hit_test()` calls
//! happen in the Widget impl, which owns the renderer-specific paragraphs. This
//! module provides the block-level offset arithmetic and the `BlockLayout`
//! abstraction so the Widget can delegate the paragraph-level work.

use crate::document::{DocPosition, DocSelection};

// ── Drawing constants ──────────────────────────────────

/// Width of the cursor caret in pixels.
pub const CURSOR_WIDTH: f32 = 1.5;

/// Blink interval in seconds (500ms on, 500ms off).
pub const BLINK_INTERVAL: f32 = 0.5;

/// Alpha value for selection highlight rectangles.
pub const SELECTION_ALPHA: f32 = 0.3;

// ── Block layout ───────────────────────────────────────

/// Layout information for a single block, computed during the widget's
/// `layout()` pass. The Widget populates a `Vec<BlockLayout>` that this
/// module's functions consume for hit testing and cursor positioning.
#[derive(Debug, Clone, Copy)]
pub struct BlockLayout {
    /// Vertical offset of this block from the top of the editor widget.
    pub y_offset: f32,
    /// Total height of this block (including any spacing).
    pub height: f32,
    /// Index into the document's block list.
    pub block_index: usize,
    /// Offset from the editor origin to the start of the paragraph content
    /// within this block. Accounts for indentation, list bullet width, etc.
    pub content_offset: iced::Vector,
}

// ── Cursor geometry ────────────────────────────────────

/// The pixel position and height of a cursor caret, relative to the editor
/// widget origin.
#[derive(Debug, Clone, Copy)]
pub struct CursorGeometry {
    /// Horizontal position of the caret's left edge.
    pub x: f32,
    /// Vertical position of the caret's top edge.
    pub y: f32,
    /// Height of the caret (matches the line height at the cursor position).
    pub height: f32,
}

// ── Selection rectangle ────────────────────────────────

/// A single rectangle in a selection highlight. One per visual line span.
#[derive(Debug, Clone, Copy)]
pub struct SelectionRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

// ── Cursor state ───────────────────────────────────────

/// Visual cursor state: blink timing, focus, and saved x-coordinate for
/// vertical movement.
#[derive(Debug, Clone)]
pub struct CursorState {
    /// Accumulated time in the current blink phase (seconds).
    blink_timer: f32,
    /// Whether the cursor is currently visible in the blink cycle.
    visible: bool,
    /// Whether the editor has focus.
    focused: bool,
    /// Saved x-coordinate for vertical cursor movement.
    /// When moving up/down, we want to maintain the same x position
    /// rather than snapping to wherever the character boundary falls.
    target_x: Option<f32>,
}

impl CursorState {
    /// Create a new cursor state (unfocused, not visible).
    pub fn new() -> Self {
        Self {
            blink_timer: 0.0,
            visible: true,
            focused: false,
            target_x: None,
        }
    }

    /// Advance the blink timer by `dt` seconds. Toggles visibility each
    /// `BLINK_INTERVAL`.
    pub fn tick(&mut self, dt: f32) {
        if !self.focused {
            return;
        }
        self.blink_timer += dt;
        while self.blink_timer >= BLINK_INTERVAL {
            self.blink_timer -= BLINK_INTERVAL;
            self.visible = !self.visible;
        }
    }

    /// Reset the blink cycle: make the cursor visible immediately.
    /// Call this on any edit, cursor movement, or focus gain.
    pub fn reset_blink(&mut self) {
        self.blink_timer = 0.0;
        self.visible = true;
    }

    /// Mark the editor as focused. Resets the blink cycle.
    pub fn focus(&mut self) {
        self.focused = true;
        self.reset_blink();
    }

    /// Mark the editor as unfocused. The cursor should not render.
    pub fn unfocus(&mut self) {
        self.focused = false;
        self.visible = false;
    }

    /// Whether the cursor should be drawn right now.
    pub fn is_visible(&self) -> bool {
        self.focused && self.visible
    }

    /// Whether the editor currently has focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Get the saved target x-coordinate for vertical movement.
    pub fn target_x(&self) -> Option<f32> {
        self.target_x
    }

    /// Set the target x-coordinate (call when starting a vertical movement
    /// sequence).
    pub fn set_target_x(&mut self, x: f32) {
        self.target_x = Some(x);
    }

    /// Clear the target x-coordinate (call on any horizontal movement or
    /// edit that changes cursor position non-vertically).
    pub fn clear_target_x(&mut self) {
        self.target_x = None;
    }
}

impl Default for CursorState {
    fn default() -> Self {
        Self::new()
    }
}

// ── Drag state ─────────────────────────────────────────

/// Tracks mouse drag for selection.
#[derive(Debug, Clone)]
pub struct DragState {
    /// The document position where the drag started (the anchor of the
    /// resulting selection).
    pub anchor: DocPosition,
    /// Whether a drag is currently in progress.
    pub active: bool,
}

impl DragState {
    /// Begin a new drag at the given anchor position.
    pub fn start(anchor: DocPosition) -> Self {
        Self {
            anchor,
            active: true,
        }
    }

    /// End the current drag. The anchor is preserved so the final selection
    /// can be read, but `active` is set to false.
    pub fn end(&mut self) {
        self.active = false;
    }
}

// ── Hit testing ────────────────────────────────────────

/// Find which block a pixel position falls within.
///
/// Returns the index into `block_layouts` (not the `block_index` field) and
/// the point translated into the block's local coordinate space (relative to
/// the paragraph content origin).
///
/// Uses binary search on `y_offset` for efficiency, though for typical email
/// documents (tens of blocks) a linear scan would be equally fast.
pub fn find_block_at_point(
    point: iced::Point,
    block_layouts: &[BlockLayout],
) -> Option<(usize, iced::Point)> {
    if block_layouts.is_empty() {
        return None;
    }

    // Binary search: find the last block whose y_offset <= point.y
    let layout_index = match block_layouts
        .binary_search_by(|bl| bl.y_offset.partial_cmp(&point.y).unwrap_or(std::cmp::Ordering::Equal))
    {
        Ok(exact) => exact,
        Err(0) => 0,
        Err(insert) => insert - 1,
    };

    let layout = block_layouts.get(layout_index)?;

    // Translate the point into the block's paragraph-local coordinate space.
    let local = iced::Point::new(
        point.x - layout.content_offset.x,
        point.y - layout.y_offset - layout.content_offset.y,
    );

    Some((layout_index, local))
}

/// Convert a pixel position (relative to the editor widget) to a
/// `DocPosition`.
///
/// This performs the block-finding step of hit testing. The caller (Widget
/// impl) must then call `Paragraph::hit_test()` on the identified block's
/// cached paragraph with the returned local point to get the character
/// offset.
///
/// Returns `(block_index, local_point)` where `block_index` is the document
/// block index and `local_point` is relative to the paragraph content origin.
pub fn hit_test(
    point: iced::Point,
    block_layouts: &[BlockLayout],
) -> Option<(usize, iced::Point)> {
    let (layout_idx, local_point) = find_block_at_point(point, block_layouts)?;
    let layout = block_layouts.get(layout_idx)?;
    Some((layout.block_index, local_point))
}

// ── Cursor position ────────────────────────────────────

/// Compute the block-level offset for rendering a cursor at `doc_pos`.
///
/// Returns the `BlockLayout` and the content offset so the Widget can call
/// `Paragraph::grapheme_position()` and then add the block-level offsets.
///
/// The full cursor position computation is:
/// 1. This function finds the block layout for `doc_pos.block_index`
/// 2. The Widget calls `grapheme_position(line, index)` on the paragraph
/// 3. The Widget adds `layout.y_offset + layout.content_offset.y` to the y
///    and `layout.content_offset.x` to the x
pub fn block_layout_for_position(
    doc_pos: DocPosition,
    block_layouts: &[BlockLayout],
) -> Option<&BlockLayout> {
    block_layouts
        .iter()
        .find(|bl| bl.block_index == doc_pos.block_index)
}

/// Construct a `CursorGeometry` from a paragraph-local grapheme position and
/// a block layout. This is the final assembly step after the Widget has
/// called `Paragraph::grapheme_position()`.
pub fn assemble_cursor_geometry(
    grapheme_x: f32,
    grapheme_y: f32,
    line_height: f32,
    layout: &BlockLayout,
) -> CursorGeometry {
    CursorGeometry {
        x: grapheme_x + layout.content_offset.x,
        y: grapheme_y + layout.y_offset + layout.content_offset.y,
        height: line_height,
    }
}

// ── Selection rectangles ───────────────────────────────

/// Describes how a single block participates in a selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockSelectionKind {
    /// The block is the only block in the selection (or selection is within
    /// one block). Both start and end offsets matter.
    Single {
        start_offset: usize,
        end_offset: usize,
    },
    /// The block is the first block in a multi-block selection.
    /// Selection runs from `start_offset` to the end of the block.
    First { start_offset: usize },
    /// The block is in the middle of a multi-block selection (fully selected).
    Full,
    /// The block is the last block in a multi-block selection.
    /// Selection runs from the start of the block to `end_offset`.
    Last { end_offset: usize },
}

/// Compute which blocks participate in a selection and how.
///
/// Returns `(block_index, BlockSelectionKind)` pairs. The Widget uses these
/// to call the appropriate paragraph-level selection rectangle computation
/// for each block, then offsets the results by the block's layout position.
pub fn selection_block_ranges(selection: DocSelection) -> Vec<(usize, BlockSelectionKind)> {
    if selection.is_collapsed() {
        return Vec::new();
    }

    let start = selection.start();
    let end = selection.end();
    let mut ranges = Vec::new();

    if start.block_index == end.block_index {
        ranges.push((
            start.block_index,
            BlockSelectionKind::Single {
                start_offset: start.offset,
                end_offset: end.offset,
            },
        ));
    } else {
        // First block
        ranges.push((
            start.block_index,
            BlockSelectionKind::First {
                start_offset: start.offset,
            },
        ));

        // Middle blocks (fully selected)
        for block_idx in (start.block_index + 1)..end.block_index {
            ranges.push((block_idx, BlockSelectionKind::Full));
        }

        // Last block
        ranges.push((
            end.block_index,
            BlockSelectionKind::Last {
                end_offset: end.offset,
            },
        ));
    }

    ranges
}

/// Compute selection rectangles for a fully-selected block. The Widget calls
/// this for `BlockSelectionKind::Full` blocks — the entire block width is
/// highlighted.
pub fn full_block_selection_rect(layout: &BlockLayout, editor_width: f32) -> SelectionRect {
    SelectionRect {
        x: 0.0,
        y: layout.y_offset,
        width: editor_width,
        height: layout.height,
    }
}

// ── Vertical cursor movement ───────────────────────────

/// Context returned by `prepare_move_up` / `prepare_move_down` to tell the
/// Widget what paragraph-level operation to perform.
#[derive(Debug, Clone)]
pub enum VerticalMoveResult {
    /// The cursor should move within the same block's paragraph. The Widget
    /// should use the paragraph's own vertical navigation (e.g., moving to
    /// the previous/next visual line at `target_x`).
    SameBlock {
        block_index: usize,
        target_x: f32,
    },
    /// The cursor should jump to a different block. The Widget should hit-test
    /// the target block's paragraph at `(target_x, local_y)` to find the
    /// character offset.
    CrossBlock {
        target_block_index: usize,
        target_x: f32,
        /// The y-coordinate within the target block's paragraph to hit-test.
        /// For move-up: the bottom of the last line (height - 1.0).
        /// For move-down: the top of the first line (0.0).
        local_y: f32,
    },
    /// No movement possible (already at document boundary).
    AtBoundary,
}

/// Prepare an upward cursor movement.
///
/// `is_first_line` should be `true` if the cursor is currently on the first
/// visual line of its block's paragraph. The Widget determines this by
/// checking if the cursor's line index within the paragraph is 0.
///
/// Returns instructions for the Widget to complete the movement.
pub fn prepare_move_up(
    current: DocPosition,
    block_layouts: &[BlockLayout],
    target_x: f32,
    is_first_line: bool,
) -> VerticalMoveResult {
    if !is_first_line {
        // Moving within the same block's paragraph — the Widget handles this
        // via paragraph-level line navigation.
        return VerticalMoveResult::SameBlock {
            block_index: current.block_index,
            target_x,
        };
    }

    // Find the layout index of the current block
    let current_layout_idx = block_layouts
        .iter()
        .position(|bl| bl.block_index == current.block_index);

    let Some(idx) = current_layout_idx else {
        return VerticalMoveResult::AtBoundary;
    };

    if idx == 0 {
        // Already at the first block — can't move up further.
        return VerticalMoveResult::AtBoundary;
    }

    let prev_layout = &block_layouts[idx - 1];

    // Target the bottom of the previous block's last line.
    // Using height - 1.0 to land on the last line rather than below it.
    let local_y = (prev_layout.height - prev_layout.content_offset.y - 1.0).max(0.0);

    VerticalMoveResult::CrossBlock {
        target_block_index: prev_layout.block_index,
        target_x,
        local_y,
    }
}

/// Prepare a downward cursor movement.
///
/// `is_last_line` should be `true` if the cursor is currently on the last
/// visual line of its block's paragraph.
///
/// Returns instructions for the Widget to complete the movement.
pub fn prepare_move_down(
    current: DocPosition,
    block_layouts: &[BlockLayout],
    target_x: f32,
    is_last_line: bool,
) -> VerticalMoveResult {
    if !is_last_line {
        return VerticalMoveResult::SameBlock {
            block_index: current.block_index,
            target_x,
        };
    }

    let current_layout_idx = block_layouts
        .iter()
        .position(|bl| bl.block_index == current.block_index);

    let Some(idx) = current_layout_idx else {
        return VerticalMoveResult::AtBoundary;
    };

    if idx + 1 >= block_layouts.len() {
        return VerticalMoveResult::AtBoundary;
    }

    let next_layout = &block_layouts[idx + 1];

    // Target the top of the next block's first line.
    VerticalMoveResult::CrossBlock {
        target_block_index: next_layout.block_index,
        target_x,
        local_y: 0.0,
    }
}

// ── Tests ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_layouts() -> Vec<BlockLayout> {
        vec![
            BlockLayout {
                y_offset: 0.0,
                height: 20.0,
                block_index: 0,
                content_offset: iced::Vector::new(0.0, 0.0),
            },
            BlockLayout {
                y_offset: 20.0,
                height: 30.0,
                block_index: 1,
                content_offset: iced::Vector::new(10.0, 0.0),
            },
            BlockLayout {
                y_offset: 50.0,
                height: 25.0,
                block_index: 2,
                content_offset: iced::Vector::new(0.0, 5.0),
            },
        ]
    }

    // ── CursorState tests ──────────────────────────────

    #[test]
    fn cursor_starts_visible_when_focused() {
        let mut cursor = CursorState::new();
        cursor.focus();
        assert!(cursor.is_visible());
    }

    #[test]
    fn cursor_blinks_after_interval() {
        let mut cursor = CursorState::new();
        cursor.focus();
        assert!(cursor.is_visible());

        cursor.tick(BLINK_INTERVAL);
        assert!(!cursor.is_visible());

        cursor.tick(BLINK_INTERVAL);
        assert!(cursor.is_visible());
    }

    #[test]
    fn cursor_reset_blink_makes_visible() {
        let mut cursor = CursorState::new();
        cursor.focus();
        cursor.tick(BLINK_INTERVAL); // now invisible
        assert!(!cursor.is_visible());

        cursor.reset_blink();
        assert!(cursor.is_visible());
    }

    #[test]
    fn cursor_unfocused_never_visible() {
        let mut cursor = CursorState::new();
        assert!(!cursor.is_visible());
        cursor.tick(0.1);
        assert!(!cursor.is_visible());
    }

    #[test]
    fn cursor_unfocus_hides() {
        let mut cursor = CursorState::new();
        cursor.focus();
        assert!(cursor.is_visible());
        cursor.unfocus();
        assert!(!cursor.is_visible());
    }

    #[test]
    fn tick_does_nothing_when_unfocused() {
        let mut cursor = CursorState::new();
        // Not focused, tick should not change state
        cursor.tick(10.0);
        assert!(!cursor.is_visible());
        assert!(!cursor.is_focused());
    }

    #[test]
    fn target_x_lifecycle() {
        let mut cursor = CursorState::new();
        assert!(cursor.target_x().is_none());

        cursor.set_target_x(42.5);
        assert_eq!(cursor.target_x(), Some(42.5));

        cursor.clear_target_x();
        assert!(cursor.target_x().is_none());
    }

    // ── DragState tests ────────────────────────────────

    #[test]
    fn drag_state_lifecycle() {
        let mut drag = DragState::start(DocPosition::new(1, 5));
        assert!(drag.active);
        assert_eq!(drag.anchor, DocPosition::new(1, 5));

        drag.end();
        assert!(!drag.active);
        assert_eq!(drag.anchor, DocPosition::new(1, 5)); // anchor preserved
    }

    // ── Hit testing ────────────────────────────────────

    #[test]
    fn hit_test_first_block() {
        let layouts = sample_layouts();
        let result = hit_test(iced::Point::new(15.0, 10.0), &layouts);
        assert!(result.is_some());
        let (block_idx, local) = result.expect("hit_test returned None");
        assert_eq!(block_idx, 0);
        assert!((local.x - 15.0).abs() < f32::EPSILON);
        assert!((local.y - 10.0).abs() < f32::EPSILON);
    }

    #[test]
    fn hit_test_second_block_with_content_offset() {
        let layouts = sample_layouts();
        // Click at (25.0, 30.0) — second block starts at y=20, has content_offset.x=10
        let result = hit_test(iced::Point::new(25.0, 30.0), &layouts);
        let (block_idx, local) = result.expect("hit_test returned None");
        assert_eq!(block_idx, 1);
        assert!((local.x - 15.0).abs() < f32::EPSILON); // 25 - 10
        assert!((local.y - 10.0).abs() < f32::EPSILON); // 30 - 20
    }

    #[test]
    fn hit_test_third_block_with_y_content_offset() {
        let layouts = sample_layouts();
        // Click at (10.0, 60.0) — third block at y=50, content_offset.y=5
        let result = hit_test(iced::Point::new(10.0, 60.0), &layouts);
        let (block_idx, local) = result.expect("hit_test returned None");
        assert_eq!(block_idx, 2);
        assert!((local.x - 10.0).abs() < f32::EPSILON);
        assert!((local.y - 5.0).abs() < f32::EPSILON); // 60 - 50 - 5
    }

    #[test]
    fn hit_test_empty_layouts() {
        let result = hit_test(iced::Point::new(10.0, 10.0), &[]);
        assert!(result.is_none());
    }

    #[test]
    fn hit_test_above_first_block() {
        let layouts = sample_layouts();
        // Click above the first block (negative conceptually, but y_offset=0
        // so it should still land on block 0)
        let result = hit_test(iced::Point::new(5.0, -5.0), &layouts);
        let (block_idx, _) = result.expect("hit_test returned None");
        assert_eq!(block_idx, 0);
    }

    // ── Block layout lookup ────────────────────────────

    #[test]
    fn block_layout_for_existing_position() {
        let layouts = sample_layouts();
        let layout = block_layout_for_position(DocPosition::new(1, 3), &layouts);
        assert!(layout.is_some());
        let bl = layout.expect("block_layout_for_position returned None");
        assert_eq!(bl.block_index, 1);
        assert!((bl.y_offset - 20.0).abs() < f32::EPSILON);
    }

    #[test]
    fn block_layout_for_missing_block() {
        let layouts = sample_layouts();
        let layout = block_layout_for_position(DocPosition::new(99, 0), &layouts);
        assert!(layout.is_none());
    }

    // ── Cursor geometry assembly ───────────────────────

    #[test]
    fn assemble_cursor_geometry_offsets_correctly() {
        let layout = BlockLayout {
            y_offset: 100.0,
            height: 20.0,
            block_index: 5,
            content_offset: iced::Vector::new(16.0, 4.0),
        };
        let geom = assemble_cursor_geometry(10.0, 2.0, 18.0, &layout);
        assert!((geom.x - 26.0).abs() < f32::EPSILON); // 10 + 16
        assert!((geom.y - 106.0).abs() < f32::EPSILON); // 2 + 100 + 4
        assert!((geom.height - 18.0).abs() < f32::EPSILON);
    }

    // ── Selection block ranges ─────────────────────────

    #[test]
    fn selection_collapsed_returns_empty() {
        let sel = DocSelection::caret(DocPosition::new(1, 5));
        assert!(selection_block_ranges(sel).is_empty());
    }

    #[test]
    fn selection_single_block() {
        let sel = DocSelection::range(DocPosition::new(2, 3), DocPosition::new(2, 10));
        let ranges = selection_block_ranges(sel);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].0, 2);
        assert_eq!(
            ranges[0].1,
            BlockSelectionKind::Single {
                start_offset: 3,
                end_offset: 10,
            }
        );
    }

    #[test]
    fn selection_multi_block() {
        let sel = DocSelection::range(DocPosition::new(1, 5), DocPosition::new(4, 2));
        let ranges = selection_block_ranges(sel);
        assert_eq!(ranges.len(), 4); // block 1 (first), 2 (full), 3 (full), 4 (last)

        assert_eq!(ranges[0].0, 1);
        assert_eq!(
            ranges[0].1,
            BlockSelectionKind::First { start_offset: 5 }
        );

        assert_eq!(ranges[1].0, 2);
        assert_eq!(ranges[1].1, BlockSelectionKind::Full);

        assert_eq!(ranges[2].0, 3);
        assert_eq!(ranges[2].1, BlockSelectionKind::Full);

        assert_eq!(ranges[3].0, 4);
        assert_eq!(
            ranges[3].1,
            BlockSelectionKind::Last { end_offset: 2 }
        );
    }

    #[test]
    fn selection_backward_normalizes() {
        // Backward selection (focus before anchor) should produce the same
        // block ranges as a forward selection.
        let sel = DocSelection::range(DocPosition::new(3, 0), DocPosition::new(1, 5));
        let ranges = selection_block_ranges(sel);
        assert_eq!(ranges.len(), 3);
        assert_eq!(ranges[0].0, 1);
        assert_eq!(
            ranges[0].1,
            BlockSelectionKind::First { start_offset: 5 }
        );
        assert_eq!(ranges[2].0, 3);
        assert_eq!(
            ranges[2].1,
            BlockSelectionKind::Last { end_offset: 0 }
        );
    }

    // ── Full block selection rect ──────────────────────

    #[test]
    fn full_block_selection_rect_covers_width() {
        let layout = BlockLayout {
            y_offset: 40.0,
            height: 20.0,
            block_index: 2,
            content_offset: iced::Vector::new(0.0, 0.0),
        };
        let rect = full_block_selection_rect(&layout, 500.0);
        assert!((rect.x).abs() < f32::EPSILON);
        assert!((rect.y - 40.0).abs() < f32::EPSILON);
        assert!((rect.width - 500.0).abs() < f32::EPSILON);
        assert!((rect.height - 20.0).abs() < f32::EPSILON);
    }

    // ── Vertical movement ──────────────────────────────

    #[test]
    fn move_up_same_block_when_not_first_line() {
        let layouts = sample_layouts();
        let result = prepare_move_up(
            DocPosition::new(1, 5),
            &layouts,
            42.0,
            false, // not on first line
        );
        match result {
            VerticalMoveResult::SameBlock {
                block_index,
                target_x,
            } => {
                assert_eq!(block_index, 1);
                assert!((target_x - 42.0).abs() < f32::EPSILON);
            }
            other => panic!("Expected SameBlock, got {other:?}"),
        }
    }

    #[test]
    fn move_up_cross_block_from_first_line() {
        let layouts = sample_layouts();
        let result = prepare_move_up(
            DocPosition::new(1, 5),
            &layouts,
            42.0,
            true, // on first line
        );
        match result {
            VerticalMoveResult::CrossBlock {
                target_block_index,
                target_x,
                local_y,
            } => {
                assert_eq!(target_block_index, 0);
                assert!((target_x - 42.0).abs() < f32::EPSILON);
                // Previous block height=20, content_offset.y=0 → local_y = 19.0
                assert!((local_y - 19.0).abs() < f32::EPSILON);
            }
            other => panic!("Expected CrossBlock, got {other:?}"),
        }
    }

    #[test]
    fn move_up_at_top_boundary() {
        let layouts = sample_layouts();
        let result = prepare_move_up(
            DocPosition::new(0, 0),
            &layouts,
            42.0,
            true,
        );
        assert!(matches!(result, VerticalMoveResult::AtBoundary));
    }

    #[test]
    fn move_down_same_block_when_not_last_line() {
        let layouts = sample_layouts();
        let result = prepare_move_down(
            DocPosition::new(0, 5),
            &layouts,
            42.0,
            false,
        );
        match result {
            VerticalMoveResult::SameBlock {
                block_index,
                target_x,
            } => {
                assert_eq!(block_index, 0);
                assert!((target_x - 42.0).abs() < f32::EPSILON);
            }
            other => panic!("Expected SameBlock, got {other:?}"),
        }
    }

    #[test]
    fn move_down_cross_block_from_last_line() {
        let layouts = sample_layouts();
        let result = prepare_move_down(
            DocPosition::new(0, 5),
            &layouts,
            42.0,
            true,
        );
        match result {
            VerticalMoveResult::CrossBlock {
                target_block_index,
                target_x,
                local_y,
            } => {
                assert_eq!(target_block_index, 1);
                assert!((target_x - 42.0).abs() < f32::EPSILON);
                assert!(local_y.abs() < f32::EPSILON); // top of next block
            }
            other => panic!("Expected CrossBlock, got {other:?}"),
        }
    }

    #[test]
    fn move_down_at_bottom_boundary() {
        let layouts = sample_layouts();
        let result = prepare_move_down(
            DocPosition::new(2, 0),
            &layouts,
            42.0,
            true,
        );
        assert!(matches!(result, VerticalMoveResult::AtBoundary));
    }

    #[test]
    fn move_up_with_content_offset() {
        let layouts = sample_layouts();
        // Block 2 has content_offset.y = 5.0, height = 25.0
        let result = prepare_move_up(
            DocPosition::new(2, 0),
            &layouts,
            10.0,
            true,
        );
        match result {
            VerticalMoveResult::CrossBlock {
                target_block_index,
                local_y,
                ..
            } => {
                assert_eq!(target_block_index, 1);
                // Block 1: height=30, content_offset.y=0 → local_y = 29.0
                assert!((local_y - 29.0).abs() < f32::EPSILON);
            }
            other => panic!("Expected CrossBlock, got {other:?}"),
        }
    }
}
