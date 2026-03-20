//! Document → HTML serialization.
//!
//! Recursive walk of the document tree. Consistent nesting order for inline
//! styles: `<a><strong><em><u><s>text</s></u></em></strong></a>`

use crate::document::{Block, Document, InlineStyle, ListItem, StyledRun};
use std::sync::Arc;

/// Serialize a document to HTML.
pub fn to_html(doc: &Document) -> String {
    let mut buf = String::new();
    for block in &doc.blocks {
        serialize_block(block, &mut buf);
    }
    buf
}

fn serialize_block(block: &Block, buf: &mut String) {
    match block {
        Block::Paragraph { runs } => {
            buf.push_str("<p>");
            serialize_runs(runs, buf);
            buf.push_str("</p>");
        }
        Block::Heading { level, runs } => {
            let n = level.as_u8();
            buf.push_str("<h");
            buf.push(char::from(b'0' + n));
            buf.push('>');
            serialize_runs(runs, buf);
            buf.push_str("</h");
            buf.push(char::from(b'0' + n));
            buf.push('>');
        }
        Block::List { ordered, items } => {
            let tag = if *ordered { "ol" } else { "ul" };
            buf.push('<');
            buf.push_str(tag);
            buf.push('>');
            for item in items {
                serialize_list_item(item, buf);
            }
            buf.push_str("</");
            buf.push_str(tag);
            buf.push('>');
        }
        Block::BlockQuote { blocks } => {
            buf.push_str("<blockquote>");
            serialize_child_blocks(blocks, buf);
            buf.push_str("</blockquote>");
        }
        Block::HorizontalRule => {
            buf.push_str("<hr>");
        }
    }
}

fn serialize_list_item(item: &ListItem, buf: &mut String) {
    buf.push_str("<li>");
    serialize_child_blocks(&item.blocks, buf);
    buf.push_str("</li>");
}

fn serialize_child_blocks(blocks: &[Arc<Block>], buf: &mut String) {
    for block in blocks {
        serialize_block(block, buf);
    }
}

fn serialize_runs(runs: &[StyledRun], buf: &mut String) {
    for run in runs {
        if run.is_empty() {
            continue;
        }
        serialize_run(run, buf);
    }
}

fn serialize_run(run: &StyledRun, buf: &mut String) {
    let has_link = run.link.is_some();
    let has_bold = run.style.contains(InlineStyle::BOLD);
    let has_italic = run.style.contains(InlineStyle::ITALIC);
    let has_underline = run.style.contains(InlineStyle::UNDERLINE);
    let has_strikethrough = run.style.contains(InlineStyle::STRIKETHROUGH);

    // Open tags: link → bold → italic → underline → strikethrough
    if let Some(href) = &run.link {
        buf.push_str("<a href=\"");
        html_escape_into(href, buf);
        buf.push_str("\">");
    }
    if has_bold {
        buf.push_str("<strong>");
    }
    if has_italic {
        buf.push_str("<em>");
    }
    if has_underline {
        buf.push_str("<u>");
    }
    if has_strikethrough {
        buf.push_str("<s>");
    }

    // Text content (escaped)
    html_escape_into(&run.text, buf);

    // Close tags: strikethrough → underline → italic → bold → link
    if has_strikethrough {
        buf.push_str("</s>");
    }
    if has_underline {
        buf.push_str("</u>");
    }
    if has_italic {
        buf.push_str("</em>");
    }
    if has_bold {
        buf.push_str("</strong>");
    }
    if has_link {
        buf.push_str("</a>");
    }
}

/// HTML-escape `&`, `<`, `>`, and `"` into the buffer.
fn html_escape_into(text: &str, buf: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => buf.push_str("&amp;"),
            '<' => buf.push_str("&lt;"),
            '>' => buf.push_str("&gt;"),
            '"' => buf.push_str("&quot;"),
            _ => buf.push(ch),
        }
    }
}

// ── Tests ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{HeadingLevel, InlineStyle};
    use std::sync::Arc;

    #[test]
    fn empty_document() {
        let doc = Document::new();
        // Document::new() creates a single empty paragraph with one empty run.
        // Empty runs are skipped, so we get <p></p>.
        assert_eq!(to_html(&doc), "<p></p>");
    }

    #[test]
    fn plain_paragraph() {
        let doc = Document::from_blocks(vec![Block::paragraph("Hello, world!")]);
        assert_eq!(to_html(&doc), "<p>Hello, world!</p>");
    }

    #[test]
    fn bold_italic_run() {
        let doc = Document::from_blocks(vec![Block::Paragraph {
            runs: vec![StyledRun::styled(
                "text",
                InlineStyle::BOLD | InlineStyle::ITALIC,
            )],
        }]);
        assert_eq!(to_html(&doc), "<p><strong><em>text</em></strong></p>");
    }

    #[test]
    fn all_inline_styles() {
        let doc = Document::from_blocks(vec![Block::Paragraph {
            runs: vec![StyledRun {
                text: "styled".into(),
                style: InlineStyle::BOLD
                    | InlineStyle::ITALIC
                    | InlineStyle::UNDERLINE
                    | InlineStyle::STRIKETHROUGH,
                link: Some("https://example.com".into()),
            }],
        }]);
        assert_eq!(
            to_html(&doc),
            "<p><a href=\"https://example.com\"><strong><em><u><s>styled</s></u></em></strong></a></p>"
        );
    }

    #[test]
    fn multiple_runs_different_styles() {
        let doc = Document::from_blocks(vec![Block::Paragraph {
            runs: vec![
                StyledRun::plain("normal "),
                StyledRun::styled("bold", InlineStyle::BOLD),
                StyledRun::plain(" and "),
                StyledRun::styled("italic", InlineStyle::ITALIC),
            ],
        }]);
        assert_eq!(
            to_html(&doc),
            "<p>normal <strong>bold</strong> and <em>italic</em></p>"
        );
    }

    #[test]
    fn heading_levels() {
        let doc = Document::from_blocks(vec![
            Block::Heading {
                level: HeadingLevel::H1,
                runs: vec![StyledRun::plain("Title")],
            },
            Block::Heading {
                level: HeadingLevel::H2,
                runs: vec![StyledRun::plain("Subtitle")],
            },
            Block::Heading {
                level: HeadingLevel::H3,
                runs: vec![StyledRun::plain("Section")],
            },
        ]);
        assert_eq!(
            to_html(&doc),
            "<h1>Title</h1><h2>Subtitle</h2><h3>Section</h3>"
        );
    }

    #[test]
    fn unordered_list() {
        let doc = Document::from_blocks(vec![Block::List {
            ordered: false,
            items: vec![
                ListItem::plain("Alpha"),
                ListItem::plain("Beta"),
                ListItem::plain("Gamma"),
            ],
        }]);
        assert_eq!(
            to_html(&doc),
            "<ul><li><p>Alpha</p></li><li><p>Beta</p></li><li><p>Gamma</p></li></ul>"
        );
    }

    #[test]
    fn ordered_list() {
        let doc = Document::from_blocks(vec![Block::List {
            ordered: true,
            items: vec![ListItem::plain("First"), ListItem::plain("Second")],
        }]);
        assert_eq!(
            to_html(&doc),
            "<ol><li><p>First</p></li><li><p>Second</p></li></ol>"
        );
    }

    #[test]
    fn nested_list() {
        let inner_list = Block::List {
            ordered: false,
            items: vec![ListItem::plain("nested-a"), ListItem::plain("nested-b")],
        };
        let doc = Document::from_blocks(vec![Block::List {
            ordered: true,
            items: vec![
                ListItem {
                    blocks: vec![
                        Arc::new(Block::paragraph("outer item")),
                        Arc::new(inner_list),
                    ],
                },
                ListItem::plain("second outer"),
            ],
        }]);
        assert_eq!(
            to_html(&doc),
            "<ol><li><p>outer item</p><ul><li><p>nested-a</p></li><li><p>nested-b</p></li></ul></li><li><p>second outer</p></li></ol>"
        );
    }

    #[test]
    fn blockquote_with_paragraphs() {
        let doc = Document::from_blocks(vec![Block::BlockQuote {
            blocks: vec![
                Arc::new(Block::paragraph("Line one")),
                Arc::new(Block::paragraph("Line two")),
            ],
        }]);
        assert_eq!(
            to_html(&doc),
            "<blockquote><p>Line one</p><p>Line two</p></blockquote>"
        );
    }

    #[test]
    fn horizontal_rule() {
        let doc = Document::from_blocks(vec![
            Block::paragraph("Above"),
            Block::HorizontalRule,
            Block::paragraph("Below"),
        ]);
        assert_eq!(to_html(&doc), "<p>Above</p><hr><p>Below</p>");
    }

    #[test]
    fn link_with_styles() {
        let doc = Document::from_blocks(vec![Block::Paragraph {
            runs: vec![StyledRun::linked("click here", InlineStyle::BOLD, "https://example.com")],
        }]);
        assert_eq!(
            to_html(&doc),
            "<p><a href=\"https://example.com\"><strong>click here</strong></a></p>"
        );
    }

    #[test]
    fn html_escaping_in_text() {
        let doc = Document::from_blocks(vec![Block::paragraph("x < y & y > z \"quoted\"")]);
        assert_eq!(
            to_html(&doc),
            "<p>x &lt; y &amp; y &gt; z &quot;quoted&quot;</p>"
        );
    }

    #[test]
    fn html_escaping_in_link_href() {
        let doc = Document::from_blocks(vec![Block::Paragraph {
            runs: vec![StyledRun::linked(
                "link",
                InlineStyle::empty(),
                "https://example.com/search?q=a&b=c\"d",
            )],
        }]);
        assert_eq!(
            to_html(&doc),
            "<p><a href=\"https://example.com/search?q=a&amp;b=c&quot;d\">link</a></p>"
        );
    }

    #[test]
    fn empty_runs_skipped() {
        let doc = Document::from_blocks(vec![Block::Paragraph {
            runs: vec![
                StyledRun::plain(""),
                StyledRun::plain("visible"),
                StyledRun::styled("", InlineStyle::BOLD),
            ],
        }]);
        assert_eq!(to_html(&doc), "<p>visible</p>");
    }

    #[test]
    fn mixed_document() {
        let doc = Document::from_blocks(vec![
            Block::Heading {
                level: HeadingLevel::H1,
                runs: vec![StyledRun::styled("Welcome", InlineStyle::BOLD)],
            },
            Block::paragraph("Some intro text."),
            Block::List {
                ordered: false,
                items: vec![ListItem::plain("Point A"), ListItem::plain("Point B")],
            },
            Block::HorizontalRule,
            Block::BlockQuote {
                blocks: vec![Arc::new(Block::paragraph("A wise quote."))],
            },
        ]);
        assert_eq!(
            to_html(&doc),
            "<h1><strong>Welcome</strong></h1>\
             <p>Some intro text.</p>\
             <ul><li><p>Point A</p></li><li><p>Point B</p></li></ul>\
             <hr>\
             <blockquote><p>A wise quote.</p></blockquote>"
        );
    }

    #[test]
    fn underline_and_strikethrough() {
        let doc = Document::from_blocks(vec![Block::Paragraph {
            runs: vec![StyledRun::styled(
                "deleted",
                InlineStyle::UNDERLINE | InlineStyle::STRIKETHROUGH,
            )],
        }]);
        assert_eq!(to_html(&doc), "<p><u><s>deleted</s></u></p>");
    }

    #[test]
    fn blockquote_nested_in_blockquote() {
        let doc = Document::from_blocks(vec![Block::BlockQuote {
            blocks: vec![
                Arc::new(Block::paragraph("Outer")),
                Arc::new(Block::BlockQuote {
                    blocks: vec![Arc::new(Block::paragraph("Inner"))],
                }),
            ],
        }]);
        assert_eq!(
            to_html(&doc),
            "<blockquote><p>Outer</p><blockquote><p>Inner</p></blockquote></blockquote>"
        );
    }

    #[test]
    fn unicode_content() {
        let doc = Document::from_blocks(vec![Block::paragraph("Héllo wörld 🌍")]);
        assert_eq!(to_html(&doc), "<p>Héllo wörld 🌍</p>");
    }
}
