//! DOM-to-widget HTML email rendering pipeline.
//!
//! Parses HTML into a lightweight block structure, then emits iced widgets.
//! Handles: paragraphs, links, lists, blockquotes, pre/code blocks, images
//! (alt text), headings, horizontal rules, and basic tables.
//!
//! Includes a complexity heuristic for fallback detection - emails with deep
//! table nesting (layout tables) or heavy style blocks fall back to plain text.

use std::collections::HashMap;

use iced::widget::{Space, column, container, image, row, text};
use iced::{Element, Length, Padding};

use crate::ui::layout::*;
use crate::ui::theme;

/// Maximum CSS/table depth threshold for complexity.
const COMPLEXITY_TABLE_DEPTH_THRESHOLD: usize = 5;

/// Complexity assessment of an HTML document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HtmlComplexity {
    /// Simple enough for native widget rendering.
    Simple,
    /// Too complex; should fall back to text-only display.
    Complex,
}

/// Assess whether an HTML email is simple enough for native rendering.
#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn assess_complexity(html: &str) -> HtmlComplexity {
    let lower = html.to_lowercase();
    let mut table_depth: usize = 0;
    let mut max_table_depth: usize = 0;
    let mut style_tag_count: usize = 0;

    let bytes = lower.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'<' {
            if starts_with_at(&lower, i, "<table") {
                table_depth += 1;
                if table_depth > max_table_depth {
                    max_table_depth = table_depth;
                }
            } else if starts_with_at(&lower, i, "</table") {
                table_depth = table_depth.saturating_sub(1);
            } else if starts_with_at(&lower, i, "<style") {
                style_tag_count += 1;
            }
        }
        i += 1;
    }

    if max_table_depth > COMPLEXITY_TABLE_DEPTH_THRESHOLD || style_tag_count > 2 {
        HtmlComplexity::Complex
    } else {
        HtmlComplexity::Simple
    }
}

fn starts_with_at(haystack: &str, pos: usize, prefix: &str) -> bool {
    haystack.get(pos..).is_some_and(|s| s.starts_with(prefix))
}

/// Pre-parsed HTML body, cached to avoid re-parsing on every view cycle.
pub(super) enum CachedHtmlBody {
    /// HTML was too complex; render as plain text fallback.
    Complex,
    /// Parsed block structure ready for rendering.
    Blocks(Vec<Block>),
    /// Empty HTML body.
    Empty,
}

/// Pre-parse an HTML body into a cached block structure.
///
/// Call once when thread detail loads; store the result and use
/// `render_cached_html` on each view cycle.
#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub(super) fn preparse_html(html: &str) -> CachedHtmlBody {
    if assess_complexity(html) == HtmlComplexity::Complex {
        return CachedHtmlBody::Complex;
    }
    let blocks = parse_html_to_blocks(html);
    if blocks.is_empty() {
        CachedHtmlBody::Empty
    } else {
        CachedHtmlBody::Blocks(blocks)
    }
}

/// Render pre-parsed HTML blocks to iced widgets, using fallback text for
/// complex/empty HTML. Avoids re-parsing HTML on every view cycle.
#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn render_cached_html<'a, M: Clone + 'a>(
    cached: &CachedHtmlBody,
    fallback_text: Option<&str>,
    on_link_click: impl Fn(String) -> M + 'a,
    inline_images: &'a HashMap<String, Vec<u8>>,
) -> Element<'a, M> {
    match cached {
        CachedHtmlBody::Complex => {
            let display = fallback_text.unwrap_or("(complex HTML email - plain text unavailable)");
            text(display.to_string())
                .size(TEXT_LG)
                .style(text::secondary)
                .into()
        }
        CachedHtmlBody::Empty => {
            let display = fallback_text.unwrap_or("(empty message)");
            text(display.to_string())
                .size(TEXT_LG)
                .style(text::secondary)
                .into()
        }
        CachedHtmlBody::Blocks(blocks) => {
            let on_link: std::rc::Rc<dyn Fn(String) -> M + 'a> = std::rc::Rc::new(on_link_click);
            let mut col = column![].spacing(SPACE_XS).width(Length::Fill);
            for block in blocks {
                col = col.push(render_block_ref(block, std::rc::Rc::clone(&on_link), inline_images));
            }
            col.into()
        }
    }
}

/// Render HTML email body to iced widgets.
///
/// For simple HTML, parses into blocks and emits native iced widgets.
/// For complex HTML, falls back to plain text rendering.
pub fn render_html<'a, M: Clone + 'a>(
    html: &str,
    fallback_text: Option<&str>,
    on_link_click: impl Fn(String) -> M + 'a,
    inline_images: &'a HashMap<String, Vec<u8>>,
) -> Element<'a, M> {
    if assess_complexity(html) == HtmlComplexity::Complex {
        let display = fallback_text.unwrap_or("(complex HTML email - plain text unavailable)");
        return text(display.to_string())
            .size(TEXT_LG)
            .style(text::secondary)
            .into();
    }

    let blocks = parse_html_to_blocks(html);
    if blocks.is_empty() {
        let display = fallback_text.unwrap_or("(empty message)");
        return text(display.to_string())
            .size(TEXT_LG)
            .style(text::secondary)
            .into();
    }

    let on_link: std::rc::Rc<dyn Fn(String) -> M + 'a> = std::rc::Rc::new(on_link_click);
    let mut col = column![].spacing(SPACE_XS).width(Length::Fill);
    for block in blocks {
        col = col.push(render_block(block, std::rc::Rc::clone(&on_link), inline_images));
    }
    col.into()
}

// ── Block model ─────────────────────────────────────────

/// Per-span inline formatting flags. Combinations are derived from any
/// inline tags that are open at the moment a text run is captured
/// (`<b><i>x</i></b>` produces a span with both `bold` and `italic`).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) struct InlineStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    /// Inline `<code>` / `<tt>` / `<kbd>` / `<samp>`. Renders in a monospace
    /// font; we don't currently apply the boxed background that browsers do.
    pub code: bool,
}

impl InlineStyle {
    fn font(self) -> iced::Font {
        if self.code {
            return crate::font::monospace();
        }
        match (self.bold, self.italic) {
            (true, true) => crate::font::text_bold_italic(),
            (true, false) => crate::font::text_bold(),
            (false, true) => crate::font::text_italic(),
            (false, false) => crate::font::text(),
        }
    }
}

/// An inline segment within a paragraph or list item.
#[derive(Clone)]
pub(super) enum InlineSpan {
    /// Styled text.
    Text {
        content: String,
        style: InlineStyle,
    },
    /// A hyperlink with display text, target URL, and any inline styling
    /// that was active inside the `<a>...</a>` (so a bold link still
    /// renders bold).
    Link {
        display: String,
        href: String,
        style: InlineStyle,
    },
}

pub(super) enum Block {
    /// A paragraph containing mixed text and link spans.
    Paragraph(Vec<InlineSpan>),
    Heading(String, u8),
    Preformatted(String),
    Blockquote(String),
    ListItem {
        prefix: String,
        content: Vec<InlineSpan>,
    },
    HorizontalRule,
    /// An inline image referenced by Content-ID (from `<img src="cid:...">`).
    InlineImage {
        cid: String,
        alt: String,
    },
}

/// Render inline spans using `iced::widget::rich_text` so per-span fonts,
/// underline, strikethrough, and link clicks all flow through one shaper
/// pass with proper wrapping.
fn render_spans<'a, M: Clone + 'a>(
    spans: &[InlineSpan],
    on_link_click: &std::rc::Rc<dyn Fn(String) -> M + 'a>,
) -> Element<'a, M> {
    // Fast path: single unstyled text span (still the most common email
    // shape) - skip rich_text entirely.
    if spans.len() == 1
        && let InlineSpan::Text { content, style } = &spans[0]
        && *style == InlineStyle::default()
    {
        return text(content.clone())
            .size(TEXT_LG)
            .style(text::secondary)
            .into();
    }

    let on_click = std::rc::Rc::clone(on_link_click);
    let mut iced_spans: Vec<iced::widget::text::Span<'_, String>> =
        Vec::with_capacity(spans.len());
    for span in spans {
        match span {
            InlineSpan::Text { content, style } => {
                iced_spans.push(make_span(content.clone(), *style, None));
            }
            InlineSpan::Link {
                display,
                href,
                style,
            } => {
                let mut link_style = *style;
                // Underline links by default so they're visually
                // distinguishable even when the accent color is subtle.
                link_style.underline = true;
                iced_spans.push(make_span(display.clone(), link_style, Some(href.clone())));
            }
        }
    }

    iced::widget::rich_text(iced_spans)
        .size(TEXT_LG)
        .on_link_click(move |url: String| (on_click)(url))
        .into()
}

fn make_span<'a>(
    content: String,
    style: InlineStyle,
    link: Option<String>,
) -> iced::widget::text::Span<'a, String> {
    let mut span = iced::widget::span(content)
        .font(style.font())
        .underline(style.underline)
        .strikethrough(style.strikethrough);
    if let Some(href) = link {
        span = span.link(href);
    }
    span
}

/// Render a block by reference (for cached rendering). Clones the owned
/// strings inside the block - this is cheap compared to re-parsing HTML.
#[allow(clippy::needless_pass_by_value)]
fn render_block_ref<'a, M: Clone + 'a>(
    block: &Block,
    on_link_click: std::rc::Rc<dyn Fn(String) -> M + 'a>,
    inline_images: &'a HashMap<String, Vec<u8>>,
) -> Element<'a, M> {
    match block {
        Block::Paragraph(spans) => render_spans(spans, &on_link_click),
        Block::Heading(txt, level) => {
            let size = match level {
                1 => TEXT_HEADING,
                2 => TEXT_TITLE,
                _ => TEXT_XL,
            };
            text(txt.clone())
                .size(size)
                .font(crate::font::text_semibold())
                .style(text::base)
                .into()
        }
        Block::Preformatted(txt) => container(
            text(txt.clone())
                .size(TEXT_MD)
                .font(iced::Font::MONOSPACE)
                .style(text::secondary),
        )
        .padding(PAD_CARD)
        .style(theme::ContainerClass::Elevated.style())
        .width(Length::Fill)
        .into(),
        Block::Blockquote(txt) => container(
            text(txt.clone())
                .size(TEXT_LG)
                .style(theme::TextClass::Tertiary.style()),
        )
        .padding(Padding {
            top: SPACE_XXS,
            right: SPACE_SM,
            bottom: SPACE_XXS,
            left: SPACE_MD,
        })
        .style(theme::ContainerClass::Elevated.style())
        .width(Length::Fill)
        .into(),
        Block::ListItem { prefix, content } => row![
            container(
                text(prefix.clone())
                    .size(TEXT_LG)
                    .style(theme::TextClass::Tertiary.style()),
            )
            .width(Length::Fixed(SPACE_LG)),
        ]
        .push(render_spans(content, &on_link_click))
        .spacing(SPACE_XXS)
        .into(),
        Block::HorizontalRule => iced::widget::rule::horizontal(1).into(),
        Block::InlineImage { cid, alt } => render_cid_image(cid, alt, inline_images),
    }
}

#[allow(clippy::needless_pass_by_value)]
pub(super) fn render_block<'a, M: Clone + 'a>(
    block: Block,
    on_link_click: std::rc::Rc<dyn Fn(String) -> M + 'a>,
    inline_images: &'a HashMap<String, Vec<u8>>,
) -> Element<'a, M> {
    // Delegate to the by-ref version - the block is consumed but
    // render_block_ref only clones the inner strings anyway.
    render_block_ref(&block, on_link_click, inline_images)
}

/// Render a CID-referenced inline image, or fall back to alt text.
fn render_cid_image<'a, M: 'a>(
    cid: &str,
    alt: &str,
    inline_images: &'a HashMap<String, Vec<u8>>,
) -> Element<'a, M> {
    if let Some(data) = inline_images.get(cid) {
        let handle = image::Handle::from_bytes(data.clone());
        image(handle)
            .content_fit(iced::ContentFit::ScaleDown)
            .width(Length::Shrink)
            .into()
    } else if !alt.is_empty() {
        text(format!("[{alt}]"))
            .size(TEXT_LG)
            .style(text::secondary)
            .into()
    } else {
        // No image data and no alt text - render nothing.
        Space::new().width(0).height(0).into()
    }
}

// ── Lightweight HTML parser ─────────────────────────────

/// Parse HTML into a flat list of blocks.
///
/// This is a streaming tag-walking parser that doesn't build a full DOM.
/// It handles common email HTML patterns: paragraphs, headings, lists,
/// blockquotes, pre/code, and basic tables.
fn parse_html_to_blocks(html: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut parser = HtmlParser::new(html);
    parser.parse(&mut blocks);
    blocks
}

struct HtmlParser<'a> {
    source: &'a str,
    pos: usize,
    current_text: String,
    /// Completed inline spans for the current paragraph/list-item.
    current_spans: Vec<InlineSpan>,
    /// When inside `<a href="...">`, holds the href URL.
    current_link_href: Option<String>,
    /// Reference counts for each style flag. Incremented on opening tag,
    /// decremented on close. `current_style()` projects nonzero counts to
    /// `true`, which handles arbitrary nesting (`<b><b>x</b>y</b>`) and
    /// interleaved styles correctly.
    style_counts: StyleCounts,
    in_pre: bool,
    in_blockquote: bool,
    blockquote_text: String,
    list_stack: Vec<ListKind>,
    list_counters: Vec<usize>,
    skip_content: bool,
}

#[derive(Default)]
struct StyleCounts {
    bold: u32,
    italic: u32,
    underline: u32,
    strikethrough: u32,
    code: u32,
}

impl StyleCounts {
    fn current(&self) -> InlineStyle {
        InlineStyle {
            bold: self.bold > 0,
            italic: self.italic > 0,
            underline: self.underline > 0,
            strikethrough: self.strikethrough > 0,
            code: self.code > 0,
        }
    }
}

/// Map an HTML tag name to which style flag it toggles, if any.
fn style_flag_for_tag(tag: &str) -> Option<StyleFlag> {
    match tag {
        "b" | "strong" => Some(StyleFlag::Bold),
        "i" | "em" | "cite" | "var" => Some(StyleFlag::Italic),
        "u" | "ins" => Some(StyleFlag::Underline),
        "s" | "strike" | "del" => Some(StyleFlag::Strikethrough),
        "code" | "tt" | "kbd" | "samp" => Some(StyleFlag::Code),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum StyleFlag {
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Code,
}

#[derive(Clone, Copy)]
enum ListKind {
    Unordered,
    Ordered,
}

impl<'a> HtmlParser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            pos: 0,
            current_text: String::new(),
            current_spans: Vec::new(),
            current_link_href: None,
            style_counts: StyleCounts::default(),
            in_pre: false,
            in_blockquote: false,
            blockquote_text: String::new(),
            list_stack: Vec::new(),
            list_counters: Vec::new(),
            skip_content: false,
        }
    }

    fn bump_style(&mut self, flag: StyleFlag, delta: i32) {
        let counter = match flag {
            StyleFlag::Bold => &mut self.style_counts.bold,
            StyleFlag::Italic => &mut self.style_counts.italic,
            StyleFlag::Underline => &mut self.style_counts.underline,
            StyleFlag::Strikethrough => &mut self.style_counts.strikethrough,
            StyleFlag::Code => &mut self.style_counts.code,
        };
        if delta > 0 {
            *counter += 1;
        } else {
            *counter = counter.saturating_sub(1);
        }
    }

    fn parse(&mut self, blocks: &mut Vec<Block>) {
        while self.pos < self.source.len() {
            if self.source.as_bytes()[self.pos] == b'<' {
                self.handle_tag(blocks);
            } else {
                self.consume_text();
            }
        }
        self.flush_text(blocks);
    }

    fn consume_text(&mut self) {
        if self.skip_content {
            // Advance past non-tag content
            if let Some(idx) = self.source[self.pos..].find('<') {
                self.pos += idx;
            } else {
                self.pos = self.source.len();
            }
            return;
        }

        let start = self.pos;
        while self.pos < self.source.len() && self.source.as_bytes()[self.pos] != b'<' {
            self.pos += 1;
        }
        let chunk = &self.source[start..self.pos];
        let decoded = decode_entities(chunk);

        if self.in_pre {
            self.current_text.push_str(&decoded);
        } else {
            // Collapse whitespace for non-pre content
            let collapsed = collapse_whitespace(&decoded);
            if !collapsed.is_empty() {
                self.current_text.push_str(&collapsed);
            }
        }
    }

    fn handle_tag(&mut self, blocks: &mut Vec<Block>) {
        let _tag_start = self.pos;
        // Find end of tag
        if let Some(end_offset) = self.source[self.pos..].find('>') {
            let tag_end = self.pos + end_offset + 1;
            let tag_content = &self.source[self.pos + 1..self.pos + end_offset];
            self.pos = tag_end;
            self.process_tag(tag_content, blocks);
        } else {
            // Malformed - skip the '<'
            self.pos += 1;
            self.current_text.push('<');
        }
    }

    fn process_tag(&mut self, tag_content: &str, blocks: &mut Vec<Block>) {
        let tag_content = tag_content.trim();
        if tag_content.is_empty() {
            return;
        }

        let is_closing = tag_content.starts_with('/');
        let tag_str = if is_closing {
            &tag_content[1..]
        } else {
            tag_content
        };

        // Extract tag name (before any attributes or self-close slash)
        let tag_name = tag_str
            .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .next()
            .unwrap_or("")
            .to_lowercase();

        if is_closing {
            self.handle_close_tag(&tag_name, blocks);
        } else {
            self.handle_open_tag(&tag_name, tag_str, blocks);
        }
    }

    fn handle_open_tag(&mut self, tag_name: &str, _full_tag: &str, blocks: &mut Vec<Block>) {
        match tag_name {
            "p" | "div" => self.flush_text(blocks),
            "br" => {
                // Insert a newline within the current block rather than
                // flushing to a new paragraph. This preserves mid-paragraph
                // line breaks common in HTML email.
                self.current_text.push('\n');
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.flush_text(blocks);
            }
            "pre" => {
                self.flush_text(blocks);
                self.in_pre = true;
            }
            "blockquote" => {
                self.flush_text(blocks);
                self.in_blockquote = true;
                self.blockquote_text.clear();
            }
            "ul" => {
                self.flush_text(blocks);
                self.list_stack.push(ListKind::Unordered);
            }
            "ol" => {
                self.flush_text(blocks);
                self.list_stack.push(ListKind::Ordered);
                self.list_counters.push(0);
            }
            "li" => {
                self.flush_text(blocks);
            }
            "hr" => {
                self.flush_text(blocks);
                blocks.push(Block::HorizontalRule);
            }
            "style" | "script" | "head" => {
                self.skip_content = true;
            }
            "img" => {
                let alt = extract_attr(_full_tag, "alt").unwrap_or_default();
                let src = extract_attr(_full_tag, "src").unwrap_or_default();

                // Check for CID-referenced inline image.
                if let Some(cid) = src.strip_prefix("cid:") {
                    self.flush_text(blocks);
                    blocks.push(Block::InlineImage {
                        cid: cid.to_string(),
                        alt,
                    });
                } else if !alt.is_empty() {
                    self.current_text.push('[');
                    self.current_text.push_str(&alt);
                    self.current_text.push(']');
                }
            }
            "a" => {
                // Flush accumulated plain text as a Text span before the link.
                self.flush_text_span();
                let href = extract_attr(_full_tag, "href").unwrap_or_default();
                self.current_link_href = Some(href);
            }
            other => {
                if let Some(flag) = style_flag_for_tag(other) {
                    // Flush text accumulated under the OLD style first, then
                    // bump the counter so subsequent text is captured with
                    // the new style on top.
                    self.flush_text_span();
                    self.bump_style(flag, 1);
                }
                // Other inline tags (span, font, ...) are passthrough.
            }
        }
    }

    fn handle_close_tag(&mut self, tag_name: &str, blocks: &mut Vec<Block>) {
        match tag_name {
            "p" | "div" => self.flush_text(blocks),
            "h1" => self.flush_heading(blocks, 1),
            "h2" => self.flush_heading(blocks, 2),
            "h3" => self.flush_heading(blocks, 3),
            "h4" | "h5" | "h6" => self.flush_heading(blocks, 4),
            "pre" => {
                let pre_text = std::mem::take(&mut self.current_text);
                self.in_pre = false;
                if !pre_text.trim().is_empty() {
                    blocks.push(Block::Preformatted(pre_text));
                }
            }
            "blockquote" => {
                self.flush_text(blocks);
                let bq = std::mem::take(&mut self.blockquote_text);
                self.in_blockquote = false;
                if !bq.trim().is_empty() {
                    blocks.push(Block::Blockquote(bq));
                }
            }
            "ul" => {
                self.list_stack.pop();
            }
            "ol" => {
                self.list_stack.pop();
                self.list_counters.pop();
            }
            "li" => {
                let spans = self.take_spans();
                if !spans.is_empty() {
                    let prefix = match self.list_stack.last() {
                        Some(ListKind::Unordered) => "\u{2022}".to_string(),
                        Some(ListKind::Ordered) => {
                            if let Some(counter) = self.list_counters.last_mut() {
                                *counter += 1;
                                format!("{counter}.")
                            } else {
                                "\u{2022}".to_string()
                            }
                        }
                        None => "\u{2022}".to_string(),
                    };
                    blocks.push(Block::ListItem {
                        prefix,
                        content: spans,
                    });
                }
            }
            "a" => {
                // Close the link span: take the accumulated text as the link display text.
                let link_text = std::mem::take(&mut self.current_text);
                let trimmed = link_text.trim().to_string();
                let style = self.style_counts.current();
                if let Some(href) = self.current_link_href.take() {
                    if !trimmed.is_empty() {
                        self.current_spans.push(InlineSpan::Link {
                            display: trimmed,
                            href,
                            style,
                        });
                    }
                } else if !trimmed.is_empty() {
                    // Orphaned </a> - treat text as plain.
                    self.current_spans.push(InlineSpan::Text {
                        content: trimmed,
                        style,
                    });
                }
            }
            "style" | "script" | "head" => {
                self.skip_content = false;
            }
            other => {
                if let Some(flag) = style_flag_for_tag(other) {
                    // Flush the styled text under this tag, then drop the
                    // counter so subsequent text loses the style.
                    self.flush_text_span();
                    self.bump_style(flag, -1);
                }
            }
        }
    }

    /// Flush accumulated plain text into `current_spans`, tagged with the
    /// currently-open inline style. Called whenever style or link state
    /// changes mid-paragraph, so we deliberately preserve internal spaces -
    /// trimming at edges happens once at block boundaries (`take_spans`).
    fn flush_text_span(&mut self) {
        let text = std::mem::take(&mut self.current_text);
        if !text.is_empty() {
            self.current_spans.push(InlineSpan::Text {
                content: text,
                style: self.style_counts.current(),
            });
        }
    }

    /// Collect all pending spans (and any trailing text) into a completed
    /// span list, draining the parser's inline state. Trims leading
    /// whitespace from the first span and trailing whitespace from the
    /// last so block boundaries don't accumulate stray spaces, while
    /// preserving the internal spacing between styled runs.
    fn take_spans(&mut self) -> Vec<InlineSpan> {
        self.flush_text_span();
        if let Some(_href) = self.current_link_href.take() {
            // Link was never closed; its text already flushed as a Text span.
        }
        let mut spans = std::mem::take(&mut self.current_spans);
        trim_span_edges(&mut spans);
        spans
    }

    fn flush_text(&mut self, blocks: &mut Vec<Block>) {
        let spans = self.take_spans();
        if spans.is_empty() {
            return;
        }

        if self.in_blockquote {
            // Blockquotes flatten to plain text for now.
            let plain = spans_to_plain_text(&spans);
            if !self.blockquote_text.is_empty() {
                self.blockquote_text.push(' ');
            }
            self.blockquote_text.push_str(&plain);
        } else {
            blocks.push(Block::Paragraph(spans));
        }
    }

    fn flush_heading(&mut self, blocks: &mut Vec<Block>, level: u8) {
        let text = std::mem::take(&mut self.current_text);
        let trimmed = text.trim().to_string();
        if !trimmed.is_empty() {
            blocks.push(Block::Heading(trimmed, level));
        }
    }
}

// ── Helpers ─────────────────────────────────────────────

/// Collapse runs of whitespace to single spaces.
fn collapse_whitespace(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut prev_ws = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_ws {
                result.push(' ');
                prev_ws = true;
            }
        } else {
            result.push(ch);
            prev_ws = false;
        }
    }
    result
}

/// Decode HTML entities: named entities, decimal (`&#123;`), and hex (`&#x7B;`).
fn decode_entities(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '&' {
            result.push(ch);
            continue;
        }
        // Collect entity up to `;` (max 10 chars to avoid runaway)
        let mut entity = String::new();
        let mut found_semi = false;
        for _ in 0..10 {
            match chars.peek() {
                Some(&';') => {
                    chars.next();
                    found_semi = true;
                    break;
                }
                Some(_) => entity.push(chars.next().expect("peeked")),
                None => break,
            }
        }
        if !found_semi {
            // Not a valid entity - emit as literal
            result.push('&');
            result.push_str(&entity);
            continue;
        }
        // Decode
        if let Some(decoded) = decode_named_entity(&entity) {
            result.push_str(decoded);
        } else if let Some(stripped) = entity.strip_prefix('#') {
            let codepoint = if let Some(hex) = stripped
                .strip_prefix('x')
                .or_else(|| stripped.strip_prefix('X'))
            {
                u32::from_str_radix(hex, 16).ok()
            } else {
                stripped.parse::<u32>().ok()
            };
            match codepoint.and_then(char::from_u32) {
                Some(c) => result.push(c),
                None => {
                    result.push('&');
                    result.push_str(&entity);
                    result.push(';');
                }
            }
        } else {
            result.push('&');
            result.push_str(&entity);
            result.push(';');
        }
    }
    result
}

fn decode_named_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "nbsp" => "\u{00A0}",
        "quot" => "\"",
        "apos" => "'",
        "mdash" => "\u{2014}",
        "ndash" => "\u{2013}",
        "hellip" => "\u{2026}",
        "copy" => "\u{00A9}",
        "reg" => "\u{00AE}",
        "trade" => "\u{2122}",
        "laquo" => "\u{00AB}",
        "raquo" => "\u{00BB}",
        "lsquo" => "\u{2018}",
        "rsquo" => "\u{2019}",
        "ldquo" => "\u{201C}",
        "rdquo" => "\u{201D}",
        "bull" => "\u{2022}",
        "middot" => "\u{00B7}",
        "deg" => "\u{00B0}",
        "times" => "\u{00D7}",
        "divide" => "\u{00F7}",
        "euro" => "\u{20AC}",
        "pound" => "\u{00A3}",
        "yen" => "\u{00A5}",
        "cent" => "\u{00A2}",
        "sect" => "\u{00A7}",
        "para" => "\u{00B6}",
        "dagger" => "\u{2020}",
        "Dagger" => "\u{2021}",
        "ensp" => "\u{2002}",
        "emsp" => "\u{2003}",
        "thinsp" => "\u{2009}",
        "zwnj" => "\u{200C}",
        "zwj" => "\u{200D}",
        _ => return None,
    })
}

/// Flatten a span list to plain text (for blockquotes and other contexts
/// that don't support inline widgets).
fn spans_to_plain_text(spans: &[InlineSpan]) -> String {
    let mut out = String::new();
    for span in spans {
        match span {
            InlineSpan::Text { content, .. } => out.push_str(content),
            InlineSpan::Link { display, .. } => out.push_str(display),
        }
    }
    out
}

/// Trim leading whitespace from the first text span and trailing
/// whitespace from the last; drop spans that go empty as a result.
/// Spans between the edges keep their internal whitespace intact so
/// `Hello <b>world</b>` doesn't lose the space between the runs.
fn trim_span_edges(spans: &mut Vec<InlineSpan>) {
    if let Some(first) = spans.first_mut()
        && let InlineSpan::Text { content, .. } = first
    {
        *content = content.trim_start().to_string();
    }
    if let Some(last) = spans.last_mut()
        && let InlineSpan::Text { content, .. } = last
    {
        *content = content.trim_end().to_string();
    }
    spans.retain(|span| match span {
        InlineSpan::Text { content, .. } => !content.is_empty(),
        InlineSpan::Link { display, .. } => !display.is_empty(),
    });
}

/// Extract an attribute value from a raw tag string.
fn extract_attr(tag: &str, attr_name: &str) -> Option<String> {
    let lower = tag.to_lowercase();
    let pattern = format!("{attr_name}=\"");
    if let Some(start) = lower.find(&pattern) {
        let value_start = start + pattern.len();
        let rest = &tag[value_start..];
        if let Some(end) = rest.find('"') {
            return Some(rest[..end].to_string());
        }
    }
    // Try single quotes
    let pattern_sq = format!("{attr_name}='");
    if let Some(start) = lower.find(&pattern_sq) {
        let value_start = start + pattern_sq.len();
        let rest = &tag[value_start..];
        if let Some(end) = rest.find('\'') {
            return Some(rest[..end].to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: extract the plain text from a span list (for test assertions).
    fn spans_text(spans: &[InlineSpan]) -> String {
        spans_to_plain_text(spans)
    }

    #[test]
    fn simple_paragraph() {
        let blocks = parse_html_to_blocks("<p>Hello world</p>");
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Paragraph(spans) => assert_eq!(spans_text(spans), "Hello world"),
            _ => panic!("expected paragraph"),
        }
    }

    #[test]
    fn heading_levels() {
        let blocks = parse_html_to_blocks("<h1>Title</h1><h2>Subtitle</h2>");
        assert_eq!(blocks.len(), 2);
        match &blocks[0] {
            Block::Heading(s, 1) => assert_eq!(s, "Title"),
            _ => panic!("expected h1"),
        }
    }

    #[test]
    fn unordered_list() {
        let blocks = parse_html_to_blocks("<ul><li>One</li><li>Two</li></ul>");
        assert_eq!(blocks.len(), 2);
        match &blocks[0] {
            Block::ListItem { prefix, content } => {
                assert_eq!(prefix, "\u{2022}");
                assert_eq!(spans_text(content), "One");
            }
            _ => panic!("expected list item"),
        }
    }

    #[test]
    fn ordered_list() {
        let blocks = parse_html_to_blocks("<ol><li>First</li><li>Second</li></ol>");
        assert_eq!(blocks.len(), 2);
        match &blocks[0] {
            Block::ListItem { prefix, content } => {
                assert_eq!(prefix, "1.");
                assert_eq!(spans_text(content), "First");
            }
            _ => panic!("expected ordered list item"),
        }
    }

    #[test]
    fn preformatted() {
        let blocks = parse_html_to_blocks("<pre>  code here\n  indented</pre>");
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Preformatted(s) => assert!(s.contains("code here")),
            _ => panic!("expected pre block"),
        }
    }

    #[test]
    fn entity_decoding() {
        let blocks = parse_html_to_blocks("<p>&amp; &lt; &gt; &quot;</p>");
        match &blocks[0] {
            Block::Paragraph(spans) => assert_eq!(spans_text(spans), "& < > \""),
            _ => panic!("expected paragraph"),
        }
    }

    #[test]
    fn style_stripped() {
        let blocks = parse_html_to_blocks("<style>.foo{color:red}</style><p>visible</p>");
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Paragraph(spans) => assert_eq!(spans_text(spans), "visible"),
            _ => panic!("expected paragraph"),
        }
    }

    #[test]
    fn link_extraction() {
        let blocks =
            parse_html_to_blocks("<p>Click <a href=\"https://example.com\">here</a> for more.</p>");
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Paragraph(spans) => {
                assert_eq!(spans.len(), 3);
                match &spans[0] {
                    InlineSpan::Text { content, .. } => assert_eq!(content.trim(), "Click"),
                    _ => panic!("expected text span"),
                }
                match &spans[1] {
                    InlineSpan::Link { display, href, .. } => {
                        assert_eq!(display, "here");
                        assert_eq!(href, "https://example.com");
                    }
                    _ => panic!("expected link span"),
                }
                match &spans[2] {
                    InlineSpan::Text { content, .. } => assert_eq!(content.trim(), "for more."),
                    _ => panic!("expected text span"),
                }
            }
            _ => panic!("expected paragraph"),
        }
    }

    #[test]
    fn link_only_paragraph() {
        let blocks = parse_html_to_blocks("<p><a href=\"https://example.com\">Example</a></p>");
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Paragraph(spans) => {
                assert_eq!(spans.len(), 1);
                match &spans[0] {
                    InlineSpan::Link { display, href, .. } => {
                        assert_eq!(display, "Example");
                        assert_eq!(href, "https://example.com");
                    }
                    _ => panic!("expected link span"),
                }
            }
            _ => panic!("expected paragraph"),
        }
    }

    #[test]
    fn bold_and_italic_inline_styles() {
        let blocks =
            parse_html_to_blocks("<p>plain <b>bold</b> <i>italic</i> <b><i>both</i></b></p>");
        let Block::Paragraph(spans) = &blocks[0] else {
            panic!("expected paragraph");
        };
        // Pull out (content, bold, italic) for each text span.
        let summary: Vec<(&str, bool, bool)> = spans
            .iter()
            .filter_map(|s| match s {
                InlineSpan::Text { content, style } => {
                    Some((content.as_str(), style.bold, style.italic))
                }
                InlineSpan::Link { .. } => None,
            })
            .collect();
        // The exact whitespace runs are sensitive to the parser's whitespace
        // collapsing; assert the styled fragments appear with the right flags.
        assert!(summary.iter().any(|(t, b, i)| t.contains("bold") && *b && !*i));
        assert!(summary.iter().any(|(t, b, i)| t.contains("italic") && !*b && *i));
        assert!(summary.iter().any(|(t, b, i)| t.contains("both") && *b && *i));
    }

    #[test]
    fn underline_and_strikethrough_styles() {
        let blocks = parse_html_to_blocks("<p><u>up</u> <s>out</s></p>");
        let Block::Paragraph(spans) = &blocks[0] else {
            panic!("expected paragraph");
        };
        let mut underlined = false;
        let mut struck = false;
        for span in spans {
            if let InlineSpan::Text { content, style } = span {
                if content.contains("up") && style.underline {
                    underlined = true;
                }
                if content.contains("out") && style.strikethrough {
                    struck = true;
                }
            }
        }
        assert!(underlined, "expected an underlined span");
        assert!(struck, "expected a strikethrough span");
    }

    #[test]
    fn nested_styles_pop_correctly() {
        let blocks = parse_html_to_blocks("<p><b>x<b>y</b>z</b>t</p>");
        let Block::Paragraph(spans) = &blocks[0] else {
            panic!("expected paragraph");
        };
        // 't' must be unstyled - both <b>s are closed by then.
        let last_text = spans
            .iter()
            .rev()
            .find_map(|s| match s {
                InlineSpan::Text { content, style } => Some((content, style)),
                InlineSpan::Link { .. } => None,
            })
            .expect("a final text span");
        assert_eq!(last_text.0.trim(), "t");
        assert!(!last_text.1.bold, "trailing 't' should not be bold");
    }

    #[test]
    fn list_item_with_link() {
        let blocks =
            parse_html_to_blocks("<ul><li>See <a href=\"https://example.com\">this</a></li></ul>");
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::ListItem { content, .. } => {
                assert_eq!(content.len(), 2);
                match &content[1] {
                    InlineSpan::Link { display, href, .. } => {
                        assert_eq!(display, "this");
                        assert_eq!(href, "https://example.com");
                    }
                    _ => panic!("expected link in list item"),
                }
            }
            _ => panic!("expected list item"),
        }
    }

    #[test]
    fn complex_detection() {
        let simple = "<p>Hello</p><table><tr><td>A</td></tr></table>";
        assert_eq!(assess_complexity(simple), HtmlComplexity::Simple);

        // Deeply nested tables (marketing email layout)
        let complex = "<table><tr><td><table><tr><td><table><tr><td>\
                        <table><tr><td><table><tr><td><table><tr><td>\
                        nested</td></tr></table></td></tr></table>\
                        </td></tr></table></td></tr></table>\
                        </td></tr></table></td></tr></table>";
        assert_eq!(assess_complexity(complex), HtmlComplexity::Complex);
    }
}
