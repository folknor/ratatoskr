use regex::Regex;
use std::sync::LazyLock;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use xxhash_rust::xxh3::xxh3_64;

use ratatoskr_inline_image_store::{InlineImage, InlineImageStoreState};

// ---------------------------------------------------------------------------
// Regex patterns – compiled once via LazyLock
// ---------------------------------------------------------------------------

/// Matches `<img …>` tags (case-insensitive, non-greedy).
static IMG_TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<img\b[^>]*>").expect("IMG_TAG_RE"));

/// Extracts the `src` attribute value from inside an `<img>` tag body.
static SRC_ATTR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)\bsrc\s*=\s*["']([^"']+)["']"#).expect("SRC_ATTR_RE"));

/// Parses a data URI: `data:<mime>;base64,<payload>`.
static DATA_URI_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^data:([^;]+);base64,(.+)$").expect("DATA_URI_RE")
});

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Result of processing signature images: the rewritten HTML and the images
/// extracted from data URIs, ready for batch insertion into the inline store.
pub struct ProcessedSignatureImages {
    /// HTML with data-URI `<img src="…">` rewritten to content-hash references.
    pub html: String,
    /// Decoded images to insert into the inline image store.
    pub images: Vec<InlineImage>,
}

/// Extract base64 data-URI images from signature HTML, decode them, generate
/// content hashes for deduplication, and rewrite the `src` attributes to use
/// `inline-image:<content_hash>` references that the rendering layer resolves.
///
/// `cid:` references are left untouched — they require MIME context that is not
/// available during signature import.
///
/// This is a **synchronous** extraction step. Call
/// [`store_signature_images`] afterwards to persist the images.
pub fn process_signature_images(html: &str) -> ProcessedSignatureImages {
    let mut images: Vec<InlineImage> = Vec::new();
    let mut seen_hashes: std::collections::HashSet<String> = std::collections::HashSet::new();

    let result = IMG_TAG_RE.replace_all(html, |caps: &regex::Captures<'_>| {
        let tag = caps.get(0).map_or("", |m| m.as_str());

        let Some(src_caps) = SRC_ATTR_RE.captures(tag) else {
            return tag.to_string();
        };
        let src = src_caps.get(1).map_or("", |m| m.as_str());

        // Only process data URIs; leave cid: and http(s): references alone.
        let Some(data_caps) = DATA_URI_RE.captures(src) else {
            return tag.to_string();
        };

        let mime_type = data_caps.get(1).map_or("", |m| m.as_str());
        let b64_payload = data_caps.get(2).map_or("", |m| m.as_str());

        // Decode base64 payload — skip this image on failure.
        let Ok(data) = BASE64.decode(b64_payload) else {
            return tag.to_string();
        };

        // Content-addressed hash for deduplication.
        let content_hash = format!("{:016x}", xxh3_64(&data));
        let local_ref = format!("inline-image:{content_hash}");

        // Collect unique images for storage.
        if seen_hashes.insert(content_hash.clone()) {
            images.push(InlineImage {
                content_hash: content_hash.clone(),
                data,
                mime_type: mime_type.to_string(),
            });
        }

        // Rewrite the src attribute value inside the tag.
        SRC_ATTR_RE
            .replace(tag, |src_caps: &regex::Captures<'_>| {
                let full = src_caps.get(0).map_or("", |m| m.as_str());
                // Preserve the quote style from the original attribute.
                let quote = if full.contains('"') { '"' } else { '\'' };
                format!("src={quote}{local_ref}{quote}")
            })
            .to_string()
    });

    ProcessedSignatureImages {
        html: result.into_owned(),
        images,
    }
}

/// Persist extracted signature images into the inline image store.
///
/// Convenience wrapper around [`InlineImageStoreState::put_batch`] that takes
/// the images from [`process_signature_images`].
pub async fn store_signature_images(
    inline_store: &InlineImageStoreState,
    images: Vec<InlineImage>,
) -> Result<(), String> {
    inline_store.put_batch(images).await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_data_uri_image() {
        // 1x1 red PNG pixel (minimal valid PNG).
        let png_b64 = BASE64.encode(b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR");
        let html = format!(r#"<p>Hello</p><img src="data:image/png;base64,{png_b64}"><p>Bye</p>"#);

        let result = process_signature_images(&html);

        assert_eq!(result.images.len(), 1);
        assert_eq!(result.images[0].mime_type, "image/png");
        assert!(!result.html.contains("data:image/png"));
        assert!(result.html.contains("inline-image:"));
    }

    #[test]
    fn leaves_cid_references_alone() {
        let html = r#"<img src="cid:logo@example.com">"#;
        let result = process_signature_images(html);
        assert!(result.images.is_empty());
        assert_eq!(result.html, html);
    }

    #[test]
    fn leaves_http_references_alone() {
        let html = r#"<img src="https://example.com/logo.png">"#;
        let result = process_signature_images(html);
        assert!(result.images.is_empty());
        assert_eq!(result.html, html);
    }

    #[test]
    fn deduplicates_identical_images() {
        let png_b64 = BASE64.encode(b"identical-image-data");
        let html = format!(
            r#"<img src="data:image/png;base64,{png_b64}"><img src="data:image/png;base64,{png_b64}">"#
        );

        let result = process_signature_images(&html);

        // Both tags should be rewritten but only one image stored.
        assert_eq!(result.images.len(), 1);
        assert!(!result.html.contains("data:image/png"));
        // Both tags should reference the same hash.
        let hash = &result.images[0].content_hash;
        let count = result.html.matches(hash.as_str()).count();
        assert_eq!(count, 2);
    }

    #[test]
    fn handles_multiple_different_images() {
        let img1 = BASE64.encode(b"image-data-1");
        let img2 = BASE64.encode(b"image-data-2");
        let html = format!(
            r#"<img src="data:image/png;base64,{img1}"><img src="data:image/jpeg;base64,{img2}">"#
        );

        let result = process_signature_images(&html);

        assert_eq!(result.images.len(), 2);
        assert_eq!(result.images[0].mime_type, "image/png");
        assert_eq!(result.images[1].mime_type, "image/jpeg");
    }

    #[test]
    fn preserves_non_img_html() {
        let html = r#"<div><p>My signature</p><a href="https://example.com">Link</a></div>"#;
        let result = process_signature_images(html);
        assert!(result.images.is_empty());
        assert_eq!(result.html, html);
    }

    #[test]
    fn handles_invalid_base64_gracefully() {
        let html = r#"<img src="data:image/png;base64,!!!not-valid-base64!!!">"#;
        let result = process_signature_images(html);
        // Invalid base64 should be left as-is.
        assert!(result.images.is_empty());
        assert_eq!(result.html, html);
    }

    #[test]
    fn case_insensitive_data_uri() {
        let png_b64 = BASE64.encode(b"case-test");
        let html = format!(r#"<IMG SRC="data:Image/PNG;Base64,{png_b64}">"#);

        let result = process_signature_images(&html);

        assert_eq!(result.images.len(), 1);
        assert!(result.html.contains("inline-image:"));
    }

    #[test]
    fn preserves_other_img_attributes() {
        let png_b64 = BASE64.encode(b"attr-test");
        let html = format!(
            r#"<img alt="Logo" width="100" src="data:image/png;base64,{png_b64}" height="50">"#
        );

        let result = process_signature_images(&html);

        assert!(result.html.contains("alt=\"Logo\""));
        assert!(result.html.contains("width=\"100\""));
        assert!(result.html.contains("height=\"50\""));
        assert!(result.html.contains("inline-image:"));
    }
}
