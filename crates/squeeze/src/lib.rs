pub mod archive;
pub mod config;
pub mod detect;
pub mod error;
pub mod estimate;
pub mod image;
pub mod pdf;
pub mod svg;

use archive::ArchiveKind;
use config::Config;
use detect::Format;
use error::SqueezeError;

/// Result of a compression operation.
#[derive(Debug)]
pub struct CompressResult {
    /// Size of the original input in bytes.
    pub original_size: usize,
    /// Size of the compressed output in bytes (equals `original_size` if unchanged).
    pub compressed_size: usize,
    /// The compressed output, or `Unchanged` if compression was not worthwhile.
    pub output: CompressOutput,
    /// New MIME type if the format changed (e.g. BMP -> JPEG).
    pub new_mime_type: Option<String>,
}

impl CompressResult {
    /// Returns true if the input was actually compressed.
    #[must_use]
    pub fn was_compressed(&self) -> bool {
        matches!(self.output, CompressOutput::Compressed(_))
    }

    /// Savings as a percentage (0.0-100.0).
    #[must_use]
    pub fn savings_pct(&self) -> f32 {
        if self.original_size == 0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        let pct =
            ((1.0 - (self.compressed_size as f64 / self.original_size as f64)) * 100.0) as f32;
        pct
    }

    /// Consume the result and return the output bytes.
    /// Returns the original input if compression was not applied.
    #[must_use]
    pub fn into_bytes(self, original: &[u8]) -> Vec<u8> {
        match self.output {
            CompressOutput::Compressed(data) => data,
            CompressOutput::Unchanged => original.to_vec(),
        }
    }
}

/// The output of a compression operation.
#[derive(Debug)]
pub enum CompressOutput {
    /// The input was not modified (unsupported format, already small, or savings below threshold).
    Unchanged,
    /// The compressed bytes.
    Compressed(Vec<u8>),
}

/// Compress an attachment.
///
/// # Arguments
/// * `input` - Raw attachment bytes
/// * `mime_type` - MIME type string (e.g. "image/jpeg", "application/pdf")
/// * `config` - Compression configuration
///
/// # Returns
/// A `CompressResult` describing what happened. If the format is unsupported
/// or compression would not save enough space, returns `CompressOutput::Unchanged`.
pub fn compress(
    input: &[u8],
    mime_type: &str,
    config: &Config,
) -> Result<CompressResult, SqueezeError> {
    let format = detect::detect(mime_type, input);

    log::info!(
        "Compression start: mime={}, format={:?}, original_size={} bytes",
        mime_type,
        format,
        input.len()
    );

    if format == Format::Unsupported {
        log::warn!("Unsupported format for compression: mime={mime_type}");
    }

    let unchanged = || {
        Ok(CompressResult {
            original_size: input.len(),
            compressed_size: input.len(),
            output: CompressOutput::Unchanged,
            new_mime_type: None,
        })
    };

    let result = match format {
        Format::Jpeg | Format::Png | Format::WebP | Format::Gif | Format::Bmp | Format::Tiff => {
            log::debug!("Using image compression strategy for {format:?}");
            image::compress_image(input, format, config)
        }
        Format::Heic => {
            // HEIC requires the `heic` feature (C dependency).
            // Without it, pass through unchanged.
            #[cfg(feature = "heic")]
            {
                image::compress_image(input, format, config)
            }
            #[cfg(not(feature = "heic"))]
            {
                unchanged()
            }
        }
        Format::Pdf => {
            log::debug!("Using PDF compression strategy");
            pdf::compress_pdf(input, config)
        }
        Format::Ooxml(kind) => {
            let archive_kind = match kind {
                detect::OoxmlKind::Docx => ArchiveKind::Docx,
                detect::OoxmlKind::Xlsx => ArchiveKind::Xlsx,
                detect::OoxmlKind::Pptx => ArchiveKind::Pptx,
            };
            log::debug!("Using OOXML archive compression strategy for {kind:?}");
            archive::compress_archive(input, archive_kind, config)
        }
        Format::Odf(kind) => {
            let archive_kind = match kind {
                detect::OdfKind::Odt => ArchiveKind::Odt,
                detect::OdfKind::Ods => ArchiveKind::Ods,
                detect::OdfKind::Odp => ArchiveKind::Odp,
            };
            log::debug!("Using ODF archive compression strategy for {kind:?}");
            archive::compress_archive(input, archive_kind, config)
        }
        Format::Svg => {
            log::debug!("Using SVG compression strategy");
            svg::compress_svg(input, config.min_savings_pct)
        }
        Format::Unsupported => unchanged(),
    };

    match &result {
        Ok(r) => {
            if r.was_compressed() {
                log::info!(
                    "Compression complete: {} -> {} bytes ({:.1}% savings)",
                    r.original_size,
                    r.compressed_size,
                    r.savings_pct()
                );
            } else {
                log::info!(
                    "Compression skipped (insufficient savings): {} bytes unchanged",
                    r.original_size
                );
            }
        }
        Err(e) => {
            log::error!("Compression failed: {e}");
        }
    }

    result
}
