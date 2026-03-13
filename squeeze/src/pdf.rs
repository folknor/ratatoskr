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

/// Compress images embedded inside a PDF document.
///
/// Handles DCTDecode (JPEG) and FlateDecode (raw pixel) image XObjects.
/// FlateDecode images are decoded from raw pixels, re-encoded as JPEG, and
/// the stream filter is updated to DCTDecode.
pub fn compress_pdf(input: &[u8], config: &Config) -> Result<CompressResult, SqueezeError> {
    let mut doc =
        Document::load_mem(input).map_err(|e| SqueezeError::PdfParse(e.to_string()))?;

    // Collect object IDs first to avoid borrow issues.
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
            ImageFilter::Dct => try_recompress_dct(&mut doc, id, &info, config),
            ImageFilter::Flat => try_recompress_flat(&mut doc, id, &info, config),
        }
    }

    // Write the PDF using modern format (PDF 1.5 object streams + xref
    // streams) for additional 11-38% structural compression — even if no
    // images were recompressed, the structural savings alone are worthwhile.
    let mut output = Vec::new();
    doc.save_modern(&mut output)
        .map_err(|e| SqueezeError::PdfWrite(e.to_string()))?;

    if output.len() < input.len() {
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

/// Classify a PDF object: is it an image XObject, and what filter does it use?
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
            _ => return None, // Indexed, ICCBased, etc. — skip for now.
        },
        // Some PDFs use an array for ColorSpace (e.g. [/ICCBased ref]).
        // Skip those for now.
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
        Object::Integer(n) => {
            let val = *n;
            if val > 0 {
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                Some(val as u32)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Re-compress a DCTDecode (JPEG) image stream.
fn try_recompress_dct(doc: &mut Document, id: ObjectId, info: &ImageInfo, config: &Config) {
    // Skip CMYK — image crate can't handle it.
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

/// Decompress a FlateDecode image, reconstruct pixels, and re-encode as JPEG.
fn try_recompress_flat(doc: &mut Document, id: ObjectId, info: &ImageInfo, config: &Config) {
    // Skip CMYK — image crate can't handle it.
    if info.components == 4 {
        return;
    }
    // Only handle 8-bit images.
    if info.bpc != 8 {
        return;
    }

    // Decompress the FlateDecode stream.
    let raw_pixels = {
        let Some(Object::Stream(stream)) = doc.objects.get_mut(&id) else {
            return;
        };
        // lopdf's decompress() inflates in-place and clears the Filter.
        if stream.decompress().is_err() {
            return;
        }
        stream.content.clone()
    };

    let expected_len = (info.width * info.height * info.components) as usize;
    // Some PDFs add stride padding; accept if we have at least enough data.
    if raw_pixels.len() < expected_len {
        // Can't reconstruct — restore original by re-reading.
        // Since we already decompressed in-place, we need to re-compress.
        recompress_flat_stream(doc, id);
        return;
    }

    // Reconstruct a DynamicImage from raw pixel data.
    let img: Option<DynamicImage> = match info.components {
        3 => image::RgbImage::from_raw(info.width, info.height, raw_pixels[..expected_len].to_vec())
            .map(DynamicImage::ImageRgb8),
        1 => {
            image::GrayImage::from_raw(info.width, info.height, raw_pixels[..expected_len].to_vec())
                .map(DynamicImage::ImageLuma8)
        }
        _ => None,
    };

    let Some(img) = img else {
        recompress_flat_stream(doc, id);
        return;
    };

    // Encode as JPEG via mozjpeg.
    let jpeg_result = compress_image_raw(
        &img,
        expected_len, // Compare against uncompressed size.
        config,
        config.pdf_image_quality,
        config.pdf_image_max_dim,
    );

    match jpeg_result {
        Ok(Some(jpeg_bytes)) => {
            if let Some(Object::Stream(stream)) = doc.objects.get_mut(&id) {
                stream.set_content(jpeg_bytes);
                // Update filter from FlateDecode to DCTDecode.
                stream
                    .dict
                    .set("Filter", Object::Name(b"DCTDecode".to_vec()));
                // Remove DecodeParms if present (not applicable to DCTDecode).
                stream.dict.remove(b"DecodeParms");
            }
        }
        _ => {
            // Re-compress as FlateDecode since we decompressed in-place.
            recompress_flat_stream(doc, id);
        }
    }
}

/// Re-compress a stream that was decompressed in-place back to FlateDecode.
fn recompress_flat_stream(doc: &mut Document, id: ObjectId) {
    if let Some(Object::Stream(stream)) = doc.objects.get_mut(&id) {
        let _ = stream.compress();
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
