//! Paragraph caching, draw logic, block-level rendering.
//!
//! Provides:
//! - [`build_spans_for_block`] — converts a [`Block`]'s [`StyledRun`]s to iced [`Span`]s
//! - [`ParagraphCache`] — stores pre-built paragraphs per block with dirty flags
//! - Drawing helpers for horizontal rules, blockquote borders, and list markers

use crate::document::{Block, HeadingLevel, InlineStyle, StyledRun};

use iced::advanced::text::{Paragraph, Span, Text};
use iced::advanced::renderer;
use iced::{Color, Font, Point, Rectangle, Size};

// ── Font size constants ─────────────────────────────────
//
// These mirror the app's layout.rs type scale. They live here so the
// editor crate is self-contained (no dependency on the app crate).

/// Body text / paragraph font size (px).
pub const FONT_SIZE_BODY: f32 = 13.0;
/// Heading H3 font size (px).
pub const FONT_SIZE_H3: f32 = 14.0;
/// Heading H2 font size (px).
pub const FONT_SIZE_H2: f32 = 16.0;
/// Heading H1 font size (px).
pub const FONT_SIZE_H1: f32 = 18.0;

/// Default line-height multiplier (relative).
pub const LINE_HEIGHT_MULTIPLIER: f32 = 1.4;

// ── Block spacing constants ─────────────────────────────

/// Vertical spacing after a paragraph block (px).
pub const SPACING_PARAGRAPH: f32 = 8.0;
/// Vertical spacing after a heading block (px).
pub const SPACING_HEADING: f32 = 12.0;
/// Vertical spacing after a list block (px).
pub const SPACING_LIST: f32 = 8.0;
/// Vertical spacing after a blockquote block (px).
pub const SPACING_BLOCKQUOTE: f32 = 12.0;
/// Vertical spacing after a horizontal rule (px).
pub const SPACING_HR: f32 = 12.0;
/// Vertical spacing between list items (px).
pub const SPACING_LIST_ITEM: f32 = 4.0;

// ── Blockquote constants ────────────────────────────────

/// Width of the blockquote left border (px).
pub const BLOCKQUOTE_BORDER_WIDTH: f32 = 2.0;
/// Horizontal indent for blockquote content (px).
pub const BLOCKQUOTE_INDENT: f32 = 16.0;

// ── List constants ──────────────────────────────────────

/// Width of the leading container for bullet/number (px).
pub const LIST_MARKER_WIDTH: f32 = 24.0;
/// Additional indent per nesting level (px).
pub const LIST_INDENT_PER_LEVEL: f32 = 24.0;

/// The bullet character for unordered lists.
const BULLET_CHAR: &str = "\u{2022}"; // •

// ── Horizontal rule constants ───────────────────────────

/// Height of the horizontal rule line (px).
pub const HR_LINE_HEIGHT: f32 = 1.0;
/// Total height of the horizontal rule block including padding (px).
pub const HR_BLOCK_HEIGHT: f32 = 16.0;

// ── Image placeholder constants ──────────────────────────

/// Default placeholder height for image blocks (px).
pub const IMAGE_PLACEHOLDER_HEIGHT: f32 = 150.0;
/// Padding inside the image placeholder rectangle (px).
pub const IMAGE_PLACEHOLDER_PADDING: f32 = 8.0;

// ── Font size resolution ────────────────────────────────

/// Returns the font size in pixels for a given block.
pub fn block_font_size(block: &Block) -> f32 {
    match block {
        Block::Heading { level, .. } => heading_font_size(*level),
        Block::Paragraph { .. } => FONT_SIZE_BODY,
        Block::ListItem { .. } => FONT_SIZE_BODY,
        Block::BlockQuote { .. } => FONT_SIZE_BODY,
        Block::HorizontalRule => FONT_SIZE_BODY,
        Block::Image { .. } => FONT_SIZE_BODY,
    }
}

/// Returns the font size for a heading level.
pub fn heading_font_size(level: HeadingLevel) -> f32 {
    match level {
        HeadingLevel::H1 => FONT_SIZE_H1,
        HeadingLevel::H2 => FONT_SIZE_H2,
        HeadingLevel::H3 => FONT_SIZE_H3,
    }
}

/// Returns the vertical spacing after a block (px).
pub fn block_spacing(block: &Block) -> f32 {
    match block {
        Block::Paragraph { .. } => SPACING_PARAGRAPH,
        Block::Heading { .. } => SPACING_HEADING,
        Block::ListItem { .. } => SPACING_LIST,
        Block::BlockQuote { .. } => SPACING_BLOCKQUOTE,
        Block::HorizontalRule => SPACING_HR,
        Block::Image { .. } => SPACING_PARAGRAPH,
    }
}

// ── Span building ───────────────────────────────────────

/// Resolve the iced [`Font`] for an inline style.
///
/// `base_font` is the font family to use (typically the editor's configured
/// body font). Bold/italic/bold-italic variants are derived from it.
pub fn font_for_style(base_font: Font, style: InlineStyle) -> Font {
    let bold = style.contains(InlineStyle::BOLD);
    let italic = style.contains(InlineStyle::ITALIC);
    match (bold, italic) {
        (true, true) => Font {
            weight: iced::font::Weight::Bold,
            style: iced::font::Style::Italic,
            ..base_font
        },
        (true, false) => Font {
            weight: iced::font::Weight::Bold,
            ..base_font
        },
        (false, true) => Font {
            style: iced::font::Style::Italic,
            ..base_font
        },
        (false, false) => base_font,
    }
}

/// Convert a single [`StyledRun`] to an iced [`Span`].
///
/// The `Link` type parameter is `String` — the href of the link, if any.
/// `base_font` is the editor's configured body font. `font_size` is the
/// size for this block (e.g. heading size or body size).
/// `link_color` is the color to use for linked spans.
pub fn run_to_span<'a>(
    run: &'a StyledRun,
    base_font: Font,
    font_size: f32,
    text_color: Color,
    link_color: Color,
) -> Span<'a, String, Font> {
    let font = font_for_style(base_font, run.style);
    let underline =
        run.style.contains(InlineStyle::UNDERLINE) || run.link.is_some();
    let strikethrough = run.style.contains(InlineStyle::STRIKETHROUGH);

    let color = if run.link.is_some() {
        link_color
    } else {
        text_color
    };

    let mut span = Span::new(run.text.as_str())
        .font(font)
        .size(font_size)
        .color(color)
        .underline(underline)
        .strikethrough(strikethrough);

    if let Some(href) = &run.link {
        span = span.link(href.clone());
    }

    span
}

/// Build iced [`Span`]s for all runs in a block.
///
/// Returns an empty vec for blocks that have no inline content
/// (e.g. `HorizontalRule`, `List`, `BlockQuote`).
pub fn build_spans_for_block<'a>(
    block: &'a Block,
    base_font: Font,
    text_color: Color,
    link_color: Color,
) -> Vec<Span<'a, String, Font>> {
    let Some(runs) = block.runs() else {
        return Vec::new();
    };

    let font_size = block_font_size(block);

    runs.iter()
        .map(|run| run_to_span(run, base_font, font_size, text_color, link_color))
        .collect()
}

/// Build spans for a block, handling container blocks by recursively
/// collecting styled spans from their children. Unlike [`build_spans_for_block`],
/// this never returns empty for valid blocks — container blocks (List,
/// BlockQuote) produce spans that preserve inline formatting and links.
pub fn build_spans_for_any_block(
    block: &Block,
    base_font: Font,
    text_color: Color,
    link_color: Color,
) -> Vec<Span<'static, String, Font>> {
    // Try the normal path first (works for Paragraph, Heading).
    if let Some(runs) = block.runs() {
        let font_size = block_font_size(block);
        return runs
            .iter()
            .map(|run| owned_run_to_span(run, base_font, font_size, text_color, link_color))
            .collect();
    }

    // For container blocks, recursively collect spans from children.
    let mut spans = Vec::new();
    collect_container_spans(block, base_font, text_color, link_color, &mut spans, false);
    spans
}

/// Recursively collect styled spans from a container block's children,
/// preserving inline formatting and links.
fn collect_container_spans(
    block: &Block,
    base_font: Font,
    text_color: Color,
    link_color: Color,
    spans: &mut Vec<Span<'static, String, Font>>,
    needs_separator: bool,
) {
    if needs_separator && !spans.is_empty() {
        spans.push(
            Span::new("\n".to_owned())
                .font(base_font)
                .size(FONT_SIZE_BODY)
                .color(text_color),
        );
    }

    match block {
        Block::Paragraph { runs }
        | Block::Heading { runs, .. }
        | Block::ListItem { runs, .. } => {
            let font_size = block_font_size(block);
            for run in runs {
                spans.push(owned_run_to_span(run, base_font, font_size, text_color, link_color));
            }
        }
        Block::BlockQuote { blocks } => {
            for (i, child) in blocks.iter().enumerate() {
                collect_container_spans(child, base_font, text_color, link_color, spans, i > 0);
            }
        }
        Block::HorizontalRule | Block::Image { .. } => {}
    }
}

/// Like [`run_to_span`] but returns an owned Span (for container block flattening
/// where we can't borrow from the block).
fn owned_run_to_span(
    run: &StyledRun,
    base_font: Font,
    font_size: f32,
    text_color: Color,
    link_color: Color,
) -> Span<'static, String, Font> {
    let font = font_for_style(base_font, run.style);
    let underline = run.style.contains(InlineStyle::UNDERLINE) || run.link.is_some();
    let strikethrough = run.style.contains(InlineStyle::STRIKETHROUGH);
    let color = if run.link.is_some() {
        link_color
    } else {
        text_color
    };

    let mut span = Span::new(run.text.clone())
        .font(font)
        .size(font_size)
        .color(color)
        .underline(underline)
        .strikethrough(strikethrough);

    if let Some(href) = &run.link {
        span = span.link(href.clone());
    }

    span
}

// ── Paragraph cache ─────────────────────────────────────

/// A laid-out paragraph for a child element within a container block
/// (list item or blockquote child).
pub struct ChildParagraph<P: Paragraph<Font = Font>> {
    /// The laid-out paragraph for this child.
    pub paragraph: P,
    /// Y offset relative to the container block's top edge (px).
    pub local_y_offset: f32,
    /// Height of this child paragraph (px).
    pub height: f32,
}

/// A cached paragraph for a single document block.
pub struct CacheEntry<P: Paragraph<Font = Font>> {
    /// The laid-out paragraph. `None` if the block has no inline content
    /// (e.g. `HorizontalRule`) or if it is a container block that uses
    /// `child_paragraphs` instead.
    paragraph: Option<P>,
    /// Per-child paragraphs for container blocks (List, BlockQuote).
    /// Empty for non-container blocks.
    child_paragraphs: Vec<ChildParagraph<P>>,
    /// Whether this entry needs re-layout on the next frame.
    dirty: bool,
    /// Y offset from the top of the editor widget (px).
    y_offset: f32,
    /// Height of this block including its content (px).
    height: f32,
}

impl<P: Paragraph<Font = Font>> Default for CacheEntry<P> {
    fn default() -> Self {
        Self {
            paragraph: None,
            child_paragraphs: Vec::new(),
            dirty: true,
            y_offset: 0.0,
            height: 0.0,
        }
    }
}

impl<P: Paragraph<Font = Font>> CacheEntry<P> {
    /// The pre-laid-out paragraph, if this block has inline content.
    pub fn paragraph(&self) -> Option<&P> {
        self.paragraph.as_ref()
    }

    /// Per-child paragraphs for container blocks (List, BlockQuote).
    /// Empty for non-container blocks.
    pub fn child_paragraphs(&self) -> &[ChildParagraph<P>] {
        &self.child_paragraphs
    }

    /// Whether this entry is dirty (needs re-layout).
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Y offset from the top of the editor widget.
    pub fn y_offset(&self) -> f32 {
        self.y_offset
    }

    /// Height of this block's rendered content.
    pub fn height(&self) -> f32 {
        self.height
    }
}

/// Paragraph cache: one [`CacheEntry`] per document block.
///
/// The cache is rebuilt (or partially updated) during the widget's `layout()`
/// pass. Only dirty entries are re-laid-out; clean entries keep their existing
/// paragraph and just have their y-offsets recomputed.
pub struct ParagraphCache<P: Paragraph<Font = Font>> {
    entries: Vec<CacheEntry<P>>,
    /// Cached total height from the last `layout()` call.
    last_layout_height: f32,
}

impl<P: Paragraph<Font = Font>> Default for ParagraphCache<P> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            last_layout_height: 0.0,
        }
    }
}

impl<P: Paragraph<Font = Font>> ParagraphCache<P> {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Access the cache entry for block `index`.
    pub fn get(&self, index: usize) -> Option<&CacheEntry<P>> {
        self.entries.get(index)
    }

    /// Mark a single block as dirty (needs re-layout).
    pub fn mark_dirty(&mut self, index: usize) {
        if let Some(entry) = self.entries.get_mut(index) {
            entry.dirty = true;
        }
    }

    /// Mark all entries as dirty.
    pub fn mark_all_dirty(&mut self) {
        for entry in &mut self.entries {
            entry.dirty = true;
        }
    }

    /// Resize the cache to match the document's block count.
    ///
    /// New entries are created as dirty. Excess entries are removed.
    /// Existing entries at unchanged indices are preserved.
    pub fn resize(&mut self, block_count: usize) {
        match self.entries.len().cmp(&block_count) {
            std::cmp::Ordering::Less => {
                self.entries
                    .resize_with(block_count, CacheEntry::default);
            }
            std::cmp::Ordering::Greater => {
                self.entries.truncate(block_count);
            }
            std::cmp::Ordering::Equal => {}
        }
    }

    /// Mark a block as dirty and resize if the block was inserted.
    ///
    /// Call after inserting a new block at `index`.
    pub fn insert_dirty(&mut self, index: usize) {
        if index <= self.entries.len() {
            self.entries.insert(index, CacheEntry::default());
        }
    }

    /// Remove the cache entry at `index`.
    ///
    /// Call after removing a block from the document.
    pub fn remove(&mut self, index: usize) {
        if index < self.entries.len() {
            self.entries.remove(index);
        }
    }

    /// Rebuild dirty paragraphs and recompute all y-offsets.
    ///
    /// `available_width` is the width available for text layout.
    /// `blocks` is the slice of blocks from the document (must match the
    /// cache length — call [`resize`] first).
    /// `base_font` is the editor's configured body font.
    /// `text_color` and `link_color` are the colors for normal and linked text.
    ///
    /// Returns the total height of all blocks.
    pub fn layout(
        &mut self,
        blocks: &[impl AsRef<Block>],
        available_width: f32,
        base_font: Font,
        text_color: Color,
        link_color: Color,
    ) -> f32 {
        self.resize(blocks.len());
        self.mark_all_dirty();

        let mut y = 0.0f32;

        for (i, block_ref) in blocks.iter().enumerate() {
            let block = block_ref.as_ref();
            let entry = &mut self.entries[i];
            entry.y_offset = y;

            if entry.dirty {
                entry.height = layout_block::<P>(
                    entry,
                    block,
                    available_width,
                    base_font,
                    text_color,
                    link_color,
                );
                entry.dirty = false;
            }

            y += entry.height + block_spacing(block);
        }

        self.last_layout_height = y;
        y
    }

    /// Total height of all cached blocks (including spacing).
    ///
    /// This is the value returned by the last [`layout`] call. If no layout
    /// has been performed, returns 0.
    pub fn total_height(&self) -> f32 {
        self.last_layout_height
    }

    /// Find the block index at a given y-coordinate (relative to the editor top).
    ///
    /// Returns `None` if the cache is empty. If `y` is below the last block,
    /// returns the last block index.
    pub fn block_at_y(&self, y: f32) -> Option<usize> {
        if self.entries.is_empty() {
            return None;
        }

        // Binary search: find the last entry whose y_offset <= y
        let mut lo = 0usize;
        let mut hi = self.entries.len();

        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.entries[mid].y_offset <= y {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        // lo is now the first entry with y_offset > y, so the block is lo - 1
        Some(lo.saturating_sub(1))
    }

    /// Iterate over all cache entries.
    pub fn entries(&self) -> &[CacheEntry<P>] {
        &self.entries
    }
}

// ── Block layout helper ─────────────────────────────────

/// Lay out a single block, storing the paragraph in `entry`.
/// Returns the height of the block.
fn layout_block<P: Paragraph<Font = Font>>(
    entry: &mut CacheEntry<P>,
    block: &Block,
    available_width: f32,
    base_font: Font,
    text_color: Color,
    link_color: Color,
) -> f32 {
    match block {
        Block::Paragraph { .. } | Block::Heading { .. } | Block::ListItem { .. } => {
            let content_width = if let Block::ListItem { indent_level, .. } = block {
                let indent = LIST_MARKER_WIDTH
                    + (*indent_level as f32) * LIST_INDENT_PER_LEVEL;
                (available_width - indent).max(0.0)
            } else {
                available_width
            };
            let spans = build_spans_for_block(block, base_font, text_color, link_color);
            let font_size = block_font_size(block);

            let paragraph = build_paragraph::<P>(
                &spans,
                content_width,
                base_font,
                font_size,
            );

            let height = paragraph.min_bounds().height;
            entry.paragraph = Some(paragraph);
            entry.child_paragraphs.clear();
            height
        }
        Block::HorizontalRule => {
            entry.paragraph = None;
            entry.child_paragraphs.clear();
            HR_BLOCK_HEIGHT
        }
        Block::Image { alt, height, .. } => {
            // Lay out alt text as a simple paragraph for placeholder display.
            let placeholder_text = if alt.is_empty() {
                "[image]"
            } else {
                alt.as_str()
            };
            let runs = [StyledRun::plain(placeholder_text)];
            let spans: Vec<_> = runs
                .iter()
                .map(|run| run_to_span(run, base_font, FONT_SIZE_BODY, text_color, link_color))
                .collect();
            let paragraph = build_paragraph::<P>(
                &spans,
                available_width - IMAGE_PLACEHOLDER_PADDING * 2.0,
                base_font,
                FONT_SIZE_BODY,
            );

            let text_height = paragraph.min_bounds().height;
            let block_height = height
                .map(|h| h as f32)
                .unwrap_or(IMAGE_PLACEHOLDER_HEIGHT)
                .max(text_height + IMAGE_PLACEHOLDER_PADDING * 2.0);

            entry.paragraph = Some(paragraph);
            entry.child_paragraphs.clear();
            block_height
        }
        Block::BlockQuote { blocks: bq_children } => {
            // Lay out each child block as a separate paragraph within a
            // narrower width, storing them as child_paragraphs.
            let inner_width = (available_width - BLOCKQUOTE_INDENT).max(0.0);
            let mut children = Vec::with_capacity(bq_children.len());
            let mut y = 0.0f32;

            for (i, child) in bq_children.iter().enumerate() {
                let child_spans = build_spans_for_any_block(
                    child.as_ref(),
                    base_font,
                    text_color,
                    link_color,
                );
                let child_font_size = block_font_size(child.as_ref());
                let child_para = build_paragraph::<P>(
                    &child_spans,
                    inner_width,
                    base_font,
                    child_font_size,
                );
                let h = child_para.min_bounds().height;
                children.push(ChildParagraph {
                    paragraph: child_para,
                    local_y_offset: y,
                    height: h,
                });
                y += h;

                if i + 1 < bq_children.len() {
                    y += SPACING_PARAGRAPH;
                }
            }

            entry.paragraph = None;
            entry.child_paragraphs = children;
            y
        }
    }
}

/// Build an iced [`Paragraph`] from a set of spans.
fn build_paragraph<P: Paragraph<Font = Font>>(
    spans: &[Span<'_, String, Font>],
    available_width: f32,
    base_font: Font,
    font_size: f32,
) -> P {
    let text = Text {
        content: spans,
        bounds: Size::new(available_width, f32::INFINITY),
        size: iced::Pixels(font_size),
        line_height: iced::advanced::text::LineHeight::Relative(LINE_HEIGHT_MULTIPLIER),
        font: base_font,
        align_x: iced::advanced::text::Alignment::Default,
        align_y: iced::alignment::Vertical::Top,
        shaping: iced::advanced::text::Shaping::Advanced,
        wrapping: iced::advanced::text::Wrapping::Word,
        ellipsis: iced::advanced::text::Ellipsis::default(),
        hint_factor: None,
    };
    P::with_spans(text)
}

// ── Drawing helpers ─────────────────────────────────────

/// Draw a horizontal rule across the given bounds.
///
/// Renders a 1px line horizontally centered within `bounds`.
pub fn draw_horizontal_rule<R: iced::advanced::Renderer>(
    renderer: &mut R,
    bounds: Rectangle,
    color: Color,
) {
    let y_center = bounds.y + bounds.height / 2.0;
    renderer.fill_quad(
        renderer::Quad {
            bounds: Rectangle::new(
                Point::new(bounds.x, y_center - HR_LINE_HEIGHT / 2.0),
                Size::new(bounds.width, HR_LINE_HEIGHT),
            ),
            ..Default::default()
        },
        color,
    );
}

/// Draw the left border for a blockquote.
///
/// Renders a vertical line on the left side of the blockquote bounds.
pub fn draw_blockquote_border<R: iced::advanced::Renderer>(
    renderer: &mut R,
    bounds: Rectangle,
    color: Color,
) {
    renderer.fill_quad(
        renderer::Quad {
            bounds: Rectangle::new(
                Point::new(bounds.x, bounds.y),
                Size::new(BLOCKQUOTE_BORDER_WIDTH, bounds.height),
            ),
            ..Default::default()
        },
        color,
    );
}

/// Draw a list item marker (bullet or number).
///
/// For unordered lists, draws a bullet character. For ordered lists, draws
/// the item number. The marker is drawn in a fixed-width leading container
/// to the left of `content_bounds`.
///
/// `item_index` is the 0-based index of the item in the list.
pub fn draw_list_marker<R>(
    renderer: &mut R,
    content_bounds: Rectangle,
    ordered: bool,
    item_index: usize,
    base_font: Font,
    text_color: Color,
    clip_bounds: Rectangle,
) where
    R: iced::advanced::Renderer + iced::advanced::text::Renderer<Font = Font>,
{
    let marker_text = if ordered {
        let mut s = (item_index + 1).to_string();
        s.push('.');
        s
    } else {
        BULLET_CHAR.to_owned()
    };

    let marker_para = <R::Paragraph as Paragraph>::with_text(Text {
        content: marker_text.as_str(),
        bounds: Size::new(LIST_MARKER_WIDTH, f32::INFINITY),
        size: iced::Pixels(FONT_SIZE_BODY),
        line_height: iced::advanced::text::LineHeight::Relative(LINE_HEIGHT_MULTIPLIER),
        font: base_font,
        align_x: iced::advanced::text::Alignment::Right,
        align_y: iced::alignment::Vertical::Top,
        shaping: iced::advanced::text::Shaping::Advanced,
        wrapping: iced::advanced::text::Wrapping::None,
        ellipsis: iced::advanced::text::Ellipsis::default(),
        hint_factor: None,
    });

    let marker_x = content_bounds.x - LIST_MARKER_WIDTH;
    renderer.fill_paragraph(
        &marker_para,
        Point::new(marker_x, content_bounds.y),
        text_color,
        clip_bounds,
    );
}

/// Draw a filled paragraph at a given position.
///
/// Convenience wrapper around `renderer.fill_paragraph()`.
pub fn draw_paragraph<R>(
    renderer: &mut R,
    paragraph: &R::Paragraph,
    position: Point,
    text_color: Color,
    clip_bounds: Rectangle,
) where
    R: iced::advanced::text::Renderer,
{
    renderer.fill_paragraph(paragraph, position, text_color, clip_bounds);
}

// ── AsRef<Block> for Arc<Block> ─────────────────────────
// Already implemented by std, but we need it for the layout method.
// `Arc<Block>` implements `AsRef<Block>` automatically.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_size_constants_match_spec() {
        assert!((FONT_SIZE_BODY - 13.0).abs() < f32::EPSILON);
        assert!((FONT_SIZE_H1 - 18.0).abs() < f32::EPSILON);
        assert!((FONT_SIZE_H2 - 16.0).abs() < f32::EPSILON);
        assert!((FONT_SIZE_H3 - 14.0).abs() < f32::EPSILON);
    }

    #[test]
    fn block_font_size_paragraph() {
        let block = Block::empty_paragraph();
        assert!((block_font_size(&block) - FONT_SIZE_BODY).abs() < f32::EPSILON);
    }

    #[test]
    fn block_font_size_headings() {
        let h1 = Block::Heading {
            level: HeadingLevel::H1,
            runs: vec![StyledRun::plain("test")],
        };
        assert!((block_font_size(&h1) - FONT_SIZE_H1).abs() < f32::EPSILON);

        let h2 = Block::Heading {
            level: HeadingLevel::H2,
            runs: vec![StyledRun::plain("test")],
        };
        assert!((block_font_size(&h2) - FONT_SIZE_H2).abs() < f32::EPSILON);

        let h3 = Block::Heading {
            level: HeadingLevel::H3,
            runs: vec![StyledRun::plain("test")],
        };
        assert!((block_font_size(&h3) - FONT_SIZE_H3).abs() < f32::EPSILON);
    }

    #[test]
    fn font_for_style_variants() {
        let base = Font::DEFAULT;

        let plain = font_for_style(base, InlineStyle::empty());
        assert_eq!(plain.weight, iced::font::Weight::Normal);
        assert_eq!(plain.style, iced::font::Style::Normal);

        let bold = font_for_style(base, InlineStyle::BOLD);
        assert_eq!(bold.weight, iced::font::Weight::Bold);
        assert_eq!(bold.style, iced::font::Style::Normal);

        let italic = font_for_style(base, InlineStyle::ITALIC);
        assert_eq!(italic.weight, iced::font::Weight::Normal);
        assert_eq!(italic.style, iced::font::Style::Italic);

        let bold_italic = font_for_style(base, InlineStyle::BOLD | InlineStyle::ITALIC);
        assert_eq!(bold_italic.weight, iced::font::Weight::Bold);
        assert_eq!(bold_italic.style, iced::font::Style::Italic);
    }

    #[test]
    fn block_spacing_values() {
        assert!(block_spacing(&Block::empty_paragraph()) > 0.0);
        assert!(block_spacing(&Block::HorizontalRule) > 0.0);
    }

    #[test]
    fn block_font_size_list_item() {
        let block = Block::list_item("item", false);
        assert!((block_font_size(&block) - FONT_SIZE_BODY).abs() < f32::EPSILON);
    }

    #[test]
    fn block_font_size_blockquote() {
        let block = Block::BlockQuote {
            blocks: vec![std::sync::Arc::new(Block::paragraph("quoted"))],
        };
        assert!((block_font_size(&block) - FONT_SIZE_BODY).abs() < f32::EPSILON);
    }
}
