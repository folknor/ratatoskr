use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;
use std::io::Cursor;

use crate::error::SqueezeError;
use crate::{CompressOutput, CompressResult};

const BASE64_PNG_PREFIX: &str = "data:image/png;base64,";
const BASE64_JPEG_PREFIX: &str = "data:image/jpeg;base64,";

/// Editor-specific elements to strip entirely.
const EDITOR_ELEMENTS: &[&[u8]] = &[
    b"metadata",
    b"sodipodi:namedview",
    b"inkscape:grid",
    b"inkscape:perspective",
];

/// Editor-specific attribute prefixes to strip.
const EDITOR_ATTR_PREFIXES: &[&[u8]] = &[
    b"inkscape:",
    b"sodipodi:",
    b"xmlns:inkscape",
    b"xmlns:sodipodi",
    b"xmlns:dc",
    b"xmlns:cc",
    b"xmlns:rdf",
    b"xml:space",
];

/// Compress an SVG: strip editor metadata/comments, optimize embedded images.
pub fn compress_svg(input: &[u8], min_savings_pct: f32) -> Result<CompressResult, SqueezeError> {
    let result =
        minify_svg(input).map_err(|e| SqueezeError::ImageEncode(format!("SVG minify: {e}")))?;

    if result.len() >= input.len() {
        return Ok(CompressResult {
            original_size: input.len(),
            compressed_size: input.len(),
            output: CompressOutput::Unchanged,
            new_mime_type: None,
        });
    }

    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    let pct = ((1.0 - (result.len() as f64 / input.len() as f64)) * 100.0) as f32;

    if pct < min_savings_pct {
        return Ok(CompressResult {
            original_size: input.len(),
            compressed_size: input.len(),
            output: CompressOutput::Unchanged,
            new_mime_type: None,
        });
    }

    Ok(CompressResult {
        original_size: input.len(),
        compressed_size: result.len(),
        output: CompressOutput::Compressed(result),
        new_mime_type: None,
    })
}

fn minify_svg(data: &[u8]) -> Result<Vec<u8>, quick_xml::Error> {
    let mut reader = Reader::from_reader(data);
    reader.config_mut().trim_text_start = true;
    reader.config_mut().trim_text_end = true;

    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();
    let mut skip_depth: usize = 0;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            // Strip comments.
            Ok(Event::Comment(_)) => {}
            Ok(Event::Start(ref e)) => {
                if skip_depth > 0 {
                    skip_depth += 1;
                    buf.clear();
                    continue;
                }

                if should_skip_element(e.name().as_ref()) {
                    skip_depth = 1;
                    buf.clear();
                    continue;
                }

                if needs_attr_processing(e) {
                    let elem = process_attrs(e);
                    writer.write_event(Event::Start(elem))?;
                } else {
                    writer.write_event(Event::Start(e.clone().into_owned()))?;
                }
                buf.clear();
                continue;
            }
            Ok(Event::End(ref e)) => {
                if skip_depth > 0 {
                    skip_depth -= 1;
                    buf.clear();
                    continue;
                }
                writer.write_event(Event::End(e.clone()))?;
            }
            Ok(Event::Empty(ref e)) => {
                if skip_depth > 0 {
                    buf.clear();
                    continue;
                }

                if should_skip_element(e.name().as_ref()) {
                    buf.clear();
                    continue;
                }

                if needs_attr_processing(e) {
                    let elem = process_attrs(e);
                    writer.write_event(Event::Empty(elem))?;
                } else {
                    writer.write_event(Event::Empty(e.clone().into_owned()))?;
                }
                buf.clear();
                continue;
            }
            Ok(event) => {
                if skip_depth == 0 {
                    writer.write_event(event)?;
                }
            }
            Err(e) => return Err(e),
        }
        buf.clear();
    }

    Ok(writer.into_inner().into_inner())
}

fn should_skip_element(name: &[u8]) -> bool {
    EDITOR_ELEMENTS.contains(&name)
}

fn should_strip_attr(key: &[u8]) -> bool {
    EDITOR_ATTR_PREFIXES.iter().any(|&prefix| key.starts_with(prefix))
}

/// Check if an element has any attributes that need processing (editor attrs or embedded images).
fn needs_attr_processing(e: &quick_xml::events::BytesStart<'_>) -> bool {
    for attr in e.attributes().flatten() {
        if should_strip_attr(attr.key.as_ref()) {
            return true;
        }
        // Check for embedded base64 image data URIs.
        if attr.value.len() > 22 && attr.value.starts_with(b"data:image/") {
            return true;
        }
    }
    false
}

/// Process attributes: strip editor attrs, optimize embedded base64 images.
fn process_attrs(
    e: &quick_xml::events::BytesStart<'_>,
) -> quick_xml::events::BytesStart<'static> {
    let local_name = e.name().as_ref().to_vec();
    let mut elem = quick_xml::events::BytesStart::new(
        String::from_utf8_lossy(&local_name).into_owned(),
    );

    for attr in e.attributes().flatten() {
        if should_strip_attr(attr.key.as_ref()) {
            continue;
        }

        // Unescape XML entities in the value for inspection/optimization.
        let value = attr
            .unescape_value()
            .unwrap_or(std::borrow::Cow::Borrowed(""));

        // Try to optimize embedded PNG images.
        if value.starts_with(BASE64_PNG_PREFIX)
            && let Some(optimized) = optimize_embedded_png(&value)
        {
            elem.push_attribute((
                std::str::from_utf8(attr.key.as_ref()).unwrap_or_default(),
                optimized.as_str(),
            ));
            continue;
        }

        // Try to optimize embedded JPEG images.
        if value.starts_with(BASE64_JPEG_PREFIX)
            && let Some(optimized) = optimize_embedded_jpeg(&value)
        {
            elem.push_attribute((
                std::str::from_utf8(attr.key.as_ref()).unwrap_or_default(),
                optimized.as_str(),
            ));
            continue;
        }

        // Push original raw attribute (preserves entity encoding).
        elem.push_attribute(attr);
    }

    elem
}

/// Maximum base64 data URI length we'll decode and optimize (10 MB of base64 ≈ 7.5 MB decoded).
const MAX_EMBEDDED_B64_LEN: usize = 10 * 1024 * 1024;

/// Strip whitespace from base64 data (handles &#10; decoded to \n, spaces, etc.).
fn decode_base64_lenient(b64_data: &str) -> Option<Vec<u8>> {
    let clean: String = b64_data
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    BASE64.decode(&clean).ok()
}

/// Optimize an embedded base64 PNG via oxipng.
fn optimize_embedded_png(value: &str) -> Option<String> {
    let b64_data = value.strip_prefix(BASE64_PNG_PREFIX)?;
    if b64_data.len() > MAX_EMBEDDED_B64_LEN {
        return None;
    }
    let png_bytes = decode_base64_lenient(b64_data)?;

    let opts = oxipng::Options::from_preset(4);
    let optimized = oxipng::optimize_from_memory(&png_bytes, &opts).ok()?;

    if optimized.len() >= png_bytes.len() {
        return None;
    }

    let mut result = String::with_capacity(BASE64_PNG_PREFIX.len() + optimized.len() * 4 / 3 + 4);
    result.push_str(BASE64_PNG_PREFIX);
    BASE64.encode_string(&optimized, &mut result);
    Some(result)
}

/// Optimize an embedded base64 JPEG via mozjpeg.
fn optimize_embedded_jpeg(value: &str) -> Option<String> {
    let b64_data = value.strip_prefix(BASE64_JPEG_PREFIX)?;
    if b64_data.len() > MAX_EMBEDDED_B64_LEN {
        return None;
    }
    let jpeg_bytes = decode_base64_lenient(b64_data)?;

    let img = image::load_from_memory_with_format(&jpeg_bytes, image::ImageFormat::Jpeg).ok()?;
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();

    let compressed = mozjpeg_rs::Encoder::new(mozjpeg_rs::Preset::ProgressiveBalanced)
        .quality(75)
        .encode_rgb(rgb.as_raw(), w, h)
        .ok()?;

    if compressed.len() >= jpeg_bytes.len() {
        return None;
    }

    let mut result =
        String::with_capacity(BASE64_JPEG_PREFIX.len() + compressed.len() * 4 / 3 + 4);
    result.push_str(BASE64_JPEG_PREFIX);
    BASE64.encode_string(&compressed, &mut result);
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strips_comments() {
        let svg = br#"<svg><!-- comment --><rect/></svg>"#;
        let result = minify_svg(svg).expect("minify failed");
        let out = String::from_utf8(result).expect("invalid utf8");
        assert!(!out.contains("comment"));
        assert!(out.contains("<rect"));
    }

    #[test]
    fn test_strips_metadata() {
        let svg = br#"<svg><metadata><rdf>stuff</rdf></metadata><rect/></svg>"#;
        let result = minify_svg(svg).expect("minify failed");
        let out = String::from_utf8(result).expect("invalid utf8");
        assert!(!out.contains("metadata"));
        assert!(!out.contains("rdf"));
        assert!(out.contains("<rect"));
    }

    #[test]
    fn test_strips_inkscape_attrs() {
        let svg = br#"<svg inkscape:version="1.0" width="100"><rect/></svg>"#;
        let result = minify_svg(svg).expect("minify failed");
        let out = String::from_utf8(result).expect("invalid utf8");
        assert!(!out.contains("inkscape:version"));
        assert!(out.contains("width"));
    }
}
