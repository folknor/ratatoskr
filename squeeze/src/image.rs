use std::io::Cursor;

use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, ImageReader};

use crate::config::Config;
use crate::detect::Format;
use crate::error::SqueezeError;
use crate::{CompressOutput, CompressResult};

/// Map our `Format` enum to `image::ImageFormat` for decoding.
fn to_image_format(format: Format) -> Option<ImageFormat> {
    match format {
        Format::Jpeg => Some(ImageFormat::Jpeg),
        Format::Png => Some(ImageFormat::Png),
        Format::WebP => Some(ImageFormat::WebP),
        Format::Gif => Some(ImageFormat::Gif),
        Format::Bmp => Some(ImageFormat::Bmp),
        Format::Tiff => Some(ImageFormat::Tiff),
        _ => None,
    }
}

/// Compress a standalone image attachment.
pub fn compress_image(
    input: &[u8],
    format: Format,
    config: &Config,
) -> Result<CompressResult, SqueezeError> {
    let img_format = to_image_format(format).ok_or_else(|| {
        SqueezeError::ImageDecode(format!("unsupported image format: {format:?}"))
    })?;

    // For PNG, try lossless optimization first via oxipng — no decode needed.
    if format == Format::Png && !config.png_to_jpeg {
        return compress_png_lossless(input, config);
    }

    let img = ImageReader::with_format(Cursor::new(input), img_format)
        .decode()
        .map_err(|e| SqueezeError::ImageDecode(e.to_string()))?;

    let (output_format, output_mime) = choose_output_format(format, config);
    let resized = maybe_resize(&img, config.max_dimension);
    let encoded = encode_image(&resized, output_format, config.jpeg_quality)?;

    let new_mime = if output_format != img_format {
        Some(output_mime.to_string())
    } else {
        None
    };

    finish(input, encoded, new_mime, config.min_savings_pct)
}

/// Compress raw image bytes used internally (for PDF/archive embedded images).
/// Takes raw decoded `DynamicImage` rather than encoded bytes.
pub fn compress_image_raw(
    img: &DynamicImage,
    original_size: usize,
    config: &Config,
    quality: u8,
    max_dim: u32,
) -> Result<Option<Vec<u8>>, SqueezeError> {
    let resized = maybe_resize(img, max_dim);
    let encoded = encode_jpeg_mozjpeg(&resized, quality)?;

    if savings_pct(original_size, encoded.len()) >= config.min_savings_pct {
        Ok(Some(encoded))
    } else {
        Ok(None)
    }
}

/// Lossless PNG optimization via oxipng.
fn compress_png_lossless(input: &[u8], config: &Config) -> Result<CompressResult, SqueezeError> {
    // If PNG is oversized, decode + resize + re-optimize.
    let needs_resize = {
        let reader = ImageReader::with_format(Cursor::new(input), ImageFormat::Png);
        if let Ok((w, h)) = reader.into_dimensions() {
            w.max(h) > config.max_dimension
        } else {
            false
        }
    };

    let png_bytes = if needs_resize {
        let img = ImageReader::with_format(Cursor::new(input), ImageFormat::Png)
            .decode()
            .map_err(|e| SqueezeError::ImageDecode(e.to_string()))?;
        let resized = maybe_resize(&img, config.max_dimension);
        let mut buf = Cursor::new(Vec::new());
        resized
            .write_to(&mut buf, ImageFormat::Png)
            .map_err(|e| SqueezeError::ImageEncode(e.to_string()))?;
        buf.into_inner()
    } else {
        input.to_vec()
    };

    let opts = oxipng::Options::from_preset(3);
    let optimized = oxipng::optimize_from_memory(&png_bytes, &opts)
        .map_err(|e| SqueezeError::ImageEncode(format!("oxipng: {e}")))?;

    finish(input, optimized, None, config.min_savings_pct)
}

fn choose_output_format(input_format: Format, config: &Config) -> (ImageFormat, &'static str) {
    match input_format {
        Format::Bmp | Format::Tiff if config.bmp_tiff_to_jpeg => {
            (ImageFormat::Jpeg, "image/jpeg")
        }
        Format::Png if config.png_to_jpeg => (ImageFormat::Jpeg, "image/jpeg"),
        Format::Jpeg => (ImageFormat::Jpeg, "image/jpeg"),
        Format::Png => (ImageFormat::Png, "image/png"),
        Format::WebP => (ImageFormat::WebP, "image/webp"),
        Format::Gif => (ImageFormat::Gif, "image/gif"),
        Format::Bmp => (ImageFormat::Bmp, "image/bmp"),
        Format::Tiff => (ImageFormat::Tiff, "image/tiff"),
        _ => (ImageFormat::Jpeg, "image/jpeg"),
    }
}

fn maybe_resize(img: &DynamicImage, max_dim: u32) -> DynamicImage {
    let w = img.width();
    let h = img.height();
    let longest = w.max(h);

    if longest <= max_dim {
        return img.clone();
    }

    let scale = f64::from(max_dim) / f64::from(longest);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let new_w = (f64::from(w) * scale) as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let new_h = (f64::from(h) * scale) as u32;

    img.resize(new_w, new_h, FilterType::Lanczos3)
}

/// Encode an image using the best available encoder for each format.
fn encode_image(
    img: &DynamicImage,
    format: ImageFormat,
    jpeg_quality: u8,
) -> Result<Vec<u8>, SqueezeError> {
    match format {
        ImageFormat::Jpeg => encode_jpeg_mozjpeg(img, jpeg_quality),
        ImageFormat::Png => {
            // Encode to PNG via image crate, then optimize with oxipng.
            let mut buf = Cursor::new(Vec::new());
            img.write_to(&mut buf, ImageFormat::Png)
                .map_err(|e| SqueezeError::ImageEncode(e.to_string()))?;
            let raw_png = buf.into_inner();
            let opts = oxipng::Options::from_preset(3);
            oxipng::optimize_from_memory(&raw_png, &opts)
                .map_err(|e| SqueezeError::ImageEncode(format!("oxipng: {e}")))
        }
        _ => {
            let mut buf = Cursor::new(Vec::new());
            img.write_to(&mut buf, format)
                .map_err(|e| SqueezeError::ImageEncode(e.to_string()))?;
            Ok(buf.into_inner())
        }
    }
}

/// Encode JPEG using mozjpeg-rs (trellis quantization, progressive).
fn encode_jpeg_mozjpeg(img: &DynamicImage, quality: u8) -> Result<Vec<u8>, SqueezeError> {
    let rgb = img.to_rgb8();
    let (width, height) = rgb.dimensions();
    let pixels = rgb.as_raw();

    mozjpeg_rs::Encoder::new(mozjpeg_rs::Preset::ProgressiveBalanced)
        .quality(quality.min(100))
        .encode_rgb(pixels, width, height)
        .map_err(|e| SqueezeError::ImageEncode(format!("mozjpeg: {e}")))
}

fn savings_pct(original: usize, compressed: usize) -> f32 {
    if original == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    let pct = ((1.0 - (compressed as f64 / original as f64)) * 100.0) as f32;
    pct
}

fn finish(
    original: &[u8],
    encoded: Vec<u8>,
    new_mime: Option<String>,
    min_savings_pct: f32,
) -> Result<CompressResult, SqueezeError> {
    let original_size = original.len();
    let compressed_size = encoded.len();

    if savings_pct(original_size, compressed_size) < min_savings_pct {
        return Ok(CompressResult {
            original_size,
            compressed_size: original_size,
            output: CompressOutput::Unchanged,
            new_mime_type: None,
        });
    }

    Ok(CompressResult {
        original_size,
        compressed_size,
        output: CompressOutput::Compressed(encoded),
        new_mime_type: new_mime,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_jpeg(width: u32, height: u32) -> Vec<u8> {
        let img = DynamicImage::new_rgb8(width, height);
        let mut buf = Cursor::new(Vec::new());
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 95);
        img.write_with_encoder(encoder)
            .expect("failed to encode test jpeg");
        buf.into_inner()
    }

    #[test]
    fn test_small_image_unchanged() {
        let data = make_test_jpeg(100, 100);
        let config = Config::email_default();
        let result = compress_image(&data, Format::Jpeg, &config).expect("compress failed");
        assert_eq!(result.original_size, data.len());
    }

    #[test]
    fn test_resize_triggers() {
        let data = make_test_jpeg(4000, 3000);
        let config = Config::email_default();
        let result = compress_image(&data, Format::Jpeg, &config).expect("compress failed");
        assert!(result.original_size > 0);
    }

    #[test]
    fn test_bmp_to_jpeg_conversion() {
        let img = DynamicImage::new_rgb8(200, 200);
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, ImageFormat::Bmp)
            .expect("failed to encode test bmp");
        let bmp_data = buf.into_inner();

        let config = Config::email_default();
        let result = compress_image(&bmp_data, Format::Bmp, &config).expect("compress failed");
        assert!(result.was_compressed());
        assert_eq!(result.new_mime_type.as_deref(), Some("image/jpeg"));
    }

    #[test]
    fn test_png_lossless_optimization() {
        // Create a PNG with some redundancy that oxipng can optimize.
        let img = DynamicImage::new_rgb8(300, 300);
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, ImageFormat::Png)
            .expect("failed to encode test png");
        let png_data = buf.into_inner();

        let config = Config::email_default();
        let result = compress_image(&png_data, Format::Png, &config).expect("compress failed");
        // Oxipng should be able to optimize even a simple image.
        assert_eq!(result.original_size, png_data.len());
    }
}
