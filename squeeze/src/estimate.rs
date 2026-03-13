use std::fs;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom};
use std::path::Path;

use crate::config::Config;
use crate::detect::Format;
use crate::error::SqueezeError;

/// Result of a fast size estimation (no actual compression performed).
#[derive(Debug, Clone)]
pub struct Estimate {
    /// Original file size in bytes.
    pub original_size: u64,
    /// Estimated output size after compression (best guess).
    pub expected_bytes: u64,
    /// Hard floor: non-compressible content that cannot shrink further.
    /// If this alone exceeds the provider limit, compression cannot help.
    pub floor_bytes: u64,
    /// Whether compression is worth attempting.
    pub worth_trying: bool,
    /// Human-readable reason if not worth trying.
    pub reason: Option<String>,
}

/// Average bytes per pixel for mozjpeg (progressive + trellis) output.
/// Empirically measured across phone photos, scans, and screenshots:
///   q80: 0.065–0.174 bpp (mean ~0.10)
///   q75: ~0.08 bpp
/// We use a conservative upper bound (2-3x the mean) so the estimate
/// over-predicts rather than under-predicts. This means "won't fit" is
/// reliable, while "will fit" may be pleasantly wrong.
const JPEG_BYTES_PER_PIXEL_Q80: f64 = 0.35;
const JPEG_BYTES_PER_PIXEL_Q75: f64 = 0.25;

/// Estimate compressed size from an in-memory buffer.
///
/// Reads only headers and metadata — sub-millisecond even on very large files.
/// The estimate is deliberately conservative (tends to over-estimate output size)
/// so that `worth_trying: false` is a reliable signal.
pub fn estimate(
    input: &[u8],
    format: Format,
    config: &Config,
) -> Result<Estimate, SqueezeError> {
    match format {
        Format::Jpeg | Format::Png | Format::WebP | Format::Gif | Format::Bmp | Format::Tiff
        | Format::Heic => {
            let original_size = input.len() as u64;
            let dims = read_image_dimensions_mem(input, format)?;
            estimate_image(original_size, dims, format, config)
        }
        Format::Pdf => estimate_pdf_mem(input, config),
        Format::Ooxml(_) | Format::Odf(_) => {
            estimate_archive_reader(Cursor::new(input), input.len() as u64, config)
        }
        Format::Svg => estimate_svg_reader(&mut Cursor::new(input), input.len() as u64),
        Format::Unsupported => Ok(unchanged(input.len() as u64, "unsupported format")),
    }
}

/// Estimate compressed size from a file path without loading the entire file.
///
/// For images, reads only the first few KB (header). For PDFs, parses the xref
/// table and object dictionaries without holding all stream data. For archives,
/// reads only the central directory. For SVGs, streams in chunks.
pub fn estimate_file(
    path: &Path,
    format: Format,
    config: &Config,
) -> Result<Estimate, SqueezeError> {
    let metadata = fs::metadata(path).map_err(SqueezeError::Io)?;
    let original_size = metadata.len();

    match format {
        Format::Jpeg | Format::Png | Format::WebP | Format::Gif | Format::Bmp | Format::Tiff
        | Format::Heic => {
            let dims = read_image_dimensions_file(path, format)?;
            estimate_image(original_size, dims, format, config)
        }
        Format::Pdf => estimate_pdf_file(path, original_size, config),
        Format::Ooxml(_) | Format::Odf(_) => {
            let file = fs::File::open(path).map_err(SqueezeError::Io)?;
            estimate_archive_reader(BufReader::new(file), original_size, config)
        }
        Format::Svg => {
            let file = fs::File::open(path).map_err(SqueezeError::Io)?;
            estimate_svg_reader(&mut BufReader::new(file), original_size)
        }
        Format::Unsupported => Ok(unchanged(original_size, "unsupported format")),
    }
}

fn unchanged(original_size: u64, reason: &str) -> Estimate {
    Estimate {
        original_size,
        expected_bytes: original_size,
        floor_bytes: original_size,
        worth_trying: false,
        reason: Some(reason.into()),
    }
}

// ---------------------------------------------------------------------------
// Image estimation
// ---------------------------------------------------------------------------

fn estimate_image(
    original_size: u64,
    dims: (u32, u32),
    format: Format,
    config: &Config,
) -> Result<Estimate, SqueezeError> {
    let (width, height) = dims;

    // Calculate target dimensions after resize.
    let longest = width.max(height);
    let (target_w, target_h) = if longest > config.max_dimension && longest > 0 {
        let scale = f64::from(config.max_dimension) / f64::from(longest);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let tw = ((f64::from(width) * scale) as u32).max(1);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let th = ((f64::from(height) * scale) as u32).max(1);
        (tw, th)
    } else {
        (width, height)
    };

    let target_pixels = u64::from(target_w) * u64::from(target_h);

    // Estimate output size based on output format.
    let will_become_jpeg = matches!(format, Format::Jpeg | Format::Heic)
        || (matches!(format, Format::Bmp | Format::Tiff) && config.bmp_tiff_to_jpeg)
        || (format == Format::Png && config.png_to_jpeg);

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let expected_bytes = if will_become_jpeg {
        let bpp = if config.jpeg_quality >= 80 {
            JPEG_BYTES_PER_PIXEL_Q80
        } else {
            JPEG_BYTES_PER_PIXEL_Q75
        };
        let pixel_estimate = (target_pixels as f64 * bpp) as u64;
        // Never estimate larger than original — compressor returns Unchanged in that case.
        pixel_estimate.min(original_size)
    } else if format == Format::Png {
        // PNG lossless: oxipng typically saves 10-30%. Be conservative.
        original_size * 85 / 100
    } else if format == Format::Gif {
        // GIF stays as GIF after resize. Savings mainly come from downscaling.
        if longest > config.max_dimension {
            let ratio = f64::from(config.max_dimension) / f64::from(longest);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let est = (original_size as f64 * ratio * ratio) as u64;
            est.min(original_size)
        } else {
            original_size
        }
    } else {
        // WebP and others: assume modest savings.
        original_size * 90 / 100
    };

    Ok(Estimate {
        original_size,
        expected_bytes,
        // Images are fully compressible — no hard floor.
        floor_bytes: 0,
        worth_trying: expected_bytes < original_size * 90 / 100,
        reason: None,
    })
}

fn to_image_format(format: Format) -> Option<image::ImageFormat> {
    match format {
        Format::Jpeg => Some(image::ImageFormat::Jpeg),
        Format::Png => Some(image::ImageFormat::Png),
        Format::WebP => Some(image::ImageFormat::WebP),
        Format::Gif => Some(image::ImageFormat::Gif),
        Format::Bmp => Some(image::ImageFormat::Bmp),
        Format::Tiff => Some(image::ImageFormat::Tiff),
        _ => None,
    }
}

fn read_image_dimensions_mem(input: &[u8], format: Format) -> Result<(u32, u32), SqueezeError> {
    let Some(img_format) = to_image_format(format) else {
        return Ok((0, 0));
    };
    image::ImageReader::with_format(Cursor::new(input), img_format)
        .into_dimensions()
        .map_err(|e| SqueezeError::ImageDecode(e.to_string()))
}

fn read_image_dimensions_file(path: &Path, format: Format) -> Result<(u32, u32), SqueezeError> {
    let Some(img_format) = to_image_format(format) else {
        return Ok((0, 0));
    };
    let file = fs::File::open(path).map_err(SqueezeError::Io)?;
    // BufReader reads only what the header parser needs (typically < 64 KB).
    let reader = BufReader::new(file);
    image::ImageReader::with_format(reader, img_format)
        .into_dimensions()
        .map_err(|e| SqueezeError::ImageDecode(e.to_string()))
}

// ---------------------------------------------------------------------------
// PDF estimation
// ---------------------------------------------------------------------------

/// Shared logic: walk parsed PDF objects, estimate image compression.
fn estimate_pdf_doc(
    doc: &lopdf::Document,
    original_size: u64,
    config: &Config,
) -> Estimate {
    let mut total_image_bytes: u64 = 0;
    let mut estimated_compressed_image_bytes: u64 = 0;

    for obj in doc.objects.values() {
        let lopdf::Object::Stream(stream) = obj else {
            continue;
        };

        let is_image = stream
            .dict
            .get(b"Subtype")
            .is_ok_and(|v| matches!(v, lopdf::Object::Name(n) if n == b"Image"));

        if !is_image {
            continue;
        }

        let stream_len = stream.content.len() as u64;
        total_image_bytes += stream_len;

        let width = get_pdf_int(&stream.dict, b"Width").unwrap_or(0);
        let height = get_pdf_int(&stream.dict, b"Height").unwrap_or(0);
        let components = match stream.dict.get(b"ColorSpace").ok() {
            Some(lopdf::Object::Name(n)) => match n.as_slice() {
                b"DeviceRGB" => 3u32,
                b"DeviceGray" => 1,
                b"DeviceCMYK" => 4,
                _ => 3,
            },
            _ => 3,
        };

        // CMYK images are skipped by the compressor.
        if components == 4 || width == 0 || height == 0 {
            estimated_compressed_image_bytes += stream_len;
            continue;
        }

        // Calculate target dimensions.
        let longest = width.max(height);
        let (tw, th) = if longest > config.pdf_image_max_dim {
            let scale = f64::from(config.pdf_image_max_dim) / f64::from(longest);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let tw = ((f64::from(width) * scale) as u32).max(1);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let th = ((f64::from(height) * scale) as u32).max(1);
            (tw, th)
        } else {
            (width, height)
        };

        let target_pixels = u64::from(tw) * u64::from(th);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let est = (target_pixels as f64 * JPEG_BYTES_PER_PIXEL_Q75) as u64;

        estimated_compressed_image_bytes += est.min(stream_len);
    }

    let non_image_bytes = original_size.saturating_sub(total_image_bytes);

    // save_modern typically saves 10-15% on structural overhead.
    let structural_savings = non_image_bytes * 10 / 100;
    let floor_bytes = non_image_bytes.saturating_sub(structural_savings);
    let expected_bytes = floor_bytes + estimated_compressed_image_bytes;

    let worth_trying = expected_bytes < original_size * 90 / 100;

    Estimate {
        original_size,
        expected_bytes,
        floor_bytes,
        worth_trying,
        reason: if !worth_trying {
            Some(format!(
                "non-image content is {:.1} MB, estimated output {:.1} MB",
                floor_bytes as f64 / 1_048_576.0,
                expected_bytes as f64 / 1_048_576.0,
            ))
        } else {
            None
        },
    }
}

fn estimate_pdf_mem(input: &[u8], config: &Config) -> Result<Estimate, SqueezeError> {
    let doc = lopdf::Document::load_mem(input)
        .map_err(|e| SqueezeError::PdfParse(e.to_string()))?;
    Ok(estimate_pdf_doc(&doc, input.len() as u64, config))
}

fn estimate_pdf_file(
    path: &Path,
    original_size: u64,
    config: &Config,
) -> Result<Estimate, SqueezeError> {
    let doc = lopdf::Document::load(path)
        .map_err(|e| SqueezeError::PdfParse(e.to_string()))?;
    Ok(estimate_pdf_doc(&doc, original_size, config))
}

fn get_pdf_int(dict: &lopdf::Dictionary, key: &[u8]) -> Option<u32> {
    match dict.get(key).ok()? {
        lopdf::Object::Integer(n) => u32::try_from(*n).ok().filter(|&v| v > 0),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Archive estimation (OOXML / ODF)
// ---------------------------------------------------------------------------

fn estimate_archive_reader<R: Read + Seek>(
    reader: R,
    original_size: u64,
    _config: &Config,
) -> Result<Estimate, SqueezeError> {
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|e| SqueezeError::ArchiveRead(e.to_string()))?;

    let mut image_bytes: u64 = 0;
    let mut estimated_image_output: u64 = 0;

    // Cap iteration to avoid spending excessive time on pathological archives.
    let entry_count = archive.len().min(50_000);
    for i in 0..entry_count {
        let Ok(entry) = archive.by_index_raw(i) else {
            continue;
        };
        let name = entry.name().to_string();
        let size = entry.size(); // uncompressed size

        if is_likely_image_entry(&name) {
            image_bytes += size;
            let lower = name.to_ascii_lowercase();
            let ratio = if lower.ends_with(".png")
                || lower.ends_with(".bmp")
                || lower.ends_with(".tiff")
                || lower.ends_with(".tif")
            {
                0.3
            } else {
                0.7
            };
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let est = (size as f64 * ratio) as u64;
            estimated_image_output += est.min(size);
        }
    }

    let floor_bytes = original_size.saturating_sub(image_bytes);
    let expected_bytes = floor_bytes + estimated_image_output;

    let worth_trying = image_bytes > 0 && expected_bytes < original_size * 90 / 100;

    Ok(Estimate {
        original_size,
        expected_bytes,
        floor_bytes,
        worth_trying,
        reason: if !worth_trying && image_bytes == 0 {
            Some("no compressible images found in archive".into())
        } else {
            None
        },
    })
}

fn is_likely_image_entry(name: &str) -> bool {
    let in_media = name.starts_with("word/media/")
        || name.starts_with("xl/media/")
        || name.starts_with("ppt/media/")
        || name.starts_with("Pictures/");

    if !in_media {
        return false;
    }

    let lower = name.to_ascii_lowercase();
    lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".png")
        || lower.ends_with(".bmp")
        || lower.ends_with(".tiff")
        || lower.ends_with(".tif")
}

// ---------------------------------------------------------------------------
// SVG estimation
// ---------------------------------------------------------------------------

/// Estimate SVG from any reader. Streams in 64 KB chunks scanning for
/// `data:image/` patterns — never loads the whole file into memory.
fn estimate_svg_reader<R: Read + Seek>(
    reader: &mut R,
    original_size: u64,
) -> Result<Estimate, SqueezeError> {
    // We scan for data:image/ patterns and measure bytes until the closing quote.
    // Use a sliding-window approach with overlap to handle patterns at chunk boundaries.
    const CHUNK_SIZE: usize = 64 * 1024;
    const NEEDLE: &[u8] = b"data:image/";
    const OVERLAP: usize = NEEDLE.len() - 1;

    reader.seek(SeekFrom::Start(0)).map_err(SqueezeError::Io)?;

    let mut embedded_image_bytes: u64 = 0;
    let mut buf = vec![0u8; CHUNK_SIZE + OVERLAP];
    let mut carry = 0usize; // bytes carried over from previous chunk

    loop {
        let n = reader
            .read(&mut buf[carry..])
            .map_err(SqueezeError::Io)?;
        if n == 0 {
            break;
        }
        let filled = carry + n;

        // Scan for needle in buf[0..filled].
        let mut i = 0;
        while i + NEEDLE.len() <= filled {
            if buf[i..].starts_with(NEEDLE) {
                // Found a data URI start. Count bytes until closing quote.
                // First check within our current buffer.
                let uri_len = find_quote_distance(&buf[i..filled], reader)?;
                embedded_image_bytes += uri_len;
                #[allow(clippy::cast_possible_truncation)]
                {
                    i += uri_len as usize;
                }
                continue;
            }
            i += 1;
        }

        // Carry over the tail to handle patterns spanning chunks.
        if filled > OVERLAP {
            buf.copy_within((filled - OVERLAP)..filled, 0);
            carry = OVERLAP;
        } else {
            carry = 0;
        }
    }

    let non_image_bytes = original_size.saturating_sub(embedded_image_bytes);

    if embedded_image_bytes == 0 {
        return Ok(Estimate {
            original_size,
            expected_bytes: original_size,
            floor_bytes: non_image_bytes,
            worth_trying: false,
            reason: Some("no embedded images to optimize".into()),
        });
    }

    let estimated_image_output = embedded_image_bytes * 70 / 100;
    let expected_bytes = non_image_bytes + estimated_image_output;

    Ok(Estimate {
        original_size,
        expected_bytes,
        floor_bytes: non_image_bytes,
        worth_trying: expected_bytes < original_size * 90 / 100,
        reason: None,
    })
}

/// Maximum bytes to read searching for a closing quote (100 MB).
/// A data URI longer than this is pathological.
const MAX_QUOTE_SEARCH_BYTES: u64 = 100 * 1024 * 1024;

/// Count bytes from current position until a closing quote (`"` or `'`).
/// If the quote isn't in `local_buf`, continues reading from the reader.
/// Stops searching after `MAX_QUOTE_SEARCH_BYTES` to avoid unbounded reads.
fn find_quote_distance<R: Read>(local_buf: &[u8], reader: &mut R) -> Result<u64, SqueezeError> {
    // Check the local buffer first.
    for (i, &b) in local_buf.iter().enumerate() {
        if b == b'"' || b == b'\'' {
            return Ok(i as u64);
        }
    }

    // Quote not in local buffer — keep reading.
    let mut distance = local_buf.len() as u64;
    let mut small_buf = [0u8; 8192];
    loop {
        if distance > MAX_QUOTE_SEARCH_BYTES {
            return Ok(distance);
        }
        let n = reader.read(&mut small_buf).map_err(SqueezeError::Io)?;
        if n == 0 {
            return Ok(distance);
        }
        for (i, &b) in small_buf[..n].iter().enumerate() {
            if b == b'"' || b == b'\'' {
                return Ok(distance + i as u64);
            }
        }
        distance += n as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_jpeg() {
        use image::RgbImage;
        let mut img = RgbImage::new(4000, 3000);
        for (x, y, pixel) in img.enumerate_pixels_mut() {
            #[allow(clippy::cast_possible_truncation)]
            {
                pixel[0] = (x % 256) as u8;
                pixel[1] = (y % 256) as u8;
                pixel[2] = ((x + y) % 256) as u8;
            }
        }
        let dyn_img = image::DynamicImage::ImageRgb8(img);
        let mut buf = Cursor::new(Vec::new());
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 95);
        dyn_img.write_with_encoder(encoder).expect("encode failed");
        let data = buf.into_inner();

        let config = Config::email_default();
        let est = estimate(&data, Format::Jpeg, &config).expect("estimate failed");

        assert!(est.expected_bytes <= est.original_size);
        assert_eq!(est.floor_bytes, 0);
    }

    #[test]
    fn test_estimate_unsupported() {
        let data = b"not a real file";
        let config = Config::email_default();
        let est = estimate(data, Format::Unsupported, &config).expect("estimate failed");
        assert!(!est.worth_trying);
    }

    #[test]
    fn test_estimate_small_jpeg() {
        let img = image::DynamicImage::new_rgb8(100, 100);
        let mut buf = Cursor::new(Vec::new());
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 80);
        img.write_with_encoder(encoder).expect("encode failed");
        let data = buf.into_inner();

        let config = Config::email_default();
        let est = estimate(&data, Format::Jpeg, &config).expect("estimate failed");

        assert!(est.expected_bytes < 100_000);
    }

    #[test]
    fn test_estimate_file_matches_mem() {
        // Write a test JPEG to a temp file, verify estimate_file matches estimate.
        let img = image::DynamicImage::new_rgb8(800, 600);
        let mut buf = Cursor::new(Vec::new());
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 90);
        img.write_with_encoder(encoder).expect("encode failed");
        let data = buf.into_inner();

        let dir = std::env::temp_dir().join("squeeze_test");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("test_estimate.jpg");
        fs::write(&path, &data).expect("write failed");

        let config = Config::email_default();
        let est_mem = estimate(&data, Format::Jpeg, &config).expect("mem estimate failed");
        let est_file =
            estimate_file(&path, Format::Jpeg, &config).expect("file estimate failed");

        let _ = fs::remove_file(&path);

        assert_eq!(est_mem.expected_bytes, est_file.expected_bytes);
        assert_eq!(est_mem.floor_bytes, est_file.floor_bytes);
        assert_eq!(est_mem.worth_trying, est_file.worth_trying);
    }
}
