use std::borrow::Cow;

use iced::font::{Style, Weight};
use iced::widget::text::LineHeight;
use iced::Pixels;

// ── Font constants ───────────────────────────────────────

pub const ICON: iced::Font = iced::Font::with_name("lucide");

pub const TEXT: iced::Font = iced::Font::with_name("Inter");
pub const TEXT_BOLD: iced::Font = iced::Font {
    weight: Weight::Bold,
    ..iced::Font::with_name("Inter")
};
pub const TEXT_ITALIC: iced::Font = iced::Font {
    style: Style::Italic,
    ..iced::Font::with_name("Inter")
};
pub const TEXT_BOLD_ITALIC: iced::Font = iced::Font {
    weight: Weight::Bold,
    style: Style::Italic,
    ..iced::Font::with_name("Inter")
};
pub const TEXT_SEMIBOLD: iced::Font = iced::Font {
    weight: Weight::Semibold,
    ..iced::Font::with_name("Inter")
};
pub const TEXT_LIGHT: iced::Font = iced::Font {
    weight: Weight::Light,
    ..iced::Font::with_name("Inter")
};

// ── Sizes ────────────────────────────────────────────────

pub const TEXT_SIZE: f32 = 13.0;
pub const ICON_SIZE: f32 = 14.0;

// ── Line height ──────────────────────────────────────────

pub const DEFAULT_LINE_HEIGHT: f32 = 1.4;

pub fn line_height() -> LineHeight {
    LineHeight::Relative(DEFAULT_LINE_HEIGHT)
}

/// Resolve the absolute line height in pixels for a given font size.
pub fn resolve_line_height(font_size: f32) -> f32 {
    line_height()
        .to_absolute(Pixels(font_size))
        .0
}

// ── Font loading ─────────────────────────────────────────

pub fn load() -> impl Iterator<Item = Cow<'static, [u8]>> {
    [
        Cow::Borrowed(include_bytes!("../fonts/InterVariable.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../fonts/InterVariable-Italic.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../fonts/lucide.ttf").as_slice()),
    ]
    .into_iter()
}
