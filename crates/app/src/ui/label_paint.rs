use iced::Color;
use label_colors::LabelStyleHex;

use crate::ui::theme;

/// Complete UI paint for a label-shaped surface.
///
/// Fields are private and there is no public struct literal. Every
/// `LabelPaint` is produced by one of the constructors below, which all
/// ensure a complete `(bg, fg)` pair: either a `LabelStyleHex` (validated
/// complete pair from the resolver) or a hashed-name fallback that pairs
/// the deterministic avatar color with `theme::ON_AVATAR`.
///
/// ```compile_fail,E0451
/// use app::LabelPaint;
/// let _ = LabelPaint {
///     bg: iced::Color::BLACK,
///     fg: iced::Color::WHITE,
/// };
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LabelPaint {
    bg: Color,
    fg: Color,
}

impl LabelPaint {
    pub fn from_hex(hex: LabelStyleHex<'_>) -> Self {
        Self {
            bg: theme::hex_to_color(hex.bg()),
            fg: theme::hex_to_color(hex.fg()),
        }
    }

    /// Convenience for the common `from_hex(LabelStyleHex::new(bg, fg))`
    /// shape at the DB/settings boundary, where the two hex strings come
    /// out of separate columns.
    pub fn from_hex_pair(bg: &str, fg: &str) -> Self {
        Self::from_hex(LabelStyleHex::new(bg, fg))
    }

    /// Hashed-name fallback for label-shaped surfaces with no stored
    /// color pair (e.g. a `label_groups` row whose `color_bg`/`color_fg`
    /// are both NULL). Background is derived deterministically from the
    /// name; foreground is `theme::ON_AVATAR` so the pair stays complete.
    pub fn hashed_from_name(name: &str) -> Self {
        Self {
            bg: theme::avatar_color(name),
            fg: theme::ON_AVATAR,
        }
    }

    pub fn bg(self) -> Color {
        self.bg
    }

    pub fn fg(self) -> Color {
        self.fg
    }
}

#[cfg(test)]
mod tests {
    use super::LabelPaint;

    #[test]
    fn converts_complete_hex_pair() {
        let paint = LabelPaint::from_hex_pair("#ff0000", "#ffffff");
        assert_eq!(paint.bg(), iced::Color::from_rgb8(255, 0, 0));
        assert_eq!(paint.fg(), iced::Color::from_rgb8(255, 255, 255));
    }

    #[test]
    fn hashed_fallback_pairs_with_on_avatar() {
        let paint = LabelPaint::hashed_from_name("Work");
        assert_eq!(paint.fg(), crate::ui::theme::ON_AVATAR);
    }
}
