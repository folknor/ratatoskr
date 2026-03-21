//! DOM-to-widget HTML email rendering pipeline.
//!
//! Parses HTML into a lightweight block structure, then emits iced widgets.
//! Handles: paragraphs, links, lists, blockquotes, pre/code blocks, images
//! (alt text), headings, horizontal rules, and basic tables.
//!
//! Includes a complexity heuristic for fallback detection — emails with deep
//! table nesting (layout tables) or heavy style blocks fall back to plain text.

use iced::widget::{column, container, row, text, Space};
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

/// Render HTML email body to iced widgets.
///
/// For simple HTML, parses into blocks and emits native iced widgets.
/// For complex HTML, falls back to plain text rendering.
pub fn render_html<'a, M: Clone + 'a>(
    html: &str,
    fallback_text: Option<&str>,
) -> Element<'a, M> {
    if assess_complexity(html) == HtmlComplexity::Complex {
        let display = fallback_text
            .unwrap_or("(complex HTML email — plain text unavailable)");
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

    let mut col = column![].spacing(SPACE_XS).width(Length::Fill);
    for block in blocks {
        col = col.push(render_block::<M>(block));
    }
    col.into()
}

// ── Block model ─────────────────────────────────────────

enum Block {
    Paragraph(String),
    Heading(String, u8),
    Preformatted(String),
    Blockquote(String),
    ListItem { prefix: String, content: String },
    HorizontalRule,
}

fn render_block<'a, M: Clone + 'a>(block: Block) -> Element<'a, M> {
    match block {
        Block::Paragraph(txt) => {
            text(txt)
                .size(TEXT_LG)
                .style(text::secondary)
                .into()
        }
        Block::Heading(txt, level) => {
            let size = match level {
                1 => TEXT_HEADING,
                2 => TEXT_TITLE,
                _ => TEXT_XL,
            };
            text(txt)
                .size(size)
                .font(crate::font::text_semibold())
                .style(text::base)
                .into()
        }
        Block::Preformatted(txt) => {
            container(
                text(txt)
                    .size(TEXT_MD)
                    .font(iced::Font::MONOSPACE)
                    .style(text::secondary),
            )
            .padding(PAD_CARD)
            .style(theme::ContainerClass::Elevated.style())
            .width(Length::Fill)
            .into()
        }
        Block::Blockquote(txt) => {
            container(
                text(txt)
                    .size(TEXT_LG)
                    .style(theme::TextClass::Tertiary.style()),
            )
            .padding(Padding { top: SPACE_XXS, right: SPACE_SM, bottom: SPACE_XXS, left: SPACE_MD })
            .style(theme::ContainerClass::Elevated.style())
            .width(Length::Fill)
            .into()
        }
        Block::ListItem { prefix, content } => {
            row![
                container(
                    text(prefix)
                        .size(TEXT_LG)
                        .style(theme::TextClass::Tertiary.style()),
                )
                .width(Length::Fixed(SPACE_LG)),
                text(content).size(TEXT_LG).style(text::secondary),
            ]
            .spacing(SPACE_XXS)
            .into()
        }
        Block::HorizontalRule => {
            iced::widget::rule::horizontal(1).into()
        }
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
    in_pre: bool,
    in_blockquote: bool,
    blockquote_text: String,
    list_stack: Vec<ListKind>,
    list_counters: Vec<usize>,
    skip_content: bool,
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
            in_pre: false,
            in_blockquote: false,
            blockquote_text: String::new(),
            list_stack: Vec::new(),
            list_counters: Vec::new(),
            skip_content: false,
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
        let tag_start = self.pos;
        // Find end of tag
        if let Some(end_offset) = self.source[self.pos..].find('>') {
            let tag_end = self.pos + end_offset + 1;
            let tag_content = &self.source[self.pos + 1..self.pos + end_offset];
            self.pos = tag_end;
            self.process_tag(tag_content, blocks);
        } else {
            // Malformed — skip the '<'
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
                if self.in_pre {
                    self.current_text.push('\n');
                } else {
                    self.flush_text(blocks);
                }
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
                // Extract alt text
                let alt = extract_attr(_full_tag, "alt");
                if let Some(alt_text) = alt {
                    if !alt_text.is_empty() {
                        self.current_text.push('[');
                        self.current_text.push_str(&alt_text);
                        self.current_text.push(']');
                    }
                }
            }
            _ => {} // span, strong, b, em, i, a, etc. — inline, just consume text
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
                let content = std::mem::take(&mut self.current_text);
                let trimmed = content.trim().to_string();
                if !trimmed.is_empty() {
                    let prefix = match self.list_stack.last() {
                        Some(ListKind::Unordered) => "\u{2022}".to_string(),
                        Some(ListKind::Ordered) => {
                            if let Some(counter) = self.list_counters.last_mut() {
                                *counter += 1;
                                format!("{}.", counter)
                            } else {
                                "\u{2022}".to_string()
                            }
                        }
                        None => "\u{2022}".to_string(),
                    };
                    blocks.push(Block::ListItem {
                        prefix,
                        content: trimmed,
                    });
                }
            }
            "style" | "script" | "head" => {
                self.skip_content = false;
            }
            _ => {}
        }
    }

    fn flush_text(&mut self, blocks: &mut Vec<Block>) {
        let text = std::mem::take(&mut self.current_text);
        let trimmed = text.trim().to_string();
        if trimmed.is_empty() {
            return;
        }

        if self.in_blockquote {
            if !self.blockquote_text.is_empty() {
                self.blockquote_text.push(' ');
            }
            self.blockquote_text.push_str(&trimmed);
        } else {
            blocks.push(Block::Paragraph(trimmed));
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

/// Decode common HTML entities.
fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
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

    #[test]
    fn simple_paragraph() {
        let blocks = parse_html_to_blocks("<p>Hello world</p>");
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Paragraph(s) => assert_eq!(s, "Hello world"),
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
                assert_eq!(content, "One");
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
                assert_eq!(content, "First");
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
            Block::Paragraph(s) => assert_eq!(s, "& < > \""),
            _ => panic!("expected paragraph"),
        }
    }

    #[test]
    fn style_stripped() {
        let blocks = parse_html_to_blocks(
            "<style>.foo{color:red}</style><p>visible</p>"
        );
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Paragraph(s) => assert_eq!(s, "visible"),
            _ => panic!("expected paragraph"),
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
