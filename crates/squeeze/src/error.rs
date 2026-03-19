use std::fmt;

/// Errors that can occur during attachment compression.
#[derive(Debug)]
pub enum SqueezeError {
    /// Failed to decode an image.
    ImageDecode(String),
    /// Failed to encode an image.
    ImageEncode(String),
    /// Failed to parse a PDF document.
    PdfParse(String),
    /// Failed to write a modified PDF document.
    PdfWrite(String),
    /// Failed to read a ZIP-based archive.
    ArchiveRead(String),
    /// Failed to write a ZIP-based archive.
    ArchiveWrite(String),
    /// Generic I/O error.
    Io(std::io::Error),
}

impl fmt::Display for SqueezeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ImageDecode(msg) => write!(f, "image decode error: {msg}"),
            Self::ImageEncode(msg) => write!(f, "image encode error: {msg}"),
            Self::PdfParse(msg) => write!(f, "PDF parse error: {msg}"),
            Self::PdfWrite(msg) => write!(f, "PDF write error: {msg}"),
            Self::ArchiveRead(msg) => write!(f, "archive read error: {msg}"),
            Self::ArchiveWrite(msg) => write!(f, "archive write error: {msg}"),
            Self::Io(err) => write!(f, "I/O error: {err}"),
        }
    }
}

impl std::error::Error for SqueezeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for SqueezeError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}
