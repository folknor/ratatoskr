use std::collections::{HashMap, HashSet};
use std::io::Cursor;

use image::{DynamicImage, ImageReader};
use lopdf::{Document, Object, ObjectId};

use crate::config::Config;
use crate::error::SqueezeError;
use crate::image::compress_image_raw;
use crate::{CompressOutput, CompressResult};

/// The type of image encoding in a PDF stream.
#[derive(Debug, Clone, Copy)]
enum ImageFilter {
    /// DCTDecode — stream bytes are a raw JPEG.
    Dct,
    /// FlateDecode — stream bytes are zlib-compressed raw pixels.
    Flat,
}

/// Metadata extracted from an image XObject dictionary.
struct ImageInfo {
    filter: ImageFilter,
    width: u32,
    height: u32,
    /// Bits per component (typically 8).
    bpc: u32,
    /// Number of color components (1=gray, 3=RGB, 4=CMYK).
    components: u32,
}

/// Compress a PDF document: image recompression, stream dedup, structural cleanup.
pub fn compress_pdf(input: &[u8], config: &Config) -> Result<CompressResult, SqueezeError> {
    let mut doc =
        Document::load_mem(input).map_err(|e| SqueezeError::PdfParse(e.to_string()))?;

    // Phase 1: Recompress image XObjects.
    recompress_images(&mut doc, config);

    // Phase 2: Deduplicate identical streams (repeated logos, headers, etc.).
    deduplicate_streams(&mut doc);

    // Phase 3: Remove unreferenced objects.
    remove_unused_objects(&mut doc);

    // Phase 4: Strip non-essential metadata.
    strip_metadata(&mut doc);

    // Phase 5: Renumber objects sequentially.
    doc.renumber_objects();

    // Phase 6: Write using modern format (PDF 1.5 object streams + xref streams).
    let mut output = Vec::new();
    doc.save_modern(&mut output)
        .map_err(|e| SqueezeError::PdfWrite(e.to_string()))?;

    // Apply the same min_savings_pct threshold as images and SVGs.
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    let savings = if input.is_empty() {
        0.0_f32
    } else {
        ((1.0 - (output.len() as f64 / input.len() as f64)) * 100.0) as f32
    };

    if savings >= config.min_savings_pct {
        Ok(CompressResult {
            original_size: input.len(),
            compressed_size: output.len(),
            output: CompressOutput::Compressed(output),
            new_mime_type: None,
        })
    } else {
        Ok(CompressResult {
            original_size: input.len(),
            compressed_size: input.len(),
            output: CompressOutput::Unchanged,
            new_mime_type: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Phase 1: Image recompression
// ---------------------------------------------------------------------------

fn recompress_images(doc: &mut Document, config: &Config) {
    let object_ids: Vec<_> = doc.objects.keys().copied().collect();

    for id in object_ids {
        let info = {
            let Some(obj) = doc.objects.get(&id) else {
                continue;
            };
            match classify_image_xobject(obj) {
                Some(info) => info,
                None => continue,
            }
        };

        match info.filter {
            ImageFilter::Dct => try_recompress_dct(doc, id, &info, config),
            ImageFilter::Flat => try_recompress_flat(doc, id, &info, config),
        }
    }
}

fn classify_image_xobject(obj: &Object) -> Option<ImageInfo> {
    let Object::Stream(stream) = obj else {
        return None;
    };

    let dict = &stream.dict;

    let is_image = dict
        .get(b"Subtype")
        .is_ok_and(|v| matches!(v, Object::Name(n) if n == b"Image"));

    if !is_image {
        return None;
    }

    let filter = match dict.get(b"Filter").ok() {
        Some(Object::Name(n)) if n == b"DCTDecode" => ImageFilter::Dct,
        Some(Object::Array(arr))
            if arr.len() == 1
                && matches!(arr.first(), Some(Object::Name(n)) if n == b"DCTDecode") =>
        {
            ImageFilter::Dct
        }
        Some(Object::Name(n)) if n == b"FlateDecode" => ImageFilter::Flat,
        Some(Object::Array(arr))
            if arr.len() == 1
                && matches!(arr.first(), Some(Object::Name(n)) if n == b"FlateDecode") =>
        {
            ImageFilter::Flat
        }
        _ => return None,
    };

    let width = get_int(dict, b"Width")?;
    let height = get_int(dict, b"Height")?;
    let bpc = get_int(dict, b"BitsPerComponent").unwrap_or(8);

    let components = match dict.get(b"ColorSpace").ok() {
        Some(Object::Name(n)) => match n.as_slice() {
            b"DeviceRGB" => 3,
            b"DeviceGray" => 1,
            b"DeviceCMYK" => 4,
            _ => return None,
        },
        _ => return None,
    };

    Some(ImageInfo {
        filter,
        width,
        height,
        bpc,
        components,
    })
}

fn get_int(dict: &lopdf::Dictionary, key: &[u8]) -> Option<u32> {
    match dict.get(key).ok()? {
        Object::Integer(n) => u32::try_from(*n).ok().filter(|&v| v > 0),
        _ => None,
    }
}

fn try_recompress_dct(doc: &mut Document, id: ObjectId, info: &ImageInfo, config: &Config) {
    if info.components == 4 {
        return;
    }

    let stream_bytes = {
        let Some(Object::Stream(stream)) = doc.objects.get(&id) else {
            return;
        };
        stream.content.clone()
    };
    let original_len = stream_bytes.len();

    let result = (|| -> Result<Option<Vec<u8>>, SqueezeError> {
        let img = ImageReader::with_format(Cursor::new(&stream_bytes), image::ImageFormat::Jpeg)
            .decode()
            .map_err(|e| SqueezeError::ImageDecode(e.to_string()))?;

        compress_image_raw(
            &img,
            original_len,
            config,
            config.pdf_image_quality,
            config.pdf_image_max_dim,
        )
    })();

    if let Ok(Some(compressed)) = result
        && compressed.len() < original_len
        && let Some(Object::Stream(stream)) = doc.objects.get_mut(&id)
    {
        stream.set_content(compressed);
    }
}

/// Maximum raw pixel buffer we'll allocate for a single PDF image (256 MB).
const MAX_PDF_RAW_IMAGE_BYTES: u64 = 256 * 1024 * 1024;

fn try_recompress_flat(doc: &mut Document, id: ObjectId, info: &ImageInfo, config: &Config) {
    if info.components == 4 {
        return;
    }
    if info.bpc != 8 {
        return;
    }

    // Skip images with predictors (e.g. PNG predictors prepend a filter byte
    // per row). After zlib decompression, the bytes aren't raw interleaved
    // pixels — treating them as such produces silently corrupted JPEGs.
    {
        let Some(Object::Stream(stream)) = doc.objects.get(&id) else {
            return;
        };
        if has_predictor(&stream.dict) {
            return;
        }
    }

    let raw_pixels = {
        let Some(Object::Stream(stream)) = doc.objects.get_mut(&id) else {
            return;
        };
        if stream.decompress().is_err() {
            return;
        }
        stream.content.clone()
    };

    // Use checked arithmetic to prevent overflow on crafted dimensions.
    let Some(expected_len) = u64::from(info.width)
        .checked_mul(u64::from(info.height))
        .and_then(|n| n.checked_mul(u64::from(info.components)))
    else {
        recompress_flat_stream(doc, id);
        return;
    };

    // Bail if the raw pixel buffer would be unreasonably large.
    if expected_len > MAX_PDF_RAW_IMAGE_BYTES {
        recompress_flat_stream(doc, id);
        return;
    }

    #[allow(clippy::cast_possible_truncation)]
    let expected_len = expected_len as usize;

    if raw_pixels.len() < expected_len {
        recompress_flat_stream(doc, id);
        return;
    }

    let img: Option<DynamicImage> = match info.components {
        3 => image::RgbImage::from_raw(
            info.width,
            info.height,
            raw_pixels[..expected_len].to_vec(),
        )
        .map(DynamicImage::ImageRgb8),
        1 => image::GrayImage::from_raw(
            info.width,
            info.height,
            raw_pixels[..expected_len].to_vec(),
        )
        .map(DynamicImage::ImageLuma8),
        _ => None,
    };

    let Some(img) = img else {
        recompress_flat_stream(doc, id);
        return;
    };

    let jpeg_result = compress_image_raw(
        &img,
        expected_len,
        config,
        config.pdf_image_quality,
        config.pdf_image_max_dim,
    );

    match jpeg_result {
        Ok(Some(jpeg_bytes)) => {
            if let Some(Object::Stream(stream)) = doc.objects.get_mut(&id) {
                stream.set_content(jpeg_bytes);
                stream
                    .dict
                    .set("Filter", Object::Name(b"DCTDecode".to_vec()));
                stream.dict.remove(b"DecodeParms");
            }
        }
        _ => {
            recompress_flat_stream(doc, id);
        }
    }
}

fn recompress_flat_stream(doc: &mut Document, id: ObjectId) {
    if let Some(Object::Stream(stream)) = doc.objects.get_mut(&id) {
        let _ = stream.compress();
    }
}

/// Check if a stream's /DecodeParms specifies a predictor (> 1).
/// Predictor 1 = no prediction. Predictor 2 = TIFF. 10-15 = PNG sub/up/avg/paeth/optimum.
fn has_predictor(dict: &lopdf::Dictionary) -> bool {
    let Ok(params) = dict.get(b"DecodeParms") else {
        return false;
    };
    let check = |d: &lopdf::Dictionary| -> bool {
        matches!(d.get(b"Predictor").ok(), Some(Object::Integer(p)) if *p > 1)
    };
    match params {
        Object::Dictionary(d) => check(d),
        // Filter arrays can have per-filter DecodeParms.
        Object::Array(arr) => arr.iter().any(|item| {
            matches!(item, Object::Dictionary(d) if check(d))
        }),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Phase 2: Stream deduplication
// ---------------------------------------------------------------------------

/// Find streams with identical content+filter, replace duplicates with
/// references to a single canonical object.
fn deduplicate_streams(doc: &mut Document) {
    use std::hash::{Hash, Hasher};

    // Hash content + filter for each stream.
    let mut hash_map: HashMap<u64, ObjectId> = HashMap::new();
    let mut duplicates: Vec<(ObjectId, ObjectId)> = Vec::new();

    let object_ids: Vec<_> = doc.objects.keys().copied().collect();

    for id in &object_ids {
        let Some(Object::Stream(stream)) = doc.objects.get(id) else {
            continue;
        };

        let mut hasher = std::hash::DefaultHasher::new();
        stream.content.hash(&mut hasher);
        if let Ok(filter) = stream.dict.get(b"Filter") {
            format!("{filter:?}").hash(&mut hasher);
        }
        let h = hasher.finish();

        if let Some(&canonical_id) = hash_map.get(&h) {
            if canonical_id != *id {
                // Verify actual content equality (hash collision guard).
                let content_matches = doc
                    .objects
                    .get(&canonical_id)
                    .is_some_and(|canonical_obj| {
                        if let (Object::Stream(a), Object::Stream(b)) = (canonical_obj, &Object::Stream(stream.clone())) {
                            a.content == b.content
                        } else {
                            false
                        }
                    });
                if content_matches {
                    duplicates.push((*id, canonical_id));
                }
            }
        } else {
            hash_map.insert(h, *id);
        }
    }

    if duplicates.is_empty() {
        return;
    }

    let replace_map: HashMap<ObjectId, ObjectId> = duplicates.iter().copied().collect();

    // Replace all references to duplicates with canonical.
    let all_ids: Vec<_> = doc.objects.keys().copied().collect();
    for id in all_ids {
        if let Some(obj) = doc.objects.get_mut(&id) {
            replace_references(obj, &replace_map, 0);
        }
    }
    replace_refs_in_dict(&mut doc.trailer, &replace_map, 0);

    // Remove duplicate objects.
    for (dup_id, _) in &duplicates {
        doc.objects.remove(dup_id);
    }
}

/// Maximum nesting depth for intra-object traversal (arrays/dicts within one object).
const MAX_OBJECT_DEPTH: usize = 100;

fn replace_references(obj: &mut Object, map: &HashMap<ObjectId, ObjectId>, depth: usize) {
    if depth > MAX_OBJECT_DEPTH {
        return;
    }
    match obj {
        Object::Reference(id) => {
            if let Some(&new_id) = map.get(id) {
                *id = new_id;
            }
        }
        Object::Array(arr) => {
            for item in arr.iter_mut() {
                replace_references(item, map, depth + 1);
            }
        }
        Object::Dictionary(dict) => {
            replace_refs_in_dict(dict, map, depth + 1);
        }
        Object::Stream(stream) => {
            replace_refs_in_dict(&mut stream.dict, map, depth + 1);
        }
        _ => {}
    }
}

fn replace_refs_in_dict(dict: &mut lopdf::Dictionary, map: &HashMap<ObjectId, ObjectId>, depth: usize) {
    for (_, value) in dict.iter_mut() {
        replace_references(value, map, depth);
    }
}

// ---------------------------------------------------------------------------
// Phase 3: Remove unreferenced objects
// ---------------------------------------------------------------------------

fn remove_unused_objects(doc: &mut Document) {
    let referenced = collect_referenced_ids(doc);
    let all_ids: Vec<ObjectId> = doc.objects.keys().copied().collect();

    for id in all_ids {
        if !referenced.contains(&id) {
            doc.objects.remove(&id);
        }
    }
}

fn collect_referenced_ids(doc: &Document) -> HashSet<ObjectId> {
    let mut visited = HashSet::new();
    let mut stack: Vec<ObjectId> = Vec::new();

    // Seed from trailer references.
    if let Ok(root) = doc.trailer.get(b"Root")
        && let Ok(id) = root.as_reference()
    {
        stack.push(id);
    }
    if let Ok(info) = doc.trailer.get(b"Info")
        && let Ok(id) = info.as_reference()
    {
        stack.push(id);
    }

    // Iterative traversal — avoids stack overflow on deep object chains.
    while let Some(id) = stack.pop() {
        if !visited.insert(id) {
            continue;
        }
        if let Some(obj) = doc.objects.get(&id) {
            collect_refs_into(obj, &mut stack, 0);
        }
    }

    visited
}

/// Collect all object-id references from a single PDF object.
/// Depth-limited to prevent stack overflow from crafted deeply-nested objects.
fn collect_refs_into(obj: &Object, stack: &mut Vec<ObjectId>, depth: usize) {
    if depth > MAX_OBJECT_DEPTH {
        return;
    }
    match obj {
        Object::Reference(id) => stack.push(*id),
        Object::Array(arr) => {
            for item in arr {
                collect_refs_into(item, stack, depth + 1);
            }
        }
        Object::Dictionary(dict) => {
            for (_, value) in dict {
                collect_refs_into(value, stack, depth + 1);
            }
        }
        Object::Stream(stream) => {
            for (_, value) in &stream.dict {
                collect_refs_into(value, stack, depth + 1);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Phase 4: Strip non-essential metadata
// ---------------------------------------------------------------------------

fn strip_metadata(doc: &mut Document) {
    // Remove /Info dictionary (author, title, creation date, etc.).
    if let Ok(Object::Reference(info_id)) = doc.trailer.get(b"Info") {
        let info_id = *info_id;
        doc.objects.remove(&info_id);
        doc.trailer.remove(b"Info");
    }

    // Remove XMP metadata from catalog.
    let catalog_id = doc
        .trailer
        .get(b"Root")
        .ok()
        .and_then(|r| r.as_reference().ok());

    if let Some(cat_id) = catalog_id {
        let xmp_ref = doc
            .objects
            .get(&cat_id)
            .and_then(|obj| match obj {
                Object::Dictionary(dict) => dict.get(b"Metadata").ok(),
                _ => None,
            })
            .and_then(|m| m.as_reference().ok());

        if let Some(meta_id) = xmp_ref {
            doc.objects.remove(&meta_id);
        }
        if let Some(Object::Dictionary(dict)) = doc.objects.get_mut(&cat_id) {
            dict.remove(b"Metadata");
        }
    }

    // Remove PieceInfo / LastModified / Metadata from page objects.
    let page_ids: Vec<_> = doc.get_pages().values().copied().collect();
    for page_id in page_ids {
        if let Some(Object::Dictionary(dict)) = doc.objects.get_mut(&page_id) {
            dict.remove(b"PieceInfo");
            dict.remove(b"LastModified");
            dict.remove(b"Metadata");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::Stream;

    fn make_image_stream(filter: &[u8], colorspace: &[u8], w: i64, h: i64) -> Object {
        let mut dict = lopdf::Dictionary::new();
        dict.set("Subtype", Object::Name(b"Image".to_vec()));
        dict.set("Filter", Object::Name(filter.to_vec()));
        dict.set("Width", Object::Integer(w));
        dict.set("Height", Object::Integer(h));
        dict.set("BitsPerComponent", Object::Integer(8));
        dict.set("ColorSpace", Object::Name(colorspace.to_vec()));
        Object::Stream(Stream::new(dict, Vec::new()))
    }

    #[test]
    fn test_classify_dct() {
        let obj = make_image_stream(b"DCTDecode", b"DeviceRGB", 100, 200);
        let info = classify_image_xobject(&obj).expect("should classify");
        assert!(matches!(info.filter, ImageFilter::Dct));
        assert_eq!(info.width, 100);
        assert_eq!(info.height, 200);
        assert_eq!(info.components, 3);
    }

    #[test]
    fn test_classify_flat() {
        let obj = make_image_stream(b"FlateDecode", b"DeviceGray", 50, 50);
        let info = classify_image_xobject(&obj).expect("should classify");
        assert!(matches!(info.filter, ImageFilter::Flat));
        assert_eq!(info.components, 1);
    }

    #[test]
    fn test_classify_non_image() {
        let mut dict = lopdf::Dictionary::new();
        dict.set("Subtype", Object::Name(b"Form".to_vec()));
        dict.set("Filter", Object::Name(b"FlateDecode".to_vec()));
        let obj = Object::Stream(Stream::new(dict, Vec::new()));
        assert!(classify_image_xobject(&obj).is_none());
    }

    #[test]
    fn test_classify_cmyk() {
        let obj = make_image_stream(b"DCTDecode", b"DeviceCMYK", 100, 100);
        let info = classify_image_xobject(&obj).expect("should classify");
        assert_eq!(info.components, 4);
    }
}
