use std::io::{Cursor, Read, Write};

use image::ImageReader;
use zip::ZipWriter;
use zip::read::ZipArchive;
use zip::write::FileOptions;

use crate::config::Config;
use crate::error::SqueezeError;
use crate::image::compress_image_raw;
use crate::{CompressOutput, CompressResult};

/// Maximum number of ZIP entries we'll process.
const MAX_ARCHIVE_ENTRIES: usize = 50_000;
/// Maximum decompressed size of a single ZIP entry (512 MB).
const MAX_ARCHIVE_ENTRY_BYTES: u64 = 512 * 1024 * 1024;
/// Maximum total decompressed bytes across all entries (2 GB).
const MAX_ARCHIVE_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Kind of ZIP-based document archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    /// Office Open XML
    Docx,
    Xlsx,
    Pptx,
    /// Open Document Format
    Odt,
    Ods,
    Odp,
}

impl ArchiveKind {
    /// Returns path prefixes where images live for this archive kind.
    fn image_prefixes(self) -> &'static [&'static str] {
        match self {
            Self::Docx => &["word/media/"],
            Self::Xlsx => &["xl/media/"],
            Self::Pptx => &["ppt/media/"],
            Self::Odt | Self::Ods | Self::Odp => &["Pictures/"],
        }
    }
}

/// Compress images inside a ZIP-based document (OOXML or ODF).
pub fn compress_archive(
    input: &[u8],
    kind: ArchiveKind,
    config: &Config,
) -> Result<CompressResult, SqueezeError> {
    let reader = Cursor::new(input);
    let mut archive =
        ZipArchive::new(reader).map_err(|e| SqueezeError::ArchiveRead(e.to_string()))?;

    let mut output_buf = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut output_buf);

    let mut any_compressed = false;
    let prefixes = kind.image_prefixes();

    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Ok(CompressResult {
            original_size: input.len(),
            compressed_size: input.len(),
            output: CompressOutput::Unchanged,
            new_mime_type: None,
        });
    }

    let mut total_decompressed: u64 = 0;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| SqueezeError::ArchiveRead(e.to_string()))?;

        let name = entry.name().to_string();
        let is_image_entry = is_compressible_image(&name, prefixes);

        // Guard against zip bombs: cap actual decompressed bytes via take().
        // We don't trust the declared size — attackers control that field.
        let mut content = Vec::new();
        entry
            .by_ref()
            .take(MAX_ARCHIVE_ENTRY_BYTES + 1)
            .read_to_end(&mut content)
            .map_err(|e| SqueezeError::ArchiveRead(e.to_string()))?;

        if content.len() as u64 > MAX_ARCHIVE_ENTRY_BYTES {
            return Ok(unchanged_result(input));
        }

        total_decompressed += content.len() as u64;
        if total_decompressed > MAX_ARCHIVE_TOTAL_BYTES {
            return Ok(unchanged_result(input));
        }

        let options = FileOptions::<()>::default().compression_method(entry.compression());

        if is_image_entry {
            match try_compress_archive_image(&content, config) {
                Ok(Some(compressed)) => {
                    writer
                        .start_file(&name, options)
                        .map_err(|e| SqueezeError::ArchiveWrite(e.to_string()))?;
                    writer
                        .write_all(&compressed)
                        .map_err(|e| SqueezeError::ArchiveWrite(e.to_string()))?;
                    any_compressed = true;
                    continue;
                }
                Ok(None) | Err(_) => {
                    // Not worth compressing or failed — write original.
                }
            }
        }

        // Copy entry as-is.
        writer
            .start_file(&name, options)
            .map_err(|e| SqueezeError::ArchiveWrite(e.to_string()))?;
        writer
            .write_all(&content)
            .map_err(|e| SqueezeError::ArchiveWrite(e.to_string()))?;
    }

    writer
        .finish()
        .map_err(|e| SqueezeError::ArchiveWrite(e.to_string()))?;

    let output = output_buf.into_inner();

    if !any_compressed || output.len() >= input.len() {
        return Ok(CompressResult {
            original_size: input.len(),
            compressed_size: input.len(),
            output: CompressOutput::Unchanged,
            new_mime_type: None,
        });
    }

    Ok(CompressResult {
        original_size: input.len(),
        compressed_size: output.len(),
        output: CompressOutput::Compressed(output),
        new_mime_type: None,
    })
}

fn unchanged_result(input: &[u8]) -> CompressResult {
    CompressResult {
        original_size: input.len(),
        compressed_size: input.len(),
        output: CompressOutput::Unchanged,
        new_mime_type: None,
    }
}

/// Check if a ZIP entry path is a compressible image in the expected media directory.
fn is_compressible_image(name: &str, prefixes: &[&str]) -> bool {
    let in_media_dir = prefixes.iter().any(|p| name.starts_with(p));
    if !in_media_dir {
        return false;
    }

    let lower = name.to_ascii_lowercase();
    // Only compress raster image formats. Skip EMF/WMF (vector).
    lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".png")
        || lower.ends_with(".bmp")
        || lower.ends_with(".tiff")
        || lower.ends_with(".tif")
}

/// Try to compress an image from inside an archive.
fn try_compress_archive_image(
    data: &[u8],
    config: &Config,
) -> Result<Option<Vec<u8>>, SqueezeError> {
    // Pre-flight dimension check to reject decompression bombs.
    let dims_reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|e| SqueezeError::ImageDecode(e.to_string()))?;
    if let Ok((w, h)) = dims_reader.into_dimensions()
        && u64::from(w) * u64::from(h) > crate::image::MAX_IMAGE_PIXELS
    {
        return Ok(None);
    }

    let img = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|e| SqueezeError::ImageDecode(e.to_string()))?
        .decode()
        .map_err(|e| SqueezeError::ImageDecode(e.to_string()))?;

    // Use PDF quality/dimension settings for embedded images too.
    compress_image_raw(
        &img,
        data.len(),
        config,
        config.pdf_image_quality,
        config.pdf_image_max_dim,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_compressible_image() {
        let docx_prefixes = ArchiveKind::Docx.image_prefixes();
        assert!(is_compressible_image(
            "word/media/image1.jpg",
            docx_prefixes
        ));
        assert!(is_compressible_image(
            "word/media/image2.png",
            docx_prefixes
        ));
        assert!(!is_compressible_image(
            "word/media/image3.emf",
            docx_prefixes
        ));
        assert!(!is_compressible_image("word/document.xml", docx_prefixes));

        let odf_prefixes = ArchiveKind::Odt.image_prefixes();
        assert!(is_compressible_image("Pictures/photo.jpeg", odf_prefixes));
        assert!(!is_compressible_image("content.xml", odf_prefixes));
    }
}
