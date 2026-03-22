use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;

/// Regex matching `@import` rules in inline style values.
static IMPORT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)@import\s+[^;]+;?"#).expect("IMPORT_RE"));

/// Regex matching `javascript:` URLs (with optional whitespace/encoding tricks).
static JS_URL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)^\s*javascript\s*:"#).expect("JS_URL_RE"));

/// Regex matching event handler attribute names (`on*`).
static EVENT_HANDLER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^on[a-z]").expect("EVENT_HANDLER_RE"));

// ---------------------------------------------------------------------------
// Stage 1: CSS inlining
// ---------------------------------------------------------------------------

/// Inline `<style>` blocks into element `style=` attributes so that styles
/// survive later sanitisation passes that strip `<style>` tags.
fn inline_css(html: &str) -> String {
    let inliner = css_inline::CSSInliner::options().build();

    match inliner.inline(html) {
        Ok(result) => result,
        Err(e) => {
            log::warn!("css-inline failed, continuing with original HTML: {e}");
            html.to_string()
        }
    }
}

// ---------------------------------------------------------------------------
// Stage 2: Streaming element removal via lol_html
// ---------------------------------------------------------------------------

/// Tags whose opening *and* content should be removed entirely.
const REMOVE_TAGS_WITH_CONTENT: &[&str] = &[
    "script", "style", "iframe", "object", "embed", "applet", "form",
];

/// Tags that should be removed (opening + closing) but whose content is kept.
const REMOVE_TAGS_KEEP_CONTENT: &[&str] = &[
    "link", "input", "button", "select", "textarea", "meta",
];

fn strip_dangerous_elements(html: &str) -> String {
    use lol_html::{element, rewrite_str, RewriteStrSettings};

    let mut element_handlers = Vec::new();

    // Tags that should be fully removed (including their children).
    for &tag in REMOVE_TAGS_WITH_CONTENT {
        element_handlers.push(element!(tag, |el| {
            el.remove();
            Ok(())
        }));
    }

    // Tags removed but content kept.
    for &tag in REMOVE_TAGS_KEEP_CONTENT {
        element_handlers.push(element!(tag, |el| {
            el.remove_and_keep_content();
            Ok(())
        }));
    }

    // Wildcard handler: strip event-handler attributes, javascript: URLs,
    // and @import in inline styles on every element.
    element_handlers.push(element!("*", |el| {
        // Collect attribute names first to avoid borrowing issues.
        let attr_names: Vec<String> =
            el.attributes().iter().map(lol_html::html_content::Attribute::name).collect();

        for name in &attr_names {
            // Remove event handler attributes (onclick, onload, etc.)
            if EVENT_HANDLER_RE.is_match(name) {
                el.remove_attribute(name);
                continue;
            }

            // Strip javascript: URLs in href and src.
            if (name.eq_ignore_ascii_case("href") || name.eq_ignore_ascii_case("src"))
                && el.get_attribute(name).is_some_and(|val| JS_URL_RE.is_match(&val))
            {
                el.remove_attribute(name);
            }

            // Strip @import from inline styles.
            if name.eq_ignore_ascii_case("style")
                && let Some(val) = el.get_attribute(name)
                && val.to_ascii_lowercase().contains("@import")
            {
                let cleaned = IMPORT_RE.replace_all(&val, "").to_string();
                el.set_attribute("style", &cleaned)
                    .unwrap_or_else(|e| {
                        log::warn!("failed to rewrite style attribute: {e}");
                    });
            }
        }

        Ok(())
    }));

    let settings = RewriteStrSettings {
        element_content_handlers: element_handlers,
        ..RewriteStrSettings::default()
    };

    match rewrite_str(html, settings) {
        Ok(result) => result,
        Err(e) => {
            log::warn!("lol_html rewrite failed, continuing with input: {e}");
            html.to_string()
        }
    }
}

// ---------------------------------------------------------------------------
// Stage 3: Whitelist sanitization via ammonia
// ---------------------------------------------------------------------------

/// Build a configured `ammonia::Builder` for email HTML.
fn build_ammonia() -> ammonia::Builder<'static> {
    let mut builder = ammonia::Builder::default();

    let tags: HashSet<&str> = [
        "div", "span", "p", "br", "hr",
        "h1", "h2", "h3", "h4", "h5", "h6",
        "a", "img",
        "table", "thead", "tbody", "tfoot", "tr", "td", "th", "caption", "colgroup", "col",
        "ul", "ol", "li", "dl", "dt", "dd",
        "blockquote", "pre", "code",
        "em", "strong", "b", "i", "u", "s", "sub", "sup",
        "font", "center", "big", "small",
        "abbr", "cite", "q", "mark", "wbr",
    ]
    .into_iter()
    .collect();
    builder.tags(tags);

    // Generic attributes allowed on all tags.
    let generic: HashSet<&str> =
        ["style", "class", "id", "dir", "lang", "title"].into_iter().collect();
    builder.generic_attributes(generic);

    // Tag-specific attributes.
    let tag_attr_list: &[(&str, &[&str])] = &[
        ("a", &["href"]),
        ("img", &["src", "alt", "width", "height"]),
        ("td", &["colspan", "rowspan", "width", "height", "align", "valign", "bgcolor"]),
        ("th", &["colspan", "rowspan", "width", "height", "align", "valign", "bgcolor"]),
        ("table", &["width", "height", "align", "bgcolor", "border", "cellpadding", "cellspacing"]),
        ("tr", &["align", "valign", "bgcolor"]),
        ("font", &["color", "face", "size"]),
        ("col", &["width", "span"]),
        ("colgroup", &["width", "span"]),
    ];
    let mut tag_attrs: HashMap<&str, HashSet<&str>> = HashMap::new();
    for &(tag, attrs) in tag_attr_list {
        tag_attrs.insert(tag, attrs.iter().copied().collect());
    }
    builder.tag_attributes(tag_attrs);

    // URL schemes.
    let schemes: HashSet<&str> =
        ["http", "https", "mailto", "cid", "data"].into_iter().collect();
    builder.url_schemes(schemes);

    // Link security.
    builder.link_rel(Some("noopener noreferrer"));

    builder
}

static AMMONIA: LazyLock<ammonia::Builder<'static>> = LazyLock::new(build_ammonia);

fn whitelist_sanitize(html: &str) -> String {
    AMMONIA.clean(html).to_string()
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Sanitize an HTML email body for safe display.
///
/// Runs a three-stage pipeline:
/// 1. **CSS inlining** -- `<style>` blocks are inlined into element `style=`
///    attributes so styling survives later sanitisation.
/// 2. **Dangerous element removal** -- Streaming removal of `<script>`,
///    `<iframe>`, event-handler attributes, `javascript:` URLs, etc.
/// 3. **Whitelist sanitisation** -- Only explicitly allowed tags and attributes
///    remain.
///
/// If any stage fails internally it logs a warning and passes its input
/// through unchanged, so the pipeline is best-effort and never panics.
pub fn sanitize_html_body(html: &str) -> String {
    if html.is_empty() {
        return String::new();
    }

    log::debug!("Sanitizing HTML body ({} bytes)", html.len());

    let stage1 = inline_css(html);
    let stage2 = strip_dangerous_elements(&stage1);

    if stage2.len() < stage1.len() {
        log::warn!(
            "Stripped dangerous content from HTML: {} -> {} bytes",
            stage1.len(),
            stage2.len()
        );
    }

    whitelist_sanitize(&stage2)
}

/// Sanitize HTML with remote image blocking.
///
/// Same three-stage pipeline as [`sanitize_html_body`], plus:
/// - Remote `<img src="http(s)://...">` tags are replaced with a
///   placeholder unless the sender is allowlisted.
/// - `cid:` and `data:` URIs are always preserved (inline attachments).
/// - AMP-specific elements (`amp-img`, `amp-list`, etc.) are stripped.
pub fn sanitize_html_body_with_image_policy(
    html: &str,
    block_remote_images: bool,
    sender_is_allowlisted: bool,
) -> String {
    if html.is_empty() {
        return String::new();
    }

    let stage1 = inline_css(html);
    let stage2 = strip_dangerous_elements(&stage1);

    let stage2b = strip_amp_elements(&stage2);

    let stage2c = if block_remote_images && !sender_is_allowlisted {
        strip_remote_images(&stage2b)
    } else {
        stage2b
    };

    whitelist_sanitize(&stage2c)
}

// ---------------------------------------------------------------------------
// Remote image stripping
// ---------------------------------------------------------------------------

/// Replace remote `<img src="http(s)://...">` with a placeholder.
/// Preserves `cid:` (inline attachments) and `data:` (embedded) URIs.
fn strip_remote_images(html: &str) -> String {
    use lol_html::{element, rewrite_str, RewriteStrSettings};

    let settings = RewriteStrSettings {
        element_content_handlers: vec![element!("img[src]", |el| {
            if let Some(src) = el.get_attribute("src") {
                let lower = src.trim().to_ascii_lowercase();
                if lower.starts_with("http://") || lower.starts_with("https://") {
                    // Replace with a blocked-image placeholder
                    el.remove_attribute("src");
                    el.set_attribute("data-blocked-src", &src)
                        .unwrap_or_default();
                    el.set_attribute("alt", "[Remote image blocked]")
                        .unwrap_or_default();
                    el.set_attribute("style", "display:inline-block;width:20px;height:20px;background:#ddd;border:1px solid #ccc;")
                        .unwrap_or_default();
                }
                // cid: and data: URIs pass through untouched
            }
            Ok(())
        })],
        ..RewriteStrSettings::default()
    };

    match rewrite_str(html, settings) {
        Ok(result) => result,
        Err(e) => {
            log::warn!("Remote image stripping failed: {e}");
            html.to_string()
        }
    }
}

// ---------------------------------------------------------------------------
// AMP HTML stripping
// ---------------------------------------------------------------------------

/// AMP email elements that should be removed entirely (with content).
const AMP_REMOVE_TAGS: &[&str] = &[
    "amp-img",
    "amp-anim",
    "amp-carousel",
    "amp-accordion",
    "amp-sidebar",
    "amp-image-lightbox",
    "amp-fit-text",
    "amp-layout",
    "amp-selector",
    "amp-bind-macro",
    "amp-list",
    "amp-form",
    "amp-mustache",
    "amp-timeago",
];

/// Strip AMP-specific elements from HTML.
///
/// AMP for Email (`text/x-amp-html`) can execute dynamic content.
/// We neutralize by stripping all `amp-*` custom elements and removing
/// the `<html amp4email>` attribute.
fn strip_amp_elements(html: &str) -> String {
    use lol_html::{element, rewrite_str, RewriteStrSettings};

    let mut handlers = Vec::new();

    // Remove known AMP elements entirely
    for &tag in AMP_REMOVE_TAGS {
        handlers.push(element!(tag, |el| {
            el.remove();
            Ok(())
        }));
    }

    // Strip amp4email attribute from <html> tag
    handlers.push(element!("html", |el| {
        el.remove_attribute("amp4email");
        el.remove_attribute("⚡4email");
        Ok(())
    }));

    let settings = RewriteStrSettings {
        element_content_handlers: handlers,
        ..RewriteStrSettings::default()
    };

    match rewrite_str(html, settings) {
        Ok(result) => result,
        Err(e) => {
            log::warn!("AMP stripping failed: {e}");
            html.to_string()
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input() {
        assert_eq!(sanitize_html_body(""), "");
    }

    #[test]
    fn whitespace_only() {
        let result = sanitize_html_body("   \n\t  ");
        assert_eq!(result.trim(), "");
    }

    #[test]
    fn script_tag_removal() {
        let html = "<p>Hello</p><script>alert('xss')</script><p>World</p>";
        let result = sanitize_html_body(html);
        assert!(!result.contains("script"));
        assert!(!result.contains("alert"));
        assert!(result.contains("Hello"));
        assert!(result.contains("World"));
    }

    #[test]
    fn event_handler_removal() {
        let html = "<div onclick=\"alert('xss')\" onmouseover=\"steal()\">Safe text</div>";
        let result = sanitize_html_body(html);
        assert!(!result.contains("onclick"));
        assert!(!result.contains("onmouseover"));
        assert!(!result.contains("alert"));
        assert!(result.contains("Safe text"));
    }

    #[test]
    fn javascript_url_removal() {
        let html = "<a href=\"javascript:alert('xss')\">Click me</a>";
        let result = sanitize_html_body(html);
        assert!(!result.contains("javascript:"));
        assert!(result.contains("Click me"));
    }

    #[test]
    fn javascript_url_in_img_src() {
        let html = "<img src=\"javascript:alert('xss')\">";
        let result = sanitize_html_body(html);
        assert!(!result.contains("javascript:"));
    }

    #[test]
    fn css_inlining_preserves_styles() {
        let html = "<html><head><style>p { color: red; }</style></head><body><p>Red text</p></body></html>";
        let result = sanitize_html_body(html);
        assert!(!result.contains("<style>"));
        assert!(result.contains("color"));
        assert!(result.contains("Red text"));
    }

    #[test]
    fn safe_html_preserved() {
        let html = "<div><h1>Title</h1><p>Paragraph with <strong>bold</strong> and <em>italic</em>.</p><ul><li>Item 1</li><li>Item 2</li></ul></div>";
        let result = sanitize_html_body(html);
        assert!(result.contains("<h1>"));
        assert!(result.contains("<strong>"));
        assert!(result.contains("<em>"));
        assert!(result.contains("<li>"));
        assert!(result.contains("Item 1"));
    }

    #[test]
    fn table_preserved() {
        let html = "<table width=\"100%\" border=\"1\"><tr><td colspan=\"2\" bgcolor=\"#eee\">Cell</td></tr></table>";
        let result = sanitize_html_body(html);
        assert!(result.contains("<table"));
        assert!(result.contains("<td"));
        assert!(result.contains("Cell"));
        assert!(result.contains("colspan"));
    }

    #[test]
    fn links_preserved_with_safe_schemes() {
        let html = "<a href=\"https://example.com\">Link</a>";
        let result = sanitize_html_body(html);
        assert!(result.contains("https://example.com"));
        assert!(result.contains("noopener noreferrer"));
    }

    #[test]
    fn mailto_links_preserved() {
        let html = "<a href=\"mailto:user@example.com\">Email</a>";
        let result = sanitize_html_body(html);
        assert!(result.contains("mailto:user@example.com"));
    }

    #[test]
    fn images_preserved() {
        let html = "<img src=\"https://example.com/photo.jpg\" alt=\"Photo\" width=\"600\" height=\"400\">";
        let result = sanitize_html_body(html);
        assert!(result.contains("<img"));
        assert!(result.contains("https://example.com/photo.jpg"));
        assert!(result.contains("alt=\"Photo\""));
    }

    #[test]
    fn cid_images_preserved() {
        let html = "<img src=\"cid:image001@01D1234.ABCDEF00\">";
        let result = sanitize_html_body(html);
        assert!(result.contains("cid:"));
    }

    #[test]
    fn meta_refresh_removal() {
        let html = "<meta http-equiv=\"refresh\" content=\"0;url=https://evil.com\"><p>Content</p>";
        let result = sanitize_html_body(html);
        assert!(!result.contains("meta"));
        assert!(!result.contains("refresh"));
        assert!(result.contains("Content"));
    }

    #[test]
    fn iframe_removal() {
        let html = "<p>Before</p><iframe src=\"https://evil.com/tracker\"></iframe><p>After</p>";
        let result = sanitize_html_body(html);
        assert!(!result.contains("iframe"));
        assert!(!result.contains("evil.com"));
        assert!(result.contains("Before"));
        assert!(result.contains("After"));
    }

    #[test]
    fn form_element_removal() {
        let html = "<form action=\"https://evil.com/steal\"><input type=\"text\" name=\"password\"><button type=\"submit\">Submit</button></form>";
        let result = sanitize_html_body(html);
        assert!(!result.contains("form"));
        assert!(!result.contains("input"));
        assert!(!result.contains("button"));
        assert!(!result.contains("evil.com"));
    }

    #[test]
    fn object_embed_applet_removal() {
        let html = "<object data=\"evil.swf\"></object><embed src=\"evil.swf\"><applet code=\"Evil.class\"></applet><p>Safe</p>";
        let result = sanitize_html_body(html);
        assert!(!result.contains("object"));
        assert!(!result.contains("embed"));
        assert!(!result.contains("applet"));
        assert!(result.contains("Safe"));
    }

    #[test]
    fn import_in_inline_style_removed() {
        let html = "<div style=\"@import url('https://evil.com/steal.css'); color: blue;\">Text</div>";
        let result = sanitize_html_body(html);
        assert!(!result.contains("@import"));
        assert!(result.contains("color"));
        assert!(result.contains("Text"));
    }

    #[test]
    fn malformed_html_handled() {
        let html = "<div><p>Unclosed paragraph<div>Nested<script>bad()</script></div>";
        let result = sanitize_html_body(html);
        assert!(!result.contains("script"));
        assert!(!result.contains("bad()"));
        assert!(result.contains("Unclosed paragraph"));
    }

    #[test]
    fn style_attribute_preserved_after_inlining() {
        let html = "<div style=\"background-color: #f0f0f0; padding: 10px;\">Styled content</div>";
        let result = sanitize_html_body(html);
        assert!(result.contains("background-color"));
        assert!(result.contains("Styled content"));
    }

    #[test]
    fn link_tag_removal() {
        let html = "<link rel=\"stylesheet\" href=\"https://evil.com/styles.css\"><p>Content</p>";
        let result = sanitize_html_body(html);
        assert!(!result.contains("<link"));
        assert!(!result.contains("evil.com"));
        assert!(result.contains("Content"));
    }

    #[test]
    fn select_textarea_removal() {
        let html = "<select><option>A</option></select><textarea>Notes</textarea><p>Safe</p>";
        let result = sanitize_html_body(html);
        assert!(!result.contains("<select"));
        assert!(!result.contains("<textarea"));
        assert!(result.contains("Safe"));
    }

    #[test]
    fn data_uri_images_preserved() {
        let html = "<img src=\"data:image/png;base64,iVBOR...\">";
        let result = sanitize_html_body(html);
        assert!(result.contains("data:image/png"));
    }

    #[test]
    fn font_tag_preserved() {
        let html = "<font color=\"red\" face=\"Arial\" size=\"3\">Formatted</font>";
        let result = sanitize_html_body(html);
        assert!(result.contains("<font"));
        assert!(result.contains("Formatted"));
    }

    // ── Remote image blocking tests ────────────────────────

    #[test]
    fn remote_images_blocked() {
        let html = r#"<img src="https://tracker.example.com/pixel.gif"><p>Text</p>"#;
        let result = sanitize_html_body_with_image_policy(html, true, false);
        assert!(!result.contains("tracker.example.com"));
        assert!(result.contains("Remote image blocked"));
        assert!(result.contains("Text"));
    }

    #[test]
    fn remote_images_allowed_when_allowlisted() {
        let html = r#"<img src="https://example.com/photo.jpg"><p>Text</p>"#;
        let result = sanitize_html_body_with_image_policy(html, true, true);
        assert!(result.contains("https://example.com/photo.jpg"));
    }

    #[test]
    fn cid_images_not_blocked() {
        let html = r#"<img src="cid:image001@01D1234"><p>Text</p>"#;
        let result = sanitize_html_body_with_image_policy(html, true, false);
        assert!(result.contains("cid:"));
    }

    #[test]
    fn data_uri_images_not_blocked() {
        let html = r#"<img src="data:image/png;base64,iVBOR"><p>Text</p>"#;
        let result = sanitize_html_body_with_image_policy(html, true, false);
        assert!(result.contains("data:image/png"));
    }

    #[test]
    fn remote_images_pass_when_policy_off() {
        let html = r#"<img src="https://example.com/photo.jpg">"#;
        let result = sanitize_html_body_with_image_policy(html, false, false);
        assert!(result.contains("https://example.com/photo.jpg"));
    }

    // ── AMP blocking tests ──────────────────────────────────

    #[test]
    fn amp_elements_stripped() {
        let html = r#"<amp-img src="photo.jpg" width="300" height="200"></amp-img><p>Text</p>"#;
        let result = sanitize_html_body_with_image_policy(html, false, false);
        assert!(!result.contains("amp-img"));
        assert!(result.contains("Text"));
    }

    #[test]
    fn amp_carousel_stripped() {
        let html = r#"<amp-carousel type="slides"><div>Slide 1</div></amp-carousel><p>After</p>"#;
        let result = sanitize_html_body_with_image_policy(html, false, false);
        assert!(!result.contains("amp-carousel"));
        assert!(result.contains("After"));
    }

    #[test]
    fn amp4email_attribute_stripped() {
        let html = r#"<html amp4email><head></head><body><p>Content</p></body></html>"#;
        let result = sanitize_html_body_with_image_policy(html, false, false);
        assert!(!result.contains("amp4email"));
        assert!(result.contains("Content"));
    }

    // ── Existing tests ──────────────────────────────────────

    #[test]
    fn blockquote_preserved() {
        let html = "<blockquote style=\"border-left: 2px solid #ccc; padding-left: 10px;\">Quoted text</blockquote>";
        let result = sanitize_html_body(html);
        assert!(result.contains("<blockquote"));
        assert!(result.contains("Quoted text"));
    }

    #[test]
    fn multiple_event_handlers() {
        let html = "<img src=\"https://example.com/photo.jpg\" onerror=\"alert(1)\" onload=\"track()\">";
        let result = sanitize_html_body(html);
        assert!(!result.contains("onerror"));
        assert!(!result.contains("onload"));
        assert!(result.contains("https://example.com/photo.jpg"));
    }
}
