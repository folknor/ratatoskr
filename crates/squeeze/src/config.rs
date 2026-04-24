/// Common email provider attachment size limits (in bytes), accounting for
/// ~37% MIME base64 encoding overhead. These are the *file* sizes you should
/// target - the on-wire encoded size will be larger.
pub mod limits {
    /// 7 MB - clears even strict on-prem Exchange (10 MB limit after base64).
    pub const EXCHANGE_STRICT: usize = 7 * 1024 * 1024;
    /// 15 MB - clears Outlook.com and iCloud (20 MB limit after base64).
    pub const OUTLOOK_ICLOUD: usize = 15 * 1024 * 1024;
    /// 18 MB - clears Gmail, Yahoo, Proton (25 MB limit after base64).
    pub const GMAIL_YAHOO: usize = 18 * 1024 * 1024;
}

/// Configuration for attachment compression.
#[derive(Debug, Clone)]
pub struct Config {
    /// Maximum longest edge in pixels for standalone image attachments.
    pub max_dimension: u32,
    /// JPEG encoding quality (0-100).
    pub jpeg_quality: u8,
    /// Convert PNG to JPEG for smaller size (lossy).
    pub png_to_jpeg: bool,
    /// Convert BMP/TIFF to JPEG (almost always desirable).
    pub bmp_tiff_to_jpeg: bool,
    /// Minimum savings percentage to bother compressing. If the compressed
    /// output is not at least this much smaller, return `Unchanged`.
    pub min_savings_pct: f32,
    /// JPEG quality for images embedded inside PDFs.
    pub pdf_image_quality: u8,
    /// Maximum longest edge for images embedded inside PDFs.
    pub pdf_image_max_dim: u32,
}

impl Config {
    /// Defaults tuned for email attachments.
    #[must_use]
    pub fn email_default() -> Self {
        Self {
            max_dimension: 2048,
            jpeg_quality: 80,
            png_to_jpeg: false,
            bmp_tiff_to_jpeg: true,
            min_savings_pct: 10.0,
            pdf_image_quality: 75,
            pdf_image_max_dim: 1500,
        }
    }

    /// Lossless-only configuration. Only performs lossless PNG recompression
    /// and skips lossy conversions. Useful as a baseline.
    #[must_use]
    pub fn lossless() -> Self {
        Self {
            max_dimension: u32::MAX,
            jpeg_quality: 100,
            png_to_jpeg: false,
            bmp_tiff_to_jpeg: false,
            min_savings_pct: 5.0,
            pdf_image_quality: 100,
            pdf_image_max_dim: u32::MAX,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::email_default()
    }
}
