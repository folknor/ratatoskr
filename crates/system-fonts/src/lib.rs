//! Cross-platform system font detection.
//!
//! Queries the desktop environment for the user's configured fonts, rather than
//! relying on fontconfig's static XML configuration (which does not reflect
//! GNOME/KDE font settings).
//!
//! # Linux
//!
//! Uses the `org.freedesktop.portal.Settings` D-Bus interface to read
//! `org.gnome.desktop.interface` font keys. Works on GNOME, KDE, and any DE
//! with an xdg-desktop-portal backend.
//!
//! # Windows
//!
//! Reads the system `NONCLIENTMETRICS` for the UI font (typically Segoe UI).

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

/// A system font with family name and size.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemFont {
    /// Font family name (e.g., "Inter", "Segoe UI").
    pub family: String,
    /// Font size in points. `None` if the platform does not report a size.
    pub size: Option<f32>,
}

/// The set of fonts configured by the desktop environment.
#[derive(Debug, Clone, Default)]
pub struct SystemFonts {
    /// The default UI font (window titles, buttons, labels).
    pub ui: Option<SystemFont>,
    /// The monospace font (terminal, code).
    pub monospace: Option<SystemFont>,
    /// The document font (body text in document-oriented apps).
    pub document: Option<SystemFont>,
}

impl SystemFonts {
    /// Query the desktop environment for configured fonts.
    ///
    /// Returns `SystemFonts::default()` (all `None`) if detection fails or the
    /// platform is unsupported. This is intentional — callers should always have
    /// a bundled fallback font.
    pub async fn detect() -> Self {
        #[cfg(target_os = "linux")]
        {
            linux::detect().await
        }
        #[cfg(target_os = "windows")]
        {
            windows::detect().await
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            Self::default()
        }
    }
}

/// Parse a GTK/GNOME font description string like "Inter 15" or "Noto Sans Bold 12"
/// into a family name and size.
///
/// The format is `<family> <size>` where family may contain spaces and the size
/// is always the last whitespace-separated token. Style words (Bold, Italic, etc.)
/// that appear between the family name and size are stripped.
fn parse_font_description(desc: &str) -> Option<SystemFont> {
    let desc = desc.trim();
    if desc.is_empty() {
        return None;
    }

    // The size is always the last token
    let last_space = desc.rfind(' ')?;
    let (prefix, size_str) = desc.split_at(last_space);
    let size: f32 = size_str.trim().parse().ok()?;

    // Strip trailing style words from the family name (Bold, Italic, Medium, etc.)
    let style_words = [
        "Thin",
        "ExtraLight",
        "Extra Light",
        "UltraLight",
        "Ultra Light",
        "Light",
        "Regular",
        "Medium",
        "SemiBold",
        "Semi Bold",
        "DemiBold",
        "Demi Bold",
        "Bold",
        "ExtraBold",
        "Extra Bold",
        "UltraBold",
        "Ultra Bold",
        "Black",
        "Heavy",
        "Italic",
        "Oblique",
    ];

    let mut family = prefix.trim().to_string();
    // Repeatedly strip trailing style words
    loop {
        let mut stripped = false;
        for word in &style_words {
            if let Some(rest) = family.strip_suffix(word) {
                let rest = rest.trim_end();
                if !rest.is_empty() {
                    family = rest.to_string();
                    stripped = true;
                    break;
                }
            }
        }
        if !stripped {
            break;
        }
    }

    if family.is_empty() {
        return None;
    }

    Some(SystemFont {
        family,
        size: Some(size),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple() {
        let font = parse_font_description("Inter 15").expect("should parse");
        assert_eq!(font.family, "Inter");
        assert_eq!(font.size, Some(15.0));
    }

    #[test]
    fn parse_with_spaces_in_name() {
        let font = parse_font_description("Noto Sans 12").expect("should parse");
        assert_eq!(font.family, "Noto Sans");
        assert_eq!(font.size, Some(12.0));
    }

    #[test]
    fn parse_with_style_words() {
        let font = parse_font_description("Noto Sans Bold 12").expect("should parse");
        assert_eq!(font.family, "Noto Sans");
        assert_eq!(font.size, Some(12.0));
    }

    #[test]
    fn parse_with_multiple_style_words() {
        let font = parse_font_description("Noto Sans Bold Italic 10").expect("should parse");
        assert_eq!(font.family, "Noto Sans");
        assert_eq!(font.size, Some(10.0));
    }

    #[test]
    fn parse_medium_weight() {
        let font =
            parse_font_description("IosevkaTerm Nerd Font Medium 15").expect("should parse");
        assert_eq!(font.family, "IosevkaTerm Nerd Font");
        assert_eq!(font.size, Some(15.0));
    }

    #[test]
    fn parse_fractional_size() {
        let font = parse_font_description("Inter 10.5").expect("should parse");
        assert_eq!(font.family, "Inter");
        assert_eq!(font.size, Some(10.5));
    }

    #[test]
    fn parse_empty() {
        assert!(parse_font_description("").is_none());
    }

    #[test]
    fn parse_no_size() {
        assert!(parse_font_description("Inter").is_none());
    }

    #[test]
    fn parse_ambiguous_style_as_family() {
        // "Bold 12" — "Bold" could be a font family name; we don't strip
        // it because that would leave an empty family.
        let font = parse_font_description("Bold 12").expect("should parse");
        assert_eq!(font.family, "Bold");
        assert_eq!(font.size, Some(12.0));
    }
}
