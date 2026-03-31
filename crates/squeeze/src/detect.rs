/// Detected format of an attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Jpeg,
    Png,
    WebP,
    Gif,
    Bmp,
    Tiff,
    Heic,
    Pdf,
    Ooxml(OoxmlKind),
    Odf(OdfKind),
    Svg,
    /// Format we cannot or should not compress.
    Unsupported,
}

/// Sub-kinds of Office Open XML documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OoxmlKind {
    Docx,
    Xlsx,
    Pptx,
}

/// Sub-kinds of Open Document Format files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OdfKind {
    Odt,
    Ods,
    Odp,
}

impl Format {
    /// Returns true if this format is a standalone image.
    #[must_use]
    pub fn is_image(self) -> bool {
        matches!(
            self,
            Self::Jpeg | Self::Png | Self::WebP | Self::Gif | Self::Bmp | Self::Tiff | Self::Heic
        )
    }

    /// Returns a canonical MIME type string for this format.
    #[must_use]
    pub fn to_mime_type(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::WebP => "image/webp",
            Self::Gif => "image/gif",
            Self::Bmp => "image/bmp",
            Self::Tiff => "image/tiff",
            Self::Heic => "image/heic",
            Self::Pdf => "application/pdf",
            Self::Ooxml(OoxmlKind::Docx) => {
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            }
            Self::Ooxml(OoxmlKind::Xlsx) => {
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            }
            Self::Ooxml(OoxmlKind::Pptx) => {
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            }
            Self::Odf(OdfKind::Odt) => "application/vnd.oasis.opendocument.text",
            Self::Odf(OdfKind::Ods) => "application/vnd.oasis.opendocument.spreadsheet",
            Self::Odf(OdfKind::Odp) => "application/vnd.oasis.opendocument.presentation",
            Self::Svg => "image/svg+xml",
            Self::Unsupported => "application/octet-stream",
        }
    }
}

/// Detect format from MIME type string, falling back to magic bytes.
#[must_use]
pub fn detect(mime_type: &str, data: &[u8]) -> Format {
    // Try MIME type first.
    match mime_type.to_ascii_lowercase().as_str() {
        "image/jpeg" | "image/jpg" => return Format::Jpeg,
        "image/png" => return Format::Png,
        "image/webp" => return Format::WebP,
        "image/gif" => return Format::Gif,
        "image/bmp" | "image/x-bmp" | "image/x-ms-bmp" => return Format::Bmp,
        "image/tiff" => return Format::Tiff,
        "image/heic" | "image/heif" => return Format::Heic,
        "application/pdf" => return Format::Pdf,
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
            return Format::Ooxml(OoxmlKind::Docx);
        }
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        | "application/vnd.ms-excel.sheet.macroenabled.12" => {
            return Format::Ooxml(OoxmlKind::Xlsx);
        }
        "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        | "application/vnd.ms-powerpoint.presentation.macroenabled.12" => {
            return Format::Ooxml(OoxmlKind::Pptx);
        }
        "application/vnd.ms-word.document.macroenabled.12" => {
            return Format::Ooxml(OoxmlKind::Docx);
        }
        "application/vnd.oasis.opendocument.text" => return Format::Odf(OdfKind::Odt),
        "application/vnd.oasis.opendocument.spreadsheet" => return Format::Odf(OdfKind::Ods),
        "application/vnd.oasis.opendocument.presentation" => return Format::Odf(OdfKind::Odp),
        "image/svg+xml" => return Format::Svg,
        _ => {}
    }

    // Fall back to magic bytes.
    detect_from_magic(data)
}

/// Detect format from file extension (for CLI usage).
#[must_use]
pub fn detect_from_extension(ext: &str, data: &[u8]) -> Format {
    match ext.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => Format::Jpeg,
        "png" => Format::Png,
        "webp" => Format::WebP,
        "gif" => Format::Gif,
        "bmp" => Format::Bmp,
        "tif" | "tiff" => Format::Tiff,
        "heic" | "heif" => Format::Heic,
        "pdf" => Format::Pdf,
        "docx" | "docm" => Format::Ooxml(OoxmlKind::Docx),
        "xlsx" | "xlsm" => Format::Ooxml(OoxmlKind::Xlsx),
        "pptx" | "pptm" => Format::Ooxml(OoxmlKind::Pptx),
        "svg" | "svgz" => Format::Svg,
        "odt" => Format::Odf(OdfKind::Odt),
        "ods" => Format::Odf(OdfKind::Ods),
        "odp" => Format::Odf(OdfKind::Odp),
        _ => detect_from_magic(data),
    }
}

fn trim_leading_whitespace_and_bom(data: &[u8]) -> &[u8] {
    let mut s = data;
    // Skip UTF-8 BOM.
    if s.starts_with(&[0xEF, 0xBB, 0xBF]) {
        s = &s[3..];
    }
    // Skip whitespace.
    while let Some((&first, rest)) = s.split_first() {
        if first == b' ' || first == b'\t' || first == b'\n' || first == b'\r' {
            s = rest;
        } else {
            break;
        }
    }
    s
}

fn detect_from_magic(data: &[u8]) -> Format {
    if data.len() < 4 {
        return Format::Unsupported;
    }

    // JPEG: FF D8 FF
    if data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        return Format::Jpeg;
    }

    // PNG: 89 50 4E 47
    if data[0] == 0x89 && data[1] == 0x50 && data[2] == 0x4E && data[3] == 0x47 {
        return Format::Png;
    }

    // PDF: 25 50 44 46 (%PDF)
    if data[0] == 0x25 && data[1] == 0x50 && data[2] == 0x44 && data[3] == 0x46 {
        return Format::Pdf;
    }

    // RIFF....WEBP
    if data.len() >= 12
        && data[0] == 0x52
        && data[1] == 0x49
        && data[2] == 0x46
        && data[3] == 0x46
        && data[8] == 0x57
        && data[9] == 0x45
        && data[10] == 0x42
        && data[11] == 0x50
    {
        return Format::WebP;
    }

    // GIF: GIF8
    if data[0] == 0x47 && data[1] == 0x49 && data[2] == 0x46 && data[3] == 0x38 {
        return Format::Gif;
    }

    // BMP: BM
    if data[0] == 0x42 && data[1] == 0x4D {
        return Format::Bmp;
    }

    // TIFF: II (little-endian) or MM (big-endian)
    if (data[0] == 0x49 && data[1] == 0x49 && data[2] == 0x2A && data[3] == 0x00)
        || (data[0] == 0x4D && data[1] == 0x4D && data[2] == 0x00 && data[3] == 0x2A)
    {
        return Format::Tiff;
    }

    // SVG: starts with <svg or <?xml (with svg element inside).
    // Only check for the <svg prefix — <?xml is too ambiguous.
    if data.len() >= 4 && &data[..4] == b"<svg" {
        return Format::Svg;
    }
    // Also check for BOM + <svg or whitespace + <svg.
    if data.len() >= 10 {
        let trimmed = trim_leading_whitespace_and_bom(data);
        if trimmed.starts_with(b"<svg") {
            return Format::Svg;
        }
    }

    // ZIP-based (PK\x03\x04) — could be OOXML or ODF, but without MIME we
    // can't distinguish. Return Unsupported and let the caller provide a MIME type.
    if data[0] == 0x50 && data[1] == 0x4B && data[2] == 0x03 && data[3] == 0x04 {
        // We could try to peek inside the ZIP for content types, but that's
        // complex. For magic-only detection, return Unsupported.
        return Format::Unsupported;
    }

    Format::Unsupported
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mime_detection() {
        assert_eq!(detect("image/jpeg", &[]), Format::Jpeg);
        assert_eq!(detect("image/png", &[]), Format::Png);
        assert_eq!(detect("application/pdf", &[]), Format::Pdf);
        assert_eq!(
            detect(
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                &[]
            ),
            Format::Ooxml(OoxmlKind::Docx)
        );
    }

    #[test]
    fn test_magic_bytes_jpeg() {
        let data = [0xFF, 0xD8, 0xFF, 0xE0, 0x00];
        assert_eq!(detect("application/octet-stream", &data), Format::Jpeg);
    }

    #[test]
    fn test_magic_bytes_png() {
        let data = [0x89, 0x50, 0x4E, 0x47, 0x0D];
        assert_eq!(detect("application/octet-stream", &data), Format::Png);
    }

    #[test]
    fn test_magic_bytes_pdf() {
        let data = [0x25, 0x50, 0x44, 0x46, 0x2D];
        assert_eq!(detect("application/octet-stream", &data), Format::Pdf);
    }

    #[test]
    fn test_unknown_returns_unsupported() {
        assert_eq!(
            detect("application/octet-stream", &[0x00, 0x01, 0x02, 0x03]),
            Format::Unsupported
        );
    }

    #[test]
    fn test_extension_detection() {
        assert_eq!(detect_from_extension("jpg", &[]), Format::Jpeg);
        assert_eq!(detect_from_extension("PDF", &[]), Format::Pdf);
        assert_eq!(
            detect_from_extension("docx", &[]),
            Format::Ooxml(OoxmlKind::Docx)
        );
    }
}
