use regex::Regex;
use std::sync::LazyLock;

/// Why an image was flagged as a likely tracking pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackingPixelReason {
    /// `width`/`height` HTML attributes are 0 or 1 (e.g., `<img width="1" height="1">`)
    TinyDimensions,
    /// Inline CSS sets dimensions to 0–1 px, `display:none`, or `visibility:hidden`
    HiddenByStyle,
    /// The image URL matches a known tracking-pixel domain or path pattern
    KnownTracker,
}

/// A single detected tracking pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackingPixelResult {
    pub url: String,
    pub reason: TrackingPixelReason,
}

// ---------------------------------------------------------------------------
// Regex patterns – compiled once via LazyLock
// ---------------------------------------------------------------------------

/// Matches `<img …>` tags (case-insensitive, non-greedy).
static IMG_TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<img\b[^>]*>").expect("IMG_TAG_RE"));

/// Extracts the `src` attribute value from inside an `<img>` tag body.
static SRC_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)\bsrc\s*=\s*["']([^"']+)["']"#).expect("SRC_RE"));

/// Extracts the `width` HTML attribute (bare, not inside `style`).
static WIDTH_ATTR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)\bwidth\s*=\s*["']?\s*([0-9]+)"#).expect("WIDTH_ATTR_RE"));

/// Extracts the `height` HTML attribute (bare, not inside `style`).
static HEIGHT_ATTR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\bheight\s*=\s*["']?\s*([0-9]+)"#).expect("HEIGHT_ATTR_RE")
});

/// Extracts the `style` attribute value.
static STYLE_ATTR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)\bstyle\s*=\s*["']([^"']*)["']"#).expect("STYLE_ATTR_RE"));

/// Matches `width` in CSS (e.g., `width:1px`, `width: 0`).
static CSS_WIDTH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bwidth\s*:\s*([0-9]+)\s*px").expect("CSS_WIDTH_RE"));

/// Matches `height` in CSS.
static CSS_HEIGHT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bheight\s*:\s*([0-9]+)\s*px").expect("CSS_HEIGHT_RE"));

/// Matches `display:none` in CSS.
static DISPLAY_NONE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bdisplay\s*:\s*none\b").expect("DISPLAY_NONE_RE"));

/// Matches `visibility:hidden` in CSS.
static VISIBILITY_HIDDEN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bvisibility\s*:\s*hidden\b").expect("VISIBILITY_HIDDEN_RE")
});

// ---------------------------------------------------------------------------
// Known tracker domain/path patterns
// ---------------------------------------------------------------------------

/// Exact path-prefix patterns checked with `contains` or `starts_with`.
const KNOWN_TRACKER_PATHS: &[&str] = &[
    "list-manage.com/track",
    "sendgrid.net/wf/open",
    "track.hubspot.com",
    "beacon.krxd.net",
    "mail.google.com/mail/u/0/images/cleardot.gif",
    // Outlook / Microsoft
    "tse-mercury.com",
];

/// Subdomain prefixes that strongly indicate tracking pixels.
const TRACKER_SUBDOMAINS: &[&str] = &["open.", "track.", "pixel.", "beacon.", "trk."];

/// Returns `true` if `url` matches a known tracking pixel domain or path.
fn is_known_tracker(url: &str) -> bool {
    // Normalise for comparison.
    let lower = url.to_ascii_lowercase();

    for pattern in KNOWN_TRACKER_PATHS {
        if lower.contains(pattern) {
            return true;
        }
    }

    // Extract the host portion (between `://` and the next `/` or end).
    if let Some(after_scheme) = lower.split_once("://").map(|(_, rest)| rest) {
        let host = after_scheme.split('/').next().unwrap_or(after_scheme);
        for sub in TRACKER_SUBDOMAINS {
            if host.starts_with(sub) {
                return true;
            }
        }
    }

    false
}

// ---------------------------------------------------------------------------
// Dimension helpers
// ---------------------------------------------------------------------------

/// Returns `true` when the value parsed from a dimension attribute is 0 or 1.
fn is_tiny(value: &str) -> bool {
    matches!(value, "0" | "1")
}

/// Check HTML attributes `width` and `height` for 0/1 values.
fn has_tiny_attribute_dimensions(tag_body: &str) -> bool {
    let w = WIDTH_ATTR_RE
        .captures(tag_body)
        .and_then(|c| c.get(1).map(|m| m.as_str()));
    let h = HEIGHT_ATTR_RE
        .captures(tag_body)
        .and_then(|c| c.get(1).map(|m| m.as_str()));

    // Both must be present and tiny (avoid false positives on images where only
    // one dimension is set to 1 for aspect-ratio reasons).
    matches!((w, h), (Some(w), Some(h)) if is_tiny(w) && is_tiny(h))
}

/// Check inline `style` for hidden/tiny-dimension CSS.
fn has_hidden_style(tag_body: &str) -> bool {
    let Some(caps) = STYLE_ATTR_RE.captures(tag_body) else {
        return false;
    };
    let style = caps.get(1).map_or("", |m| m.as_str());

    // display:none or visibility:hidden → definitely hidden
    if DISPLAY_NONE_RE.is_match(style) || VISIBILITY_HIDDEN_RE.is_match(style) {
        return true;
    }

    // Both CSS width and height ≤1 px
    let w = CSS_WIDTH_RE
        .captures(style)
        .and_then(|c| c.get(1).map(|m| m.as_str()));
    let h = CSS_HEIGHT_RE
        .captures(style)
        .and_then(|c| c.get(1).map(|m| m.as_str()));
    matches!((w, h), (Some(w), Some(h)) if is_tiny(w) && is_tiny(h))
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Scan an HTML string for `<img>` tags that look like tracking pixels.
///
/// Returns one [`TrackingPixelResult`] per detected pixel. A single `<img>` tag
/// can only produce one result — the first matching reason wins (known-tracker
/// is checked first so the reason is as specific as possible).
pub fn detect_tracking_pixels_in_html(html: &str) -> Vec<TrackingPixelResult> {
    let mut results: Vec<TrackingPixelResult> = Vec::new();

    for img_match in IMG_TAG_RE.find_iter(html) {
        let tag = img_match.as_str();

        // Extract src – skip tags without one.
        let Some(src_caps) = SRC_RE.captures(tag) else {
            continue;
        };
        let url = src_caps
            .get(1)
            .map_or_else(String::new, |m| m.as_str().to_string());
        if url.is_empty() {
            continue;
        }

        // 1. Known tracker domain/path (most specific reason)
        if is_known_tracker(&url) {
            results.push(TrackingPixelResult {
                url,
                reason: TrackingPixelReason::KnownTracker,
            });
            continue;
        }

        // 2. Tiny HTML attribute dimensions (width="1" height="1")
        if has_tiny_attribute_dimensions(tag) {
            results.push(TrackingPixelResult {
                url,
                reason: TrackingPixelReason::TinyDimensions,
            });
            continue;
        }

        // 3. Hidden via inline style
        if has_hidden_style(tag) {
            results.push(TrackingPixelResult {
                url,
                reason: TrackingPixelReason::HiddenByStyle,
            });
            continue;
        }
    }

    results
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_tiny_dimension_pixel() {
        let html = r#"<img src="https://example.com/logo.png" width="1" height="1">"#;
        let results = detect_tracking_pixels_in_html(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].reason, TrackingPixelReason::TinyDimensions);
        assert_eq!(results[0].url, "https://example.com/logo.png");
    }

    #[test]
    fn detects_zero_dimension_pixel() {
        let html = r#"<img width="0" height="0" src="https://example.com/t.gif">"#;
        let results = detect_tracking_pixels_in_html(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].reason, TrackingPixelReason::TinyDimensions);
    }

    #[test]
    fn detects_css_tiny_dimensions() {
        let html =
            r#"<img src="https://example.com/pixel.gif" style="width:1px;height:1px;border:0">"#;
        let results = detect_tracking_pixels_in_html(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].reason, TrackingPixelReason::HiddenByStyle);
    }

    #[test]
    fn detects_display_none() {
        let html =
            r#"<img src="https://example.com/pixel.gif" style="display:none" width="100" height="100">"#;
        let results = detect_tracking_pixels_in_html(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].reason, TrackingPixelReason::HiddenByStyle);
    }

    #[test]
    fn detects_visibility_hidden() {
        let html =
            r#"<img src="https://example.com/pixel.gif" style="visibility: hidden">"#;
        let results = detect_tracking_pixels_in_html(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].reason, TrackingPixelReason::HiddenByStyle);
    }

    #[test]
    fn detects_known_tracker_mailchimp() {
        let html =
            r#"<img src="https://list-manage.com/track/open.php?u=abc&id=123" width="100" height="50">"#;
        let results = detect_tracking_pixels_in_html(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].reason, TrackingPixelReason::KnownTracker);
    }

    #[test]
    fn detects_known_tracker_sendgrid() {
        let html = r#"<img src="https://u123.ct.sendgrid.net/wf/open?upn=abc">"#;
        let results = detect_tracking_pixels_in_html(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].reason, TrackingPixelReason::KnownTracker);
    }

    #[test]
    fn detects_tracker_subdomain() {
        let html = r#"<img src="https://pixel.example.com/track.gif">"#;
        let results = detect_tracking_pixels_in_html(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].reason, TrackingPixelReason::KnownTracker);
    }

    #[test]
    fn detects_open_subdomain() {
        let html = r#"<img src="https://open.company.com/email/12345">"#;
        let results = detect_tracking_pixels_in_html(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].reason, TrackingPixelReason::KnownTracker);
    }

    #[test]
    fn ignores_normal_images() {
        let html = r#"<img src="https://example.com/photo.jpg" width="600" height="400">"#;
        let results = detect_tracking_pixels_in_html(html);
        assert!(results.is_empty());
    }

    #[test]
    fn ignores_single_small_dimension() {
        // Only width=1 but height is normal — not a tracking pixel
        let html = r#"<img src="https://example.com/spacer.gif" width="1" height="20">"#;
        let results = detect_tracking_pixels_in_html(html);
        assert!(results.is_empty());
    }

    #[test]
    fn ignores_img_without_src() {
        let html = r#"<img width="1" height="1">"#;
        let results = detect_tracking_pixels_in_html(html);
        assert!(results.is_empty());
    }

    #[test]
    fn detects_multiple_pixels() {
        let html = r#"
            <img src="https://example.com/real.jpg" width="600" height="400">
            <img src="https://track.hubspot.com/e.gif">
            <img src="https://example.com/pix.gif" width="1" height="1">
        "#;
        let results = detect_tracking_pixels_in_html(html);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].reason, TrackingPixelReason::KnownTracker);
        assert_eq!(results[1].reason, TrackingPixelReason::TinyDimensions);
    }

    #[test]
    fn case_insensitive_tag_and_attrs() {
        let html = r#"<IMG SRC="https://example.com/t.gif" WIDTH="1" HEIGHT="1">"#;
        let results = detect_tracking_pixels_in_html(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].reason, TrackingPixelReason::TinyDimensions);
    }

    #[test]
    fn detects_google_cleardot() {
        let html = r#"<img src="https://mail.google.com/mail/u/0/images/cleardot.gif">"#;
        let results = detect_tracking_pixels_in_html(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].reason, TrackingPixelReason::KnownTracker);
    }
}
